use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::builder::{ListBuilder, StringBuilder};
use arrow_array::{Array as _, ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use spur_graph::{
    load_temporal_artifact_parquet, read_artifact_header_parquet, read_artifact_parquet,
    stamp_sidecar_status, write_artifact_parquet, ArtifactStagingDir, ChangeKind, CommitArtifact,
    Confidence, EdgeEndpoint, GitPath, GraphArtifactManifest, GraphArtifactSidecarRowCounts,
    GraphArtifactSidecarStatus, GraphEdgeArtifact, GraphEdgeKind, GraphFileArtifact,
    GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphSymbolArtifact,
    GraphTombstoneEntry, NodeId, RelationKind, SnapshotKey, SymbolSnapshotArtifact,
    TemporalEdgeArtifact, WriteOptions, GRAPH_INDEX_VERSION_TEMPORAL,
};

#[test]
fn parquet_artifact_round_trips_all_tables_with_exact_node_ids() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = fixture_artifact();

    assert!(WriteOptions::default().emit_edges_by_dst);

    let dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions {
            emit_edges_by_dst: true,
        },
        Vec::new(),
    )
    .expect("write parquet artifact");

    assert_parquet_files_exist(&dir);

    let manifest = read_artifact_header_parquet(&dir).expect("read manifest");
    assert!(manifest.complete);
    assert!(manifest.edges_by_dst_present);
    assert_eq!(manifest.row_counts.nodes, 2);
    assert_eq!(manifest.row_counts.edges, 2);
    assert_eq!(manifest.row_counts.edges_by_dst, Some(2));
    assert_eq!(manifest.row_counts.edges_unresolved, 1);
    assert_eq!(manifest.row_counts.files, 3);
    assert_eq!(manifest.row_counts.file_manifests, 3);
    assert_eq!(manifest.row_counts.tombstones, 1);
    assert_eq!(manifest.parquet_writer.row_group_size, 16_384);
    assert_eq!(manifest.parquet_writer.compression, "zstd-3");

    let actual = read_artifact_parquet(&dir).expect("read parquet artifact");
    assert_artifact_eq(&actual, &artifact);
}

#[test]
fn temporal_parquet_loader_reads_only_temporal_tables_and_metadata() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = temporal_fixture_artifact();
    let dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");

    let actual = load_temporal_artifact_parquet(&dir).expect("read temporal parquet artifact");

    assert_eq!(actual.header, artifact.header);
    assert_eq!(actual.manifest_version, artifact.manifest_version);
    assert_eq!(actual.graph_content_hash, artifact.graph_content_hash);
    assert_eq!(actual.commits, artifact.commits);
    assert_eq!(actual.symbol_snapshots, artifact.symbol_snapshots);
    assert_eq!(actual.temporal_edges, artifact.temporal_edges);
    assert_eq!(actual.diagnostics, artifact.diagnostics);
    assert!(actual.file_manifests.is_empty());
    assert!(actual.files.is_empty());
    assert!(actual.file_node_ids.is_empty());
    assert!(actual.symbols.is_empty());
    assert!(actual.symbol_node_ids.is_empty());
    assert!(actual.edges.is_empty());
    assert!(actual.tombstones.is_empty());
}

#[test]
fn default_write_emits_edges_by_dst_with_edges_schema_and_dst_src_order() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = fixture_artifact_with_unsorted_resolved_edges();
    let dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let edges_path = dir.join("edges.parquet");
    let edges_by_dst_path = dir.join("edges_by_dst.parquet");

    assert!(
        edges_by_dst_path.exists(),
        "default write should emit edges_by_dst.parquet"
    );

    let edges = read_batches(&edges_path);
    let edges_by_dst = read_batches(&edges_by_dst_path);
    let resolved_edge_count = artifact
        .edges
        .iter()
        .filter(|edge| edge.target_stable_symbol_id.is_some())
        .count();

    assert_eq!(row_count(&edges_by_dst), row_count(&edges));
    assert_eq!(row_count(&edges), resolved_edge_count);
    assert_eq!(column_schema(&edges_by_dst), column_schema(&edges));

    let endpoints = read_edge_endpoints(&edges_by_dst);
    let mut sorted = endpoints.clone();
    sorted.sort_by_key(|(src_id, dst_id)| (*dst_id, *src_id));
    assert_eq!(
        endpoints, sorted,
        "edges_by_dst.parquet rows should be sorted by (dst_id, src_id)"
    );

    let manifest = read_artifact_header_parquet(&dir).expect("read manifest");
    assert!(manifest.edges_by_dst_present);
    assert_eq!(manifest.row_counts.edges_by_dst, Some(resolved_edge_count));
}

#[test]
fn sidecar_manifest_status_round_trips_after_stamp() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let dir = write_artifact_parquet(
        &fixture_artifact(),
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");

    stamp_sidecar_status(
        &dir,
        GraphArtifactSidecarStatus {
            complete: true,
            row_counts: GraphArtifactSidecarRowCounts {
                section_bodies: 7,
                code_symbols: 11,
            },
        },
    )
    .expect("stamp sidecar manifest status");

    let manifest = read_artifact_header_parquet(&dir).expect("read manifest");
    assert!(
        manifest.complete,
        "Parquet completeness should be unchanged"
    );
    assert!(manifest.sidecar_complete);
    assert_eq!(manifest.sidecar_row_counts.section_bodies, 7);
    assert_eq!(manifest.sidecar_row_counts.code_symbols, 11);

    let manifest_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("read manifest"))
            .expect("parse manifest");
    assert_eq!(manifest_json["sidecar_complete"], true);
    assert_eq!(manifest_json["sidecar_row_counts"]["section_bodies"], 7);
    assert_eq!(manifest_json["sidecar_row_counts"]["code_symbols"], 11);
}

#[test]
fn reads_symbol_snapshot_file_path_b64_with_padding_and_url_safe_alphabet() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let mut artifact = fixture_artifact();
    artifact.graph_content_hash = "b64-lenient-symbol-snapshots".to_owned();
    artifact.symbol_snapshots = vec![
        symbol_snapshot("sym-standard-padded", b"x"),
        symbol_snapshot("sym-url-safe-padded", &[0xff]),
    ];
    let dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");

    rewrite_symbol_snapshot_b64_values(&dir, &["eA==", "_w=="])
        .expect("rewrite symbol snapshot shard file_path_b64 values");
    let actual = read_artifact_parquet(&dir).expect("read parquet artifact");

    assert_eq!(actual.symbol_snapshots, artifact.symbol_snapshots);
}

#[test]
fn reads_v5_edge_tables_without_bind_method_column_as_none() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let mut artifact = fixture_artifact();
    artifact
        .edges
        .iter_mut()
        .for_each(|edge| edge.bind_method = Some("macro_body_singleton".to_owned()));
    let dir = write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");

    rewrite_manifest_schema_version(&dir, "spur-graph-schema-v5")
        .expect("rewrite manifest schema version");
    for file_name in [
        "edges.parquet",
        "edges_by_dst.parquet",
        "edges_unresolved.parquet",
    ] {
        rewrite_without_column(&dir.join(file_name), "bind_method")
            .expect("drop bind_method column");
    }

    let actual = read_artifact_parquet(&dir).expect("read v5 parquet artifact");

    assert!(actual.edges.iter().all(|edge| edge.bind_method.is_none()));
}

#[test]
fn rejects_directory_without_manifest() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let dir = write_artifact_parquet(
        &fixture_artifact(),
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    std::fs::remove_file(dir.join("manifest.json")).expect("remove manifest");

    let err = read_artifact_parquet(&dir).expect_err("missing manifest must be rejected");

    assert!(
        err.to_string().contains("manifest.json"),
        "error should mention manifest.json: {err:#}"
    );
}

#[test]
fn rejects_directory_with_incomplete_manifest() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let dir = write_artifact_parquet(
        &fixture_artifact(),
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write parquet artifact");
    let manifest_path = dir.join("manifest.json");
    let mut manifest: GraphArtifactManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    manifest.complete = false;
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("encode manifest"),
    )
    .expect("write incomplete manifest");

    let err = read_artifact_parquet(&dir).expect_err("incomplete manifest must be rejected");

    assert!(
        err.to_string().contains("complete"),
        "error should mention complete: {err:#}"
    );
}

#[test]
fn write_replaces_existing_hash_directory_before_publish() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = fixture_artifact();
    let staging = ArtifactStagingDir::new(tempdir.path(), &artifact.graph_content_hash)
        .expect("create staging dir");
    write_artifact_parquet(
        &artifact,
        staging.path(),
        WriteOptions {
            emit_edges_by_dst: true,
        },
        Vec::new(),
    )
    .expect("write parquet artifact");
    let dir = staging.commit().expect("commit parquet artifact");
    let stale = dir.join("stale-file");
    std::fs::write(&stale, b"stale").expect("write stale file");

    let staging = ArtifactStagingDir::new(tempdir.path(), &artifact.graph_content_hash)
        .expect("create replacement staging dir");
    write_artifact_parquet(
        &artifact,
        staging.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("rewrite parquet artifact");
    let rewritten = staging
        .commit()
        .expect("commit replacement parquet artifact");

    assert_eq!(rewritten, dir);
    assert!(
        !stale.exists(),
        "existing hash directory should be removed before publication"
    );
    assert!(dir.join("edges_by_dst.parquet").exists());
}

fn symbol_snapshot(stable_symbol_id: &str, file_path: &[u8]) -> SymbolSnapshotArtifact {
    SymbolSnapshotArtifact {
        key: SnapshotKey {
            stable_symbol_id: stable_symbol_id.to_owned(),
            commit: "commit-a".to_owned(),
        },
        file_path: GitPath::from_bytes(file_path.to_vec()),
        entity_name: "sample".to_owned(),
        symbol_kind: "function".to_owned(),
        enclosing_scope: None,
        byte_range: [0, 1],
        line_range: [1, 1],
        anchor_hash: format!("anchor-{stable_symbol_id}"),
        tokens: Vec::new(),
    }
}

fn rewrite_symbol_snapshot_b64_values(dir: &Path, encoded_paths: &[&str]) -> anyhow::Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("key_stable_symbol_id", DataType::Utf8, false),
        Field::new("key_commit", DataType::Utf8, false),
        Field::new("file_path_b64", DataType::Utf8, false),
        Field::new("entity_name", DataType::Utf8, false),
        Field::new("symbol_kind", DataType::Utf8, false),
        Field::new("enclosing_scope", DataType::Utf8, true),
        Field::new("byte_range_start", DataType::Int64, false),
        Field::new("byte_range_end", DataType::Int64, false),
        Field::new("line_range_start", DataType::Int64, false),
        Field::new("line_range_end", DataType::Int64, false),
        Field::new("anchor_hash", DataType::Utf8, false),
        Field::new(
            "tokens",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            true,
        ),
    ]));
    let key_stable_symbol_id = StringArray::from(vec![
        "sym-standard-padded".to_owned(),
        "sym-url-safe-padded".to_owned(),
    ]);
    let key_commit = StringArray::from(vec!["commit-a", "commit-a"]);
    let file_path_b64 = StringArray::from_iter_values(encoded_paths.iter().copied());
    let entity_name = StringArray::from(vec!["sample", "sample"]);
    let symbol_kind = StringArray::from(vec!["function", "function"]);
    let enclosing_scope = StringArray::from(vec![None::<&str>, None::<&str>]);
    let byte_range_start = Int64Array::from(vec![0, 0]);
    let byte_range_end = Int64Array::from(vec![1, 1]);
    let line_range_start = Int64Array::from(vec![1, 1]);
    let line_range_end = Int64Array::from(vec![1, 1]);
    let anchor_hash = StringArray::from(vec![
        "anchor-sym-standard-padded",
        "anchor-sym-url-safe-padded",
    ]);
    let mut token_builder = ListBuilder::new(StringBuilder::new());
    token_builder.append(true);
    token_builder.append(true);
    let tokens = token_builder.finish();
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(key_stable_symbol_id) as ArrayRef,
            Arc::new(key_commit),
            Arc::new(file_path_b64),
            Arc::new(entity_name),
            Arc::new(symbol_kind),
            Arc::new(enclosing_scope),
            Arc::new(byte_range_start),
            Arc::new(byte_range_end),
            Arc::new(line_range_start),
            Arc::new(line_range_end),
            Arc::new(anchor_hash),
            Arc::new(tokens),
        ],
    )?;
    let file = File::create(dir.join("symbol_snapshots/00000.parquet"))?;
    let mut writer = ArrowWriter::try_new(file, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn rewrite_manifest_schema_version(dir: &Path, schema_version: &str) -> anyhow::Result<()> {
    let manifest_path = dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
    manifest["schema_version"] = serde_json::Value::String(schema_version.to_owned());
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

fn rewrite_without_column(path: &Path, column_name: &str) -> anyhow::Result<()> {
    let batches = read_batches(path);
    let first_batch = batches
        .first()
        .unwrap_or_else(|| panic!("`{}` must have at least one batch", path.display()));
    let original_schema = first_batch.schema();
    let indices = original_schema
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(index, field)| (field.name() != column_name).then_some(index))
        .collect::<Vec<_>>();
    assert!(
        indices.len() < original_schema.fields().len(),
        "`{}` should contain column `{column_name}`",
        path.display()
    );
    let rewritten_schema = Arc::new(Schema::new(
        indices
            .iter()
            .map(|index| original_schema.field(*index).clone())
            .collect::<Vec<_>>(),
    ));
    let file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, Arc::clone(&rewritten_schema), None)?;
    for batch in batches {
        let columns = indices
            .iter()
            .map(|index| Arc::clone(batch.column(*index)))
            .collect::<Vec<_>>();
        let rewritten = RecordBatch::try_new(Arc::clone(&rewritten_schema), columns)?;
        writer.write(&rewritten)?;
    }
    writer.close()?;
    Ok(())
}

fn fixture_artifact_with_unsorted_resolved_edges() -> GraphIndexArtifact {
    let mut artifact = fixture_artifact();
    artifact.edges.push(GraphEdgeArtifact {
        source_stable_symbol_id: "sym-b-fn".to_owned(),
        target_stable_symbol_id: Some("sym-a-fn".to_owned()),
        target_label: Some("a_fn".to_owned()),
        import_path: None,
        receiver_text: None,
        scope_text: None,
        relation: RelationKind::Calls,
        confidence: Confidence::SyntaxExact,
        confidence_score: 0.75,
        change_kind: None,

        edge_kind: Some(GraphEdgeKind::Calls),
        bind_method: None,
    });
    artifact
}

fn fixture_artifact() -> GraphIndexArtifact {
    GraphIndexArtifact {
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: "test-manifest-version".to_owned(),
        graph_content_hash: "test-graph-content-hash".to_owned(),
        file_manifests: vec![
            GraphFileManifestEntry {
                stable_file_id: "file-a".to_owned(),
                path: "src/a.rs".to_owned(),
                content_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                node_ids: vec![NodeId(11)],
            },
            GraphFileManifestEntry {
                stable_file_id: "file-b".to_owned(),
                path: "src/b.rs".to_owned(),
                content_oid: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
                node_ids: vec![NodeId(21)],
            },
            GraphFileManifestEntry {
                stable_file_id: "file-c".to_owned(),
                path: "src/c.rs".to_owned(),
                content_oid: "cccccccccccccccccccccccccccccccccccccccc".to_owned(),
                node_ids: Vec::new(),
            },
        ],
        files: vec![
            GraphFileArtifact {
                stable_file_id: "file-a".to_owned(),
                file_path: "src/a.rs".to_owned(),
            },
            GraphFileArtifact {
                stable_file_id: "file-b".to_owned(),
                file_path: "src/b.rs".to_owned(),
            },
            GraphFileArtifact {
                stable_file_id: "file-c".to_owned(),
                file_path: "src/c.rs".to_owned(),
            },
        ],
        file_node_ids: vec![NodeId(10), NodeId(20), NodeId(30)],
        symbols: vec![
            GraphSymbolArtifact {
                stable_symbol_id: "sym-a-fn".to_owned(),
                file_path: "src/a.rs".to_owned(),
                byte_range: [10, 42],
                line_range: [2, 5],
                entity_name: "a_fn".to_owned(),
                qualified_name: "crate::a::a_fn".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-a".to_owned(),
                enclosing_scope: Some("mod a".to_owned()),
            },
            GraphSymbolArtifact {
                stable_symbol_id: "sym-b-fn".to_owned(),
                file_path: "src/b.rs".to_owned(),
                byte_range: [3, 19],
                line_range: [1, 3],
                entity_name: "b_fn".to_owned(),
                qualified_name: "crate::b::b_fn".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-b".to_owned(),
                enclosing_scope: None,
            },
        ],
        symbol_node_ids: vec![NodeId(11), NodeId(21)],
        edges: vec![
            GraphEdgeArtifact {
                source_stable_symbol_id: "file-a".to_owned(),
                target_stable_symbol_id: Some("sym-a-fn".to_owned()),
                target_label: Some("a_fn".to_owned()),
                import_path: None,
                receiver_text: None,
                scope_text: None,
                relation: RelationKind::Contains,
                confidence: Confidence::SyntaxExact,
                confidence_score: 1.0,
                change_kind: None,

                edge_kind: Some(GraphEdgeKind::ReferencesOther),
                bind_method: None,
            },
            GraphEdgeArtifact {
                source_stable_symbol_id: "sym-a-fn".to_owned(),
                target_stable_symbol_id: Some("sym-b-fn".to_owned()),
                target_label: Some("b_fn".to_owned()),
                import_path: None,
                receiver_text: Some("catalog".to_owned()),
                scope_text: Some("Catalog".to_owned()),
                relation: RelationKind::Calls,
                confidence: Confidence::SyntaxExact,
                confidence_score: 0.875,
                change_kind: None,

                edge_kind: Some(GraphEdgeKind::Calls),
                bind_method: Some("macro_body_singleton".to_owned()),
            },
            GraphEdgeArtifact {
                source_stable_symbol_id: "sym-b-fn".to_owned(),
                target_stable_symbol_id: None,
                target_label: Some("missing_fn".to_owned()),
                import_path: None,
                receiver_text: None,
                scope_text: None,
                relation: RelationKind::Calls,
                confidence: Confidence::Heuristic,
                confidence_score: f32::from_bits(0x7fc0_1234),
                change_kind: None,

                edge_kind: Some(GraphEdgeKind::CallsDyn),
                bind_method: None,
            },
        ],
        tombstones: vec![GraphTombstoneEntry {
            path: "src/removed.rs".to_owned(),
            stable_file_id: "file-removed".to_owned(),
        }],
        diagnostics: Vec::new(),

        commits: Vec::new(),

        symbol_snapshots: Vec::new(),

        temporal_edges: Vec::new(),
    }
}

fn temporal_fixture_artifact() -> GraphIndexArtifact {
    let mut artifact = fixture_artifact();
    artifact.diagnostics = vec!["parser recovered from fixture warning".to_owned()];
    artifact.commits = vec![
        CommitArtifact {
            sha: "commit-a".to_owned(),
            parents: Vec::new(),
            author_time: 1_700_000_001,
            author_name: String::new(),
            author_email: String::new(),
            summary: "initial".to_owned(),
        },
        CommitArtifact {
            sha: "commit-b".to_owned(),
            parents: vec!["commit-a".to_owned()],
            author_time: 1_700_000_002,
            author_name: String::new(),
            author_email: String::new(),
            summary: "next".to_owned(),
        },
    ];
    artifact.symbol_snapshots = vec![symbol_snapshot("sym-a-fn", b"src/a.rs")];
    artifact.temporal_edges = vec![
        TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "commit-a".to_owned(),
            },
            target: EdgeEndpoint::Snapshot {
                key: SnapshotKey {
                    stable_symbol_id: "sym-a-fn".to_owned(),
                    commit: "commit-a".to_owned(),
                },
            },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::Added),
        },
        TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "commit-b".to_owned(),
            },
            target: EdgeEndpoint::Snapshot {
                key: SnapshotKey {
                    stable_symbol_id: "sym-a-fn".to_owned(),
                    commit: "commit-b".to_owned(),
                },
            },
            relation: RelationKind::Touches,
            parent: Some("commit-a".to_owned()),
            change_kind: Some(ChangeKind::Modified),
        },
    ];
    artifact
}

fn assert_parquet_files_exist(dir: &Path) {
    for name in [
        "nodes.parquet",
        "edges.parquet",
        "edges_by_dst.parquet",
        "edges_unresolved.parquet",
        "files.parquet",
        "file_manifests.parquet",
        "tombstones.parquet",
        "manifest.json",
    ] {
        assert!(dir.join(name).exists(), "{name} should exist");
    }
}

fn read_batches(path: &Path) -> Vec<RecordBatch> {
    let file = File::open(path).unwrap_or_else(|err| panic!("open `{}`: {err}", path.display()));
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap_or_else(|err| panic!("create Arrow reader for `{}`: {err}", path.display()))
        .build()
        .unwrap_or_else(|err| panic!("build Arrow reader for `{}`: {err}", path.display()));
    reader
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|err| panic!("read Arrow batches from `{}`: {err}", path.display()))
}

fn row_count(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

fn column_schema(batches: &[RecordBatch]) -> Vec<(String, String, bool)> {
    batches
        .first()
        .expect("Parquet file should contain at least one batch")
        .schema()
        .fields()
        .iter()
        .map(|field| {
            (
                field.name().clone(),
                format!("{:?}", field.data_type()),
                field.is_nullable(),
            )
        })
        .collect()
}

fn read_edge_endpoints(batches: &[RecordBatch]) -> Vec<(i64, i64)> {
    let mut endpoints = Vec::new();
    for batch in batches {
        let src_ids = int64_column(batch, "src_id");
        let dst_ids = int64_column(batch, "dst_id");
        for row_index in 0..batch.num_rows() {
            assert!(
                !src_ids.is_null(row_index) && !dst_ids.is_null(row_index),
                "edge row {row_index} should have non-null endpoints"
            );
            endpoints.push((src_ids.value(row_index), dst_ids.value(row_index)));
        }
    }
    endpoints
}

fn int64_column<'a>(batch: &'a RecordBatch, column_name: &str) -> &'a Int64Array {
    let index = batch
        .schema()
        .index_of(column_name)
        .unwrap_or_else(|err| panic!("find column `{column_name}`: {err}"));
    batch
        .column(index)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap_or_else(|| panic!("column `{column_name}` is not Int64"))
}

fn assert_artifact_eq(actual: &GraphIndexArtifact, expected: &GraphIndexArtifact) {
    assert_eq!(actual.header, expected.header);
    assert_eq!(actual.manifest_version, expected.manifest_version);
    assert_eq!(actual.graph_content_hash, expected.graph_content_hash);
    assert_eq!(actual.file_manifests, expected.file_manifests);
    assert_eq!(actual.files, expected.files);
    assert_eq!(actual.file_node_ids, expected.file_node_ids);
    assert_eq!(actual.symbols, expected.symbols);
    assert_eq!(actual.symbol_node_ids, expected.symbol_node_ids);
    assert_eq!(actual.tombstones, expected.tombstones);
    assert_eq!(actual.diagnostics, expected.diagnostics);
    assert_eq!(actual.edges.len(), expected.edges.len());
    for (actual, expected) in actual.edges.iter().zip(&expected.edges) {
        assert_eq!(
            actual.source_stable_symbol_id,
            expected.source_stable_symbol_id
        );
        assert_eq!(
            actual.target_stable_symbol_id,
            expected.target_stable_symbol_id
        );
        assert_eq!(actual.target_label, expected.target_label);
        assert_eq!(actual.relation, expected.relation);
        assert_eq!(actual.confidence, expected.confidence);
        assert_eq!(
            actual.confidence_score.to_bits(),
            expected.confidence_score.to_bits()
        );
        assert_eq!(actual.edge_kind, expected.edge_kind);
        assert_eq!(actual.bind_method, expected.bind_method);
        assert_eq!(actual.receiver_text, expected.receiver_text);
        assert_eq!(actual.scope_text, expected.scope_text);
    }
}
