-- Executed recipes from the DuckDB 2.0 notebook.
-- Local paths refer to this workspace. Installed artifact SHA256 values are in extension-runtime.json.
LOAD '/Users/kevintruong/project/SpurCode/.spur/artifacts/duck-tails-pr28731/duck_tails.duckdb_extension';
LOAD '/Users/kevintruong/project/SpurCode/.spur/artifacts/explore-audit-bd-2bhj-native/spur_graph.duckdb_extension';

SELECT version(), spur_graph_info();

DESCRIBE SELECT * FROM git_diff_tree(
 '/Users/kevintruong/project/duckdb',
 '629e395a854d72dcb1f414e1a0451c68ac0e9f1f',
 'd29b26bb92ac940a0807be0f43cc385b31623839');

SELECT file_path,status,old_path FROM git_diff_tree(
 '/Users/kevintruong/project/duckdb',
 '629e395a854d72dcb1f414e1a0451c68ac0e9f1f',
 'd29b26bb92ac940a0807be0f43cc385b31623839') ORDER BY file_path;

-- Resolve a canonical revision-pinned URI; the Python cell binds this result to git_log(?).
SELECT git_uri('/Users/kevintruong/project/duckdb',
 'src/optimizer/relation_statistics/relation_statistics_helper.cpp',
 '629e395a854d72dcb1f414e1a0451c68ac0e9f1f');

SELECT commit_hash,author_name,author_date,commit_date,message
FROM git_log('git:///Users/kevintruong/project/duckdb/src/optimizer/relation_statistics/relation_statistics_helper.cpp?ref=629e395a854d72dcb1f414e1a0451c68ac0e9f1f')
WHERE author_date >= TIMESTAMP '2026-05-21'
ORDER BY author_date DESC LIMIT 40;

-- Explicit base-revision semantic region; no author-email publication.
SELECT author_name,count(*) AS attributed_lines,max(author_date) AS latest_attributed_commit
FROM git_blame('src/optimizer/relation_statistics/relation_statistics_helper.cpp',
 repo_path:='/Users/kevintruong/project/duckdb',
 revision:='629e395a854d72dcb1f414e1a0451c68ac0e9f1f',
 min_line:=450,max_line:=565,ignore_whitespace:=true,use_mailmap:=true)
GROUP BY author_name ORDER BY attributed_lines DESC;

-- The notebook repeats history/blame for first_last_any.cpp (blame lines 19..60).
-- Spur Graph source extraction binds exact UTF-8 git-show bytes:
-- SELECT spur_graph_symbols(?, ?);
-- SELECT spur_graph_file_facts_result(?, ?);
-- It preserves source SHA256/Git blob identity in source-manifest.json.
-- Full notebook source, executed SQL views and evidence receipts accompany this file.
