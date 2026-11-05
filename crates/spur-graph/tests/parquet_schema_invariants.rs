use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _};
use arrow_array::{Array as _, Int64Array, ListArray, RecordBatch};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use spur_graph::{
    read_artifact_header_parquet, write_artifact_parquet, Confidence, GraphEdgeArtifact,
    GraphEdgeKind, GraphFileArtifact, GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader,
    GraphSymbolArtifact, NodeId, RelationKind, WriteOptions,
};

#[test]
fn read_artifact_header_parquet_returns_counts_and_hash_under_50ms() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir().context("create tempdir")?;
    let artifact = fixture_artifact();
    assert_fixture_has_file_source_contains_edge(&artifact);
    let dir = write_fixture(&artifact, tempdir.path())?;

    fs::remove_file(dir.join("edges.parquet")).context("remove edges.parquet")?;

    let started = Instant::now();
    let manifest = read_artifact_header_parquet(&dir).context("read artifact header")?;
    let elapsed = started.elapsed();

    assert!(manifest.complete);
    assert!(manifest.edges_by_dst_present);
    assert_eq!(manifest.graph_content_hash, artifact.graph_content_hash);
    assert_eq!(manifest.row_counts.nodes, 2);
    assert_eq!(manifest.row_counts.edges, 2);
    assert_eq!(manifest.row_counts.edges_by_dst, Some(2));
    assert_eq!(manifest.row_counts.edges_unresolved, 1);
    assert_eq!(manifest.row_counts.files, 2);
    assert_eq!(manifest.row_counts.file_manifests, 2);
    assert_eq!(manifest.row_counts.tombstones, 0);
    assert!(
        elapsed < Duration::from_millis(50),
        "header fast path took {elapsed:?}"
    );

    Ok(())
}

#[test]
fn manifest_defaults_missing_sidecar_fields() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir().context("create tempdir")?;
    let artifact = fixture_artifact();
    let dir = write_fixture(&artifact, tempdir.path())?;
    let manifest_path = dir.join("manifest.json");
    let mut manifest_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    let manifest_object = manifest_json
        .as_object_mut()
        .expect("manifest JSON should be an object");
    manifest_object.remove("sidecar_complete");
    manifest_object.remove("sidecar_row_counts");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest_json)?)?;

    let manifest = read_artifact_header_parquet(&dir).context("read artifact header")?;

    assert!(!manifest.sidecar_complete);
    assert_eq!(manifest.sidecar_row_counts.section_bodies, 0);
    assert_eq!(manifest.sidecar_row_counts.code_symbols, 0);
    Ok(())
}

#[test]
fn family_2_6_endpoint_namespace_consistency() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir().context("create tempdir")?;
    let artifact = fixture_artifact();
    assert_fixture_has_file_source_contains_edge(&artifact);
    let dir = write_fixture(&artifact, tempdir.path())?;

    let symbol_node_ids: HashSet<_> = read_i64_column(&dir.join("nodes.parquet"), "node_id")?
        .into_iter()
        .collect();
    let file_node_ids: HashSet<_> = read_i64_column(&dir.join("files.parquet"), "node_id")?
        .into_iter()
        .collect();
    let endpoint_node_ids: HashSet<_> = symbol_node_ids.union(&file_node_ids).copied().collect();

    let edge_endpoints = read_edge_endpoints(&dir.join("edges.parquet"))?;
    assert!(
        !edge_endpoints.is_empty(),
        "fixture must include resolved edges"
    );
    let mut saw_file_source_edge = false;
    for (row_index, (src_id, dst_id)) in edge_endpoints.iter().copied().enumerate() {
        assert!(
            endpoint_node_ids.contains(&src_id),
            "edges.parquet row {row_index} src_id {src_id} is not in nodes/files endpoint namespace"
        );
        assert!(
            endpoint_node_ids.contains(&dst_id),
            "edges.parquet row {row_index} dst_id {dst_id} is not in nodes/files endpoint namespace"
        );
        saw_file_source_edge |= file_node_ids.contains(&src_id);
    }
    assert!(
        saw_file_source_edge,
        "fixture must include at least one resolved edge with a file source"
    );

    let file_manifest_node_ids = read_i64_lists(&dir.join("file_manifests.parquet"), "node_ids")?;
    assert!(
        file_manifest_node_ids.iter().any(|ids| !ids.is_empty()),
        "fixture must include file_manifest node_ids"
    );
    for (row_index, node_ids) in file_manifest_node_ids.iter().enumerate() {
        for node_id in node_ids {
            assert!(
                symbol_node_ids.contains(node_id),
                "file_manifests.parquet row {row_index} node_id {node_id} is not in nodes.node_id"
            );
        }
    }

    Ok(())
}

#[test]
fn edges_by_dst_columns_match_edges_columns() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir().context("create tempdir")?;
    let artifact = fixture_artifact();
    let dir = write_fixture(&artifact, tempdir.path())?;

    assert_eq!(
        column_schema(&dir.join("edges_by_dst.parquet"))?,
        column_schema(&dir.join("edges.parquet"))?
    );

    Ok(())
}

fn write_fixture(artifact: &GraphIndexArtifact, base_dir: &Path) -> anyhow::Result<PathBuf> {
    write_artifact_parquet(artifact, base_dir, WriteOptions::default(), Vec::new())
        .context("write parquet artifact")
}

fn column_schema(path: &Path) -> anyhow::Result<Vec<(String, String, bool)>> {
    let batches = read_batches(path)?;
    let batch = batches
        .first()
        .ok_or_else(|| anyhow!("`{}` has no record batches", path.display()))?;
    Ok(batch
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
        .collect())
}

fn read_i64_column(path: &Path, column_name: &str) -> anyhow::Result<Vec<i64>> {
    let mut values = Vec::new();
    for batch in read_batches(path)? {
        let column = int64_column(&batch, column_name)?;
        for row_index in 0..column.len() {
            if column.is_null(row_index) {
                bail!("`{}` row {row_index} is null", path.display());
            }
            values.push(column.value(row_index));
        }
    }
    Ok(values)
}

fn read_edge_endpoints(path: &Path) -> anyhow::Result<Vec<(i64, i64)>> {
    let mut endpoints = Vec::new();
    for batch in read_batches(path)? {
        let src_ids = int64_column(&batch, "src_id")?;
        let dst_ids = int64_column(&batch, "dst_id")?;
        for row_index in 0..batch.num_rows() {
            if src_ids.is_null(row_index) || dst_ids.is_null(row_index) {
                bail!(
                    "`{}` edge row {row_index} has a null endpoint",
                    path.display()
                );
            }
            endpoints.push((src_ids.value(row_index), dst_ids.value(row_index)));
        }
    }
    Ok(endpoints)
}

fn read_i64_lists(path: &Path, column_name: &str) -> anyhow::Result<Vec<Vec<i64>>> {
    let mut rows = Vec::new();
    for batch in read_batches(path)? {
        let lists = list_column(&batch, column_name)?;
        for row_index in 0..lists.len() {
            if lists.is_null(row_index) {
                bail!("`{}` row {row_index} list is null", path.display());
            }
            let values = lists.value(row_index);
            let values = values
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| {
                    anyhow!(
                        "`{}` column `{column_name}` is not List<Int64>",
                        path.display()
                    )
                })?;
            let mut row = Vec::with_capacity(values.len());
            for value_index in 0..values.len() {
                if values.is_null(value_index) {
                    bail!(
                        "`{}` row {row_index} list value {value_index} is null",
                        path.display()
                    );
                }
                row.push(values.value(value_index));
            }
            rows.push(row);
        }
    }
    Ok(rows)
}

fn read_batches(path: &Path) -> anyhow::Result<Vec<RecordBatch>> {
    let file = File::open(path).with_context(|| format!("open `{}`", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("create Arrow reader for `{}`", path.display()))?
        .build()
        .with_context(|| format!("build Arrow reader for `{}`", path.display()))?;
    reader
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("read Arrow batches from `{}`", path.display()))
}

fn int64_column<'a>(batch: &'a RecordBatch, column_name: &str) -> anyhow::Result<&'a Int64Array> {
    let index = batch
        .schema()
        .index_of(column_name)
        .with_context(|| format!("find column `{column_name}`"))?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| anyhow!("column `{column_name}` is not Int64"))
}

fn list_column<'a>(batch: &'a RecordBatch, column_name: &str) -> anyhow::Result<&'a ListArray> {
    let index = batch
        .schema()
        .index_of(column_name)
        .with_context(|| format!("find column `{column_name}`"))?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<ListArray>()
        .ok_or_else(|| anyhow!("column `{column_name}` is not List"))
}

fn assert_fixture_has_file_source_contains_edge(artifact: &GraphIndexArtifact) {
    let file_ids: HashSet<_> = artifact
        .files
        .iter()
        .map(|file| file.stable_file_id.as_str())
        .collect();
    assert!(
        artifact.edges.iter().any(|edge| {
            edge.relation == RelationKind::Contains
                && edge.target_stable_symbol_id.is_some()
                && file_ids.contains(edge.source_stable_symbol_id.as_str())
        }),
        "fixture must include at least one Contains edge with a file source"
    );
}

fn fixture_artifact() -> GraphIndexArtifact {
    GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: "spur-graph-phase2".to_owned(),
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
        ],
        file_node_ids: vec![NodeId(10), NodeId(20)],
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
                receiver_text: None,
                scope_text: None,
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
                confidence_score: 0.5,
                change_kind: None,

                edge_kind: Some(GraphEdgeKind::CallsDyn),
                bind_method: None,
            },
        ],
        tombstones: Vec::new(),
        diagnostics: Vec::new(),

        commits: Vec::new(),

        symbol_snapshots: Vec::new(),

        temporal_edges: Vec::new(),
    }
}
