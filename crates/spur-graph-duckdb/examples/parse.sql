-- DuckDB 2.0.0 with the locally packaged spur_graph extension loaded.
WITH blobs(blob_id, path, source) AS (
    VALUES
        ('rust-example', 'input.rs', 'pub fn alpha() -> i32 { 1 }'),
        ('cpp-example', 'input.cpp', 'int total(int x) { return x + 1; }'),
        ('empty-example', 'empty.rs', '')
), parsed AS MATERIALIZED (
    SELECT blob_id, path, spur_graph_symbols(path, source) AS symbols
    FROM blobs
)
SELECT blob_id, path, unnest(symbols, recursive := true)
FROM parsed;

-- Preserve the parsed parent rows when recording completion: an empty symbol
-- list represents a completed parse and will produce no rows when unnested.
