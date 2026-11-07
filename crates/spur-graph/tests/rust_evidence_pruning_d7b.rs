use std::{fs, path::Path, sync::Arc};

use serde_json::json;
use spur_graph::query_client::{GraphQueryClient, InMemoryClient, ParquetClient};
use spur_graph::{
    artifact_from_facts, build_facts, read_artifact_parquet, write_artifact_parquet,
    RustCallEvidence, RustEvidenceAvailability, RustEvidenceFilter, WriteOptions,
};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn footer_statistics_preserve_nonzero_rows(table: &str) {
    let root = tempfile::tempdir().unwrap();
    let source = b"struct Pair(pub u8, pub u16);\nfn a() { leaf(); leaf(); }\n";
    fs::write(root.path().join("a.rs"), source).unwrap();
    fs::write(root.path().join("b.rs"), b"fn b() { other(); }\n").unwrap();
    let graph =
        artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(&graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    let memory = InMemoryClient::new(Arc::new(graph.clone()));
    let oid = graph
        .file_manifests
        .iter()
        .find(|f| f.path == "a.rs")
        .unwrap()
        .content_oid
        .clone();
    let filters = [
        RustEvidenceFilter {
            file_path: Some("a.rs".into()),
            source_oid: None,
        },
        RustEvidenceFilter {
            file_path: Some("a.rs".into()),
            source_oid: Some(oid),
        },
    ];
    let expected = filters
        .iter()
        .map(|f| memory.rust_evidence(f).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(expected[0], expected[1]);
    assert_eq!(expected[0].availability, RustEvidenceAvailability::Complete);
    assert_eq!(expected[0].files.len(), 1);
    let file = &expected[0].files[0];
    assert!(!file.declarations.rows.is_empty());
    let RustCallEvidence::Complete { evidence } = &file.calls else {
        panic!("calls unavailable")
    };
    assert!(!evidence.bodies.is_empty());
    assert!(!evidence.sites.is_empty());
    let control = ParquetClient::open(out.path()).unwrap();
    let control_results = filters
        .iter()
        .map(|f| control.rust_evidence(f).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(control_results, expected);

    let packet = std::env::var_os("SPUR_D7B_PRUNING_PACKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("spur-d7b-pruning-{}", std::process::id()))
        })
        .join(table.trim_end_matches(".parquet"));
    fs::create_dir_all(&packet).unwrap();
    fs::write(packet.join("a.rs"), source).unwrap();
    fs::copy(root.path().join("b.rs"), packet.join("b.rs")).unwrap();
    fs::write(
        packet.join("artifact.json"),
        serde_json::to_vec_pretty(&graph).unwrap(),
    )
    .unwrap();
    copy_dir(out.path(), &packet.join("original"));

    let path = out.path().join(table);
    let original = fs::read(&path).unwrap();
    let end = original.len();
    assert_eq!(&original[end - 4..], b"PAR1");
    let footer_len = u32::from_le_bytes(original[end - 8..end - 4].try_into().unwrap()) as usize;
    let footer_start = end - 8 - footer_len;
    let positions = (footer_start..=end - 8 - b"a.rs".len())
        .filter(|&i| &original[i..i + b"a.rs".len()] == b"a.rs")
        .collect::<Vec<_>>();
    assert!(
        !positions.is_empty(),
        "fixture has no path statistics: {table}"
    );
    let mut changed = original.clone();
    for &i in &positions {
        changed[i..i + b"a.rs".len()].copy_from_slice(b"z.rs");
    }
    assert_eq!(&changed[..footer_start], &original[..footer_start]);
    assert_eq!(&changed[end - 8..], &original[end - 8..]);
    fs::write(path, changed).unwrap();
    copy_dir(out.path(), &packet.join("footer-mutated"));
    let loaded = read_artifact_parquet(out.path());
    if let Ok(loaded) = &loaded {
        assert_eq!(loaded.rust_evidence, graph.rust_evidence);
    }
    let actual = filters
        .iter()
        .map(|filter| {
            ParquetClient::open(out.path())
                .unwrap()
                .rust_evidence(filter)
                .map_err(|error| format!("{error:#}"))
        })
        .collect::<Vec<_>>();
    let observation = json!({
        "table": table, "footer_positions": positions, "data_pages_unchanged": true,
        "manifest_unchanged": fs::read(packet.join("original/manifest.json")).unwrap()
            == fs::read(out.path().join("manifest.json")).unwrap(),
        "decoded_envelope_unchanged": loaded.as_ref().is_ok_and(|v| v.rust_evidence == graph.rust_evidence),
        "full_read_error": loaded.as_ref().err().map(|e| format!("{e:#}")),
        "filters": filters, "memory": expected, "control_disk": control_results,
        "cold_footer_mutated_disk": actual, "packet": packet,
    });
    fs::write(
        packet.join("actual.json"),
        serde_json::to_vec_pretty(&observation).unwrap(),
    )
    .unwrap();
    println!("D7B_PRUNING {observation}");
    for (result, wanted) in actual.iter().zip(&expected) {
        assert!(
            result.as_ref().map_or(true, |value| value == wanted),
            "footer-only statistics silently changed whole-file rows in {table}: {result:?}"
        );
    }
}

#[test]
fn footer_statistics_cannot_drop_nonzero_bodies() {
    footer_statistics_preserve_nonzero_rows("rust_evidence_bodies.parquet");
}

#[test]
fn footer_statistics_cannot_drop_nonzero_sites() {
    footer_statistics_preserve_nonzero_rows("rust_evidence_sites.parquet");
}

#[test]
fn footer_statistics_cannot_drop_nonzero_declarations() {
    footer_statistics_preserve_nonzero_rows("rust_declarations.parquet");
}
