use serde_json::{json, Value};
use spur_graph::{collect_rust_declarations, RustDeclarationEvidenceSet, RustDeclarationSource};

const SIMPLE: &str = r#"mod ffi {
    #[link(name = "demo")]
    unsafe extern "C" { pub fn imported(x: u8) -> u8; }
    pub extern "C" fn bodyful(x: u8) -> u8 { x }
}"#;
const RICH: &str = r##"// é ordinary
#[allow(dead_code)] fn previous() {}
mod first {
    /// block docs
    /* ordinary */
    #[link(name = "demo")]
    unsafe extern "C" {
        /// function docs
        #[link_name = r#"native"#]
        #[cfg_attr(any(), link_ordinal(3))]
        pub(crate) unsafe fn imported(x: u8) -> u8;
    }
    unsafe extern "C" { fn second(); }
    extern { fn omitted(); }
    pub extern "C" fn bodyful() {}
    trait T { fn required(); extern "C" fn provided() {} }
    fn host() { unsafe extern "C-unwind" { fn nested(); } }
}
mod second { unsafe extern "C" { fn other(); } }
"##;

fn fixture(source: &str) -> (tempfile::TempDir, spur_graph::GraphIndexArtifact) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), source).unwrap();
    std::fs::write(dir.path().join("zero.rs"), "pub fn zero() {}").unwrap();
    let facts = spur_graph::build_facts(dir.path(), None).unwrap().0;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path()).unwrap();
    (dir, graph)
}

fn sources(source: &str) -> [RustDeclarationSource<'_>; 2] {
    [
        RustDeclarationSource {
            file_path: "lib.rs",
            bytes: source.as_bytes(),
        },
        RustDeclarationSource {
            file_path: "zero.rs",
            bytes: b"pub fn zero() {}",
        },
    ]
}

#[test]
fn current_public_collector_serializes_v2_foreign_contract() {
    let (_dir, graph) = fixture(SIMPLE);
    let set = collect_rust_declarations(&graph, &sources(SIMPLE)).unwrap();
    let json = serde_json::to_value(&set).unwrap();
    assert_eq!(json["schema_version"], "spur-rust-declarations-v2");
    assert_eq!(json["foreign_functions"].as_array().unwrap().len(), 1);
    assert_eq!(json["extern_blocks"].as_array().unwrap().len(), 1);
    set.validate(&graph, &sources(SIMPLE)).unwrap();
    assert_eq!(
        json["covered_kinds"],
        json!(["tuple_field", "foreign_function", "extern_block"])
    );
    assert!(json["rows"].as_array().unwrap().is_empty());
    assert_eq!(
        json["sources"][0]["availability"],
        json!({"complete":{"row_count":2}})
    );
    assert_eq!(
        json["sources"][1]["availability"],
        json!({"complete":{"row_count":0}})
    );
}

fn span<'a>(source: &'a str, range: &Value) -> &'a str {
    &source[range[0].as_u64().unwrap() as usize..range[1].as_u64().unwrap() as usize]
}

#[test]
fn raw_syntax_owners_abi_and_body_categories_are_distinct() {
    let (_dir, graph) = fixture(RICH);
    let set = collect_rust_declarations(&graph, &sources(RICH)).unwrap();
    let v = serde_json::to_value(&set).unwrap();
    assert_eq!(v["foreign_functions"].as_array().unwrap().len(), 5);
    assert_eq!(v["extern_blocks"].as_array().unwrap().len(), 5);
    let f = &v["foreign_functions"][0];
    let b = &v["extern_blocks"][0];
    assert_eq!(f["entity_name"], "imported");
    assert_eq!(f["qualified_name"], "first::imported");
    assert_eq!(
        f["structural_owner"],
        json!({"symbol":{"stable_symbol_id":b["stable_symbol_id"]}})
    );
    assert_eq!(f["lexical_owner"], b["lexical_owner"]);
    assert_eq!(
        f["abi"],
        json!({"inherited":{"extern_block_id":b["stable_symbol_id"]}})
    );
    assert_eq!(b["abi"]["explicit"]["literal"]["text"], "\"C\"");
    assert_eq!(f["body"], "absent_foreign_declaration");
    assert_eq!(b["body"], "non_executable_container");
    assert_eq!(f["visibility"]["text"], "pub(crate)");
    assert_eq!(f["qualifiers"][0]["text"], "unsafe");
    assert_eq!(b["qualifiers"][0]["text"], "unsafe");
    assert_eq!(b["qualifiers"][1]["text"], "extern");
    assert_eq!(
        f["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "/// function docs\n",
            "#[link_name = r#\"native\"#]",
            "#[cfg_attr(any(), link_ordinal(3))]"
        ]
    );
    assert_eq!(
        span(RICH, &f["core_range"]),
        "pub(crate) unsafe fn imported(x: u8) -> u8;"
    );
    assert!(span(RICH, &f["full_range"]).starts_with("/// function docs"));
    assert_eq!(v["extern_blocks"][2]["abi"], "omitted");
    assert_eq!(
        v["foreign_functions"][3]["qualified_name"],
        "first::host::nested"
    );
    for collection in ["foreign_functions", "extern_blocks"] {
        for row in v[collection].as_array().unwrap() {
            let symbol = graph
                .symbols
                .iter()
                .find(|s| s.stable_symbol_id == row["stable_symbol_id"].as_str().unwrap())
                .unwrap();
            assert_eq!(json!(symbol.byte_range), row["full_range"]);
            assert_eq!(json!(symbol.line_range), row["line_range"]);
            assert_eq!(
                symbol.qualified_name,
                row["qualified_name"].as_str().unwrap()
            );
            for t in row["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .chain(row["qualifiers"].as_array().unwrap())
            {
                assert_eq!(span(RICH, &t["byte_range"]), t["text"].as_str().unwrap());
            }
        }
    }
    set.validate(&graph, &sources(RICH)).unwrap();
}

#[test]
fn canonical_recollection_rejects_independent_substitutions() {
    let (_dir, graph) = fixture(RICH);
    let set = collect_rust_declarations(&graph, &sources(RICH)).unwrap();
    let original = serde_json::to_value(&set).unwrap();
    let mut controls = Vec::new();
    for (name, pointer, replacement) in [
        (
            "same_abi_existing_block",
            "/foreign_functions/0/structural_owner",
            original["foreign_functions"][1]["structural_owner"].clone(),
        ),
        (
            "other_existing_lexical_owner",
            "/foreign_functions/0/lexical_owner",
            original["foreign_functions"][4]["lexical_owner"].clone(),
        ),
        (
            "copied_id",
            "/foreign_functions/0/stable_symbol_id",
            original["foreign_functions"][1]["stable_symbol_id"].clone(),
        ),
        (
            "copied_name",
            "/foreign_functions/0/qualified_name",
            original["foreign_functions"][1]["qualified_name"].clone(),
        ),
        (
            "copied_full_range",
            "/foreign_functions/0/full_range",
            original["foreign_functions"][1]["full_range"].clone(),
        ),
        (
            "copied_core_range",
            "/foreign_functions/0/core_range",
            original["foreign_functions"][1]["core_range"].clone(),
        ),
        (
            "qualifier",
            "/foreign_functions/0/qualifiers/0/text",
            json!("safe"),
        ),
        (
            "body_tag",
            "/foreign_functions/0/body",
            json!("non_executable_container"),
        ),
        (
            "abi_literal",
            "/extern_blocks/0/abi/explicit/literal/text",
            json!("\"system\""),
        ),
        (
            "abi_reference",
            "/foreign_functions/0/abi/inherited/extern_block_id",
            original["extern_blocks"][1]["stable_symbol_id"].clone(),
        ),
        (
            "raw_link",
            "/extern_blocks/0/attributes/1/text",
            json!("#[link(name = \"forged\")]"),
        ),
        (
            "stale_header",
            "/schema_version",
            json!("spur-rust-declarations-v1"),
        ),
        ("stale_extractor", "/extractor_version", json!("stale")),
        ("stale_graph", "/graph_content_hash", json!("stale")),
        ("stale_source", "/sources/0/source_oid", json!("stale")),
    ] {
        let mut v = original.clone();
        let value = v.pointer_mut(pointer).unwrap();
        assert_ne!(*value, replacement, "mutation must change {name}");
        *value = replacement;
        controls.push((name, v));
    }
    for collection in ["foreign_functions", "extern_blocks", "sources"] {
        let mut omitted = original.clone();
        omitted[collection].as_array_mut().unwrap().pop();
        controls.push((collection, omitted));
        let mut duplicate = original.clone();
        duplicate[collection]
            .as_array_mut()
            .unwrap()
            .push(original[collection][0].clone());
        controls.push((collection, duplicate));
    }
    let mut reports = Vec::new();
    for (name, value) in controls {
        let forged: RustDeclarationEvidenceSet = serde_json::from_value(value.clone()).unwrap();
        let error = forged
            .validate(&graph, &sources(RICH))
            .unwrap_err()
            .to_string();
        reports.push(json!({"name":name,"set":value,"error":error,"rejected":true}));
    }
    if let Some(path) = std::env::var_os("SPUR_G2C_EXPORT_SMALL") {
        std::fs::write(path, serde_json::to_vec_pretty(&json!({"source":RICH,"zero_source":"pub fn zero() {}","graph":graph,"valid":original,"controls":reports})).unwrap()).unwrap();
    }
}

#[test]
fn strict_decoding_rejects_unknown_fields_at_every_evidence_layer() {
    let (_dir, graph) = fixture(SIMPLE);
    let set = collect_rust_declarations(&graph, &sources(SIMPLE)).unwrap();
    let original = serde_json::to_value(&set).unwrap();
    for pointer in [
        "",
        "/sources/0",
        "/sources/0/availability/complete",
        "/foreign_functions/0",
        "/foreign_functions/0/abi/inherited",
        "/foreign_functions/0/structural_owner/symbol",
        "/extern_blocks/0",
        "/extern_blocks/0/abi/explicit",
        "/extern_blocks/0/abi/explicit/literal",
    ] {
        let mut v = original.clone();
        v.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unrecognized".into(), json!(true));
        assert!(
            serde_json::from_value::<RustDeclarationEvidenceSet>(v).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn recovered_source_keeps_independent_healthy_ffi_rows_unknown() {
    let text = "unsafe extern \"C\" { fn healthy(); } fn broken( {";
    let (_dir, graph) = fixture(text);
    let set = collect_rust_declarations(&graph, &sources(text)).unwrap();
    let v = serde_json::to_value(&set).unwrap();
    assert!(v["sources"][0]["availability"].get("unknown").is_some());
    assert_eq!(v["foreign_functions"].as_array().unwrap().len(), 1);
    assert_eq!(v["extern_blocks"].as_array().unwrap().len(), 1);
    set.validate(&graph, &sources(text)).unwrap();
}

#[test]
#[ignore = "opt-in export of isolated frozen source inputs; requires SPUR_G2C_FROZEN_ROOT and SPUR_G2C_FROZEN_EXPORT"]
fn export_frozen_foreign_evidence() {
    let root = std::path::PathBuf::from(std::env::var_os("SPUR_G2C_FROZEN_ROOT").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("SPUR_G2C_FROZEN_EXPORT").unwrap());
    let graph =
        spur_graph::artifact_from_facts(&spur_graph::build_facts(&root, None).unwrap().0, &root)
            .unwrap();
    let inputs: Vec<_> = graph
        .file_manifests
        .iter()
        .filter(|m| m.path.ends_with(".rs"))
        .map(|m| (m.path.clone(), std::fs::read(root.join(&m.path)).unwrap()))
        .collect();
    let sources: Vec<_> = inputs
        .iter()
        .map(|(file_path, bytes)| RustDeclarationSource { file_path, bytes })
        .collect();
    // Independent batches must also recollect to precisely the whole-source rows.
    let batches: Vec<_> = sources
        .chunks(17)
        .map(|batch| {
            let set = collect_rust_declarations(&graph, batch).unwrap();
            set.validate(&graph, batch).unwrap();
            set
        })
        .collect();
    let set = collect_rust_declarations(&graph, &sources).unwrap();
    set.validate(&graph, &sources).unwrap();
    std::fs::write(
        output,
        serde_json::to_vec(&json!({"graph":graph,"set":set,"batches":batches})).unwrap(),
    )
    .unwrap();
}
