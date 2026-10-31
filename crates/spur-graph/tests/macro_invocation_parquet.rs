use std::{fs, fs::File, path::Path, sync::Arc};

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};
use parquet::file::properties::WriterProperties;
use spur_graph::{
    artifact_from_facts, build_facts, read_artifact_header_parquet, read_artifact_parquet,
    write_artifact_parquet, write_worktree_delta, GraphIndexArtifact, WriteOptions,
};

const SOURCE: &str = r#"#![cfg_attr(feature = "sample", allow(dead_code))]
macro_rules! local { () => { 1 }; }
const VALUE: i32 = local!();
#[cfg(test)]
mod tests {
    #[test]
    fn check() {
        std :: assert!(true);
        debug_assert_eq!(1, 1);
        alias!("unresolved");
        local!();
    }
}
"#;

fn build(root: &Path) -> GraphIndexArtifact {
    artifact_from_facts(&build_facts(root, None).unwrap().0, root).unwrap()
}

fn fixture() -> (tempfile::TempDir, GraphIndexArtifact) {
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("lib.rs"), SOURCE).unwrap();
    let artifact = build(source.path());
    assert!(!artifact.macro_invocations.as_ref().unwrap().rows.is_empty());
    (source, artifact)
}

fn write(artifact: &GraphIndexArtifact, dir: &Path) {
    write_artifact_parquet(artifact, dir, WriteOptions::default(), vec![]).unwrap();
}

fn manifest(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap()
}

fn change_manifest(dir: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let mut value = manifest(dir);
    change(&mut value);
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}

fn batches(path: &Path) -> Vec<RecordBatch> {
    ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
        .unwrap()
        .build()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

// Preserve valid Parquet and all metadata while changing one same-count cell.
fn tamper_string(path: &Path, column: &str, replacement: &str) {
    let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap()).unwrap();
    let schema = reader.schema().clone();
    let metadata = reader
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.key != "ARROW:schema")
        .collect();
    let mut rows = batches(path);
    let batch = &rows[0];
    let index = schema.index_of(column).unwrap();
    let strings = batch
        .column(index)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let mut values: Vec<Option<&str>> = strings.iter().collect();
    values[0] = Some(replacement);
    let mut columns: Vec<ArrayRef> = batch.columns().to_vec();
    columns[index] = Arc::new(StringArray::from(values));
    rows[0] = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(metadata))
        .build();
    let mut writer =
        ArrowWriter::try_new(File::create(path).unwrap(), schema, Some(props)).unwrap();
    for batch in rows {
        writer.write(&batch).unwrap();
    }
    writer.close().unwrap();
}

#[test]
fn full_roundtrip_preserves_all_fields_and_snapshot_after_source_changes() {
    let (source, artifact) = fixture();
    let out = tempfile::tempdir().unwrap();
    write(&artifact, out.path());
    fs::write(source.path().join("lib.rs"), "pub fn newer() {}\n").unwrap();
    let loaded = read_artifact_parquet(out.path()).unwrap();
    assert_eq!(loaded.macro_invocations, artifact.macro_invocations);
    let edge_set = |a: &GraphIndexArtifact| {
        let mut edges: Vec<_> = a
            .edges
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        edges.sort();
        edges
    };
    assert_eq!(edge_set(&loaded), edge_set(&artifact));
    let mut expected_symbols = artifact.symbols.clone();
    expected_symbols.sort_by(|a, b| a.stable_symbol_id.cmp(&b.stable_symbol_id));
    assert_eq!(loaded.symbols, expected_symbols);
    let set = artifact.macro_invocations.as_ref().unwrap();
    let m = manifest(out.path());
    assert_eq!(m["row_counts"]["macro_invocations"], set.rows.len());
    assert_eq!(m["row_counts"]["macro_scopes"], set.scopes.len());
    assert_eq!(m["macro_evidence"]["source_scope"], "physical_rust_files");
    assert_eq!(m["macro_evidence"]["schema_version"], set.schema_version);
    assert_eq!(m["macro_evidence"]["complete"], true);
    assert_eq!(
        m["macro_evidence"]["checksum_blake3"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    for (table, columns) in [
        (
            "macro_invocations.parquet",
            vec![
                "site_id",
                "file_path",
                "source_oid",
                "caller_stable_symbol_id",
                "raw_macro_path",
                "normalized_macro_path",
                "byte_range_start",
                "byte_range_end",
                "line_start",
                "line_end",
                "enclosing_scope_id",
                "enclosing_scope_kind",
                "test_context",
                "execution_phase",
                "resolution",
                "attributes",
            ],
        ),
        (
            "macro_scopes.parquet",
            vec![
                "scope_id",
                "file_path",
                "source_oid",
                "kind",
                "byte_range_start",
                "byte_range_end",
                "line_start",
                "line_end",
            ],
        ),
    ] {
        let reader =
            ParquetRecordBatchReaderBuilder::try_new(File::open(out.path().join(table)).unwrap())
                .unwrap();
        for column in columns {
            assert!(
                reader.schema().field_with_name(column).is_ok(),
                "{table}: {column}"
            );
        }
        assert_eq!(
            reader.schema().metadata()["source_scope"],
            "physical_rust_files"
        );
        assert_eq!(
            reader.schema().metadata()["graph_content_hash"],
            artifact.graph_content_hash
        );
    }
    eprintln!(
        "M4_OBSERVED={}",
        serde_json::json!({"manifest":m,"evidence":set,"sources":artifact.file_manifests})
    );
}

#[test]
fn slim_reader_preserves_core_evidence_and_validates_its_provenance() {
    let (_source, artifact) = fixture();
    let out = tempfile::tempdir().unwrap();
    write(&artifact, out.path());
    let loaded = spur_graph::store::parquet::read_artifact_parquet_slim(out.path()).unwrap();
    assert_eq!(loaded.macro_invocations, artifact.macro_invocations);
    fs::remove_file(out.path().join("macro_scopes.parquet")).unwrap();
    assert!(spur_graph::store::parquet::read_artifact_parquet_slim(out.path()).is_err());
}

#[test]
fn captured_empty_writes_both_zero_row_tables_and_legacy_none_stays_unavailable() {
    let source = tempfile::tempdir().unwrap();
    let mut artifact = build(source.path());
    assert!(artifact.macro_invocations.as_ref().unwrap().rows.is_empty());
    assert!(artifact
        .macro_invocations
        .as_ref()
        .unwrap()
        .scopes
        .is_empty());
    let out = tempfile::tempdir().unwrap();
    write(&artifact, out.path());
    for table in ["macro_invocations.parquet", "macro_scopes.parquet"] {
        let reader =
            ParquetRecordBatchReaderBuilder::try_new(File::open(out.path().join(table)).unwrap())
                .unwrap();
        assert_eq!(reader.metadata().file_metadata().num_rows(), 0);
    }
    assert_eq!(
        read_artifact_parquet(out.path()).unwrap().macro_invocations,
        artifact.macro_invocations
    );
    artifact.macro_invocations = None;
    let legacy = tempfile::tempdir().unwrap();
    write(&artifact, legacy.path());
    change_manifest(legacy.path(), |m| {
        m["schema_version"] = "spur-graph-schema-v10".into();
        m.as_object_mut().unwrap().remove("macro_evidence");
        m["row_counts"]
            .as_object_mut()
            .unwrap()
            .remove("macro_invocations");
        m["row_counts"]
            .as_object_mut()
            .unwrap()
            .remove("macro_scopes");
    });
    assert!(read_artifact_parquet(legacy.path())
        .unwrap()
        .macro_invocations
        .is_none());
}

#[test]
fn delta_roundtrip_preserves_rows_scopes_and_captured_empty() {
    let (source, artifact) = fixture();
    fs::write(
        source.path().join("lib.rs"),
        "pub fn changed() { panic!(\"dirty\"); }\n",
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    write_worktree_delta(&artifact, source.path(), out.path()).unwrap();
    let loaded = read_artifact_parquet(out.path()).unwrap();
    let set = loaded.macro_invocations.as_ref().unwrap();
    assert_eq!(set.rows.len(), 1);
    assert!(!set.scopes.is_empty());
    assert_eq!(
        set.rows[0].source_oid,
        spur_graph::git_blob_oid(&fs::read(source.path().join("lib.rs")).unwrap())
    );
    // No changes relative to the freshly built artifact produces a captured empty delta.
    let current = build(source.path());
    let unchanged = tempfile::tempdir().unwrap();
    write_worktree_delta(&current, source.path(), unchanged.path()).unwrap();
    let set = read_artifact_parquet(unchanged.path())
        .unwrap()
        .macro_invocations
        .unwrap();
    assert!(set.rows.is_empty() && set.scopes.is_empty());
}

#[test]
fn declared_complete_evidence_rejects_missing_truncated_tables_even_when_empty() {
    for empty in [false, true] {
        for table in ["macro_invocations.parquet", "macro_scopes.parquet"] {
            for truncate in [false, true] {
                let (_source, mut artifact) = fixture();
                if empty {
                    let set = artifact.macro_invocations.as_mut().unwrap();
                    set.rows.clear();
                    set.scopes.clear();
                }
                let out = tempfile::tempdir().unwrap();
                write(&artifact, out.path());
                if truncate {
                    fs::write(out.path().join(table), b"PAR1").unwrap();
                } else {
                    let _ = fs::remove_file(out.path().join(table));
                }
                assert!(
                    read_artifact_parquet(out.path()).is_err(),
                    "accepted {table}, empty={empty}, truncate={truncate}"
                );
            }
        }
    }
}

#[test]
fn counts_and_manifest_snapshot_metadata_are_enforced() {
    for pointer in [
        "/row_counts/macro_invocations",
        "/row_counts/macro_scopes",
        "/macro_evidence/schema_version",
        "/macro_evidence/extractor_version",
        "/macro_evidence/source_scope",
        "/macro_evidence/checksum_blake3",
        "/graph_content_hash",
        "/schema_version",
        "/extractor_version",
    ] {
        let (_source, artifact) = fixture();
        let out = tempfile::tempdir().unwrap();
        write(&artifact, out.path());
        change_manifest(out.path(), |m| {
            // JSON indexing keeps the pre-implementation test behavioral, not a compile error.
            if pointer.starts_with("/row_counts/") {
                m["row_counts"][pointer.rsplit('/').next().unwrap()] = 999.into();
            } else if pointer.starts_with("/macro_evidence/") {
                m["macro_evidence"][pointer.rsplit('/').next().unwrap()] = "wrong".into();
            } else {
                m[pointer.trim_start_matches('/')] = "wrong".into();
            }
        });
        assert!(
            read_artifact_parquet(out.path()).is_err(),
            "accepted changed {pointer}"
        );
    }
}

#[test]
fn same_count_valid_version_cell_tamper_is_rejected_for_both_tables() {
    for (table, column, value) in [
        ("macro_invocations.parquet", "raw_macro_path", "other"),
        ("macro_scopes.parquet", "scope_id", "other-scope"),
    ] {
        let (_source, artifact) = fixture();
        let out = tempfile::tempdir().unwrap();
        write(&artifact, out.path());
        assert!(
            out.path().join(table).exists(),
            "missing core table {table}"
        );
        tamper_string(&out.path().join(table), column, value);
        let error = read_artifact_parquet(out.path()).unwrap_err();
        assert!(format!("{error:#}").contains("checksum"), "{error:#}");
    }
}

#[test]
fn writer_rejects_invalid_source_identity_scope_caller_and_attribute_owner() {
    for case in 0..8 {
        let (_source, mut artifact) = fixture();
        let set = artifact.macro_invocations.as_mut().unwrap();
        match case {
            0 => set.rows[0].source_oid = "stale".into(),
            1 => set.rows[0].site_id = "wrong".into(),
            2 => set.scopes.clear(),
            3 => set.rows[0].enclosing.stable_symbol_id = "dangling".into(),
            4 => {
                set.rows
                    .iter_mut()
                    .find(|r| r.caller_stable_symbol_id.is_some())
                    .unwrap()
                    .caller_stable_symbol_id = Some("invented".into())
            }
            5 => {
                set.rows
                    .iter_mut()
                    .find(|r| !r.attributes.is_empty())
                    .unwrap()
                    .attributes[0]
                    .owner
                    .stable_symbol_id = "dangling".into()
            }
            6 => set.scopes[0].source_oid = "stale".into(),
            7 => set.rows.push(set.rows[0].clone()),
            _ => unreachable!(),
        }
        let out = tempfile::tempdir().unwrap();
        assert!(
            write_artifact_parquet(&artifact, out.path(), WriteOptions::default(), vec![]).is_err(),
            "accepted invalid case {case}"
        );
        assert!(!out.path().join("manifest.json").exists());
    }
}

#[test]
fn failed_core_table_write_never_declares_complete() {
    let (_source, artifact) = fixture();
    let out = tempfile::tempdir().unwrap();
    fs::create_dir(out.path().join("macro_scopes.parquet")).unwrap();
    assert!(
        write_artifact_parquet(&artifact, out.path(), WriteOptions::default(), vec![]).is_err()
    );
    assert!(!out.path().join("manifest.json").exists());
    assert!(read_artifact_header_parquet(out.path()).is_err());
}
