use std::collections::BTreeSet;

use spur_graph::panic_evidence::{
    collect_method_call_sites, CompilerTargetState, MethodCallLexicalPhase,
    MethodCallSiteEvidenceSet, PanicEvidenceSource,
};
use spur_graph::{artifact_from_facts, build_facts, git_blob_oid, GraphIndexArtifact};

const SOURCE: &str = r#"
struct Safe;
impl Safe { fn unwrap(&self) -> u8 { 7 } }
fn repeat(a: Option<u8>, b: Option<u8>, safe: Safe) {
    let _ = a.unwrap(); let _ = b.unwrap(); let _ = a.unwrap();
    let _ = safe.unwrap();
}
"#;

fn graph(files: &[(&str, &str)]) -> anyhow::Result<GraphIndexArtifact> {
    let root = tempfile::tempdir()?;
    for (path, source) in files {
        let path = root.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, source)?;
    }
    let (facts, _) = build_facts(root.path(), None)?;
    artifact_from_facts(&facts, root.path())
}

fn sources(source: &str) -> [PanicEvidenceSource<'_>; 1] {
    [PanicEvidenceSource {
        file_path: "src/lib.rs",
        bytes: source.as_bytes(),
    }]
}

fn fixture() -> anyhow::Result<(GraphIndexArtifact, MethodCallSiteEvidenceSet)> {
    let graph = graph(&[("src/lib.rs", SOURCE)])?;
    let set = collect_method_call_sites(&graph, &sources(SOURCE))?;
    Ok((graph, set))
}

#[test]
fn repeated_calls_remain_individual_source_bound_sites() -> anyhow::Result<()> {
    let (graph, set) = fixture()?;
    assert_eq!(set.sites.len(), 4, "each direct invocation needs a site");
    assert_eq!(
        set.sites
            .iter()
            .map(|s| &s.site_id)
            .collect::<BTreeSet<_>>()
            .len(),
        4
    );
    assert_eq!(set.sources.len(), 1);
    assert_eq!(set.sources[0].site_count, 4);
    assert!(!set.sources[0].has_parse_errors);
    let caller = graph
        .symbols
        .iter()
        .find(|s| s.entity_name == "repeat")
        .unwrap();
    let expected = [
        ([123, 133], "a"),
        ([143, 153], "b"),
        ([163, 173], "a"),
        ([187, 200], "safe"),
    ];
    for (site, (bytes, receiver)) in set.sites.iter().zip(expected) {
        assert_eq!(site.byte_range, bytes);
        assert_eq!(site.call_text, SOURCE[bytes[0]..bytes[1]]);
        assert_eq!(
            &SOURCE[site.method_byte_range[0]..site.method_byte_range[1]],
            "unwrap"
        );
        assert_eq!(site.method_name, "unwrap");
        assert_eq!(site.receiver_text, receiver);
        assert_eq!(
            &SOURCE[site.receiver_byte_range[0]..site.receiver_byte_range[1]],
            receiver
        );
        assert_eq!(site.source_oid, git_blob_oid(SOURCE.as_bytes()));
        assert_eq!(
            site.caller_stable_symbol_id.as_deref(),
            Some(caller.stable_symbol_id.as_str())
        );
        assert_eq!(
            site.lexical_phase,
            MethodCallLexicalPhase::FunctionBodySyntax
        );
        assert_eq!(site.compiler_target, CompilerTargetState::Unknown);
    }
    set.validate(&graph, &sources(SOURCE))?;
    println!("P1_OBSERVED={}", serde_json::to_string(&set)?);
    Ok(())
}

#[test]
fn chains_generics_utf8_and_nonpanic_method_names_are_exact() -> anyhow::Result<()> {
    let source = "fn generic<T>(a: T) { let café = a; café.first::<u8>().second().r#type();\n café\n .third(); }\n";
    let graph = graph(&[("src/lib.rs", source)])?;
    let set = collect_method_call_sites(&graph, &sources(source))?;
    assert_eq!(
        set.sites.len(),
        4,
        "chains and generic methods are direct calls"
    );
    let by_name = |name| set.sites.iter().find(|s| s.method_name == name).unwrap();
    assert_eq!(by_name("first").receiver_text, "café");
    assert_eq!(by_name("second").receiver_text, "café.first::<u8>()");
    assert_eq!(
        by_name("r#type").receiver_text,
        "café.first::<u8>().second()"
    );
    assert_eq!(by_name("third").line_range, [2, 3]);
    for site in &set.sites {
        assert_eq!(
            site.method_name,
            source[site.method_byte_range[0]..site.method_byte_range[1]]
        );
        assert_eq!(
            site.call_text,
            source[site.byte_range[0]..site.byte_range[1]]
        );
    }
    set.validate(&graph, &sources(source))
}

#[test]
fn comments_strings_macros_and_field_function_calls_are_excluded() -> anyhow::Result<()> {
    let source = r####"
macro_rules! hidden { () => { a.unwrap() }; }
fn demo(a: A) {
    // a.unwrap();
    let _ = "a.unwrap()";
    let _ = r###"a.unwrap()"###;
    hidden!(a.unwrap());
    (a.callback)();
    a.visible();
}
"####;
    let graph = graph(&[("src/lib.rs", source)])?;
    let set = collect_method_call_sites(&graph, &sources(source))?;
    assert_eq!(
        set.sites.len(),
        1,
        "only direct field-expression method calls"
    );
    assert_eq!(set.sites[0].method_name, "visible");
    set.validate(&graph, &sources(source))
}

#[test]
fn closure_const_item_and_nested_function_ownership_stays_qualified() -> anyhow::Result<()> {
    let source = r#"
const C: u8 = a.const_call();
static S: u8 = a.static_call();
type Array = [u8; a.array_call()];
fn outer(a: A) {
    let _ = || a.closure_call();
    let _ = const { a.block_call() };
    const INNER: u8 = a.item_call();
    fn inner(a: A) { a.nested_call(); }
    a.direct_call();
}
"#;
    let graph = graph(&[("src/lib.rs", source)])?;
    let set = collect_method_call_sites(&graph, &sources(source))?;
    assert_eq!(set.sites.len(), 8);
    let by_name = |name| set.sites.iter().find(|s| s.method_name == name).unwrap();
    for name in ["const_call", "static_call", "block_call", "item_call"] {
        assert_eq!(
            by_name(name).lexical_phase,
            MethodCallLexicalPhase::ConstContextSyntax
        );
        assert_eq!(by_name(name).caller_stable_symbol_id, None);
    }
    assert_eq!(
        by_name("array_call").lexical_phase,
        MethodCallLexicalPhase::ItemSyntax
    );
    assert_eq!(by_name("array_call").caller_stable_symbol_id, None);
    assert_eq!(
        by_name("closure_call").lexical_phase,
        MethodCallLexicalPhase::ClosureBodySyntax
    );
    assert_eq!(by_name("closure_call").caller_stable_symbol_id, None);
    assert!(by_name("closure_call")
        .lexical_owners
        .iter()
        .any(|o| o.syntax_kind == "closure_expression"));
    for (method, function) in [("nested_call", "inner"), ("direct_call", "outer")] {
        let actual = graph
            .symbols
            .iter()
            .find(|s| s.entity_name == function)
            .unwrap();
        assert_eq!(
            by_name(method).caller_stable_symbol_id.as_deref(),
            Some(actual.stable_symbol_id.as_str())
        );
    }
    set.validate(&graph, &sources(source))
}

#[test]
fn absent_ambiguous_and_malformed_function_owners_remain_unknown() -> anyhow::Result<()> {
    let (mut graph, _) = fixture()?;
    graph.symbols.retain(|s| s.entity_name != "repeat");
    let set = collect_method_call_sites(&graph, &sources(SOURCE))?;
    assert_eq!(set.sites.len(), 4);
    assert!(set.sites.iter().all(|s| s.caller_stable_symbol_id.is_none()
        && s.lexical_phase == MethodCallLexicalPhase::Unknown));
    let (mut graph, _) = fixture()?;
    let mut duplicate = graph
        .symbols
        .iter()
        .find(|s| s.entity_name == "repeat")
        .unwrap()
        .clone();
    duplicate.stable_symbol_id = "ambiguous-real-graph-entry".into();
    graph.symbols.push(duplicate);
    let set = collect_method_call_sites(&graph, &sources(SOURCE))?;
    assert!(set.sites.iter().all(|s| s.caller_stable_symbol_id.is_none()
        && s.lexical_phase == MethodCallLexicalPhase::Unknown));
    let broken = "fn malformed(a: A) { a.visible(); let broken = ; }";
    let graph = graph_for_broken(broken)?;
    let set = collect_method_call_sites(&graph, &sources(broken))?;
    assert_eq!(set.sites.len(), 1);
    assert!(set.sources[0].has_parse_errors);
    assert_eq!(set.sites[0].lexical_phase, MethodCallLexicalPhase::Unknown);
    assert_eq!(set.sites[0].caller_stable_symbol_id, None);
    Ok(())
}

fn graph_for_broken(source: &str) -> anyhow::Result<GraphIndexArtifact> {
    graph(&[("src/lib.rs", source)])
}

#[test]
fn validation_rejects_missing_duplicate_and_reordered_sites() -> anyhow::Result<()> {
    let (graph, set) = fixture()?;
    for mode in ["missing", "duplicate", "reordered"] {
        let mut bad = set.clone();
        match mode {
            "missing" => {
                bad.sites.pop();
            }
            "duplicate" => bad.sites.push(bad.sites[0].clone()),
            _ => bad.sites.reverse(),
        }
        assert!(
            bad.validate(&graph, &sources(SOURCE)).is_err(),
            "accepted {mode} sites"
        );
    }
    Ok(())
}

#[test]
fn validation_rejects_changed_text_spans_id_phase_and_provenance() -> anyhow::Result<()> {
    let (graph, set) = fixture()?;
    assert_eq!(set.sites.len(), 4, "tamper controls require collected sites");
    for (field, value) in [
        ("site_id", serde_json::json!("forged")),
        ("source_oid", serde_json::json!(git_blob_oid(b"stale"))),
        ("file_path", serde_json::json!("src/other.rs")),
        ("call_text", serde_json::json!("b.unwrap()")),
        ("method_name", serde_json::json!("expect")),
        ("receiver_text", serde_json::json!("b")),
        ("byte_range", serde_json::json!([143, 153])),
        ("method_byte_range", serde_json::json!([0, 6])),
        ("receiver_byte_range", serde_json::json!([0, 1])),
        ("line_range", serde_json::json!([1, 1])),
        ("lexical_phase", serde_json::json!("unknown")),
        ("lexical_owners", serde_json::json!([])),
    ] {
        let mut json = serde_json::to_value(&set)?;
        json["sites"][0][field] = value;
        let bad: MethodCallSiteEvidenceSet = serde_json::from_value(json)?;
        assert!(
            bad.validate(&graph, &sources(SOURCE)).is_err(),
            "accepted altered {field}"
        );
    }
    Ok(())
}

#[test]
fn validation_rejects_another_existing_function_in_the_same_file() -> anyhow::Result<()> {
    let (graph, mut set) = fixture()?;
    assert_eq!(
        set.sites.len(),
        4,
        "ownership test requires collected sites"
    );
    let unrelated = graph
        .symbols
        .iter()
        .find(|s| s.entity_name == "unwrap")
        .unwrap();
    set.sites[0].caller_stable_symbol_id = Some(unrelated.stable_symbol_id.clone());
    assert!(
        set.validate(&graph, &sources(SOURCE)).is_err(),
        "membership alone cannot prove ownership"
    );
    Ok(())
}

#[test]
fn source_binding_requires_unique_physical_graph_file_and_exact_manifest_oid() -> anyhow::Result<()>
{
    let (graph, _) = fixture()?;
    let input = sources(SOURCE);
    assert!(
        collect_method_call_sites(&graph, &[input[0], input[0]]).is_err(),
        "duplicate source paths"
    );
    assert!(
        collect_method_call_sites(&graph, &sources("fn stale() {}")).is_err(),
        "stale source bytes"
    );
    assert!(collect_method_call_sites(
        &graph,
        &[PanicEvidenceSource {
            file_path: "src/missing.rs",
            bytes: SOURCE.as_bytes()
        }]
    )
    .is_err());
    for mode in [
        "missing_file",
        "duplicate_file",
        "duplicate_manifest",
        "missing_manifest",
        "oid",
        "file_id",
    ] {
        let mut bad = graph.clone();
        match mode {
            "missing_file" => bad.files.clear(),
            "duplicate_file" => bad.files.push(bad.files[0].clone()),
            "duplicate_manifest" => bad.file_manifests.push(bad.file_manifests[0].clone()),
            "missing_manifest" => bad.file_manifests.clear(),
            "oid" => bad.file_manifests[0].content_oid = git_blob_oid(b"stale"),
            _ => bad.file_manifests[0].stable_file_id = "different-file".into(),
        }
        assert!(
            collect_method_call_sites(&bad, &input).is_err(),
            "accepted {mode}"
        );
    }
    let graph = graph_for_broken("fn fine() {}")?;
    assert!(collect_method_call_sites(
        &graph,
        &[PanicEvidenceSource {
            file_path: "notebook.ipynb#cell.rs",
            bytes: b"fn fine() {}"
        }]
    )
    .is_err());
    Ok(())
}

#[test]
fn roundtrip_ordering_and_supplied_file_coverage_are_deterministic() -> anyhow::Result<()> {
    let graph = graph(&[
        ("src/lib.rs", SOURCE),
        ("src/other.rs", "fn other(x: X) { x.go(); }"),
        ("src/empty.rs", "// no sites\n"),
    ])?;
    let input = [
        PanicEvidenceSource {
            file_path: "src/other.rs",
            bytes: b"fn other(x: X) { x.go(); }",
        },
        sources(SOURCE)[0],
        PanicEvidenceSource {
            file_path: "src/empty.rs",
            bytes: b"// no sites\n",
        },
    ];
    let set = collect_method_call_sites(&graph, &input)?;
    assert_eq!(set.sites.len(), 5);
    assert_eq!(set.sources.len(), 3);
    let mut reversed = input;
    reversed.reverse();
    assert_eq!(set, collect_method_call_sites(&graph, &reversed)?);
    let decoded: MethodCallSiteEvidenceSet = serde_json::from_str(&serde_json::to_string(&set)?)?;
    assert_eq!(set, decoded);
    decoded.validate(&graph, &input)?;
    assert!(
        set.validate(&graph, &input[..2]).is_err(),
        "omitted zero-site source still changes coverage"
    );
    let partial = collect_method_call_sites(&graph, &sources(SOURCE))?;
    assert_eq!(
        partial.sites.len(),
        4,
        "scope is supplied files, not workspace"
    );
    partial.validate(&graph, &sources(SOURCE))?;
    let empty = collect_method_call_sites(&graph, &[])?;
    assert!(empty.sites.is_empty());
    empty.validate(&graph, &[])?;
    for (field, value) in [
        ("graph_content_hash", "mixed-snapshot"),
        ("schema_version", "future"),
        ("extractor_version", "future"),
    ] {
        let mut json = serde_json::to_value(&set)?;
        json[field] = value.into();
        let bad: MethodCallSiteEvidenceSet = serde_json::from_value(json)?;
        assert!(bad.validate(&graph, &input).is_err(), "accepted {field}");
    }
    let mut bad = set.clone();
    bad.sources[0].site_count += 1;
    assert!(bad.validate(&graph, &input).is_err());
    let mut bad = set.clone();
    bad.sources.push(bad.sources[0].clone());
    assert!(bad.validate(&graph, &input).is_err());
    Ok(())
}
