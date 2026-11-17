-- SQL transcript, not a standalone driver. Interleave with the Python cells in notebook order.
-- Requires the same managed DuckDB 2.0 connection and saved GitHub connection setup.

-- Notebook cell 21d2fbb3-e881-4620-bd3d-28646b16fed3
-- Minimal DuckPGQ smoke test. This is a synthetic grammar/engine fixture.
CREATE OR REPLACE TABLE pgq_probe_nodes (id BIGINT PRIMARY KEY, name VARCHAR);
INSERT INTO pgq_probe_nodes VALUES (1,'changed'),(2,'caller'),(3,'consumer');
CREATE OR REPLACE TABLE pgq_probe_edges (src BIGINT,dst BIGINT);
INSERT INTO pgq_probe_edges VALUES (1,2),(2,3);
CREATE OR REPLACE PROPERTY GRAPH pgq_probe
  VERTEX TABLES (pgq_probe_nodes LABEL symbol)
  EDGE TABLES (pgq_probe_edges
    SOURCE KEY (src) REFERENCES pgq_probe_nodes (id)
    DESTINATION KEY (dst) REFERENCES pgq_probe_nodes (id)
    LABEL relates);
SELECT * FROM GRAPH_TABLE(pgq_probe
  MATCH p = ANY SHORTEST (a:symbol)-[e:relates]->{1,3}(b:symbol)
  WHERE a.id = 1
  COLUMNS (a.name AS source_name,b.name AS target_name,path_length(p) AS hops))
ORDER BY hops,target_name;

-- Notebook cell 0a163789-4e20-4732-b16f-95b0f3abe56a
-- Read-only, tightly scoped inbox query through the saved @github-pat connection.
SELECT number,title,state,user_login,html_url,updated_at
FROM github_search_issues(
  "q" = 'repo:duckdb/duckdb is:pr 28731',
  "sort" = 'updated',
  "order" = 'desc')
ORDER BY updated_at DESC LIMIT 5;

-- Notebook cell 6d34cc21-304b-44cf-8011-cb3c435bb531
-- A narrow real inbox slice. Search limits are not a claim of a complete repository inbox.
CREATE OR REPLACE TABLE github_poc_pr_inbox AS
SELECT number,title,state,user_login,html_url,updated_at
FROM github_search_issues("q" = 'repo:duckdb/duckdb is:pr 28731',"sort"='updated',"order"='desc');
CREATE OR REPLACE TABLE github_poc_issue_inbox AS
SELECT number,title,state,user_login,html_url,updated_at
FROM github_search_issues("q" = 'repo:duckdb/duckdb is:issue 28615',"sort"='updated',"order"='desc');
SELECT 'pull_request' AS item_type,* FROM github_poc_pr_inbox
UNION ALL
SELECT 'issue' AS item_type,* FROM github_poc_issue_inbox
ORDER BY item_type,number;

-- Notebook cell 642d1bfe-9085-4a55-8004-7cab15e2d3e3
-- Real property graph from the native Spur Graph fact rows.
-- Calls are directed caller -> callee. Reverse impact asks which callers reach a changed callee.
ALTER TABLE native_nodes ADD PRIMARY KEY (id);
CREATE OR REPLACE PROPERTY GRAPH native_pr_graph
  VERTEX TABLES (native_nodes LABEL symbol)
  EDGE TABLES (native_calls
    SOURCE KEY (src) REFERENCES native_nodes (id)
    DESTINATION KEY (dst) REFERENCES native_nodes (id)
    LABEL calls);

CREATE OR REPLACE TABLE native_blast_radius AS
SELECT * FROM GRAPH_TABLE(native_pr_graph
  MATCH p = ANY SHORTEST (caller:symbol)-[e:calls]->{1,2}(changed:symbol)
  WHERE changed.changed = true AND caller.id <> changed.id
  COLUMNS (caller.id AS caller_id, caller.label AS caller_name,
           caller.start_line AS caller_line, caller.component AS component,
           changed.id AS changed_id,changed.label AS changed_name,
           path_length(p) AS hops))
ORDER BY changed_name,hops,caller_line;

SELECT caller_name,caller_line,changed_name,hops,component
FROM native_blast_radius ORDER BY changed_name,hops,caller_line;

-- Notebook cell 5d3cd78f-66f1-4bbe-95dd-f6f5448a179b
-- Separate curated propagation graph, preserving its evidence origin.
CREATE OR REPLACE PROPERTY GRAPH curated_pr_graph
  VERTEX TABLES (curated_nodes LABEL source_point)
  EDGE TABLES (curated_edges
    SOURCE KEY (from_node) REFERENCES curated_nodes (id)
    DESTINATION KEY (to_node) REFERENCES curated_nodes (id)
    LABEL propagates);
CREATE OR REPLACE TABLE curated_propagation AS
SELECT * FROM GRAPH_TABLE(curated_pr_graph
  MATCH p = ANY SHORTEST (a:source_point)-[e:propagates]->{1,6}(b:source_point)
  WHERE a.id='ordered' AND b.id IN ('manager','estimator','cost')
  COLUMNS (a.statement AS changed_behavior,b.id AS downstream_id,
           b.component AS downstream_component,b.statement AS downstream_behavior,
           b.url AS source_url,path_length(p) AS hops))
ORDER BY hops;
SELECT downstream_id,downstream_component,hops,source_url FROM curated_propagation ORDER BY hops;

-- Notebook cell fb442ea8-61be-4e65-a707-611854b9e974
-- Component membership is an explicit POC path rule, not an approved architecture policy.
CREATE OR REPLACE TABLE component_rollup AS
SELECT n.component,count(*) AS normalized_vertices,
       count(*) FILTER (WHERE n.changed) AS directly_changed_symbols,
       count(DISTINCT b.caller_id) AS caller_vertices_in_limited_graph,
       'POC path mapping' AS mapping_source,
       'not_evaluated_no_architecture_policy' AS alignment_state
FROM native_nodes n LEFT JOIN
 (SELECT DISTINCT caller_id FROM native_blast_radius) b ON b.caller_id=n.id
GROUP BY n.component;
SELECT component,directly_changed_symbols,caller_vertices_in_limited_graph,alignment_state
FROM component_rollup ORDER BY component;

-- Notebook cell bf780887-c6dd-4c3f-b135-d15913bf0456
-- Provenance-aware people view: current participants and historical candidates have separate roles.
SELECT user_login AS person,'PR author (live inbox)' AS role,
       title AS evidence,NULL::BIGINT AS base_region_lines
FROM github_poc_pr_inbox
UNION ALL
SELECT user_login,'issue reporter (live inbox)',title,NULL::BIGINT FROM github_poc_issue_inbox
UNION ALL
SELECT github_login,'historical expertise candidate',file_path,base_region_lines
FROM stakeholder_candidates WHERE github_login IS NOT NULL
ORDER BY role,person;

-- Notebook cell 5917d06b-d070-4b50-a89a-8490c511fc96
-- End-to-end join: @github-pat inbox + Spur Graph changes + DuckPGQ paths + DuckTails expertise.
CREATE OR REPLACE TABLE review_poc_join AS
SELECT p.number AS pr_number,n.label AS changed_symbol,n.component,
       count(DISTINCT b.caller_id) AS callers_found_in_limited_graph,
       s.github_login AS historical_expertise_candidate,s.base_region_lines,
       'partial_single_file' AS impact_coverage,
       'not_evaluated_no_policy' AS architecture_alignment
FROM github_poc_pr_inbox p
JOIN native_nodes n ON n.changed
LEFT JOIN native_blast_radius b ON b.changed_id=n.id
LEFT JOIN stakeholder_candidates s ON s.file_path=n.file_path AND s.github_login IS NOT NULL
WHERE p.number=28731
GROUP BY p.number,n.id,n.label,n.component,s.github_login,s.base_region_lines;
SELECT changed_symbol,callers_found_in_limited_graph,historical_expertise_candidate,
       impact_coverage,architecture_alignment
FROM review_poc_join ORDER BY changed_symbol;
