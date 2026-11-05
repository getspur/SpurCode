use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};
use spur_graph::{
    artifact_from_facts, artifact_from_facts_for_paths, build_facts, build_facts_for_paths,
    collect_rust_evidence, git_blob_oid, GraphIndexArtifact, InMemoryClient, OverlayClient,
    RustCallEvidence, RustDeclarationSource,
};

fn build(root: &Path) -> spur_graph::GraphIndexArtifact {
    artifact_from_facts(&build_facts(root, None).unwrap().0, root).unwrap()
}

#[test]
fn full_build_captures_zero_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph = build(root.path());
    let value = serde_json::to_value(&graph).unwrap();
    assert!(value["rust_evidence"].is_object());
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let pre: Value = serde_json::from_str(include_str!("../../../docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/root-pre/inputs.json")).unwrap();
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

fn write(root: &Path, inputs: &[(String, Vec<u8>)]) {
    for (path, bytes) in inputs {
        fs::write(root.join(path), bytes).unwrap();
    }
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

fn export(name: &str, graph: &GraphIndexArtifact, inputs: &[(String, Vec<u8>)]) {
    export_value(name, &packet(graph, inputs));
}

fn export_value(name: &str, value: &Value) {
    if let Ok(dir) = std::env::var("D3_EXPORT_DIR") {
        let dir = PathBuf::from(dir);
        let dir = if dir.is_absolute() {
            dir
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(dir)
        };
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            Path::new(&dir).join(name),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

fn packet(graph: &GraphIndexArtifact, inputs: &[(String, Vec<u8>)]) -> Value {
    use sha2::{Digest, Sha256};
    json!({"artifact":graph,
        "sources": inputs.iter().map(|(path, bytes)| json!({"path":path,
            "hex":bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "sha256":format!("{:x}", Sha256::digest(bytes)), "source_oid":git_blob_oid(bytes),
            "byte_length":bytes.len()})).collect::<Vec<_>>(),
        "canonical_hash":spur_graph::artifact_content_hash_blake3_hex(graph).unwrap(),
        "capability_digest":spur_graph::rust_evidence_capability_digest(graph).unwrap(),
        "envelope_checksum":graph.rust_evidence.as_ref().map(|s| spur_graph::rust_evidence_checksum_blake3_hex(s).unwrap()),
        "source_error": graph.validate_rust_evidence_sources(&sources(inputs)).err().map(|e| format!("{e:#}"))})
}

fn paths(graph: &GraphIndexArtifact) -> BTreeSet<&str> {
    graph
        .file_manifests
        .iter()
        .map(|m| m.path.as_str())
        .collect()
}

#[test]
fn full_six_sources_match_accepted_manual_collection() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let graph = build(root.path());
    graph
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    let set = graph.rust_evidence.as_ref().unwrap();
    assert_eq!(set.files.len(), 6);
    assert_eq!(
        *set,
        collect_rust_evidence(&graph, &sources(&inputs)).unwrap()
    );
    let manual: Value = serde_json::from_str(include_str!("../../../docs/research/artifacts/2026-11-06-rust-evidence-durability/D2/ROOT-review/actual-full/populated.json")).unwrap();
    assert_eq!(
        serde_json::to_value(set).unwrap(),
        manual["present"]["artifact"]["rust_evidence"]
    );
    for file in &set.files {
        let input = inputs.iter().find(|(p, _)| p == &file.file_path).unwrap();
        assert_eq!(file.source_oid, git_blob_oid(&input.1));
        assert_eq!(file.byte_length, input.1.len());
    }
    let main = set.files.iter().find(|f| f.file_path == "main.rs").unwrap();
    match &main.calls {
        RustCallEvidence::Complete { evidence } => {
            assert_eq!(evidence.bodies.len(), 3);
            assert_eq!(evidence.sites.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    export("full.json", &graph, &inputs);
}

#[test]
fn selected_domain_contains_only_requested_current_files() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let selected = [PathBuf::from("main.rs"), PathBuf::from("empty.rs")];
    let facts = build_facts_for_paths(root.path(), &selected).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["empty.rs", "main.rs"]));
    let inputs: Vec<_> = inputs
        .into_iter()
        .filter(|(p, _)| p == "main.rs" || p == "empty.rs")
        .collect();
    graph
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    assert_eq!(graph.rust_evidence.as_ref().unwrap().files.len(), 2);
    export("selected.json", &graph, &inputs);
}

#[test]
fn partial_facts_never_advertise_unselected_zero_coverage() {
    let root = tempfile::tempdir().unwrap();
    let inputs = vec![
        ("a.rs".into(), b"pub fn a() {}\n".to_vec()),
        ("b.rs".into(), b"pub struct B;\n".to_vec()),
    ];
    write(root.path(), &inputs);
    let selected = [PathBuf::from("a.rs")];
    let facts = build_facts_for_paths(root.path(), &selected).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["a.rs"]));
    assert_eq!(
        graph
            .rust_evidence
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|f| f.file_path.as_str())
            .collect::<Vec<_>>(),
        ["a.rs"]
    );
}

#[test]
fn selected_normalizes_duplicates_and_rejects_escaping_paths() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let facts = build_facts(root.path(), None).unwrap().0;
    let absolute = root.path().canonicalize().unwrap().join("empty.rs");
    let selected = [
        PathBuf::from("./empty.rs"),
        PathBuf::from("unused/../empty.rs"),
        absolute,
    ];
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["empty.rs"]));
    assert_eq!(graph.rust_evidence.as_ref().unwrap().files.len(), 1);
    for path in [PathBuf::from("../outside.rs"), PathBuf::from("/outside.rs")] {
        assert!(artifact_from_facts_for_paths(&facts, root.path(), &[path]).is_err());
    }
}

#[test]
fn empty_workspace_is_captured_and_deleted_selection_has_no_active_source() {
    let root = tempfile::tempdir().unwrap();
    let graph = build(root.path());
    let set = graph.rust_evidence.as_ref().unwrap();
    assert!(set.files.is_empty() && set.origins.is_empty());
    graph.validate_rust_evidence_sources(&[]).unwrap();
    export("empty.json", &graph, &[]);
    let selected = [PathBuf::from("deleted.rs")];
    let facts = build_facts_for_paths(root.path(), &selected).unwrap();
    let graph = artifact_from_facts_for_paths(&facts, root.path(), &selected).unwrap();
    assert!(graph.file_manifests.is_empty());
    assert!(graph.rust_evidence.as_ref().unwrap().files.is_empty());
}

#[test]
fn mixed_nonrust_and_virtual_sources_do_not_enter_rust_domain() {
    let root = tempfile::tempdir().unwrap();
    let inputs = vec![("empty.rs".into(), b"pub struct Empty;\n".to_vec()), ("helper.py".into(), b"def hello():\n    pass\n".to_vec()),
        ("book.ipynb".into(), br##"{"cells":[{"cell_type":"code","id":"r","metadata":{"language":"rust"},"source":["pub fn cell() {}"],"outputs":[],"execution_count":null}],"metadata":{"kernelspec":{"language":"rust","name":"rust","display_name":"Rust"}},"nbformat":4,"nbformat_minor":5}"##.to_vec())];
    write(root.path(), &inputs);
    let graph = build(root.path());
    assert!(paths(&graph).contains("helper.py"));
    assert_eq!(
        graph
            .rust_evidence
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|f| f.file_path.as_str())
            .collect::<Vec<_>>(),
        ["empty.rs"]
    );
    graph
        .validate_rust_evidence_sources(&sources(&inputs[..1]))
        .unwrap();
    export("mixed.json", &graph, &inputs[..1]);
}

#[test]
fn missing_tuple_and_foreign_owners_are_integrity_errors() {
    let mut errors = Vec::new();
    for path in ["ffi.rs", "recovery.rs"] {
        let root = tempfile::tempdir().unwrap();
        let inputs: Vec<_> = fixtures()
            .into_iter()
            .filter(|(p, _)| p == path || p == "empty.rs")
            .collect();
        write(root.path(), &inputs);
        let mut facts = build_facts(root.path(), None).unwrap().0;
        facts.nodes.retain(|n| n.kind == spur_graph::NodeKind::File);
        let error = artifact_from_facts(&facts, root.path()).unwrap_err();
        assert!(!error.to_string().is_empty());
        errors.push(json!({"source":path,"error":format!("{error:#}")}));
    }
    export_value("owner-errors.json", &json!(errors));
}

#[test]
fn source_and_oid_mutations_fail_current_native_validation() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let graph = build(root.path());
    let mut mutated = inputs.clone();
    mutated.iter_mut().find(|(p, _)| p == "main.rs").unwrap().1[0] = b'X';
    assert!(graph
        .validate_rust_evidence_sources(&sources(&mutated))
        .is_err());
    let mut forged = graph.clone();
    forged
        .file_manifests
        .iter_mut()
        .find(|m| m.path == "main.rs")
        .unwrap()
        .content_oid = git_blob_oid(b"wrong\n");
    assert!(forged
        .validate_rust_evidence_sources(&sources(&inputs))
        .is_err());
    let mut verdicts = vec![
        json!({"case":"unchanged_graph_changed_source", "error":graph.validate_rust_evidence_sources(&sources(&mutated)).unwrap_err().to_string()}),
        json!({"case":"manifest_oid_mutation", "error":forged.validate_rust_evidence_sources(&sources(&inputs)).unwrap_err().to_string()}),
    ];
    for path in ["ffi.rs", "recovery.rs"] {
        let mut forged = graph.clone();
        forged.symbols.retain(|s| s.file_path != path);
        verdicts.push(json!({"case":format!("missing_owner:{path}"), "error":forged.validate_rust_evidence_sources(&sources(&inputs)).unwrap_err().to_string()}));
    }
    export_value("mutation-verdicts.json", &json!(verdicts));
}

#[test]
fn selected_full_facts_restrict_macro_carriers_before_composition() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.rs"),
        "pub fn a() { println!(\"a\"); }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("b.rs"),
        "pub fn b() { println!(\"b\"); }\n",
    )
    .unwrap();
    let facts = build_facts(root.path(), None).unwrap().0;
    let graph =
        artifact_from_facts_for_paths(&facts, root.path(), &[PathBuf::from("a.rs")]).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["a.rs"]));
    assert!(graph
        .macro_invocations
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .all(|r| r.file_path == "a.rs"));
}

#[test]
fn stale_index_oid_is_rejected_after_current_bytes_are_read() {
    use std::process::Command;
    let root = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(root.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    git(&["add", "empty.rs"]);
    git(&[
        "-c",
        "user.name=D3 fixture",
        "-c",
        "user.email=d3@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--no-verify",
        "-qm",
        "fixture",
    ]);
    git(&["update-index", "--assume-unchanged", "empty.rs"]);
    fs::write(root.path().join("empty.rs"), "pub struct Other;\n").unwrap();
    let facts = build_facts(root.path(), None).unwrap().0;
    let error = artifact_from_facts(&facts, root.path()).unwrap_err();
    assert!(
        error.to_string().contains("source OID mismatch"),
        "{error:#}"
    );
    export_value(
        "oid-race-error.json",
        &json!({"error":format!("{error:#}"),"method":"tracked assume-unchanged index OID versus freshly read worktree bytes"}),
    );
}

#[test]
fn selected_io_errors_and_symlink_escapes_are_not_unknown() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("directory.rs")).unwrap();
    let facts = spur_graph::extract::GraphFacts::empty();
    assert!(
        artifact_from_facts_for_paths(&facts, root.path(), &[PathBuf::from("directory.rs")])
            .is_err()
    );
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.rs"), "pub struct Outside;\n").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("outside.rs"),
            root.path().join("escape.rs"),
        )
        .unwrap();
        assert!(
            artifact_from_facts_for_paths(&facts, root.path(), &[PathBuf::from("escape.rs")])
                .is_err()
        );
    }
}

#[test]
fn overlay_extraction_uses_exact_selected_domain() {
    let root = tempfile::tempdir().unwrap();
    let inputs = fixtures();
    write(root.path(), &inputs);
    let selected = [PathBuf::from("main.rs"), PathBuf::from("empty.rs")];
    let (graph, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &selected).unwrap();
    assert_eq!(paths(&graph), BTreeSet::from(["empty.rs", "main.rs"]));
    assert_eq!(shadowed.len(), 2);
    let inputs: Vec<_> = inputs
        .into_iter()
        .filter(|(p, _)| p == "main.rs" || p == "empty.rs")
        .collect();
    graph
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    export("overlay.json", &graph, &inputs);
}

/// Reusable native validator: D3_PACKET names an arbitrary exported/forged packet.
#[test]
fn replay_packet_through_current_api() {
    let Ok(path) = std::env::var("D3_PACKET") else {
        return;
    };
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let graph: GraphIndexArtifact = serde_json::from_value(value["artifact"].clone()).unwrap();
    let inputs: Vec<_> = value["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let hex = s["hex"].as_str().unwrap();
            (
                s["path"].as_str().unwrap().to_owned(),
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect(),
            )
        })
        .collect();
    let result = packet(&graph, &inputs);
    println!(
        "{}",
        json!({"source_error":result["source_error"],"canonical_hash":result["canonical_hash"],"capability_digest":result["capability_digest"],"envelope_checksum":result["envelope_checksum"]})
    );
    if let Ok(out) = std::env::var("D3_REPLAY_OUTPUT") {
        fs::write(out, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}
