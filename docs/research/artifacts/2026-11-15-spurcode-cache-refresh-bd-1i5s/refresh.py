"""Refresh identical frozen inputs using the shipped, versioned DuckDB workflow.

Run each phase in a fresh pinned DuckDB 2 Python process. Publication is separate
from writes/verification so the notebook only attaches the completed database.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

import duckdb

ART = Path(__file__).resolve().parent
REPO = ART.parents[3]
SNAP = ART.parent / "2026-11-15-spurcode-project-quality/snapshot.duckdb"
FIX = ART.parent / "2026-11-15-rust-macro-qualification-fix"
EXT = FIX / "build-4d32f19d4/spur_graph.duckdb_extension"
OLD_EXT = ART.parent / "2026-11-15-duckdb-file-facts/integration/spur_graph.duckdb_extension"
SQL = REPO / "crates/spur-graph-duckdb/examples/file_facts.sql"
PENDING = ART / "cache.pending.duckdb"
FINAL = ART / "cache.duckdb"
BASE = "e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028"
NEW = "be0548ce3a1217cf0b50110ad02419223c895daa8d2a932d7a81747da4c38892"
SNAP_SHA = "c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e"
REV = "61ba3d200439a541dce4c1617ca9277be7d438af"
COLLECTIONS = ("nodes", "edges", "spans", "macro_invocations", "macro_scopes")
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--phase", choices=("plan", "ingest", "replay", "verify", "publish"), required=True)
args = parser.parse_args()
receipts = []
timings = []


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(name, value):
    (ART / name).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def q(c, name, sql, params=()):
    started = time.perf_counter()
    cursor = c.execute(sql, list(params))
    columns = [d[0] for d in cursor.description]
    rows = [dict(zip(columns, row)) for row in cursor.fetchall()]
    receipts.append(dict(name=name, sql=sql, params=list(params), rows=rows,
                         seconds=time.perf_counter() - started))
    return rows


def discover(c):
    q(c, "schema", """SELECT table_catalog,table_name,column_name,data_type
       FROM information_schema.columns WHERE table_schema='main'
       ORDER BY table_catalog,table_name,ordinal_position""")


def attach(c):
    c.execute("ATTACH '" + str(SNAP).replace("'", "''") + "' AS frozen (READ_ONLY)")


def source_manifest(c, prefix):
    sql = f"""SELECT s.logical_path,s.source,s.source_bytes,s.file_oid,s.sha256,i.git_blob_oid
      FROM {prefix}.sources s JOIN {prefix}.inventory i ON i.path=s.logical_path
      ORDER BY s.logical_path"""
    rows = c.execute(sql).fetchall()
    records = []
    for path, text, data, file_oid, digest, blob in rows:
        assert text.encode("utf-8") == data, path
        assert hashlib.sha256(data).hexdigest() == digest, path
        git_digest = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        assert git_digest == blob, path
        records.append(dict(logical_path=path, git_blob_oid=blob, file_oid=file_oid,
                            sha256=digest, source_bytes=len(data)))
    assert len({r["logical_path"] for r in records}) == len(records)
    payload = "".join(json.dumps(r, ensure_ascii=False, sort_keys=True) + "\n" for r in records)
    path = ART / "source-manifest.jsonl"
    if path.exists():
        assert path.read_text() == payload, "Frozen source identity changed"
    else:
        path.write_text(payload)
    receipts.append(dict(name="source_byte_verification", sql=sql, params=[],
                         input_rows=len(rows), result_storage="source-manifest.jsonl",
                         verified_utf8_sha256_git_blob=True, manifest_sha256=sha(path)))
    return len(records)


def workflow(c):
    exports = ART / "parquet"
    exports.mkdir(exist_ok=True)
    os.chdir(exports)
    for index, statement in enumerate(c.extract_statements((ART / "workflow.sql").read_text())):
        started = time.perf_counter()
        c.execute(statement)
        rows = c.fetchall()
        timings.append(dict(index=index, sql=statement.query, seconds=time.perf_counter() - started,
                            result=rows))
        save("progress.json", dict(phase=args.phase, completed_statement=index,
                                   sql=statement.query, elapsed_seconds=timings[-1]["seconds"]))


def deltas(c):
    return q(c, "pipeline_deltas", """SELECT
       (SELECT count(*) FROM file_facts_missing) missing,
       (SELECT count(*) FROM file_facts_extracted) extracted,
       (SELECT count(*) FROM file_facts_child_refresh) child_refresh,
       (SELECT count(*) FROM file_facts_child_removed) child_removed""")[0]


def controls(c):
    rows = q(c, "controlled_calls", """WITH ns AS (SELECT unnest(value.nodes) n FROM validation_controls),
       es AS (SELECT unnest(value.edges) e FROM validation_controls)
       SELECT src.n.label caller,e.e.target_label,e.e.scope_text,e.e.target_node_id,
              dst.n.label target_name,owner.n.label target_owner,e.e.bind_method
       FROM es e JOIN ns src ON src.n.node_id=e.e.source_node_id
       LEFT JOIN ns dst ON dst.n.node_id=e.e.target_node_id
       LEFT JOIN es parent ON parent.e.relation='contains' AND parent.e.target_node_id=dst.n.node_id
       LEFT JOIN ns owner ON owner.n.node_id=parent.e.source_node_id
       WHERE e.e.relation='calls' AND src.n.label IN
        ('observed_macro','observed_helper','observed_local_macro','observed_mixed_macro')
       ORDER BY caller,e.e.target_label,e.e.scope_text NULLS FIRST""")
    selected = lambda caller, leaf: [r for r in rows if r["caller"] == caller and r["target_label"] == leaf]
    bad = selected("observed_macro", "new")
    assert bad and all(r["target_node_id"] is None and r["scope_text"] == "Vec" for r in bad)
    assert any(r["target_name"] == "helper" for r in selected("observed_helper", "helper"))
    assert any(r["target_owner"] == "LocalFactory" for r in selected("observed_local_macro", "new"))
    assert any(r["target_owner"] == "LocalFactory" for r in selected("observed_mixed_macro", "new"))
    assert any(r["target_node_id"] is None for r in selected("observed_mixed_macro", "new"))


assert duckdb.__version__ == "2.0.0"
assert sha(SNAP) == SNAP_SHA and sha(EXT) == NEW and sha(OLD_EXT) == BASE
started = time.perf_counter()
if args.phase == "publish":
    verified = json.loads((ART / "phase-verify.json").read_text())
    replayed = json.loads((ART / "phase-replay.json").read_text())
    assert replayed["all_checks_passed"] and replayed["extension_sha256"] == NEW
    assert verified["all_checks_passed"] and not FINAL.exists()
    assert sha(PENDING) == verified["database_sha256"]
    assert not Path(str(PENDING) + ".wal").exists()
    PENDING.rename(FINAL)
    with duckdb.connect(str(FINAL), read_only=True) as published:
        assert published.execute("SELECT count(*) FROM facts").fetchone()[0] == verified["admitted_sources"]
        assert published.execute("SELECT count(DISTINCT extractor_sha) FROM file_facts_cache").fetchone()[0] == 2
    save("published.json", dict(database=str(FINAL), database_sha256=sha(FINAL),
         extension_sha256=NEW, baseline_extension_sha256=BASE, snapshot_sha256=SNAP_SHA,
         snapshot_commit=REV, all_checks_passed=True))
    print(json.dumps(dict(published=str(FINAL), database_sha256=sha(FINAL))))
    raise SystemExit(0)

if args.phase == "plan":
    c = duckdb.connect(config={"allow_unsigned_extensions": "true"})
elif args.phase == "ingest":
    assert not PENDING.exists() and not FINAL.exists(), "Refuse to overwrite an existing cache"
    c = duckdb.connect(str(PENDING), config={"allow_unsigned_extensions": "true"})
else:
    assert PENDING.exists() and not FINAL.exists()
    c = duckdb.connect(str(PENDING), read_only=args.phase == "verify",
                       config={"allow_unsigned_extensions": "true"})
attach(c)
discover(c)
runtime = q(c, "runtime", """SELECT version() version,current_setting('threads') threads,
     current_setting('memory_limit') memory_limit""")[0]
if args.phase != "verify":
    c.execute("LOAD '" + str(EXT).replace("'", "''") + "'")
    loaded = q(c, "loaded_extension", """SELECT extension_name,loaded,extension_version
       FROM duckdb_extensions() WHERE extension_name='spur_graph'""")
    assert len(loaded) == 1 and loaded[0]["loaded"]
count = source_manifest(c, "frozen.main" if args.phase in ("plan", "ingest") else "main")

if args.phase == "plan":
    sql = SQL.read_text()
    assert not (ART / "workflow.sql").exists() or (ART / "workflow.sql").read_text() == sql
    (ART / "workflow.sql").write_text(sql)
    # Probe the actual pinned statement API without parsing any project source.
    statement = c.extract_statements("SELECT 1")[0]
    assert c.execute(statement).fetchone() == (1,) and statement.query.strip() == "SELECT 1"
    q(c, "inventory", "SELECT status,count(*) files,sum(bytes) bytes FROM frozen.main.inventory GROUP BY status ORDER BY status")
    q(c, "admission", "SELECT accepted,count(*) files FROM frozen.main.admission GROUP BY accepted ORDER BY accepted")
elif args.phase == "ingest":
    c.execute("BEGIN")
    for table in ("inventory", "sources", "admission"):
        c.execute(f"CREATE TABLE {table} AS SELECT * FROM frozen.main.{table}")
    c.execute("CREATE TABLE refresh_meta AS SELECT ?::VARCHAR extractor_sha,?::VARCHAR baseline_sha,?::VARCHAR snapshot_commit,?::VARCHAR snapshot_sha256,?::VARCHAR source_manifest_sha256",
              [NEW, BASE, REV, SNAP_SHA, sha(ART / "source-manifest.jsonl")])
    c.execute("""CREATE TABLE file_facts_cache AS SELECT i.git_blob_oid blob_oid,f.logical_path,
      ?::VARCHAR extractor_sha,f.value facts,
      CASE WHEN s.source IS NULL THEN 'null_input' WHEN f.value IS NULL THEN 'extraction_error'
           WHEN s.source='' THEN 'empty_source'
           WHEN len(f.value.nodes)+len(f.value.edges)+len(f.value.spans)+len(f.value.macro_invocations)+len(f.value.macro_scopes)=0 THEN 'empty_facts'
           ELSE 'ok' END status
      FROM frozen.main.facts f JOIN sources s USING(logical_path) JOIN inventory i ON i.path=f.logical_path""", [BASE])
    c.execute("""CREATE VIEW staged_sources AS SELECT i.git_blob_oid blob_oid,s.logical_path,s.source,m.extractor_sha
       FROM sources s JOIN inventory i ON i.path=s.logical_path CROSS JOIN refresh_meta m""")
    c.execute("""CREATE TABLE source_lineage AS SELECT i.git_blob_oid blob_oid,s.logical_path,v.extractor_sha,
       'SpurCode' repository,?::VARCHAR snapshot_commit,v.role,s.sha256 source_sha256
       FROM sources s JOIN inventory i ON i.path=s.logical_path
       CROSS JOIN (VALUES (?, 'frozen_baseline'), (?, 'corrected')) v(extractor_sha,role)""", [REV, BASE, NEW])
    c.execute("COMMIT")
    workflow(c)
    delta = deltas(c)
    assert delta == dict(missing=count, extracted=count, child_refresh=2 * count, child_removed=0), delta
    c.execute("CREATE VIEW facts AS SELECT logical_path,facts value FROM file_facts_cache WHERE extractor_sha='" + NEW + "'")
    for collection in COLLECTIONS:
        c.execute(f"CREATE VIEW {collection} AS SELECT * EXCLUDE(blob_oid,extractor_sha,fact_ordinal) FROM file_facts_{collection} WHERE extractor_sha='{NEW}'")
    control = (FIX / "control.rs").read_text()
    c.execute("CREATE TABLE validation_controls AS SELECT ?::VARCHAR logical_path,spur_graph_file_facts(?,?) value", ["control.rs", "control.rs", control])
    controls(c)
elif args.phase == "replay":
    children_sql = """SELECT table_name,table_oid FROM duckdb_tables()
       WHERE database_name=current_database() AND schema_name='main'
        AND table_name IN ('file_facts_child_state','file_facts_nodes','file_facts_edges','file_facts_spans','file_facts_macro_invocations','file_facts_macro_scopes') ORDER BY table_name"""
    before = q(c, "children_before_replay", children_sql)
    workflow(c)
    assert deltas(c) == dict(missing=0, extracted=0, child_refresh=0, child_removed=0)
    after = q(c, "children_after_replay", children_sql)
    assert before == after
else:
    assert c.execute("SELECT count(*) FROM duckdb_extensions() WHERE extension_name='spur_graph' AND loaded").fetchone()[0] == 0
    for table in ("inventory", "sources", "admission"):
        diff = q(c, "input_equality_" + table, f"""WITH missing AS (SELECT * FROM frozen.main.{table} EXCEPT ALL SELECT * FROM main.{table}),
          extra AS (SELECT * FROM main.{table} EXCEPT ALL SELECT * FROM frozen.main.{table})
          SELECT (SELECT count(*) FROM missing) missing,(SELECT count(*) FROM extra) extra""")[0]
        assert diff == dict(missing=0, extra=0), (table, diff)
    versions = q(c, "cache_versions", """SELECT extractor_sha,status,count(*) files FROM file_facts_cache GROUP BY ALL ORDER BY extractor_sha,status""")
    assert c.execute("SELECT count(*) FROM file_facts_cache").fetchone()[0] == 2 * count
    assert c.execute("SELECT count(*) FROM file_facts_cache WHERE status IN ('extraction_error','null_input')").fetchone()[0] == 0
    old = q(c, "original_facts_unchanged", """SELECT count(*) rows,count(*) FILTER(WHERE c.facts IS NOT DISTINCT FROM f.value) identical
      FROM file_facts_cache c JOIN frozen.main.facts f USING(logical_path) WHERE c.extractor_sha=?""", [BASE])[0]
    assert old == dict(rows=count, identical=count)
    q(c, "cache_state", "SELECT (SELECT count(*) FROM file_facts_child_state) checkpoint,(SELECT count(*) FROM file_facts_occurrences) occurrences")
    for collection in COLLECTIONS:
        diff = q(c, "child_parity_" + collection, f"""WITH expected AS (
          SELECT blob_oid,logical_path,extractor_sha,fact_ordinal,item.* FROM file_facts_cache,
            UNNEST(facts.{collection}) WITH ORDINALITY child(item,fact_ordinal)),
          missing AS (SELECT * FROM expected EXCEPT ALL SELECT * FROM file_facts_{collection}),
          extra AS (SELECT * FROM file_facts_{collection} EXCEPT ALL SELECT * FROM expected)
          SELECT (SELECT count(*) FROM missing) missing,(SELECT count(*) FROM extra) extra""")[0]
        assert diff == dict(missing=0, extra=0), (collection, diff)
    comparison = q(c, "per_file_comparison", """SELECT n.logical_path,i.suffix,
       n.facts.nodes IS NOT DISTINCT FROM o.facts.nodes nodes_same,
       n.facts.spans IS NOT DISTINCT FROM o.facts.spans spans_same,
       n.facts.macro_invocations IS NOT DISTINCT FROM o.facts.macro_invocations invocations_same,
       n.facts.macro_scopes IS NOT DISTINCT FROM o.facts.macro_scopes scopes_same,
       n.facts.edges IS NOT DISTINCT FROM o.facts.edges edges_same
       FROM file_facts_cache n JOIN file_facts_cache o ON o.blob_oid=n.blob_oid AND o.logical_path=n.logical_path
       JOIN inventory i ON i.path=n.logical_path WHERE n.extractor_sha=? AND o.extractor_sha=? ORDER BY n.logical_path""", [NEW, BASE])
    assert len(comparison) == count
    rust = [r for r in comparison if r["suffix"] == ".rs"]
    assert rust and all(all(r[k] for k in ("nodes_same", "spans_same", "invocations_same", "scopes_same")) for r in rust)
    q(c, "rust_calls_by_version", """SELECT e.extractor_sha,count(*) calls,
       count(*) FILTER(WHERE e.target_node_id IS NOT NULL) local_targets,
       count(*) FILTER(WHERE e.target_node_id IS NULL) unresolved,
       100.0*count(*) FILTER(WHERE e.target_node_id IS NOT NULL)/count(*) local_target_pct
       FROM file_facts_edges e JOIN inventory i ON i.path=e.logical_path
       WHERE i.suffix='.rs' AND e.relation='calls' GROUP BY e.extractor_sha ORDER BY e.extractor_sha""")
    q(c, "rust_bind_methods", """SELECT e.extractor_sha,e.bind_method,count(*) calls
       FROM file_facts_edges e JOIN inventory i ON i.path=e.logical_path WHERE i.suffix='.rs' AND e.relation='calls'
       GROUP BY ALL ORDER BY e.extractor_sha,calls DESC,e.bind_method NULLS LAST""")
    real = q(c, "real_vec_control", """SELECT src.label caller,e.target_label,e.scope_text,e.target_node_id,e.bind_method
       FROM edges e JOIN nodes src ON src.logical_path=e.logical_path AND src.node_id=e.source_node_id
       WHERE e.logical_path='crates/spur-acp/src/capability_evidence.rs' AND e.relation='calls'
        AND src.label='rejected_evidence_demotes_while_inconclusive_evidence_preserves_prior_routes' AND e.target_label='new'""")
    assert real and all(r["scope_text"] == "Vec" and r["target_node_id"] is None for r in real)
    controls(c)
    exports = []
    for name in ("cache", *COLLECTIONS, "occurrences"):
        path = ART / "parquet" / f"file-facts-{name}.parquet"
        table = "file_facts_" + name
        result = c.execute("SELECT count(*) FROM read_parquet(?)", [str(path)]).fetchone()[0]
        assert result == c.execute("SELECT count(*) FROM " + table).fetchone()[0]
        table_schema = [(r[0], r[1]) for r in c.execute("DESCRIBE SELECT * FROM " + table).fetchall()]
        export_schema = [(r[0], r[1]) for r in c.execute("DESCRIBE SELECT * FROM read_parquet(?)", [str(path)]).fetchall()]
        assert table_schema == export_schema, table
        exports.append(dict(path=str(path), rows=result, schema=export_schema,
                            bytes=path.stat().st_size, sha256=sha(path)))
    save("parquet-manifest.json", exports)

if args.phase in ("ingest", "replay"):
    c.execute("CHECKPOINT")
c.close()
assert sha(SNAP) == SNAP_SHA and sha(EXT) == NEW and sha(OLD_EXT) == BASE
report = dict(phase=args.phase, all_checks_passed=True, runtime=runtime,
              pid=os.getpid(), python=sys.executable, script_sha256=sha(__file__),
              workflow_sha256=sha(ART / "workflow.sql"), extension_sha256=NEW,
              baseline_extension_sha256=BASE, snapshot_sha256=SNAP_SHA,
              snapshot_commit=REV, admitted_sources=count, seconds=time.perf_counter() - started,
              receipts=receipts, workflow_statements=timings)
if args.phase == "verify":
    reader = REPO / "crates/spur-graph-duckdb/examples/project_quality.py"
    spec = importlib.util.spec_from_file_location("cache_quality_reader", reader)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    for name, path in (("baseline", SNAP), ("corrected", PENDING)):
        audit = module.audit_database(path)
        save("structural-" + name + ".json", audit)
    report["structural_reader_contract_note"] = (
        "Unmodified reader reports legacy baseline extension hash as a contract reference, "
        "not verified ingestion provenance. This phase and refresh_meta authenticate the corrected artifact.")
    report["database_sha256"] = sha(PENDING)
report["seconds"] = time.perf_counter() - started
save("phase-" + args.phase + ".json", report)
print(json.dumps(dict(phase=args.phase, all_checks_passed=True, sources=count,
                     seconds=report["seconds"], database=str(PENDING) if args.phase != "plan" else None)))
