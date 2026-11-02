//! Conservative AST translation for a declared bool/u8 source model.
//! No compiler name lookup, macro expansion or memory alias analysis is claimed.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context as _};
use tree_sitter::{Node, Parser};

use super::*;
use crate::GraphIndexArtifact;

/// Collection cannot attach a body-local obligation when R3 cannot establish
/// an exact body inventory (notably cfg or parse uncertainty). Preserve a typed
/// source-level obligation in the Result error instead of inventing an owner.
#[derive(Debug)]
pub struct PanicCollectionObligation {
    pub kind: PanicUnknownKind,
    pub detail: String,
}

impl std::fmt::Display for PanicCollectionObligation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsupported semantic collection ({:?}): {}",
            self.kind, self.detail
        )
    }
}
impl std::error::Error for PanicCollectionObligation {}

/// Collect facts from source, never from fixture names or supplied conclusions.
/// The build is a required declaration, not evidence that rustc was executed.
pub fn collect_panic_semantics(
    graph: &GraphIndexArtifact,
    sources: &[PanicEvidenceSource<'_>],
    build: &PanicBuildConfig,
) -> anyhow::Result<PanicSemanticFacts> {
    build.validate()?;
    ensure!(
        build.graph_content_hash == graph.graph_content_hash,
        "build/graph mismatch"
    );
    let calls = collect_rust_call_sites(graph, sources).map_err(|error| {
        let detail = format!("{error:#}");
        if detail.contains("cfg") || detail.contains("parse") {
            anyhow::Error::new(PanicCollectionObligation {
                kind: if detail.contains("cfg") {
                    PanicUnknownKind::Cfg
                } else {
                    PanicUnknownKind::Parse
                },
                detail,
            })
        } else {
            error
        }
    })?;
    let mut facts = PanicSemanticFacts {
        schema_version: PANIC_SEMANTIC_FACTS_SCHEMA_VERSION.into(),
        graph_content_hash: graph.graph_content_hash.clone(),
        build: build.clone(),
        build_hash: build.content_hash()?,
        sources: calls.sources.clone(),
        bodies: calls.bodies.clone(),
        body_coverage: vec![],
        values: vec![],
        expressions: vec![],
        control_flow: vec![],
        assignments: vec![],
        sites: vec![],
        primitive_contracts: vec![],
        unknown_obligations: vec![],
        // No exploration is performed by collection. R6 supplies caller limits.
        exploration: PanicExplorationLimits {
            max_call_depth: 0,
            max_loop_iterations: 0,
            max_states: 1,
            frontiers: vec![],
        },
        extraction: Some(PanicExtractedSemantics {
            extractor_version: PANIC_EXTRACTION_VERSION.into(),
            returns: vec![],
            operations: vec![],
            loops: vec![],
        }),
    };
    for source in sources {
        let text = std::str::from_utf8(source.bytes)?;
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
        let tree = parser
            .parse(text, None)
            .context("semantic parser unavailable")?;
        let root = tree.root_node();
        let nodes = descendants(root);
        // This is deliberately a closed, single crate-root source lane. Imports,
        // modules, attributes and any other expansion can introduce competing
        // macro/type definitions outside the physical AST. Absence of a visible
        // `macro_rules! assert` alone is insufficient authority.
        let assert_authority = sources.len() == 1
            && matches!(source.file_path, "src/lib.rs" | "src/main.rs" | "lib.rs" | "main.rs")
            && graph.files.iter().filter(|f| f.file_path.ends_with(".rs")).count() == 1
            && build.codegen_flags.is_empty()
            && build.features.is_empty()
            && !text.contains("no_implicit_prelude")
            && !nodes.iter().any(|n| match n.kind() {
                "use_declaration" | "extern_crate_declaration" | "macro_definition" | "mod_item" => true,
                "attribute_item" | "inner_attribute_item" => !benign_attribute(*n, text),
                "macro_invocation" => assertion_syntax(*n, text).is_none(),
                _ => false,
            });
        let primitive_types = !nodes.iter().any(|n| {
            matches!(
                n.kind(),
                "use_declaration"
                    | "extern_crate_declaration"
                    | "macro_definition"
            ) || (n.kind() == "macro_invocation" && !assert_authority) || (matches!(
                n.kind(),
                "struct_item" | "enum_item" | "union_item" | "type_item" | "mod_item" | "trait_item"
            ) && field_text(*n, "name", text)
                .is_some_and(|s| matches!(ident(s), "bool" | "u8" | "usize")))
        });
        let mut helpers = BTreeMap::new();
        for function in nodes.iter().filter(|n| {
            n.kind() == "function_item" && n.parent().is_some_and(|p| p.kind() == "source_file")
        }) {
            let Some(body) = function.child_by_field_name("body") else {
                continue;
            };
            let Some(owner) = calls
                .bodies
                .iter()
                .find(|b| b.file_path == source.file_path && b.byte_range == range(body))
            else {
                continue;
            };
            if let Some(signature) = signature(*function, text) {
                let name = ident(field_text(*function, "name", text).unwrap_or("")).to_owned();
                helpers.entry(name).or_insert_with(Vec::new).push(Helper {
                    body_id: owner.body_id.clone(),
                    parameters: signature.0,
                    result: signature.1,
                });
            }
        }
        // Conservatively disable implicit prelude spelling in the presence of
        // imports/macros or local items that could alter constructor lookup.
        let prelude = !nodes.iter().any(|n| {
            matches!(
                n.kind(),
                "use_declaration" | "extern_crate_declaration" | "macro_definition"
            ) || (matches!(
                n.kind(),
                "function_item"
                    | "struct_item"
                    | "enum_item"
                    | "mod_item"
                    | "type_item"
                    | "const_item"
                    | "static_item"
            ) && field_text(*n, "name", text)
                .is_some_and(|s| matches!(ident(s), "Some" | "None" | "Option" | "std" | "core")))
        }) && !text.contains("no_implicit_prelude");
        // is_some needs autoref. A trait method taking Option by value can
        // win before its inherent &self method. This source lane does not
        // resolve trait applicability, imports, defaults or blanket impls.
        let option_is_some_authoritative = !nodes.iter().any(|n| {
            n.kind() == "trait_item"
                || (n.kind() == "impl_item" && n.child_by_field_name("trait").is_some())
        });
        let bodies: Vec<_> = calls
            .bodies
            .iter()
            .filter(|b| b.file_path == source.file_path)
            .cloned()
            .collect();
        for body in bodies {
            let node = nodes
                .iter()
                .find(|n| range(**n) == body.byte_range)
                .copied()
                .context("R3 body AST missing")?;
            let mut collector = Collector::new(
                &mut facts,
                &calls,
                &body,
                text,
                &helpers,
                prelude,
                option_is_some_authoritative,
                assert_authority,
            );
            let function = node.parent().filter(|p| p.kind() == "function_item");
            let supported_body = matches!(
                body.kind,
                RustBodyKind::Function | RustBodyKind::NestedFunction
            );
            if !primitive_types
                || !supported_body
                || function.and_then(|n| signature(n, text)).is_none()
                || has_unsupported_attributes(node, text)
            {
                collector.unknown(
                    node,
                    PanicUnknownKind::Opaque,
                    "unsupported body signature, deferred/constant body or attribute expansion",
                );
            } else {
                let function = function.unwrap();
                let (params, result) = signature(function, text).unwrap();
                for (index, (name, sort)) in params.iter().enumerate() {
                    let id = format!("{}:parameter:{index}", body.body_id);
                    let value = collector.value(
                        &id,
                        *sort,
                        node,
                        PanicValueOrigin::Parameter {
                            index: index as u32,
                        },
                    );
                    collector.env.insert(name.clone(), Binding::Scalar(value));
                }
                let evaluated = collector.eval(node)?;
                if collector.alive {
                    match (result, evaluated) {
                        (Some(sort), Eval::Scalar(expression))
                            if collector.sort(&expression) == sort =>
                        {
                            collector.ret(node, expression)
                        }
                        (None, Eval::Unit) => {}
                        _ => {
                            collector.unknown(
                                node,
                                PanicUnknownKind::Opaque,
                                "unsupported or missing return value",
                            );
                        }
                    }
                }
            }
            // R3 inventories syntax even when it is not evaluated by this lane.
            // Missing invocation semantics always retain explicit uncertainty.
            let missing: Vec<_> = calls
                .sites
                .iter()
                .filter(|s| {
                    s.body_id == body.body_id
                        && !collector.facts.sites.iter().any(|f| f.site_id == s.site_id)
                })
                .cloned()
                .collect();
            for site in missing {
                let loc = PanicFactLocation {
                    body_id: body.body_id.clone(),
                    file_path: body.file_path.clone(),
                    source_oid: body.source_oid.clone(),
                    byte_range: site.byte_range,
                };
                let obligation_id = collector.unknown_location(
                    loc.clone(),
                    PanicUnknownKind::Callee,
                    "invocation in unsupported or unreachable syntax",
                );
                collector.facts.sites.push(PanicSemanticSite {
                    site_id: site.site_id,
                    context: collector.context_location(loc),
                    semantics: PanicSiteSemantics::Unknown { obligation_id },
                });
            }
        }
    }
    facts.values.sort_by(|a, b| a.value_id.cmp(&b.value_id));
    facts
        .expressions
        .sort_by(|a, b| a.expression_id.cmp(&b.expression_id));
    facts
        .assignments
        .sort_by(|a, b| a.assignment_id.cmp(&b.assignment_id));
    facts.sites.sort_by(|a, b| a.site_id.cmp(&b.site_id));
    facts
        .unknown_obligations
        .sort_by(|a, b| a.obligation_id.cmp(&b.obligation_id));
    facts
        .control_flow
        .sort_by(|a, b| (&a.location.body_id, a.order).cmp(&(&b.location.body_id, b.order)));
    facts
        .body_coverage
        .sort_by(|a, b| a.body_id.cmp(&b.body_id));
    for coverage in &mut facts.body_coverage {
        coverage.unknown_obligations = facts
            .unknown_obligations
            .iter()
            .filter(|u| u.location.body_id == coverage.body_id)
            .map(|u| u.obligation_id.clone())
            .collect();
    }
    if let Some(extraction) = &mut facts.extraction {
        extraction.returns.sort_by_key(|r| {
            (
                r.context.location.body_id.clone(),
                r.context.flow_id.clone(),
            )
        });
        extraction
            .operations
            .sort_by(|a, b| a.operation_id.cmp(&b.operation_id));
        extraction.loops.sort_by(|a, b| a.loop_id.cmp(&b.loop_id));
    }
    facts.validate_against(graph, sources, &calls)?;
    Ok(facts)
}

#[derive(Clone)]
struct Helper {
    body_id: String,
    parameters: Vec<(String, PanicSort)>,
    result: Option<PanicSort>,
}

fn signature(node: Node<'_>, text: &str) -> Option<(Vec<(String, PanicSort)>, Option<PanicSort>)> {
    if node.child_by_field_name("type_parameters").is_some()
        || children(node)
            .iter()
            .any(|n| n.kind() == "function_modifiers")
    {
        return None;
    }
    let params = node.child_by_field_name("parameters")?;
    let mut parameters = vec![];
    for param in children(params) {
        if param.kind() != "parameter" {
            return None;
        }
        let pattern = param.child_by_field_name("pattern")?;
        if pattern.kind() != "identifier" {
            return None;
        }
        let sort = parse_sort(&syntax(param.child_by_field_name("type")?, text))?;
        parameters.push((ident(&text[pattern.byte_range()]).into(), sort));
    }
    let result = match node.child_by_field_name("return_type") {
        None => None,
        Some(n) if syntax(n, text) == "()" => None,
        Some(n) => Some(parse_sort(&syntax(n, text))?),
    };
    Some((parameters, result))
}
fn parse_sort(s: &str) -> Option<PanicSort> {
    match s {
        "u8" => Some(PanicSort::U8),
        "bool" => Some(PanicSort::Bool),
        _ => None,
    }
}
fn ident(s: &str) -> &str {
    s.strip_prefix("r#").unwrap_or(s)
}
// Read Rust tokens, never text substrings: comments cannot manufacture a
// suffix, generic argument, namespace binding, or constructor path.
fn syntax(n: Node<'_>, text: &str) -> String {
    if matches!(n.kind(), "line_comment" | "block_comment") {
        return String::new();
    }
    if n.child_count() == 0 {
        return ident(&text[n.byte_range()]).into();
    }
    let mut cursor = n.walk();
    n.children(&mut cursor).map(|c| syntax(c, text)).collect()
}
fn u8_type_argument(n: Node<'_>, text: &str) -> bool {
    n.child_by_field_name("type_arguments").is_some_and(|a| {
        let args = children(a);
        args.len() == 1 && syntax(args[0], text) == "u8"
    })
}
fn field_text<'a>(n: Node<'_>, field: &str, text: &'a str) -> Option<&'a str> {
    n.child_by_field_name(field).map(|n| &text[n.byte_range()])
}
fn range(n: Node<'_>) -> [usize; 2] {
    [n.start_byte(), n.end_byte()]
}
fn children(n: Node<'_>) -> Vec<Node<'_>> {
    let mut c = n.walk();
    n.named_children(&mut c)
        .filter(|n| !matches!(n.kind(), "line_comment" | "block_comment"))
        .collect()
}
fn descendants(n: Node<'_>) -> Vec<Node<'_>> {
    let mut result = vec![];
    let mut todo = vec![n];
    while let Some(n) = todo.pop() {
        todo.extend(children(n));
        result.push(n);
    }
    result
}
fn has_unsupported_attributes(n: Node<'_>, text: &str) -> bool {
    std::iter::successors(n.parent(), |n| n.parent()).any(|p| {
        let siblings = children(p);
        siblings.iter().any(|s| {
            matches!(s.kind(), "attribute_item" | "inner_attribute_item") && {
                !benign_attribute(*s, text)
            }
        })
    })
}

fn benign_attribute(n: Node<'_>, text: &str) -> bool {
    let t = &text[n.byte_range()];
    t.starts_with("#[allow(") || t.starts_with("#![allow(")
        || t.starts_with("#[doc =") || t.starts_with("#![doc =")
}

/// Parse only a pure predicate and an optional plain ASCII literal message.
/// The parser view masks the macro prefix/message with same-length whitespace;
/// predicate bytes, offsets, body and original source OID are never replaced.
/// No normalized fixture, macro expansion or synthetic call site is collected.
fn assertion_syntax(n: Node<'_>, text: &str) -> Option<(tree_sitter::Tree, [usize; 2], Option<String>)> {
    if n.kind() != "macro_invocation" || field_text(n, "macro", text)? != "assert" {
        return None;
    }
    let tokens = children(n).into_iter().find(|n| n.kind() == "token_tree")?;
    let token_bytes = &text.as_bytes()[tokens.byte_range()];
    if token_bytes.first() != Some(&b'(') || token_bytes.last() != Some(&b')') { return None; }
    let mut cursor = tokens.walk();
    let commas: Vec<_> = tokens.children(&mut cursor).filter(|n| n.kind() == ",").collect();
    if commas.len() > 2 { return None; }
    let predicate_end = commas.first().map_or(tokens.end_byte()-1, |c|c.start_byte());
    let message = if let Some(comma) = commas.first() {
        let end = commas.get(1).map_or(tokens.end_byte()-1, |c|c.start_byte());
        let message = text[comma.end_byte()..end].trim();
        if commas.get(1).is_some_and(|c| !text[c.end_byte()..tokens.end_byte()-1].trim().is_empty()) { return None; }
        if message.is_empty() && commas.len() == 1 { None } else {
            let inner = message.strip_prefix('"')?.strip_suffix('"')?;
            if !inner.bytes().all(|c| c.is_ascii_graphic() || c == b' ')
                || inner.contains(['\\', '"', '{', '}']) { return None; }
            Some(inner.to_owned())
        }
    } else { None };
    let mut view = text.as_bytes().to_vec();
    for byte in &mut view[n.start_byte()..tokens.start_byte()] { *byte = b' '; }
    for byte in &mut view[predicate_end..tokens.end_byte()-1] { *byte = b' '; }
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into()).ok()?;
    let tree = parser.parse(&view, None)?;
    if tree.root_node().has_error() { return None; }
    let predicate = descendants(tree.root_node()).into_iter().find(|node|
        node.kind() == "parenthesized_expression" && range(*node) == range(tokens))?;
    if !pure_assertion_predicate(predicate, text) { return None; }
    Some((tree, range(tokens), message))
}

fn pure_assertion_predicate(n: Node<'_>, text: &str) -> bool {
    match n.kind() {
        "identifier" | "integer_literal" | "boolean_literal" => true,
        "parenthesized_expression" => children(n).into_iter().all(|n| pure_assertion_predicate(n, text)),
        "unary_expression" => text[n.byte_range()].trim_start().starts_with('!')
            && children(n).into_iter().all(|n| pure_assertion_predicate(n, text)),
        "binary_expression" => matches!(field_text(n,"operator",text), Some("=="|"!="|"<"|">"|"<="|">="|"&&"|"||"))
            && children(n).into_iter().all(|n| pure_assertion_predicate(n, text)),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Binding {
    Scalar(String),
    Option(String, String),
    Array(Vec<u8>),
    Unknown,
}
#[derive(Clone, Debug)]
enum Eval {
    Scalar(String),
    Option(String, String),
    Array(Vec<u8>),
    Unit,
    Unknown,
}

#[derive(Clone)]
struct State {
    env: BTreeMap<String, Binding>,
    flow: String,
    guard: PanicGuard,
    alive: bool,
}

struct Collector<'a> {
    facts: &'a mut PanicSemanticFacts,
    calls: &'a RustCallSiteEvidenceSet,
    body: &'a RustBodyEvidence,
    text: &'a str,
    helpers: &'a BTreeMap<String, Vec<Helper>>,
    prelude: bool,
    option_is_some_authoritative: bool,
    assert_authority: bool,
    env: BTreeMap<String, Binding>,
    flow: String,
    guard: PanicGuard,
    alive: bool,
    order: u32,
    sequence: u32,
    versions: BTreeMap<String, u32>,
    grounded_bindings: BTreeSet<String>,
    grounded_expressions: BTreeSet<String>,
}

impl<'a> Collector<'a> {
    fn new(
        facts: &'a mut PanicSemanticFacts,
        calls: &'a RustCallSiteEvidenceSet,
        body: &'a RustBodyEvidence,
        text: &'a str,
        helpers: &'a BTreeMap<String, Vec<Helper>>,
        prelude: bool,
        option_is_some_authoritative: bool,
        assert_authority: bool,
    ) -> Self {
        let flow = format!("{}:flow:0", body.body_id);
        let location = PanicFactLocation {
            body_id: body.body_id.clone(),
            file_path: body.file_path.clone(),
            source_oid: body.source_oid.clone(),
            byte_range: body.byte_range,
        };
        facts.control_flow.push(PanicControlFlow {
            flow_id: flow.clone(),
            location,
            order: 0,
            guard: PanicGuard::Always,
            predecessors: vec![],
        });
        facts.body_coverage.push(PanicBodyCoverage {
            body_id: body.body_id.clone(),
            entry_flow_id: flow.clone(),
            unknown_obligations: vec![],
        });
        Self {
            facts,
            calls,
            body,
            text,
            helpers,
            prelude,
            option_is_some_authoritative,
            assert_authority,
            env: BTreeMap::new(),
            flow,
            guard: PanicGuard::Always,
            alive: true,
            order: 0,
            sequence: 0,
            versions: BTreeMap::new(),
            grounded_bindings: BTreeSet::new(),
            grounded_expressions: BTreeSet::new(),
        }
    }
    fn id(&mut self, kind: &str) -> String {
        self.sequence += 1;
        format!("{}:{kind}:{}", self.body.body_id, self.sequence)
    }
    fn location(&self, n: Node<'_>) -> PanicFactLocation {
        PanicFactLocation {
            body_id: self.body.body_id.clone(),
            file_path: self.body.file_path.clone(),
            source_oid: self.body.source_oid.clone(),
            byte_range: range(n),
        }
    }
    fn context_location(&self, location: PanicFactLocation) -> PanicFactContext {
        PanicFactContext {
            location,
            flow_id: self.flow.clone(),
            guard: self.guard.clone(),
        }
    }
    fn context(&self, n: Node<'_>) -> PanicFactContext {
        self.context_location(self.location(n))
    }
    fn step(&mut self, n: Node<'_>) {
        let pred = self.flow.clone();
        self.join(n, vec![pred]);
    }
    fn join(&mut self, n: Node<'_>, mut predecessors: Vec<String>) {
        predecessors.sort();
        predecessors.dedup();
        self.order += 1;
        self.flow = format!("{}:flow:{}", self.body.body_id, self.order);
        self.facts.control_flow.push(PanicControlFlow {
            flow_id: self.flow.clone(),
            location: self.location(n),
            order: self.order,
            guard: self.guard.clone(),
            predecessors,
        });
    }
    fn state(&self) -> State {
        State {
            env: self.env.clone(),
            flow: self.flow.clone(),
            guard: self.guard.clone(),
            alive: self.alive,
        }
    }
    fn restore(&mut self, s: &State) {
        self.env = s.env.clone();
        self.flow = s.flow.clone();
        self.guard = s.guard.clone();
        self.alive = s.alive;
    }
    fn unknown_location(
        &mut self,
        location: PanicFactLocation,
        kind: PanicUnknownKind,
        detail: &str,
    ) -> String {
        let id = self.id("unknown");
        self.facts.unknown_obligations.push(PanicUnknownObligation {
            obligation_id: id.clone(),
            kind,
            location,
            detail: detail.into(),
        });
        id
    }
    fn unknown(&mut self, n: Node<'_>, kind: PanicUnknownKind, detail: &str) -> String {
        self.unknown_location(self.location(n), kind, detail)
    }
    fn taint(&mut self, n: Node<'_>, detail: &str) {
        self.unknown(n, PanicUnknownKind::Mutation, detail);
        for value in self.env.values_mut() {
            *value = Binding::Unknown;
        }
    }
    fn sort(&self, id: &str) -> PanicSort {
        self.facts
            .expressions
            .iter()
            .find(|e| e.expression_id == id)
            .unwrap()
            .sort
    }
    fn expression(
        &mut self,
        n: Node<'_>,
        sort: PanicSort,
        derivation: PanicExpressionDerivation,
    ) -> String {
        self.step(n);
        let id = self.id("expression");
        let domain = match derivation {
            PanicExpressionDerivation::BoolLiteral { value } => PanicValueDomain::Bool {
                values: vec![value],
            },
            PanicExpressionDerivation::U8Literal { value } => PanicValueDomain::U8 {
                minimum: value.into(),
                maximum: value.into(),
            },
            _ => domain(sort),
        };
        self.facts.expressions.push(PanicTypedExpression {
            expression_id: id.clone(),
            sort,
            domain,
            context: self.context(n),
            derivation,
        });
        id
    }
    fn boolean(&mut self, n: Node<'_>, value: bool) -> String {
        self.expression(
            n,
            PanicSort::Bool,
            PanicExpressionDerivation::BoolLiteral { value },
        )
    }
    fn integer(&mut self, n: Node<'_>, value: u8) -> String {
        self.expression(
            n,
            PanicSort::U8,
            PanicExpressionDerivation::U8Literal { value },
        )
    }
    fn value(
        &mut self,
        binding: &str,
        sort: PanicSort,
        n: Node<'_>,
        origin: PanicValueOrigin,
    ) -> String {
        let version = self
            .versions
            .entry(binding.into())
            .and_modify(|v| *v += 1)
            .or_insert(0);
        let version = *version;
        let id = self.id("value");
        self.facts.values.push(PanicValueVersion {
            value_id: id.clone(),
            binding_id: binding.into(),
            version,
            sort,
            domain: domain(sort),
            context: self.context(n),
            origin,
        });
        id
    }
    fn read(&mut self, n: Node<'_>, id: &str) -> String {
        let sort = self
            .facts
            .values
            .iter()
            .find(|v| v.value_id == id)
            .unwrap()
            .sort;
        self.expression(
            n,
            sort,
            PanicExpressionDerivation::Value {
                value_id: id.into(),
            },
        )
    }
    fn assigned(&mut self, n: Node<'_>, binding: &str, expression: &str) -> String {
        let sort = self.sort(expression);
        if self.versions.contains_key(binding) {
            let assignment_id = self.id("assignment");
            let value = self.value(
                binding,
                sort,
                n,
                PanicValueOrigin::Assignment {
                    assignment_id: assignment_id.clone(),
                },
            );
            self.facts.assignments.push(PanicGuardedAssignment {
                assignment_id,
                context: self.context(n),
                target_value_id: value.clone(),
                expression_id: expression.into(),
            });
            value
        } else {
            self.value(
                binding,
                sort,
                n,
                PanicValueOrigin::Expression {
                    expression_id: expression.into(),
                },
            )
        }
    }
    fn bind(&mut self, n: Node<'_>, binding: &str, evaluated: Eval) -> Binding {
        match evaluated {
            Eval::Scalar(e) => Binding::Scalar(self.assigned(n, binding, &e)),
            Eval::Option(p, v) => Binding::Option(
                self.assigned(n, &format!("{binding}:present"), &p),
                self.assigned(n, &format!("{binding}:payload"), &v),
            ),
            Eval::Array(v) => Binding::Array(v),
            _ => Binding::Unknown,
        }
    }
    fn from_binding(&mut self, n: Node<'_>, binding: Binding) -> Eval {
        match binding {
            Binding::Scalar(v) => Eval::Scalar(self.read(n, &v)),
            Binding::Option(p, v) => Eval::Option(self.read(n, &p), self.read(n, &v)),
            Binding::Array(v) => Eval::Array(v),
            Binding::Unknown => {
                self.unknown(
                    n,
                    PanicUnknownKind::Opaque,
                    "read of unsupported lexical value or effect-tainted binding",
                );
                Eval::Unknown
            }
        }
    }
    fn scalar(
        &mut self,
        n: Node<'_>,
        expected: Option<PanicSort>,
    ) -> anyhow::Result<Option<String>> {
        match self.eval(n)? {
            Eval::Scalar(e) if expected.is_none_or(|s| s == self.sort(&e)) => Ok(Some(e)),
            _ => {
                self.unknown(
                    n,
                    PanicUnknownKind::Opaque,
                    "expected typed scalar expression",
                );
                Ok(None)
            }
        }
    }
    fn grounded_u8(&self, id: &str, seen: &mut BTreeSet<String>) -> bool {
        if self.grounded_expressions.contains(id) {
            return true;
        }
        if !seen.insert(id.into()) {
            return false;
        }
        let e = self
            .facts
            .expressions
            .iter()
            .find(|e| e.expression_id == id)
            .unwrap();
        use PanicExpressionDerivation::*;
        match &e.derivation {
            U8Literal { .. } => literal_u8_suffix(
                &self.text[e.context.location.byte_range[0]..e.context.location.byte_range[1]],
            ),
            Value { value_id } => self.grounded_value(value_id, seen),
            U8Add { left, right } => self.grounded_u8(left, seen) || self.grounded_u8(right, seen),
            U8ArrayIndex { .. } => true,
            _ => false,
        }
    }
    fn grounded_value(&self, id: &str, seen: &mut BTreeSet<String>) -> bool {
        if !seen.insert(id.into()) {
            return false;
        }
        let value = self.facts.values.iter().find(|v| v.value_id == id).unwrap();
        if self.grounded_bindings.contains(&value.binding_id) {
            return true;
        }
        match &value.origin {
            PanicValueOrigin::Parameter { .. }
            | PanicValueOrigin::CallResult { .. }
            | PanicValueOrigin::LoopVariable { .. } => true,
            PanicValueOrigin::Expression { expression_id } => self.grounded_u8(expression_id, seen),
            PanicValueOrigin::Assignment { assignment_id } => self.grounded_u8(
                &self
                    .facts
                    .assignments
                    .iter()
                    .find(|a| a.assignment_id == *assignment_id)
                    .unwrap()
                    .expression_id,
                seen,
            ),
            PanicValueOrigin::Phi { inputs } => inputs
                .iter()
                .any(|i| self.grounded_value(&i.value_id, seen)),
            _ => false,
        }
    }
    fn known_u8(&self, n: Node<'_>) -> bool {
        match n.kind() {
            "integer_literal" => literal_u8_suffix(&self.text[n.byte_range()]),
            "identifier" => match self.env.get(ident(&self.text[n.byte_range()])) {
                Some(Binding::Scalar(v)) => self.grounded_value(v, &mut BTreeSet::new()),
                _ => false,
            },
            "parenthesized_expression" => children(n).first().is_some_and(|n| self.known_u8(*n)),
            _ => false,
        }
    }
    // The helper catalog contains root functions only. RustBodyKind::Function
    // also includes module functions, so it cannot establish lexical ownership.
    // Share this gate with argument inference: wrong-scope metadata must never
    // constrain a default integer to u8 before the call itself is refused.
    fn root_helper(&self, target: Node<'_>) -> Option<&Helper> {
        if target.kind() != "identifier"
            || !self
                .body_function(target)
                .parent()
                .is_some_and(|p| p.kind() == "source_file")
        {
            return None;
        }
        let name = ident(&self.text[target.byte_range()]);
        if self.env.contains_key(name) {
            return None;
        }
        self.helpers
            .get(name)
            .filter(|h| h.len() == 1)
            .map(|h| &h[0])
    }
    // Only contexts which constrain this expression's numeric type propagate
    // authority. In particular, an if condition never inherits its result type.
    fn contextual_u8(&self, mut n: Node<'_>) -> bool {
        while let Some(parent) = n.parent() {
            match parent.kind() {
                "parenthesized_expression" | "else_clause" => {}
                "block" if children(parent).last() == Some(&n) => {}
                "expression_statement"
                    if !self.text[parent.byte_range()].trim_end().ends_with(';') => {}
                "if_expression" if parent.child_by_field_name("condition") != Some(n) => {}
                "binary_expression" => {
                    let operator = field_text(parent, "operator", self.text).unwrap_or("");
                    let sibling = ["left", "right"]
                        .iter()
                        .filter_map(|f| parent.child_by_field_name(f))
                        .find(|c| *c != n);
                    if sibling.is_some_and(|s| self.known_u8(s)) {
                        return true;
                    }
                    if operator != "+" {
                        return false;
                    }
                }
                "let_declaration" => {
                    return parent.child_by_field_name("type").is_some_and(|t| {
                        matches!(syntax(t, self.text).as_str(), "u8" | "Option<u8>")
                    })
                }
                "function_item" => {
                    return signature(parent, self.text).is_some_and(|s| s.1 == Some(PanicSort::U8))
                }
                "return_expression" => {
                    return signature(self.body_function(n), self.text)
                        .is_some_and(|s| s.1 == Some(PanicSort::U8))
                }
                "arguments" => {
                    let Some(call) = parent.parent() else {
                        return false;
                    };
                    let Some(target) = call.child_by_field_name("function") else {
                        return false;
                    };
                    if self.constructor(target) == Some(true) {
                        if u8_type_argument(target, self.text) {
                            return true;
                        }
                        n = call;
                        continue;
                    }
                    return self
                        .root_helper(target)
                        .and_then(|h| {
                            children(parent)
                                .iter()
                                .position(|a| *a == n)
                                .and_then(|i| h.parameters.get(i))
                        })
                        .is_some_and(|(_, s)| *s == PanicSort::U8);
                }
                _ => return false,
            }
            n = parent;
        }
        false
    }
    fn body_function<'n>(&self, n: Node<'n>) -> Node<'n> {
        std::iter::successors(Some(n), |n| n.parent())
            .find(|n| n.kind() == "function_item")
            .unwrap()
    }
    fn ret(&mut self, n: Node<'_>, expression_id: String) {
        let context = self.context(n);
        self.facts
            .extraction
            .as_mut()
            .unwrap()
            .returns
            .push(PanicBodyReturn {
                context,
                expression_id,
            });
    }
    fn site(&mut self, n: Node<'_>, semantics: PanicSiteSemantics) {
        if let Some(site) = self
            .calls
            .sites
            .iter()
            .find(|s| s.body_id == self.body.body_id && s.byte_range == range(n))
        {
            self.facts.sites.push(PanicSemanticSite {
                site_id: site.site_id.clone(),
                context: self.context(n),
                semantics,
            });
        }
    }
    fn opaque_call(&mut self, n: Node<'_>) -> Eval {
        self.taint(n, "opaque call may panic, mutate or alias live values");
        let obligation_id = self.unknown(
            n,
            PanicUnknownKind::Callee,
            "call target/effects are outside the bounded source model",
        );
        self.site(n, PanicSiteSemantics::Unknown { obligation_id });
        Eval::Unknown
    }

    fn eval(&mut self, n: Node<'_>) -> anyhow::Result<Eval> {
        if !self.alive {
            return Ok(Eval::Unit);
        }
        let text = &self.text[n.byte_range()];
        Ok(match n.kind() {
            "block" => {
                let mut shadows: BTreeMap<String, Option<Binding>> = BTreeMap::new();
                let mut result = Eval::Unit;
                // Rust local items are in scope for the entire block, including
                // before the item declaration. Do not redirect to a top-level
                // helper with the same spelling.
                for child in children(n) {
                    if matches!(
                        child.kind(),
                        "function_item"
                            | "use_declaration"
                            | "struct_item"
                            | "enum_item"
                            | "union_item"
                            | "mod_item"
                            | "type_item"
                            | "const_item"
                            | "static_item"
                            | "macro_definition"
                    ) {
                        if let Some(name) = field_text(child, "name", self.text) {
                            let name = ident(name);
                            shadows
                                .entry(name.into())
                                .or_insert_with(|| self.env.get(name).cloned());
                            self.env.insert(name.into(), Binding::Unknown);
                        } else {
                            self.unknown(
                                child,
                                PanicUnknownKind::Opaque,
                                "unsupported local import",
                            );
                        }
                    }
                }
                for child in children(n) {
                    if !self.alive {
                        break;
                    }
                    if child.kind() == "let_declaration" {
                        if let Some(p) = child.child_by_field_name("pattern") {
                            let name = ident(&self.text[p.byte_range()]).to_owned();
                            let old = self.env.get(&name).cloned();
                            shadows.entry(name).or_insert(old);
                        }
                    }
                    result = self.eval(child)?;
                }
                for (name, old) in shadows {
                    match old {
                        Some(old) => {
                            self.env.insert(name, old);
                        }
                        None => {
                            self.env.remove(&name);
                        }
                    }
                }
                result
            }
            "expression_statement" => {
                let mut value = Eval::Unit;
                for child in children(n) {
                    value = self.eval(child)?;
                }
                if text.trim_end().ends_with(';') {
                    Eval::Unit
                } else {
                    value
                }
            }
            "empty_statement" | "function_item" | "struct_item" | "impl_item" | "const_item"
            | "static_item" | "enum_item" | "union_item" | "type_item" | "mod_item" => Eval::Unit,
            "let_declaration" => {
                let Some(pattern) = n.child_by_field_name("pattern") else {
                    return Ok(Eval::Unknown);
                };
                let Some(initializer) = n.child_by_field_name("value") else {
                    self.unknown(n, PanicUnknownKind::Opaque, "uninitialized local");
                    return Ok(Eval::Unknown);
                };
                let mut result = self.eval(initializer)?;
                let declared = n.child_by_field_name("type").map(|t| syntax(t, self.text));
                let valid_type = match (declared.as_deref(), &result) {
                    (Some("u8"), Eval::Scalar(e)) => self.sort(e) == PanicSort::U8,
                    (Some("bool"), Eval::Scalar(e)) => self.sort(e) == PanicSort::Bool,
                    (Some("Option<u8>"), Eval::Option(..)) => self.prelude,
                    (Some(s), Eval::Array(_)) => s.starts_with("[u8;") && s.ends_with(']'),
                    (None, Eval::Scalar(e)) if self.sort(e) == PanicSort::U8 => {
                        self.grounded_u8(e, &mut BTreeSet::new())
                    }
                    (None, Eval::Option(_, v)) => {
                        self.grounded_u8(v, &mut BTreeSet::new())
                            || u8_type_argument(initializer, self.text)
                            || (initializer.kind() == "call_expression"
                                && initializer
                                    .child_by_field_name("function")
                                    .is_some_and(|t| u8_type_argument(t, self.text)))
                    }
                    (None, _) => true,
                    _ => false,
                };
                if !valid_type {
                    self.unknown(
                        n,
                        PanicUnknownKind::Opaque,
                        "unsupported or unconstrained local type",
                    );
                    result = Eval::Unknown;
                }
                if pattern.kind() == "identifier" {
                    let name = ident(&self.text[pattern.byte_range()]).to_owned();
                    let binding = format!("{}:local:{}", self.body.body_id, pattern.start_byte());
                    let bound = self.bind(initializer, &binding, result);
                    // Only a successfully checked source declaration can make
                    // a local's type authoritative for subsequent aliases.
                    for id in match &bound {
                        Binding::Scalar(id) => vec![id],
                        Binding::Option(a, b) => vec![a, b],
                        _ => vec![],
                    } {
                        let value = self
                            .facts
                            .values
                            .iter()
                            .find(|v| v.value_id == *id)
                            .unwrap();
                        self.grounded_bindings.insert(value.binding_id.clone());
                    }
                    self.env.insert(name, bound);
                } else if &self.text[pattern.byte_range()] != "_" {
                    self.taint(n, "unsupported local destructuring or pattern");
                }
                Eval::Unit
            }
            "identifier" => {
                if let Some(binding) = self.env.get(ident(text)).cloned() {
                    self.from_binding(n, binding)
                } else if self.constructor(n) == Some(false) {
                    let p = self.boolean(n, false);
                    let v = self.integer(n, 0);
                    if u8_type_argument(n, self.text) {
                        self.grounded_expressions.insert(v.clone());
                    }
                    Eval::Option(p, v)
                } else {
                    self.unknown(
                        n,
                        PanicUnknownKind::Opaque,
                        "unbound or unsupported identifier",
                    );
                    Eval::Unknown
                }
            }
            "generic_function" | "scoped_identifier" => {
                if self.constructor(n) == Some(false) {
                    let p = self.boolean(n, false);
                    let v = self.integer(n, 0);
                    if u8_type_argument(n, self.text) {
                        self.grounded_expressions.insert(v.clone());
                    }
                    Eval::Option(p, v)
                } else {
                    self.unknown(
                        n,
                        PanicUnknownKind::Opaque,
                        "unsupported path or generic expression",
                    );
                    Eval::Unknown
                }
            }
            "integer_literal" => match literal(text) {
                Some(v) => Eval::Scalar(self.integer(n, v)),
                None => {
                    self.unknown(n, PanicUnknownKind::Opaque, "literal outside u8 subset");
                    Eval::Unknown
                }
            },
            "boolean_literal" => Eval::Scalar(self.boolean(n, text == "true")),
            "parenthesized_expression" => self.eval(children(n)[0])?,
            "binary_expression" => self.binary(n)?,
            "unary_expression" => {
                if field_text(n, "operator", self.text) == Some("!") || text.starts_with('!') {
                    if let Some(operand) =
                        self.scalar(*children(n).last().unwrap(), Some(PanicSort::Bool))?
                    {
                        Eval::Scalar(self.expression(
                            n,
                            PanicSort::Bool,
                            PanicExpressionDerivation::Not { operand },
                        ))
                    } else {
                        Eval::Unknown
                    }
                } else {
                    self.taint(n, "unsupported unary operation");
                    Eval::Unknown
                }
            }
            "if_expression" => {
                let Some(condition) = self.scalar(
                    n.child_by_field_name("condition").unwrap(),
                    Some(PanicSort::Bool),
                )?
                else {
                    self.taint(n, "unknown branch condition");
                    return Ok(Eval::Unknown);
                };
                self.branch(
                    n,
                    condition,
                    n.child_by_field_name("consequence").unwrap(),
                    n.child_by_field_name("alternative"),
                )?
            }
            "else_clause" => self.eval(*children(n).last().unwrap())?,
            "assignment_expression" => {
                let left = n.child_by_field_name("left").unwrap();
                let right = n.child_by_field_name("right").unwrap();
                let result = self.eval(right)?;
                let name = ident(&self.text[left.byte_range()]);
                let old = self.env.get(name).cloned();
                if left.kind() == "identifier" {
                    if let Some(old) = old {
                        let binding = match &old {
                            Binding::Scalar(v) | Binding::Option(v, _) => self
                                .facts
                                .values
                                .iter()
                                .find(|r| r.value_id == *v)
                                .map(|v| v.binding_id.trim_end_matches(":present").to_owned()),
                            _ => None,
                        };
                        if let Some(binding) = binding {
                            let value = self.bind(n, &binding, result);
                            self.env.insert(name.into(), value);
                        } else {
                            self.taint(n, "unsupported reassignment");
                        }
                    } else {
                        self.taint(n, "assignment to unbound storage");
                    }
                } else {
                    self.taint(n, "aliased or indirect assignment");
                }
                Eval::Unit
            }
            "call_expression" => self.call(n)?,
            "return_expression" => {
                if let Some(value) = children(n).first().copied() {
                    if let Eval::Scalar(e) = self.eval(value)? {
                        self.ret(n, e);
                    } else {
                        self.unknown(n, PanicUnknownKind::Opaque, "unsupported early return");
                    }
                }
                self.alive = false;
                Eval::Unit
            }
            "array_expression" => {
                let values: Option<Vec<_>> = children(n)
                    .iter()
                    .map(|e| {
                        (e.kind() == "integer_literal")
                            .then(|| literal(&self.text[e.byte_range()]))
                            .flatten()
                    })
                    .collect();
                let typed = children(n)
                    .iter()
                    .any(|e| literal_u8_suffix(&self.text[e.byte_range()]))
                    || n.parent()
                        .filter(|p| p.kind() == "let_declaration")
                        .and_then(|p| p.child_by_field_name("type"))
                        .is_some_and(|t| syntax(t, self.text).starts_with("[u8;"));
                if typed
                    && n.child_by_field_name("length").is_none()
                    && values
                        .as_ref()
                        .is_some_and(|v| !v.is_empty() && v.len() <= 255)
                {
                    Eval::Array(values.unwrap())
                } else {
                    self.unknown(n, PanicUnknownKind::Opaque, "unsupported array shape");
                    Eval::Unknown
                }
            }
            "index_expression" => self.index(n)?,
            "for_expression" => self.literal_loop(n)?,
            "type_cast_expression" => {
                self.taint(n, "unsupported cast outside bounded array index");
                Eval::Unknown
            }
            "macro_invocation" => self.assertion(n)?,
            "reference_expression"
            | "unsafe_block"
            | "compound_assignment_expr"
            | "match_expression"
            | "loop_expression"
            | "while_expression" => {
                self.taint(
                    n,
                    "unsupported syntax, macro expansion, aliasing or control flow",
                );
                Eval::Unknown
            }
            _ => {
                self.taint(n, &format!("unsupported Rust AST node {}", n.kind()));
                Eval::Unknown
            }
        })
    }

    fn assertion(&mut self, n: Node<'_>) -> anyhow::Result<Eval> {
        if self.assert_authority {
            if let Some((tree, predicate_range, message)) = assertion_syntax(n, self.text) {
                let predicate = descendants(tree.root_node()).into_iter()
                    .find(|node| node.kind() == "parenthesized_expression" && range(*node) == predicate_range)
                    .unwrap();
                if let Some(predicate) = self.scalar(predicate, Some(PanicSort::Bool))? {
                    // Evaluation happens first. The operation retains the full
                    // original macro range and its exact order/branch guard.
                    self.step(n);
                    let operation_id = self.id("assertion");
                    let context = self.context(n);
                    self.facts.extraction.as_mut().unwrap().operations.push(PanicImplicitOperation {
                        operation_id, context, result_expression_id: predicate.clone(),
                        operation: PanicImplicitOperationKind::AssertBool {predicate, message},
                    });
                    return Ok(Eval::Unit);
                }
            }
        }
        self.taint(n, "assertion macro/type authority, predicate, message or effects are unsupported");
        Ok(Eval::Unknown)
    }

    fn constructor(&self, n: Node<'_>) -> Option<bool> {
        let text = syntax(n, self.text);
        let bare = if n.kind() == "generic_function" {
            if !u8_type_argument(n, self.text) {
                return None;
            }
            syntax(n.child_by_field_name("function")?, self.text)
        } else {
            text
        };
        if !self.prelude
            || self.env.contains_key("Some")
            || self.env.contains_key("None")
            || self.env.contains_key("Option")
        {
            return None;
        }
        match bare.as_str() {
            "Some"
            | "Option::Some"
            | "std::option::Option::Some"
            | "core::option::Option::Some" => Some(true),
            "None"
            | "Option::None"
            | "std::option::Option::None"
            | "core::option::Option::None" => Some(false),
            _ => None,
        }
    }
    fn binary(&mut self, n: Node<'_>) -> anyhow::Result<Eval> {
        let left = n.child_by_field_name("left").unwrap();
        let right = n.child_by_field_name("right").unwrap();
        let op = field_text(n, "operator", self.text).unwrap_or("");
        let Some(a) = self.scalar(left, None)? else {
            self.taint(n, "unknown binary operand");
            return Ok(Eval::Unknown);
        };
        if matches!(op, "&&" | "||") {
            if self.sort(&a) != PanicSort::Bool {
                return Ok(Eval::Unknown);
            };
            let condition = if op == "&&" {
                a
            } else {
                self.expression(
                    n,
                    PanicSort::Bool,
                    PanicExpressionDerivation::Not { operand: a },
                )
            };
            return self.short_circuit(n, condition, right, op == "||");
        }
        let Some(b) = self.scalar(right, Some(self.sort(&a)))? else {
            return Ok(Eval::Unknown);
        };
        if self.sort(&a) == PanicSort::U8
            && !self.grounded_u8(&a, &mut BTreeSet::new())
            && !self.grounded_u8(&b, &mut BTreeSet::new())
            && !self.contextual_u8(n)
        {
            self.unknown(
                n,
                PanicUnknownKind::Opaque,
                "binary operands lack source u8 authority",
            );
            return Ok(Eval::Unknown);
        }
        use PanicExpressionDerivation::*;
        let derivation = match op {
            "==" | "!=" => Equal {
                left: a.clone(),
                right: b.clone(),
            },
            "<" | ">=" if self.sort(&a) == PanicSort::U8 => LessThan {
                left: a.clone(),
                right: b.clone(),
            },
            ">" | "<=" if self.sort(&a) == PanicSort::U8 => LessThan {
                left: b.clone(),
                right: a.clone(),
            },
            "+" if self.sort(&a) == PanicSort::U8 => U8Add {
                left: a.clone(),
                right: b.clone(),
            },
            _ => {
                self.unknown(n, PanicUnknownKind::Opaque, "unsupported binary operator");
                return Ok(Eval::Unknown);
            }
        };
        let sort = if op == "+" {
            PanicSort::U8
        } else {
            PanicSort::Bool
        };
        let mut result = self.expression(n, sort, derivation);
        if matches!(op, "!=" | ">=" | "<=") {
            result = self.expression(n, PanicSort::Bool, Not { operand: result });
        }
        if op == "+" {
            let operation_id = self.id("operation");
            let context = self.context(n);
            let overflow_checks = self.facts.build.overflow_checks;
            self.facts
                .extraction
                .as_mut()
                .unwrap()
                .operations
                .push(PanicImplicitOperation {
                    operation_id,
                    context,
                    result_expression_id: result.clone(),
                    operation: PanicImplicitOperationKind::U8Add {
                        left: a,
                        right: b,
                        overflow_checks,
                    },
                });
        }
        Ok(Eval::Scalar(result))
    }
    fn branch_guard(
        &mut self,
        n: Node<'_>,
        outer: &PanicGuard,
        condition: &str,
        expected: bool,
    ) -> PanicGuard {
        let condition = if expected {
            condition.into()
        } else {
            self.expression(
                n,
                PanicSort::Bool,
                PanicExpressionDerivation::Not {
                    operand: condition.into(),
                },
            )
        };
        let condition = match outer {
            PanicGuard::Always => condition,
            PanicGuard::When {
                expression_id,
                expected,
            } => {
                let parent = if *expected {
                    expression_id.clone()
                } else {
                    self.expression(
                        n,
                        PanicSort::Bool,
                        PanicExpressionDerivation::Not {
                            operand: expression_id.clone(),
                        },
                    )
                };
                self.expression(
                    n,
                    PanicSort::Bool,
                    PanicExpressionDerivation::And {
                        left: parent,
                        right: condition,
                    },
                )
            }
        };
        PanicGuard::When {
            expression_id: condition,
            expected: true,
        }
    }
    fn branch(
        &mut self,
        n: Node<'_>,
        condition: String,
        yes: Node<'_>,
        no: Option<Node<'_>>,
    ) -> anyhow::Result<Eval> {
        let outer = self.guard.clone();
        let yes_guard = self.branch_guard(n, &outer, &condition, true);
        let no_guard = self.branch_guard(n, &outer, &condition, false);
        let start = self.state();
        self.guard = yes_guard;
        self.step(yes);
        let a = self.eval(yes)?;
        let yes_state = self.state();
        self.restore(&start);
        self.guard = no_guard;
        self.step(n);
        let b = match no {
            Some(no) => self.eval(no)?,
            None => Eval::Unit,
        };
        let no_state = self.state();
        self.merge(n, &start, yes_state, a, no_state, b)
    }
    fn short_circuit(
        &mut self,
        n: Node<'_>,
        condition: String,
        right: Node<'_>,
        skipped: bool,
    ) -> anyhow::Result<Eval> {
        let outer = self.guard.clone();
        let yes_guard = self.branch_guard(n, &outer, &condition, true);
        let no_guard = self.branch_guard(n, &outer, &condition, false);
        let start = self.state();
        self.guard = yes_guard;
        self.step(right);
        let a = self.eval(right)?;
        let yes = self.state();
        self.restore(&start);
        self.guard = no_guard;
        self.step(n);
        let b = Eval::Scalar(self.boolean(n, skipped));
        let no = self.state();
        self.merge(n, &start, yes, a, no, b)
    }
    fn phi(&mut self, n: Node<'_>, a: &str, sa: &State, b: &str, sb: &State) -> String {
        if a == b {
            return a.into();
        }
        let av = self
            .facts
            .values
            .iter()
            .find(|v| v.value_id == a)
            .unwrap()
            .clone();
        let mut inputs = vec![
            PanicPhiInput {
                predecessor_flow_id: sa.flow.clone(),
                guard: sa.guard.clone(),
                value_id: a.into(),
            },
            PanicPhiInput {
                predecessor_flow_id: sb.flow.clone(),
                guard: sb.guard.clone(),
                value_id: b.into(),
            },
        ];
        inputs.sort_by(|a, b| a.predecessor_flow_id.cmp(&b.predecessor_flow_id));
        self.value(&av.binding_id, av.sort, n, PanicValueOrigin::Phi { inputs })
    }
    fn merge_binding(
        &mut self,
        n: Node<'_>,
        a: Binding,
        sa: &State,
        b: Binding,
        sb: &State,
    ) -> Binding {
        if a == b {
            return a;
        }
        match (a, b) {
            (Binding::Scalar(a), Binding::Scalar(b)) => {
                Binding::Scalar(self.phi(n, &a, sa, &b, sb))
            }
            (Binding::Option(ap, av), Binding::Option(bp, bv)) => {
                Binding::Option(self.phi(n, &ap, sa, &bp, sb), self.phi(n, &av, sa, &bv, sb))
            }
            _ => {
                self.unknown(
                    n,
                    PanicUnknownKind::Mutation,
                    "incompatible branch value shapes",
                );
                Binding::Unknown
            }
        }
    }
    fn merge(
        &mut self,
        n: Node<'_>,
        start: &State,
        mut sa: State,
        a: Eval,
        mut sb: State,
        b: Eval,
    ) -> anyhow::Result<Eval> {
        if !sa.alive && !sb.alive {
            self.alive = false;
            return Ok(Eval::Unit);
        }
        if !sa.alive {
            self.restore(&sb);
            return Ok(b);
        }
        if !sb.alive {
            self.restore(&sa);
            return Ok(a);
        }
        let temporary = self.id("branch-result");
        self.restore(&sa);
        let a = self.bind(n, &temporary, a);
        sa = self.state();
        self.restore(&sb);
        let b = self.bind(n, &temporary, b);
        sb = self.state();
        self.guard = start.guard.clone();
        self.alive = true;
        self.env = start.env.clone();
        self.join(n, vec![sa.flow.clone(), sb.flow.clone()]);
        for (name, old) in &start.env {
            let av = sa.env.get(name).unwrap_or(old).clone();
            let bv = sb.env.get(name).unwrap_or(old).clone();
            let value = self.merge_binding(n, av, &sa, bv, &sb);
            self.env.insert(name.clone(), value);
        }
        if matches!((&a, &b), (Binding::Unknown, Binding::Unknown)) {
            return Ok(Eval::Unit);
        }
        let result = self.merge_binding(n, a, &sa, b, &sb);
        Ok(self.from_binding(n, result))
    }
    fn call(&mut self, n: Node<'_>) -> anyhow::Result<Eval> {
        let target = n.child_by_field_name("function").unwrap();
        let args = children(n.child_by_field_name("arguments").unwrap());
        if self.constructor(target) == Some(true) && args.len() == 1 {
            if let Some(value) = self.scalar(args[0], Some(PanicSort::U8))? {
                if u8_type_argument(target, self.text) {
                    self.grounded_expressions.insert(value.clone());
                }
                let is_some = self.boolean(n, true);
                self.site(
                    n,
                    PanicSiteSemantics::OptionConstructor {
                        is_some: is_some.clone(),
                        value: value.clone(),
                    },
                );
                return Ok(Eval::Option(is_some, value));
            }
        }
        if target.kind() == "field_expression" {
            let receiver = target.child_by_field_name("value").unwrap();
            let method = ident(field_text(target, "field", self.text).unwrap_or(""));
            let receiver = self.eval(receiver)?;
            match (method, receiver, args.len()) {
                ("unwrap", Eval::Option(is_some, value), 0) => {
                    self.site(
                        n,
                        PanicSiteSemantics::OptionUnwrap {
                            is_some,
                            value: value.clone(),
                        },
                    );
                    return Ok(Eval::Scalar(value));
                }
                ("is_some", Eval::Option(is_some, _), 0) if self.option_is_some_authoritative => {
                    self.site(
                        n,
                        PanicSiteSemantics::OptionIsSome {
                            is_some: is_some.clone(),
                        },
                    );
                    return Ok(Eval::Scalar(is_some));
                }
                ("checked_add", Eval::Scalar(left), 1)
                    if self.sort(&left) == PanicSort::U8
                        && self.grounded_u8(&left, &mut BTreeSet::new()) =>
                {
                    if let Some(right) = self.scalar(args[0], Some(PanicSort::U8))? {
                        let is_some = self.expression(
                            n,
                            PanicSort::Bool,
                            PanicExpressionDerivation::U8AddFits {
                                left: left.clone(),
                                right: right.clone(),
                            },
                        );
                        let value = self.expression(
                            n,
                            PanicSort::U8,
                            PanicExpressionDerivation::U8Add {
                                left: left.clone(),
                                right: right.clone(),
                            },
                        );
                        self.site(
                            n,
                            PanicSiteSemantics::CheckedAdd {
                                left,
                                right,
                                is_some: is_some.clone(),
                                value: value.clone(),
                            },
                        );
                        return Ok(Eval::Option(is_some, value));
                    }
                }
                _ => {}
            }
            for arg in args {
                self.eval(arg)?;
            }
            return Ok(self.opaque_call(n));
        }
        let helper = self
            .root_helper(target)
            .filter(|h| h.result.is_some() && h.parameters.len() == args.len())
            .cloned();
        if let Some(helper) = helper {
            let mut arguments = vec![];
            for ((_, sort), arg) in helper.parameters.iter().zip(&args) {
                if let Some(e) = self.scalar(*arg, Some(*sort))? {
                    arguments.push(e);
                } else {
                    return Ok(self.opaque_call(n));
                }
            }
            let site_id = self
                .calls
                .sites
                .iter()
                .find(|s| s.body_id == self.body.body_id && s.byte_range == range(n))
                .unwrap()
                .site_id
                .clone();
            self.step(n);
            let binding = self.id("call-result");
            let value = self.value(
                &binding,
                helper.result.unwrap(),
                n,
                PanicValueOrigin::CallResult { site_id },
            );
            self.site(
                n,
                PanicSiteSemantics::DirectCall {
                    callee_body_id: helper.body_id,
                    arguments,
                    result_value_id: value.clone(),
                },
            );
            return Ok(Eval::Scalar(self.read(n, &value)));
        }
        for arg in args {
            self.eval(arg)?;
        }
        Ok(self.opaque_call(n))
    }
    fn index(&mut self, n: Node<'_>) -> anyhow::Result<Eval> {
        let parts = children(n);
        if parts.len() != 2 {
            self.unknown(n, PanicUnknownKind::Opaque, "unsupported index shape");
            return Ok(Eval::Unknown);
        }
        let array = self.eval(parts[0])?;
        let index_node = parts[1];
        let index = if index_node.kind() == "type_cast_expression"
            && index_node
                .child_by_field_name("type")
                .is_some_and(|t| syntax(t, self.text) == "usize")
        {
            let operand = index_node.child_by_field_name("value").unwrap();
            self.scalar(operand, Some(PanicSort::U8))?
        } else {
            self.scalar(index_node, Some(PanicSort::U8))?
        };
        if let (Eval::Array(elements), Some(index)) = (array, index) {
            let result = self.expression(
                n,
                PanicSort::U8,
                PanicExpressionDerivation::U8ArrayIndex {
                    elements: elements.clone(),
                    index: index.clone(),
                },
            );
            let operation_id = self.id("operation");
            let context = self.context(n);
            self.facts
                .extraction
                .as_mut()
                .unwrap()
                .operations
                .push(PanicImplicitOperation {
                    operation_id,
                    context,
                    result_expression_id: result.clone(),
                    operation: PanicImplicitOperationKind::Index { elements, index },
                });
            Ok(Eval::Scalar(result))
        } else {
            self.unknown(
                n,
                PanicUnknownKind::Opaque,
                "index requires bounded constant u8 array and lossless u8 index",
            );
            Ok(Eval::Unknown)
        }
    }
    fn literal_loop(&mut self, n: Node<'_>) -> anyhow::Result<Eval> {
        let pattern = n.child_by_field_name("pattern");
        let interval = n.child_by_field_name("value");
        let body = n.child_by_field_name("body");
        let (Some(pattern), Some(interval), Some(body)) = (pattern, interval, body) else {
            self.taint(n, "unsupported for loop");
            return Ok(Eval::Unknown);
        };
        let endpoints = children(interval);
        let values: Option<Vec<u8>> = endpoints
            .iter()
            .map(|n| {
                if n.kind() == "integer_literal" {
                    literal(&self.text[n.byte_range()])
                } else {
                    None
                }
            })
            .collect();
        if pattern.kind() != "identifier"
            || interval.kind() != "range_expression"
            || values.as_ref().is_none_or(|v| v.len() != 2)
            || !endpoints
                .iter()
                .any(|e| literal_u8_suffix(&self.text[e.byte_range()]))
        {
            self.taint(
                n,
                "for loop requires literal u8 range and simple induction binding",
            );
            return Ok(Eval::Unknown);
        }
        let values = values.unwrap();
        let start = self.state();
        let context = self.context(n);
        let loop_id = self.id("loop");
        let obligation_id = self.unknown(
            n,
            PanicUnknownKind::LoopBound,
            "literal loop template requires caller-bounded R6 exploration",
        );
        self.step(body);
        let entry = self.flow.clone();
        let binding = self.id("induction");
        let induction = self.value(
            &binding,
            PanicSort::U8,
            body,
            PanicValueOrigin::LoopVariable {
                loop_id: loop_id.clone(),
            },
        );
        self.env.insert(
            ident(&self.text[pattern.byte_range()]).into(),
            Binding::Scalar(induction.clone()),
        );
        self.eval(body)?;
        let exit = self.alive.then(|| self.flow.clone());
        let template = self.state();
        self.restore(&start);
        self.step(n);
        for (name, old) in &start.env {
            if template.env.get(name) != Some(old) {
                self.env.insert(name.clone(), Binding::Unknown);
                self.unknown(
                    n,
                    PanicUnknownKind::Mutation,
                    "loop-carried mutation requires exploration",
                );
            }
        }
        self.facts
            .extraction
            .as_mut()
            .unwrap()
            .loops
            .push(PanicLiteralLoop {
                loop_id,
                context,
                start: values[0],
                end: values[1],
                inclusive: syntax(interval, self.text).contains("..="),
                induction_value_id: induction,
                iteration_entry_flow_id: entry,
                iteration_exit_flow_id: exit,
                obligation_id,
            });
        Ok(Eval::Unit)
    }
}

fn domain(sort: PanicSort) -> PanicValueDomain {
    match sort {
        PanicSort::Bool => PanicValueDomain::Bool {
            values: vec![false, true],
        },
        PanicSort::U8 => PanicValueDomain::U8 {
            minimum: 0,
            maximum: 255,
        },
    }
}
fn literal(s: &str) -> Option<u8> {
    let s = s.strip_suffix("u8").unwrap_or(s).replace('_', "");
    if let Some(s) = s.strip_prefix("0x") {
        u8::from_str_radix(s, 16).ok()
    } else if let Some(s) = s.strip_prefix("0o") {
        u8::from_str_radix(s, 8).ok()
    } else if let Some(s) = s.strip_prefix("0b") {
        u8::from_str_radix(s, 2).ok()
    } else {
        s.parse().ok()
    }
}
fn literal_u8_suffix(s: &str) -> bool {
    s.ends_with("u8") && literal(s).is_some()
}
