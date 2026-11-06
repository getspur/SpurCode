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

fn observe(client: &dyn GraphQueryClient, filter: &Filter) -> serde_json::Value {
    match client.rust_evidence(filter) {
        Ok(value) => json!({"ok":value}),
        Err(error) => json!({"error":format!("{error:#}")}),
    }
}

#[test]
fn zero_source_and_stale_oid_have_memory_disk_parity() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("empty.rs"), "pub struct Empty;\n").unwrap();
    let graph =
        artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(&graph, out.path(), WriteOptions::default(), vec![]).unwrap();
    let memory = InMemoryClient::new(Arc::new(graph));
    let disk = ParquetClient::open(out.path()).unwrap();
    let filter = Filter {
        file_path: Some("empty.rs".into()),
        source_oid: None,
    };
    let result = memory.rust_evidence(&filter).unwrap();
    assert_eq!(result.availability, Availability::Complete);
    assert_eq!(result.files.len(), 1);
    assert_eq!(disk.rust_evidence(&filter).unwrap(), result);
    let stale = Filter {
        source_oid: Some("0".repeat(40)),
        ..filter
    };
    assert!(memory.rust_evidence(&stale).is_err());
    assert!(disk.rust_evidence(&stale).is_err());
}

#[test]
fn ordered_whole_files_origins_unknowns_and_forwarding_match() {
    let (_root, graph) = fixture();
    export("original_artifact", json!(graph));
    let (_out, disk) = disk(&graph);
    let memory = InMemoryClient::new(Arc::new(graph.clone()));
    let mut filters = vec![
        Filter::default(),
        file("a.rs"),
        file("empty.rs"),
        file("cfg.rs"),
        file("recovery.rs"),
        file("ffi.rs"),
        file("missing.rs"),
        file("readme.md"),
        file("note.ipynb#cell:virtual.rs"),
        file("physical.ipynb#cell:actual.rs"),
    ];
    for source in &graph.rust_evidence.as_ref().unwrap().files {
        filters.push(Filter {
            source_oid: Some(source.source_oid.clone()),
            ..file(&source.file_path)
        });
    }
    for filter in filters {
        let result = memory.rust_evidence(&filter).unwrap();
        let on_disk = disk.rust_evidence(&filter).unwrap();
        export(
            "query",
            json!({"request":filter,"memory":result,"disk":on_disk}),
        );
        assert_eq!(on_disk, result);
        let reference: &dyn GraphQueryClient = &memory;
        assert_eq!(
            GraphQueryClient::rust_evidence(&reference, &filter).unwrap(),
            result
        );
        assert!(result.source_domain.is_some());
        let expected: Vec<_> = graph
            .rust_evidence
            .as_ref()
            .unwrap()
            .files
            .iter()
            .filter(|f| filter.file_path.as_ref().is_none_or(|p| p == &f.file_path))
            .cloned()
            .collect();
        assert_eq!(result.files, expected);
        let origins: Vec<_> = graph
            .rust_evidence
            .as_ref()
            .unwrap()
            .origins
            .iter()
            .filter(|o| {
                expected
                    .iter()
                    .any(|f| f.origin_graph_content_hash == o.graph_content_hash)
            })
            .cloned()
            .collect();
        assert_eq!(
            result.origins, origins,
            "retain whole referenced original inventories"
        );
    }
    for path in ["cfg.rs", "recovery.rs"] {
        let result = memory.rust_evidence(&file(path)).unwrap();
        assert_eq!(result.availability, Availability::Partial);
        assert!(!result.files[0].declarations.rows.is_empty());
    }
    assert_eq!(
        memory
            .rust_evidence(&file("missing.rs"))
            .unwrap()
            .availability,
        Availability::Unavailable
    );
    for path in [
        "readme.md",
        "note.ipynb#cell:virtual.rs",
        "external://pkg/x.rs",
    ] {
        assert_eq!(
            memory.rust_evidence(&file(path)).unwrap().availability,
            Availability::UnsupportedSource
        );
    }
    assert_ne!(
        memory
            .rust_evidence(&file("physical.ipynb#cell:actual.rs"))
            .unwrap()
            .availability,
        Availability::UnsupportedSource
    );
    let wrong = graph
        .file_manifests
        .iter()
        .find(|m| m.path == "b.rs")
        .unwrap()
        .content_oid
        .clone();
    let filter = Filter {
        source_oid: Some(wrong),
        ..file("a.rs")
    };
    export(
        "wrong_existing_oid",
        json!({"request":filter,"memory":observe(&memory,&filter),"disk":observe(&disk,&filter)}),
    );
    assert!(memory.rust_evidence(&filter).is_err());
    assert!(disk.rust_evidence(&filter).is_err());
}

#[test]
fn absent_carriers_keep_known_domains_and_validate_original_filters() {
    let (_root, mut graph) = fixture();
    graph.rust_evidence = None;
    export("legacy_artifact", json!(graph));
    let (_out, disk) = disk(&graph);
    let memory = InMemoryClient::new(Arc::new(graph.clone()));
    for filter in [
        Filter::default(),
        file("a.rs"),
        file("missing.rs"),
        file("readme.md"),
    ] {
        let result = memory.rust_evidence(&filter).unwrap();
        let on_disk = disk.rust_evidence(&filter).unwrap();
        export(
            "legacy_query",
            json!({"request":filter,"memory":result,"disk":on_disk}),
        );
        assert_eq!(on_disk, result);
        assert!(result.files.is_empty() && result.origins.is_empty());
        assert!(result.source_domain.is_some());
        assert!(result
            .source_domain
            .unwrap()
            .iter()
            .all(|s| s.availability == Availability::Unavailable));
        assert_eq!(
            result.availability,
            if filter.file_path.as_deref() == Some("readme.md") {
                Availability::UnsupportedSource
            } else {
                Availability::Unavailable
            }
        );
    }
    let oid = graph
        .file_manifests
        .iter()
        .find(|m| m.path == "a.rs")
        .unwrap()
        .content_oid
        .clone();
    for filter in [
        Filter {
            file_path: None,
            source_oid: Some(oid),
        },
        Filter {
            source_oid: Some("0".repeat(40)),
            ..file("a.rs")
        },
    ] {
        export(
            "legacy_invalid",
            json!({"request":filter,"memory":observe(&memory,&filter),"disk":observe(&disk,&filter)}),
        );
        assert!(memory.rust_evidence(&filter).is_err());
        assert!(disk.rust_evidence(&filter).is_err());
    }
}

#[test]
fn empty_present_and_absent_are_distinct() {
    let root = tempfile::tempdir().unwrap();
    let mut graph = build(root.path());
    for present in [true, false] {
        if !present {
            graph.rust_evidence = None;
        }
        let (_out, disk) = disk(&graph);
        let memory = InMemoryClient::new(Arc::new(graph.clone()));
        let result = memory.rust_evidence(&Filter::default()).unwrap();
        assert_eq!(disk.rust_evidence(&Filter::default()).unwrap(), result);
        assert_eq!(result.source_domain, Some(vec![]));
        assert_eq!(
            result.availability,
            if present {
                Availability::Complete
            } else {
                Availability::Unavailable
            }
        );
        export("empty_domain", json!({"present":present,"result":result}));
        let shape = Filter {
            file_path: None,
            source_oid: Some("0".repeat(40)),
        };
        assert!(memory.rust_evidence(&shape).is_err());
        assert!(disk.rust_evidence(&shape).is_err());
    }
}

fn rewrite_string(path: &Path, column: &str, select_path: Option<&str>, replacement: &str) {
    let batches: Vec<_> = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).unwrap())
        .unwrap()
        .build()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let schema = batches[0].schema();
    let mut rewritten = Vec::new();
    let mut changed = 0;
    for batch in batches {
        let target = schema.index_of(column).unwrap();
        let original = batch
            .column(target)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let paths = select_path.map(|_| {
            batch
                .column(schema.index_of("file_path").unwrap())
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
        });
        let values: Vec<_> = (0..batch.num_rows())
            .map(|i| {
                if select_path.is_none_or(|p| paths.unwrap().value(i) == p) {
                    changed += 1;
                    Some(replacement)
                } else if original.is_null(i) {
                    None
                } else {
                    Some(original.value(i))
                }
            })
            .collect();
        let mut columns = batch.columns().to_vec();
        columns[target] = Arc::new(StringArray::from(values)) as ArrayRef;
        rewritten.push(RecordBatch::try_new(schema.clone(), columns).unwrap());
    }
    assert!(changed > 0);
    let mut writer = ArrowWriter::try_new(fs::File::create(path).unwrap(), schema, None).unwrap();
    for batch in rewritten {
        writer.write(&batch).unwrap();
    }
    writer.close().unwrap();
}

#[test]
fn full_validation_rejects_filtered_out_corruption_and_actual_owner_changes() {
    let (_root, graph) = fixture();
    for (table, column, path, replacement) in [
        (
            "rust_evidence_sources.parquet",
            "source_oid",
            Some("b.rs"),
            "wrong-existing-oid",
        ),
        (
            "rust_evidence_bodies.parquet",
            "body_id",
            Some("b.rs"),
            "forged-body",
        ),
        ("rust_evidence_origins.parquet", "sources_json", None, "[]"),
    ] {
        let (out, _) = disk(&graph);
        rewrite_string(&out.path().join(table), column, path, replacement);
        let cold = ParquetClient::open(out.path()).unwrap();
        let outcome = observe(&cold, &file("empty.rs"));
        export(
            "filtered_corruption",
            json!({"table":table,"column":column,"path":path,"replacement":replacement,"request":file("empty.rs"),"outcome":outcome}),
        );
        assert!(outcome.get("error").is_some());
    }
    let mut forged = graph.clone();
    forged
        .rust_evidence
        .as_mut()
        .unwrap()
        .files
        .iter_mut()
        .find(|f| f.file_path == "b.rs")
        .unwrap()
        .source_oid = "0".repeat(40);
    assert!(InMemoryClient::new(Arc::new(forged))
        .rust_evidence(&file("empty.rs"))
        .is_err());
    let mut forged = graph.clone();
    let owner = forged
        .symbols
        .iter_mut()
        .find(|s| s.entity_name == "Pair")
        .unwrap();
    owner.symbol_kind = "function".into();
    let memory = InMemoryClient::new(Arc::new(forged));
    assert!(memory.rust_evidence(&file("empty.rs")).is_err());
    let (out, _) = disk(&graph);
    rewrite_string(
        &out.path().join("nodes.parquet"),
        "entity_name",
        Some("a.rs"),
        "ForgedOwner",
    );
    let cold = ParquetClient::open(out.path()).unwrap();
    assert!(cold.rust_evidence(&file("empty.rs")).is_err());
}

#[test]
fn frozen_tables_survive_replacement_deletion_and_source_edits_but_cold_reads_fail() {
    let (root, graph) = fixture();
    let (out, disk) = disk(&graph);
    let before = disk.rust_evidence(&Filter::default()).unwrap();
    for table in TABLES {
        fs::write(out.path().join(table), b"corrupt").unwrap();
    }
    fs::write(root.path().join("a.rs"), "different source bytes\n").unwrap();
    let replaced = disk.rust_evidence(&Filter::default()).unwrap();
    assert_eq!(replaced, before);
    let cold = ParquetClient::open(out.path()).unwrap();
    let cold_outcome = observe(&cold, &file("empty.rs"));
    assert!(cold_outcome.get("error").is_some());
    for table in TABLES {
        fs::remove_file(out.path().join(table)).unwrap();
    }
    let deleted = disk.rust_evidence(&Filter::default()).unwrap();
    assert_eq!(deleted, before);
    export(
        "immutable_lifecycle",
        json!({"before":before,"replaced":replaced,"deleted":deleted,"cold":cold_outcome}),
    );
}

#[test]
fn absent_metadata_rejects_orphan_evidence_tables() {
    let (_root, mut graph) = fixture();
    graph.rust_evidence = None;
    let (out, disk) = disk(&graph);
    fs::write(out.path().join(TABLES[2]), b"orphan").unwrap();
    assert!(disk.rust_evidence(&file("missing.rs")).is_err());
}

// The default method must validate the original shape even when coverage is unknown.
struct Custom;
impl GraphQueryClient for Custom {
    fn search_symbols(
        &self,
        _: &spur_graph::SearchOptions,
    ) -> anyhow::Result<spur_graph::SearchResult> {
        unimplemented!()
    }
    fn find_caller_edges(&self, _: &str) -> Vec<spur_graph::OwnedCallerRecord> {
        unimplemented!()
    }
    fn find_callee_edges(&self, _: &str) -> Vec<spur_graph::OwnedCalleeRecord> {
        unimplemented!()
    }
    fn resolve_selector(&self, _: &str) -> anyhow::Result<spur_graph::SelectorResolution> {
        unimplemented!()
    }
    fn symbol_by_id(&self, _: &str) -> anyhow::Result<Option<spur_graph::GraphSymbolArtifact>> {
        unimplemented!()
    }
    fn symbols_by_file(&self, _: &str) -> anyhow::Result<Vec<spur_graph::GraphSymbolArtifact>> {
        unimplemented!()
    }
    fn symbols_by_path_name(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Vec<spur_graph::GraphSymbolArtifact>> {
        unimplemented!()
    }
    fn file_manifest_by_path(
        &self,
        _: &str,
    ) -> anyhow::Result<Option<spur_graph::GraphFileManifestEntry>> {
        unimplemented!()
    }
    fn file_exists(&self, _: &str) -> anyhow::Result<bool> {
        unimplemented!()
    }
    fn temporal_index(&self) -> Arc<spur_graph::temporal::TemporalIndex> {
        unimplemented!()
    }
}

#[test]
fn custom_default_is_unknown_domain_unavailable_and_shape_checked() {
    let result: RustEvidenceQueryResult = Custom.rust_evidence(&Filter::default()).unwrap();
    assert_eq!(result.availability, Availability::Unavailable);
    assert_eq!(result.source_domain, None);
    assert!(result.files.is_empty() && result.origins.is_empty());
    assert!(Custom
        .rust_evidence(&Filter {
            file_path: None,
            source_oid: Some("0".repeat(40))
        })
        .is_err());
    export("custom_default", json!(result));
}
