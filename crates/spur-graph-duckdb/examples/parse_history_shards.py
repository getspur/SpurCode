"""Parse bounded history batches into immutable native LIST/STRUCT Parquet shards.

Usage: parse_history_shards.py --repo REPO --plan PLAN.json --extension FILE --output DIR
Each generation is OUTPUT/generations/EXTENSION_SHA256/PLAN_SHA256. Its batches
have immutable Parquet/receipt pairs; progress and result JSON describe the latest
invocation. Orphaned final files are errors, and partial temporary files are kept.
Only metadata survives between batches. Source payloads and native results live
in a fresh, non-spilling, in-memory DuckDB connection for each missing batch.
"""

import argparse
from collections import Counter
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import subprocess
import sys
import time
import uuid

import duckdb


TERMINAL = ("completed", "binary_skipped", "encoding_error", "extension_error")
OID = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def atomic_json(path, value, immutable=False):
    if immutable and os.path.lexists(path):
        raise ValueError(f"refusing to overwrite immutable receipt: {path}")
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    with temporary.open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    temporary.rename(path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def positive_integer(value, name, allow_zero=False):
    if type(value) is not int or value < (0 if allow_zero else 1):
        raise ValueError(f"{name} must be a {'nonnegative' if allow_zero else 'positive'} integer")
    return value


def validate_plan(plan):
    if not isinstance(plan.get("head"), str) or not OID.fullmatch(plan["head"]):
        raise ValueError("plan head must be a full lowercase hexadecimal commit OID")
    row_limit = positive_integer(plan.get("row_limit"), "row_limit")
    byte_budget = positive_integer(plan.get("byte_budget"), "byte_budget", allow_zero=True)
    if not isinstance(plan.get("batches"), list):
        raise ValueError("plan batches must be a list")
    batch_ids, keys, sizes = set(), set(), {}
    for batch in plan["batches"]:
        batch_id = positive_integer(batch.get("batch_id"), "batch_id", allow_zero=True)
        if batch_id in batch_ids:
            raise ValueError(f"duplicate batch_id: {batch_id}")
        batch_ids.add(batch_id)
        tasks = batch.get("tasks")
        if not isinstance(tasks, list) or len(tasks) > row_limit:
            raise ValueError(f"batch {batch_id} exceeds plan row limit or has invalid tasks")
        for task in tasks:
            oid, language, path = (task.get(field) for field in ("blob_oid", "language", "path"))
            if not isinstance(oid, str) or not OID.fullmatch(oid):
                raise ValueError(f"batch {batch_id}: invalid blob_oid")
            if not isinstance(language, str) or not language or not isinstance(path, str) or not path or "\x00" in path:
                raise ValueError(f"batch {batch_id}: language and logical path must be nonempty strings")
            size = positive_integer(task.get("byte_length"), "byte_length", allow_zero=True)
            if size > (1 << 64) - 1:
                raise ValueError("byte_length exceeds the Parquet UBIGINT domain")
            if oid in sizes and sizes[oid] != size:
                raise ValueError(f"conflicting byte_length for blob {oid}")
            sizes[oid] = size
            key = (oid, language)
            if key in keys:
                raise ValueError(f"duplicate parse key: {key}")
            keys.add(key)
        if sum(task["byte_length"] for task in tasks) > byte_budget:
            raise ValueError(f"batch {batch_id} exceeds plan byte budget")
    return len(keys)


@contextmanager
def memory_connection(extension=None):
    # An empty temp_directory disables spilling rather than retaining source
    # payloads in DuckDB's default on-disk temporary directory.
    connection = duckdb.connect(":memory:", config={"allow_unsigned_extensions": True, "temp_directory": ""})
    try:
        if extension is not None:
            connection.execute("LOAD " + quote(extension))
        yield connection
    finally:
        connection.close()


class GitObjects:
    """One lazy, synchronous cat-file reader; requests never span a batch."""

    def __init__(self, repo):
        self.repo = repo
        self.child = None
        self.blobs_read = 0
        self.bytes_read = 0

    def read(self, oid, expected_size):
        if self.child is None:
            self.child = subprocess.Popen(
                ["git", "-C", str(self.repo), "cat-file", "--batch"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
        self.child.stdin.write((oid + "\n").encode("ascii"))
        self.child.stdin.flush()
        header = self.child.stdout.readline().decode("ascii").split()
        if len(header) != 3 or header[:2] != [oid, "blob"]:
            raise ValueError(f"Git blob header mismatch for {oid}: {header}")
        if int(header[2]) != expected_size:
            raise ValueError(f"Git blob byte_length mismatch for {oid}")
        payload = self.child.stdout.read(expected_size)
        if len(payload) != expected_size or self.child.stdout.read(1) != b"\n":
            raise ValueError(f"Git blob frame is truncated for {oid}")
        self.blobs_read += 1
        self.bytes_read += expected_size
        return payload

    def close(self, failed=False):
        if self.child is None:
            return
        if failed and self.child.poll() is None:
            self.child.kill()
        self.child.stdin.close()
        stderr = self.child.stderr.read().decode(errors="replace")
        code = self.child.wait()
        self.child.stdout.close()
        self.child.stderr.close()
        self.child = None
        if not failed and code:
            raise ValueError(f"Git object reader failed ({code}): {stderr}")


def native_schema(extension):
    with memory_connection(extension) as connection:
        rows = connection.execute(
            "SELECT return_type FROM duckdb_functions() WHERE function_name='spur_graph_symbols' AND function_type='scalar'"
        ).fetchall()
        if len(rows) != 1:
            raise ValueError("native spur_graph_symbols scalar is not uniquely registered")
        symbols = rows[0][0]
    return [
        ("blob_oid", "VARCHAR"), ("language", "VARCHAR"), ("extractor_sha", "VARCHAR"),
        ("representative_path", "VARCHAR"), ("symbols", symbols), ("status", "VARCHAR"),
        ("error", "VARCHAR"), ("input_bytes", "UBIGINT"), ("binary_prefix", "BOOLEAN"),
        ("parsed_at", "TIMESTAMP WITH TIME ZONE"),
    ]


def inspect_parquet(connection, path, batch, extractor, schema):
    actual_schema = [(row[0], row[1]) for row in connection.execute(
        "DESCRIBE SELECT * FROM read_parquet(?)", [str(path)]
    ).fetchall()]
    if actual_schema != schema:
        raise ValueError(f"native Parquet schema mismatch: {path}")
    keys = connection.execute(
        "SELECT blob_oid,language,extractor_sha,representative_path,input_bytes FROM read_parquet(?) ORDER BY ALL",
        [str(path)],
    ).fetchall()
    expected = sorted((task["blob_oid"], task["language"], extractor, task["path"], task["byte_length"])
                      for task in batch["tasks"])
    if keys != expected:
        raise ValueError(f"Parquet keys do not exactly match planned tasks: {path}")
    invalid = connection.execute(
        "SELECT count(*) FROM read_parquet(?) WHERE status IS NULL OR status NOT IN ('completed','binary_skipped','encoding_error','extension_error') "
        "OR (status='completed') <> (symbols IS NOT NULL) OR binary_prefix IS NULL "
        "OR binary_prefix <> (status='binary_skipped') OR (status IN ('encoding_error','extension_error')) <> (error IS NOT NULL) "
        "OR parsed_at IS NULL", [str(path)],
    ).fetchone()[0]
    if invalid:
        raise ValueError(f"nonterminal or inconsistent Parquet rows: {path}")
    statuses = dict.fromkeys(TERMINAL, 0)
    statuses.update(dict(connection.execute(
        "SELECT status,count(*) FROM read_parquet(?) GROUP BY status", [str(path)]
    ).fetchall()))
    empty, symbols, input_bytes = connection.execute(
        "SELECT count(*) FILTER(WHERE status='completed' AND array_length(symbols)=0),"
        "coalesce(sum(array_length(symbols)),0),coalesce(sum(input_bytes),0) FROM read_parquet(?)", [str(path)],
    ).fetchone()
    return {"key_count": len(keys), "status_counts": statuses, "zero_symbol_tasks": empty,
            "symbol_rows": symbols, "input_bytes": input_bytes,
            "binary_skipped_tasks": statuses["binary_skipped"]}


def batch_paths(directory, batch):
    stem = f"batch-{batch['batch_id']:06d}"
    return directory / (stem + ".parquet"), directory / (stem + ".receipt.json")


def verify_receipt(parquet, receipt_path, batch, provenance, schema):
    receipt = json.loads(receipt_path.read_text())
    expected = dict(provenance, batch_id=batch["batch_id"], schema_version=1,
                    parquet="batches/" + parquet.name, native_symbol_type=schema[4][1])
    if any(receipt.get(key) != value for key, value in expected.items()):
        raise ValueError(f"receipt provenance mismatch: {receipt_path}")
    if receipt.get("sha256") != sha256(parquet) or receipt.get("file_bytes") != parquet.stat().st_size:
        raise ValueError(f"Parquet checksum mismatch: {parquet}")
    with memory_connection() as connection:
        observed = inspect_parquet(connection, parquet, batch, provenance["extractor_sha256"], schema)
    if any(receipt.get(key) != value for key, value in observed.items()):
        raise ValueError(f"receipt counts/status mismatch: {receipt_path}")
    return receipt


def parse_batch(batch, directory, objects, extension, provenance, schema):
    parquet, receipt_path = batch_paths(directory, batch)
    if os.path.lexists(parquet) or os.path.lexists(receipt_path):
        raise ValueError(f"refusing to overwrite existing batch files: {parquet}")
    temporary = parquet.with_name(parquet.name + "." + uuid.uuid4().hex + ".tmp")
    with memory_connection(extension) as connection:
        connection.execute("CREATE TABLE tasks(blob_oid VARCHAR,language VARCHAR,representative_path VARCHAR,input_bytes UBIGINT,PRIMARY KEY(blob_oid,language))")
        if batch["tasks"]:
            connection.executemany("INSERT INTO tasks VALUES(?,?,?,?)", [
                (task["blob_oid"], task["language"], task["path"], task["byte_length"]) for task in batch["tasks"]
            ])
        connection.execute("CREATE TABLE source_payloads(blob_oid VARCHAR PRIMARY KEY,source VARCHAR,binary_prefix BOOLEAN,encoding_error BOOLEAN)")
        # The primary key and this OID-only map prevent a language join from
        # multiplying source rows. No payload cache survives this connection.
        unique_oids = {task["blob_oid"]: task["byte_length"] for task in batch["tasks"]}
        for oid, size in unique_oids.items():
            payload = objects.read(oid, size)
            binary = b"\x00" in payload[:8192]
            source, encoding_error = None, False
            if not binary:
                try:
                    source = payload.decode("utf-8")
                except UnicodeDecodeError:
                    encoding_error = True
            connection.execute("INSERT INTO source_payloads VALUES(?,?,?,?)", [oid, source, binary, encoding_error])
            del payload, source
        eligible = connection.execute("SELECT count(*) FROM tasks JOIN source_payloads USING(blob_oid) WHERE NOT binary_prefix AND NOT encoding_error").fetchone()[0]
        parse_started = time.perf_counter()
        # Materializing only eligible input guarantees binary and encoding-error
        # payloads never enter the native AST extractor, including under TRY.
        connection.execute(
            "CREATE TABLE parsed AS SELECT blob_oid,language,TRY(spur_graph_symbols(representative_path,source)) AS symbols "
            "FROM tasks JOIN source_payloads USING(blob_oid) WHERE NOT binary_prefix AND NOT encoding_error"
        )
        parse_seconds = time.perf_counter() - parse_started
        connection.execute(
            "CREATE TABLE facts AS SELECT t.blob_oid,t.language,?::VARCHAR AS extractor_sha,t.representative_path,p.symbols,"
            "CASE WHEN s.binary_prefix THEN 'binary_skipped' WHEN s.encoding_error THEN 'encoding_error' WHEN p.symbols IS NULL THEN 'extension_error' ELSE 'completed' END::VARCHAR AS status,"
            "CASE WHEN s.binary_prefix THEN NULL WHEN s.encoding_error THEN 'source is not valid UTF-8' WHEN p.symbols IS NULL THEN 'native extension returned NULL under TRY' ELSE NULL END::VARCHAR AS error,"
            "t.input_bytes,s.binary_prefix,current_timestamp AS parsed_at FROM tasks t JOIN source_payloads s USING(blob_oid) "
            "LEFT JOIN parsed p ON t.blob_oid=p.blob_oid AND t.language=p.language", [provenance["extractor_sha256"]],
        )
        connection.execute("COPY (SELECT * FROM facts ORDER BY blob_oid,language) TO " + quote(temporary) + " (FORMAT PARQUET)")
        observed = inspect_parquet(connection, temporary, batch, provenance["extractor_sha256"], schema)
    with temporary.open("rb") as stream:
        os.fsync(stream.fileno())
    checksum, file_bytes = sha256(temporary), temporary.stat().st_size
    temporary.rename(parquet)
    receipt = dict(provenance, schema_version=1, batch_id=batch["batch_id"],
                   parquet="batches/" + parquet.name, sha256=checksum, file_bytes=file_bytes,
                   native_symbol_type=schema[4][1], **observed)
    atomic_json(receipt_path, receipt, immutable=True)
    return receipt, parse_seconds, eligible


def git_head(repo):
    return subprocess.check_output(["git", "-C", str(repo), "rev-parse", "--verify", "HEAD^{commit}"], text=True).strip()


def run(args):
    started = time.perf_counter()
    if duckdb.__version__ != "2.0.0":
        raise ValueError("this frozen native artifact requires DuckDB 2.0.0")
    repo, extension, plan_path, output = (path.resolve() for path in (args.repo, args.extension, args.plan, args.output))
    if output == repo or repo in output.parents:
        raise ValueError("output must be outside the unchanged source repository")
    plan_bytes = plan_path.read_bytes()
    plan = json.loads(plan_bytes)
    planned_tasks = validate_plan(plan)
    if git_head(repo) != plan["head"]:
        raise ValueError("source HEAD does not match the frozen plan head")
    provenance = {"head": plan["head"], "engine": duckdb.__version__, "extractor_sha256": sha256(extension),
                  "plan_sha256": hashlib.sha256(plan_bytes).hexdigest()}
    del plan_bytes
    generation = output / "generations" / provenance["extractor_sha256"] / provenance["plan_sha256"]
    directory = generation / "batches"
    directory.mkdir(parents=True, exist_ok=True)
    receipts, statuses = [], Counter()
    totals = {"completed_tasks": 0, "zero_symbol_tasks": 0, "symbol_rows": 0, "binary_skipped_tasks": 0,
              "input_bytes": 0, "new_parse_tasks_this_run": 0, "native_parse_tasks_this_run": 0, "parse_seconds": 0.0}
    objects = GitObjects(repo)

    def snapshot(state, complete=False, **extra):
        usage = resource.getrusage(resource.RUSAGE_SELF)
        rss = usage.ru_maxrss if sys.platform == "darwin" else usage.ru_maxrss * 1024
        return dict(provenance, **totals, state=state, complete=complete,
                    planned_tasks=planned_tasks, missing_tasks=planned_tasks - totals["completed_tasks"],
                    completed_parse_tasks=statuses["completed"], error_tasks=statuses["encoding_error"] + statuses["extension_error"],
                    status_counts=dict(statuses), source_blobs_read=objects.blobs_read, source_bytes_read=objects.bytes_read,
                    wall_seconds=time.perf_counter() - started, cpu_user_seconds=usage.ru_utime,
                    cpu_system_seconds=usage.ru_stime, max_rss_bytes=rss,
                    timestamp=datetime.now(timezone.utc).isoformat(), generation=str(generation),
                    native_artifact=str(extension), native_artifact_bytes=extension.stat().st_size, **extra)

    def progress(state, **extra):
        value = snapshot(state, **extra)
        atomic_json(generation / "history-parse-progress.json", value)
        print(json.dumps(value, sort_keys=True), flush=True)
        return value

    def record(receipt, receipt_path):
        statuses.update(receipt["status_counts"])
        totals["completed_tasks"] += receipt["key_count"]
        for name in ("zero_symbol_tasks", "symbol_rows", "binary_skipped_tasks", "input_bytes"):
            totals[name] += receipt[name]
        receipts.append({"batch_id": receipt["batch_id"], "parquet": receipt["parquet"], "sha256": receipt["sha256"],
                         "receipt": "batches/" + receipt_path.name, "receipt_sha256": sha256(receipt_path), "key_count": receipt["key_count"]})

    with (generation / ".lock").open("a") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            atomic_json(generation / "history-parse-result.json", snapshot("running"))
            progress("initialized")
            schema = native_schema(extension)
            expected_files = {path.name for batch in plan["batches"] for path in batch_paths(directory, batch)}
            unexpected = [path.name for path in directory.iterdir()
                          if (path.name.endswith(".parquet") or path.name.endswith(".receipt.json")) and path.name not in expected_files]
            if unexpected:
                raise ValueError(f"unexpected immutable batch files retained: {unexpected}")
            pending = []
            # Verify every existing checkpoint before any source is read or a
            # missing batch is parsed. A receipt alone never proves coverage.
            for batch in plan["batches"]:
                parquet, receipt_path = batch_paths(directory, batch)
                present = (os.path.lexists(parquet), os.path.lexists(receipt_path))
                if present == (False, False):
                    pending.append(batch)
                elif present != (True, True):
                    raise ValueError(f"incomplete immutable Parquet/receipt pair retained: {parquet}")
                else:
                    receipt = verify_receipt(parquet, receipt_path, batch, provenance, schema)
                    record(receipt, receipt_path)
                    progress("verified_checkpoint", batch_id=batch["batch_id"], batch_parse_seconds=0.0)
            for batch in pending:
                batch_started = time.perf_counter()
                batch_cpu = resource.getrusage(resource.RUSAGE_SELF)
                totals["new_parse_tasks_this_run"] += len(batch["tasks"])
                receipt, parse_seconds, eligible = parse_batch(batch, directory, objects, extension, provenance, schema)
                totals["native_parse_tasks_this_run"] += eligible
                totals["parse_seconds"] += parse_seconds
                _, receipt_path = batch_paths(directory, batch)
                record(receipt, receipt_path)
                current_cpu = resource.getrusage(resource.RUSAGE_SELF)
                progress("checkpoint", batch_id=batch["batch_id"], batch_parse_seconds=parse_seconds,
                         batch_wall_seconds=time.perf_counter() - batch_started,
                         batch_cpu_user_seconds=current_cpu.ru_utime - batch_cpu.ru_utime,
                         batch_cpu_system_seconds=current_cpu.ru_stime - batch_cpu.ru_stime)
            objects.close()
            if git_head(repo) != plan["head"] or sha256(extension) != provenance["extractor_sha256"] or sha256(plan_path) != provenance["plan_sha256"]:
                raise ValueError("source HEAD, native artifact, or plan changed during ingestion")
            if totals["completed_tasks"] != planned_tasks or len(receipts) != len(plan["batches"]):
                raise ValueError("verified terminal rows do not cover the exact plan")
            state = "completed_with_errors" if statuses["encoding_error"] + statuses["extension_error"] else "completed"
            coverage = {"planned_keys": planned_tasks, "verified_terminal_keys": totals["completed_tasks"],
                        "missing_keys": 0, "planned_batches": len(plan["batches"]), "verified_batches": len(receipts)}
            result = snapshot(state, complete=True, checksums=receipts, coverage=coverage)
            atomic_json(generation / "history-parse-result.json", result)
            progress(state, complete=True, checksums=receipts, coverage=coverage)
            return 1 if state == "completed_with_errors" else 0
        except Exception as error:
            objects.close(failed=True)
            result = snapshot("infrastructure_error", error=str(error), checksums=receipts)
            atomic_json(generation / "history-parse-result.json", result)
            progress("infrastructure_error", error=str(error))
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("repo", "plan", "extension", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    try:
        return run(parser.parse_args())
    except Exception as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
