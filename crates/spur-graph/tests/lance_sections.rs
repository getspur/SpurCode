use std::fs;
use std::path::Path;

use arrow_array::{
    Array as _, FixedSizeListArray, LargeStringArray, RecordBatch, StringArray, UInt32Array,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use spur_graph::store::lance_sections::{
    write_sections_dataset, write_sections_dataset_best_effort_with_options,
    write_sections_dataset_skipping_embeddings, SectionEmbeddingOptions, CODE_SYMBOLS_PARQUET,
    SECTIONS_PARQUET,
};
use spur_graph::{
    artifact_from_facts, build_facts, GraphFileArtifact, GraphFileManifestEntry,
    GraphIndexArtifact, GraphIndexHeader, GraphSymbolArtifact,
};

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn read_sidecar_parquet(path: &Path) -> Vec<RecordBatch> {
    let file = fs::File::open(path).expect("open parquet");
    ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("parquet builder")
        .build()
        .expect("parquet reader")
        .collect::<Result<Vec<_>, _>>()
        .expect("parquet batches")
}

fn parquet_row_count(path: &Path) -> usize {
    read_sidecar_parquet(path)
        .iter()
        .map(|batch| batch.num_rows())
        .sum()
}

fn non_null_vector_count(path: &Path) -> usize {
    let mut count = 0usize;
    for batch in read_sidecar_parquet(path) {
        let vectors = batch
            .column_by_name("vector")
            .expect("vector column")
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .expect("vector list");
        for index in 0..batch.num_rows() {
            if !vectors.is_null(index) {
                count += 1;
            }
        }
    }
    count
}

fn string_column_values(batches: &[RecordBatch], name: &str) -> Vec<String> {
    let mut values = Vec::new();
    for batch in batches {
        let column = batch.column_by_name(name).expect("column");
        if let Some(array) = column.as_any().downcast_ref::<StringArray>() {
            values.extend((0..array.len()).map(|index| array.value(index).to_owned()));
        } else if let Some(array) = column.as_any().downcast_ref::<LargeStringArray>() {
            values.extend((0..array.len()).map(|index| array.value(index).to_owned()));
        } else {
            panic!("{name} was not a string column");
        }
    }
    values
}

#[tokio::test]
async fn lance_sections_writes_markdown_sections_dataset_with_body_text() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("repo");
    fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    fs::write(
        root.join("docs/guide.md"),
        "# Guide\n\nIntro body.\n\n## Install\n\nInstall body line.\n\n## Use\n\nUse body line.\n",
    )
    .expect("write guide");
    fs::write(
        root.join("docs/notes.md"),
        "plain note without headings\nsecond line\n",
    )
    .expect("write notes");
    fs::write(root.join("docs/other.md"), "# Other\n\nOther body.\n").expect("write other");

    let facts = build_facts(&root, None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, &root).expect("artifact");
    let out_dir = tempdir.path().join("artifact");

    write_sections_dataset(&artifact, &root, &out_dir).expect("write sections sidecar");

    let parquet_path = out_dir.join(SECTIONS_PARQUET);
    assert!(parquet_path.is_file(), "sections.parquet should exist");
    assert!(
        !out_dir.join("sections.lancedb").exists(),
        "lance sidecar must not be written"
    );
    assert_eq!(parquet_row_count(&parquet_path), 5);

    let batches = read_sidecar_parquet(&parquet_path);
    let names = string_column_values(&batches, "qualified_name");
    let bodies = string_column_values(&batches, "body_text");
    let install = names
        .iter()
        .zip(bodies.iter())
        .find(|(name, _)| *name == "Guide::Install")
        .expect("install section");
    assert!(
        install.1.contains("Install body line."),
        "section body_text should include the widened section body: {:?}",
        install.1
    );
    let child_counts = batches[0]
        .column_by_name("child_count")
        .expect("child_count column")
        .as_any()
        .downcast_ref::<UInt32Array>()
        .expect("child_count u32");
    let install_index = names
        .iter()
        .position(|name| name == "Guide::Install")
        .expect("install idx");
    let mut offset = 0usize;
    let mut child = None;
    for batch in &batches {
        let counts = batch
            .column_by_name("child_count")
            .expect("child_count column")
            .as_any()
            .downcast_ref::<UInt32Array>()
            .expect("child_count u32");
        if install_index < offset + batch.num_rows() {
            child = Some(counts.value(install_index - offset));
            break;
        }
        offset += batch.num_rows();
    }
    let _ = child_counts;
    assert_eq!(child, Some(0));
}

#[tokio::test]
async fn lance_sections_skip_section_embeddings_writes_null_vectors() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("repo");
    fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    fs::write(
        root.join("docs/guide.md"),
        "# Guide\n\n## Install\n\nInstall body.\n",
    )
    .expect("write guide");

    let facts = build_facts(&root, None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, &root).expect("artifact");
    let out_dir = tempdir.path().join("artifact");

    write_sections_dataset_best_effort_with_options(
        &artifact,
        &root,
        &out_dir,
        SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: false,
            batch_size: 64,
        },
    );

    let parquet_path = out_dir.join(SECTIONS_PARQUET);
    assert_eq!(non_null_vector_count(&parquet_path), 0);
}

#[tokio::test]
async fn lance_sections_skipping_embeddings_api_writes_null_vectors_and_fts_hits() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("repo");
    fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    fs::write(
        root.join("docs/guide.md"),
        "# Guide\n\n## Install\n\nInstall body with overlayneedle.\n",
    )
    .expect("write guide");

    let facts = build_facts(&root, None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, &root).expect("artifact");
    let out_dir = tempdir.path().join("artifact");

    write_sections_dataset_skipping_embeddings(&artifact, &root, &out_dir)
        .expect("write sections sidecar");

    let parquet_path = out_dir.join(SECTIONS_PARQUET);
    assert_eq!(non_null_vector_count(&parquet_path), 0);

    let bodies = string_column_values(&read_sidecar_parquet(&parquet_path), "body_text");
    assert!(
        bodies.iter().any(|body| body.contains("overlayneedle")),
        "section bodies should include the overlay needle"
    );
}

#[tokio::test]
async fn lance_sections_streams_small_write_batches_without_vectors() {
    let _write_batch = EnvGuard::set("SPUR_GRAPH_SECTION_WRITE_BATCH_SIZE", "2");
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("repo");
    fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    fs::write(
        root.join("docs/guide.md"),
        "# Guide\n\n## One\n\nBody one.\n\n## Two\n\nBody two.\n\n## Three\n\nBody three.\n\n## Four\n\nBody four.\n\n## Five\n\nBody five.\n",
    )
    .expect("write guide");

    let facts = build_facts(&root, None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, &root).expect("artifact");
    let section_symbol_count = artifact
        .symbols
        .iter()
        .filter(|symbol| symbol.symbol_kind == "section")
        .count();
    assert!(
        section_symbol_count > 2,
        "test fixture should cross the configured write batch boundary"
    );
    let out_dir = tempdir.path().join("artifact");

    write_sections_dataset_best_effort_with_options(
        &artifact,
        &root,
        &out_dir,
        SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: false,
            batch_size: 64,
        },
    );

    let parquet_path = out_dir.join(SECTIONS_PARQUET);
    assert_eq!(parquet_row_count(&parquet_path), section_symbol_count);
    assert_eq!(non_null_vector_count(&parquet_path), 0);
}

#[tokio::test]
async fn lance_sections_refreshes_existing_fts_index_after_large_append() {
    let _skip_embeddings = EnvGuard::set("SPUR_GRAPH_SKIP_SECTION_EMBEDDINGS", "1");
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("repo");
    fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    fs::write(root.join("docs/base.md"), "# Base\n\nBase body.\n").expect("write base");

    let initial_facts = build_facts(&root, None).expect("build initial facts").0;
    let initial_artifact = artifact_from_facts(&initial_facts, &root).expect("initial artifact");
    let out_dir = tempdir.path().join("artifact");
    write_sections_dataset(&initial_artifact, &root, &out_dir)
        .expect("write initial sections sidecar");

    for index in 0..50 {
        fs::write(
            root.join("docs").join(format!("topic-{index:02}.md")),
            format!("# Topic {index:02}\n\nUnique appended body {index:02}.\n"),
        )
        .expect("write appended topic");
    }
    let updated_facts = build_facts(&root, None).expect("build updated facts").0;
    let updated_artifact = artifact_from_facts(&updated_facts, &root).expect("updated artifact");
    write_sections_dataset(&updated_artifact, &root, &out_dir)
        .expect("append updated sections sidecar");

    let parquet_path = out_dir.join(SECTIONS_PARQUET);
    assert_eq!(parquet_row_count(&parquet_path), 51);
    assert!(!out_dir.join("sections.lancedb").exists());
}

#[tokio::test]
async fn lance_sections_skip_code_symbol_embeddings_writes_code_symbol_rows_without_vectors() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    let source = concat!(
        "/// Parses the request payload into a normalized command shape.\n",
        "/// Keeps enough context for downstream handlers to preserve provenance.\n",
        "fn parse_request() {}\n",
        "\n",
        "fn x() {}\n",
        "\n",
        "struct CommandEnvelope;\n",
    );
    fs::write(root.join("src/lib.rs"), source).expect("write source");

    let facts = build_facts(&root, None).expect("build facts").0;
    let artifact = artifact_from_facts(&facts, &root).expect("artifact");
    let out_dir = tempdir.path().join("artifact");

    write_sections_dataset_best_effort_with_options(
        &artifact,
        &root,
        &out_dir,
        SectionEmbeddingOptions {
            skip_section_embeddings: false,
            skip_code_symbol_embeddings: true,
            batch_size: 64,
        },
    );

    let parquet_path = out_dir.join(CODE_SYMBOLS_PARQUET);
    assert!(parquet_path.is_file(), "code_symbols.parquet should exist");
    assert!(!out_dir.join("code_symbols.lance").exists());
    assert_eq!(parquet_row_count(&parquet_path), 2);
    assert_eq!(non_null_vector_count(&parquet_path), 0);

    let batches = read_sidecar_parquet(&parquet_path);

    let mut rows = Vec::new();
    for batch in batches {
        let entity_names = batch
            .column_by_name("entity_name")
            .expect("entity_name column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("entity_name string");
        let symbol_kinds = batch
            .column_by_name("symbol_kind")
            .expect("symbol_kind column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("symbol_kind string");
        let embed_texts = batch
            .column_by_name("embed_text")
            .expect("embed_text column")
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .expect("embed_text large string");
        for index in 0..batch.num_rows() {
            rows.push((
                entity_names.value(index).to_owned(),
                symbol_kinds.value(index).to_owned(),
                embed_texts.value(index).to_owned(),
            ));
        }
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "CommandEnvelope");
    assert_eq!(rows[0].1, "struct");
    assert_eq!(rows[0].2, "CommandEnvelope CommandEnvelope struct");
    assert_eq!(rows[1].0, "parse_request");
    assert_eq!(rows[1].1, "function");
    assert!(rows[1].2.starts_with("parse_request parse_request "));
    assert!(rows[1].2.contains("normalized command shape"));
    assert!(!rows.iter().any(|(entity_name, _, _)| entity_name == "x"));
}

#[tokio::test]
async fn lance_sections_skips_non_utf8_markdown_files() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().join("repo");
    fs::create_dir_all(root.join("docs")).expect("mkdir docs");

    let good_source = "# Good\n\nValid body.\n";
    fs::write(root.join("docs/good.md"), good_source).expect("write good");
    fs::write(root.join("docs/bad.md"), b"# Bad\n\nInvalid \xFE body.\n").expect("write bad");

    let artifact = GraphIndexArtifact {
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: "test".to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: "test".to_owned(),
        graph_content_hash: "test".to_owned(),
        file_manifests: vec![
            GraphFileManifestEntry {
                stable_file_id: "file-good".to_owned(),
                path: "docs/good.md".to_owned(),
                content_oid: "good".to_owned(),
                node_ids: Vec::new(),
            },
            GraphFileManifestEntry {
                stable_file_id: "file-bad".to_owned(),
                path: "docs/bad.md".to_owned(),
                content_oid: "bad".to_owned(),
                node_ids: Vec::new(),
            },
        ],
        files: vec![
            GraphFileArtifact {
                stable_file_id: "file-good".to_owned(),
                file_path: "docs/good.md".to_owned(),
            },
            GraphFileArtifact {
                stable_file_id: "file-bad".to_owned(),
                file_path: "docs/bad.md".to_owned(),
            },
        ],
        file_node_ids: Vec::new(),
        symbols: vec![
            GraphSymbolArtifact {
                stable_symbol_id: "section-good".to_owned(),
                file_path: "docs/good.md".to_owned(),
                byte_range: [0, good_source.len()],
                line_range: [1, 3],
                entity_name: "Good".to_owned(),
                qualified_name: "Good".to_owned(),
                symbol_kind: "section".to_owned(),
                anchor_hash: "anchor-good".to_owned(),
                enclosing_scope: None,
            },
            GraphSymbolArtifact {
                stable_symbol_id: "section-bad".to_owned(),
                file_path: "docs/bad.md".to_owned(),
                byte_range: [0, 24],
                line_range: [1, 3],
                entity_name: "Bad".to_owned(),
                qualified_name: "Bad".to_owned(),
                symbol_kind: "section".to_owned(),
                anchor_hash: "anchor-bad".to_owned(),
                enclosing_scope: None,
            },
        ],
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    };
    let out_dir = tempdir.path().join("artifact");

    write_sections_dataset(&artifact, &root, &out_dir).expect("write sections sidecar");

    let parquet_path = out_dir.join(SECTIONS_PARQUET);
    let file_paths = string_column_values(&read_sidecar_parquet(&parquet_path), "file_path");

    assert_eq!(file_paths, vec!["docs/good.md".to_owned()]);
    assert!(!file_paths.iter().any(|path| path == "docs/bad.md"));
}
