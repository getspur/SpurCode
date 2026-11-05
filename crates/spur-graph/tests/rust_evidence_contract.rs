use std::fs;

use serde_json::{json, Value};
use spur_graph::{
    artifact_from_facts, build_facts, collect_rust_evidence, compute_graph_content_hash,
    rust_evidence_checksum_blake3_hex, GraphIndexArtifact, RustCallEvidence,
    RustDeclarationAvailability, RustDeclarationBody, RustDeclarationSource, RustEvidenceSet,
    RustEvidenceUnknownReason,
};

fn graph(inputs: &[(String, Vec<u8>)]) -> GraphIndexArtifact {
    let root = tempfile::tempdir().unwrap();
    for (path, bytes) in inputs {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap()
}

fn sources(inputs: &[(String, Vec<u8>)]) -> Vec<RustDeclarationSource<'_>> {
    inputs
        .iter()
        .map(|(path, bytes)| RustDeclarationSource {
            file_path: path,
            bytes,
        })
        .collect()
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let pre: Value = serde_json::from_str(include_str!("../../../docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/root-pre/inputs.json")).unwrap();
    pre["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let hex = f["hex"].as_str().unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            (f["path"].as_str().unwrap().to_owned(), bytes)
        })
        .collect()
}

fn export(name: &str, value: &Value) {
    if let Ok(dir) = std::env::var("D1_EXPORT_DIR") {
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            std::path::Path::new(&dir).join(name),
            serde_json::to_vec_pretty(value).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn mixed_sources_retain_zero_and_unknown() {
    let inputs = vec![
        ("empty.rs".into(), b"pub struct Empty;\n".to_vec()),
        (
            "cfg.rs".into(),
            b"#[cfg(feature = \"x\")] fn hidden() { leaf(); }\n".to_vec(),
        ),
    ];
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    assert_eq!(set.files.len(), 2);
    let empty = set
        .files
        .iter()
        .find(|f| f.file_path == "empty.rs")
        .unwrap();
    assert!(empty.declarations.rows.is_empty());
    match &empty.calls {
        RustCallEvidence::Complete { evidence } => {
            assert_eq!(evidence.sources.len(), 1);
            assert!(evidence.sites.is_empty() && evidence.bodies.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        &set.files
            .iter()
            .find(|f| f.file_path == "cfg.rs")
            .unwrap()
            .calls,
        RustCallEvidence::Unknown {
            reason: RustEvidenceUnknownReason::UnevaluatedCfg
        }
    ));
    set.validate_sources(&graph, &sources(&inputs)).unwrap();
    let mut omitted = set.clone();
    omitted.files.retain(|f| f.file_path != "empty.rs");
    assert!(omitted.validate_stored(&graph).is_err());
}

#[test]
fn exact_six_sources_export_real_ids_and_independent_lanes() {
    let inputs = fixtures();
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    set.validate_stored(&graph).unwrap();
    set.validate_sources(&graph, &sources(&inputs)).unwrap();
    assert_eq!(set.files.len(), 6);
    assert_eq!(set.origins.len(), 1);
    for (path, reason) in [
        ("cfg.rs", RustEvidenceUnknownReason::UnevaluatedCfg),
        ("recovery.rs", RustEvidenceUnknownReason::ParseRecovery),
        ("invalid.rs", RustEvidenceUnknownReason::InvalidUtf8),
    ] {
        let f = set.files.iter().find(|f| f.file_path == path).unwrap();
        assert_eq!(f.calls, RustCallEvidence::Unknown { reason });
        assert_eq!(f.declarations.sources.len(), 1);
    }
    let recovered = set
        .files
        .iter()
        .find(|f| f.file_path == "recovery.rs")
        .unwrap();
    assert_eq!(recovered.declarations.rows.len(), 1);
    assert!(matches!(
        recovered.declarations.sources[0].availability,
        RustDeclarationAvailability::Unknown { .. }
    ));
    let main = set.files.iter().find(|f| f.file_path == "main.rs").unwrap();
    assert!(main.declarations.rows.is_empty());
    let RustCallEvidence::Complete { evidence } = &main.calls else {
        panic!("main unknown")
    };
    assert_eq!(evidence.bodies.len(), 3);
    assert_eq!(evidence.sites.len(), 2);
    // Independent marker/brace coordinates from the frozen input, never copied IDs.
    for (site_range, body_range) in [([34, 40], [22, 78]), ([66, 72], [58, 75])] {
        let site = evidence
            .sites
            .iter()
            .find(|s| s.byte_range == site_range)
            .unwrap();
        assert_eq!(site.call_text, "leaf()");
        let body = evidence
            .bodies
            .iter()
            .find(|b| b.body_id == site.body_id)
            .unwrap();
        assert_eq!(body.byte_range, body_range);
    }
    let ffi = set.files.iter().find(|f| f.file_path == "ffi.rs").unwrap();
    assert_eq!(
        ffi.declarations.foreign_functions[0].body,
        RustDeclarationBody::AbsentForeignDeclaration
    );
    assert_eq!(
        ffi.declarations.extern_blocks[0].body,
        RustDeclarationBody::NonExecutableContainer
    );
    let RustCallEvidence::Complete { evidence } = &ffi.calls else {
        panic!("ffi unknown")
    };
    assert!(evidence.bodies.is_empty() && evidence.sites.is_empty());
    export(
        "actual.json",
        &json!({"graph": graph, "set": set, "checksum": rust_evidence_checksum_blake3_hex(&set).unwrap()}),
    );
}

fn mutation(
    name: &str,
    original: &RustEvidenceSet,
    value: Value,
    graph: &GraphIndexArtifact,
    inputs: &[(String, Vec<u8>)],
    stored_rejects: bool,
) -> Value {
    assert_ne!(
        serde_json::to_value(original).unwrap(),
        value,
        "no-op mutation {name}"
    );
    let changed: RustEvidenceSet = serde_json::from_value(value).unwrap();
    let stored = changed
        .validate_stored(graph)
        .err()
        .map(|e| format!("{e:#}"));
    let source = changed
        .validate_sources(graph, &sources(inputs))
        .err()
        .map(|e| format!("{e:#}"));
    assert_eq!(
        stored.is_some(),
        stored_rejects,
        "stored boundary: {name}: {stored:?}"
    );
    assert!(source.is_some(), "source accepted {name}");
    assert_ne!(
        rust_evidence_checksum_blake3_hex(original).unwrap(),
        rust_evidence_checksum_blake3_hex(&changed).unwrap()
    );
    json!({"name":name,"changed":changed,"stored_error":stored,"source_error":source})
}

#[test]
fn canonical_mutations_distinguish_stored_from_unseen_syntax() {
    let inputs = fixtures();
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let v = serde_json::to_value(&set).unwrap();
    let index = |path| set.files.iter().position(|f| f.file_path == path).unwrap();
    let main = index("main.rs");
    let ffi = index("ffi.rs");
    let cfg = index("cfg.rs");
    let mut results = Vec::new();
    let mut check = |name: &str, changed, stored| {
        results.push(mutation(name, &set, changed, &graph, &inputs, stored))
    };
    let mut x = v.clone();
    x["files"].as_array_mut().unwrap().remove(index("empty.rs"));
    check("missing_zero", x, true);
    let mut x = v.clone();
    let row = x["files"][0].clone();
    x["files"].as_array_mut().unwrap().insert(0, row);
    check("duplicate_source", x, true);
    let mut x = v.clone();
    x["files"][main]["source_oid"] = v["files"][ffi]["source_oid"].clone();
    check("wrong_existing_source", x, true);
    let mut x = v.clone();
    x["files"][main]["calls"]["complete"]["evidence"]["sites"]
        .as_array_mut()
        .unwrap()
        .pop();
    x["files"][main]["calls"]["complete"]["evidence"]["sources"][0]["site_count"] = json!(1);
    check("omitted_site_recounted", x, false);
    let mut x = v.clone();
    let row = x["files"][main]["calls"]["complete"]["evidence"]["sites"][0].clone();
    x["files"][main]["calls"]["complete"]["evidence"]["sites"]
        .as_array_mut()
        .unwrap()
        .push(row);
    check("duplicate_site", x, true);
    let mut x = v.clone();
    x["files"][main]["calls"]["complete"]["evidence"]["bodies"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    x["files"][main]["calls"]["complete"]["evidence"]["sources"][0]["body_count"] = json!(2);
    check("omitted_unused_body_recounted", x, false);
    let mut x = v.clone();
    let row = x["files"][main]["calls"]["complete"]["evidence"]["bodies"][0].clone();
    x["files"][main]["calls"]["complete"]["evidence"]["bodies"]
        .as_array_mut()
        .unwrap()
        .push(row);
    check("duplicate_body", x, true);
    let outer = v["files"][main]["calls"]["complete"]["evidence"]["sites"][0]["body_id"].clone();
    let mut x = v.clone();
    x["files"][main]["calls"]["complete"]["evidence"]["sites"][1]["body_id"] = outer;
    check("same_file_wrong_existing_body", x, false);
    let mut x = v.clone();
    let mut empty = v["files"][index("empty.rs")]["calls"].clone();
    empty["complete"]["evidence"]["sources"][0] = json!({"file_path":"cfg.rs","stable_file_id":set.files[cfg].stable_file_id,"source_oid":set.files[cfg].source_oid,"byte_length":set.files[cfg].byte_length,"body_count":0,"site_count":0});
    x["files"][cfg]["calls"] = empty;
    check("unknown_as_complete_zero", x, false);
    let mut x = v.clone();
    x["origins"][0]["graph_content_hash"] = json!("forged");
    check("origin_hash", x, true);
    let mut x = v.clone();
    x["files"][main]["declarations"]["graph_content_hash"] = json!("rebound");
    check("nested_header", x, true);
    let mut x = v.clone();
    x["files"][ffi]["declarations"]["foreign_functions"][0]["body"] =
        json!("non_executable_container");
    check("foreign_body_tag", x, true);
    let mut x = v.clone();
    x["files"][ffi]["declarations"]["extern_blocks"][0]["body"] =
        json!("absent_foreign_declaration");
    check("block_body_tag", x, true);
    let mut x = v.clone();
    x["files"][ffi]["declarations"]["extern_blocks"][0]["abi"]["explicit"]["literal"]["text"] =
        json!("\"X\"");
    check("same_length_abi_text", x, false);
    let mut x = v.clone();
    x["files"][main]["calls"]["complete"]["evidence"]["bodies"][0]["body_id"] = json!("forged");
    check("body_identity", x, true);
    let mut x = v.clone();
    x["files"][cfg]["calls"]["unknown"]["reason"] = json!("parse_recovery");
    check("wrong_unknown_reason", x, false);
    let mut x = v.clone();
    x["files"][ffi]["declarations"]["foreign_functions"][0]["core_range"][1] = json!(usize::MAX);
    check("metadata_bounds", x, true);
    export("mutations.json", &json!(results));
}

#[test]
fn wrong_actual_structural_and_lexical_owners_reject() {
    let inputs=vec![("owners.rs".into(), b"mod a { struct A(pub u8); unsafe extern \"C\" { pub fn imported(); } } mod b { struct B(u16); }".to_vec())];
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let v = serde_json::to_value(&set).unwrap();
    let other = &v["files"][0]["declarations"]["rows"][1]["structural_owner"];
    let mut results = Vec::new();
    for field in ["structural_owner", "lexical_owner"] {
        let mut x = v.clone();
        x["files"][0]["declarations"]["rows"][0][field] = other.clone();
        results.push(mutation(
            &format!("tuple_{field}"),
            &set,
            x,
            &graph,
            &inputs,
            true,
        ));
        let mut x = v.clone();
        x["files"][0]["declarations"]["foreign_functions"][0][field] = other.clone();
        results.push(mutation(
            &format!("foreign_{field}"),
            &set,
            x,
            &graph,
            &inputs,
            true,
        ));
    }
    export("owner-mutations.json", &json!(results));
}

#[test]
fn exact_domain_authentication_precedes_uncertainty() {
    let inputs = fixtures();
    let graph = graph(&inputs);
    let src = sources(&inputs);
    assert!(collect_rust_evidence(&graph, &src[..5]).is_err());
    let mut duplicate = src.clone();
    duplicate.push(src[0]);
    assert!(collect_rust_evidence(&graph, &duplicate).is_err());
    let mut stale = src.clone();
    stale[0].bytes = b"#[cfg(x)] fn stale() {}";
    assert!(collect_rust_evidence(&graph, &stale).is_err());
    let mut g = graph.clone();
    g.manifest_version = "stale".into();
    assert!(collect_rust_evidence(&g, &src).is_err());
    let mut g = graph.clone();
    g.files.push(g.files[0].clone());
    assert!(collect_rust_evidence(&g, &src).is_err());
    let mut g = graph.clone();
    let id = g
        .symbols
        .iter()
        .find(|s| s.file_path == "recovery.rs" && s.symbol_kind == "field")
        .unwrap()
        .stable_symbol_id
        .clone();
    g.symbols.retain(|s| s.stable_symbol_id != id);
    assert!(
        collect_rust_evidence(&g, &src).is_err(),
        "Unknown must not swallow missing owners"
    );
}

#[test]
fn original_snapshot_survives_current_aggregate_composition() {
    let old_inputs = vec![
        ("empty.rs".into(), b"pub struct Empty;\n".to_vec()),
        ("neighbor.py".into(), b"def old(): pass\n".to_vec()),
    ];
    let old_graph = graph(&old_inputs);
    let old = collect_rust_evidence(&old_graph, &sources(&old_inputs[..1])).unwrap();
    assert!(old.origins[0]
        .sources
        .iter()
        .any(|s| s.path == "neighbor.py"));
    let mut inputs = old_inputs[..1].to_vec();
    inputs.push(("new.rs".into(), b"fn fresh() {}".to_vec()));
    let graph = graph(&inputs);
    let mut composed = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    composed.files[0] = old.files[0].clone();
    composed.origins.extend(old.origins.clone());
    composed.origins.sort_by(|a, b| {
        (&a.manifest_version, &a.graph_content_hash)
            .cmp(&(&b.manifest_version, &b.graph_content_hash))
    });
    composed.validate_stored(&graph).unwrap();
    composed
        .validate_sources(&graph, &sources(&inputs))
        .unwrap();
    assert_eq!(composed.files[0], old.files[0]);
    let mut rebound = composed.clone();
    rebound.files[0].origin_graph_content_hash = graph.graph_content_hash.clone();
    assert_ne!(rebound, composed);
    let stored_error = rebound.validate_stored(&graph).unwrap_err().to_string();
    let source_error = rebound
        .validate_sources(&graph, &sources(&inputs))
        .unwrap_err()
        .to_string();
    export(
        "rebound.json",
        &json!({"changed":rebound,"stored_error":stored_error,"source_error":source_error}),
    );
    let mut forged = composed.clone();
    forged.origins[0].sources[0].content_oid = "forged".into();
    assert!(forged.validate_stored(&graph).is_err());
    export("composed.json", &json!({"graph":graph,"set":composed}));
}

#[test]
fn captured_empty_and_excluded_virtual_domains_have_no_origins() {
    let mut graph = graph(&[]);
    let empty = collect_rust_evidence(&graph, &[]).unwrap();
    assert!(empty.files.is_empty() && empty.origins.is_empty());
    empty.validate_sources(&graph, &[]).unwrap();
    export("captured-empty.json", &json!({"graph":graph,"set":empty}));
    for path in [
        "note.ipynb#cell:one.rs",
        "external://placeholder.rs",
        "readme.md",
    ] {
        graph.files.push(spur_graph::GraphFileArtifact {
            file_path: path.into(),
            stable_file_id: path.into(),
        });
        graph
            .file_manifests
            .push(spur_graph::GraphFileManifestEntry {
                path: path.into(),
                stable_file_id: path.into(),
                content_oid: spur_graph::git_blob_oid(b""),
                node_ids: vec![],
            });
    }
    graph.graph_content_hash = compute_graph_content_hash(
        graph
            .file_manifests
            .iter()
            .map(|m| (&m.path, &m.content_oid)),
    );
    let set = collect_rust_evidence(&graph, &[]).unwrap();
    assert!(set.files.is_empty() && set.origins.is_empty());
    set.validate_sources(&graph, &[]).unwrap();
    assert!(collect_rust_evidence(
        &graph,
        &[RustDeclarationSource {
            file_path: "note.ipynb#cell:one.rs",
            bytes: b""
        }]
    )
    .is_err());
}

#[test]
fn strict_envelope_and_enum_payload_decoding() {
    let inputs = fixtures();
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let original = serde_json::to_value(&set).unwrap();
    let mut errors = Vec::new();
    for pointer in [
        "",
        "/origins/0",
        "/origins/0/sources/0",
        "/files/0",
        "/files/0/calls/unknown",
    ] {
        let mut v = original.clone();
        v.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown_field".into(), json!(true));
        assert_ne!(original, v);
        let error = serde_json::from_value::<RustEvidenceSet>(v).unwrap_err();
        errors.push(json!({"pointer":pointer,"error":error.to_string()}));
    }
    let main = set
        .files
        .iter()
        .position(|f| f.file_path == "main.rs")
        .unwrap();
    let mut v = original.clone();
    v["files"][main]["calls"]["complete"]["unknown_field"] = json!(true);
    assert!(serde_json::from_value::<RustEvidenceSet>(v).is_err());
    export("strict-decoding.json", &json!(errors));
    let mut v = original;
    v["files"][0]["calls"]["unknown"]["reason"] = json!("all_panics_proven_absent");
    assert!(serde_json::from_value::<RustEvidenceSet>(v).is_err());
    let mut h = blake3::Hasher::new();
    h.update(b"spur-rust-evidence-tables-v1\0");
    h.update(&serde_json::to_vec(&set).unwrap());
    assert_eq!(
        rust_evidence_checksum_blake3_hex(&set).unwrap(),
        h.finalize().to_hex().to_string()
    );
}

#[test]
fn wrapper_preserves_lossless_multiline_tuple_and_method_ffi_semantics() {
    let bytes=b"mod m { pub struct S(\n #[cfg(x)] pub u8,\n /// docs\n pub(crate) Vec<(u8,u8)>,\n u16,\n); enum E { A(u8), B(u16) } impl S { fn method() { unsafe extern \"C\" { fn imported(); } } } }";
    let inputs = vec![("tuple.rs".into(), bytes.to_vec())];
    let graph = graph(&inputs);
    let expected = spur_graph::collect_rust_declarations(&graph, &sources(&inputs)).unwrap();
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    assert_eq!(set.files[0].declarations, expected);
    assert_eq!(expected.rows.len(), 5);
    assert!(matches!(
        set.files[0].calls,
        RustCallEvidence::Unknown {
            reason: RustEvidenceUnknownReason::UnevaluatedCfg
        }
    ));
    set.validate_sources(&graph, &sources(&inputs)).unwrap();
}

#[test]
fn foreign_signature_constants_preserve_r3_without_inventing_foreign_bodies() {
    let inputs = vec![(
        "ffi.rs".into(),
        b"fn size() -> usize { 1 } unsafe extern \"C\" { fn imported(x: [u8; size()]); }".to_vec(),
    )];
    let graph = graph(&inputs);
    let direct = spur_graph::panic_evidence::collect_rust_call_sites(
        &graph,
        &[spur_graph::panic_evidence::PanicEvidenceSource {
            file_path: "ffi.rs",
            bytes: &inputs[0].1,
        }],
    )
    .unwrap();
    assert_eq!(direct.sites.len(), 1);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let RustCallEvidence::Complete { evidence } = &set.files[0].calls else {
        panic!("unexpected unknown")
    };
    assert_eq!(evidence, &direct);
    assert_eq!(
        set.files[0].declarations.foreign_functions[0].body,
        RustDeclarationBody::AbsentForeignDeclaration
    );
    let foreign = &set.files[0].declarations.foreign_functions[0].stable_symbol_id;
    assert!(evidence
        .bodies
        .iter()
        .all(|b| b.definition_stable_symbol_id.as_ref() != Some(foreign)));
    set.validate_sources(&graph, &sources(&inputs)).unwrap();
}

#[test]
fn macro_tokens_do_not_create_cfg_uncertainty() {
    let inputs=vec![("mac.rs".into(),b"macro_rules! opaque { () => { #[cfg(x)] fn hidden() { leaf(); } } } opaque!(#[cfg_attr(x,y)]); fn visible() {}".to_vec())];
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    assert!(
        matches!(&set.files[0].calls,RustCallEvidence::Complete {evidence} if evidence.sites.is_empty() && evidence.bodies.len()==1)
    );
    set.validate_sources(&graph, &sources(&inputs)).unwrap();
}

#[test]
fn stored_body_definition_must_be_the_actual_existing_function() {
    let inputs = vec![(
        "nested.rs".into(),
        b"fn outer() { fn inner() { leaf(); } }".to_vec(),
    )];
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let mut changed = set.clone();
    let RustCallEvidence::Complete { evidence } = &mut changed.files[0].calls else {
        panic!("unexpected unknown")
    };
    let outer = evidence
        .bodies
        .iter()
        .find(|b| b.kind == spur_graph::panic_evidence::RustBodyKind::Function)
        .unwrap()
        .definition_stable_symbol_id
        .clone();
    let inner = evidence
        .bodies
        .iter_mut()
        .find(|b| b.kind == spur_graph::panic_evidence::RustBodyKind::NestedFunction)
        .unwrap();
    assert_ne!(inner.definition_stable_symbol_id, outer);
    inner.definition_stable_symbol_id = outer;
    assert!(
        changed.validate_stored(&graph).is_err(),
        "wrong existing outer function is not the nested body's definition"
    );
    assert!(changed.validate_sources(&graph, &sources(&inputs)).is_err());
}

/// ROOT can replay independently mutated actual JSON through the current APIs.
/// The normal suite always validates a real freshly collected canonical packet.
#[test]
fn replay_wrapper_packets_through_current_public_validators() {
    let inputs = fixtures();
    let graph = graph(&inputs);
    let set = collect_rust_evidence(&graph, &sources(&inputs)).unwrap();
    let packet = if let Ok(path) = std::env::var("D1_REPLAY_INPUT") {
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap()
    } else {
        json!({"cases":[{"name":"canonical","graph":graph,"set":set,"decode_ok":true,"stored_ok":true,"sources_ok":true}]})
    };
    let mut results = Vec::new();
    for case in packet["cases"].as_array().unwrap() {
        let graph: GraphIndexArtifact = serde_json::from_value(case["graph"].clone()).unwrap();
        let decoded = serde_json::from_value::<RustEvidenceSet>(case["set"].clone());
        assert_eq!(
            decoded.is_ok(),
            case["decode_ok"].as_bool().unwrap(),
            "decode {}",
            case["name"]
        );
        let (decode_error, stored_error, source_error) = match decoded {
            Ok(set) => (
                None,
                set.validate_stored(&graph).err().map(|e| format!("{e:#}")),
                set.validate_sources(&graph, &sources(&inputs))
                    .err()
                    .map(|e| format!("{e:#}")),
            ),
            Err(error) => (Some(error.to_string()), None, None),
        };
        if decode_error.is_none() {
            assert_eq!(
                stored_error.is_none(),
                case["stored_ok"].as_bool().unwrap(),
                "stored {}: {stored_error:?}",
                case["name"]
            );
            assert_eq!(
                source_error.is_none(),
                case["sources_ok"].as_bool().unwrap(),
                "sources {}: {source_error:?}",
                case["name"]
            );
        }
        results.push(json!({"name":case["name"],"decode_error":decode_error,"stored_error":stored_error,"source_error":source_error}));
    }
    export("packet-replay.json", &json!(results));
}

#[test]
fn rehashing_cannot_authenticate_forged_sources_or_graph_owners() {
    let inputs = fixtures();
    let mut forged_graph = graph(&inputs);
    let old_hash = forged_graph.graph_content_hash.clone();
    let other = forged_graph
        .file_manifests
        .iter()
        .find(|m| m.path == "ffi.rs")
        .unwrap()
        .content_oid
        .clone();
    forged_graph
        .file_manifests
        .iter_mut()
        .find(|m| m.path == "main.rs")
        .unwrap()
        .content_oid = other;
    forged_graph.graph_content_hash = compute_graph_content_hash(
        forged_graph
            .file_manifests
            .iter()
            .map(|m| (&m.path, &m.content_oid)),
    );
    assert_ne!(old_hash, forged_graph.graph_content_hash);
    let source_error = collect_rust_evidence(&forged_graph, &sources(&inputs))
        .unwrap_err()
        .to_string();
    let owner_inputs = vec![("owners.rs".into(), b"struct A(u8); struct B(u16);".to_vec())];
    let mut graph = graph(&owner_inputs);
    let mut set = collect_rust_evidence(&graph, &sources(&owner_inputs)).unwrap();
    let before = serde_json::to_value(&set).unwrap();
    let rows = &mut set.files[0].declarations.rows;
    let replacements = [
        rows[1].structural_owner.clone(),
        rows[0].structural_owner.clone(),
    ];
    for (row, replacement) in rows.iter_mut().zip(replacements) {
        let id = match &replacement {
            spur_graph::RustDeclarationOwner::Symbol { stable_symbol_id } => {
                stable_symbol_id.clone()
            }
            _ => panic!("symbol owner expected"),
        };
        graph
            .symbols
            .iter_mut()
            .find(|s| s.stable_symbol_id == id)
            .unwrap()
            .byte_range = [0, owner_inputs[0].1.len()];
        graph
            .edges
            .iter_mut()
            .find(|e| {
                e.relation == spur_graph::RelationKind::Contains
                    && e.target_stable_symbol_id.as_deref() == Some(row.stable_symbol_id.as_str())
            })
            .unwrap()
            .source_stable_symbol_id = id;
        row.structural_owner = replacement.clone();
        row.lexical_owner = replacement;
    }
    assert_ne!(before, serde_json::to_value(&set).unwrap());
    graph.header.content_hash_blake3 =
        Some(spur_graph::artifact_content_hash_blake3_hex(&graph).unwrap());
    set.validate_stored(&graph).unwrap();
    let owner_error = set
        .validate_sources(&graph, &sources(&owner_inputs))
        .unwrap_err()
        .to_string();
    export(
        "rehashed-forgeries.json",
        &json!({"source_collector_error":source_error,"owner_graph":graph,"owner_set":set,"owner_stored_error":null,"owner_source_error":owner_error}),
    );
}
