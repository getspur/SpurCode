use spur_graph::{build_facts, extract::GraphFacts, NodeId, RelationKind};

const RECEIVERS: &str = include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/fixtures/receivers/lib.rs");
const OPTIONS: &str = include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/semantics/fixtures/lib.rs");

const SOURCE: &str = r#"
struct Alpha;
struct Beta;
struct Pretender;
struct Lookalike;
struct Tuple(u8);
struct Record { x: u8 }
impl Alpha { fn mark(&self) { /* Alpha */ } }
impl Beta { fn mark(&self) { /* Beta */ } }
impl Pretender { fn len(&self) {} }
impl Lookalike { fn unwrap(&self) {} }
impl Tuple { fn mark(&self) { /* Tuple */ } }
impl Record { fn mark(&self) { /* Record */ } }
fn string_len(text: &str) { text.len(); }
fn owned_string(text: String) { text.len(); }
fn literal_len() { "abc".len(); }
fn option(value: Option<u8>) { value.unwrap(); }
fn custom(value: Lookalike) { value.unwrap(); }
fn explicit_alpha(value: Alpha) { value.mark(); }
fn borrowed_beta(value: &Beta) { value.mark(); }
fn unit_local() { let value = Alpha; value.mark(); }
fn tuple_local() { let value = Tuple(0); value.mark(); }
fn struct_local() { let value = Record { x: 0 }; value.mark(); }
fn borrowed_local() { let b = Beta; let value = &b; value.mark(); }
fn rebound_local() { let value = Alpha; let value = Beta; value.mark(); }
fn inner_shadow() { let value = Alpha; { let value = Beta; value.mark(); } }
fn outer_interval() { let value = Alpha; { let value = Beta; } value.mark(); }
fn initializer_interval() { let value = Alpha; let value = { value.mark(); Beta }; }
fn typed_local() { let value: Alpha = opaque(); value.mark(); }
fn generic_receiver<T>(value: T) { value.mark(); }
fn opaque_origin() { let value = opaque(); value.mark(); }
fn autoderef(value: Box<Alpha>) { value.mark(); }
fn closure_shadow(value: Alpha) { let _ = |value| value.mark(); }
fn for_shadow(value: Alpha) { for value in opaque() { value.mark(); } }
fn match_shadow(value: Alpha) { match opaque() { Some(value) => value.mark(), _ => () } }
fn iflet_shadow(value: Alpha) { if let Some(value) = opaque() { value.mark(); } }
fn outside() { let value = Alpha; fn nested_function() { value.mark(); } }
fn type_parameter<Alpha>(value: Alpha) { value.mark(); }
type Alias = Alpha;
fn alias_type(value: Alias) { value.mark(); }
struct TraitOnly;
trait Mark { fn mark(&self); }
impl Mark for TraitOnly { fn mark(&self) {} }
fn trait_only(value: TraitOnly) { value.mark(); }
struct Ambiguous;
impl Ambiguous { fn mark(&self) {} }
impl Ambiguous { fn mark(&self) {} }
fn ambiguous_impl(value: Ambiguous) { value.mark(); }
mod nested {
    pub struct Alpha;
    impl Alpha { pub fn mark(&self) { /* nested Alpha */ } }
    fn module_receiver(value: Alpha) { value.mark(); }
}
fn pretender(value: Pretender) { value.len(); }
"#;

fn build(case: &str, source: &str) -> GraphFacts {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), source).unwrap();
    let facts = build_facts(dir.path(), None).unwrap().0;
    if let Some(output) = std::env::var_os("SPUR_R2_EXPORT_DIR") {
        let output = std::path::PathBuf::from(output);
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(
            output.join(format!("{case}.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "source": source, "nodes": facts.nodes,
                "edges": facts.edges, "spans": facts.spans,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    facts
}

fn definition(facts: &GraphFacts, source: &str, declaration: &str) -> NodeId {
    let start = source.find(declaration).unwrap() as u32;
    facts
        .nodes
        .iter()
        .find(|node| {
            facts
                .spans
                .iter()
                .any(|span| Some(span.span_id) == node.source_span_id && span.start_byte == start)
        })
        .unwrap_or_else(|| panic!("missing definition: {declaration}"))
        .node_id
}

fn targets(facts: &GraphFacts, owner: NodeId, method: &str) -> Vec<Option<NodeId>> {
    let mut result: Vec<_> = facts
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::Calls
                && e.source_node_id == owner
                && e.target_label.as_deref() == Some(method)
        })
        .map(|e| e.target_node_id)
        .collect();
    result.sort();
    result.dedup();
    result
}

#[test]
fn standard_receivers_never_bind_to_local_method_homonyms() {
    let facts = build("standard", SOURCE);
    for (name, method) in [
        ("string_len", "len"),
        ("owned_string", "len"),
        ("literal_len", "len"),
        ("option", "unwrap"),
    ] {
        let caller = definition(&facts, SOURCE, &format!("fn {name}"));
        assert_eq!(targets(&facts, caller, method), vec![None], "{name}");
    }
}

#[test]
fn later_typed_shadows_cannot_retype_standard_receivers() {
    let mut mismatches = Vec::new();
    for (case, source, method) in [
        ("string_shadow", "struct Pretender; impl Pretender { fn len(&self) {} } fn caller(text: &str) { text.len(); { let text: Pretender = Pretender; } }", "len"),
        ("option_shadow", "struct Lookalike; impl Lookalike { fn unwrap(&self) {} } fn caller(value: Option<u8>) { value.unwrap(); { let value: Lookalike = Lookalike; } }", "unwrap"),
    ] {
        let facts = build(case, source);
        let owner = definition(&facts, source, "fn caller");
        let observed = targets(&facts, owner, method);
        if observed != vec![None] { mismatches.push((case, observed)); }
    }
    assert!(
        mismatches.is_empty(),
        "incompatible targets: {mismatches:?}"
    );
}

#[test]
fn exact_receiver_origins_and_shadow_intervals_select_only_their_impl() {
    let facts = build("matrix", SOURCE);
    let oracle: Vec<[String; 3]> = serde_json::from_str(include_str!(
        "../../../docs/research/artifacts/2026-11-01-panic-implementation/R2/oracle.json"
    ))
    .unwrap();
    for [caller, _, target] in oracle {
        let owner = definition(&facts, SOURCE, &format!("fn {caller}"));
        let method = if ["string_len", "owned_string", "literal_len", "pretender"]
            .contains(&caller.as_str())
        {
            "len"
        } else if ["option", "custom"].contains(&caller.as_str()) {
            "unwrap"
        } else {
            "mark"
        };
        let expected = if target == "unresolved" {
            None
        } else {
            let (ty, _) = target.rsplit_once("::").unwrap();
            let search = if ty == "nested::Alpha" {
                &SOURCE[SOURCE.find("mod nested").unwrap()..]
            } else {
                SOURCE
            };
            let declaration = &search[search
                .find(&format!("impl {}", ty.rsplit("::").next().unwrap()))
                .unwrap()..];
            let declaration = &declaration[..declaration.find('}').unwrap() + 1];
            let method_declaration = &declaration[declaration
                .find("pub fn ")
                .or_else(|| declaration.find("fn "))
                .unwrap()..];
            Some(definition(&facts, SOURCE, method_declaration))
        };
        assert_eq!(
            targets(&facts, owner, method),
            vec![expected],
            "{caller}: {target}"
        );
        if target != "unresolved" {
            let receiver_type = target.rsplit_once("::").unwrap().0;
            assert!(
                facts
                    .edges
                    .iter()
                    .any(|edge| edge.relation == RelationKind::Calls
                        && edge.source_node_id == owner
                        && edge.target_node_id == expected
                        && edge.scope_text.as_deref() == Some(receiver_type)),
                "{caller}: missing nominal receiver provenance"
            );
        }
    }
}

#[test]
fn a_shadowed_tuple_constructor_is_an_unknown_function_result() {
    let source = "struct Tuple(u8); impl Tuple { fn mark(&self) {} } fn caller() { fn Tuple(_: u8) -> Unknown { opaque() } let value = Tuple(0); value.mark(); }";
    let facts = build("constructor_shadow", source);
    let owner = definition(&facts, source, "fn caller");
    assert_eq!(targets(&facts, owner, "mark"), vec![None]);
}

#[test]
fn held_out_option_sites_reject_lookalike_but_preserve_custom_unwrap() {
    let facts = build("held_out_options", OPTIONS);
    let custom = definition(&facts, OPTIONS, "pub fn unwrap");
    let caller = definition(&facts, OPTIONS, "pub fn lookalike");
    assert_eq!(targets(&facts, caller, "unwrap"), vec![Some(custom)]);
    // Eleven physical Option sites collapse to ten caller/label graph pairs.
    assert_eq!(OPTIONS.matches(".unwrap()").count(), 12);
    let option_edges: Vec<_> = facts
        .edges
        .iter()
        .filter(|e| {
            e.relation == RelationKind::Calls
                && e.target_label.as_deref() == Some("unwrap")
                && e.source_node_id != caller
        })
        .collect();
    assert_eq!(option_edges.len(), 10);
    assert!(
        option_edges.iter().all(|e| e.target_node_id.is_none()),
        "Option bound to Lookalike: {option_edges:?}"
    );
}

#[test]
fn held_out_receiver_corpus_preserves_typed_borrowed_and_constructor_targets() {
    let facts = build("held_out_receivers", RECEIVERS);
    let caller = definition(&facts, RECEIVERS, "pub fn receivers");
    let alpha = definition(&facts, RECEIVERS, "pub fn mark(&self) -> usize { 11 }");
    let beta = definition(&facts, RECEIVERS, "pub fn mark(&self) -> usize { 22 }");
    assert_eq!(targets(&facts, caller, "mark"), {
        let mut v = vec![Some(alpha), Some(beta)];
        v.sort();
        v
    });
    let custom = definition(&facts, RECEIVERS, "pub fn len");
    let mut expected = vec![None, Some(custom)];
    expected.sort();
    assert_eq!(targets(&facts, caller, "len"), expected);
}

// Hand-authored Rust inputs and PRE expectations, never generated graph goldens.
fn dispatch_cases() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!(
        "../../../docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-2/cases.json"
    ))
    .unwrap()
}

fn dispatch_observation(case: &serde_json::Value) -> (Vec<Option<NodeId>>, Option<NodeId>) {
    let name = case["case"].as_str().unwrap();
    let source = case["source"].as_str().unwrap();
    let method = case["method"].as_str().unwrap();
    let facts = build(name, source);
    let owner = definition(&facts, source, "fn caller");
    let expected = (case["target"] != "unresolved").then(|| {
        // Each positive input has one inherent method before any trait items.
        definition(&facts, source, &format!("fn {method}(&self)"))
    });
    let edges: Vec<_> = facts
        .edges
        .iter()
        .filter(|edge| {
            edge.relation == RelationKind::Calls
                && edge.source_node_id == owner
                && edge.target_label.as_deref() == Some(method)
        })
        .collect();
    assert_eq!(
        edges.len(),
        1,
        "{name}: preserve the physical call observation"
    );
    assert_eq!(
        edges[0].scope_text.as_deref(),
        case["receiver"].as_str(),
        "{name}: preserve nominal evidence even when dispatch is unsupported"
    );
    assert_eq!(
        edges[0].receiver_text.as_deref(),
        Some("value"),
        "{name}: preserve the physical receiver text"
    );
    (targets(&facts, owner, method), expected)
}

fn assert_dispatch_case(name: &str) {
    let cases = dispatch_cases();
    let case = cases.iter().find(|case| case["case"] == name).unwrap();
    let (observed, expected) = dispatch_observation(case);
    assert_eq!(
        observed,
        vec![expected],
        "{name}: unsupported trait/autoref dispatch"
    );
}

#[test]
fn owned_receiver_trait_borrow_precedes_mutable_inherent() {
    assert_dispatch_case("owned_mut");
}

#[test]
fn borrowed_receiver_trait_borrow_precedes_mutable_inherent() {
    assert_dispatch_case("borrowed_mut");
}

#[test]
fn owned_receiver_trait_value_precedes_shared_inherent() {
    assert_dispatch_case("owned_value");
}

#[test]
fn double_reference_trait_receiver_precedes_shared_inherent() {
    assert_dispatch_case("double_ref");
}

#[test]
fn renamed_bounded_default_and_constructor_trait_dispatch_remains_unresolved() {
    let mut mismatches = Vec::new();
    for case in dispatch_cases().iter().filter(|case| {
        case["target"] == "unresolved"
            && !["owned_mut", "borrowed_mut", "owned_value", "double_ref"]
                .contains(&case["case"].as_str().unwrap())
    }) {
        let (observed, expected) = dispatch_observation(case);
        if observed != vec![expected] {
            mismatches.push((case["case"].clone(), observed));
        }
    }
    assert!(
        mismatches.is_empty(),
        "false inherent dispatch: {mismatches:?}"
    );
}

#[test]
fn trait_dispatch_guard_preserves_grounded_inherent_targets() {
    for case in dispatch_cases()
        .iter()
        .filter(|case| case["target"] != "unresolved")
    {
        let (observed, expected) = dispatch_observation(case);
        assert_eq!(observed, vec![expected], "{}", case["case"]);
    }
}

fn expansion_cases() -> Vec<serde_json::Value> {
    // Independent source fixtures and expected outcomes, authored before RED.
    serde_json::from_str(include_str!(
        "../../../docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-3/cases.json"
    ))
    .unwrap()
}

fn assert_expansion_case(name: &str) {
    let cases = expansion_cases();
    let case = cases.iter().find(|case| case["case"] == name).unwrap();
    let (observed, expected) = dispatch_observation(case);
    assert_eq!(
        observed,
        vec![expected],
        "{name}: expansion dispatch is unsupported"
    );
}

#[test]
fn derived_clone_must_not_select_mutable_inherent_clone() {
    assert_expansion_case("derive_clone");
}

#[test]
fn item_macro_impl_must_not_select_mutable_inherent_method() {
    assert_expansion_case("macro_top");
}

#[test]
fn expansion_uncertainty_covers_attributes_includes_and_macro_statements() {
    let mut mismatches = Vec::new();
    for case in expansion_cases().iter().filter(|case| {
        case["target"] == "unresolved"
            && !["derive_clone", "macro_top"].contains(&case["case"].as_str().unwrap())
    }) {
        let (observed, expected) = dispatch_observation(case);
        if observed != vec![expected] {
            mismatches.push((case["case"].clone(), observed));
        }
    }
    assert!(
        mismatches.is_empty(),
        "false inherent dispatch after expansion: {mismatches:?}"
    );
}

#[test]
fn builtin_attributes_and_uninvoked_macro_tokens_preserve_grounded_methods() {
    for case in expansion_cases()
        .iter()
        .filter(|case| case["target"] != "unresolved")
    {
        let (observed, expected) = dispatch_observation(case);
        assert_eq!(observed, vec![expected], "{}", case["case"]);
    }
}
