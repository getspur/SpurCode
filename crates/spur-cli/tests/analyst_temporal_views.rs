use std::path::Path;

use spur_cli::commands::analyst::{self, AnalystBuildOptions};
use spur_graph::store::lance_sections::{
    write_sections_dataset_best_effort_with_options, SectionEmbeddingOptions, CODE_SYMBOLS_PARQUET,
    SECTIONS_PARQUET,
};
use spur_graph::{
    ChangeKind, CommitArtifact, Confidence, EdgeEndpoint, GraphArtifactSidecarRowCounts,
    GraphArtifactSidecarStatus, GraphEdgeArtifact, GraphEdgeKind, GraphFileArtifact,
    GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphSymbolArtifact, NodeId,
    RelationKind, RenamePrev, SnapshotKey, SymbolSnapshotArtifact, TemporalEdgeArtifact,
    WriteOptions,
};

const INIT_SQL: &str = include_str!("../../spur-context/analyst/init.sql");

fn query_csv(db_path: &Path, sql: &str) -> String {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let _ = conn.execute_batch("INSTALL icu; LOAD icu;");

    let _ = conn.execute_batch("INSTALL duckpgq FROM community; LOAD duckpgq;");
    let mut stmt = conn.prepare(sql).expect("prepare query");
    let mut rows = stmt.query([]).expect("query rows");
    let column_count = rows.as_ref().expect("query result").column_count();
    let mut lines = Vec::new();
    while let Some(row) = rows.next().expect("read row") {
        let mut fields = Vec::with_capacity(column_count);
        for idx in 0..column_count {
            fields.push(duckdb_value_to_csv(row.get_ref(idx).expect("read column")));
        }
        lines.push(fields.join(","));
    }
    lines.join("\n")
}

fn run_duckdb_sql(db_path: &Path, sql: &str) {
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadWrite)
        .expect("read-write duckdb config");
    let conn = duckdb::Connection::open_with_flags(db_path, config).expect("open duckdb");
    conn.execute_batch(sql).expect("duckdb init SQL");
}

fn duckdb_value_to_csv(value: duckdb::types::ValueRef<'_>) -> String {
    use duckdb::types::ValueRef;

    match value {
        ValueRef::Null => String::new(),
        ValueRef::Boolean(value) => value.to_string(),
        ValueRef::TinyInt(value) => value.to_string(),
        ValueRef::SmallInt(value) => value.to_string(),
        ValueRef::Int(value) => value.to_string(),
        ValueRef::BigInt(value) => value.to_string(),
        ValueRef::HugeInt(value) => value.to_string(),
        ValueRef::UTinyInt(value) => value.to_string(),
        ValueRef::USmallInt(value) => value.to_string(),
        ValueRef::UInt(value) => value.to_string(),
        ValueRef::UBigInt(value) => value.to_string(),
        ValueRef::Float(value) => value.to_string(),
        ValueRef::Double(value) => value.to_string(),
        ValueRef::Decimal(value) => value.to_string(),
        ValueRef::Timestamp(_, value) => value.to_string(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        ValueRef::Blob(value) => format!("{value:?}"),
        ValueRef::Date32(value) => value.to_string(),
        ValueRef::Time64(_, value) => value.to_string(),
        ValueRef::Interval {
            months,
            days,
            nanos,
        } => format!("{months} months {days} days {nanos} nanos"),
        ValueRef::List(_, _)
        | ValueRef::Enum(_, _)
        | ValueRef::Struct(_, _)
        | ValueRef::Array(_, _)
        | ValueRef::Map(_, _)
        | ValueRef::Union(_, _) => format!("{value:?}"),
    }
}

fn structural_init_sql_for_test(artifact_dir: &Path) -> String {
    let artifact_dir_sql = artifact_dir.display().to_string().replace('\'', "''");
    let sql = INIT_SQL.replace("__SPUR_GRAPH_ARTIFACT_DIR__", &artifact_dir_sql);

    let mut filtered = String::new();
    let mut skip_property_graph = false;
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("INSTALL ") || trimmed.starts_with("LOAD ") {
            continue;
        }
        if trimmed.starts_with("CREATE OR REPLACE PROPERTY GRAPH code") {
            skip_property_graph = true;
            continue;
        }
        if skip_property_graph {
            if trimmed == ");" {
                skip_property_graph = false;
            }
            continue;
        }
        filtered.push_str(line);
        filtered.push('\n');
    }
    filtered
}

fn build_analyst(root: &Path, artifact_dir: &Path, db_path: &Path) {
    let options = AnalystBuildOptions {
        artifact_dir: Some(artifact_dir.to_path_buf()),
        db_path: Some(db_path.to_path_buf()),
        quiet: true,
    };
    analyst::build(root, options.clone()).expect("analyst build");

    if !db_path.is_file() {
        analyst::build(
            root,
            AnalystBuildOptions {
                quiet: false,
                ..options
            },
        )
        .expect("analyst build retry");
    }

    assert!(
        db_path.is_file(),
        "analyst build should materialize {}",
        db_path.display()
    );
}

fn temporal_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = structural_artifact(graph_content_hash);
    let snapshot = SnapshotKey {
        stable_symbol_id: "sym-main".to_string(),
        commit: "c1".to_string(),
    };

    artifact.symbols.push(GraphSymbolArtifact {
        stable_symbol_id: "sym-main".to_string(),
        file_path: "src/lib.rs".to_string(),
        byte_range: [0, 10],
        line_range: [1, 1],
        entity_name: "main".to_string(),
        qualified_name: "main".to_string(),
        symbol_kind: "function".to_string(),
        anchor_hash: "anchor-main".to_string(),
        enclosing_scope: None,
    });
    artifact.symbol_node_ids.push(NodeId(2));

    artifact.commits.push(CommitArtifact {
        sha: "c1".to_string(),
        parents: Vec::new(),
        author_time: 1_700_000_001,
        author_name: String::new(),
        author_email: String::new(),
        summary: "initial import".to_string(),
    });
    artifact.symbol_snapshots.push(SymbolSnapshotArtifact {
        key: snapshot.clone(),
        file_path: "src/lib.rs".into(),
        entity_name: "main".to_string(),
        symbol_kind: "function".to_string(),
        enclosing_scope: None,
        byte_range: [0, 10],
        line_range: [1, 1],
        anchor_hash: "anchor-main".to_string(),
        tokens: vec!["main".to_string()],
    });
    artifact.temporal_edges.push(TemporalEdgeArtifact {
        source: EdgeEndpoint::Commit {
            sha: "c1".to_string(),
        },
        target: EdgeEndpoint::Snapshot {
            key: snapshot.clone(),
        },
        relation: RelationKind::Touches,
        parent: None,
        change_kind: Some(ChangeKind::Added),
    });
    artifact.temporal_edges.push(TemporalEdgeArtifact {
        source: EdgeEndpoint::Snapshot {
            key: snapshot.clone(),
        },
        target: EdgeEndpoint::Snapshot {
            key: snapshot.clone(),
        },
        relation: RelationKind::Touches,
        parent: None,
        change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(snapshot))),
    });
    artifact
        .diagnostics
        .push("parse_failed path=src/lib.rs".to_string());
    artifact
}

#[test]
fn analyst_build_emits_temporal_and_diagnostics_views_when_parquets_exist() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = temporal_artifact("analyst-temporal-views");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let counts = query_csv(
        &db_path,
        "SELECT commit_count, symbol_snapshot_count, temporal_edge_count, diagnostic_count
         FROM _meta;",
    );
    assert_eq!(counts.trim(), "1,1,2,1");

    let renamed_from_snapshots = query_csv(
        &db_path,
        "SELECT count(*)
         FROM temporal_edges
         WHERE source_kind = 'snapshot'
           AND change_kind LIKE 'renamed_from%';",
    );
    assert_eq!(renamed_from_snapshots.trim(), "1");
}

#[test]
fn analyst_build_skips_temporal_and_diagnostics_views_without_optional_parquets() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = structural_artifact("analyst-structural-only");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let optional_views = query_csv(
        &db_path,
        "SELECT view_name
         FROM duckdb_views()
         WHERE view_name IN ('commits', 'symbol_snapshots', 'temporal_edges', 'diagnostics')
         ORDER BY view_name;",
    );
    assert_eq!(optional_views.trim(), "");
}

#[test]
fn analyst_duckpgq_nodes_include_file_and_stub_endpoints() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = identity_integrity_artifact("analyst-identity-fk");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let kinds = query_csv(
        &db_path,
        "SELECT stable_symbol_id, symbol_kind
         FROM duckpgq_nodes
         WHERE stable_symbol_id IN ('file-src-lib', 'fn-main', 'external-foo')
         ORDER BY stable_symbol_id;",
    );
    assert_eq!(
        kinds.trim(),
        "external-foo,external\nfile-src-lib,file\nfn-main,function"
    );

    let orphans = query_csv(
        &db_path,
        "SELECT count(*)
         FROM duckpgq_edges e
         WHERE e.source_stable_id NOT IN (SELECT stable_symbol_id FROM duckpgq_nodes)
            OR e.target_stable_id NOT IN (SELECT stable_symbol_id FROM duckpgq_nodes);",
    );
    assert_eq!(orphans.trim(), "0");
}

#[test]
fn analyst_onager_dep_edges_exclude_contains() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = identity_integrity_artifact("analyst-identity-dep");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let contains_in_dep = query_csv(
        &db_path,
        "SELECT count(*)
         FROM onager_dep_edges d
         JOIN edges e ON e.src_id = d.src AND e.dst_id = d.dst
         WHERE e.relation = 'contains';",
    );
    assert_eq!(contains_in_dep.trim(), "0");

    let dep_relations = query_csv(
        &db_path,
        "SELECT DISTINCT e.relation
         FROM onager_dep_edges d
         JOIN edges e ON e.src_id = d.src AND e.dst_id = d.dst
         ORDER BY e.relation;",
    );
    assert_eq!(dep_relations.trim(), "calls\nimports");
}

#[test]
fn analyst_pagerank_is_null_outside_calls_graph() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = identity_integrity_artifact("analyst-identity-pagerank");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let unused = query_csv(
        &db_path,
        "SELECT pagerank IS NULL
         FROM v_symbol_centrality
         WHERE stable_symbol_id = 'fn-unused';",
    );
    assert_eq!(unused.trim(), "true");

    let in_calls = query_csv(
        &db_path,
        "SELECT pagerank IS NOT NULL
         FROM v_symbol_centrality
         WHERE stable_symbol_id IN ('fn-main', 'fn-helper')
         ORDER BY stable_symbol_id;",
    );
    assert_eq!(in_calls.trim(), "true\ntrue");
}

#[test]
fn analyst_build_emits_external_dependency_surface_views() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = external_dependency_artifact("analyst-external-dependencies");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let dependency_views = query_csv(
        &db_path,
        "SELECT view_name
         FROM duckdb_views()
         WHERE view_name IN ('external_nodes', 'v_dependency_surface')
         ORDER BY view_name;",
    );
    assert_eq!(
        dependency_views.trim(),
        "external_nodes\nv_dependency_surface"
    );

    let origins = query_csv(
        &db_path,
        "SELECT qualified_name, origin
         FROM external_nodes
         ORDER BY qualified_name;",
    );
    assert_eq!(
        origins.trim(),
        "alloc::vec::Vec,std\ncore::fmt::Formatter,std\nserde::Serialize,serde\nstd::fmt::Debug,std"
    );

    let surface = query_csv(
        &db_path,
        "SELECT crate_name, file_path, external_origin, external_symbol, inbound_import_count
         FROM v_dependency_surface
         ORDER BY external_origin, external_symbol;",
    );
    assert_eq!(
        surface.trim(),
        "spur-core,crates/spur-core/src/lib.rs,serde,serde::Serialize,2\nspur-core,crates/spur-core/src/lib.rs,std,alloc::vec::Vec,1\nspur-core,crates/spur-core/src/lib.rs,std,core::fmt::Formatter,1\nspur-core,crates/spur-core/src/lib.rs,std,std::fmt::Debug,1"
    );

    let pgq_imports = query_csv(
        &db_path,
        "FROM GRAPH_TABLE (code
           MATCH (s:duckpgq_nodes)-[i:imports]->(e:External)
           WHERE s.file_path = 'crates/spur-core/src/lib.rs'
           COLUMNS (e.qualified_name AS external_symbol)
         )
         ORDER BY external_symbol;",
    );
    assert_eq!(
        pgq_imports.trim(),
        "alloc::vec::Vec\ncore::fmt::Formatter\nserde::Serialize\nserde::Serialize\nstd::fmt::Debug"
    );
}

#[test]
fn analyst_build_emits_cross_crate_call_surface_views() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = cross_crate_call_artifact("analyst-cross-crate-calls");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    run_duckdb_sql(&db_path, &structural_init_sql_for_test(&artifact_dir));

    let cross_crate_views = query_csv(
        &db_path,
        "SELECT view_name
         FROM duckdb_views()
         WHERE view_name IN ('v_cross_crate_calls', 'v_import_licensed_precision_gate')
         ORDER BY view_name;",
    );
    assert_eq!(
        cross_crate_views.trim(),
        "v_cross_crate_calls\nv_import_licensed_precision_gate"
    );

    let calls = query_csv(
        &db_path,
        "SELECT source_crate, target_crate, source_symbol, target_symbol, bind_method
         FROM v_cross_crate_calls
         ORDER BY source_crate, target_crate, target_symbol;",
    );
    assert_eq!(
        calls.trim(),
        "spur-mcp,spur-graph,handle_submit,build_facts,import_licensed"
    );

    let import_only_not_counted = query_csv(
        &db_path,
        "SELECT count(*)
         FROM v_cross_crate_calls
         WHERE target_symbol = 'import_only';",
    );
    assert_eq!(import_only_not_counted.trim(), "0");

    let gate_violations = query_csv(
        &db_path,
        "SELECT count(*)
         FROM v_import_licensed_precision_gate;",
    );
    assert_eq!(gate_violations.trim(), "0");
}

#[test]
fn analyst_build_rejects_low_direct_symbol_snapshot_coverage() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = divergent_symbol_id_artifact("analyst-low-direct-coverage");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    analyst::build(
        tempdir.path(),
        AnalystBuildOptions {
            artifact_dir: Some(artifact_dir),
            db_path: Some(db_path.clone()),
            quiet: true,
        },
    )
    .expect("analyst build");

    assert!(
        !db_path.exists(),
        "analyst DB should not be created when distinct symbol snapshot coverage is below 90%"
    );
}

#[test]
fn analyst_views_map_temporal_churn_by_direct_symbol_ids() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let artifact = direct_symbol_id_artifact("analyst-direct-symbol-views");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let coverage = query_csv(
        &db_path,
        "SELECT
           (SELECT COUNT(*) FROM nodes n JOIN symbol_snapshots s USING (stable_symbol_id)),
           (SELECT node_count FROM _meta LIMIT 1);",
    );
    assert_eq!(coverage.trim(), "2,2");

    let churn = query_csv(
        &db_path,
        "SELECT stable_symbol_id, events
         FROM v_symbol_churn_90d
         ORDER BY stable_symbol_id;",
    );
    assert_eq!(churn.trim(), "struct-caller,1\nstruct-target,1");

    let blast = query_csv(
        &db_path,
        "SELECT stable_symbol_id, caller_count, caller_churn_90d, self_churn_90d,
                blast_radius_score > 0
         FROM v_blast_radius
         WHERE stable_symbol_id = 'struct-target';",
    );
    assert_eq!(blast.trim(), "struct-target,1,1,1,true");
}

#[test]
fn analyst_build_uses_complete_lance_sidecar_for_hybrid_search() {
    let tempdir = tempfile::Builder::new()
        .prefix("bundled-duckdb-complete-sidecar")
        .tempdir()
        .expect("tempdir");
    write_section_fixture_source(tempdir.path());
    let artifact = temporal_artifact_with_section("analyst-complete-sidecar");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    write_sections_dataset_best_effort_with_options(
        &artifact,
        tempdir.path(),
        &artifact_dir,
        SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: true,
            batch_size: 64,
        },
    );
    spur_graph::store::stamp_sidecar_status(
        &artifact_dir,
        GraphArtifactSidecarStatus {
            complete: true,
            row_counts: GraphArtifactSidecarRowCounts {
                section_bodies: 1,
                code_symbols: 1,
            },
        },
    )
    .expect("stamp sidecar complete");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let section_count = query_csv(&db_path, "SELECT count(*) FROM sections;");
    assert_eq!(section_count.trim(), "1");

    let hybrid_macro_count = query_csv(
        &db_path,
        "SELECT count(*)
         FROM duckdb_functions()
         WHERE function_name = 'search_context_candidates_hybrid';",
    );
    assert_eq!(hybrid_macro_count.trim(), "1");
}

#[test]
fn analyst_build_uses_complete_lance_sidecar_for_hybrid_search_without_temporal() {
    let tempdir = tempfile::Builder::new()
        .prefix("bundled-duckdb-complete-sidecar-no-temporal")
        .tempdir()
        .expect("tempdir");
    write_section_fixture_source(tempdir.path());
    let artifact = non_temporal_artifact_with_section("analyst-complete-sidecar-no-temporal");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    write_sections_dataset_best_effort_with_options(
        &artifact,
        tempdir.path(),
        &artifact_dir,
        SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: true,
            batch_size: 64,
        },
    );
    spur_graph::store::stamp_sidecar_status(
        &artifact_dir,
        GraphArtifactSidecarStatus {
            complete: true,
            row_counts: GraphArtifactSidecarRowCounts {
                section_bodies: 1,
                code_symbols: 1,
            },
        },
    )
    .expect("stamp sidecar complete");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let macros = query_csv(
        &db_path,
        "SELECT function_name
         FROM duckdb_functions()
         WHERE function_name IN (
           'search_context_candidates',
           'search_context_candidates_hybrid'
         )
         ORDER BY function_name;",
    );
    assert_eq!(
        macros.trim(),
        "search_context_candidates\nsearch_context_candidates_hybrid"
    );

    let optional_views = query_csv(
        &db_path,
        "SELECT view_name
         FROM duckdb_views()
         WHERE view_name IN ('commits', 'symbol_snapshots', 'temporal_edges')
         ORDER BY view_name;",
    );
    assert_eq!(optional_views.trim(), "");

    let section_count = query_csv(&db_path, "SELECT count(*) FROM sections;");
    assert_eq!(section_count.trim(), "1");
}

#[test]
fn analyst_build_degrades_to_bm25_when_sidecar_manifest_incomplete() {
    let tempdir = tempfile::Builder::new()
        .prefix("bundled-duckdb-incomplete-sidecar")
        .tempdir()
        .expect("tempdir");
    write_section_fixture_source(tempdir.path());
    let artifact = temporal_artifact_with_section("analyst-incomplete-sidecar");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    write_sections_dataset_best_effort_with_options(
        &artifact,
        tempdir.path(),
        &artifact_dir,
        SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: true,
            batch_size: 64,
        },
    );
    spur_graph::store::stamp_sidecar_status(
        &artifact_dir,
        GraphArtifactSidecarStatus {
            complete: false,
            row_counts: GraphArtifactSidecarRowCounts::default(),
        },
    )
    .expect("stamp sidecar incomplete");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let section_count = query_csv(&db_path, "SELECT count(*) FROM sections;");
    assert_eq!(section_count.trim(), "0");

    let hybrid_macro_count = query_csv(
        &db_path,
        "SELECT count(*)
         FROM duckdb_functions()
         WHERE function_name = 'search_context_candidates_hybrid';",
    );
    assert_eq!(
        hybrid_macro_count.trim(),
        "1",
        "cosine hybrid must remain registered when Lance sidecars are incomplete"
    );
}

#[test]
fn analyst_build_degrades_to_bm25_when_complete_sidecar_dir_is_empty() {
    let tempdir = tempfile::Builder::new()
        .prefix("bundled-duckdb-empty-sidecar")
        .tempdir()
        .expect("tempdir");
    write_section_fixture_source(tempdir.path());
    let artifact = temporal_artifact_with_section("analyst-empty-sidecar");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    std::fs::write(artifact_dir.join(SECTIONS_PARQUET), [])
        .expect("create stale empty sections.parquet");
    spur_graph::store::stamp_sidecar_status(
        &artifact_dir,
        GraphArtifactSidecarStatus {
            complete: true,
            row_counts: GraphArtifactSidecarRowCounts {
                section_bodies: 1,
                code_symbols: 1,
            },
        },
    )
    .expect("stamp sidecar complete");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let section_count = query_csv(&db_path, "SELECT count(*) FROM sections;");
    assert_eq!(section_count.trim(), "0");

    let hybrid_macro_count = query_csv(
        &db_path,
        "SELECT count(*)
         FROM duckdb_functions()
         WHERE function_name = 'search_context_candidates_hybrid';",
    );
    assert_eq!(
        hybrid_macro_count.trim(),
        "1",
        "cosine hybrid must remain registered when Lance sidecar dirs are empty"
    );
}

#[test]
fn analyst_build_degrades_to_bm25_when_code_symbols_sidecar_missing() {
    let tempdir = tempfile::Builder::new()
        .prefix("bundled-duckdb-missing-code-symbol-sidecar")
        .tempdir()
        .expect("tempdir");
    write_section_fixture_source(tempdir.path());
    let artifact = temporal_artifact_with_section("analyst-missing-code-symbol-sidecar");
    let artifact_dir = spur_graph::store::parquet::write_artifact_parquet(
        &artifact,
        tempdir.path(),
        WriteOptions::default(),
        Vec::new(),
    )
    .expect("write artifact");
    write_sections_dataset_best_effort_with_options(
        &artifact,
        tempdir.path(),
        &artifact_dir,
        SectionEmbeddingOptions {
            skip_section_embeddings: true,
            skip_code_symbol_embeddings: true,
            batch_size: 64,
        },
    );
    std::fs::remove_file(artifact_dir.join(CODE_SYMBOLS_PARQUET))
        .expect("remove stale code_symbols.parquet");
    spur_graph::store::stamp_sidecar_status(
        &artifact_dir,
        GraphArtifactSidecarStatus {
            complete: true,
            row_counts: GraphArtifactSidecarRowCounts {
                section_bodies: 1,
                code_symbols: 0,
            },
        },
    )
    .expect("stamp sidecar with missing code symbols");
    let db_path = tempdir.path().join("analyst.duckdb");

    build_analyst(tempdir.path(), &artifact_dir, &db_path);

    let section_count = query_csv(&db_path, "SELECT count(*) FROM sections;");
    assert_ne!(
        section_count.trim(),
        "0",
        "section parquet should still load when code_symbols.parquet is absent"
    );

    let hybrid_macro_count = query_csv(
        &db_path,
        "SELECT count(*)
         FROM duckdb_functions()
         WHERE function_name = 'search_context_candidates_hybrid';",
    );
    assert_eq!(
        hybrid_macro_count.trim(),
        "1",
        "cosine hybrid must remain registered when code_symbols.parquet is missing"
    );
}

fn external_dependency_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = structural_artifact(graph_content_hash);
    artifact.files = vec![GraphFileArtifact {
        stable_file_id: "file-spur-core-lib".to_string(),
        file_path: "crates/spur-core/src/lib.rs".to_string(),
    }];
    artifact.file_node_ids = vec![NodeId(1)];
    artifact.symbols = vec![
        GraphSymbolArtifact {
            stable_symbol_id: "spur-core-main".to_string(),
            file_path: "crates/spur-core/src/lib.rs".to_string(),
            byte_range: [0, 20],
            line_range: [1, 2],
            entity_name: "main".to_string(),
            qualified_name: "main".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-main".to_string(),
            enclosing_scope: None,
        },
        GraphSymbolArtifact {
            stable_symbol_id: "spur-core-helper".to_string(),
            file_path: "crates/spur-core/src/lib.rs".to_string(),
            byte_range: [30, 50],
            line_range: [4, 5],
            entity_name: "helper".to_string(),
            qualified_name: "helper".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-helper".to_string(),
            enclosing_scope: None,
        },
        external_symbol("external-serde-serialize", "serde::Serialize"),
        external_symbol("external-std-debug", "std::fmt::Debug"),
        external_symbol("external-core-formatter", "core::fmt::Formatter"),
        external_symbol("external-alloc-vec", "alloc::vec::Vec"),
    ];
    artifact.symbol_node_ids = vec![
        NodeId(2),
        NodeId(3),
        NodeId(4),
        NodeId(5),
        NodeId(6),
        NodeId(7),
    ];
    artifact.edges = vec![
        import_edge("spur-core-main", "external-serde-serialize"),
        import_edge("spur-core-helper", "external-serde-serialize"),
        import_edge("spur-core-main", "external-std-debug"),
        import_edge("spur-core-main", "external-core-formatter"),
        import_edge("spur-core-helper", "external-alloc-vec"),
    ];
    artifact
}

fn external_symbol(stable_symbol_id: &str, qualified_name: &str) -> GraphSymbolArtifact {
    GraphSymbolArtifact {
        stable_symbol_id: stable_symbol_id.to_string(),
        file_path: qualified_name.to_string(),
        byte_range: [0, 0],
        line_range: [0, 0],
        entity_name: qualified_name
            .rsplit("::")
            .next()
            .unwrap_or(qualified_name)
            .to_string(),
        qualified_name: qualified_name.to_string(),
        symbol_kind: "external".to_string(),
        anchor_hash: format!("anchor-{stable_symbol_id}"),
        enclosing_scope: None,
    }
}

fn import_edge(source: &str, target: &str) -> GraphEdgeArtifact {
    GraphEdgeArtifact {
        source_stable_symbol_id: source.to_string(),
        target_stable_symbol_id: Some(target.to_string()),
        target_label: None,
        relation: RelationKind::Imports,
        confidence: Confidence::SyntaxExact,
        confidence_score: 1.0,
        change_kind: None,
        edge_kind: Some(GraphEdgeKind::ReferencesOther),
        bind_method: Some("external_import".to_string()),
        import_path: None,
        receiver_text: None,
        scope_text: None,
    }
}

fn cross_crate_call_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = structural_artifact(graph_content_hash);
    artifact.files = vec![
        GraphFileArtifact {
            stable_file_id: "file-spur-mcp-server".to_string(),
            file_path: "crates/spur-mcp/src/server.rs".to_string(),
        },
        GraphFileArtifact {
            stable_file_id: "file-spur-graph-lib".to_string(),
            file_path: "crates/spur-graph/src/lib.rs".to_string(),
        },
    ];
    artifact.file_node_ids = vec![NodeId(1), NodeId(2)];
    artifact.symbols = vec![
        GraphSymbolArtifact {
            stable_symbol_id: "spur-mcp-handle-submit".to_string(),
            file_path: "crates/spur-mcp/src/server.rs".to_string(),
            byte_range: [0, 20],
            line_range: [1, 2],
            entity_name: "handle_submit".to_string(),
            qualified_name: "spur_mcp::server::handle_submit".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-handle-submit".to_string(),
            enclosing_scope: None,
        },
        GraphSymbolArtifact {
            stable_symbol_id: "spur-graph-build-facts".to_string(),
            file_path: "crates/spur-graph/src/lib.rs".to_string(),
            byte_range: [40, 60],
            line_range: [5, 7],
            entity_name: "build_facts".to_string(),
            qualified_name: "spur_graph::build_facts".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-build-facts".to_string(),
            enclosing_scope: None,
        },
        GraphSymbolArtifact {
            stable_symbol_id: "spur-graph-import-only".to_string(),
            file_path: "crates/spur-graph/src/lib.rs".to_string(),
            byte_range: [80, 100],
            line_range: [10, 12],
            entity_name: "import_only".to_string(),
            qualified_name: "spur_graph::import_only".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-import-only".to_string(),
            enclosing_scope: None,
        },
    ];
    artifact.symbol_node_ids = vec![NodeId(3), NodeId(4), NodeId(5)];
    artifact.edges = vec![
        workspace_import_edge(
            "spur-mcp-handle-submit",
            "spur-graph-build-facts",
            "build_facts",
            "spur_graph::build_facts",
        ),
        workspace_import_edge(
            "spur-mcp-handle-submit",
            "spur-graph-import-only",
            "import_only",
            "spur_graph::import_only",
        ),
        GraphEdgeArtifact {
            source_stable_symbol_id: "spur-mcp-handle-submit".to_string(),
            target_stable_symbol_id: Some("spur-graph-build-facts".to_string()),
            target_label: Some("build_facts".to_string()),
            relation: RelationKind::Calls,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: Some(GraphEdgeKind::Calls),
            bind_method: Some("import_licensed".to_string()),
            import_path: None,
            receiver_text: None,
            scope_text: None,
        },
    ];
    artifact
}

fn workspace_import_edge(
    source: &str,
    target: &str,
    target_label: &str,
    import_path: &str,
) -> GraphEdgeArtifact {
    GraphEdgeArtifact {
        source_stable_symbol_id: source.to_string(),
        target_stable_symbol_id: Some(target.to_string()),
        target_label: Some(target_label.to_string()),
        relation: RelationKind::Imports,
        confidence: Confidence::SyntaxExact,
        confidence_score: 1.0,
        change_kind: None,
        edge_kind: Some(GraphEdgeKind::ReferencesOther),
        bind_method: Some("import_path".to_string()),
        import_path: Some(import_path.to_string()),
        receiver_text: None,
        scope_text: None,
    }
}

fn direct_symbol_id_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = structural_artifact(graph_content_hash);
    artifact.symbols = vec![
        GraphSymbolArtifact {
            stable_symbol_id: "struct-caller".to_string(),
            file_path: "src/lib.rs".to_string(),
            byte_range: [0, 20],
            line_range: [1, 3],
            entity_name: "caller".to_string(),
            qualified_name: "caller".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-caller".to_string(),
            enclosing_scope: None,
        },
        GraphSymbolArtifact {
            stable_symbol_id: "struct-target".to_string(),
            file_path: "src/lib.rs".to_string(),
            byte_range: [40, 60],
            line_range: [5, 7],
            entity_name: "target".to_string(),
            qualified_name: "target".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-target".to_string(),
            enclosing_scope: None,
        },
    ];
    artifact.symbol_node_ids = vec![NodeId(2), NodeId(3)];
    artifact.edges.push(GraphEdgeArtifact {
        source_stable_symbol_id: "struct-caller".to_string(),
        target_stable_symbol_id: Some("struct-target".to_string()),
        target_label: None,
        relation: RelationKind::Calls,
        confidence: Confidence::SyntaxExact,
        confidence_score: 1.0,
        change_kind: None,
        edge_kind: Some(GraphEdgeKind::Calls),
        bind_method: None,
        import_path: None,
        receiver_text: None,
        scope_text: None,
    });

    artifact.commits.push(CommitArtifact {
        sha: "c1".to_string(),
        parents: Vec::new(),
        author_time: chrono::Utc::now().timestamp(),
        author_name: String::new(),
        author_email: String::new(),
        summary: "touch symbols".to_string(),
    });
    for (structural_id, snapshot_id, name, byte_range, line_range) in [
        ("struct-caller", "struct-caller", "caller", [0, 20], [1, 3]),
        ("struct-target", "struct-target", "target", [40, 60], [5, 7]),
    ] {
        let snapshot = SnapshotKey {
            stable_symbol_id: snapshot_id.to_string(),
            commit: "c1".to_string(),
        };
        artifact.symbol_snapshots.push(SymbolSnapshotArtifact {
            key: snapshot.clone(),
            file_path: "src/lib.rs".into(),
            entity_name: name.to_string(),
            symbol_kind: "function".to_string(),
            enclosing_scope: None,
            byte_range,
            line_range,
            anchor_hash: format!("anchor-{name}"),
            tokens: vec![name.to_string(), structural_id.to_string()],
        });
        artifact.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: "c1".to_string(),
            },
            target: EdgeEndpoint::Snapshot { key: snapshot },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::Modified),
        });
    }

    artifact
}

fn divergent_symbol_id_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = direct_symbol_id_artifact(graph_content_hash);
    for snapshot in &mut artifact.symbol_snapshots {
        snapshot.key.stable_symbol_id = format!("snapshot-{}", snapshot.key.stable_symbol_id);
    }
    artifact
}

fn temporal_artifact_with_section(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = temporal_artifact(graph_content_hash);
    artifact.file_manifests.push(GraphFileManifestEntry {
        stable_file_id: "file-docs-guide".to_string(),
        path: "docs/guide.md".to_string(),
        content_oid: "guide-content".to_string(),
        node_ids: vec![NodeId(10), NodeId(11)],
    });
    artifact.files.push(GraphFileArtifact {
        stable_file_id: "file-docs-guide".to_string(),
        file_path: "docs/guide.md".to_string(),
    });
    artifact.file_node_ids.push(NodeId(10));
    artifact.symbols.push(GraphSymbolArtifact {
        stable_symbol_id: "section-guide".to_string(),
        file_path: "docs/guide.md".to_string(),
        byte_range: [0, SECTION_FIXTURE_BODY.len()],
        line_range: [1, 3],
        entity_name: "Guide".to_string(),
        qualified_name: "Guide".to_string(),
        symbol_kind: "section".to_string(),
        anchor_hash: "anchor-guide".to_string(),
        enclosing_scope: None,
    });
    artifact.symbol_node_ids.push(NodeId(11));
    artifact.edges.push(GraphEdgeArtifact {
        source_stable_symbol_id: "file-docs-guide".to_string(),
        target_stable_symbol_id: Some("section-guide".to_string()),
        target_label: None,
        relation: RelationKind::Contains,
        confidence: Confidence::SyntaxExact,
        confidence_score: 1.0,
        change_kind: None,
        edge_kind: Some(GraphEdgeKind::ReferencesOther),
        bind_method: None,
        import_path: None,
        receiver_text: None,
        scope_text: None,
    });
    artifact
}

fn non_temporal_artifact_with_section(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = temporal_artifact_with_section(graph_content_hash);
    artifact.commits.clear();
    artifact.symbol_snapshots.clear();
    artifact.temporal_edges.clear();
    artifact.diagnostics.clear();
    artifact
}

const SECTION_FIXTURE_BODY: &str = "# Guide\n\nUse the analyst search fixture.\n";

fn write_section_fixture_source(root: &Path) {
    std::fs::create_dir_all(root.join("docs")).expect("create docs");
    std::fs::write(root.join("docs/guide.md"), SECTION_FIXTURE_BODY).expect("write guide");
    std::fs::create_dir_all(root.join("src")).expect("create src");
    std::fs::write(root.join("src/lib.rs"), "pub fn main() -> u32 { 42 }\n").expect("write lib");
}

fn identity_integrity_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
    let mut artifact = structural_artifact(graph_content_hash);
    artifact.symbols = vec![
        GraphSymbolArtifact {
            stable_symbol_id: "fn-main".to_string(),
            file_path: "src/lib.rs".to_string(),
            byte_range: [0, 20],
            line_range: [1, 2],
            entity_name: "main".to_string(),
            qualified_name: "main".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-main".to_string(),
            enclosing_scope: None,
        },
        GraphSymbolArtifact {
            stable_symbol_id: "fn-helper".to_string(),
            file_path: "src/lib.rs".to_string(),
            byte_range: [30, 50],
            line_range: [4, 5],
            entity_name: "helper".to_string(),
            qualified_name: "helper".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-helper".to_string(),
            enclosing_scope: None,
        },
        GraphSymbolArtifact {
            stable_symbol_id: "fn-unused".to_string(),
            file_path: "src/lib.rs".to_string(),
            byte_range: [60, 80],
            line_range: [7, 8],
            entity_name: "unused".to_string(),
            qualified_name: "unused".to_string(),
            symbol_kind: "function".to_string(),
            anchor_hash: "anchor-unused".to_string(),
            enclosing_scope: None,
        },
        GraphSymbolArtifact {
            stable_symbol_id: "external-foo".to_string(),
            file_path: "foo".to_string(),
            byte_range: [0, 0],
            line_range: [0, 0],
            entity_name: "foo".to_string(),
            qualified_name: "foo::foo".to_string(),
            symbol_kind: "external".to_string(),
            anchor_hash: "anchor-external-foo".to_string(),
            enclosing_scope: None,
        },
    ];
    artifact.symbol_node_ids = vec![NodeId(2), NodeId(3), NodeId(4), NodeId(5)];
    artifact.edges = vec![
        GraphEdgeArtifact {
            source_stable_symbol_id: "file-src-lib".to_string(),
            target_stable_symbol_id: Some("fn-main".to_string()),
            target_label: None,
            relation: RelationKind::Contains,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: Some(GraphEdgeKind::ReferencesOther),
            bind_method: None,
            import_path: None,
            receiver_text: None,
            scope_text: None,
        },
        GraphEdgeArtifact {
            source_stable_symbol_id: "fn-main".to_string(),
            target_stable_symbol_id: Some("fn-helper".to_string()),
            target_label: Some("helper".to_string()),
            relation: RelationKind::Calls,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: Some(GraphEdgeKind::Calls),
            bind_method: None,
            import_path: None,
            receiver_text: None,
            scope_text: None,
        },
        GraphEdgeArtifact {
            source_stable_symbol_id: "fn-main".to_string(),
            target_stable_symbol_id: Some("external-foo".to_string()),
            target_label: Some("foo".to_string()),
            relation: RelationKind::Imports,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: Some(GraphEdgeKind::ReferencesOther),
            bind_method: Some("external_import".to_string()),
            import_path: Some("foo".to_string()),
            receiver_text: None,
            scope_text: None,
        },
    ];
    artifact
}

fn structural_artifact(graph_content_hash: &str) -> GraphIndexArtifact {
    GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: "spur-graph-phase2".to_string(),
            content_hash_blake3: None,
        },
        manifest_version: "test-manifest-version".to_string(),
        graph_content_hash: graph_content_hash.to_string(),
        file_manifests: Vec::new(),
        files: vec![GraphFileArtifact {
            stable_file_id: "file-src-lib".to_string(),
            file_path: "src/lib.rs".to_string(),
        }],
        file_node_ids: vec![NodeId(1)],
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    }
}
