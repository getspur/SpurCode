//! Typed, bounded semantic evidence, separate from graph schema v11.
//!
//! Validation establishes source identity and structural consistency of a
//! supplied translation. It does not prove that translation faithful to Rust.
//! A canonical producer and a consumer's independently expected build identity
//! are still required. Missing information must use an explicit obligation.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    PanicEvidenceSource, RustBodyEvidence, RustCallSiteEvidenceSet, RustCallSourceBinding,
};
use crate::GraphIndexArtifact;

pub const PANIC_BUILD_CONFIG_SCHEMA_VERSION: &str = "panic-build-config-v1";
pub const PANIC_SEMANTIC_FACTS_SCHEMA_VERSION: &str = "panic-semantic-facts-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanicRustEdition {
    #[serde(rename = "2015")]
    Rust2015,
    #[serde(rename = "2018")]
    Rust2018,
    #[serde(rename = "2021")]
    Rust2021,
    #[serde(rename = "2024")]
    Rust2024,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicStrategy {
    Unwind,
    Abort,
}

/// One enabled cfg atom. A bare atom has an explicit null value; a disabled
/// atom is absent from this complete, producer-supplied cfg inventory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicCfgAtom {
    pub key: String,
    pub value: Option<String>,
}

/// Explicit semantic build inputs. No ambient defaults are inferred. cfg must
/// include target/compiler cfg as well as custom atoms, and flags preserve order.
/// This records a build declaration, not attestation that a compiler ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicBuildConfig {
    pub schema_version: String,
    pub graph_content_hash: String,
    pub target: String,
    pub edition: PanicRustEdition,
    pub cfg: Vec<PanicCfgAtom>,
    pub overflow_checks: bool,
    pub debug_assertions: bool,
    pub panic_strategy: PanicStrategy,
    pub rustc_verbose: String,
    pub codegen_flags: Vec<String>,
    pub features: Vec<String>,
}

impl PanicBuildConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == PANIC_BUILD_CONFIG_SCHEMA_VERSION,
            "unsupported panic build version"
        );
        ensure!(
            !self.graph_content_hash.is_empty()
                && !self.target.trim().is_empty()
                && !self.rustc_verbose.trim().is_empty(),
            "missing explicit build identity"
        );
        ensure!(
            strictly_sorted(&self.cfg) && self.cfg.iter().all(|a| !a.key.trim().is_empty()),
            "cfg atoms must be unique, ordered and named"
        );
        ensure!(
            strictly_sorted(&self.features) && self.features.iter().all(|s| !s.trim().is_empty()),
            "features must be unique, ordered and named"
        );
        ensure!(
            self.codegen_flags.iter().all(|s| !s.trim().is_empty()),
            "empty codegen flag"
        );
        Ok(())
    }

    pub fn content_hash(&self) -> anyhow::Result<String> {
        self.validate()?;
        Ok(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(self)?)
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicSort {
    Bool,
    U8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanicValueDomain {
    Bool {
        values: Vec<bool>,
    },
    /// Wider storage deliberately permits rejecting malformed u8 ranges.
    U8 {
        minimum: u16,
        maximum: u16,
    },
}

impl PanicValueDomain {
    fn validate(&self, sort: PanicSort) -> anyhow::Result<()> {
        match (self, sort) {
            (Self::Bool { values }, PanicSort::Bool) => ensure!(
                !values.is_empty() && strictly_sorted(values),
                "empty or unordered bool domain"
            ),
            (Self::U8 { minimum, maximum }, PanicSort::U8) => {
                ensure!(minimum <= maximum && *maximum <= 255, "invalid u8 domain")
            }
            _ => anyhow::bail!("domain sort mismatch"),
        }
        Ok(())
    }

    fn contains(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool { values: a }, Self::Bool { values: b }) => b.iter().all(|v| a.contains(v)),
            (
                Self::U8 {
                    minimum: a,
                    maximum: b,
                },
                Self::U8 {
                    minimum: c,
                    maximum: d,
                },
            ) => a <= c && d <= b,
            _ => false,
        }
    }
}

/// Half-open UTF-8 bytes of the derivation/use within its innermost R3 body.
/// For a parameter origin this is the in-body use, not the signature span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicFactLocation {
    pub body_id: String,
    pub file_path: String,
    pub source_oid: String,
    pub byte_range: [usize; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanicGuard {
    Always,
    When {
        expression_id: String,
        expected: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicFactContext {
    pub location: PanicFactLocation,
    pub flow_id: String,
    pub guard: PanicGuard,
}

/// Explicit evaluation order, independent of source order and derivation IDs.
/// Predecessors form an unrolled DAG; deferred bodies have separate entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicControlFlow {
    pub flow_id: String,
    pub location: PanicFactLocation,
    pub order: u32,
    pub guard: PanicGuard,
    pub predecessors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanicExpressionDerivation {
    BoolLiteral {
        value: bool,
    },
    U8Literal {
        value: u8,
    },
    Value {
        value_id: String,
    },
    Not {
        operand: String,
    },
    And {
        left: String,
        right: String,
    },
    Or {
        left: String,
        right: String,
    },
    Equal {
        left: String,
        right: String,
    },
    LessThan {
        left: String,
        right: String,
    },
    /// Arithmetic semantics depend on the bound build's overflow setting.
    U8Add {
        left: String,
        right: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicTypedExpression {
    pub expression_id: String,
    pub sort: PanicSort,
    pub domain: PanicValueDomain,
    pub context: PanicFactContext,
    pub derivation: PanicExpressionDerivation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicPhiInput {
    pub predecessor_flow_id: String,
    pub guard: PanicGuard,
    pub value_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanicValueOrigin {
    Parameter { index: u32 },
    Expression { expression_id: String },
    Assignment { assignment_id: String },
    Phi { inputs: Vec<PanicPhiInput> },
    Unknown { obligation_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicValueVersion {
    pub value_id: String,
    /// Opaque lexical binding identity, not a name-based alias assertion.
    pub binding_id: String,
    pub version: u32,
    pub sort: PanicSort,
    pub domain: PanicValueDomain,
    pub context: PanicFactContext,
    pub origin: PanicValueOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicGuardedAssignment {
    pub assignment_id: String,
    pub context: PanicFactContext,
    pub target_value_id: String,
    pub expression_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicPrimitive {
    AssertBool,
    U8CheckedAdd,
    U8Division,
    U8Index,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicAuthorityKind {
    BoundedSourceModel,
    CompilerObservation,
}

/// Retained derivation authority, never inferred from a callee's spelling.
/// The producer/consumer must authenticate the referenced payload separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicContractAuthority {
    pub kind: PanicAuthorityKind,
    pub reference: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicPrimitiveContract {
    pub contract_id: String,
    pub primitive: PanicPrimitive,
    pub build_hash: String,
    pub authority: PanicContractAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanicSiteSemantics {
    Primitive {
        contract_id: String,
        arguments: Vec<String>,
    },
    Unknown {
        obligation_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicSemanticSite {
    pub site_id: String,
    pub context: PanicFactContext,
    pub semantics: PanicSiteSemantics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicUnknownKind {
    Opaque,
    Mutation,
    Cfg,
    Parse,
    Callee,
    CallDepth,
    LoopBound,
    StateBudget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicUnknownObligation {
    pub obligation_id: String,
    pub kind: PanicUnknownKind,
    pub location: PanicFactLocation,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicBodyCoverage {
    pub body_id: String,
    pub entry_flow_id: String,
    /// Exact inventory of all retained obligations in this body.
    pub unknown_obligations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicFrontierKind {
    CallDepth,
    LoopBound,
    StateBudget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicExplorationFrontier {
    pub frontier_id: String,
    pub kind: PanicFrontierKind,
    pub context: PanicFactContext,
    pub obligation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicExplorationLimits {
    /// Zero is meaningful: entering a callee then requires an open frontier.
    pub max_call_depth: u32,
    pub max_loop_iterations: u32,
    pub max_states: u32,
    pub frontiers: Vec<PanicExplorationFrontier>,
}

/// All identity-indexed vectors are strictly sorted by their ID, except bodies
/// and sources (R3 canonical order) and flow (body ID, evaluation order).
/// JSON has no defaults for facts, origins, domains or uncertainty inventories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicSemanticFacts {
    pub schema_version: String,
    pub graph_content_hash: String,
    pub build: PanicBuildConfig,
    pub build_hash: String,
    pub sources: Vec<RustCallSourceBinding>,
    pub bodies: Vec<RustBodyEvidence>,
    pub body_coverage: Vec<PanicBodyCoverage>,
    pub values: Vec<PanicValueVersion>,
    pub expressions: Vec<PanicTypedExpression>,
    pub control_flow: Vec<PanicControlFlow>,
    pub assignments: Vec<PanicGuardedAssignment>,
    pub sites: Vec<PanicSemanticSite>,
    pub primitive_contracts: Vec<PanicPrimitiveContract>,
    pub unknown_obligations: Vec<PanicUnknownObligation>,
    pub exploration: PanicExplorationLimits,
}

impl PanicSemanticFacts {
    /// Independently pin the intended build. `validate_against` alone detects
    /// inconsistent/stale hashes, not an attacker rehashing a replacement build.
    pub fn validate_for_build(&self, expected: &PanicBuildConfig) -> anyhow::Result<()> {
        ensure!(
            self.build == *expected && self.build_hash == expected.content_hash()?,
            "semantic facts do not bind expected build"
        );
        Ok(())
    }

    pub fn validate_against(
        &self,
        graph: &GraphIndexArtifact,
        sources: &[PanicEvidenceSource<'_>],
        call_sites: &RustCallSiteEvidenceSet,
    ) -> anyhow::Result<()> {
        call_sites.validate(graph, sources)?;
        ensure!(
            self.schema_version == PANIC_SEMANTIC_FACTS_SCHEMA_VERSION,
            "unsupported panic semantic version"
        );
        ensure!(
            self.graph_content_hash == graph.graph_content_hash
                && self.build.graph_content_hash == graph.graph_content_hash,
            "stale semantic graph binding"
        );
        ensure!(
            self.build_hash == self.build.content_hash()?,
            "stale semantic build hash"
        );
        ensure!(
            self.sources == call_sites.sources && self.bodies == call_sites.bodies,
            "semantic source/body inventory differs from canonical evidence"
        );
        ensure!(
            self.exploration.max_states > 0,
            "state budget must be positive"
        );
        Validator::new(self, call_sites, sources)?.validate()
    }

    /// No unknown obligations or open frontiers in the declared finite model.
    /// Success is structural completeness only: it is not a safety result,
    /// source-translation proof, or a claim of generic Rust semantic coverage.
    pub fn validate_complete_against(
        &self,
        graph: &GraphIndexArtifact,
        sources: &[PanicEvidenceSource<'_>],
        call_sites: &RustCallSiteEvidenceSet,
    ) -> anyhow::Result<()> {
        self.validate_against(graph, sources, call_sites)?;
        ensure!(
            self.unknown_obligations.is_empty() && self.exploration.frontiers.is_empty(),
            "unknown obligations or exploration frontiers prevent complete evidence"
        );
        Ok(())
    }
}

fn strictly_sorted<T: Ord>(rows: &[T]) -> bool {
    rows.windows(2).all(|w| w[0] < w[1])
}

fn index<'a, T>(
    rows: &'a [T],
    id: impl Fn(&'a T) -> &'a str,
) -> anyhow::Result<BTreeMap<&'a str, &'a T>> {
    let ids: Vec<_> = rows.iter().map(&id).collect();
    ensure!(
        ids.iter().all(|s| !s.trim().is_empty()) && strictly_sorted(&ids),
        "identities must be present, unique and ordered"
    );
    Ok(rows.iter().map(|r| (id(r), r)).collect())
}

struct Validator<'a> {
    facts: &'a PanicSemanticFacts,
    calls: &'a RustCallSiteEvidenceSet,
    sources: &'a [PanicEvidenceSource<'a>],
    values: BTreeMap<&'a str, &'a PanicValueVersion>,
    expressions: BTreeMap<&'a str, &'a PanicTypedExpression>,
    flow: BTreeMap<&'a str, &'a PanicControlFlow>,
    assignments: BTreeMap<&'a str, &'a PanicGuardedAssignment>,
    unknowns: BTreeMap<&'a str, &'a PanicUnknownObligation>,
    dominators: BTreeMap<&'a str, BTreeSet<&'a str>>,
}

impl<'a> Validator<'a> {
    fn new(
        facts: &'a PanicSemanticFacts,
        calls: &'a RustCallSiteEvidenceSet,
        sources: &'a [PanicEvidenceSource<'a>],
    ) -> anyhow::Result<Self> {
        let flow: BTreeMap<_, _> = facts
            .control_flow
            .iter()
            .map(|f| (f.flow_id.as_str(), f))
            .collect();
        ensure!(
            flow.len() == facts.control_flow.len() && flow.keys().all(|s| !s.trim().is_empty()),
            "duplicate or missing flow identity"
        );
        Ok(Self {
            facts,
            calls,
            sources,
            values: index(&facts.values, |v| &v.value_id)?,
            expressions: index(&facts.expressions, |e| &e.expression_id)?,
            flow,
            assignments: index(&facts.assignments, |a| &a.assignment_id)?,
            unknowns: index(&facts.unknown_obligations, |u| &u.obligation_id)?,
            dominators: BTreeMap::new(),
        })
    }

    fn location(&self, location: &PanicFactLocation) -> anyhow::Result<()> {
        let source = self
            .sources
            .iter()
            .find(|s| s.file_path == location.file_path)
            .context("detached source span")?;
        let binding = self
            .facts
            .sources
            .iter()
            .find(|s| s.file_path == location.file_path)
            .context("missing source binding")?;
        ensure!(
            location.source_oid == binding.source_oid,
            "stale source span"
        );
        let [start, end] = location.byte_range;
        let text = std::str::from_utf8(source.bytes)?;
        ensure!(
            start < end
                && end <= text.len()
                && text.is_char_boundary(start)
                && text.is_char_boundary(end),
            "invalid UTF-8 span"
        );
        let owner = self
            .facts
            .bodies
            .iter()
            .filter(|b| {
                b.file_path == location.file_path
                    && b.source_oid == location.source_oid
                    && b.byte_range[0] <= start
                    && end <= b.byte_range[1]
            })
            .min_by_key(|b| b.byte_range[1] - b.byte_range[0])
            .context("span outside executable body")?;
        ensure!(
            owner.body_id == location.body_id,
            "span has wrong existing body"
        );
        Ok(())
    }

    fn available(&self, definition: &str, usage: &str, strict: bool) -> anyhow::Result<()> {
        ensure!(
            (!strict || definition != usage)
                && self
                    .dominators
                    .get(usage)
                    .is_some_and(|d| d.contains(definition)),
            "derivation does not dominate use"
        );
        Ok(())
    }

    fn guard(&self, guard: &PanicGuard, flow_id: &str) -> anyhow::Result<()> {
        if let PanicGuard::When { expression_id, .. } = guard {
            let expr = self
                .expressions
                .get(expression_id.as_str())
                .context("detached guard expression")?;
            ensure!(expr.sort == PanicSort::Bool, "guard is not boolean");
            self.available(&expr.context.flow_id, flow_id, true)?;
        }
        Ok(())
    }

    fn context(&self, context: &PanicFactContext) -> anyhow::Result<()> {
        self.location(&context.location)?;
        let flow = self
            .flow
            .get(context.flow_id.as_str())
            .context("detached flow context")?;
        ensure!(
            flow.location.body_id == context.location.body_id && flow.guard == context.guard,
            "detached guard or wrong flow body"
        );
        self.guard(&context.guard, &context.flow_id)
    }

    fn obligation(
        &self,
        id: &str,
        location: &PanicFactLocation,
    ) -> anyhow::Result<&'a PanicUnknownObligation> {
        let unknown = *self
            .unknowns
            .get(id)
            .context("missing unknown obligation")?;
        ensure!(
            unknown.location.body_id == location.body_id,
            "unknown obligation belongs to another body"
        );
        Ok(unknown)
    }

    fn expression(
        &self,
        id: &str,
        context: &PanicFactContext,
    ) -> anyhow::Result<&'a PanicTypedExpression> {
        let expr = *self
            .expressions
            .get(id)
            .context("missing typed expression")?;
        self.available(&expr.context.flow_id, &context.flow_id, false)?;
        Ok(expr)
    }

    fn validate(mut self) -> anyhow::Result<()> {
        let coverage = index(&self.facts.body_coverage, |b| &b.body_id)?;
        ensure!(
            coverage.len() == self.facts.bodies.len()
                && self
                    .facts
                    .bodies
                    .iter()
                    .all(|b| coverage.contains_key(b.body_id.as_str())),
            "missing body coverage"
        );
        let ordered: Vec<_> = self
            .facts
            .control_flow
            .iter()
            .map(|f| (&f.location.body_id, f.order))
            .collect();
        ensure!(
            strictly_sorted(&ordered),
            "flow must be ordered by body and evaluation order"
        );
        for flow in &self.facts.control_flow {
            self.location(&flow.location)?;
            ensure!(
                strictly_sorted(&flow.predecessors),
                "flow predecessors must be unique and ordered"
            );
            let body = coverage
                .get(flow.location.body_id.as_str())
                .context("flow has no body coverage")?;
            let mut dom: Option<BTreeSet<&str>> = None;
            for pred in &flow.predecessors {
                let parent = self
                    .flow
                    .get(pred.as_str())
                    .context("missing flow predecessor")?;
                ensure!(
                    parent.location.body_id == flow.location.body_id && parent.order < flow.order,
                    "flow cycle, ordering or body mismatch"
                );
                let pd = self
                    .dominators
                    .get(pred.as_str())
                    .context("predecessor not visited")?;
                dom = Some(match dom {
                    None => pd.clone(),
                    Some(d) => d.intersection(pd).copied().collect(),
                });
            }
            ensure!(
                if flow.flow_id == body.entry_flow_id {
                    flow.order == 0
                        && flow.predecessors.is_empty()
                        && flow.guard == PanicGuard::Always
                } else {
                    !flow.predecessors.is_empty()
                },
                "missing entry or detached flow"
            );
            let mut dom = dom.unwrap_or_default();
            dom.insert(&flow.flow_id);
            self.dominators.insert(&flow.flow_id, dom);
        }
        for body in coverage.values() {
            let entry = self
                .flow
                .get(body.entry_flow_id.as_str())
                .context("missing body entry")?;
            ensure!(
                entry.location.body_id == body.body_id && entry.order == 0,
                "wrong body entry"
            );
            let expected: Vec<_> = self
                .facts
                .unknown_obligations
                .iter()
                .filter(|u| u.location.body_id == body.body_id)
                .map(|u| u.obligation_id.clone())
                .collect();
            ensure!(
                body.unknown_obligations == expected,
                "body uncertainty inventory mismatch"
            );
        }
        for flow in &self.facts.control_flow {
            self.guard(&flow.guard, &flow.flow_id)?;
        }
        for unknown in self.unknowns.values() {
            self.location(&unknown.location)?;
            ensure!(
                !unknown.detail.trim().is_empty(),
                "unknown requires an explicit explanation"
            );
        }
        for expr in self.expressions.values() {
            self.validate_expression(expr)?;
        }
        let mut versions = BTreeSet::new();
        let mut binding_sorts = BTreeMap::new();
        let mut parameters = BTreeSet::new();
        for value in self.values.values() {
            self.context(&value.context)?;
            value.domain.validate(value.sort)?;
            ensure!(
                !value.binding_id.trim().is_empty()
                    && versions.insert((
                        &value.context.location.body_id,
                        &value.binding_id,
                        value.version
                    )),
                "missing binding or reused value version"
            );
            ensure!(
                *binding_sorts
                    .entry((&value.context.location.body_id, &value.binding_id))
                    .or_insert(value.sort)
                    == value.sort,
                "inconsistent binding sort across value versions"
            );
            match &value.origin {
                PanicValueOrigin::Parameter { index } => ensure!(
                    value.version == 0
                        && parameters.insert((&value.context.location.body_id, index)),
                    "invalid or reused parameter origin"
                ),
                PanicValueOrigin::Expression { expression_id } => {
                    let expr = self.expression(expression_id, &value.context)?;
                    ensure!(
                        expr.sort == value.sort && value.domain.contains(&expr.domain),
                        "origin sort/range mismatch"
                    );
                }
                PanicValueOrigin::Assignment { assignment_id } => {
                    let assignment = self
                        .assignments
                        .get(assignment_id.as_str())
                        .context("missing assignment origin")?;
                    ensure!(
                        assignment.target_value_id == value.value_id
                            && assignment.context == value.context,
                        "assignment origin not attached to target"
                    );
                }
                PanicValueOrigin::Phi { inputs } => {
                    let flow = self.flow[value.context.flow_id.as_str()];
                    ensure!(
                        !inputs.is_empty()
                            && inputs
                                .iter()
                                .map(|i| &i.predecessor_flow_id)
                                .eq(flow.predecessors.iter()),
                        "phi must cover exact ordered predecessor set"
                    );
                    for input in inputs {
                        let pred = self.flow[input.predecessor_flow_id.as_str()];
                        ensure!(input.guard == pred.guard, "detached phi guard");
                        let incoming = self
                            .values
                            .get(input.value_id.as_str())
                            .context("missing phi value")?;
                        ensure!(
                            incoming.binding_id == value.binding_id
                                && incoming.version < value.version
                                && incoming.sort == value.sort
                                && value.domain.contains(&incoming.domain),
                            "phi origin version/sort/range mismatch"
                        );
                        self.available(&incoming.context.flow_id, &pred.flow_id, false)?;
                    }
                }
                PanicValueOrigin::Unknown { obligation_id } => {
                    self.obligation(obligation_id, &value.context.location)?;
                }
            }
        }
        for assignment in self.assignments.values() {
            self.context(&assignment.context)?;
            let target = self
                .values
                .get(assignment.target_value_id.as_str())
                .context("missing assignment target")?;
            let expr = self.expression(&assignment.expression_id, &assignment.context)?;
            ensure!(
                matches!(&target.origin, PanicValueOrigin::Assignment { assignment_id } if assignment_id == &assignment.assignment_id)
                    && target.context == assignment.context
                    && target.version > 0
                    && target.sort == expr.sort
                    && target.domain.contains(&expr.domain),
                "inconsistent guarded assignment"
            );
        }
        self.validate_dependencies()?;
        self.validate_sites()?;
        let frontiers = index(&self.facts.exploration.frontiers, |f| &f.frontier_id)?;
        for frontier in frontiers.values() {
            self.context(&frontier.context)?;
            let unknown = self.obligation(&frontier.obligation_id, &frontier.context.location)?;
            ensure!(
                matches!(
                    (frontier.kind, unknown.kind),
                    (PanicFrontierKind::CallDepth, PanicUnknownKind::CallDepth)
                        | (PanicFrontierKind::LoopBound, PanicUnknownKind::LoopBound)
                        | (
                            PanicFrontierKind::StateBudget,
                            PanicUnknownKind::StateBudget
                        )
                ),
                "frontier obligation kind mismatch"
            );
        }
        Ok(())
    }

    fn validate_expression(&self, expr: &PanicTypedExpression) -> anyhow::Result<()> {
        self.context(&expr.context)?;
        expr.domain.validate(expr.sort)?;
        use PanicExpressionDerivation::*;
        match &expr.derivation {
            BoolLiteral { value } => ensure!(
                expr.sort == PanicSort::Bool
                    && expr.domain
                        == PanicValueDomain::Bool {
                            values: vec![*value]
                        },
                "bool literal domain mismatch"
            ),
            U8Literal { value } => ensure!(
                expr.sort == PanicSort::U8
                    && expr.domain
                        == PanicValueDomain::U8 {
                            minimum: u16::from(*value),
                            maximum: u16::from(*value)
                        },
                "u8 literal domain mismatch"
            ),
            Value { value_id } => {
                let value = self
                    .values
                    .get(value_id.as_str())
                    .context("missing expression value")?;
                self.available(&value.context.flow_id, &expr.context.flow_id, false)?;
                ensure!(
                    expr.sort == value.sort && expr.domain.contains(&value.domain),
                    "value read sort/range mismatch"
                );
            }
            Not { operand } => ensure!(
                expr.sort == PanicSort::Bool
                    && self.expression(operand, &expr.context)?.sort == PanicSort::Bool,
                "not sort mismatch"
            ),
            And { left, right }
            | Or { left, right }
            | Equal { left, right }
            | LessThan { left, right }
            | U8Add { left, right } => {
                let a = self.expression(left, &expr.context)?;
                let b = self.expression(right, &expr.context)?;
                let valid = match &expr.derivation {
                    And { .. } | Or { .. } => {
                        expr.sort == PanicSort::Bool
                            && a.sort == PanicSort::Bool
                            && b.sort == PanicSort::Bool
                    }
                    Equal { .. } => expr.sort == PanicSort::Bool && a.sort == b.sort,
                    LessThan { .. } => {
                        expr.sort == PanicSort::Bool
                            && a.sort == PanicSort::U8
                            && b.sort == PanicSort::U8
                    }
                    U8Add { .. } => {
                        expr.sort == PanicSort::U8
                            && a.sort == PanicSort::U8
                            && b.sort == PanicSort::U8
                    }
                    _ => false,
                };
                ensure!(valid, "operator sort mismatch");
            }
        }
        Ok(())
    }

    fn validate_sites(&self) -> anyhow::Result<()> {
        let contracts = index(&self.facts.primitive_contracts, |c| &c.contract_id)?;
        for c in contracts.values() {
            ensure!(
                c.build_hash == self.facts.build_hash
                    && !c.authority.reference.trim().is_empty()
                    && c.authority.sha256.len() == 64
                    && c.authority
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "missing or stale primitive contract authority"
            );
        }
        let sites = index(&self.facts.sites, |s| &s.site_id)?;
        ensure!(
            sites.len() == self.calls.sites.len(),
            "missing invocation semantics"
        );
        for source_site in &self.calls.sites {
            let site = sites
                .get(source_site.site_id.as_str())
                .context("missing canonical invocation")?;
            let loc = &site.context.location;
            ensure!(
                loc.body_id == source_site.body_id
                    && loc.file_path == source_site.file_path
                    && loc.source_oid == source_site.source_oid
                    && loc.byte_range == source_site.byte_range,
                "invocation exact source/body/span tuple mismatch"
            );
            self.context(&site.context)?;
            match &site.semantics {
                PanicSiteSemantics::Unknown { obligation_id } => {
                    self.obligation(obligation_id, loc)?;
                }
                PanicSiteSemantics::Primitive {
                    contract_id,
                    arguments,
                } => {
                    let contract = contracts
                        .get(contract_id.as_str())
                        .context("missing primitive contract")?;
                    let (arity, sort) = match contract.primitive {
                        PanicPrimitive::AssertBool => (1, PanicSort::Bool),
                        _ => (2, PanicSort::U8),
                    };
                    ensure!(
                        arguments.len() == arity,
                        "primitive argument arity mismatch"
                    );
                    for arg in arguments {
                        ensure!(
                            self.expression(arg, &site.context)?.sort == sort,
                            "primitive argument sort mismatch"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// A finite translation cannot bootstrap an origin by reading itself.
    /// Iterative Kahn traversal avoids recursion on untrusted JSON.
    fn validate_dependencies(&self) -> anyhow::Result<()> {
        let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for expr in self.expressions.values() {
            use PanicExpressionDerivation::*;
            let refs = match &expr.derivation {
                Value { value_id } => vec![format!("v:{value_id}")],
                Not { operand } => vec![format!("e:{operand}")],
                And { left, right }
                | Or { left, right }
                | Equal { left, right }
                | LessThan { left, right }
                | U8Add { left, right } => vec![format!("e:{left}"), format!("e:{right}")],
                _ => vec![],
            };
            deps.insert(
                format!("e:{}", expr.expression_id),
                refs.into_iter().collect(),
            );
        }
        for value in self.values.values() {
            let refs = match &value.origin {
                PanicValueOrigin::Expression { expression_id } => {
                    vec![format!("e:{expression_id}")]
                }
                PanicValueOrigin::Assignment { assignment_id } => vec![format!(
                    "e:{}",
                    self.assignments[assignment_id.as_str()].expression_id
                )],
                PanicValueOrigin::Phi { inputs } => {
                    inputs.iter().map(|i| format!("v:{}", i.value_id)).collect()
                }
                _ => vec![],
            };
            deps.insert(format!("v:{}", value.value_id), refs.into_iter().collect());
        }
        while !deps.is_empty() {
            let ready: BTreeSet<_> = deps
                .iter()
                .filter(|(_, d)| d.is_empty())
                .map(|(id, _)| id.clone())
                .collect();
            ensure!(!ready.is_empty(), "cyclic semantic derivation");
            deps.retain(|id, _| !ready.contains(id));
            for d in deps.values_mut() {
                d.retain(|id| !ready.contains(id));
            }
        }
        Ok(())
    }
}
