//! Bounded queries over already source/build-validated R4/R5 facts.
//!
//! This boundary has no source bytes or graph handle. Callers must first use
//! `validate_against` (and `validate_canonical` for collected facts), pinning an
//! independently expected build. Hashes bind evidence; they do not attest its
//! producer, compiler execution, or runtime reproduction.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, ensure, Context as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spur_solver::service::SolverService;
use spur_solver::types::{
    ConstraintDecl, ConstraintExpr as Expr, ConstraintItem, ConstraintOp as Op, ModelValue,
    SolveConstraintsRequest, SolveConstraintsResponse, SolveModel, SolveStatus, Variable,
    MAX_EXPRESSION_DEPTH, MAX_TIMEOUT_MS,
};

use super::*;

pub const PANIC_ANALYSIS_SCHEMA_VERSION: &str = "panic-analysis-v1";

/// Explicit entry scope and exploration budgets; no ambient build is inferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicAnalysisLimits {
    pub entry_body_ids: Vec<String>,
    pub expected_build_hash: String,
    pub max_call_depth: u32,
    pub max_loop_iterations: u32,
    /// Maximum instantiated control-flow nodes per entry query.
    pub max_states: u32,
    pub timeout_ms: u64,
    pub persist: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicAnalysisOutcome {
    CandidateWitness,
    NoCounterexampleWithinCompleteScope,
    IncompleteSemantics,
    Unknown,
    Timeout,
    OperationalError,
    InvalidModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicAnalysisInput {
    pub value_id: String,
    pub parameter_index: u32,
    pub variable: String,
    pub domain: PanicValueDomain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicAnalysisEvent {
    pub event_id: String,
    pub context: PanicFactContext,
    pub predicate: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicModelValidation {
    pub valid: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicCandidateWitness {
    pub event_ids: Vec<String>,
    pub inputs: SolveModel,
    /// Analysis alone can never establish runtime reproduction.
    pub runtime_reproduced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicAnalysisQuery {
    pub entry_body_id: String,
    pub inputs: Vec<PanicAnalysisInput>,
    pub events: Vec<PanicAnalysisEvent>,
    pub request: SolveConstraintsRequest,
    pub request_digest: String,
    /// Binds this request to the report's source/build/facts and explicit limits.
    pub request_identity: String,
    pub response: Option<SolveConstraintsResponse>,
    pub response_digest: Option<String>,
    pub receipt_binding: Option<String>,
    pub operational_error: Option<String>,
    pub model_validation: Option<PanicModelValidation>,
    pub candidate: Option<PanicCandidateWitness>,
    pub outcome: PanicAnalysisOutcome,
    pub obligations: Vec<PanicUnknownObligation>,
    pub explored_states: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicAnalysisReport {
    pub schema_version: String,
    pub graph_content_hash: String,
    pub sources: Vec<RustCallSourceBinding>,
    pub build_hash: String,
    pub fact_digest: String,
    pub limits: PanicAnalysisLimits,
    pub assumptions: Vec<String>,
    pub queries: Vec<PanicAnalysisQuery>,
}

/// Analyze one existential panic query per explicit entry. UNSAT is classified
/// as complete only when every required fact and exploration obligation closed.
/// Operational solver failures remain in the report; malformed inputs return Err.
pub async fn analyze_panic_facts(
    service: &SolverService,
    facts: &PanicSemanticFacts,
    limits: &PanicAnalysisLimits,
) -> anyhow::Result<PanicAnalysisReport> {
    validate_input(facts, limits)?;
    let fact_digest = digest(facts)?;
    let mut queries = vec![];
    for entry in &limits.entry_body_ids {
        let mut compiler = Compiler::new(facts, limits)?;
        compiler.body(entry, None, boolean(true), 0)?;
        let panic = or(compiler
            .events
            .iter()
            .map(|e| e.predicate.clone())
            .collect());
        compiler.assert("panic", panic);
        let request = compiler.request;
        let request_digest = digest(&request)?;
        let request_identity = digest(&(
            &facts.graph_content_hash,
            &facts.sources,
            &facts.build_hash,
            &fact_digest,
            limits,
            entry,
            &request_digest,
        ))?;
        let mut query = PanicAnalysisQuery {
            entry_body_id: entry.clone(),
            inputs: compiler.inputs,
            events: compiler.events,
            request,
            request_digest,
            request_identity,
            response: None,
            response_digest: None,
            receipt_binding: None,
            operational_error: None,
            model_validation: None,
            candidate: None,
            outcome: PanicAnalysisOutcome::OperationalError,
            obligations: compiler.obligations.into_values().collect(),
            explored_states: compiler.states,
        };
        // Size/depth caps belong to the existing typed service. Never loosen
        // them or turn a rejected request into a vacuous UNSAT result.
        if let Err(error) = query.request.validate() {
            query.operational_error = Some(error.to_string());
        } else {
            match service.solve_constraints(query.request.clone()).await {
                Err(error) => query.operational_error = Some(error.to_string()),
                Ok(response) => {
                    query.outcome = match response.status {
                        SolveStatus::Sat => {
                            let validation =
                                validate_model(&query.request, response.model.as_ref());
                            let valid = validation.valid;
                            query.model_validation = Some(validation);
                            if valid {
                                let model = response.model.as_ref().context("SAT missing model")?;
                                let event_ids: Vec<_> = query
                                    .events
                                    .iter()
                                    .filter(|e| {
                                        evaluate(&e.predicate, model)
                                            == Some(ModelValue::Bool(true))
                                    })
                                    .map(|e| e.event_id.clone())
                                    .collect();
                                ensure!(
                                    !event_ids.is_empty(),
                                    "validated panic model has no event"
                                );
                                query.candidate = Some(PanicCandidateWitness {
                                    event_ids,
                                    inputs: query
                                        .inputs
                                        .iter()
                                        .map(|i| (i.variable.clone(), model[&i.variable].clone()))
                                        .collect(),
                                    runtime_reproduced: false,
                                });
                                PanicAnalysisOutcome::CandidateWitness
                            } else {
                                PanicAnalysisOutcome::InvalidModel
                            }
                        }
                        SolveStatus::Unsat if query.obligations.is_empty() => {
                            PanicAnalysisOutcome::NoCounterexampleWithinCompleteScope
                        }
                        SolveStatus::Unsat => PanicAnalysisOutcome::IncompleteSemantics,
                        SolveStatus::Unknown => PanicAnalysisOutcome::Unknown,
                        SolveStatus::Timeout => PanicAnalysisOutcome::Timeout,
                        SolveStatus::Error | SolveStatus::Ended => {
                            PanicAnalysisOutcome::OperationalError
                        }
                    };
                    let response_digest = digest(&response)?;
                    query.receipt_binding =
                        Some(digest(&(&query.request_identity, &response_digest))?);
                    query.response_digest = Some(response_digest);
                    query.response = Some(response);
                }
            }
        }
        queries.push(query);
    }
    Ok(PanicAnalysisReport {
        schema_version: PANIC_ANALYSIS_SCHEMA_VERSION.into(), graph_content_hash: facts.graph_content_hash.clone(),
        sources: facts.sources.clone(), build_hash: facts.build_hash.clone(), fact_digest,
        limits: limits.clone(), assumptions: vec![
            "Caller authenticated facts against exact graph/source bytes and independently expected build; analysis cannot repeat source validation without those inputs.".into(),
            "Only the declared bool/u8 model, entry domains and explicit call/loop/state bounds are analyzed; unsupported semantics remain obligations.".into(),
            "Source model and primitive-contract authority are producer declarations, not compiler or runtime attestations.".into(),
            "Candidate models satisfy the exact emitted constraints; runtime reproduction requires a separate operation.".into(),
        ], queries,
    })
}

fn digest(value: &impl Serialize) -> anyhow::Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

fn validate_input(f: &PanicSemanticFacts, l: &PanicAnalysisLimits) -> anyhow::Result<()> {
    ensure!(
        f.schema_version == PANIC_SEMANTIC_FACTS_SCHEMA_VERSION,
        "unsupported semantic version"
    );
    ensure!(
        f.build_hash == f.build.content_hash()? && f.build_hash == l.expected_build_hash,
        "build identity mismatch"
    );
    ensure!(
        f.graph_content_hash == f.build.graph_content_hash,
        "graph/build mismatch"
    );
    ensure!(
        l.max_states > 0 && l.timeout_ms <= MAX_TIMEOUT_MS,
        "invalid analysis budget"
    );
    ensure!(!l.entry_body_ids.is_empty(), "explicit entries required");
    let entries: BTreeSet<_> = l.entry_body_ids.iter().collect();
    ensure!(entries.len() == l.entry_body_ids.len(), "duplicate entry");
    for id in entries {
        ensure!(
            f.bodies.iter().any(|b| &b.body_id == id),
            "missing entry body"
        );
    }
    for ids in [
        f.values
            .iter()
            .map(|v| v.value_id.as_str())
            .collect::<Vec<_>>(),
        f.expressions
            .iter()
            .map(|e| e.expression_id.as_str())
            .collect(),
        f.control_flow.iter().map(|c| c.flow_id.as_str()).collect(),
        f.assignments
            .iter()
            .map(|a| a.assignment_id.as_str())
            .collect(),
        f.sites.iter().map(|s| s.site_id.as_str()).collect(),
    ] {
        ensure!(
            ids.iter().all(|id| !id.is_empty())
                && ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
            "duplicate/missing fact identity"
        );
    }
    for (sort, domain, context) in f
        .values
        .iter()
        .map(|v| (v.sort, &v.domain, &v.context))
        .chain(
            f.expressions
                .iter()
                .map(|e| (e.sort, &e.domain, &e.context)),
        )
    {
        ensure!(
            match (sort, domain) {
                (PanicSort::Bool, PanicValueDomain::Bool { values }) =>
                    !values.is_empty() && values.windows(2).all(|w| w[0] < w[1]),
                (PanicSort::U8, PanicValueDomain::U8 { minimum, maximum }) =>
                    minimum <= maximum && *maximum <= 255,
                _ => false,
            },
            "invalid value domain"
        );
        ensure!(
            f.control_flow
                .iter()
                .any(|c| c.flow_id == context.flow_id
                    && c.location.body_id == context.location.body_id),
            "detached fact context"
        );
    }
    if let Some(x) = &f.extraction {
        ensure!(
            x.extractor_version == PANIC_EXTRACTION_VERSION,
            "unsupported extraction version"
        );
    }
    Ok(())
}

#[derive(Clone, Default)]
struct Frame {
    body: String,
    instance: u32,
    expressions: BTreeMap<String, Expr>,
    values: BTreeMap<String, Expr>,
    exits: BTreeMap<String, Expr>,
    reaches: BTreeMap<String, Expr>,
    visiting: BTreeSet<String>,
    returns: Vec<(Expr, Expr)>,
}

struct Compiler<'a> {
    facts: &'a PanicSemanticFacts,
    limits: &'a PanicAnalysisLimits,
    request: SolveConstraintsRequest,
    inputs: Vec<PanicAnalysisInput>,
    events: Vec<PanicAnalysisEvent>,
    obligations: BTreeMap<String, PanicUnknownObligation>,
    states: u32,
    instances: u32,
}

impl<'a> Compiler<'a> {
    fn new(facts: &'a PanicSemanticFacts, limits: &'a PanicAnalysisLimits) -> anyhow::Result<Self> {
        let request = serde_json::from_value(serde_json::json!({
            "vars":[],"constraints":[],"timeout_ms":limits.timeout_ms,
            "persist":limits.persist,"include_smt":true,"use_cache":false,
        }))?;
        Ok(Self {
            facts,
            limits,
            request,
            inputs: vec![],
            events: vec![],
            obligations: BTreeMap::new(),
            states: 0,
            instances: 0,
        })
    }
    fn assert(&mut self, label: &str, expr: Expr) {
        let id = format!("{label}_{}", self.request.constraints.len());
        self.request
            .constraints
            .push(ConstraintItem::Declared(ConstraintDecl {
                id: Some(id),
                group: None,
                soft: false,
                weight: None,
                expr,
            }));
    }
    fn variable(&mut self, sort: PanicSort, domain: &PanicValueDomain, input: bool) -> Expr {
        let name = format!(
            "{}_{}",
            if input { "input" } else { "aux" },
            self.request.vars.len()
        );
        let var = match (sort, domain) {
            (PanicSort::Bool, PanicValueDomain::Bool { .. }) => {
                Variable::Bool { name: name.clone() }
            }
            (PanicSort::U8, PanicValueDomain::U8 { minimum, maximum }) => Variable::IntRange {
                name: name.clone(),
                min: (*minimum).into(),
                max: (*maximum).into(),
            },
            _ => unreachable!("validated domain"),
        };
        self.request.vars.push(var);
        let term = Expr::Var { name };
        if let PanicValueDomain::Bool { values } = domain {
            if values.len() == 1 {
                self.assert("bool_domain", eq(term.clone(), boolean(values[0])));
            }
        }
        term
    }
    fn fresh(&mut self, sort: PanicSort) -> Expr {
        self.variable(
            sort,
            &match sort {
                PanicSort::Bool => PanicValueDomain::Bool {
                    values: vec![false, true],
                },
                PanicSort::U8 => PanicValueDomain::U8 {
                    minimum: 0,
                    maximum: 255,
                },
            },
            false,
        )
    }
    fn share(&mut self, term: Expr, sort: PanicSort) -> Expr {
        // Reserve half of the service-owned depth for its enclosing guard,
        // origin and panic combinators; retain definitions in the exact request.
        if depth(&term) < MAX_EXPRESSION_DEPTH / 2 {
            return term;
        }
        let symbol = self.fresh(sort);
        self.assert("definition", eq(symbol.clone(), term));
        symbol
    }
    fn choose(&mut self, condition: Expr, yes: Expr, no: Expr, sort: PanicSort) -> Expr {
        if yes == no {
            return yes;
        }
        if let Expr::Bool { value } = condition {
            return if value { yes } else { no };
        }
        if sort == PanicSort::Bool {
            return or(vec![
                and(vec![condition.clone(), yes]),
                and(vec![not(condition), no]),
            ]);
        }
        let result = self.fresh(sort);
        self.assert(
            "choice",
            or(vec![
                and(vec![condition.clone(), eq(result.clone(), yes)]),
                and(vec![not(condition), eq(result.clone(), no)]),
            ]),
        );
        result
    }
    fn obligation(
        &mut self,
        kind: PanicUnknownKind,
        context: &PanicFactContext,
        id: &str,
        detail: &str,
    ) {
        self.obligations
            .entry(id.into())
            .or_insert_with(|| PanicUnknownObligation {
                obligation_id: id.into(),
                kind,
                location: context.location.clone(),
                detail: detail.into(),
            });
    }
    fn body(
        &mut self,
        body: &str,
        arguments: Option<Vec<Expr>>,
        gate: Expr,
        call_depth: u32,
    ) -> anyhow::Result<Vec<(Expr, Expr)>> {
        let coverage = self
            .facts
            .body_coverage
            .iter()
            .find(|b| b.body_id == body)
            .context("missing body coverage")?
            .clone();
        let entry = self
            .facts
            .control_flow
            .iter()
            .find(|f| f.flow_id == coverage.entry_flow_id)
            .context("missing entry flow")?
            .clone();
        let context = PanicFactContext {
            location: entry.location.clone(),
            flow_id: entry.flow_id.clone(),
            guard: entry.guard.clone(),
        };
        let mut unresolved = false;
        if arguments.is_some()
            && !self
                .facts
                .extraction
                .as_ref()
                .is_some_and(|e| e.returns.iter().any(|r| r.context.location.body_id == body))
        {
            self.obligation(
                PanicUnknownKind::Callee,
                &context,
                &format!("return_{body}"),
                "callee has no retained return relation; no normal continuation is inferred",
            );
        }
        for expression in self
            .facts
            .expressions
            .iter()
            .filter(|e| e.context.location.body_id == body)
        {
            let modeled=match expression.derivation {
                PanicExpressionDerivation::U8Add{..} if self.facts.build.overflow_checks =>
                    self.facts.extraction.as_ref().is_some_and(|e|e.operations.iter().any(|o|o.result_expression_id==expression.expression_id)) ||
                    self.facts.sites.iter().any(|s|matches!(&s.semantics,PanicSiteSemantics::CheckedAdd{value,..} if value==&expression.expression_id)),
                PanicExpressionDerivation::U8ArrayIndex{..} => self.facts.extraction.as_ref().is_some_and(|e|e.operations.iter().any(|o|o.result_expression_id==expression.expression_id)),
                _=>true,
            };
            if !modeled {
                self.obligation(
                    PanicUnknownKind::Opaque,
                    &expression.context,
                    &format!("operation_{}", expression.expression_id),
                    "expression lacks its required implicit panic operation",
                );
                unresolved = true;
            }
        }
        for unknown in self
            .facts
            .unknown_obligations
            .iter()
            .filter(|o| o.location.body_id == body)
        {
            let template = self.facts.extraction.as_ref().is_some_and(|e| {
                e.loops
                    .iter()
                    .any(|l| l.obligation_id == unknown.obligation_id)
            });
            if !template {
                self.obligations
                    .insert(unknown.obligation_id.clone(), unknown.clone());
                unresolved = true;
            }
        }
        for frontier in self
            .facts
            .exploration
            .frontiers
            .iter()
            .filter(|f| f.context.location.body_id == body)
        {
            let unknown = self
                .facts
                .unknown_obligations
                .iter()
                .find(|o| o.obligation_id == frontier.obligation_id)
                .context("missing frontier obligation")?;
            self.obligations
                .insert(unknown.obligation_id.clone(), unknown.clone());
            unresolved = true;
        }
        // Unknown effects can precede a known panic and invalidate its inputs.
        // Do not synthesize candidates through an unauthenticated body summary.
        if unresolved {
            return Ok(vec![]);
        }
        self.instances += 1;
        let mut frame = Frame {
            body: body.into(),
            instance: self.instances,
            ..Frame::default()
        };
        let mut parameters: Vec<_> = self
            .facts
            .values
            .iter()
            .filter_map(|v| match v.origin {
                PanicValueOrigin::Parameter { index } if v.context.location.body_id == body => {
                    Some((index, v.clone()))
                }
                _ => None,
            })
            .collect();
        parameters.sort_by_key(|(i, _)| *i);
        ensure!(
            parameters
                .iter()
                .enumerate()
                .all(|(n, (i, _))| n == *i as usize),
            "noncontiguous parameter indices"
        );
        if let Some(args) = arguments {
            ensure!(
                parameters.len() == args.len(),
                "helper argument arity mismatch"
            );
            for ((_, v), term) in parameters.into_iter().zip(args) {
                frame.values.insert(v.value_id, term);
            }
        } else {
            for (index, v) in parameters {
                let term = self.variable(v.sort, &v.domain, true);
                let Expr::Var { name } = &term else {
                    unreachable!()
                };
                self.inputs.push(PanicAnalysisInput {
                    value_id: v.value_id.clone(),
                    parameter_index: index,
                    variable: name.clone(),
                    domain: v.domain,
                });
                frame.values.insert(v.value_id, term);
            }
        }
        let flows: Vec<_> = self
            .facts
            .control_flow
            .iter()
            .filter(|f| f.location.body_id == body)
            .cloned()
            .collect();
        self.region(&mut frame, &flows, &entry.flow_id, gate, call_depth, None)?;
        Ok(frame.returns)
    }
    fn region(
        &mut self,
        frame: &mut Frame,
        flows: &[PanicControlFlow],
        entry: &str,
        gate: Expr,
        call_depth: u32,
        current_loop: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut skipped = BTreeSet::new();
        for flow in flows {
            if skipped.contains(&flow.flow_id) {
                continue;
            }
            let template = self
                .facts
                .extraction
                .as_ref()
                .and_then(|e| {
                    e.loops.iter().find(|l| {
                        l.iteration_entry_flow_id == flow.flow_id
                            && Some(l.loop_id.as_str()) != current_loop
                    })
                })
                .cloned();
            if let Some(template) = template {
                let inside: Vec<_> = flows
                    .iter()
                    .filter(|f| contained(f.location.byte_range, flow.location.byte_range))
                    .cloned()
                    .collect();
                ensure!(!inside.is_empty(), "missing loop region");
                skipped.extend(inside.iter().map(|f| f.flow_id.clone()));
                let mut continuation = frame
                    .exits
                    .get(&template.context.flow_id)
                    .context("missing loop predecessor")?
                    .clone();
                continuation = and(vec![
                    continuation,
                    self.guard(frame, &template.context.guard)?,
                ]);
                let count = if template.start > template.end {
                    0
                } else {
                    u32::from(template.end) - u32::from(template.start)
                        + u32::from(template.inclusive)
                };
                let explored = count.min(self.limits.max_loop_iterations);
                for offset in 0..explored {
                    let mut iteration = frame.clone();
                    self.instances += 1;
                    iteration.instance = self.instances;
                    iteration.returns.clear();
                    iteration.values.insert(
                        template.induction_value_id.clone(),
                        integer(i64::from(template.start) + i64::from(offset)),
                    );
                    self.region(
                        &mut iteration,
                        &inside,
                        &template.iteration_entry_flow_id,
                        continuation,
                        call_depth,
                        Some(&template.loop_id),
                    )?;
                    continuation = template
                        .iteration_exit_flow_id
                        .as_ref()
                        .and_then(|id| iteration.exits.get(id))
                        .cloned()
                        .unwrap_or_else(|| boolean(false));
                    frame.returns.extend(iteration.returns);
                }
                if explored < count {
                    self.obligation(
                        PanicUnknownKind::LoopBound,
                        &template.context,
                        &template.obligation_id,
                        "literal loop prefix leaves unexplored iterations",
                    );
                    continuation = boolean(false);
                }
                frame
                    .exits
                    .insert(template.context.flow_id.clone(), continuation);
                continue;
            }
            if self.states >= self.limits.max_states {
                self.obligation(
                    PanicUnknownKind::StateBudget,
                    &PanicFactContext {
                        location: flow.location.clone(),
                        flow_id: flow.flow_id.clone(),
                        guard: flow.guard.clone(),
                    },
                    &format!("state_budget_{}", frame.instance),
                    "instantiated control-flow state budget exhausted",
                );
                return Ok(());
            }
            self.states += 1;
            let predecessors = if flow.flow_id == entry {
                gate.clone()
            } else {
                let mut terms = vec![];
                for pred in &flow.predecessors {
                    terms.push(
                        frame
                            .exits
                            .get(pred)
                            .context("flow predecessor unavailable or out of order")?
                            .clone(),
                    );
                }
                or(terms)
            };
            let guard = self.guard(frame, &flow.guard)?;
            let reached = self.share(and(vec![predecessors, guard]), PanicSort::Bool);
            frame.reaches.insert(flow.flow_id.clone(), reached.clone());
            let mut normal = reached;
            // Flow order, not byte order or lexically sorted site IDs, defines
            // evaluation. Multiple indistinguishable events retain an obligation.
            let sites: Vec<_> = self
                .facts
                .sites
                .iter()
                .filter(|s| s.context.flow_id == flow.flow_id)
                .cloned()
                .collect();
            let operations: Vec<_> = self
                .facts
                .extraction
                .as_ref()
                .map(|e| {
                    e.operations
                        .iter()
                        .filter(|o| o.context.flow_id == flow.flow_id)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let effects = sites
                .iter()
                .filter(|s| {
                    matches!(
                        s.semantics,
                        PanicSiteSemantics::OptionUnwrap { .. }
                            | PanicSiteSemantics::DirectCall { .. }
                            | PanicSiteSemantics::Primitive { .. }
                            | PanicSiteSemantics::Unknown { .. }
                    )
                })
                .count()
                + operations.len();
            if effects > 1 {
                self.obligation(
                    PanicUnknownKind::Opaque,
                    &PanicFactContext {
                        location: flow.location.clone(),
                        flow_id: flow.flow_id.clone(),
                        guard: flow.guard.clone(),
                    },
                    &format!("order_{}", flow.flow_id),
                    "multiple effects lack a distinct evaluation order within one flow node",
                );
                for site in &sites {
                    if let PanicSiteSemantics::DirectCall {
                        result_value_id, ..
                    } = &site.semantics
                    {
                        let result = self
                            .facts
                            .values
                            .iter()
                            .find(|v| v.value_id == *result_value_id)
                            .context("missing call result")?;
                        let term = self.fresh(result.sort);
                        frame.values.insert(result_value_id.clone(), term);
                    }
                }
                frame.exits.insert(flow.flow_id.clone(), boolean(false));
                continue;
            }
            for site in sites {
                let local_guard = self.guard(frame, &site.context.guard)?;
                let active = and(vec![normal.clone(), local_guard]);
                match site.semantics {
                    PanicSiteSemantics::OptionUnwrap { is_some, .. } => {
                        let condition = not(self.expression(frame, &is_some)?);
                        normal = self.event(
                            frame,
                            &site.site_id,
                            &site.context,
                            active,
                            normal,
                            condition,
                        );
                    }
                    PanicSiteSemantics::DirectCall {
                        callee_body_id,
                        arguments,
                        result_value_id,
                    } => {
                        if call_depth >= self.limits.max_call_depth {
                            self.obligation(
                                PanicUnknownKind::CallDepth,
                                &site.context,
                                &format!("call_{}_{}", frame.instance, site.site_id),
                                "callee exceeds explicit call-depth bound",
                            );
                            normal = and(vec![normal, not(active)]);
                            let result = self
                                .facts
                                .values
                                .iter()
                                .find(|v| v.value_id == result_value_id)
                                .context("missing call result")?;
                            let term = self.fresh(result.sort);
                            frame.values.insert(result_value_id, term);
                        } else {
                            let args = arguments
                                .iter()
                                .map(|a| self.expression(frame, a))
                                .collect::<anyhow::Result<Vec<_>>>()?;
                            let returns = self.body(
                                &callee_body_id,
                                Some(args),
                                active.clone(),
                                call_depth + 1,
                            )?;
                            let value = self
                                .facts
                                .values
                                .iter()
                                .find(|v| v.value_id == result_value_id)
                                .context("missing call result")?;
                            let result = self.fresh(value.sort);
                            let mut returned = vec![];
                            for (guard, term) in returns {
                                self.assert(
                                    "helper_return",
                                    implies(guard.clone(), eq(result.clone(), term)),
                                );
                                returned.push(guard);
                            }
                            let returned = or(returned);
                            frame.values.insert(result_value_id, result);
                            normal = and(vec![normal, or(vec![not(active), returned])]);
                        }
                    }
                    PanicSiteSemantics::Primitive {
                        contract_id,
                        arguments,
                    } => {
                        let contract = self
                            .facts
                            .primitive_contracts
                            .iter()
                            .find(|c| c.contract_id == contract_id)
                            .context("missing primitive contract")?;
                        ensure!(
                            contract.build_hash == self.facts.build_hash,
                            "primitive build mismatch"
                        );
                        let args = arguments
                            .iter()
                            .map(|a| self.expression(frame, a))
                            .collect::<anyhow::Result<Vec<_>>>()?;
                        let condition = match (contract.primitive, args.as_slice()) {
                            (PanicPrimitive::AssertBool, [a]) => not(a.clone()),
                            (PanicPrimitive::U8CheckedAdd, [a, b]) => op(
                                Op::Gt,
                                vec![op(Op::Add, vec![a.clone(), b.clone()]), integer(255)],
                            ),
                            (PanicPrimitive::U8Division, [_, b]) => eq(b.clone(), integer(0)),
                            (PanicPrimitive::U8Index, [index, length]) => {
                                op(Op::Ge, vec![index.clone(), length.clone()])
                            }
                            _ => bail!("invalid primitive arguments"),
                        };
                        normal = self.event(
                            frame,
                            &site.site_id,
                            &site.context,
                            active,
                            normal,
                            condition,
                        );
                    }
                    PanicSiteSemantics::Unknown { obligation_id } => {
                        self.obligation(
                            PanicUnknownKind::Callee,
                            &site.context,
                            &obligation_id,
                            "unresolved invocation",
                        );
                        normal = and(vec![normal, not(active)]);
                    }
                    _ => {}
                }
            }
            for operation in operations {
                let guard = self.guard(frame, &operation.context.guard)?;
                let active = and(vec![normal.clone(), guard]);
                let condition = match operation.operation {
                    PanicImplicitOperationKind::U8Add {
                        left,
                        right,
                        overflow_checks,
                    } => {
                        ensure!(
                            overflow_checks == self.facts.build.overflow_checks,
                            "addition build mismatch"
                        );
                        if overflow_checks {
                            op(
                                Op::Gt,
                                vec![
                                    op(
                                        Op::Add,
                                        vec![
                                            self.expression(frame, &left)?,
                                            self.expression(frame, &right)?,
                                        ],
                                    ),
                                    integer(255),
                                ],
                            )
                        } else {
                            boolean(false)
                        }
                    }
                    PanicImplicitOperationKind::Index { elements, index } => op(
                        Op::Ge,
                        vec![
                            self.expression(frame, &index)?,
                            integer(elements.len() as i64),
                        ],
                    ),
                };
                normal = self.event(
                    frame,
                    &operation.operation_id,
                    &operation.context,
                    active,
                    normal,
                    condition,
                );
            }
            normal = self.share(normal, PanicSort::Bool);
            frame.exits.insert(flow.flow_id.clone(), normal.clone());
            let returns: Vec<_> = self
                .facts
                .extraction
                .as_ref()
                .map(|e| {
                    e.returns
                        .iter()
                        .filter(|r| r.context.flow_id == flow.flow_id)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            for ret in returns {
                let guard = self.guard(frame, &ret.context.guard)?;
                let term = self.expression(frame, &ret.expression_id)?;
                frame.returns.push((and(vec![normal.clone(), guard]), term));
            }
        }
        Ok(())
    }
    fn event(
        &mut self,
        frame: &Frame,
        id: &str,
        context: &PanicFactContext,
        active: Expr,
        normal: Expr,
        condition: Expr,
    ) -> Expr {
        let predicate = self.share(and(vec![active, condition]), PanicSort::Bool);
        self.events.push(PanicAnalysisEvent {
            event_id: format!("{}:{id}", frame.instance),
            context: context.clone(),
            predicate: predicate.clone(),
        });
        and(vec![normal, not(predicate)])
    }
    fn guard(&mut self, frame: &mut Frame, guard: &PanicGuard) -> anyhow::Result<Expr> {
        Ok(match guard {
            PanicGuard::Always => boolean(true),
            PanicGuard::When {
                expression_id,
                expected,
            } => {
                let term = self.expression(frame, expression_id)?;
                if *expected {
                    term
                } else {
                    not(term)
                }
            }
        })
    }
    fn expression(&mut self, frame: &mut Frame, id: &str) -> anyhow::Result<Expr> {
        if let Some(term) = frame.expressions.get(id) {
            return Ok(term.clone());
        }
        ensure!(
            frame.visiting.insert(format!("e:{id}")),
            "cyclic expression"
        );
        let e = self
            .facts
            .expressions
            .iter()
            .find(|e| e.expression_id == id)
            .context("missing expression")?
            .clone();
        ensure!(
            e.context.location.body_id == frame.body,
            "cross-body expression"
        );
        use PanicExpressionDerivation::*;
        let term = match e.derivation {
            BoolLiteral { value } => boolean(value),
            U8Literal { value } => integer(value.into()),
            Value { value_id } => self.value(frame, &value_id)?,
            Not { operand } => not(self.expression(frame, &operand)?),
            And { left, right } => and(vec![
                self.expression(frame, &left)?,
                self.expression(frame, &right)?,
            ]),
            Or { left, right } => or(vec![
                self.expression(frame, &left)?,
                self.expression(frame, &right)?,
            ]),
            Equal { left, right } => eq(
                self.expression(frame, &left)?,
                self.expression(frame, &right)?,
            ),
            LessThan { left, right } => op(
                Op::Lt,
                vec![
                    self.expression(frame, &left)?,
                    self.expression(frame, &right)?,
                ],
            ),
            U8AddFits { left, right } => op(
                Op::Le,
                vec![
                    op(
                        Op::Add,
                        vec![
                            self.expression(frame, &left)?,
                            self.expression(frame, &right)?,
                        ],
                    ),
                    integer(255),
                ],
            ),
            U8Add { left, right } => {
                let sum = op(
                    Op::Add,
                    vec![
                        self.expression(frame, &left)?,
                        self.expression(frame, &right)?,
                    ],
                );
                self.choose(
                    op(Op::Le, vec![sum.clone(), integer(255)]),
                    sum.clone(),
                    op(Op::Sub, vec![sum, integer(256)]),
                    PanicSort::U8,
                )
            }
            U8ArrayIndex { elements, index } => {
                let index = self.expression(frame, &index)?;
                let mut term = integer(0);
                for (n, value) in elements.into_iter().enumerate().rev() {
                    term = self.choose(
                        eq(index.clone(), integer(n as i64)),
                        integer(value.into()),
                        term,
                        PanicSort::U8,
                    );
                }
                term
            }
        };
        let term = self.share(term, e.sort);
        frame.visiting.remove(&format!("e:{id}"));
        frame.expressions.insert(id.into(), term.clone());
        Ok(term)
    }
    fn value(&mut self, frame: &mut Frame, id: &str) -> anyhow::Result<Expr> {
        if let Some(term) = frame.values.get(id) {
            return Ok(term.clone());
        }
        ensure!(
            frame.visiting.insert(format!("v:{id}")),
            "cyclic value origin"
        );
        let v = self
            .facts
            .values
            .iter()
            .find(|v| v.value_id == id)
            .context("missing value")?
            .clone();
        ensure!(v.context.location.body_id == frame.body, "cross-body value");
        let term = match v.origin {
            PanicValueOrigin::Expression { expression_id } => {
                self.expression(frame, &expression_id)?
            }
            PanicValueOrigin::Assignment { assignment_id } => {
                let a = self
                    .facts
                    .assignments
                    .iter()
                    .find(|a| a.assignment_id == assignment_id)
                    .context("missing assignment")?;
                ensure!(a.target_value_id == id, "assignment target mismatch");
                self.expression(frame, &a.expression_id)?
            }
            PanicValueOrigin::Phi { inputs } => {
                let result = self.variable(v.sort, &v.domain, false);
                let mut alternatives = vec![];
                for input in inputs {
                    let pred = frame
                        .exits
                        .get(&input.predecessor_flow_id)
                        .context("missing phi predecessor")?
                        .clone();
                    let guard = self.guard(frame, &input.guard)?;
                    let value = self.value(frame, &input.value_id)?;
                    alternatives.push(and(vec![pred, guard, eq(result.clone(), value)]));
                }
                let reach = frame
                    .reaches
                    .get(&v.context.flow_id)
                    .cloned()
                    .context("phi context not reached")?;
                self.assert("phi", implies(reach, or(alternatives)));
                result
            }
            PanicValueOrigin::Unknown { obligation_id } => {
                self.obligation(
                    PanicUnknownKind::Opaque,
                    &v.context,
                    &obligation_id,
                    "unknown value origin",
                );
                self.variable(v.sort, &v.domain, false)
            }
            _ => bail!("unbound parameter, loop induction or helper result"),
        };
        frame.visiting.remove(&format!("v:{id}"));
        frame.values.insert(id.into(), term.clone());
        Ok(term)
    }
}

fn contained(inner: [usize; 2], outer: [usize; 2]) -> bool {
    outer[0] <= inner[0] && inner[1] <= outer[1]
}
fn boolean(value: bool) -> Expr {
    Expr::Bool { value }
}
fn integer(value: i64) -> Expr {
    Expr::Int { value }
}
fn not(term: Expr) -> Expr {
    match term {
        Expr::Bool { value } => boolean(!value),
        Expr::Op {
            op: Op::Not,
            mut args,
        } => args.remove(0),
        term => Expr::Op {
            op: Op::Not,
            args: vec![term],
        },
    }
}
fn eq(a: Expr, b: Expr) -> Expr {
    if a == b {
        boolean(true)
    } else {
        op(Op::Eq, vec![a, b])
    }
}
fn implies(a: Expr, b: Expr) -> Expr {
    or(vec![not(a), b])
}
fn and(args: Vec<Expr>) -> Expr {
    junction(Op::And, args)
}
fn or(args: Vec<Expr>) -> Expr {
    junction(Op::Or, args)
}
fn junction(operator: Op, args: Vec<Expr>) -> Expr {
    let identity = operator == Op::And;
    let mut terms = vec![];
    for term in args {
        if let Expr::Bool { value } = &term {
            if *value != identity {
                return boolean(*value);
            }
            continue;
        }
        let children = match term {
            Expr::Op { op, args } if op == operator => args,
            term => vec![term],
        };
        for child in children {
            if terms.contains(&not(child.clone())) {
                return boolean(!identity);
            }
            if !terms.contains(&child) {
                terms.push(child);
            }
        }
    }
    match terms.len() {
        0 => boolean(identity),
        1 => terms.remove(0),
        _ => Expr::Op {
            op: operator,
            args: terms,
        },
    }
}
fn op(operator: Op, args: Vec<Expr>) -> Expr {
    let term = Expr::Op { op: operator, args };
    match evaluate(&term, &BTreeMap::new()) {
        Some(ModelValue::Bool(value)) => boolean(value),
        Some(ModelValue::Int(value)) => integer(value),
        _ => term,
    }
}
fn depth(term: &Expr) -> usize {
    match term {
        Expr::Op { args, .. } => 1 + args.iter().map(depth).max().unwrap_or(0),
        _ => 1,
    }
}

fn validate_model(
    request: &SolveConstraintsRequest,
    model: Option<&SolveModel>,
) -> PanicModelValidation {
    let mut failures = vec![];
    let Some(model) = model else {
        return PanicModelValidation {
            valid: false,
            failures: vec!["SAT missing model".into()],
        };
    };
    for variable in &request.vars {
        let valid = match (variable, model.get(variable.name())) {
            (Variable::Bool { .. }, Some(ModelValue::Bool(_))) => true,
            (Variable::IntRange { min, max, .. }, Some(ModelValue::Int(n))) => min <= n && n <= max,
            _ => false,
        };
        if !valid {
            failures.push(format!("invalid/missing domain value {}", variable.name()));
        }
    }
    for (index, c) in request.constraints.iter().enumerate() {
        if evaluate(c.expr(), model) != Some(ModelValue::Bool(true)) {
            failures.push(format!(
                "constraint {} is false or unevaluable",
                c.id()
                    .map(str::to_owned)
                    .unwrap_or_else(|| index.to_string())
            ));
        }
    }
    PanicModelValidation {
        valid: failures.is_empty(),
        failures,
    }
}

/// Independent exact scalar evaluator for the emitted Int/Bool subset. Any
/// unsupported sort/operator, missing value, or arithmetic overflow fails closed.
fn evaluate(term: &Expr, model: &SolveModel) -> Option<ModelValue> {
    let (operator, args) = match term {
        Expr::Var { name } => return model.get(name).cloned(),
        Expr::Int { value } => return Some(ModelValue::Int(*value)),
        Expr::Bool { value } => return Some(ModelValue::Bool(*value)),
        Expr::Op { op, args } => (*op, args),
        _ => return None,
    };
    let values = args
        .iter()
        .map(|a| evaluate(a, model))
        .collect::<Option<Vec<_>>>()?;
    let ints = || {
        values
            .iter()
            .map(|v| {
                if let ModelValue::Int(n) = v {
                    Some(*n)
                } else {
                    None
                }
            })
            .collect::<Option<Vec<_>>>()
    };
    let bools = || {
        values
            .iter()
            .map(|v| {
                if let ModelValue::Bool(b) = v {
                    Some(*b)
                } else {
                    None
                }
            })
            .collect::<Option<Vec<_>>>()
    };
    Some(match operator {
        Op::And => ModelValue::Bool(bools()?.into_iter().all(|x| x)),
        Op::Or => ModelValue::Bool(bools()?.into_iter().any(|x| x)),
        Op::Not => ModelValue::Bool(!bools()?[0]),
        Op::Eq => ModelValue::Bool(values[0] == values[1]),
        Op::Ne => ModelValue::Bool(values[0] != values[1]),
        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let n = ints()?;
            ModelValue::Bool(match operator {
                Op::Lt => n[0] < n[1],
                Op::Le => n[0] <= n[1],
                Op::Gt => n[0] > n[1],
                _ => n[0] >= n[1],
            })
        }
        Op::Add => ModelValue::Int(
            ints()?
                .into_iter()
                .try_fold(0i64, |a, b| a.checked_add(b))?,
        ),
        Op::Sub => {
            let n = ints()?;
            ModelValue::Int(n[0].checked_sub(n[1])?)
        }
        _ => return None,
    })
}
