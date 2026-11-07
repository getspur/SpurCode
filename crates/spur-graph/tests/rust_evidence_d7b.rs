use std::{fs, path::Path, sync::Arc};

use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
use parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};
use serde_json::json;
use spur_graph::query_client::{GraphQueryClient, InMemoryClient, ParquetClient};
use spur_graph::{
    artifact_from_facts, build_facts, write_artifact_parquet, GraphIndexArtifact,
    RustEvidenceAvailability as Availability, RustEvidenceFilter as Filter,
    RustEvidenceQueryResult, WriteOptions,
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

fn file(path: &str) -> Filter {
    Filter {
        file_path: Some(path.into()),
        source_oid: None,
    }
}

fn fixture() -> (tempfile::TempDir, GraphIndexArtifact) {
    let root = tempfile::tempdir().unwrap();
    for (path, text) in [
        (
            "a.rs",
            "struct Pair(pub u8, pub u16);\nfn a() { leaf(); leaf(); }\n",
        ),
        ("b.rs", "fn b() { other(); }\n"),
        ("empty.rs", "pub struct Empty;\n"),
        (
            "cfg.rs",
            "struct Visible(pub u8);\n#[cfg(feature = \"x\")] fn hidden() { leaf(); }\n",
        ),
        ("recovery.rs", "struct Kept(pub u8);\nfn broken( {\n"),
        ("ffi.rs", "unsafe extern \"C\" { pub fn foreign(x: u8); }\n"),
        ("readme.md", "physical non-Rust source\n"),
        ("physical.ipynb#cell:actual.rs", "fn physical() {}\n"),
    ] {
        fs::write(root.path().join(path), text).unwrap();
    }
    let artifact = build(root.path());
    (root, artifact)
}

fn disk(graph: &GraphIndexArtifact) -> (tempfile::TempDir, ParquetClient) {
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    let client = ParquetClient::open(out.path()).unwrap();
    (out, client)
}

fn export(name: &str, value: serde_json::Value) {
    println!("D7_NATIVE {}", json!({"name":name,"value":value}));
}

#[test]
fn d7b_static_reference_forwards_native_evidence() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph = build(root.path());
    let (_out, disk) = disk(&graph);
    let memory = InMemoryClient::new(Arc::new(graph));
    fn through_reference<T: GraphQueryClient>(client: &T) -> RustEvidenceQueryResult {
        <&T as GraphQueryClient>::rust_evidence(&client, &file("empty.rs")).unwrap()
    }
    for result in [through_reference(&memory), through_reference(&disk)] {
        assert_eq!(result.availability, Availability::Complete);
        assert_eq!(result.source_domain.as_ref().unwrap().len(), 1);
        assert_eq!(result.files.len(), 1);
    }
}

#[test]
fn d7b_declaration_unknown_retains_complete_calls_and_rows() {
    let (_root, mut graph) = fixture();
    let record = graph
        .rust_evidence
        .as_mut()
        .unwrap()
        .files
        .iter_mut()
        .find(|f| f.file_path == "a.rs")
        .unwrap();
    record.declarations.sources[0].availability =
        spur_graph::RustDeclarationAvailability::Unknown {
            reason: "recorded declaration uncertainty".into(),
        };
    let expected = record.clone();
    assert!(!expected.declarations.rows.is_empty());
    assert!(matches!(
        &expected.calls,
        spur_graph::RustCallEvidence::Complete { .. }
    ));
    let (_out, disk) = disk(&graph);
    let memory = InMemoryClient::new(Arc::new(graph));
    let result = memory.rust_evidence(&file("a.rs")).unwrap();
    assert_eq!(result.availability, Availability::Partial);
    assert_eq!(result.files, vec![expected]);
    assert_eq!(disk.rust_evidence(&file("a.rs")).unwrap(), result);
    assert_eq!(
        memory
            .rust_evidence(&Filter::default())
            .unwrap()
            .availability,
        Availability::Partial
    );
    export("declaration_unknown", json!(result));
}

#[test]
fn d7b_legacy_domain_checks_wrong_existing_oid_and_shape_before_unsupported() {
    let (_root, mut graph) = fixture();
    graph.rust_evidence = None;
    let wrong = graph
        .file_manifests
        .iter()
        .find(|m| m.path == "b.rs")
        .unwrap()
        .content_oid
        .clone();
    let correct = graph
        .file_manifests
        .iter()
        .find(|m| m.path == "a.rs")
        .unwrap()
        .content_oid
        .clone();
    let (_out, disk) = disk(&graph);
    let memory = InMemoryClient::new(Arc::new(graph));
    for client in [&memory as &dyn GraphQueryClient, &disk] {
        assert!(client
            .rust_evidence(&Filter {
                source_oid: Some(wrong.clone()),
                ..file("a.rs")
            })
            .is_err());
        let result = client
            .rust_evidence(&Filter {
                source_oid: Some(correct.clone()),
                ..file("a.rs")
            })
            .unwrap();
        assert_eq!(result.availability, Availability::Unavailable);
        assert_eq!(result.source_domain.unwrap()[0].source_oid, correct);
        assert!(result.files.is_empty());
        for path in [
            "readme.md",
            "note.ipynb#cell:virtual.rs",
            "external://pkg/x.rs",
        ] {
            assert!(client
                .rust_evidence(&Filter {
                    source_oid: Some(wrong.clone()),
                    ..file(path)
                })
                .is_err());
        }
    }
}

#[test]
fn d7b_frozen_pruned_queries_keep_original_values_after_removal() {
    let (root, graph) = fixture();
    let (out, disk) = disk(&graph);
    let memory = InMemoryClient::new(Arc::new(graph));
    let first = disk.rust_evidence(&file("empty.rs")).unwrap();
    assert_eq!(first.availability, Availability::Complete);
    assert_eq!(first.files.len(), 1);
    for table in TABLES {
        fs::remove_file(out.path().join(table)).unwrap();
    }
    fs::write(root.path().join("a.rs"), "changed current bytes\n").unwrap();
    for filter in [
        file("a.rs"),
        file("cfg.rs"),
        file("ffi.rs"),
        Filter::default(),
    ] {
        let expected = memory.rust_evidence(&filter).unwrap();
        assert!(!expected.files.is_empty());
        assert_eq!(disk.rust_evidence(&filter).unwrap(), expected);
    }
}

// Recompute the shared envelope checksum and every transport metadata copy.
// The forgery must reach graph-context validation, not fail a stale checksum.
fn rehash_replacement(
    graph: &GraphIndexArtifact,
    dir: &Path,
    old: &str,
    new: &str,
) -> GraphIndexArtifact {
    let mut forged = graph.clone();
    let encoded = serde_json::to_string(forged.rust_evidence.as_ref().unwrap()).unwrap();
    assert!(encoded.contains(old));
    forged.rust_evidence = Some(serde_json::from_str(&encoded.replace(old, new)).unwrap());
    let checksum =
        spur_graph::rust_evidence_checksum_blake3_hex(forged.rust_evidence.as_ref().unwrap())
            .unwrap();
    for table in TABLES {
        let path = dir.join(table);
        let reader =
            ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).unwrap()).unwrap();
        let mut metadata = reader.schema().metadata().clone();
        metadata.insert("checksum_blake3".into(), checksum.clone());
        let schema = Arc::new(arrow_schema::Schema::new_with_metadata(
            reader.schema().fields().clone(),
            metadata,
        ));
        let batches = reader
            .build()
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let mut writer =
            ArrowWriter::try_new(fs::File::create(&path).unwrap(), schema.clone(), None).unwrap();
        for batch in batches {
            let columns = batch
                .columns()
                .iter()
                .map(|column| {
                    if let Some(strings) = column.as_any().downcast_ref::<StringArray>() {
                        Arc::new(StringArray::from(
                            strings
                                .iter()
                                .map(|s| s.map(|s| s.replace(old, new)))
                                .collect::<Vec<_>>(),
                        )) as ArrayRef
                    } else {
                        column.clone()
                    }
                })
                .collect();
            writer
                .write(&RecordBatch::try_new(schema.clone(), columns).unwrap())
                .unwrap();
        }
        writer.close().unwrap();
    }
    let path = dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["rust_evidence"]["checksum_blake3"] = json!(checksum);
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    forged
}

#[test]
fn d7b_rehashed_filtered_sources_and_wrong_existing_owners_are_rejected() {
    let (_root, graph) = fixture();
    let a = graph
        .file_manifests
        .iter()
        .find(|m| m.path == "a.rs")
        .unwrap();
    let b = graph
        .file_manifests
        .iter()
        .find(|m| m.path == "b.rs")
        .unwrap();
    let pair = graph
        .symbols
        .iter()
        .find(|s| s.entity_name == "Pair")
        .unwrap();
    let other = graph.symbols.iter().find(|s| s.entity_name == "b").unwrap();
    for (old, new) in [
        (&b.content_oid, &a.content_oid),
        (&pair.stable_symbol_id, &other.stable_symbol_id),
    ] {
        let (out, _) = disk(&graph);
        let forged = rehash_replacement(&graph, out.path(), old, new);
        let memory = InMemoryClient::new(Arc::new(forged.clone()));
        let cold = ParquetClient::open(out.path()).unwrap();
        let filter = file("empty.rs");
        let memory_error = memory.rust_evidence(&filter).unwrap_err();
        let disk_error = cold.rust_evidence(&filter).unwrap_err();
        let message = format!("{disk_error:#}");
        assert!(
            !message.contains("checksum mismatch") && !message.contains("metadata mismatch"),
            "{message}"
        );
        export(
            "rehashed_context_forgery",
            json!({"old":old,"new":new,"artifact":forged,"request":filter,"memory_error":format!("{memory_error:#}"),"disk_error":message}),
        );
    }
}

fn copy_packet_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_packet_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn replay_mutations(dir: &Path, graph: &GraphIndexArtifact, operations: &serde_json::Value) {
    for op in operations.as_array().into_iter().flatten() {
        let table = op["table"].as_str().unwrap_or("");
        assert!(!table.contains('/') && !table.contains('\\'));
        match op["kind"].as_str().unwrap() {
            "write" => {
                fs::write(dir.join(table), op["text"].as_str().unwrap().as_bytes()).unwrap();
            }
            "delete" => {
                fs::remove_file(dir.join(table)).unwrap();
            }
            "rehash_replace" => {
                rehash_replacement(
                    graph,
                    dir,
                    op["old"].as_str().unwrap(),
                    op["new"].as_str().unwrap(),
                );
            }
            "replace_string" => {
                let path = dir.join(table);
                let reader =
                    ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).unwrap())
                        .unwrap();
                let schema = reader.schema().clone();
                let batches = reader
                    .build()
                    .unwrap()
                    .map(Result::unwrap)
                    .collect::<Vec<_>>();
                let mut writer =
                    ArrowWriter::try_new(fs::File::create(path).unwrap(), schema.clone(), None)
                        .unwrap();
                let mut changed = 0;
                for batch in batches {
                    let column = schema.index_of(op["column"].as_str().unwrap()).unwrap();
                    let strings = batch
                        .column(column)
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .unwrap();
                    let paths = op["file_path"].as_str().map(|_| {
                        batch
                            .column(schema.index_of("file_path").unwrap())
                            .as_any()
                            .downcast_ref::<StringArray>()
                            .unwrap()
                    });
                    let values = strings
                        .iter()
                        .enumerate()
                        .map(|(i, original)| {
                            if op["file_path"]
                                .as_str()
                                .is_none_or(|p| paths.unwrap().value(i) == p)
                            {
                                changed += 1;
                                Some(op["replacement"].as_str().unwrap())
                            } else {
                                original
                            }
                        })
                        .collect::<Vec<_>>();
                    let mut columns = batch.columns().to_vec();
                    columns[column] = Arc::new(StringArray::from(values));
                    writer
                        .write(&RecordBatch::try_new(schema.clone(), columns).unwrap())
                        .unwrap();
                }
                writer.close().unwrap();
                assert!(changed > 0);
            }
            kind => panic!("unknown replay mutation {kind}"),
        }
    }
}

fn replay_observe(client: &dyn GraphQueryClient, filters: &[Filter]) -> Vec<serde_json::Value> {
    filters
        .iter()
        .map(|filter| match client.rust_evidence(filter) {
            Ok(value) => json!({"request":filter,"ok":value}),
            Err(error) => json!({"request":filter,"error":format!("{error:#}")}),
        })
        .collect()
}

// Native replay entry point. ROOT may independently supply an artifact JSON,
// existing disk directory, arbitrary original filters and disk mutations.
// Supplied cases export actual output without assuming memory/disk agreement.
#[test]
fn d7b_native_replay() {
    let request = std::env::var_os("SPUR_D7B_REPLAY_REQUEST")
        .map(|p| serde_json::from_slice::<serde_json::Value>(&fs::read(p).unwrap()).unwrap());
    let out = std::env::var_os("SPUR_D7B_PACKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("spur-d7b-native-{}", std::process::id()))
        });
    fs::create_dir_all(&out).unwrap();
    let (root, mut default_graph) = fixture();
    if request.is_none() {
        fs::write(root.path().join("invalid.rs"), [0xff, 0xfe]).unwrap();
        default_graph = build(root.path());
    }
    let graph: GraphIndexArtifact = if let Some(request) = &request {
        serde_json::from_slice(&fs::read(request["artifact"].as_str().unwrap()).unwrap()).unwrap()
    } else {
        default_graph
    };
    fs::write(
        out.join("artifact.json"),
        serde_json::to_vec_pretty(&graph).unwrap(),
    )
    .unwrap();
    let source_dir = out.join("raw-sources");
    fs::create_dir_all(&source_dir).unwrap();
    if request.is_none() {
        for entry in fs::read_dir(root.path()).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), source_dir.join(entry.file_name())).unwrap();
        }
    }
    let disk_dir = if let Some(request) = &request {
        let live = out.join("disk-live");
        copy_packet_dir(Path::new(request["disk_dir"].as_str().unwrap()), &live);
        live
    } else {
        let original = out.join("disk-original");
        write_artifact_parquet(&graph, &original, WriteOptions::default(), vec![]).unwrap();
        let live = out.join("disk-live");
        copy_packet_dir(&original, &live);
        live
    };
    let mut filters = vec![
        Filter::default(),
        file("a.rs"),
        file("empty.rs"),
        file("cfg.rs"),
        file("recovery.rs"),
        file("ffi.rs"),
        file("invalid.rs"),
        file("missing.rs"),
        file("readme.md"),
        file("note.ipynb#cell:virtual.rs"),
        file("physical.ipynb#cell:actual.rs"),
    ];
    if let Some(request) = &request {
        filters = serde_json::from_value(request["filters"].clone()).unwrap();
    } else {
        for f in &graph.rust_evidence.as_ref().unwrap().files {
            filters.push(Filter {
                source_oid: Some(f.source_oid.clone()),
                ..file(&f.file_path)
            });
        }
        let wrong = graph
            .file_manifests
            .iter()
            .find(|m| m.path == "b.rs")
            .unwrap()
            .content_oid
            .clone();
        filters.push(Filter {
            source_oid: Some(wrong.clone()),
            ..file("a.rs")
        });
        filters.push(Filter {
            source_oid: Some(wrong),
            file_path: None,
        });
    }
    fs::write(
        out.join("filters.json"),
        serde_json::to_vec_pretty(&filters).unwrap(),
    )
    .unwrap();
    if let Some(request) = &request {
        replay_mutations(&disk_dir, &graph, &request["before"]);
    }
    let memory = InMemoryClient::new(Arc::new(graph.clone()));
    let client = ParquetClient::open(&disk_dir).unwrap();
    let memory_results = replay_observe(&memory, &filters);
    let initial = replay_observe(&client, &filters);
    if request.is_none() {
        assert_eq!(memory_results, initial);
    }
    if let Some(request) = &request {
        replay_mutations(&disk_dir, &graph, &request["after"]);
    } else {
        for table in TABLES {
            fs::write(disk_dir.join(table), b"corrupt").unwrap();
        }
        fs::write(root.path().join("a.rs"), "changed current source bytes\n").unwrap();
    }
    let frozen = replay_observe(&client, &filters);
    let cold = replay_observe(&ParquetClient::open(&disk_dir).unwrap(), &filters);
    if request.is_none() {
        assert_eq!(initial, frozen);
    }
    let deleted = if request.is_none() {
        for table in TABLES {
            fs::remove_file(disk_dir.join(table)).unwrap();
        }
        let values = replay_observe(&client, &filters);
        assert_eq!(initial, values);
        Some(values)
    } else {
        None
    };
    let mut profiles = Vec::new();
    if request.is_none() {
        let mut legacy = graph.clone();
        legacy.rust_evidence = None;
        let mut declarations_unknown = graph.clone();
        declarations_unknown
            .rust_evidence
            .as_mut()
            .unwrap()
            .files
            .iter_mut()
            .find(|f| f.file_path == "a.rs")
            .unwrap()
            .declarations
            .sources[0]
            .availability = spur_graph::RustDeclarationAvailability::Unknown {
            reason: "recorded declaration uncertainty".into(),
        };
        let empty_root = tempfile::tempdir().unwrap();
        let empty = build(empty_root.path());
        let mut absent_empty = empty.clone();
        absent_empty.rust_evidence = None;
        for (name, artifact) in [
            ("legacy-known-domain", legacy),
            ("declaration-unknown", declarations_unknown),
            ("present-empty", empty),
            ("absent-empty", absent_empty),
        ] {
            let profile = out.join("profiles").join(name);
            fs::create_dir_all(&profile).unwrap();
            fs::write(
                profile.join("artifact.json"),
                serde_json::to_vec_pretty(&artifact).unwrap(),
            )
            .unwrap();
            let disk_path = profile.join("disk");
            write_artifact_parquet(&artifact, &disk_path, WriteOptions::default(), vec![]).unwrap();
            let memory = replay_observe(&InMemoryClient::new(Arc::new(artifact.clone())), &filters);
            let disk = replay_observe(&ParquetClient::open(disk_path).unwrap(), &filters);
            assert_eq!(memory, disk);
            profiles.push(json!({"name":name,"artifact":artifact,"memory":memory,"disk":disk}));
        }
    }
    let result = json!({"supplied_request":request,"artifact":graph,"filters":filters,"memory":memory_results,"initial_disk":initial,"frozen_disk":frozen,"cold_disk":cold,"deleted_disk":deleted,"profiles":profiles,"packet":out,"disk_dir":disk_dir});
    fs::write(
        out.join("actual.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    export("native_replay_packet", result);
}
