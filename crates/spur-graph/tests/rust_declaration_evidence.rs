use spur_graph::{collect_rust_declarations, RustDeclarationEvidenceSet, RustDeclarationSource};

const SOURCE: &str = "/* é */ struct S(#[cfg(any())] pub u8, /** docs */ pub(crate) Vec<(u8,u8)>, u16); enum E { A(u8), B(u8) }";
const EMPTY: &str = "pub fn marker() {}";

fn fixture() -> (tempfile::TempDir, spur_graph::GraphIndexArtifact) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), SOURCE).unwrap();
    std::fs::write(dir.path().join("empty.rs"), EMPTY).unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    (dir, graph)
}

fn sources() -> [RustDeclarationSource<'static>; 2] {
    [
        RustDeclarationSource {
            file_path: "lib.rs",
            bytes: SOURCE.as_bytes(),
        },
        RustDeclarationSource {
            file_path: "empty.rs",
            bytes: EMPTY.as_bytes(),
        },
    ]
}

#[test]
fn provenance_covers_each_source_and_exact_tuple_syntax() {
    let (_dir, graph) = fixture();
    let set = collect_rust_declarations(&graph, &sources()).unwrap();
    set.validate(&graph, &sources()).unwrap();
    let v = serde_json::to_value(&set).unwrap();
    assert_eq!(v["scope"], "supplied_physical_rust_files");
    assert_eq!(v["covered_kinds"], serde_json::json!(["tuple_field"]));
    assert_eq!(v["sources"].as_array().unwrap().len(), 2);
    assert_eq!(v["rows"].as_array().unwrap().len(), 5);
    let zero = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["file_path"] == "empty.rs")
        .unwrap();
    assert_eq!(
        zero["availability"],
        serde_json::json!({"complete":{"row_count":0}})
    );
    let rows = v["rows"].as_array().unwrap();
    let first = rows
        .iter()
        .find(|r| r["ordinal"] == 0 && r["full_range"][0] == 18)
        .unwrap();
    let span = |range: &serde_json::Value| {
        &SOURCE[range[0].as_u64().unwrap() as usize..range[1].as_u64().unwrap() as usize]
    };
    assert_eq!(span(&first["full_range"]), "#[cfg(any())] pub u8");
    assert_eq!(span(&first["type_range"]), "u8");
    assert_eq!(first["visibility"]["text"], "pub");
    assert_eq!(first["attributes"][0]["text"], "#[cfg(any())]");
    assert!(graph.symbols.iter().any(|s| s.stable_symbol_id
        == first["stable_symbol_id"].as_str().unwrap()
        && s.symbol_kind == "field"));
}

#[test]
fn canonical_validation_rejects_forged_and_missing_evidence() {
    let (_dir, graph) = fixture();
    let set = collect_rust_declarations(&graph, &sources()).unwrap();
    let original = serde_json::to_value(&set).unwrap();
    let reject = |v: serde_json::Value| {
        let forged: RustDeclarationEvidenceSet = serde_json::from_value(v).unwrap();
        assert!(forged.validate(&graph, &sources()).is_err());
    };
    let mut v = original.clone();
    v["rows"].as_array_mut().unwrap().pop();
    reject(v);
    let mut v = original.clone();
    let row = v["rows"][0].clone();
    v["rows"].as_array_mut().unwrap().push(row);
    reject(v);
    let mut v = original.clone();
    v["sources"]
        .as_array_mut()
        .unwrap()
        .retain(|s| s["file_path"] != "empty.rs");
    reject(v);
    for key in [
        "ordinal",
        "full_range",
        "type_range",
        "line_range",
        "visibility",
        "attributes",
    ] {
        let mut v = original.clone();
        match key {
            "ordinal" => v["rows"][0][key] = serde_json::json!(99),
            "full_range" | "type_range" | "line_range" => {
                v["rows"][0][key][0] = serde_json::json!(0)
            }
            "visibility" => v["rows"][0][key] = serde_json::Value::Null,
            _ => v["rows"][0][key] = serde_json::json!([]),
        }
        reject(v);
    }
    let mut v = original.clone();
    let other = v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["structural_owner"] != v["rows"][0]["structural_owner"])
        .unwrap()["structural_owner"]
        .clone();
    v["rows"][0]["structural_owner"] = other.clone();
    reject(v);
    let mut v = original.clone();
    v["rows"][0]["lexical_owner"] = other;
    reject(v);
    let mut v = original.clone();
    v["rows"][0]["attributes"][0]["text"] = serde_json::json!("#[cfg(all())]");
    reject(v);
    let mut v = original.clone();
    v["rows"][0]["attributes"][0]["byte_range"][0] = serde_json::json!(0);
    reject(v);
    let mut v = original.clone();
    v["rows"][0]["visibility"]["text"] = serde_json::json!("pub(crate)");
    reject(v);
    let mut v = original.clone();
    v["graph_content_hash"] = serde_json::json!("stale");
    reject(v);
    let mut v = original.clone();
    v["sources"][0]["source_oid"] = serde_json::json!("stale");
    reject(v);
    let stale = [
        RustDeclarationSource {
            file_path: "lib.rs",
            bytes: b"struct S(u8);",
        },
        sources()[1],
    ];
    assert!(set.validate(&graph, &stale).is_err());
    let duplicates = [sources()[0], sources()[0]];
    assert!(collect_rust_declarations(&graph, &duplicates).is_err());
}

#[test]
fn notebook_virtual_paths_cannot_impersonate_physical_rust_sources() {
    let (_dir, mut graph) = fixture();
    let virtual_path = "notebook.ipynb#cell:one.rs";
    let file = graph
        .files
        .iter_mut()
        .find(|f| f.file_path == "lib.rs")
        .unwrap();
    file.file_path = virtual_path.into();
    let manifest = graph
        .file_manifests
        .iter_mut()
        .find(|f| f.path == "lib.rs")
        .unwrap();
    manifest.path = virtual_path.into();
    graph.graph_content_hash = spur_graph::compute_graph_content_hash(
        graph
            .file_manifests
            .iter()
            .map(|m| (&m.path, &m.content_oid)),
    );
    let sources = [RustDeclarationSource {
        file_path: virtual_path,
        bytes: SOURCE.as_bytes(),
    }];
    assert!(collect_rust_declarations(&graph, &sources).is_err());
}

#[test]
fn export_actual_adversarial_results() {
    let (_dir, graph) = fixture();
    let set = collect_rust_declarations(&graph, &sources()).unwrap();
    let value = serde_json::to_value(&set).unwrap();
    let mut reports = Vec::new();
    let mut check = |name: &str, forged: serde_json::Value| {
        let evidence: RustDeclarationEvidenceSet = serde_json::from_value(forged.clone()).unwrap();
        let rejected = evidence.validate(&graph, &sources()).is_err();
        assert!(rejected, "{name}");
        reports.push(serde_json::json!({"name": name, "set": forged, "rejected": rejected}));
    };
    let mut wrong_owner = value.clone();
    let other = value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["structural_owner"] != value["rows"][0]["structural_owner"])
        .unwrap()["structural_owner"]
        .clone();
    wrong_owner["rows"][0]["structural_owner"] = other;
    check("wrong_existing_owner", wrong_owner);
    let mut changed_type = value.clone();
    changed_type["rows"][0]["type_range"][0] = serde_json::json!(0);
    check("changed_type_range", changed_type);
    let mut changed_full = value.clone();
    changed_full["rows"][0]["full_range"][0] = serde_json::json!(0);
    check("changed_full_range", changed_full);
    let mut changed_ordinal = value.clone();
    changed_ordinal["rows"][0]["ordinal"] = serde_json::json!(99);
    check("changed_ordinal", changed_ordinal);
    let mut changed_attribute = value.clone();
    changed_attribute["rows"][0]["attributes"][0]["text"] = serde_json::json!("#[cfg(all())]");
    check("changed_attribute", changed_attribute);
    let mut changed_visibility = value.clone();
    changed_visibility["rows"][0]["visibility"]["text"] = serde_json::json!("private");
    check("changed_visibility", changed_visibility);
    let mut omitted = value.clone();
    omitted["rows"].as_array_mut().unwrap().pop();
    check("omitted_row", omitted);
    let mut duplicated = value.clone();
    duplicated["rows"]
        .as_array_mut()
        .unwrap()
        .push(value["rows"][0].clone());
    check("duplicate_row", duplicated);
    let mut missing_zero = value.clone();
    missing_zero["sources"]
        .as_array_mut()
        .unwrap()
        .retain(|s| s["file_path"] != "empty.rs");
    check("missing_zero_source", missing_zero);
    if let Some(path) = std::env::var_os("SPUR_G1C_EXPORT_SMALL") {
        std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"sources": {"lib.rs": SOURCE, "empty.rs": EMPTY}, "graph": graph, "valid": value, "controls": reports})).unwrap()).unwrap();
    }
}

#[test]
#[ignore]
fn export_frozen_declarations() {
    let root = std::path::PathBuf::from(std::env::var_os("SPUR_G1C_ROOT").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("SPUR_G1C_EXPORT").unwrap());
    let mut sets = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let fixture = entry.unwrap().path();
        let graph = spur_graph::artifact_from_facts(
            &spur_graph::build_facts(&fixture, None).unwrap().0,
            &fixture,
        )
        .unwrap();
        let inputs: Vec<_> = graph
            .file_manifests
            .iter()
            .map(|m| {
                (
                    m.path.clone(),
                    std::fs::read(fixture.join(&m.path)).unwrap(),
                )
            })
            .collect();
        let sources: Vec<_> = inputs
            .iter()
            .map(|(file_path, bytes)| RustDeclarationSource { file_path, bytes })
            .collect();
        let set = collect_rust_declarations(&graph, &sources).unwrap();
        set.validate(&graph, &sources).unwrap();
        sets.push(set);
    }
    std::fs::write(output, serde_json::to_vec(&sets).unwrap()).unwrap();
}

#[test]
fn parse_errors_never_certify_complete() {
    let dir = tempfile::tempdir().unwrap();
    let broken = "struct Broken(u8, ;";
    std::fs::write(dir.path().join("broken.rs"), broken).unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let source = [RustDeclarationSource {
        file_path: "broken.rs",
        bytes: broken.as_bytes(),
    }];
    let set = collect_rust_declarations(&graph, &source).unwrap();
    set.validate(&graph, &source).unwrap();
    let v = serde_json::to_value(set).unwrap();
    assert!(v["sources"][0]["availability"].get("unknown").is_some());
}

#[test]
fn parse_recovery_retains_independently_healthy_tuple_rows_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let source = "struct Healthy(u8); fn broken( {";
    std::fs::write(dir.path().join("broken.rs"), source).unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    let inputs = [RustDeclarationSource {
        file_path: "broken.rs",
        bytes: source.as_bytes(),
    }];
    let set = collect_rust_declarations(&graph, &inputs).unwrap();
    set.validate(&graph, &inputs).unwrap();
    let v = serde_json::to_value(set).unwrap();
    assert!(v["sources"][0]["availability"].get("unknown").is_some());
    assert_eq!(v["rows"].as_array().unwrap().len(), 1);
    assert_eq!(v["rows"][0]["ordinal"], 0);
}
