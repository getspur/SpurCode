use std::collections::HashMap;
use std::path::PathBuf;

use spur_graph::build_facts;
use spur_graph::store::build::artifact_from_facts;
use spur_graph::RelationKind;

fn corpus_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Allowed resolved-target `symbol_kind`s per guarded relation. Mirrors the resolver +
/// `rebind_cross_file_edges` union kind-sets. `None` = relation not range-checked here.
fn allowed_target_kinds(relation: RelationKind) -> Option<&'static [&'static str]> {
    match relation {
        RelationKind::Extends => Some(&["trait", "interface", "class"]),
        RelationKind::Implements => Some(&["trait", "interface"]),
        RelationKind::Calls => Some(&["function", "method"]),
        _ => None,
    }
}

#[test]
fn assembled_artifact_has_no_out_of_range_resolved_edges() {
    // (corpus dir, language label for diagnostics)
    let corpora = [
        ("sample_corpus", "rust"),
        ("python_corpus", "python"),
        ("typescript_corpus", "typescript"),
        ("cpp_corpus", "cpp"),
    ];

    let mut violations: Vec<String> = Vec::new();

    for (dir, lang) in corpora {
        let root = corpus_root(dir);
        let facts = build_facts(&root, None)
            .unwrap_or_else(|e| panic!("extract {lang} corpus: {e:?}"))
            .0;
        // artifact_from_facts runs rebind_cross_file_edges internally — this is the
        // assembled (post-rebind) artifact the CLI actually persists.
        let artifact = artifact_from_facts(&facts, &root)
            .unwrap_or_else(|e| panic!("assemble {lang} artifact: {e:?}"));

        let kind_by_id: HashMap<&str, &str> = artifact
            .symbols
            .iter()
            .map(|s| (s.stable_symbol_id.as_str(), s.symbol_kind.as_str()))
            .collect();

        for edge in &artifact.edges {
            let Some(allowed) = allowed_target_kinds(edge.relation) else {
                continue;
            };
            let Some(target_id) = edge.target_stable_symbol_id.as_deref() else {
                continue; // unresolved edges are correct; skip
            };
            let Some(target_kind) = kind_by_id.get(target_id).copied() else {
                continue; // dangling id (shouldn't happen); not this test's concern
            };
            if !allowed.contains(&target_kind) {
                violations.push(format!(
                    "[{lang}] {:?} edge resolves to out-of-range kind {target_kind:?} \
                     (target_label={:?}, allowed={allowed:?})",
                    edge.relation, edge.target_label
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "assembled artifact contains out-of-range resolved edges:\n{}",
        violations.join("\n")
    );
}

#[test]
fn artifact_rebind_preserves_cross_crate_singleton_function_unresolved() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("crates/source/src")).expect("source crate dir");
    std::fs::create_dir_all(dir.path().join("crates/callee/src")).expect("callee crate dir");
    std::fs::write(
        dir.path().join("crates/source/src/lib.rs"),
        r#"
pub fn caller() {
    cross_crate_helper();
}
"#,
    )
    .expect("write source lib");
    std::fs::write(
        dir.path().join("crates/callee/src/lib.rs"),
        "pub fn cross_crate_helper() {}\n",
    )
    .expect("write callee lib");

    let facts = build_facts(dir.path(), None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, dir.path()).expect("assemble artifact");

    let edge = artifact
        .edges
        .iter()
        .find(|edge| {
            edge.relation == RelationKind::Calls
                && edge.target_label.as_deref() == Some("cross_crate_helper")
        })
        .expect("cross-crate call edge");

    assert_eq!(edge.target_stable_symbol_id, None);
    assert_eq!(edge.bind_method.as_deref(), None);
}

#[test]
fn artifact_preserves_grounded_inherent_method_and_rejects_incompatible_receiver() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("crates/foo/src/sub")).expect("foo sub dir");
    std::fs::write(
        dir.path().join("crates/foo/src/a.rs"),
        r#"
pub struct Widget;
pub struct Panel;

impl Widget {
    pub fn repaint_panel(&self) {}
}

pub fn compatible(widget: &Widget) { widget.repaint_panel(); }
"#,
    )
    .expect("write foo a");
    std::fs::write(
        dir.path().join("crates/foo/src/sub/b.rs"),
        r#"
pub fn caller(panel: &Panel) {
    panel.repaint_panel();
}
"#,
    )
    .expect("write foo b");

    let facts = build_facts(dir.path(), None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, dir.path()).expect("assemble artifact");

    let edge = artifact
        .edges
        .iter()
        .find(|edge| {
            edge.relation == RelationKind::Calls
                && edge.target_label.as_deref() == Some("repaint_panel")
        })
        .expect("same-crate method call edge");

    let source = artifact.symbols.iter().find(|s|s.stable_symbol_id == edge.source_stable_symbol_id).unwrap();
    let incompatible = artifact.edges.iter().find(|e|e.target_label.as_deref() == Some("repaint_panel") && artifact.symbols.iter().any(|s|s.stable_symbol_id == e.source_stable_symbol_id && s.entity_name == "caller")).unwrap();
    assert_eq!(incompatible.target_stable_symbol_id, None);
    assert_eq!(incompatible.bind_method, None);
    let compatible = artifact.edges.iter().find(|e|e.target_label.as_deref() == Some("repaint_panel") && artifact.symbols.iter().any(|s|s.stable_symbol_id == e.source_stable_symbol_id && s.entity_name == "compatible")).unwrap();
    assert!(compatible.target_stable_symbol_id.is_some(), "{source:?}");
    assert_eq!(compatible.bind_method.as_deref(), Some("scope_match"));
}

#[test]
fn artifact_rebind_preserves_cross_crate_singleton_reference_unresolved() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("crates/source/src")).expect("source crate dir");
    std::fs::create_dir_all(dir.path().join("crates/callee/src")).expect("callee crate dir");
    std::fs::write(
        dir.path().join("crates/source/src/lib.rs"),
        r#"
pub fn caller(items: Vec<i32>) {
    let _ = items.iter().copied().map(cross_crate_mapper).collect::<Vec<_>>();
}
"#,
    )
    .expect("write source lib");
    std::fs::write(
        dir.path().join("crates/callee/src/lib.rs"),
        "pub fn cross_crate_mapper(value: i32) -> i32 { value }\n",
    )
    .expect("write callee lib");

    let facts = build_facts(dir.path(), None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, dir.path()).expect("assemble artifact");

    let edge = artifact
        .edges
        .iter()
        .find(|edge| {
            edge.relation == RelationKind::References
                && edge.target_label.as_deref() == Some("cross_crate_mapper")
        })
        .expect("cross-crate reference edge");

    assert_eq!(edge.target_stable_symbol_id, None);
    assert_eq!(edge.bind_method.as_deref(), None);
}
