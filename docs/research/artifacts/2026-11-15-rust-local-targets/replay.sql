-- Cell 9fb655d8-ab91-4268-ad83-4016870abe2f
SELECT version() AS engine_version,
       current_setting('allow_unsigned_extensions') AS allow_unsigned_extensions,
       (SELECT count(*) FROM duckdb_functions() WHERE function_name LIKE 'spur_graph%') AS spur_graph_functions_before_load;

-- Cell cdb72c52-ed01-44cf-9bc5-a38ce6c6bfe9
LOAD '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension';
SELECT version() AS engine_version, extension_name, loaded, extension_version
FROM duckdb_extensions()
WHERE extension_name = 'spur_graph';

-- Cell 102b27ce-9c90-4d0c-a12d-b359582f45be
ATTACH IF NOT EXISTS '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-spurcode-project-quality/snapshot.duckdb' AS spur_rust_snapshot (READ_ONLY);
SELECT table_name, column_name, left(data_type, 90) AS data_type_preview
FROM information_schema.columns
WHERE table_catalog = 'spur_rust_snapshot'
  AND table_name IN ('inventory', 'nodes', 'edges', 'spans', 'sources', 'facts')
ORDER BY table_name, ordinal_position;

-- Cell c4eff936-2a56-4ca9-bb73-e2b28d8d8d4a
SELECT table_name, string_agg(column_name, ', ' ORDER BY ordinal_position) AS columns
FROM information_schema.columns
WHERE table_catalog = 'spur_rust_snapshot'
  AND table_name IN ('inventory', 'nodes', 'edges', 'spans', 'sources', 'facts')
GROUP BY table_name ORDER BY table_name;

-- Cell 9d149c27-1217-4daf-a178-17e3b42a96c8
CREATE OR REPLACE TEMP TABLE spur_rust_package_roots(directory, package) AS
SELECT * FROM (VALUES
('crates/spur-core/tests/fixtures/code_graph_sample', 'code_graph_sample'),
('crates/spur-graph/tests/wrapper-regressions', 'spur-ort-wrapper-regressions'),
('infra/spur-context-service/poc/auth-client', 'spur-context-auth-client'),
('docs/spikes/duckdb-loadable-ext-probe', 'spur-probe-duckdb-extension'),
('crates/spur-utilities/commands', 'spur-commands'),
('crates/spur-utilities/mentions', 'spur-mentions'),
('docs/spikes/duckdb-vtab-probe', 'duckdb-vtab-probe'),
('crates/spur-embedding-worker', 'spur-embedding-worker'),
('crates/spur-utilities/facade', 'spur-utilities'),
('crates/spur-context-fetcher', 'spur-context-fetcher'),
('crates/spur-context-service', 'spur-context-service'),
('crates/spur-context-source', 'spur-context-source'),
('crates/tree-sitter-mermaid', 'tree-sitter-mermaid'),
('crates/spur-license-admin', 'spur-license-admin'),
('crates/spur-context-auth', 'spur-context-auth'),
('crates/spur-graph-duckdb', 'spur-graph-duckdb'),
('crates/spur-interactive', 'spur-interactive'),
('crates/spur-test-madsim', 'spur-test-madsim'),
('crates/spur-blob-store', 'spur-blob-store'),
('crates/spur-code-eval', 'spur-code-eval'),
('crates/spur-embedding', 'spur-embedding'),
('crates/spur-telemetry', 'spur-telemetry'),
('crates/spur-worktree', 'spur-worktree'),
('crates/spur-analyst', 'spur-analyst'),
('crates/spur-context', 'spur-context'),
('crates/spur-license', 'spur-license'),
('crates/spur-solver', 'spur-solver'),
('crates/spur-graph', 'spur-graph'),
('crates/spur-core', 'spur-core'),
('crates/spur-cost', 'spur-cost'),
('crates/spur-acp', 'spur-acp'),
('crates/spur-bot', 'spur-bot'),
('crates/spur-cli', 'spur-cli'),
('crates/spur-jev', 'spur-jev'),
('crates/spur-mcp', 'spur-mcp'),
('crates/spur-tui', 'spur-tui'),
('crates/spur-pm', 'spur-pm'),
('vendor/ort', 'ort'),
('xtask', 'xtask')
) AS roots(directory, package);

CREATE OR REPLACE TEMP TABLE spur_rust_package_paths AS
SELECT i.path,
       (SELECT r.package FROM spur_rust_package_roots r
        WHERE r.directory = '.' OR starts_with(i.path, r.directory || '/')
        ORDER BY length(r.directory) DESC, r.directory LIMIT 1) AS package
FROM spur_rust_snapshot.main.inventory i WHERE i.suffix = '.rs';

CREATE OR REPLACE TEMP TABLE spur_rust_name_counts AS
SELECT n.logical_path, n.label, count(*) AS definitions
FROM spur_rust_snapshot.main.nodes n JOIN spur_rust_package_paths p ON p.path = n.logical_path
WHERE n.kind IN ('function', 'method') GROUP BY ALL;

CREATE OR REPLACE TEMP TABLE spur_rust_local_calls AS
SELECT e.logical_path, e.edge_id, p.package,
       e.source_node_id, e.target_node_id, e.target_label, e.scope_text,
       left(e.receiver_text, 120) AS receiver_preview,
       e.receiver_text IS NOT NULL AS has_receiver,
       e.confidence, e.confidence_score, e.bind_method, e.edge_kind,
       src.label AS caller, src.kind AS caller_kind, src.stable_key AS caller_key,
       dst.node_id IS NOT NULL AS target_exists,
       dst.label AS target_name, dst.kind AS target_kind, dst.stable_key AS target_key,
       dst.source_span_id AS target_span_id, src.source_span_id AS caller_span_id,
       e.target_node_id IS NOT NULL AS has_local_target,
       coalesce(names.definitions, 0) AS same_file_name_definitions,
       CASE WHEN e.logical_path LIKE 'vendor/%' THEN 'vendor'
            WHEN e.logical_path LIKE '%/tests/%' THEN 'tests_directory'
            WHEN e.logical_path LIKE '%/examples/%' THEN 'examples_directory'
            WHEN e.logical_path LIKE '%/benches/%' THEN 'benches_directory'
            ELSE 'other_source_paths' END AS path_category
FROM spur_rust_snapshot.main.edges e
JOIN spur_rust_package_paths p ON p.path = e.logical_path
LEFT JOIN spur_rust_snapshot.main.nodes src ON src.logical_path = e.logical_path AND src.node_id = e.source_node_id
LEFT JOIN spur_rust_snapshot.main.nodes dst ON dst.logical_path = e.logical_path AND dst.node_id = e.target_node_id
LEFT JOIN spur_rust_name_counts names ON names.logical_path = e.logical_path
 AND names.label = regexp_extract(e.target_label, '([^:]+)$', 1)
WHERE e.relation = 'calls';

SELECT count(*) AS recorded_calls,
       count(DISTINCT (logical_path, edge_id)) AS unique_call_keys,
       count(*) FILTER (WHERE has_local_target) AS local_targets,
       count(*) FILTER (WHERE NOT has_local_target) AS unresolved,
       round(100.0 * count(*) FILTER (WHERE has_local_target) / count(*), 4) AS local_target_pct,
       count(*) FILTER (WHERE has_local_target AND NOT target_exists) AS missing_target_nodes,
       count(*) FILTER (WHERE caller IS NULL) AS missing_caller_nodes
FROM spur_rust_local_calls;

-- Cell 926f5db5-7606-4ef8-be93-cc0cf1d0ca6f
CREATE OR REPLACE TEMP VIEW spur_rust_package_rates AS
SELECT coalesce(package, '<unknown package>') AS package,
       count(*) AS recorded_calls,
       count(*) FILTER (WHERE has_local_target) AS local_targets,
       count(*) FILTER (WHERE NOT has_local_target) AS unresolved,
       round(100.0 * count(*) FILTER (WHERE has_local_target) / count(*), 4) AS local_target_pct
FROM spur_rust_local_calls GROUP BY package;
COPY (SELECT * FROM spur_rust_package_rates ORDER BY recorded_calls DESC, package)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/packages.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_package_rates ORDER BY recorded_calls DESC, package;

-- Cell a621ff2d-9444-40d6-a82d-9513cb46a47f
CREATE OR REPLACE TEMP VIEW spur_rust_binding_evidence AS
SELECT CASE WHEN has_local_target THEN 'local_target' ELSE 'unresolved' END AS population,
       coalesce(confidence, '<null>') AS confidence,
       coalesce(bind_method, '<null>') AS bind_method,
       coalesce(edge_kind, '<null>') AS edge_kind,
       count(*) AS calls, min(confidence_score) AS min_score, max(confidence_score) AS max_score
FROM spur_rust_local_calls GROUP BY ALL;
COPY (SELECT * FROM spur_rust_binding_evidence ORDER BY population, calls DESC)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/binding-evidence.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_binding_evidence ORDER BY population, calls DESC;

-- Cell 742b7b13-0e36-4fe9-b310-e37ce0fe74a3
CREATE OR REPLACE TEMP VIEW spur_rust_target_diagnostics AS
SELECT count(*) AS local_target_calls,
       count(*) FILTER (WHERE NOT target_exists) AS missing_target_nodes,
       count(*) FILTER (WHERE target_kind NOT IN ('function', 'method')) AS targets_of_other_kinds,
       count(*) FILTER (WHERE target_name != regexp_extract(target_label, '([^:]+)$', 1)) AS target_leaf_name_differs,
       count(*) FILTER (WHERE source_node_id = target_node_id) AS self_edges,
       count(*) FILTER (WHERE same_file_name_definitions > 1) AS multiple_same_name_definitions,
       count(*) FILTER (WHERE same_file_name_definitions = 1) AS one_same_name_definition,
       count(*) FILTER (WHERE same_file_name_definitions = 0) AS zero_function_method_name_definitions
FROM spur_rust_local_calls WHERE has_local_target;
COPY (SELECT * FROM spur_rust_target_diagnostics)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/target-diagnostics.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_target_diagnostics;

-- Cell 80e6594e-9540-4f90-a396-ecc9cf9268da
CREATE OR REPLACE TEMP VIEW spur_rust_top_local_targets AS
SELECT logical_path, target_node_id, target_name, target_kind, count(*) AS recorded_calls,
       count(DISTINCT source_node_id) AS distinct_caller_nodes,
       string_agg(DISTINCT coalesce(bind_method, '<null>'), ', ' ORDER BY coalesce(bind_method, '<null>')) AS binding_methods
FROM spur_rust_local_calls WHERE has_local_target
GROUP BY ALL ORDER BY recorded_calls DESC, logical_path, target_node_id LIMIT 15;
COPY (SELECT * FROM spur_rust_top_local_targets)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/top-local-targets.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_top_local_targets;

-- Cell b0621ff3-a496-469a-803a-430ba7dbf0a7
CREATE OR REPLACE TEMP VIEW spur_rust_path_rates AS
SELECT path_category, count(*) AS recorded_calls,
       count(*) FILTER (WHERE has_local_target) AS local_targets,
       count(*) FILTER (WHERE NOT has_local_target) AS unresolved,
       round(100.0 * count(*) FILTER (WHERE has_local_target) / count(*), 4) AS local_target_pct
FROM spur_rust_local_calls GROUP BY path_category;
COPY (SELECT * FROM spur_rust_path_rates ORDER BY recorded_calls DESC)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/path-categories.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_path_rates ORDER BY recorded_calls DESC;

-- Cell bf0e03c8-a719-4827-abd8-595f882e4616
CREATE OR REPLACE TEMP TABLE spur_rust_fresh_facts AS
SELECT s.logical_path, spur_graph_file_facts(s.logical_path, s.source) AS fresh_value
FROM spur_rust_snapshot.main.sources s
WHERE s.logical_path IN (
 'crates/spur-graph-duckdb/src/lib.rs',
 'crates/spur-graph-duckdb/src/file_facts.rs',
 'crates/spur-graph/src/extract/tree_sitter.rs',
 'crates/spur-acp/src/agent_model_catalog.rs',
 'crates/spur-graph/src/schema.rs',
 'crates/spur-graph/src/mcp/mod.rs'
);
CREATE OR REPLACE TEMP VIEW spur_rust_fresh_comparison AS
SELECT f.logical_path,
       len(f.fresh_value.nodes) AS nodes,
       len(f.fresh_value.edges) AS edges,
       len(f.fresh_value.spans) AS spans,
       len(f.fresh_value.macro_invocations) AS macro_invocations,
       len(f.fresh_value.macro_scopes) AS macro_scopes,
       f.fresh_value IS NOT DISTINCT FROM stored.value AS all_five_collections_match
FROM spur_rust_fresh_facts f
JOIN spur_rust_snapshot.main.facts stored USING (logical_path);
COPY (SELECT * FROM spur_rust_fresh_comparison ORDER BY logical_path)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/fresh-extraction.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_fresh_comparison ORDER BY logical_path;

-- Cell 50aabed2-ef79-44d4-a316-d829bc3756cd
CREATE OR REPLACE TEMP VIEW spur_rust_binding_shares AS
SELECT coalesce(bind_method, '<null>') AS bind_method, confidence, count(*) AS local_calls,
       round(100.0 * count(*) / sum(count(*)) OVER (), 4) AS share_of_local_targets_pct,
       round(100.0 * count(*) / (SELECT count(*) FROM spur_rust_local_calls), 4) AS share_of_all_calls_pct
FROM spur_rust_local_calls WHERE has_local_target GROUP BY ALL;
COPY (SELECT * FROM spur_rust_binding_shares ORDER BY local_calls DESC)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/binding-shares.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_binding_shares ORDER BY local_calls DESC;

-- Cell 6688dadc-1e9e-4829-8b0b-6e14fe02b01f
CREATE OR REPLACE TEMP TABLE spur_rust_sample_keys AS
WITH ranked AS (
 SELECT logical_path, edge_id, coalesce(bind_method, '<null>') AS method,
        row_number() OVER (PARTITION BY coalesce(bind_method, '<null>') ORDER BY logical_path, edge_id) AS rn
 FROM spur_rust_local_calls WHERE has_local_target
)
SELECT logical_path, edge_id, 'method:' || method AS sample_reason FROM ranked WHERE rn = 1
UNION ALL
SELECT logical_path, edge_id, 'macro_body_new' FROM (
 SELECT logical_path, edge_id FROM spur_rust_local_calls
 WHERE has_local_target AND bind_method = 'macro_body_singleton' AND target_name = 'new'
 ORDER BY logical_path, edge_id LIMIT 1
)
UNION ALL
SELECT logical_path, edge_id, 'self_edge' FROM (
 SELECT logical_path, edge_id FROM spur_rust_local_calls
 WHERE has_local_target AND source_node_id = target_node_id
 ORDER BY logical_path, edge_id LIMIT 1
);
CREATE OR REPLACE TEMP TABLE spur_rust_source_samples AS
SELECT k.sample_reason, c.logical_path, c.edge_id, c.caller, c.target_name,
       c.bind_method, c.confidence, c.scope_text, c.receiver_preview,
       c.same_file_name_definitions, ps.start_line AS caller_start_line,
       pt.start_line AS target_start_line, s.file_oid,
       substring(hex(s.source_bytes), 2 * CAST(ps.start_byte AS BIGINT) + 1,
                 2 * CAST(ps.end_byte - ps.start_byte AS BIGINT)) AS caller_source_hex,
       substring(hex(s.source_bytes), 2 * CAST(pt.start_byte AS BIGINT) + 1,
                 2 * CAST(pt.end_byte - pt.start_byte AS BIGINT)) AS target_source_hex
FROM spur_rust_sample_keys k
JOIN spur_rust_local_calls c ON c.logical_path = k.logical_path AND c.edge_id = k.edge_id
JOIN spur_rust_snapshot.main.spans ps ON ps.logical_path = c.logical_path AND ps.span_id = c.caller_span_id
JOIN spur_rust_snapshot.main.spans pt ON pt.logical_path = c.logical_path AND pt.span_id = c.target_span_id
JOIN spur_rust_snapshot.main.sources s ON s.logical_path = c.logical_path;
COPY (SELECT * FROM spur_rust_source_samples ORDER BY sample_reason, logical_path, edge_id)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/resolved-samples.json' (FORMAT JSON, ARRAY true);
SELECT sample_reason, logical_path, caller, target_name, bind_method,
       caller_start_line, target_start_line, same_file_name_definitions
FROM spur_rust_source_samples ORDER BY sample_reason;

-- Cell 67b1d3f2-2877-43dd-8c9d-6611e93cba37
CREATE OR REPLACE TEMP VIEW spur_rust_name_review AS
SELECT bind_method, target_name, count(*) AS local_target_calls
FROM spur_rust_local_calls
WHERE has_local_target AND target_name IN ('new', 'drop')
GROUP BY ALL;
COPY (SELECT * FROM spur_rust_name_review ORDER BY target_name, local_target_calls DESC)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/name-review-candidates.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_name_review ORDER BY target_name, local_target_calls DESC;

-- Cell 61fa1864-44cb-4e86-9571-ff5ce03c95d3
CREATE OR REPLACE TEMP TABLE spur_rust_collision_facts AS
SELECT spur_graph_file_facts('name-collision-control.rs', content) AS value
FROM read_text('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/name-collision-control.rs');
CREATE OR REPLACE TEMP VIEW spur_rust_collision_calls AS
WITH nodes AS (SELECT unnest(value.nodes) AS n FROM spur_rust_collision_facts),
     edges AS (SELECT unnest(value.edges) AS e FROM spur_rust_collision_facts)
SELECT src.n.label AS caller, e.e.target_label, e.e.target_node_id,
       dst.n.label AS stored_target_name, e.e.bind_method, e.e.confidence,
       e.e.scope_text, e.e.receiver_text
FROM edges e
LEFT JOIN nodes src ON src.n.node_id = e.e.source_node_id
LEFT JOIN nodes dst ON dst.n.node_id = e.e.target_node_id
WHERE e.e.relation = 'calls' AND src.n.label IN ('observed_macro', 'observed_drop', 'observed_helper');
COPY (SELECT * FROM spur_rust_collision_calls ORDER BY caller, target_label)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/name-collision-extraction.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_collision_calls ORDER BY caller, target_label;

-- Cell aa99ddfa-194d-4766-a442-b2dad8ac9f74
CREATE OR REPLACE TEMP VIEW spur_rust_collision_result AS
WITH nodes AS (SELECT unnest(value.nodes) AS n FROM spur_rust_collision_facts),
     contains_edges AS (
       SELECT e FROM (SELECT unnest(value.edges) AS e FROM spur_rust_collision_facts) WHERE e.relation = 'contains'
     )
SELECT c.caller, c.target_label, c.stored_target_name, parent.n.label AS stored_target_owner,
       c.bind_method,
       CASE c.caller
        WHEN 'observed_macro' THEN proof.function_scoped_mir_checks.observed_macro
        WHEN 'observed_drop' THEN proof.function_scoped_mir_checks.observed_drop
        WHEN 'observed_helper' THEN proof.function_scoped_mir_checks.observed_helper
       END AS compiler_target,
       CASE WHEN c.caller = 'observed_macro' AND parent.n.label = 'LocalFactory' THEN 'wrong_target'
            WHEN c.target_node_id IS NULL THEN 'unresolved'
            WHEN c.caller = 'observed_helper' AND c.stored_target_name = 'helper' THEN 'matches_control'
            ELSE 'inspect' END AS control_result
FROM spur_rust_collision_calls c
LEFT JOIN contains_edges e ON e.e.target_node_id = c.target_node_id
LEFT JOIN nodes parent ON parent.n.node_id = e.e.source_node_id
CROSS JOIN read_json('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/name-collision-compiler.json') proof
WHERE (c.caller = 'observed_macro' AND c.target_label = 'new')
   OR (c.caller = 'observed_drop' AND c.target_label = 'drop')
   OR (c.caller = 'observed_helper' AND c.target_label = 'helper');
COPY (SELECT * FROM spur_rust_collision_result ORDER BY caller)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/name-collision-result.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_collision_result ORDER BY caller;

-- Cell 6dbb871c-0e53-4c62-9f6c-25d55a625d2a
CREATE OR REPLACE TEMP VIEW spur_rust_notebook_checks AS
SELECT version() AS engine_version,
       (SELECT loaded FROM duckdb_extensions() WHERE extension_name = 'spur_graph') AS extension_loaded,
       count(*) AS recorded_calls, count(DISTINCT (logical_path, edge_id)) AS unique_call_keys,
       count(*) FILTER (WHERE has_local_target) AS local_targets,
       count(*) FILTER (WHERE NOT has_local_target) AS unresolved,
       round(100.0 * count(*) FILTER (WHERE has_local_target) / count(*), 4) AS local_target_pct,
       count(*) FILTER (WHERE has_local_target AND NOT target_exists) AS missing_target_nodes,
       (SELECT count(*) FROM spur_rust_fresh_comparison) AS fresh_files,
       (SELECT bool_and(all_five_collections_match) FROM spur_rust_fresh_comparison) AS fresh_fact_parity,
       (SELECT count(*) FROM spur_rust_collision_result WHERE control_result = 'wrong_target') AS wrong_target_controls_reproduced
FROM spur_rust_local_calls;
COPY (SELECT * FROM spur_rust_notebook_checks)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/notebook-checks.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_notebook_checks;

-- Cell b7ea321c-d285-42cb-969f-503e32b9fbbd
CREATE OR REPLACE TEMP VIEW spur_rust_caller_rates AS
SELECT logical_path, source_node_id, caller, caller_kind, count(*) AS recorded_calls,
       count(*) FILTER (WHERE has_local_target) AS local_targets,
       count(*) FILTER (WHERE NOT has_local_target) AS unresolved,
       round(100.0 * count(*) FILTER (WHERE has_local_target) / count(*), 4) AS local_target_pct
FROM spur_rust_local_calls GROUP BY ALL;
COPY (SELECT * FROM spur_rust_caller_rates ORDER BY local_targets DESC, logical_path, source_node_id LIMIT 15)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/top-callers.json' (FORMAT JSON, ARRAY true);
SELECT * FROM spur_rust_caller_rates ORDER BY local_targets DESC, logical_path, source_node_id LIMIT 15;

-- Cell ff73ba5b-1b0e-4964-83ae-c54242326dd6
CREATE OR REPLACE TEMP VIEW spur_rust_sample_owners AS
SELECT s.sample_reason, s.logical_path, s.edge_id, s.caller, s.target_name,
       owner.label AS target_owner, owner.kind AS owner_kind
FROM spur_rust_source_samples s
JOIN spur_rust_local_calls c ON c.logical_path = s.logical_path AND c.edge_id = s.edge_id
LEFT JOIN spur_rust_snapshot.main.edges e ON e.logical_path = c.logical_path
  AND e.target_node_id = c.target_node_id AND e.relation = 'contains'
LEFT JOIN spur_rust_snapshot.main.nodes owner ON owner.logical_path = e.logical_path AND owner.node_id = e.source_node_id;
COPY (SELECT * FROM spur_rust_sample_owners ORDER BY sample_reason)
TO '/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-local-targets/sample-owners.json' (FORMAT JSON, ARRAY true);
SELECT sample_reason, caller, target_owner, target_name FROM spur_rust_sample_owners ORDER BY sample_reason;
