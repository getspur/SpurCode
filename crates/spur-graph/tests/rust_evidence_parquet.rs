use std::fs;
use std::{fs::File, path::Path, sync::Arc};

use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};
use serde_json::{json, Value};

use spur_graph::store::parquet::read_artifact_parquet_slim;
use spur_graph::{
    artifact_from_facts, build_facts, read_artifact_parquet, write_artifact_parquet, WriteOptions,
};
use spur_graph::{
    read_artifact_header_parquet, rust_evidence_capability_digest,
    rust_evidence_checksum_blake3_hex, rust_evidence_manifest_capability_digest,
    write_worktree_delta, GraphIndexArtifact, RustCallEvidence, RustDeclarationSource,
};

const TABLES: [&str; 5] = [
    "rust_evidence_origins.parquet",
    "rust_evidence_sources.parquet",
    "rust_evidence_bodies.parquet",
    "rust_evidence_sites.parquet",
    "rust_declarations.parquet",
];

fn build(root: &Path) -> GraphIndexArtifact {
    artifact_from_facts(&build_facts(root, None).unwrap().0, root).unwrap()
}

fn fixture() -> (
    tempfile::TempDir,
    GraphIndexArtifact,
    Vec<(String, Vec<u8>)>,
) {
    // Raw inputs and marker geometry predate transport; recollect every v13 ID natively.
    let pre: Value = serde_json::from_str(include_str!("../../../docs/research/artifacts/2026-11-06-rust-evidence-durability/D1/root-pre/inputs.json")).unwrap();
    let mut inputs: Vec<_> = pre["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let hex = f["hex"].as_str().unwrap();
            (
                f["path"].as_str().unwrap().to_owned(),
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    inputs.push((
        "tuple.rs".into(),
        b"pub struct A(pub u8); pub struct B(u16);\n".to_vec(),
    ));
    inputs.sort_by(|a, b| a.0.cmp(&b.0));
    let root = tempfile::tempdir().unwrap();
    for (path, bytes) in &inputs {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    let graph = build(root.path());
    (root, graph, inputs)
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

fn write(graph: &GraphIndexArtifact, dir: &Path) {
    write_artifact_parquet(graph, dir, WriteOptions::default(), vec![]).unwrap();
}

fn change_manifest(dir: &Path, f: impl FnOnce(&mut Value)) {
    let path = dir.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    f(&mut value);
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn batches(path: &Path) -> Vec<RecordBatch> {
    ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
        .unwrap()
        .build()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

// Keep valid Parquet, row counts and old metadata/checksum while replacing cells.
fn rewrite(path: &Path, f: impl FnOnce(Arc<Schema>, &mut Vec<RecordBatch>) -> Arc<Schema>) {
    let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap()).unwrap();
    let schema = reader.schema().clone();
    let mut rows = batches(path);
    let schema = f(schema, &mut rows);
    let mut writer = ArrowWriter::try_new(File::create(path).unwrap(), schema, None).unwrap();
    for row in rows {
        writer.write(&row).unwrap();
    }
    writer.close().unwrap();
}

fn tamper_string(path: &Path, column: &str, value: &str) {
    tamper_string_at(path, column, 0, Some(value));
}

fn tamper_string_at(path: &Path, column: &str, row: usize, value: Option<&str>) {
    rewrite(path, |schema, rows| {
        let index = schema.index_of(column).unwrap();
        let strings = rows[0]
            .column(index)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let mut values: Vec<_> = strings.iter().collect();
        values[row] = value;
        let mut columns = rows[0].columns().to_vec();
        columns[index] = Arc::new(StringArray::from(values));
        rows[0] = RecordBatch::try_new(schema.clone(), columns).unwrap();
        schema
    });
}

fn tamper_metadata(path: &Path, key: &str, value: &str) {
    rewrite(path, |schema, rows| {
        let mut metadata = schema.metadata().clone();
        metadata.insert(key.into(), value.into());
        let schema = Arc::new(Schema::new_with_metadata(schema.fields().clone(), metadata));
        for row in rows {
            *row = RecordBatch::try_new(schema.clone(), row.columns().to_vec()).unwrap();
        }
        schema
    });
}

fn table_export(path: &Path) -> Value {
    let result = (|| -> anyhow::Result<Value> {
        let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path)?)?;
        let schema = reader.schema().clone();
        let fields:Vec<_>=schema.fields().iter().map(|f|json!({"name":f.name(),"type":format!("{:?}",f.data_type()),"nullable":f.is_nullable()})).collect();
        let kv: Vec<_> = reader
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .into_iter()
            .flatten()
            .map(|v| json!({"key":v.key,"value":v.value}))
            .collect();
        let count = reader.metadata().file_metadata().num_rows();
        let mut rows = Vec::new();
        for batch in reader.build()? {
            let batch = batch?;
            for i in 0..batch.num_rows() {
                let mut row = serde_json::Map::new();
                for (field, column) in schema.fields().iter().zip(batch.columns()) {
                    let v = if column.is_null(i) {
                        Value::Null
                    } else if let Some(v) = column.as_any().downcast_ref::<StringArray>() {
                        json!(v.value(i))
                    } else if let Some(v) = column.as_any().downcast_ref::<Int64Array>() {
                        json!(v.value(i))
                    } else {
                        json!(format!("{:?}", column))
                    };
                    row.insert(field.name().clone(), v);
                }
                rows.push(Value::Object(row));
            }
        }
        Ok(
            json!({"fields":fields,"metadata":schema.metadata(),"file_metadata":kv,"count":count,"rows":rows}),
        )
    })();
    result.unwrap_or_else(|e| json!({"error":format!("{e:#}")}))
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest)
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn capture(name: &str, dir: &Path, graph: &GraphIndexArtifact, inputs: &[(String, Vec<u8>)]) {
    let Ok(root) = std::env::var("D5_EXPORT_DIR") else {
        return;
    };
    let root = Path::new(&root);
    let root = if root.is_absolute() {
        root.to_owned()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(root)
    };
    let out = root.join(name);
    fs::create_dir_all(&out).unwrap();
    copy_tree(dir, &out.join("disk"));
    let tables: serde_json::Map<_, _> = TABLES
        .iter()
        .map(|t| ((*t).into(), table_export(&dir.join(t))))
        .collect();
    let full = read_artifact_parquet(dir);
    let slim = read_artifact_parquet_slim(dir);
    let source_verdict = full.as_ref().ok().map(|g| {
        g.validate_rust_evidence_sources(&sources(inputs))
            .err()
            .map(|e| format!("{e:#}"))
    });
    let saved:Vec<_>=inputs.iter().map(|(p,b)|json!({"path":p,"hex":b.iter().map(|b|format!("{b:02x}")).collect::<String>(),"byte_length":b.len(),"source_oid":spur_graph::git_blob_oid(b)})).collect();
    let manifest_digest = read_artifact_header_parquet(dir)
        .ok()
        .map(|m| rust_evidence_manifest_capability_digest(&m).unwrap());
    let full_digest = full
        .as_ref()
        .ok()
        .map(|g| rust_evidence_capability_digest(g).unwrap());
    let slim_digest = slim
        .as_ref()
        .ok()
        .map(|g| rust_evidence_capability_digest(g).unwrap());
    let value = json!({"original_artifact":graph,"sources":saved,"tables":tables,"full":full.as_ref().ok(),"full_error":full.as_ref().err().map(|e|format!("{e:#}")),"slim":slim.as_ref().ok(),"slim_error":slim.as_ref().err().map(|e|format!("{e:#}")),"source_validation_error":source_verdict,"manifest_capability_digest":manifest_digest,"full_capability_digest":full_digest,"slim_capability_digest":slim_digest});
    fs::write(
        out.join("actual.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}

fn rejected(name: &str, dir: &Path, graph: &GraphIndexArtifact, inputs: &[(String, Vec<u8>)]) {
    assert!(read_artifact_parquet(dir).is_err(), "accepted full {name}");
    assert!(
        read_artifact_parquet_slim(dir).is_err(),
        "accepted slim {name}"
    );
    capture(name, dir, graph, inputs);
}

#[test]
fn disk_preserves_complete_zero_source() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph =
        artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(&graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    assert!(out.path().join("rust_evidence_sources.parquet").is_file());
    assert_eq!(
        read_artifact_parquet(out.path()).unwrap().rust_evidence,
        graph.rust_evidence
    );
    capture(
        "complete-zero-source",
        out.path(),
        &graph,
        &[("empty.rs".into(), b"pub struct Empty;\n".to_vec())],
    );
}

#[test]
fn full_and_slim_preserve_exact_snapshot_and_source_authentication_is_separate() {
    let (root, graph, inputs) = fixture();
    let out = tempfile::tempdir().unwrap();
    write(&graph, out.path());
    fs::write(root.path().join("main.rs"), b"pub fn replacement() {}\n").unwrap();
    let full = read_artifact_parquet(out.path()).unwrap();
    let slim = read_artifact_parquet_slim(out.path()).unwrap();
    assert_eq!(full.rust_evidence, graph.rust_evidence);
    assert_eq!(slim.rust_evidence, graph.rust_evidence);
    full.validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    let mut changed = inputs.clone();
    changed.iter_mut().find(|(p, _)| p == "main.rs").unwrap().1 =
        b"pub fn replacement() {}\n".to_vec();
    assert!(full
        .validate_rust_evidence_sources(&sources(&changed))
        .is_err());
    assert!(slim.file_manifests.is_empty() && slim.edges.is_empty());
    assert!(full.header.content_hash_blake3.is_none() && slim.header.content_hash_blake3.is_none());
    assert!(spur_graph::load_temporal_artifact_parquet(out.path())
        .unwrap()
        .rust_evidence
        .is_none());
    let set = full.rust_evidence.as_ref().unwrap();
    assert_eq!(set.files.len(), 7);
    let ffi = set.files.iter().find(|f| f.file_path == "ffi.rs").unwrap();
    assert_eq!(ffi.declarations.foreign_functions.len(), 1);
    assert_eq!(ffi.declarations.extern_blocks.len(), 1);
    let recovery = set
        .files
        .iter()
        .find(|f| f.file_path == "recovery.rs")
        .unwrap();
    assert_eq!(recovery.declarations.rows.len(), 1);
    assert!(matches!(recovery.calls, RustCallEvidence::Unknown { .. }));
    let main = set.files.iter().find(|f| f.file_path == "main.rs").unwrap();
    let RustCallEvidence::Complete { evidence } = &main.calls else {
        panic!()
    };
    assert_eq!(evidence.bodies.len(), 3);
    assert_eq!(evidence.sites.len(), 2);
    let raw = &inputs.iter().find(|(p, _)| p == "main.rs").unwrap().1;
    for (site, marker) in evidence
        .sites
        .iter()
        .zip([b"/*a*/ ".as_slice(), b"/*b*/ ".as_slice()])
    {
        let start = raw.windows(marker.len()).position(|w| w == marker).unwrap() + marker.len();
        assert_eq!(site.byte_range, [start, start + 6]);
        assert_eq!(site.call_text, "leaf()");
    }
    assert_ne!(evidence.sites[0].body_id, evidence.sites[1].body_id);
    capture("baseline-snapshot", out.path(), &graph, &inputs);
}

#[test]
fn some_empty_writes_five_exact_zero_schemas_and_none_writes_none() {
    // Explicit schema oracle from the frozen row contract, independent of writer helpers.
    let expected=[
        "origin_graph_content_hash:s,manifest_version:s,sources_json:s",
        "file_path:s,stable_file_id:s,source_oid:s,byte_length:i,origin_graph_content_hash:s,declaration_header_json:s,declaration_source_json:s,call_availability:s,call_unknown_reason:s?,call_header_json:s?,call_source_json:s?",
        "file_path:s,source_oid:s,origin_graph_content_hash:s,body_id:s,byte_range_start:i,byte_range_end:i,kind:s,definition_stable_symbol_id:s?",
        "file_path:s,source_oid:s,origin_graph_content_hash:s,site_id:s,body_id:s,byte_range_start:i,byte_range_end:i,call_text:s",
        "file_path:s,stable_file_id:s,source_oid:s,origin_graph_content_hash:s,stable_symbol_id:s,kind:s,structural_owner_json:s,lexical_owner_json:s,full_range_start:i,full_range_end:i,line_start:i,line_end:i,payload_json:s",
    ];
    let root = tempfile::tempdir().unwrap();
    let mut graph = build(root.path());
    let out = tempfile::tempdir().unwrap();
    write(&graph, out.path());
    for (table, expected) in TABLES.iter().zip(expected) {
        let v = table_export(&out.path().join(table));
        assert_eq!(v["count"], 0, "{table}");
        let actual = v["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                format!(
                    "{}:{}{}",
                    f["name"].as_str().unwrap(),
                    if f["type"] == "Int64" {
                        "i"
                    } else {
                        assert_eq!(f["type"], "Utf8");
                        "s"
                    },
                    if f["nullable"] == true { "?" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(actual, expected, "{table}");
        assert_eq!(v["metadata"]["table_name"], *table);
    }
    assert_eq!(
        read_artifact_parquet(out.path()).unwrap().rust_evidence,
        graph.rust_evidence
    );
    capture("some-empty", out.path(), &graph, &[]);
    graph.rust_evidence = None;
    let absent = tempfile::tempdir().unwrap();
    write(&graph, absent.path());
    for t in TABLES {
        assert!(!absent.path().join(t).exists());
    }
    assert!(read_artifact_parquet(absent.path())
        .unwrap()
        .rust_evidence
        .is_none());
    capture("none", absent.path(), &graph, &[]);
    assert_ne!(
        rust_evidence_capability_digest(&build(root.path())).unwrap(),
        rust_evidence_capability_digest(&graph).unwrap()
    );
}

#[test]
fn manifest_and_artifact_capabilities_match_for_present_zero_absent_and_unknown() {
    let (root, mut graph, inputs) = fixture();
    for name in ["mixed", "unknown", "zero", "none"] {
        if name == "unknown" {
            for (p, _) in &inputs {
                fs::remove_file(root.path().join(p)).unwrap();
            }
            fs::write(
                root.path().join("cfg.rs"),
                b"#[cfg(x)] pub struct Kept(u8);\n",
            )
            .unwrap();
            graph = build(root.path());
        }
        if name == "zero" {
            fs::remove_file(root.path().join("cfg.rs")).unwrap();
            graph = build(root.path());
        }
        if name == "none" {
            graph.rust_evidence = None;
        }
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        let manifest = read_artifact_header_parquet(out.path()).unwrap();
        assert_eq!(
            rust_evidence_manifest_capability_digest(&manifest).unwrap(),
            rust_evidence_capability_digest(&graph).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn every_table_is_required_even_at_zero_rows_and_failed_writes_do_not_publish() {
    let (_root, graph, inputs) = fixture();
    let empty_root = tempfile::tempdir().unwrap();
    let empty = build(empty_root.path());
    for (profile, graph, inputs) in [
        ("populated", &graph, inputs.as_slice()),
        ("zero", &empty, &[]),
    ] {
        for table in TABLES {
            for action in ["missing", "truncated"] {
                let out = tempfile::tempdir().unwrap();
                write(graph, out.path());
                if action == "missing" {
                    fs::remove_file(out.path().join(table)).unwrap();
                } else {
                    fs::write(out.path().join(table), b"PAR1").unwrap();
                }
                rejected(
                    &format!("{profile}-{table}-{action}"),
                    out.path(),
                    graph,
                    inputs,
                );
            }
            let out = tempfile::tempdir().unwrap();
            fs::create_dir(out.path().join(table)).unwrap();
            assert!(
                write_artifact_parquet(graph, out.path(), WriteOptions::default(), vec![]).is_err(),
                "{table}"
            );
            assert!(!out.path().join("manifest.json").exists());
        }
    }
}

#[test]
fn every_table_rejects_counts_and_all_shared_metadata_mutations() {
    let (_root, graph, inputs) = fixture();
    for (table, count) in
        TABLES
            .iter()
            .zip(["origins", "sources", "bodies", "sites", "declarations"])
    {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        change_manifest(out.path(), |m| {
            m["rust_evidence"]["row_counts"][count] = 999.into()
        });
        rejected(&format!("count-{count}"), out.path(), &graph, &inputs);
        for key in [
            "table_name",
            "graph_schema_version",
            "graph_extractor_version",
            "manifest_version",
            "graph_content_hash",
            "schema_version",
            "extractor_version",
            "declaration_schema_version",
            "declaration_extractor_version",
            "call_schema_version",
            "call_extractor_version",
            "source_scope",
            "checksum_blake3",
        ] {
            let out = tempfile::tempdir().unwrap();
            write(&graph, out.path());
            tamper_metadata(&out.path().join(table), key, "wrong");
            rejected(
                &format!("metadata-{table}-{key}"),
                out.path(),
                &graph,
                &inputs,
            );
        }
    }
}

#[test]
fn valid_parquet_same_count_mutations_cannot_keep_old_checksum() {
    let (_root, graph, inputs) = fixture();
    for (table, column, replacement) in [
        (TABLES[0], "sources_json", "[]"),
        (TABLES[1], "stable_file_id", "different"),
        (TABLES[2], "kind", "async_block"),
        (TABLES[3], "call_text", "fake()"),
        (TABLES[4], "payload_json", "{}"),
    ] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        let path = out.path().join(table);
        let before = table_export(&path)["count"].clone();
        let replacement = if table == TABLES[4] {
            let original = table_export(&path);
            let mut payload: Value =
                serde_json::from_str(original["rows"][0]["payload_json"].as_str().unwrap())
                    .unwrap();
            if payload.get("ordinal").is_some() {
                payload["ordinal"] = 99.into();
            } else {
                payload["entity_name"] = "different".into();
            }
            serde_json::to_string(&payload).unwrap()
        } else {
            replacement.to_owned()
        };
        tamper_string(&path, column, &replacement);
        assert_eq!(table_export(&path)["count"], before);
        rejected(
            &format!("cell-{table}-{column}"),
            out.path(),
            &graph,
            &inputs,
        );
        assert!(
            format!("{:#}", read_artifact_parquet(out.path()).unwrap_err()).contains("checksum"),
            "{table}"
        );
    }
}

#[test]
fn required_fields_wrong_types_negative_integers_enums_and_strict_json_reject() {
    let (_root, graph, inputs) = fixture();
    for table in TABLES {
        for mode in ["null", "type"] {
            let out = tempfile::tempdir().unwrap();
            write(&graph, out.path());
            rewrite(&out.path().join(table), |schema, rows| {
                let mut fields = schema
                    .fields()
                    .iter()
                    .map(|f| f.as_ref().clone())
                    .collect::<Vec<_>>();
                fields[0] = Field::new(
                    fields[0].name(),
                    if mode == "type" {
                        DataType::Int64
                    } else {
                        DataType::Utf8
                    },
                    mode == "null",
                );
                let schema = Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()));
                for row in rows {
                    let mut columns = row.columns().to_vec();
                    columns[0] = if mode == "type" {
                        Arc::new(Int64Array::from(vec![0; row.num_rows()])) as ArrayRef
                    } else {
                        Arc::new(StringArray::from(vec![None::<&str>; row.num_rows()]))
                    };
                    *row = RecordBatch::try_new(schema.clone(), columns).unwrap();
                }
                schema
            });
            rejected(&format!("{table}-{mode}"), out.path(), &graph, &inputs);
        }
    }
    for (table, column) in [
        (TABLES[1], "byte_length"),
        (TABLES[2], "byte_range_start"),
        (TABLES[3], "byte_range_end"),
        (TABLES[4], "line_start"),
    ] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        rewrite(&out.path().join(table), |schema, rows| {
            let index = schema.index_of(column).unwrap();
            let mut columns = rows[0].columns().to_vec();
            let mut values = columns[index]
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec();
            values[0] = -1;
            columns[index] = Arc::new(Int64Array::from(values));
            rows[0] = RecordBatch::try_new(schema.clone(), columns).unwrap();
            schema
        });
        rejected(&format!("negative-{table}"), out.path(), &graph, &inputs);
    }
    for (table, column) in [
        (TABLES[0], "sources_json"),
        (TABLES[1], "declaration_header_json"),
        (TABLES[1], "declaration_source_json"),
        (TABLES[4], "payload_json"),
        (TABLES[4], "structural_owner_json"),
        (TABLES[4], "lexical_owner_json"),
    ] {
        for value in ["{", "{\"unexpected\":true}"] {
            let out = tempfile::tempdir().unwrap();
            write(&graph, out.path());
            tamper_string(&out.path().join(table), column, value);
            rejected(
                &format!("json-{table}-{column}-{}", value.len()),
                out.path(),
                &graph,
                &inputs,
            );
        }
    }
    for (table, column, value) in [
        (TABLES[1], "call_availability", "mystery"),
        (TABLES[1], "call_unknown_reason", "mystery"),
        (TABLES[2], "kind", "foreign_body"),
        (TABLES[4], "kind", "field"),
    ] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        tamper_string(&out.path().join(table), column, value);
        rejected(
            &format!("enum-{table}-{column}"),
            out.path(),
            &graph,
            &inputs,
        );
    }
}

#[test]
fn manifest_version_errors_are_actionable_and_orphans_reject() {
    let (_root, graph, inputs) = fixture();
    for key in [
        "schema_version",
        "extractor_version",
        "declaration_schema_version",
        "declaration_extractor_version",
        "call_schema_version",
        "call_extractor_version",
    ] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        change_manifest(out.path(), |m| {
            m["rust_evidence"][key] = "unsupported".into()
        });
        let error = format!("{:#}", read_artifact_parquet(out.path()).unwrap_err());
        assert!(
            error.contains("incompatible Rust evidence")
                && error.contains("spur explore sync")
                && error.contains("unsupported")
                && error.contains(out.path().to_str().unwrap()),
            "{error}"
        );
    }
    for (key, value) in [
        ("schema_version", "spur-graph-schema-v12"),
        ("schema_version", "unknown"),
        ("manifest_version", "old"),
        ("extractor_version", "old"),
    ] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        change_manifest(out.path(), |m| m[key] = value.into());
        rejected(
            &format!("manifest-{key}-{value}"),
            out.path(),
            &graph,
            &inputs,
        );
    }
    for key in ["complete", "checksum_blake3", "row_counts"] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        change_manifest(out.path(), |m| {
            m["rust_evidence"].as_object_mut().unwrap().remove(key);
        });
        rejected(
            &format!("missing-manifest-{key}"),
            out.path(),
            &graph,
            &inputs,
        );
    }
    for table in TABLES {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        change_manifest(out.path(), |m| m["rust_evidence"] = Value::Null);
        for other in TABLES {
            if other != table {
                fs::remove_file(out.path().join(other)).unwrap();
            }
        }
        rejected(&format!("orphan-{table}"), out.path(), &graph, &inputs);
    }
}

#[test]
fn strict_json_lanes_and_duplicate_source_owner_projections_reject() {
    let (_root, graph, inputs) = fixture();
    for column in [
        "declaration_header_json",
        "declaration_source_json",
        "call_header_json",
        "call_source_json",
    ] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        let path = out.path().join(TABLES[1]);
        let rows = batches(&path);
        let batch = &rows[0];
        let idx = batch.schema().index_of(column).unwrap();
        let values = batch
            .column(idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let row = values.iter().position(|v| v.is_some()).unwrap();
        let mut value: Value = serde_json::from_str(values.value(row)).unwrap();
        value["unexpected"] = true.into();
        let encoded = serde_json::to_string(&value).unwrap();
        tamper_string_at(&path, column, row, Some(&encoded));
        rejected(&format!("strict-{column}"), out.path(), &graph, &inputs);
    }
    for column in ["call_header_json", "call_source_json"] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        let path = out.path().join(TABLES[1]);
        let rows = batches(&path);
        let idx = rows[0].schema().index_of(column).unwrap();
        let values = rows[0]
            .column(idx)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let row = values.iter().position(|v| v.is_some()).unwrap();
        tamper_string_at(&path, column, row, None);
        rejected(
            &format!("complete-null-{column}"),
            out.path(),
            &graph,
            &inputs,
        );
    }
    for table in &TABLES[1..] {
        for column in ["source_oid", "origin_graph_content_hash", "file_path"] {
            let out = tempfile::tempdir().unwrap();
            write(&graph, out.path());
            tamper_string(&out.path().join(table), column, "wrong");
            rejected(
                &format!("projection-{table}-{column}"),
                out.path(),
                &graph,
                &inputs,
            );
        }
    }
}

#[test]
fn absent_rust_evidence_accepts_only_exact_canonical_schema_names() {
    let root = tempfile::tempdir().unwrap();
    let mut graph = build(root.path());
    graph.rust_evidence = None;
    // Isolate the Rust absence guard from the independently versioned macro reader.
    graph.macro_invocations = None;
    let mut accepted_unknown = Vec::new();
    for version in [
        "spur-graph-schema-v012",
        "spur-graph-schema-v+12",
        "spur-graph-schema-v01",
        "spur-graph-schema-v+1",
        "spur-graph-schema-v013",
        "spur-graph-schema-v+13",
        "spur-graph-schema-v-12",
        "spur-graph-schema-v0",
        "spur-graph-schema-v999",
        "spur-graph-schema-v12 ",
        "unknown",
    ] {
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        change_manifest(out.path(), |m| m["schema_version"] = version.into());
        capture(&format!("absent-schema-{version}"), out.path(), &graph, &[]);
        for (reader, result) in [
            ("full", read_artifact_parquet(out.path())),
            ("slim", read_artifact_parquet_slim(out.path())),
        ] {
            match result {
                Ok(_) => accepted_unknown.push(format!("{reader}: {version}")),
                Err(error) => {
                    let error = format!("{error:#}");
                    assert!(
                        error.contains("incompatible Rust evidence disk schema")
                            && error.contains("spur-graph-schema-v13")
                            && error.contains(version)
                            && error.contains(out.path().to_str().unwrap())
                            && error.contains("rebuild with spur explore sync"),
                        "{reader}: {error}"
                    );
                }
            }
        }
    }
    assert!(
        accepted_unknown.is_empty(),
        "accepted undeclared schema names: {accepted_unknown:?}"
    );
}

#[test]
fn absent_rust_evidence_retains_all_canonical_legacy_schema_names() {
    let root = tempfile::tempdir().unwrap();
    let mut graph = build(root.path());
    graph.rust_evidence = None;
    graph.macro_invocations = None;
    for number in 1..=13 {
        let version = format!("spur-graph-schema-v{number}");
        let out = tempfile::tempdir().unwrap();
        write(&graph, out.path());
        change_manifest(out.path(), |m| m["schema_version"] = version.clone().into());
        for result in [
            read_artifact_parquet(out.path()),
            read_artifact_parquet_slim(out.path()),
        ] {
            assert!(result.unwrap().rust_evidence.is_none(), "{version}");
        }
        capture(&format!("absent-schema-{version}"), out.path(), &graph, &[]);
    }
}

#[test]
fn recognized_v12_absence_keeps_macro_tables_and_forces_full_rebuild() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("lib.rs"),
        b"pub fn check() { assert!(true); }\n",
    )
    .unwrap();
    let mut graph = build(root.path());
    graph.rust_evidence = None;
    let out = tempfile::tempdir().unwrap();
    write(&graph, out.path());
    change_manifest(out.path(), |m| {
        m["schema_version"] = "spur-graph-schema-v12".into();
        m["manifest_version"] = "recognized-v12-manifest".into();
    });
    for table in ["macro_invocations.parquet", "macro_scopes.parquet"] {
        tamper_metadata(
            &out.path().join(table),
            "graph_schema_version",
            "spur-graph-schema-v12",
        );
        tamper_metadata(
            &out.path().join(table),
            "manifest_version",
            "recognized-v12-manifest",
        );
    }
    let loaded = read_artifact_parquet(out.path()).unwrap();
    assert!(loaded.rust_evidence.is_none());
    assert_eq!(loaded.macro_invocations, graph.macro_invocations);
    assert_eq!(
        read_artifact_parquet_slim(out.path())
            .unwrap()
            .macro_invocations,
        graph.macro_invocations
    );
    let (_, mode, _) = spur_graph::artifact_from_facts_incremental(&loaded, root.path()).unwrap();
    assert_eq!(mode, spur_graph::BuildMode::Full);
    capture(
        "legacy-v12",
        out.path(),
        &graph,
        &[(
            "lib.rs".into(),
            b"pub fn check() { assert!(true); }\n".to_vec(),
        )],
    );
    change_manifest(out.path(), |m| {
        m["schema_version"] = "spur-graph-schema-v999".into()
    });
    assert!(read_artifact_parquet(out.path()).is_err());
}

#[test]
fn rehashed_existing_owner_forgery_passes_transport_but_fails_saved_sources() {
    let (_root, mut graph, inputs) = fixture();
    let file = graph
        .rust_evidence
        .as_mut()
        .unwrap()
        .files
        .iter_mut()
        .find(|f| f.file_path == "tuple.rs")
        .unwrap();
    let rows = &mut file.declarations.rows;
    let replacements = [
        rows[1].structural_owner.clone(),
        rows[0].structural_owner.clone(),
    ];
    for (row, replacement) in rows.iter_mut().zip(replacements) {
        let spur_graph::RustDeclarationOwner::Symbol {
            stable_symbol_id: id,
        } = &replacement
        else {
            panic!()
        };
        graph
            .symbols
            .iter_mut()
            .find(|s| &s.stable_symbol_id == id)
            .unwrap()
            .byte_range = [0, file.byte_length];
        graph
            .edges
            .iter_mut()
            .find(|e| {
                e.relation == spur_graph::RelationKind::Contains
                    && e.target_stable_symbol_id.as_deref() == Some(&row.stable_symbol_id)
            })
            .unwrap()
            .source_stable_symbol_id = id.clone();
        row.structural_owner = replacement.clone();
        row.lexical_owner = replacement;
    }
    graph
        .rust_evidence
        .as_ref()
        .unwrap()
        .validate_stored(&graph)
        .unwrap();
    graph.header.content_hash_blake3 =
        Some(spur_graph::artifact_content_hash_blake3_hex(&graph).unwrap());
    let out = tempfile::tempdir().unwrap();
    write(&graph, out.path());
    let loaded = read_artifact_parquet(out.path()).unwrap();
    assert!(loaded
        .validate_rust_evidence_sources(&sources(&inputs))
        .is_err());
    capture("rehashed-existing-owner", out.path(), &graph, &inputs);
}

#[test]
fn delta_roundtrips_changed_zero_unknown_no_change_and_deletion() {
    let (root, base, inputs) = fixture();
    for (name, replacement) in [
        (
            "changed",
            b"fn leaf() {} fn changed() { leaf(); }\n".as_slice(),
        ),
        ("zero-source", b"pub struct Empty;\n".as_slice()),
        (
            "unknown-source",
            b"#[cfg(x)] pub struct Kept(u8);\n".as_slice(),
        ),
    ] {
        fs::write(root.path().join("main.rs"), replacement).unwrap();
        let out = tempfile::tempdir().unwrap();
        write_worktree_delta(&base, root.path(), out.path()).unwrap();
        let delta = read_artifact_parquet(out.path()).unwrap();
        assert_eq!(delta.rust_evidence.as_ref().unwrap().files.len(), 1);
        let input = vec![("main.rs".into(), replacement.to_vec())];
        delta
            .validate_rust_evidence_sources(&sources(&input))
            .unwrap();
        assert_eq!(
            delta.rust_evidence,
            read_artifact_parquet_slim(out.path())
                .unwrap()
                .rust_evidence
        );
        capture(&format!("delta-{name}"), out.path(), &delta, &input);
    }
    let current = build(root.path());
    let out = tempfile::tempdir().unwrap();
    write_worktree_delta(&current, root.path(), out.path()).unwrap();
    let delta = read_artifact_parquet(out.path()).unwrap();
    assert!(delta.rust_evidence.as_ref().unwrap().files.is_empty());
    capture("delta-no-change", out.path(), &delta, &[]);
    fs::remove_file(root.path().join("main.rs")).unwrap();
    let out = tempfile::tempdir().unwrap();
    write_worktree_delta(&current, root.path(), out.path()).unwrap();
    let delta = read_artifact_parquet(out.path()).unwrap();
    assert!(delta.rust_evidence.as_ref().unwrap().files.is_empty());
    assert!(delta.tombstones.iter().any(|t| t.path == "main.rs"));
    capture("delta-deletion", out.path(), &delta, &[]);
    let _ = inputs;
}

#[test]
fn composed_incremental_snapshot_retains_complete_original_inventories() {
    let (root, base, mut inputs) = fixture();
    let initial = tempfile::tempdir().unwrap();
    write(&base, initial.path());
    let seed = read_artifact_parquet(initial.path()).unwrap();
    let replacement = b"fn leaf() {} fn changed() { leaf(); }\n";
    fs::write(root.path().join("main.rs"), replacement).unwrap();
    inputs.iter_mut().find(|(p, _)| p == "main.rs").unwrap().1 = replacement.to_vec();
    let (current, mode, stats) =
        spur_graph::artifact_from_facts_incremental(&seed, root.path()).unwrap();
    assert_eq!(mode, spur_graph::BuildMode::Incremental);
    assert_eq!(stats.reused_buckets, 6);
    assert_eq!(stats.changed_paths, 1);
    let before = seed.rust_evidence.as_ref().unwrap();
    let after = current.rust_evidence.as_ref().unwrap();
    assert_eq!(after.origins.len(), 2);
    assert!(after.origins.contains(&before.origins[0]));
    assert_eq!(before.origins[0].sources.len(), 7);
    for file in before.files.iter().filter(|f| f.file_path != "main.rs") {
        assert_eq!(
            after
                .files
                .iter()
                .find(|f| f.file_path == file.file_path)
                .unwrap(),
            file
        );
    }
    let out = tempfile::tempdir().unwrap();
    write(&current, out.path());
    let loaded = read_artifact_parquet(out.path()).unwrap();
    assert_eq!(loaded.rust_evidence, current.rust_evidence);
    loaded
        .validate_rust_evidence_sources(&sources(&inputs))
        .unwrap();
    assert_eq!(
        read_artifact_parquet_slim(out.path())
            .unwrap()
            .rust_evidence,
        current.rust_evidence
    );
    capture("incremental-snapshot", out.path(), &current, &inputs);
}

#[test]
fn rehashed_existing_body_forgery_passes_transport_but_fails_saved_sources() {
    let (_root, mut graph, inputs) = fixture();
    let set = graph.rust_evidence.as_mut().unwrap();
    let file = set
        .files
        .iter_mut()
        .find(|f| f.file_path == "main.rs")
        .unwrap();
    let RustCallEvidence::Complete { evidence } = &mut file.calls else {
        panic!()
    };
    evidence.sites[1].body_id = evidence.sites[0].body_id.clone();
    let checksum = rust_evidence_checksum_blake3_hex(set).unwrap();
    graph
        .rust_evidence
        .as_ref()
        .unwrap()
        .validate_stored(&graph)
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    write(&graph, out.path());
    let loaded = read_artifact_parquet(out.path()).unwrap();
    assert_eq!(
        rust_evidence_checksum_blake3_hex(loaded.rust_evidence.as_ref().unwrap()).unwrap(),
        checksum
    );
    assert!(loaded
        .validate_rust_evidence_sources(&sources(&inputs))
        .is_err());
    capture("rehashed-existing-body", out.path(), &graph, &inputs);
}

/// ROOT may copy a saved disk directory, mutate it with this native adapter,
/// and rerun the current reader; the output contains actual rows and verdicts.
#[test]
fn replay_disk_through_current_api() {
    let Ok(dir) = std::env::var("D5_REPLAY_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    if let Ok(mutation) = std::env::var("D5_REPLAY_MUTATION") {
        let parts: Vec<_> = mutation.splitn(4, ':').collect();
        match parts.as_slice() {
            ["delete", table] => {
                fs::remove_file(dir.join(table)).unwrap();
            }
            ["cell", table, column, value] => tamper_string(&dir.join(table), column, value),
            ["metadata", table, key, value] => tamper_metadata(&dir.join(table), key, value),
            ["checksum"] => change_manifest(dir, |m| {
                m["rust_evidence"]["checksum_blake3"] = "0".repeat(64).into()
            }),
            ["schema"] => change_manifest(dir, |m| m["schema_version"] = "unsupported".into()),
            _ => panic!("unknown D5 replay mutation"),
        }
    }
    let full = read_artifact_parquet(dir);
    let slim = read_artifact_parquet_slim(dir);
    let tables: serde_json::Map<_, _> = TABLES
        .iter()
        .map(|t| ((*t).into(), table_export(&dir.join(t))))
        .collect();
    let result = json!({"full":full.as_ref().ok(),"full_error":full.as_ref().err().map(|e|format!("{e:#}")),"slim":slim.as_ref().ok(),"slim_error":slim.as_ref().err().map(|e|format!("{e:#}")),"tables":tables});
    println!("{result}");
    if let Ok(out) = std::env::var("D5_REPLAY_OUTPUT") {
        fs::write(out, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}
