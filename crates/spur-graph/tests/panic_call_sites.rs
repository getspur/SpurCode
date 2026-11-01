use std::collections::BTreeSet;

use spur_graph::panic_evidence::{
    collect_method_call_sites, collect_rust_call_sites, PanicEvidenceSource, RustBodyKind,
    RustCallSiteEvidenceSet,
};
use spur_graph::{artifact_from_facts, build_facts, git_blob_oid, GraphIndexArtifact};

const HELD_OUT: &str = include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/fixtures/bodies/lib.rs");

const ASYNC_PHASES: &str = include_str!(
    "../../../docs/research/artifacts/2026-11-01-panic-implementation/R3/attempt-2/async.rs.txt"
);
const REPEAT_PHASES: &str = include_str!(
    "../../../docs/research/artifacts/2026-11-01-panic-implementation/R3/attempt-2/repeats.rs.txt"
);
const PHASE_ORACLE: &str = include_str!(
    "../../../docs/research/artifacts/2026-11-01-panic-implementation/R3/attempt-2/pre-oracle.json"
);

fn phase_body<'a>(
    set: &'a RustCallSiteEvidenceSet,
    source: &str,
    marker: &str,
) -> &'a spur_graph::panic_evidence::RustBodyEvidence {
    let marker = format!("/*site:{marker}*/");
    let offset = source.find(&marker).unwrap() + marker.len();
    let site = set
        .sites
        .iter()
        .find(|s| s.byte_range[0] >= offset)
        .unwrap();
    set.bodies
        .iter()
        .find(|b| b.body_id == site.body_id)
        .unwrap()
}

fn assert_phase_oracle(
    name: &str,
    source: &str,
    set: &RustCallSiteEvidenceSet,
) -> anyhow::Result<()> {
    // This oracle was derived from input markers before RED or collector edits.
    let oracle: serde_json::Value = serde_json::from_str(PHASE_ORACLE)?;
    let expected = &oracle[name];
    let bodies = expected["bodies"].as_object().unwrap();
    assert_eq!(set.bodies.len(), bodies.len());
    assert_eq!(set.sites.len(), expected["sites"].as_array().unwrap().len());
    assert_eq!(
        set.bodies
            .iter()
            .map(|b| &b.body_id)
            .collect::<BTreeSet<_>>()
            .len(),
        bodies.len()
    );
    assert_eq!(
        set.sites
            .iter()
            .map(|s| &s.site_id)
            .collect::<BTreeSet<_>>()
            .len(),
        set.sites.len()
    );
    for (label, expected_body) in bodies {
        let body = set
            .bodies
            .iter()
            .find(|b| serde_json::json!(b.byte_range) == expected_body["byte_range"])
            .unwrap_or_else(|| panic!("missing {name}/{label}"));
        assert_eq!(
            serde_json::to_value(body.kind)?,
            expected_body["kind"],
            "{name}/{label}"
        );
        assert_eq!(body.source_oid, git_blob_oid(source.as_bytes()));
        assert_eq!(body.file_path, "src/lib.rs");
    }
    for expected_site in expected["sites"].as_array().unwrap() {
        let site = set
            .sites
            .iter()
            .find(|s| serde_json::json!(s.byte_range) == expected_site["byte_range"])
            .expect("missing invocation");
        let body = set
            .bodies
            .iter()
            .find(|b| b.body_id == site.body_id)
            .unwrap();
        assert_eq!(
            serde_json::json!(body.byte_range),
            bodies[expected_site["body"].as_str().unwrap()]["byte_range"],
            "{name}/{}",
            expected_site["label"]
        );
        assert_eq!(site.call_text, expected_site["call_text"].as_str().unwrap());
        assert_eq!(
            site.call_text,
            source[site.byte_range[0]..site.byte_range[1]]
        );
        assert_eq!(site.source_oid, body.source_oid);
        assert_eq!(site.file_path, body.file_path);
    }
    assert_eq!(
        set.sources[0].source_oid,
        expected["source_oid"].as_str().unwrap()
    );
    assert_eq!(set.sources[0].body_count, set.bodies.len());
    assert_eq!(set.sources[0].site_count, set.sites.len());
    println!(
        "R3_REWORK_OBSERVED={}",
        serde_json::json!({"name": name, "set": set})
    );
    Ok(())
}

#[test]
fn async_closure_invocation_creates_a_future_without_entering_its_body() -> anyhow::Result<()> {
    let graph = graph(&[("src/lib.rs", ASYNC_PHASES)])?;
    let methods = collect_method_call_sites(&graph, &sources(ASYNC_PHASES))?;
    let set = collect_rust_call_sites(&graph, &sources(ASYNC_PHASES))?;
    for marker in ["deferred_one", "deferred_two", "moving", "expression"] {
        assert_eq!(
            serde_json::to_value(phase_body(&set, ASYNC_PHASES, marker).kind)?,
            "async_closure",
            "async closure invocation creates a future; polling enters its body"
        );
    }
    assert_eq!(
        phase_body(&set, ASYNC_PHASES, "plain").kind,
        RustBodyKind::Closure
    );
    assert_phase_oracle("async", ASYNC_PHASES, &set)?;
    let mut json = serde_json::to_value(&set)?;
    json["execution_edges"] = serde_json::json!([]);
    assert!(serde_json::from_value::<RustCallSiteEvidenceSet>(json).is_err());
    set.validate(&graph, &sources(ASYNC_PHASES))?;
    assert_eq!(
        methods,
        collect_method_call_sites(&graph, &sources(ASYNC_PHASES))?
    );
    Ok(())
}

#[test]
fn repeat_counts_have_constant_bodies_separate_from_runtime_elements() -> anyhow::Result<()> {
    let graph = graph(&[("src/lib.rs", REPEAT_PHASES)])?;
    let methods = collect_method_call_sites(&graph, &sources(REPEAT_PHASES))?;
    let set = collect_rust_call_sites(&graph, &sources(REPEAT_PHASES))?;
    assert_eq!(
        phase_body(&set, REPEAT_PHASES, "count").kind,
        RustBodyKind::ConstExpression,
        "array-repeat length is CTFE, separately from its runtime element"
    );
    assert_eq!(
        phase_body(&set, REPEAT_PHASES, "element").kind,
        RustBodyKind::Function
    );
    assert_eq!(
        phase_body(&set, REPEAT_PHASES, "runtime_const").kind,
        RustBodyKind::Function,
        "calling a const fn at runtime does not imply CTFE"
    );
    assert_phase_oracle("repeats", REPEAT_PHASES, &set)?;
    set.validate(&graph, &sources(REPEAT_PHASES))?;
    assert_eq!(
        methods,
        collect_method_call_sites(&graph, &sources(REPEAT_PHASES))?
    );
    Ok(())
}

#[test]
fn phase_body_metadata_must_match_canonical_source_evidence() -> anyhow::Result<()> {
    for (name, source, marker) in [
        ("async", ASYNC_PHASES, "deferred_one"),
        ("repeats", REPEAT_PHASES, "count"),
    ] {
        let graph = graph(&[("src/lib.rs", source)])?;
        let set = collect_rust_call_sites(&graph, &sources(source))?;
        let target = phase_body(&set, source, marker);
        let index = set
            .bodies
            .iter()
            .position(|b| b.body_id == target.body_id)
            .unwrap();
        let site = set
            .sites
            .iter()
            .position(|s| s.body_id == target.body_id)
            .unwrap();
        let parent = set
            .bodies
            .iter()
            .find(|b| {
                b.kind == RustBodyKind::Function
                    && b.byte_range[0] < target.byte_range[0]
                    && b.byte_range[1] > target.byte_range[1]
            })
            .unwrap();
        let mut forged = set.clone();
        forged.bodies[index].kind = RustBodyKind::Closure;
        assert!(
            forged.validate(&graph, &sources(source)).is_err(),
            "{name}: accepted forged ordinary-closure kind"
        );
        forged = set.clone();
        forged.sites[site].body_id = parent.body_id.clone();
        assert!(
            forged.validate(&graph, &sources(source)).is_err(),
            "{name}: accepted parent execution context"
        );
        for relation in ["bodies", "sites"] {
            for field in [
                "source_oid",
                "file_path",
                if relation == "bodies" {
                    "body_id"
                } else {
                    "site_id"
                },
            ] {
                let mut json = serde_json::to_value(&set)?;
                json[relation][0][field] = serde_json::json!("forged");
                let forged: RustCallSiteEvidenceSet = serde_json::from_value(json)?;
                assert!(forged.validate(&graph, &sources(source)).is_err());
                let mut json = serde_json::to_value(&set)?;
                json[relation][0].as_object_mut().unwrap().remove(field);
                assert!(serde_json::from_value::<RustCallSiteEvidenceSet>(json).is_err());
            }
            let mut json = serde_json::to_value(&set)?;
            let rows = json[relation].as_array_mut().unwrap();
            rows.push(rows[0].clone());
            let forged: RustCallSiteEvidenceSet = serde_json::from_value(json)?;
            assert!(forged.validate(&graph, &sources(source)).is_err());
            let mut json = serde_json::to_value(&set)?;
            json[relation].as_array_mut().unwrap().remove(0);
            let forged: RustCallSiteEvidenceSet = serde_json::from_value(json)?;
            assert!(forged.validate(&graph, &sources(source)).is_err());
        }
        let changed = source.to_owned() + "\n";
        assert!(set.validate(&graph, &sources(&changed)).is_err());
        println!(
            "R3_REWORK_VALIDATION={}",
            serde_json::json!({"name": name, "forged_kind_accepted": false, "parent_owner_accepted": false, "forged_source_accepted": false, "missing_identity_accepted": false, "duplicate_identity_accepted": false, "omission_accepted": false, "stale_bytes_accepted": false})
        );
    }
    Ok(())
}

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

fn fixture() -> anyhow::Result<(GraphIndexArtifact, RustCallSiteEvidenceSet)> {
    let graph = graph(&[("src/lib.rs", HELD_OUT)])?;
    let set = collect_rust_call_sites(&graph, &sources(HELD_OUT))?;
    Ok((graph, set))
}

fn marked_site<'a>(
    set: &'a RustCallSiteEvidenceSet,
    marker: &str,
) -> &'a spur_graph::panic_evidence::RustCallSiteEvidence {
    let offset = HELD_OUT.find(&format!("/*site:{marker}*/")).unwrap();
    set.sites.iter().find(|s| s.byte_range[0] > offset).unwrap()
}

#[test]
fn held_out_calls_have_individual_ids_and_exact_executable_bodies() -> anyhow::Result<()> {
    let (graph, set) = fixture()?;
    assert_eq!(
        set.sites.len(),
        14,
        "include free calls and static initializers"
    );
    assert_eq!(set.bodies.len(), 8);
    assert_eq!(
        set.sites
            .iter()
            .map(|s| &s.site_id)
            .collect::<BTreeSet<_>>()
            .len(),
        14
    );
    assert_eq!(
        set.bodies
            .iter()
            .map(|b| &b.body_id)
            .collect::<BTreeSet<_>>()
            .len(),
        8
    );
    assert_eq!(set.sources[0].site_count, 14);
    assert_eq!(set.sources[0].body_count, 8);
    let parent = &marked_site(&set, "repeat_one").body_id;
    for (first, second) in [("repeat_one", "repeat_two"), ("free_one", "free_two")] {
        let a = marked_site(&set, first);
        let b = marked_site(&set, second);
        assert_eq!(a.call_text, b.call_text);
        assert_ne!(a.site_id, b.site_id);
        assert_eq!(&a.body_id, parent);
        assert_eq!(a.body_id, b.body_id);
    }
    let mut ids = BTreeSet::from([parent.clone()]);
    for (prefix, kind) in [
        ("closure", RustBodyKind::Closure),
        ("async", RustBodyKind::AsyncBlock),
        ("nested", RustBodyKind::NestedFunction),
    ] {
        let a = marked_site(&set, &format!("{prefix}_method"));
        let b = marked_site(&set, &format!("{prefix}_free"));
        assert_eq!(a.body_id, b.body_id);
        assert!(
            ids.insert(a.body_id.clone()),
            "creation must not assign child calls to parent"
        );
        let body = set
            .bodies
            .iter()
            .find(|body| body.body_id == a.body_id)
            .unwrap();
        assert_eq!(body.kind, kind);
        assert_eq!(
            body.definition_stable_symbol_id.is_some(),
            prefix == "nested"
        );
    }
    for site in &set.sites {
        assert_eq!(
            site.call_text,
            HELD_OUT[site.byte_range[0]..site.byte_range[1]]
        );
        assert_eq!(site.source_oid, git_blob_oid(HELD_OUT.as_bytes()));
        let body = set
            .bodies
            .iter()
            .find(|b| b.body_id == site.body_id)
            .unwrap();
        assert_eq!(body.source_oid, site.source_oid);
        assert_eq!(body.file_path, site.file_path);
        assert!(
            body.byte_range[0] <= site.byte_range[0] && site.byte_range[1] <= body.byte_range[1]
        );
    }
    // The data model records syntax ownership only. It has no execution edges.
    let mut json = serde_json::to_value(&set)?;
    json["execution_edges"] =
        serde_json::json!([[parent, &marked_site(&set, "closure_method").body_id]]);
    assert!(serde_json::from_value::<RustCallSiteEvidenceSet>(json).is_err());
    set.validate(&graph, &sources(HELD_OUT))?;
    println!("R3_OBSERVED={}", serde_json::to_string(&set)?);
    println!(
        "R3_GRAPH_SYMBOLS={}",
        serde_json::to_string(&graph.symbols)?
    );
    Ok(())
}

#[test]
fn validation_recollects_all_metadata_and_rejects_same_file_owner_forgery() -> anyhow::Result<()> {
    let (graph, set) = fixture()?;
    let target = marked_site(&set, "closure_method");
    let i = set
        .sites
        .iter()
        .position(|s| s.site_id == target.site_id)
        .unwrap();
    for (field, value) in [
        (
            "body_id",
            serde_json::json!(marked_site(&set, "repeat_one").body_id),
        ),
        ("site_id", serde_json::json!("forged")),
        ("source_oid", serde_json::json!("stale")),
        ("file_path", serde_json::json!("src/other.rs")),
        ("byte_range", serde_json::json!([527, 535])),
        ("call_text", serde_json::json!("ping()")),
    ] {
        let mut json = serde_json::to_value(&set)?;
        json["sites"][i][field] = value;
        let bad: RustCallSiteEvidenceSet = serde_json::from_value(json)?;
        assert!(
            bad.validate(&graph, &sources(HELD_OUT)).is_err(),
            "accepted {field}"
        );
    }
    let function = graph
        .symbols
        .iter()
        .find(|s| s.entity_name == "ping")
        .unwrap();
    for (field, value) in [
        ("body_id", serde_json::json!("forged")),
        ("source_oid", serde_json::json!("stale")),
        ("file_path", serde_json::json!("src/other.rs")),
        ("byte_range", serde_json::json!([0, HELD_OUT.len()])),
        ("kind", serde_json::json!("function")),
        (
            "definition_stable_symbol_id",
            serde_json::json!(function.stable_symbol_id),
        ),
    ] {
        let mut json = serde_json::to_value(&set)?;
        let j = set
            .bodies
            .iter()
            .position(|b| b.body_id == target.body_id)
            .unwrap();
        json["bodies"][j][field] = value;
        let bad: RustCallSiteEvidenceSet = serde_json::from_value(json)?;
        assert!(
            bad.validate(&graph, &sources(HELD_OUT)).is_err(),
            "accepted body {field}"
        );
    }
    for collection in ["sites", "bodies", "sources"] {
        for mutation in ["omit", "duplicate", "reverse"] {
            let mut json = serde_json::to_value(&set)?;
            let rows = json[collection].as_array_mut().unwrap();
            match mutation {
                "omit" => {
                    rows.pop();
                }
                "duplicate" => rows.push(rows[0].clone()),
                _ if rows.len() > 1 => rows.reverse(),
                _ => {
                    rows[0]["site_count"] = serde_json::json!(999);
                }
            }
            let bad: RustCallSiteEvidenceSet = serde_json::from_value(json)?;
            assert!(
                bad.validate(&graph, &sources(HELD_OUT)).is_err(),
                "accepted {collection} {mutation}"
            );
        }
    }
    let mut json = serde_json::to_value(&set)?;
    json["schema_version"] = serde_json::json!("unknown-version");
    assert!(serde_json::from_value::<RustCallSiteEvidenceSet>(json)?
        .validate(&graph, &sources(HELD_OUT))
        .is_err());
    Ok(())
}

#[test]
fn stale_duplicate_and_unbound_source_inputs_are_rejected() -> anyhow::Result<()> {
    let (mut graph, set) = fixture()?;
    assert!(set
        .validate(&graph, &sources(&(HELD_OUT.to_owned() + "\n")))
        .is_err());
    let input = sources(HELD_OUT);
    assert!(collect_rust_call_sites(&graph, &[input[0], input[0]]).is_err());
    graph.file_manifests[0].content_oid = git_blob_oid(b"other");
    assert!(set.validate(&graph, &input).is_err());
    Ok(())
}

#[test]
fn parse_and_cfg_uncertainty_cannot_validate_completeness() -> anyhow::Result<()> {
    for source in [
        "fn broken() { ping(); let x = ; }",
        "#[cfg(feature = \"maybe\")] fn conditional() { ping(); }",
        "#[r#cfg(feature = \"maybe\")] fn conditional() { ping(); }",
        "#[r#cfg_attr(feature = \"maybe\", inline)] fn conditional() { ping(); }",
        "#[cfg_attr(feature = \"maybe\", inline)] fn conditional() { ping(); }",
        "#![cfg(any())]\nfn conditional() { ping(); }",
        "fn outer() { #[cfg(any())] ping(); }",
    ] {
        let graph = graph(&[("src/lib.rs", source)])?;
        let result = collect_rust_call_sites(&graph, &sources(source));
        println!(
            "R3_UNCERTAINTY={}",
            serde_json::json!({
                "source": source,
                "source_oid": git_blob_oid(source.as_bytes()),
                "accepted": result.is_ok(),
                "error": result.as_ref().err().map(ToString::to_string),
            })
        );
        assert!(result.is_err(), "accepted uncertain source {source}");
    }
    Ok(())
}

#[test]
fn direct_syntax_supports_chains_generics_utf8_and_const_contexts() -> anyhow::Result<()> {
    let source = r####"
macro_rules! hidden { () => { ignored() }; }
const K: usize = size();
type Array = [u8; size()];
fn demo<T>(a: T) {
    // ignored();
    let _ = "ignored() #[cfg(any())]";
    hidden!(ignored());
    (a.callback)();
    (|| closure_only())();
    let café = a;
    café.first::<u8>().second();
    module::free::<T>();
    let _ = const { size() };
    let _: [u8; size()];
}
async fn deferred() { ping(); }
"####;
    let graph = graph(&[("src/lib.rs", source)])?;
    let methods_before = collect_method_call_sites(&graph, &sources(source))?;
    let set = collect_rust_call_sites(&graph, &sources(source))?;
    assert_eq!(set.sites.len(), 9);
    assert!(!set
        .sites
        .iter()
        .any(|s| s.call_text.contains("ignored") || s.call_text.contains("callback")));
    assert!(set
        .sites
        .iter()
        .any(|s| s.call_text == "module::free::<T>()"));
    assert!(set
        .sites
        .iter()
        .any(|s| s.call_text == "café.first::<u8>()"));
    let kind_for = |text| {
        let site = set.sites.iter().find(|s| s.call_text == text).unwrap();
        set.bodies
            .iter()
            .find(|b| b.body_id == site.body_id)
            .unwrap()
            .kind
    };
    assert_eq!(kind_for("closure_only()"), RustBodyKind::Closure);
    assert_eq!(kind_for("ping()"), RustBodyKind::AsyncFunction);
    let size_kinds: BTreeSet<_> = set
        .sites
        .iter()
        .filter(|s| s.call_text == "size()")
        .map(|s| {
            format!(
                "{:?}",
                set.bodies
                    .iter()
                    .find(|b| b.body_id == s.body_id)
                    .unwrap()
                    .kind
            )
        })
        .collect();
    assert_eq!(
        size_kinds,
        BTreeSet::from([
            "ConstInitializer".to_owned(),
            "ConstBlock".to_owned(),
            "ConstExpression".to_owned()
        ])
    );
    set.validate(&graph, &sources(source))?;
    assert_eq!(
        methods_before,
        collect_method_call_sites(&graph, &sources(source))?
    );
    Ok(())
}

#[test]
fn identities_are_snapshot_bound_and_inputs_are_canonically_ordered() -> anyhow::Result<()> {
    let a = "fn one() { ping(); }";
    let b = "fn two() { ping(); }";
    let graph = graph(&[("src/lib.rs", a), ("src/b.rs", b)])?;
    let input = [
        sources(a)[0],
        PanicEvidenceSource {
            file_path: "src/b.rs",
            bytes: b.as_bytes(),
        },
    ];
    let set = collect_rust_call_sites(&graph, &input)?;
    assert_eq!(set, collect_rust_call_sites(&graph, &[input[1], input[0]])?);
    assert_ne!(set.sites[0].site_id, set.sites[1].site_id);
    let changed = a.to_owned() + "\n";
    let changed_graph = self::graph(&[("src/lib.rs", &changed)])?;
    let changed_set = collect_rust_call_sites(&changed_graph, &sources(&changed))?;
    let original = set
        .sites
        .iter()
        .find(|s| s.file_path == "src/lib.rs")
        .unwrap();
    assert_eq!(original.byte_range, changed_set.sites[0].byte_range);
    assert_ne!(original.site_id, changed_set.sites[0].site_id);
    assert_ne!(original.body_id, changed_set.sites[0].body_id);
    let empty = collect_rust_call_sites(&graph, &[])?;
    assert!(empty.sites.is_empty() && empty.bodies.is_empty() && empty.sources.is_empty());
    empty.validate(&graph, &[])?;
    Ok(())
}

#[test]
fn definition_identity_requires_exact_unambiguous_graph_metadata() -> anyhow::Result<()> {
    let (mut graph, _) = fixture()?;
    graph.symbols.retain(|s| s.entity_name != "bodies");
    let set = collect_rust_call_sites(&graph, &sources(HELD_OUT))?;
    let body = set
        .bodies
        .iter()
        .find(|b| b.body_id == marked_site(&set, "repeat_one").body_id)
        .unwrap();
    assert!(body.definition_stable_symbol_id.is_none());
    let (mut graph, _) = fixture()?;
    let mut duplicate = graph
        .symbols
        .iter()
        .find(|s| s.entity_name == "bodies")
        .unwrap()
        .clone();
    duplicate.stable_symbol_id = "ambiguous".into();
    graph.symbols.push(duplicate);
    let set = collect_rust_call_sites(&graph, &sources(HELD_OUT))?;
    let body = set
        .bodies
        .iter()
        .find(|b| b.body_id == marked_site(&set, "repeat_one").body_id)
        .unwrap();
    assert!(body.definition_stable_symbol_id.is_none());
    Ok(())
}
