//! Run explicitly on each prebuilt target with signed DuckDB extensions available.
//! The build wrapper separately verifies that libduckdb-sys/bundled is disabled.

#[test]
#[ignore = "requires cached/vendored signed DuckPGQ and Onager extensions or network access"]
fn native_extensions_preserve_graph_queries_and_error_recovery() -> anyhow::Result<()> {
    let conn = duckdb::Connection::open_in_memory()?;
    let version: String = conn.query_row("SELECT version()", [], |row| row.get(0))?;
    assert_eq!(version, "v1.4.4");

    for extension in ["duckpgq", "onager"] {
        conn.execute_batch(&spur_analyst::analyst_extension_load_sql(extension))?;
    }
    let loaded: i64 = conn.query_row(
        "SELECT count(*) FROM duckdb_extensions() \
         WHERE extension_name IN ('duckpgq', 'onager') AND loaded",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(loaded, 2);

    conn.execute_batch(
        "CREATE TABLE vertices(id INTEGER PRIMARY KEY); \
         CREATE TABLE edges(src INTEGER, dst INTEGER); \
         INSERT INTO vertices VALUES (1), (2); \
         INSERT INTO edges VALUES (1, 2); \
         CREATE PROPERTY GRAPH linkage_test \
           VERTEX TABLES (vertices) \
           EDGE TABLES (edges SOURCE KEY (src) REFERENCES vertices (id) \
                             DESTINATION KEY (dst) REFERENCES vertices (id));",
    )?;
    let edge: (i32, i32) = conn.query_row(
        "SELECT source_id, target_id FROM GRAPH_TABLE (linkage_test \
           MATCH (a:vertices)-[e:edges]->(b:vertices) \
           COLUMNS (a.id AS source_id, b.id AS target_id))",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(edge, (1, 2));

    let ranked: i64 = conn.query_row(
        "SELECT count(*) FROM onager_ctr_pagerank(\
           (SELECT src::BIGINT, dst::BIGINT FROM edges))",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(ranked, 2);

    // This exercises a C++ exception across the engine/extension boundary.
    // A mismatched C++ runtime can abort the process instead of returning Err.
    let error = conn
        .execute_batch(
            "SELECT * FROM GRAPH_TABLE (missing_graph \
             MATCH (a:vertices) COLUMNS (a.id))",
        )
        .expect_err("an unknown property graph must return an error");
    assert!(error.to_string().contains("missing_graph"));
    let answer: i32 = conn.query_row("SELECT 42", [], |row| row.get(0))?;
    assert_eq!(answer, 42);
    Ok(())
}

#[test]
#[ignore = "run explicitly against the selected native DuckDB artifact"]
fn native_json_parquet_and_transactions_round_trip() -> anyhow::Result<()> {
    let conn = duckdb::Connection::open_in_memory()?;
    let answer: i32 = conn.query_row(
        "SELECT json_extract('{\"answer\":42}', '$.answer')::INTEGER",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(answer, 42);
    conn.execute_batch(
        "CREATE TABLE values_test AS SELECT 42 AS answer; \
         BEGIN; INSERT INTO values_test VALUES (100); ROLLBACK;",
    )?;
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("round-trip.parquet");
    let sql_path = path
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "''");
    conn.execute_batch(&format!(
        "COPY values_test TO '{sql_path}' (FORMAT PARQUET)"
    ))?;
    let total: i64 = conn.query_row(
        &format!("SELECT sum(answer)::BIGINT FROM read_parquet('{sql_path}')"),
        [],
        |row| row.get(0),
    )?;
    assert_eq!(total, 42);
    Ok(())
}
