use spur_graph::{build_facts, extract::GraphFacts, NodeId, NodeKind, RelationKind};

const HELD_OUT: &str =
    include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation/R1/held-out.rs");

fn build(case: &str, source: &str) -> GraphFacts {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("lib.rs"), source).expect("write fixture");
    let facts = build_facts(dir.path(), None).expect("build facts").0;
    if let Some(output) = std::env::var_os("SPUR_R1_EXPORT_DIR") {
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

fn function(facts: &GraphFacts, source: &str, declaration: &str) -> NodeId {
    let start = source.find(declaration).expect("declaration") as u32;
    facts
        .nodes
        .iter()
        .find(|node| {
            node.kind == NodeKind::Function
                && facts.spans.iter().any(|span| {
                    Some(span.span_id) == node.source_span_id && span.start_byte == start
                })
        })
        .unwrap_or_else(|| panic!("missing function at {start}: {declaration}"))
        .node_id
}

fn targets(facts: &GraphFacts, owner: NodeId, name: &str) -> Vec<Option<NodeId>> {
    let mut targets: Vec<_> = facts
        .edges
        .iter()
        .filter(|edge| {
            edge.relation == RelationKind::Calls
                && edge.source_node_id == owner
                && edge.target_label.as_deref() == Some(name)
        })
        .map(|edge| edge.target_node_id)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

#[test]
fn held_out_recursion_and_homonyms_keep_exact_owners() {
    let facts = build("held_out", HELD_OUT);
    for (declaration, name) in [
        ("pub fn plain", "plain"),
        ("pub fn shadowed", "shadowed"),
        ("pub fn qualified", "qualified"),
    ] {
        let owner = function(&facts, HELD_OUT, declaration);
        assert!(
            targets(&facts, owner, name).contains(&Some(owner)),
            "missing recursive Calls edge for {declaration}"
        );
    }
    let nested = function(&facts, HELD_OUT, "pub fn nested");
    let local = function(&facts, HELD_OUT, "fn nested(local");
    assert_eq!(targets(&facts, nested, "nested"), vec![Some(local)]);
    let same = function(&facts, HELD_OUT, "pub fn same(n");
    let other = function(&facts, HELD_OUT, "pub fn same(value");
    assert_eq!(targets(&facts, same, "same"), vec![Some(other)]);
}

#[test]
fn isolated_lexical_binding_intervals_survive_renaming() {
    for name in ["recur", "renamed"] {
        for (case, body, recursive) in [
            ("plain", "NAME();", true),
            ("before", "NAME(); let NAME = || {};", true),
            ("after_block", "{ let NAME = || {}; } NAME();", true),
            ("qualified", "let NAME = || {}; self::NAME();", true),
            ("shadow", "let NAME = || {}; NAME();", false),
            ("inner_shadow", "{ let NAME = || {}; NAME(); }", false),
            // A let binding starts after its initializer, not at its pattern.
            ("initializer", "let NAME = { NAME(); || {} };", true),
        ] {
            let source = format!("fn {name}() {{ {} }}", body.replace("NAME", name));
            let facts = build(&format!("{case}_{name}"), &source);
            let owner = function(&facts, &source, &format!("fn {name}"));
            assert_eq!(
                targets(&facts, owner, name),
                if recursive {
                    vec![Some(owner)]
                } else {
                    vec![None]
                },
                "wrong binding in {case}_{name}: {source}"
            );
        }
    }
}

#[test]
fn local_items_are_hoisted_only_within_their_own_block() {
    let source = "fn recur() { { recur(); fn recur() {} } recur(); }";
    let facts = build("local_block", source);
    let outer = function(&facts, source, "fn recur()");
    let inner = function(&facts, source, "fn recur() {}");
    assert_eq!(
        targets(&facts, outer, "recur"),
        vec![Some(outer), Some(inner)]
    );
    assert!(targets(&facts, inner, "recur").is_empty());
}

#[test]
fn parameters_and_patterns_block_same_name_function_fallbacks() {
    for (case, source) in [
        ("parameter", "fn recur(recur: fn()) { recur(); }"),
        (
            "tuple",
            "fn recur() { let (recur, _) = (|| {}, 0); recur(); }",
        ),
        (
            "closure_parameter",
            "fn recur() { let c = |recur: fn()| { recur(); }; }",
        ),
        (
            "for_pattern",
            "fn recur() { for recur in [|| {}] { recur(); } }",
        ),
        (
            "match_pattern",
            "fn recur() { match Some(|| {}) { Some(recur) => recur(), _ => () } }",
        ),
    ] {
        let facts = build(case, source);
        let owner = function(&facts, source, "fn recur");
        assert_eq!(targets(&facts, owner, "recur"), vec![None], "{case}");
    }
}

#[test]
fn qualified_self_and_super_calls_ignore_value_shadows() {
    let source = "mod m { pub fn recur() { let recur = || {}; self::recur(); } mod child { fn caller() { super::recur(); } } }";
    let facts = build("module_paths", source);
    let recur = function(&facts, source, "pub fn recur");
    let caller = function(&facts, source, "fn caller");
    assert_eq!(targets(&facts, recur, "recur"), vec![Some(recur)]);
    assert_eq!(targets(&facts, caller, "recur"), vec![Some(recur)]);
}

#[test]
fn conditional_bindings_cover_the_body_but_exclude_initializer_and_else() {
    for (case, body, recursive) in [
        (
            "if_let_body",
            "if let Some(recur) = Some(|| {}) { recur(); }",
            false,
        ),
        (
            "if_let_else",
            "if let Some(recur) = Some(|| {}) {} else { recur(); }",
            true,
        ),
        (
            "if_let_initializer",
            "if let Some(recur) = { recur(); Some(|| {}) } {}",
            true,
        ),
        (
            "while_let_body",
            "while let Some(recur) = Some(|| {}) { recur(); break; }",
            false,
        ),
        ("const_item", "const recur: fn() = || {}; recur();", false),
        (
            "match_guard",
            "match 0 { n if { recur(); true } => (), _ => () }",
            true,
        ),
    ] {
        let source = format!("fn recur() {{ {body} }}");
        let facts = build(case, &source);
        let owner = function(&facts, &source, "fn recur");
        assert_eq!(
            targets(&facts, owner, "recur"),
            if recursive {
                vec![Some(owner)]
            } else {
                vec![None]
            },
            "{case}"
        );
    }
}

#[test]
fn imported_and_constructor_bindings_do_not_become_self_calls() {
    for (case, source) in [
        (
            "imported_item",
            "mod other { pub fn recur() {} } fn recur() { use other::recur; recur(); }",
        ),
        (
            "aliased_item",
            "mod other { pub fn helper() {} } fn recur() { use other::helper as recur; recur(); }",
        ),
        (
            "constructor_item",
            "fn recur() { struct recur(); recur(); }",
        ),
    ] {
        let facts = build(case, source);
        let declaration = if case == "constructor_item" {
            "fn recur() { struct"
        } else {
            "fn recur() { use"
        };
        let owner = function(&facts, source, declaration);
        assert!(
            !targets(&facts, owner, "recur").contains(&Some(owner)),
            "{case}"
        );
        if case == "imported_item" {
            let other = function(&facts, source, "pub fn recur");
            assert_eq!(targets(&facts, owner, "recur"), vec![Some(other)]);
        }
    }
}

#[test]
fn generic_free_self_calls_keep_exact_owners() {
    for name in ["recur", "renamed"] {
        let source = format!("fn {name}<T>() {{ {name}::<T>(); }}");
        let facts = build(&format!("generic_{name}"), &source);
        let owner = function(&facts, &source, &format!("fn {name}"));
        assert_eq!(targets(&facts, owner, name), vec![Some(owner)]);
    }
}

#[test]
fn qualified_generic_free_calls_keep_exact_owners() {
    for qualifier in ["self", "crate"] {
        let source = format!("fn recur<T>() {{ {qualifier}::recur::<T>(); }}");
        let facts = build(&format!("generic_{qualifier}"), &source);
        let owner = function(&facts, &source, "fn recur");
        assert_eq!(targets(&facts, owner, "recur"), vec![Some(owner)]);
    }
    let source =
        "mod a { pub fn recur<T>() { self::recur::<T>(); } } fn caller() { a::recur::<u8>(); }";
    let facts = build("generic_module", source);
    let recur = function(&facts, source, "pub fn recur");
    let caller = function(&facts, source, "fn caller");
    assert_eq!(targets(&facts, recur, "recur"), vec![Some(recur)]);
    assert_eq!(targets(&facts, caller, "recur"), vec![Some(recur)]);
}

fn assert_no_outer_module_target(case: &str, source: &str) {
    let facts = build(case, source);
    let owner = function(&facts, source, "fn caller");
    let outer = function(&facts, source, "pub fn ping");
    let observed = targets(&facts, owner, "ping");
    assert!(
        !observed.contains(&Some(outer)),
        "{case}: wrong outer module target"
    );
    // Unsupported namespace bindings must retain explicit unresolved evidence.
    assert!(observed.contains(&None), "{case}: missing unresolved call");
}

#[test]
fn local_module_alias_must_not_bind_outer_module() {
    assert_no_outer_module_target(
        "module_alias",
        "mod a { pub fn ping() {} } mod b { pub fn ping() {} } fn caller() { use b as a; a::ping(); }",
    );
}

#[test]
fn local_type_must_not_bind_outer_module() {
    assert_no_outer_module_target(
        "local_type",
        "mod a { pub fn ping() {} } fn caller() { struct a; impl a { fn ping() {} } a::ping(); }",
    );
}

#[test]
fn type_parameter_must_not_bind_outer_module() {
    assert_no_outer_module_target(
        "type_parameter",
        "mod a { pub fn ping() {} } trait P { fn ping(); } fn caller<a: P>() { a::ping(); }",
    );
}

#[test]
fn namespace_items_shadow_for_the_whole_lexical_block() {
    for (case, source) in [
        ("hoisted_alias", "mod a { pub fn ping() {} } mod b { pub fn ping() {} } fn caller() { a::ping(); use b as a; }"),
        ("type_alias", "mod a { pub fn ping() {} } struct B; impl B { fn ping() {} } fn caller() { type a = B; a::ping(); }"),
    ] {
        assert_no_outer_module_target(case, source);
    }
    let source = "mod a { pub fn ping() {} } fn caller() { a::ping(); mod a { pub fn ping() { let local = 1; } } }";
    let facts = build("local_module", source);
    let owner = function(&facts, source, "fn caller");
    let local = function(&facts, source, "pub fn ping() { let local");
    assert_eq!(targets(&facts, owner, "ping"), vec![Some(local)]);
}

#[test]
fn module_paths_ignore_expired_or_value_bindings() {
    for (case, source) in [
        ("after_alias_block", "mod a { pub fn ping() {} } mod b { pub fn ping() {} } fn caller() { { use b as a; } a::ping(); }"),
        ("value_namespace", "mod a { pub fn ping() {} } fn caller() { let a = || {}; a::ping(); }"),
        ("explicit_self_module", "mod a { pub fn ping() {} } mod b { pub fn ping() {} } fn caller() { use b as a; self::a::ping(); }"),
    ] {
        let facts = build(case, source);
        let owner = function(&facts, source, "fn caller");
        let outer = function(&facts, source, "pub fn ping");
        assert_eq!(targets(&facts, owner, "ping"), vec![Some(outer)], "{case}");
    }
}

#[test]
fn generic_receiver_and_associated_calls_keep_existing_resolution() {
    for (case, source) in [
        ("generic_receiver", "struct Worker; impl Worker { fn hop<T>(&self) {} } fn caller(w: Worker) { w.hop::<u8>(); }"),
        ("generic_associated", "struct Worker; impl Worker { fn hop<T>() {} } fn caller() { Worker::hop::<u8>(); }"),
    ] {
        let facts = build(case, source);
        let owner = function(&facts, source, "fn caller");
        // The existing extractor does not emit these generic method calls.
        // Free-call normalization must leave that receiver/type path alone.
        assert!(
            !facts.edges.iter().any(|edge| edge.relation == RelationKind::Calls
                && edge.source_node_id == owner),
            "{case}: changed existing generic method handling"
        );
    }
}
