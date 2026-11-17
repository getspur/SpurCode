"""Index the complete pinned repository's C++ inventory through the native SQL API.

This is a research consumer, not a replacement C++ resolver. All file-local IDs
retain their snapshot/path/blob/extractor context. Candidate names never become
confirmed edges. Run from a DuckDB 2.0 Python environment.
"""
import argparse
import hashlib
import itertools
import json
from pathlib import Path, PurePosixPath
import subprocess
import time

import duckdb


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, default=str) + "\n")
    temporary.replace(path)


def inventory(repo, revision, snapshot):
    output = subprocess.check_output(
        ["git", "-C", str(repo), "ls-tree", "-r", "-l", "-z", revision]
    )
    # Includes all native C++ extensions plus neighboring C++ source forms so
    # unsupported forms are measured explicitly instead of silently omitted.
    extensions = {"cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "tpp", "h", "inl", "cu", "cuh"}
    rows = []
    for entry in output.split(b"\0"):
        if not entry:
            continue
        metadata, path = entry.split(b"\t", 1)
        mode, kind, oid, length = metadata.decode().split()
        path = path.decode("utf-8", errors="strict")
        rows.append({
            "snapshot": snapshot, "revision": revision, "logical_path": path,
            "blob_oid": oid, "mode": mode, "git_kind": kind,
            "source_bytes": int(length) if length != "-" else 0,
            "cpp_scope": kind == "blob" and PurePosixPath(path).suffix.lower().lstrip(".") in extensions,
        })
    return rows


def rows(con, query, args=None):
    result = con.execute(query, args or [])
    names = [column[0] for column in result.description]
    return [dict(zip(names, row)) for row in result.fetchall()]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    assert duckdb.__version__ == "2.0.0", duckdb.__version__
    args.out.mkdir(parents=True, exist_ok=True)
    package_sha = hashlib.sha256(args.package.read_bytes()).hexdigest()
    con = duckdb.connect(str(args.out / "repository.duckdb"), config={"allow_unsigned_extensions": True})
    con.execute("LOAD '" + str(args.package).replace("'", "''") + "'")
    info = con.execute("SELECT spur_graph_info()").fetchone()[0]
    assert info["resolution_scope"] == "single_file", info
    extractor_id = info["extractor_id"]
    identities = {"base": args.base, "head": args.head, "package_sha256": package_sha, "info": info}
    previous = args.out / "identities.json"
    if previous.exists():
        assert json.loads(previous.read_text()) == identities, "output belongs to a different probe"
    save(previous, identities)
    manifest = inventory(args.repo, args.head, "head") + inventory(args.repo, args.base, "base")
    save(args.out / "repository-manifest.json", manifest)
    con.execute("CREATE OR REPLACE TABLE manifest AS SELECT * FROM read_json(?)", [str(args.out / "repository-manifest.json")])
    con.execute("""CREATE TABLE IF NOT EXISTS extraction_cache AS
        SELECT ''::VARCHAR logical_path, ''::VARCHAR blob_oid, ''::VARCHAR extractor_id,
          ''::VARCHAR source_sha256, 0.0::DOUBLE elapsed_seconds,
          NULL::VARCHAR consumer_error,
          spur_graph_file_facts_result('seed.cpp','') AS result WHERE false""")
    pending = rows(con, """SELECT DISTINCT m.logical_path, m.blob_oid, m.source_bytes
        FROM manifest m WHERE m.cpp_scope AND NOT EXISTS (
          SELECT 1 FROM extraction_cache c WHERE c.logical_path=m.logical_path
            AND c.blob_oid=m.blob_oid AND c.extractor_id=?)
        ORDER BY m.source_bytes, m.logical_path""", [extractor_id])
    started = time.monotonic()
    reader = subprocess.Popen(["git", "-C", str(args.repo), "cat-file", "--batch"],
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    completed = 0
    batch_receipts = []
    # One query per source directory amortizes the measured per-query parser
    # initialization while retaining a natural, replayable checkpoint unit.
    grouped = itertools.groupby(sorted(pending, key=lambda r: (str(PurePosixPath(r["logical_path"]).parent), r["logical_path"])),
                                key=lambda r: str(PurePosixPath(r["logical_path"]).parent))
    try:
        for directory, group in grouped:
            batch = []
            before = time.monotonic()
            for item in group:
                reader.stdin.write((item["blob_oid"] + "\n").encode())
                reader.stdin.flush()
                oid, kind, length = reader.stdout.readline().decode().strip().split()
                assert oid == item["blob_oid"] and kind == "blob"
                data = reader.stdout.read(int(length))
                assert len(data) == int(length) and reader.stdout.read(1) == b"\n"
                sha = hashlib.sha256(data).hexdigest()
                try:
                    source = data.decode("utf-8", errors="strict")
                except UnicodeDecodeError as exc:
                    con.execute("INSERT INTO extraction_cache VALUES (?,?,?,?,0,?,NULL)",
                                [item["logical_path"], item["blob_oid"], extractor_id, sha, "invalid_utf8: " + str(exc)])
                    completed += 1
                    continue
                batch.append(dict(item, source=source, sha=sha))
            if batch:
                con.execute("""INSERT INTO extraction_cache
                    SELECT logical_path,blob_oid,?,source_sha256,0,NULL,
                      spur_graph_file_facts_result(logical_path,source)
                    FROM (SELECT unnest(?) logical_path,unnest(?) blob_oid,
                                 unnest(?) source_sha256,unnest(?) source)""",
                    [extractor_id, [r["logical_path"] for r in batch], [r["blob_oid"] for r in batch],
                     [r["sha"] for r in batch], [r["source"] for r in batch]])
                completed += len(batch)
            batch_receipts.append({"directory": directory, "files": len(batch),
                "source_bytes": sum(r["source_bytes"] for r in batch),
                "elapsed_seconds": time.monotonic()-before})
            save(args.out / "batch-receipts.json", batch_receipts)
            save(args.out / "progress.json", {"state": "indexing", "completed_this_run": completed,
                "pending_at_start": len(pending), "last_directory": directory,
                "elapsed_seconds": time.monotonic() - started})
    finally:
        reader.stdin.close()
        reader.wait(timeout=10)
    con.execute("""CREATE OR REPLACE VIEW indexed_sources AS
        SELECT m.*, c.extractor_id, c.source_sha256, c.consumer_error, c.result, c.elapsed_seconds
        FROM manifest m JOIN extraction_cache c USING(logical_path,blob_oid) WHERE m.cpp_scope""")
    con.execute("""CREATE OR REPLACE TABLE nodes AS SELECT snapshot, revision, logical_path,
        blob_oid, extractor_id, n.* FROM indexed_sources, unnest(result.facts.nodes) AS t(n)""")
    con.execute("""CREATE OR REPLACE TABLE edges AS SELECT snapshot, revision, logical_path,
        blob_oid, extractor_id, e.* FROM indexed_sources, unnest(result.facts.edges) AS t(e)""")
    con.execute("""CREATE OR REPLACE TABLE callable_name_inventory AS
        SELECT snapshot,label,count(*) raw_candidate_nodes,count(DISTINCT logical_path) candidate_files
        FROM nodes WHERE kind IN ('function','method') GROUP BY snapshot,label""")
    totals = rows(con, """SELECT snapshot,count(*) tracked_entries,
        count(*) FILTER (WHERE cpp_scope) cpp_inventory_files,
        sum(source_bytes) FILTER (WHERE cpp_scope) cpp_inventory_bytes FROM manifest GROUP BY snapshot""")
    statuses = rows(con, """SELECT snapshot,result.status status,result.error_code error_code,
        consumer_error,count(*) files FROM indexed_sources GROUP BY ALL ORDER BY snapshot,files DESC""")
    coverage = rows(con, """SELECT snapshot,relation,count(*) facts,
        count(*) FILTER (WHERE target_node_id IS NOT NULL) bound,
        count(*) FILTER (WHERE target_node_id IS NULL) unresolved FROM edges GROUP BY snapshot,relation""")
    missing_endpoints = con.execute("""SELECT count(*) FROM edges e LEFT JOIN nodes n
        ON e.snapshot=n.snapshot AND e.logical_path=n.logical_path AND e.blob_oid=n.blob_oid
        AND e.extractor_id=n.extractor_id AND e.target_node_id=n.node_id
        WHERE e.target_node_id IS NOT NULL AND n.node_id IS NULL""").fetchone()[0]
    assert missing_endpoints == 0, "file-local target escaped its source context"
    changed = ["src/function/aggregate/distributive/first_last_any.cpp",
               "src/optimizer/relation_statistics/relation_statistics_helper.cpp"]
    changed_counts = rows(con, """SELECT snapshot,count(*) call_facts,
        count(*) FILTER (WHERE target_node_id IS NOT NULL) bound,
        count(*) FILTER (WHERE target_node_id IS NULL) unresolved FROM edges
        WHERE logical_path IN (?,?) AND relation='calls' GROUP BY snapshot""", changed)
    assert next(row for row in changed_counts if row["snapshot"] == "head") == {
        "snapshot": "head", "call_facts": 366, "bound": 17, "unresolved": 349}
    candidates = rows(con, """SELECT e.snapshot,e.logical_path,e.edge_id,e.target_label,
        e.receiver_text,e.scope_text,coalesce(n.raw_candidate_nodes,0) raw_same_name_candidates,
        coalesce(n.candidate_files,0) candidate_files
        FROM edges e LEFT JOIN callable_name_inventory n ON e.snapshot=n.snapshot AND e.target_label=n.label
        WHERE e.snapshot='head' AND e.logical_path IN (?,?) AND e.relation='calls' AND e.target_node_id IS NULL""", changed)
    save(args.out / "changed-unresolved-name-inventory.json", candidates)
    assert len(candidates) == 349
    failures = sum(row["files"] for row in statuses if row["consumer_error"] or row["status"] not in ("ok", "empty_source"))
    summary = {"state": "completed" if failures == 0 else "completed_with_diagnostics",
        "scope": "All tracked C++/header forms in both immutable repository snapshots, including third_party, tests, tools and extensions",
        "whole_repository_manifest": totals, "statuses": statuses, "edge_coverage": coverage,
        "changed_file_calls": changed_counts, "missing_composite_key_endpoints": missing_endpoints,
        "confirmed_cross_file_calls": 0, "cross_file_note": "The native result has single-file scope; same-name inventories are not linked edges",
        "name_only_candidates": {key: sum(predicate(row) for row in candidates) for key, predicate in {
            "zero": lambda row: row["raw_same_name_candidates"] == 0,
            "one": lambda row: row["raw_same_name_candidates"] == 1,
            "many": lambda row: row["raw_same_name_candidates"] > 1}.items()},
        "elapsed_seconds_this_run": time.monotonic() - started, "identities": identities}
    save(args.out / "summary.json", summary)
    save(args.out / "progress.json", {"state": summary["state"], "completed_this_run": len(pending)})
    con.execute("CHECKPOINT")
    con.close()
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
