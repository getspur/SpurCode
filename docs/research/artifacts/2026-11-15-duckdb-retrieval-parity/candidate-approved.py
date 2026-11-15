"""Independent retrieval experiment; run with the pinned DuckDB 2.0.0 Python.

CLI (all paths may be absolute; output must not exist)::

    python retrieval_parity.py compare --native /path/to/retrieval_native \
      --corpus /path/to/prepared-corpus --extension /path/to/spur_graph.duckdb_extension \
      --out /private/tmp/new-parity-run

Prepare the corpus with ``retrieval_native prepare spec.json NEW_CORPUS`` first.
``--spec`` optionally verifies an external copy of that exact shared spec.
``serve-sql`` accepts the same {case_id,tool,args} JSONL as ``retrieval_native
serve CORPUS``. Both processes warm the whole shared matrix before emitting a
ready receipt on stderr. JsonlEndpoint is importable by the parent notebook for
paired repetitions. This script performs ONE correctness smoke pass, no ratios.

Data flow: shared spec/source bytes -> spur_graph_file_facts -> typed tables ->
SQL queries -> candidate replies. Candidate never opens .retrieval/native or a
native graph artifact. Only the comparison coordinator reads native provenance.
No source parser or repository resolver is implemented. Recursive containment
joins reconstruct qualified names; edge joins ALWAYS include logical_path.
Unbound edges stay unbound. All five fact collections and every field/ordinal
are retained in candidate.duckdb, facts.jsonl, and schema.json.

Semantic identity mapping: each reply independently replaces non-null id/uri
values on symbol rows with (file_path, symbol_kind, qualified_name, line_range).
For traversal rows qualified_name is enclosing_scope::entity_name; the source
response format exposes only (file,name,range), which is documented separately.
Null/missing identity fields remain null/missing. No oracle ID lookup is used.
Only the enumerated graph freshness/provenance fields move out of the semantic
comparison into the separately reported API comparison. ALL other fields,
including unknown ones, remain. Traversal lists compare as multisets; observed
order is also reported. Search/ambiguity ranking and unresolved_sample retain
order. Full API comparison includes metadata, IDs, URIs, nullability, and order.
Line reads include entire lines (including LF/CRLF), exactly as the native API;
extension byte spans are additionally retained as evidence, never substituted
for whole-line source. Validation uses UTF-8 bytes and splits only at LF.

Coverage: file-local edges only; no repository import/name binding or native
metadata service. Simple * and ? globs are supported; richer valid glob syntax
is explicitly unsupported. Invalid '[' matches no files, as native globset does.
Full/table/compact production formatting beyond full and read-source is not
implemented. Unsupported requests return an explicit unsupported error, never
an empty success. Native anomalies and negative parity are experiment results.

Timing: extraction, typed materialization, connection/load, first dispatch and
startup matrix warmup are separate SQL receipts. Native prepare construction is
copied intact; native serve exposes only aggregate verification/startup/warmup
and dispatch timing, not a cold first-query observation. Client wall samples on
BOTH endpoints include JSON encoding, pipe write/flush/read and JSON decoding.
elapsed_ns on BOTH excludes JSON transport and includes handler/SQL plus reply
body construction (SQL also records query text/parameters); envelope wrapping is
excluded. Native additionally performs metadata/freshness work; bytes and
API limits are disclosed. Parent notebook owns repetitions and eligibility.
"""

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import queue
import re
import subprocess
import sys
import threading
import time


APPROVED_EXTENSION_SHA256 = "e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028"
FIXTURE_SHA256 = "6a82c682c4b595c55135527dbc58811c28a725d058eb6f1b1bd68a84fcf2b836"
METADATA = frozenset({
    "graph_content_hash", "graph_index_version", "graph_built_at",
    "indexed_head_oid", "worktree_head_oid", "worktree_dirty",
    "response_file_oids_match", "rebuild_status",
})
LIMITS = [
    "Extension edges are file-local; no repository binding or label-inbound resolution.",
    "Native graph provenance/freshness service is unavailable to SQL candidate.",
    "Native serve prewarms the matrix; native cold first-query time is not exposed.",
    "Full API and transport sizes differ; semantic equality alone is not API compatibility.",
]
FIELD_COVERAGE = {
    "semantic": "All body/error fields except the explicitly listed metadata and independently mapped symbol id/uri values.",
    "metadata_separate": sorted(METADATA),
    "source": "Whole-line UTF-8 bytes including original CRLF/LF, file_oid, returned and symbol ranges, context and requested clamping fields.",
    "edge_api": "Every returned native field, including kind/bind_method/confidence, counts, unresolved labels/sample and duplicates.",
    "extension_evidence": "All five raw fact collections, all struct fields and ordinals retained; relevant raw edge records accompany traversal replies.",
    "evidence_not_exposed_by_native": "Raw node/edge numeric IDs, byte spans, confidence_score, evidence_id, receiver/import/scope text, directed/change_kind and macro facts have no complete code_* traversal counterpart. They are preserved, with no equality assertion for those unexposed fields.",
    "identity_limit": "API symbols expose line ranges, not byte spans; source-format identity also lacks kind. Mapping establishes only this documented semantic identity, never full API compatibility.",
}


def dumps(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def blob_oid(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def write_json(path, value):
    Path(path).write_bytes((json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n").encode())


def shared_inputs(corpus, spec_path=None):
    """Read shared inputs ONLY; deliberately no native artifact/provenance access."""
    corpus = Path(corpus).resolve(strict=True)
    raw = (corpus / ".retrieval/spec.json").read_bytes()
    if spec_path and Path(spec_path).read_bytes() != raw:
        raise ValueError("external spec differs from prepared shared spec bytes")
    spec = json.loads(raw)
    if set(spec) != {"files", "requests"} or not spec["files"]:
        raise ValueError("expected nonempty shared files and requests")
    if not (corpus / ".git").is_dir() or (corpus / ".git/commondir").exists():
        raise ValueError("expected standalone prepared Git corpus")
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
    def git(*args):
        result = subprocess.run(["git", "--no-optional-locks", "-C", str(corpus), *args], env=env, capture_output=True)
        if result.returncode:
            raise ValueError("corpus git verification failed: " + result.stderr.decode())
        return result.stdout
    if git("rev-parse", "--show-toplevel").decode().strip() != str(corpus):
        raise ValueError("wrong corpus Git root")
    head = git("rev-parse", "HEAD").decode().strip()
    seen = set()
    sources = []
    for item in spec["files"]:
        if set(item) != {"path", "source"} or not isinstance(item["source"], str):
            raise ValueError("invalid shared source")
        path = item["path"]
        parts = PurePosixPath(path).parts
        if (not parts or path != "/".join(parts) or path.startswith("/")
                or "\\" in path or "\0" in path or ".." in parts
                or any(p.lower() in {".git", ".spur", ".retrieval"} for p in parts)
                or path in seen):
            raise ValueError(f"unsafe/duplicate logical path: {path}")
        target = corpus / path
        if target.resolve(strict=True) != target:
            raise ValueError(f"source symlink: {path}")
        data = target.read_bytes()
        if data != item["source"].encode("utf-8"):
            raise ValueError(f"source differs from shared spec: {path}")
        if git("show", f"HEAD:{path}") != data:
            raise ValueError(f"committed source differs: {path}")
        seen.add(path)
        sources.append(dict(path=path, bytes=len(data), sha256=sha256(data), git_blob_oid=blob_oid(data)))
    ids = set()
    for req in spec["requests"]:
        if (set(req) != {"case_id", "tool", "args"} or not isinstance(req["args"], dict)
                or not isinstance(req["tool"], str)
                or not isinstance(req["case_id"], str) or not req["case_id"] or req["case_id"] in ids):
            raise ValueError("invalid/duplicate request")
        ids.add(req["case_id"])
    tracked = {p.decode("utf-8") for p in git("ls-files", "-z").split(b"\0") if p}
    if tracked != seen or git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("corpus contains changed/additional files outside the complete shared spec")
    return spec, dict(spec_sha256=sha256(raw), head_oid=head, sources=sources)


class RequestError(Exception):
    def __init__(self, code, message, data=None, unsupported=False):
        super().__init__(message)
        self.error = dict(code=code, message=message, data=data)
        self.unsupported = unsupported


def unsupported(message):
    raise RequestError(-32099, message, {"kind": "unsupported"}, unsupported=True)


def string_arg(args, key, required=False):
    if key not in args:
        if required:
            raise RequestError(-32602, f"Missing required field '{key}'")
        return None
    value = args[key]
    if not isinstance(value, str):
        raise RequestError(-32602, f"field '{key}' must be a string")
    if not value.strip():
        raise RequestError(-32602, f"field '{key}' must not be empty")
    return value


def relative_path(value, field):
    if value.startswith("/"):
        raise RequestError(-32602, f"field '{field}' must be a worktree-relative path")
    for part in value.split("/"):
        if part in {".", ".."}:
            raise RequestError(-32602, f"field '{field}' must not contain '{part}' path components")
    if value != "/".join(PurePosixPath(value).parts):
        raise RequestError(-32602, f"field '{field}' must be a normalized worktree-relative path without '.' or '..' components")
    return value


def clamped_arg(args, key, default, lower, upper):
    value = args.get(key, default)
    if type(value) is not int:
        raise RequestError(-32602, f"field '{key}' must be an integer")
    actual = min(upper, max(lower, value))
    return actual, value if value != actual else None


# Every numeric node/span/edge ID is file-local. Never join these without path.
MATERIALIZE = """
CREATE TABLE nodes AS SELECT logical_path, ordinal, n.* FROM facts,
  UNNEST(value.nodes) WITH ORDINALITY AS u(n, ordinal);
CREATE TABLE edges AS SELECT logical_path, ordinal, e.* FROM facts,
  UNNEST(value.edges) WITH ORDINALITY AS u(e, ordinal);
CREATE TABLE spans AS SELECT logical_path, ordinal, s.* FROM facts,
  UNNEST(value.spans) WITH ORDINALITY AS u(s, ordinal);
CREATE TABLE macro_invocations AS SELECT logical_path, ordinal, m.* FROM facts,
  UNNEST(value.macro_invocations) WITH ORDINALITY AS u(m, ordinal);
CREATE TABLE macro_scopes AS SELECT logical_path, ordinal, m.* FROM facts,
  UNNEST(value.macro_scopes) WITH ORDINALITY AS u(m, ordinal);
CREATE TABLE symbols AS
WITH RECURSIVE names AS (
  SELECT n.logical_path, n.node_id, ''::VARCHAR AS enclosing,
    CASE WHEN kind='file' THEN '' WHEN kind='impl' THEN 'impl '||label ELSE label END AS qualified
  FROM nodes n WHERE NOT EXISTS (SELECT 1 FROM edges e
    WHERE e.logical_path=n.logical_path AND e.target_node_id=n.node_id AND e.relation='contains')
  UNION ALL
  SELECT n.logical_path,n.node_id,p.qualified,
    concat_ws('::',nullif(p.qualified,''),CASE WHEN n.kind='impl' THEN 'impl '||n.label ELSE n.label END)
  FROM names p JOIN edges e ON e.logical_path=p.logical_path AND e.source_node_id=p.node_id AND e.relation='contains'
  JOIN nodes n ON n.logical_path=e.logical_path AND n.node_id=e.target_node_id
)
SELECT n.logical_path AS file_path,n.node_id,n.stable_key AS id,
  'graph://symbol/'||n.stable_key AS uri,n.label AS entity_name,n.kind AS symbol_kind,
  names.qualified AS qualified_name,nullif(names.enclosing,'') AS enclosing_scope,
  [s.start_line,s.end_line] AS line_range,s.start_line,s.end_line,s.start_byte,s.end_byte,
  n.logical_path||'::'||names.qualified AS selector
FROM names JOIN nodes n USING(logical_path,node_id)
JOIN spans s ON s.logical_path=n.logical_path AND s.span_id=n.source_span_id
WHERE n.kind <> 'file';
CREATE TABLE source_lines AS
SELECT logical_path,ordinal AS line_number,
  line||CASE WHEN ordinal<len(parts) THEN chr(10) ELSE '' END AS line
FROM (SELECT logical_path,string_split(source,chr(10)) AS parts FROM sources),
  UNNEST(parts) WITH ORDINALITY AS u(line,ordinal)
WHERE ordinal<len(parts) OR line<>'';
CREATE VIEW local_edges AS
SELECT e.*,src.id AS source_id,dst.id AS target_id,
  coalesce(e.edge_kind,CASE WHEN relation='calls' THEN 'calls' ELSE 'references_other' END) AS effective_kind
FROM edges e
JOIN symbols src ON src.file_path=e.logical_path AND src.node_id=e.source_node_id
LEFT JOIN symbols dst ON dst.file_path=e.logical_path AND dst.node_id=e.target_node_id
WHERE e.relation IN ('calls','references');
"""
SYMBOL_COLUMNS = "selector,uri,id,entity_name,qualified_name,file_path,line_range,symbol_kind,enclosing_scope"
LOCATION_ORDER = "file_path,start_line,end_line,id"


class Candidate:
    def __init__(self, corpus, extension, output=None, expected_sha=APPROVED_EXTENSION_SHA256, spec_path=None):
        import duckdb
        import _duckdb

        started = time.perf_counter_ns()
        if duckdb.__version__ != "2.0.0":
            raise ValueError(f"native DuckDB 2.0.0 required, got {duckdb.__version__}")
        self.spec, self.inputs = shared_inputs(corpus, spec_path)
        extension = Path(extension).resolve(strict=True)
        self.extension = extension
        actual_sha = sha256(extension.read_bytes())
        if expected_sha != APPROVED_EXTENSION_SHA256 or actual_sha != expected_sha:
            raise ValueError(f"unapproved extension hash: {actual_sha}")
        self.output = Path(output) if output else None
        if self.output:
            self.output.mkdir()
        self.query_log = []
        self.db = duckdb.connect(str(self.output / "candidate.duckdb") if self.output else ":memory:",
                                 config={"allow_unsigned_extensions": "true", "threads": "2"})
        self.db.execute("SET temp_directory=''")
        # LOAD doesn't accept bound parameters. Escape the sole SQL identifier literal.
        self.db.execute("LOAD '" + str(extension).replace("'", "''") + "'")
        engine_version = self.db.execute("SELECT version()").fetchone()[0]
        if engine_version != "v2.0.0":
            raise ValueError(f"wrong native engine: {engine_version}")
        loaded = time.perf_counter_ns()
        self.db.execute("CREATE TABLE sources(logical_path VARCHAR, source VARCHAR, source_bytes BLOB, file_oid VARCHAR, sha256 VARCHAR)")
        self.db.executemany("INSERT INTO sources VALUES (?,?,?,?,?)", [
            (f["path"], f["source"], f["source"].encode(), blob_oid(f["source"].encode()), sha256(f["source"].encode()))
            for f in self.spec["files"]])
        staged = time.perf_counter_ns()
        self.db.execute("CREATE TABLE facts AS SELECT logical_path,spur_graph_file_facts(logical_path,source) AS value FROM sources")
        extracted = time.perf_counter_ns()
        self.db.execute(MATERIALIZE)
        materialized = time.perf_counter_ns()
        self.provenance = dict(
            **self.inputs, extension=str(extension), extension_sha256=actual_sha,
            duckdb=duckdb.__version__, python=sys.version, python_executable=sys.executable,
            duckdb_module=duckdb.__file__, host=platform.platform(), machine=platform.machine(),
            native_engine_version=engine_version, native_engine_binary=_duckdb.__file__,
            native_engine_sha256=sha256(Path(_duckdb.__file__).read_bytes()),
            python_executable_sha256=sha256(Path(sys.executable).read_bytes()),
            implementation_sha256=sha256(Path(__file__).read_bytes()),
            timings_ns=dict(connection_load_and_input_validation=loaded-started, source_staging=staged-loaded,
                            extraction=extracted-staged, typed_materialization=materialized-extracted),
            candidate_inputs=[".retrieval/spec.json", "exact corpus source bytes", "approved extension"],
            limitations=LIMITS, field_coverage=FIELD_COVERAGE, sql=MATERIALIZE,
            counts={t: self.db.execute(f"SELECT count(*) FROM {t}").fetchone()[0]
                    for t in ("sources", "facts", "nodes", "edges", "spans", "symbols", "macro_invocations", "macro_scopes")})
        if self.output:
            with (self.output / "facts.jsonl").open("wb") as stream:
                for path, value in self.db.execute("SELECT * FROM facts ORDER BY logical_path").fetchall():
                    stream.write((dumps(dict(logical_path=path, facts=value)) + "\n").encode())
            write_json(self.output / "schema.json", self.db.execute(
                "SELECT table_name,column_name,data_type,is_nullable FROM information_schema.columns ORDER BY table_name,ordinal_position").fetchall())
        finished = time.perf_counter_ns()
        self.provenance["timings_ns"].update(provenance_and_export=finished-materialized,
                                              total_construction=finished-started)

    def rows(self, sql, params=None):
        params = params or {}
        self.query_log.append(dict(sql=sql, params=params))
        cur = self.db.execute(sql, params)
        columns = [d[0] for d in cur.description]
        return [dict(zip(columns, row)) for row in cur.fetchall()]

    def dispatch(self, request):
        started = time.perf_counter_ns()
        coverage = dict(supported=True, resolution="extension_file_local", full_api_supported=False,
                        repository_binding_supported=False, api_limits=LIMITS[:2])
        evidence = None
        try:
            args, tool = request["args"], request["tool"]
            if not isinstance(args, dict):
                raise RequestError(-32602, "args must be an object")
            fmt = args.get("response_format", "full")
            if fmt != "full" and not (tool == "code_read_symbol" and fmt == "source"):
                unsupported(f"response_format {fmt!r} is not implemented by the SQL experiment")
            known = {
                "code_symbol_search": {"query", "mode", "symbol_kind", "file", "file_glob", "limit", "response_format"},
                "code_read_symbol": {"path", "name", "stable_symbol_id", "context_lines", "response_format"},
                "code_callers": {"selector", "include_unresolved", "on_ambiguous", "response_format"},
                "code_callees": {"selector", "include_unresolved", "on_ambiguous", "response_format"},
            }
            if tool not in known:
                unsupported(f"tool {tool!r} is outside the SQL experiment")
            if set(args) - known[tool]:
                unsupported(f"unsupported request fields: {sorted(set(args)-known[tool])}")
            if tool == "code_symbol_search":
                body = self.search(args)
            else:
                body, evidence = self.read_or_edges(tool, args)
            error = None
        except RequestError as exc:
            body, error = None, exc.error
            coverage["supported"] = not exc.unsupported
        elapsed_ns = time.perf_counter_ns() - started
        return dict(request, body=body, error=error, elapsed_ns=elapsed_ns, coverage=coverage, evidence=evidence)

    def search(self, args):
        query = string_arg(args, "query", True)
        mode = args.get("mode", "substring")
        if mode not in ("exact", "prefix", "substring"):
            raise RequestError(-32602, f"invalid mode `{mode}`; expected `exact`, `prefix`, or `substring`")
        kind, file, glob = [string_arg(args, k) for k in ("symbol_kind", "file", "file_glob")]
        if file:
            file = relative_path(file, "file")
        if glob:
            glob = relative_path(glob, "file_glob")
        if file and glob:
            raise RequestError(-32602, "fields 'file' and 'file_glob' are mutually exclusive")
        limit, requested = clamped_arg(args, "limit", 20, 1, 200)
        pattern = ".*"
        if glob:
            if glob == "[":
                pattern = "a^"  # globset rejects this pattern; native produces no matches.
            elif any(c in glob for c in "[]{}\\") or "**" in glob:
                unsupported(f"glob syntax outside the supported * and ? subset: {glob!r}")
            else:
                pattern = "".join(".*" if c == "*" else "." if c == "?" else re.escape(c) for c in glob)
        params = dict(query=query, kind=kind, file=file, pattern=pattern, limit=limit)
        predicate = {
            "exact": "(entity_name=$query OR qualified_name=$query)",
            "prefix": "starts_with(entity_name,$query)",
            "substring": "contains(entity_name,$query)",
        }[mode]
        rank = {
            "exact": "CASE WHEN entity_name=$query THEN 0 ELSE 1 END",
            "prefix": "octet_length(encode(entity_name))",
            "substring": "octet_length(encode(substr(entity_name,1,strpos(entity_name,$query)-1))),octet_length(encode(entity_name))",
        }[mode]
        where = predicate + " AND ($kind IS NULL OR symbol_kind=$kind) AND ($file IS NULL OR file_path=$file) AND regexp_full_match(file_path,$pattern)"
        # Count before LIMIT, including zero rows; no Python row filtering/ranking.
        count_params = {k: v for k, v in params.items() if k != "limit"}
        total = self.rows("SELECT count(*) AS n FROM symbols WHERE " + where, count_params)[0]["n"]
        candidates = self.rows(f"SELECT {SYMBOL_COLUMNS} FROM symbols WHERE {where} ORDER BY {rank},{LOCATION_ORDER} LIMIT $limit", params)
        body = dict(query=query, mode=mode, symbol_kind=kind, file=file, file_glob=glob,
                    limit=limit, total_matches=total, truncated=total > limit, candidates=candidates)
        if requested is not None:
            body["requested_limit"] = requested
        return body

    def select(self, tool, args):
        if tool == "code_read_symbol":
            sid, path, name = [string_arg(args, k) for k in ("stable_symbol_id", "path", "name")]
            if sid and (path or name):
                raise RequestError(-32602, "field 'stable_symbol_id' is mutually exclusive with fields 'path' and 'name'")
            if sid:
                matches = self.rows(f"SELECT * FROM symbols WHERE id=$id ORDER BY {LOCATION_ORDER}", {"id": sid.removeprefix("graph://symbol/")})
            elif path and name:
                path = relative_path(path, "path")
                matches = self.rows(f"SELECT * FROM symbols WHERE file_path=$path AND (qualified_name=$name OR entity_name=$name) ORDER BY {LOCATION_ORDER}", dict(path=path, name=name))
            else:
                raise RequestError(-32602, "fields 'path' and 'name' must be provided together")
            label = f"symbol `{name}` in file `{path}`" if path else f"symbol `{sid}`"
        else:
            selector = string_arg(args, "selector", True)
            if selector.startswith("graph://symbol/"):
                matches = self.rows(f"SELECT * FROM symbols WHERE id=$id ORDER BY {LOCATION_ORDER}", {"id": selector.removeprefix("graph://symbol/")})
            elif "::" in selector:
                path, name = selector.split("::", 1)
                if "/" in path or "." in path:
                    path = relative_path(path, "selector")
                    matches = self.rows(f"SELECT * FROM symbols WHERE file_path=$path AND qualified_name=$name ORDER BY {LOCATION_ORDER}", dict(path=path, name=name))
                else:
                    matches = self.rows(f"SELECT * FROM symbols WHERE qualified_name=$name ORDER BY {LOCATION_ORDER}", dict(name=selector))
            else:
                matches = self.rows(f"SELECT * FROM symbols WHERE qualified_name=$name ORDER BY {LOCATION_ORDER}", dict(name=selector))
                if not matches:
                    matches = self.rows(f"SELECT * FROM symbols WHERE entity_name=$name OR id=$name ORDER BY {LOCATION_ORDER}", dict(name=selector))
            label = f"symbol `{selector}`"
        if not matches:
            raise RequestError(-32004, label + " not found in graph artifact", {"kind": "not_found"})
        return matches

    def read_or_edges(self, tool, args):
        if tool != "code_read_symbol" and args.get("on_ambiguous", "candidates") not in ("candidates", "error"):
            unsupported("unsupported on_ambiguous value")
        matches = self.select(tool, args)
        if len(matches) > 1:
            if args.get("on_ambiguous", "candidates") == "error":
                selector = args.get("selector")
                raise RequestError(-32602, f"selector `{selector}` is ambiguous; choose one candidate selector or uri")
            if args.get("on_ambiguous", "candidates") != "candidates":
                unsupported("unsupported on_ambiguous value")
            return dict(ambiguous=True, candidates=[{k: m[k] for k in SYMBOL_COLUMNS.split(",")} for m in matches]), None
        symbol = matches[0]
        if tool == "code_read_symbol":
            context, requested = clamped_arg(args, "context_lines", 0, 0, 50)
            text = self.rows("""
              WITH bounds AS (SELECT greatest(1,$start-$context) AS first,
                greatest(greatest(1,$start-$context)-1,least($end+$context,
                  (SELECT count(*) FROM source_lines WHERE logical_path=$path))) AS last)
              SELECT first,last,(SELECT coalesce(string_agg(line,'' ORDER BY line_number),'')
                FROM source_lines,bounds WHERE logical_path=$path AND line_number BETWEEN first AND last) AS source,
                (SELECT file_oid FROM sources WHERE logical_path=$path) AS file_oid FROM bounds
              """, dict(start=symbol["start_line"], end=symbol["end_line"], context=context, path=symbol["file_path"]))[0]
            line_range = dict(start=text["first"], end=text["last"])
            if args.get("response_format") == "source":
                body = dict(id=symbol["id"], name=symbol["qualified_name"], file=symbol["file_path"],
                            range=line_range, source=text["source"], file_oid=text["file_oid"])
            else:
                info = {k: symbol[k] for k in SYMBOL_COLUMNS.split(",") if k != "selector"}
                body = dict(symbol=info, source=text["source"], line_range=line_range,
                            file_oid=text["file_oid"], context_lines=context)
            if requested is not None:
                body["requested_context_lines"] = requested
            return body, dict(byte_range=[symbol["start_byte"], symbol["end_byte"]],
                              line_range=symbol["line_range"], identity=symbol["selector"])
        return self.traverse(tool, args, symbol)

    def traverse(self, tool, args, symbol):
        include = args.get("include_unresolved", False)
        if type(include) is not bool:
            raise RequestError(-32602, "field 'include_unresolved' must be a boolean")
        callers = tool == "code_callers"
        selected = "target_node_id" if callers else "source_node_id"
        neighbor = "source_node_id" if callers else "target_node_id"
        # No joining unbound labels to other files (or to symbols in this file).
        query = f"""SELECT e.*,s.uri,s.entity_name,s.enclosing_scope,s.file_path,s.line_range,s.symbol_kind
          FROM local_edges e LEFT JOIN symbols s ON s.file_path=e.logical_path AND s.node_id=e.{neighbor}
          WHERE e.logical_path=$path AND e.{selected}=$node
          ORDER BY s.file_path NULLS LAST,s.start_line,s.end_line,s.id,e.target_label,e.effective_kind,e.ordinal"""
        params = dict(path=symbol["file_path"], node=symbol["node_id"])
        records = self.rows(query, params)
        # Inclusion is a SQL filter; Python only serializes the selected API rows.
        visible = self.rows(f"""SELECT * FROM ({query}) WHERE $include OR target_id IS NOT NULL
                            ORDER BY file_path NULLS LAST,line_range[1],line_range[2],uri,target_label,effective_kind,ordinal""",
                            dict(params, include=include))
        # Aggregation and sampling happen in SQL, before include_unresolved filtering.
        base = f"FROM local_edges e WHERE e.logical_path=$path AND e.{selected}=$node"
        kinds = self.rows("SELECT " + ",".join(
            f"count(*) FILTER (WHERE effective_kind='{k}') AS {k}" for k in
            ("calls", "calls_dyn", "references_hof", "references_other", "references_address"))
            + ",count(*) FILTER (WHERE target_id IS NULL) AS unresolved " + base, params)[0]
        context = self.rows(f"""WITH rows AS ({query}), classified AS (
          SELECT CASE WHEN contains(file_path,'/benches/') OR starts_with(file_path,'benches/') THEN 'bench'
            WHEN contains(file_path,'/tests/') OR starts_with(file_path,'tests/') THEN 'test'
            WHEN file_path IS NOT NULL THEN 'production' END AS context FROM rows)
          SELECT count(*) FILTER(WHERE context='production') AS production,
            count(*) FILTER(WHERE context='test') AS test,count(*) FILTER(WHERE context='bench') AS bench FROM classified""", params)[0]
        sample = self.rows(f"""WITH ordered AS ({query}), labels AS (
          SELECT target_label,min(ordinal) AS pos FROM ordered WHERE target_id IS NULL GROUP BY target_label),
          bounded AS (SELECT target_label,row_number() OVER(ORDER BY target_label,pos) AS n,
            sum(octet_length(encode(target_label))) OVER(ORDER BY target_label,pos) AS bytes FROM labels)
          SELECT target_label FROM bounded WHERE n<=5 AND bytes<=120 ORDER BY n""", params)
        output = []
        for row in visible:
            resolved = row["target_id"] is not None
            if callers or resolved:
                item = {k: row[k] for k in ("uri", "entity_name", "enclosing_scope", "file_path", "line_range", "symbol_kind")}
            else:
                item = dict(entity_name=row["target_label"], target_label=row["target_label"])
            item.update(resolved=resolved, edge_kind=row["effective_kind"])
            if row["bind_method"] is not None:
                item.update(bind_method=row["bind_method"], confidence=row["confidence"])
            elif row["effective_kind"] == "calls_dyn":
                item["confidence"] = row["confidence"]
            if callers and not resolved:
                item["target_label"] = row["target_label"]
            output.append(item)
        key = "callers" if callers else "callees"
        return {key: output, "include_unresolved": include, "counts_by_kind": kinds,
                "counts_by_context": context, "unresolved_sample": [r["target_label"] for r in sample]}, {
                    "extension_edge_records": records,
                    "coverage": "file-local; unbound cross-file/import edges are never rebound"}

    def finish(self):
        if self.output:
            write_json(self.output / "provenance.json", self.provenance)
            write_json(self.output / "queries.json", self.query_log)
        self.db.close()
        if sha256(self.extension.read_bytes()) != self.provenance["extension_sha256"]:
            raise ValueError("extension changed during SQL session")


def symbol_key(row):
    if "file_path" in row and "line_range" in row:
        name = row.get("qualified_name")
        if name is None and "entity_name" in row:
            name = "::".join(filter(None, [row.get("enclosing_scope"), row["entity_name"]]))
        return [row["file_path"], row.get("symbol_kind"), name, row["line_range"]]
    if {"file", "name", "range"} <= row.keys():
        return [row["file"], "source-format:kind-unavailable", row["name"], row["range"]]
    return None


def semantic_value(value, at="", traversal_multiset=False):
    if isinstance(value, dict):
        key = symbol_key(value)
        result = {}
        for field, item in value.items():
            # Metadata is excluded ONLY at the documented API metadata locations.
            if at in {"/body", "/error/data"} and field in METADATA:
                continue
            if key is not None and field in {"id", "uri"} and isinstance(item, str):
                result[field] = {"semantic_identity": key}
            else:
                result[field] = semantic_value(item, at + "/" + field, traversal_multiset)
        return result
    if isinstance(value, list):
        result = [semantic_value(v, at + "/" + str(i), traversal_multiset) for i, v in enumerate(value)]
        if traversal_multiset and at in {"/body/callers", "/body/callees"}:
            result.sort(key=dumps)
        return result
    return value


def differences(left, right, at=""):
    """Lossless structural diff: absent != null, bool != number; no output cap."""
    if type(left) is not type(right):
        return [dict(path=at, kind="type_or_null", native=left, candidate=right)]
    if isinstance(left, dict):
        result = []
        for key in sorted(left.keys() | right.keys()):
            path = at + "/" + str(key)
            if key not in left:
                result.append(dict(path=path, kind="extra_field", candidate=right[key]))
            elif key not in right:
                result.append(dict(path=path, kind="missing_field", native=left[key]))
            else:
                result.extend(differences(left[key], right[key], path))
        return result
    if isinstance(left, list):
        result = []
        if len(left) != len(right):
            result.append(dict(path=at, kind="length", native=len(left), candidate=len(right)))
        for i in range(max(len(left), len(right))):
            path = at + "/" + str(i)
            if i >= len(left):
                result.append(dict(path=path, kind="extra_row", candidate=right[i]))
            elif i >= len(right):
                result.append(dict(path=path, kind="missing_row", native=left[i]))
            else:
                result.extend(differences(left[i], right[i], path))
        return result
    return [] if left == right else [dict(path=at, kind="value", native=left, candidate=right)]


def payload(reply):
    return {k: reply[k] for k in ("body", "error")}


def validate_envelope(request, reply):
    if any(reply.get(k) != request[k] for k in ("case_id", "tool", "args")):
        raise ValueError("endpoint did not echo the exact request")
    if "body" not in reply or "error" not in reply or ((reply["body"] is None) == (reply["error"] is None)):
        raise ValueError("endpoint must return exactly one body/error")


def identities(value, at=""):
    found = {}
    if isinstance(value, dict):
        for k, v in value.items():
            path = at + "/" + k
            if k in {"id", "uri", "selector"}:
                found[path] = v
            found.update(identities(v, path))
    elif isinstance(value, list):
        for i, v in enumerate(value):
            found.update(identities(v, at + "/" + str(i)))
    return found


def compare_replies(request, native, candidate):
    validate_envelope(request, native)
    validate_envelope(request, candidate)
    a, b = payload(native), payload(candidate)
    sem_a, sem_b = [semantic_value(x, traversal_multiset=True) for x in (a, b)]
    semantic_diff = differences(sem_a, sem_b)
    api_diff = differences(a, b)
    components = {}
    for key in ("candidates", "callers", "callees"):
        rows_a, rows_b = (a["body"] or {}).get(key), (b["body"] or {}).get(key)
        if rows_a is None and rows_b is None:
            continue
        if not isinstance(rows_a, list) or not isinstance(rows_b, list):
            components[key] = dict(comparable=False, reason="row array missing/null on one endpoint")
            continue
        norm_a, norm_b = [[semantic_value(r) for r in rows] for rows in (rows_a, rows_b)]
        ca, cb = Counter(map(dumps, norm_a)), Counter(map(dumps, norm_b))
        components[key] = dict(
            comparable=True, native_count=len(rows_a), candidate_count=len(rows_b),
            multiset_equal=ca == cb, observed_order_equal=norm_a == norm_b,
            order_contract="ordered" if key == "candidates" else "multiset; observed order separately retained",
            native_duplicates=sum(ca.values())-len(ca), candidate_duplicates=sum(cb.values())-len(cb),
            missing=[dict(row=json.loads(row), count=count) for row, count in sorted((ca-cb).items())],
            extra=[dict(row=json.loads(row), count=count) for row, count in sorted((cb-ca).items())])
    native_ids, candidate_ids = identities(a), identities(b)
    identity_diff = differences(native_ids, candidate_ids)
    metadata = lambda x: {loc: {k: v for k, v in obj.items() if k in METADATA}
                          for loc, obj in (("body", x["body"] or {}), ("error.data", (x["error"] or {}).get("data") or {}))}
    supported = candidate.get("coverage", {}).get("supported", False)
    return dict(case_id=request["case_id"], tool=request["tool"], args=request["args"],
                candidate_supported=supported, semantic_equal=not semantic_diff,
                full_api_equal=not api_diff,
                raw_identity_present=bool(native_ids or candidate_ids),
                raw_identity_equal=(not identity_diff) if native_ids or candidate_ids else None,
                semantic_differences=semantic_diff, api_differences=api_diff,
                identity_differences=identity_diff, metadata_differences=differences(metadata(a), metadata(b)),
                rows=components, candidate_coverage=candidate.get("coverage"),
                matched_semantic_work=not semantic_diff and supported,
                timing_eligibility="parent review required; no ratio produced", api_limits=LIMITS)


def source_checks(reply, spec):
    body = reply.get("body") or {}
    if "source" not in body:
        return []
    symbol = body.get("symbol", {})
    path = symbol.get("file_path", body.get("file"))
    source = next((f["source"].encode() for f in spec["files"] if f["path"] == path), None)
    bounds = body.get("line_range", body.get("range"))
    if source is None or not isinstance(bounds, dict) or set(bounds) != {"start", "end"}:
        return [dict(check="source_provenance", passed=False, detail="missing source path/range")]
    # splitlines() also splits other Unicode/control characters; split ONLY LF.
    parts = source.split(b"\n")
    lines = [line + (b"\n" if i < len(parts)-1 else b"") for i, line in enumerate(parts)
             if i < len(parts)-1 or line]
    first, last = bounds["start"], bounds["end"]
    valid = type(first) is int and type(last) is int and 1 <= first <= len(lines)+1 and first-1 <= last <= len(lines)
    expected = b"".join(lines[first-1:last]) if valid else None
    actual = body["source"].encode() if isinstance(body["source"], str) else None
    symbol_range = symbol.get("line_range")
    context_match = True
    if symbol_range is not None:
        context = min(50, max(0, reply.get("args", {}).get("context_lines", 0)))
        requested_first = max(1, symbol_range[0]-context)
        requested_last = max(requested_first-1, min(len(lines), symbol_range[1]+context))
        context_match = (first, last) == (requested_first, requested_last)
    return [dict(check="exact_utf8_source_and_context", passed=valid and actual == expected and context_match,
                 path=path, range=bounds, expected_sha256=sha256(expected) if expected is not None else None,
                 actual_sha256=sha256(actual) if actual is not None else None,
                 context_bounds_match=context_match,
                 context_validation="symbol range plus request" if symbol_range is not None else "source-format omits symbol range; exact returned range checked",
                 file_oid_matches=body.get("file_oid") == blob_oid(source))]


def validate_native_matrix(spec, replies, spec_sha):
    """Generic source validation plus assertions tied to the immutable known fixture."""
    checks = [dict(case_id=r["case_id"], **check) for r in replies for check in source_checks(r, spec)]
    for check in checks:
        check["passed"] = check["passed"] and check.get("file_oid_matches", False)
    if spec_sha != FIXTURE_SHA256:
        return dict(known_fixture=False, checks=checks,
                    limitation="Custom corpus has byte checks only; parent must supply positive/negative/ambiguous expectations.")
    rows = {r["case_id"]: r for r in replies}
    def check(name, passed):
        checks.append(dict(check=name, passed=bool(passed)))
    for case in ("search_exact", "search_prefix", "search_duplicates", "search_kind", "search_method", "search_unicode", "search_macro"):
        check(case + ":nonempty", bool((rows[case]["body"] or {}).get("candidates")))
    for case in ("search_empty", "search_wrong_file", "search_wrong_kind", "search_literal_glob_empty"):
        body = rows[case]["body"] or {}
        check(case + ":empty", body.get("candidates") == [] and body.get("total_matches") == 0)
    for case, code in (("read_missing", -32004), ("read_missing_file", -32004),
                       ("read_path_rejected", -32602), ("search_invalid_mode", -32602),
                       ("search_conflicting_filters", -32602), ("code_callees_missing", -32004),
                       ("code_callers_missing", -32004), ("code_callees_ambiguity_error", -32602),
                       ("code_callers_ambiguity_error", -32602)):
        check(case + ":error", (rows[case]["error"] or {}).get("code") == code)
    for case in ("read_ambiguous", "code_callees_ambiguous", "code_callers_ambiguous"):
        body = rows[case]["body"] or {}
        check(case + ":ambiguous", body.get("ambiguous") is True and len(body.get("candidates", [])) >= 2)
    for case, field in (("code_callees_samefile_all", "callees"), ("code_callers_samefile_all", "callers"),
                        ("code_callers_crossfile_all", "callers"), ("callees_scoped_duplicate", "callees")):
        check(case + ":positive_edge", bool((rows[case]["body"] or {}).get(field)))
    for tool, field in (("code_callers", "callers"), ("code_callees", "callees")):
        check(tool + ":empty", (rows[tool+"_empty"]["body"] or {}).get(field) == [])
    external = rows["code_callees_external_all"]["body"] or {}
    check("unresolved:positive", any(r.get("resolved") is False for r in external.get("callees", [])))
    check("unresolved:filtered", (rows["code_callees_external_resolved"]["body"] or {}).get("callees") == [])
    check("duplicate-name:four", len((rows["search_duplicates"]["body"] or {}).get("candidates", [])) == 4)
    limit = rows["search_limit"]["body"] or {}
    check("search:limit_and_truncation", len(limit.get("candidates", [])) == 1 and limit.get("total_matches") == 4 and limit.get("truncated") is True)
    return dict(known_fixture=True, checks=checks)


class JsonlEndpoint:
    """Persistent local pipe endpoint. Parent may reuse request() for timed pairs."""
    def __init__(self, command, stderr_path, ready_timeout=120):
        self.command = list(map(str, command))
        self.stderr_path = Path(stderr_path)
        self.ready_queue = queue.Queue()
        started = time.perf_counter_ns()
        self.process = subprocess.Popen(self.command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        def drain():
            with self.stderr_path.open("wb") as log:
                for line in self.process.stderr:
                    log.write(line)
                    log.flush()
                    try:
                        value = json.loads(line)
                        if isinstance(value, dict) and value.get("ready"):
                            self.ready_queue.put(value)
                    except (ValueError, UnicodeError):
                        pass
            self.ready_queue.put(None)
        self.reader = threading.Thread(target=drain, daemon=True)
        self.reader.start()
        try:
            self.ready = self.ready_queue.get(timeout=ready_timeout)
            if self.ready is None:
                raise RuntimeError(f"endpoint exited before ready: {self.command}; see {stderr_path}")
        except BaseException:
            self.process.kill()
            self.process.wait()
            self.reader.join()
            raise
        self.startup_wall_ns = time.perf_counter_ns()-started

    def request(self, request):
        started = time.perf_counter_ns()
        wire = (dumps(request) + "\n").encode()
        self.process.stdin.write(wire)
        self.process.stdin.flush()
        raw = self.process.stdout.readline()
        if not raw:
            raise RuntimeError(f"endpoint closed without response: {self.command}")
        reply = json.loads(raw)
        wall_ns = time.perf_counter_ns()-started
        validate_envelope(request, reply)
        return reply, raw, dict(client_wall_ns=wall_ns, dispatch_ns=reply["elapsed_ns"],
                               request_bytes=len(wire), response_bytes=len(raw))

    def close(self):
        self.process.stdin.close()
        try:
            code = self.process.wait(timeout=120)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise
        finally:
            self.reader.join(timeout=5)
            self.process.stdout.close()
            self.process.stderr.close()
        if code:
            raise RuntimeError(f"endpoint exit {code}: {self.command}; see {self.stderr_path}")


def serve_sql(args):
    candidate = Candidate(args.corpus, args.extension, args.output, args.extension_sha256, args.spec)
    try:
        started = time.perf_counter_ns()
        first = None
        for request in candidate.spec["requests"]:
            result = candidate.dispatch(request)
            if first is None:
                first = dict(case_id=request["case_id"], dispatch_ns=result["elapsed_ns"])
        candidate.provenance["first_query"] = first
        candidate.provenance["startup_matrix_warmup_ns"] = time.perf_counter_ns()-started
        print(dumps(dict(ready=True, **candidate.provenance)), file=sys.stderr, flush=True)
        for raw in sys.stdin.buffer:
            try:
                request = json.loads(raw)
                if set(request) != {"case_id", "tool", "args"}:
                    raise ValueError("expected case_id,tool,args")
                result = candidate.dispatch(request)
            except (ValueError, KeyError, TypeError) as exc:
                result = dict(protocol_error=str(exc))
            sys.stdout.buffer.write((dumps(result) + "\n").encode())
            sys.stdout.buffer.flush()
        # Verify shared source bytes still match after the session.
        _, final = shared_inputs(args.corpus, args.spec)
        if final != candidate.inputs:
            raise ValueError("shared input changed during SQL session")
    finally:
        candidate.finish()


def compare(args):
    output = Path(args.out).resolve()
    output.mkdir()  # exclusive new directory; never overwrite prior evidence
    spec, inputs = shared_inputs(args.corpus, args.spec)
    native_path = Path(args.native).resolve(strict=True)
    native_sha = sha256(native_path.read_bytes())
    # Build/extract SQL independently BEFORE opening the native process/provenance.
    sql_command = [sys.executable, str(Path(__file__).resolve()), "serve-sql", "--corpus", str(Path(args.corpus).resolve()),
                   "--extension", str(Path(args.extension).resolve()), "--extension-sha256", args.extension_sha256,
                   "--output", str(output / "sql")]
    candidate = JsonlEndpoint(sql_command, output / "sql.stderr.jsonl")
    native = None
    native_replies, candidate_replies, comparisons, samples = [], [], [], []
    try:
        native = JsonlEndpoint([str(native_path), "serve", str(Path(args.corpus).resolve())], output / "native.stderr.jsonl")
        if any(native.ready.get(k) != inputs[k] for k in ("head_oid", "spec_sha256")):
            raise ValueError("native ready receipt differs from independently verified shared input")
        with (output / "native.jsonl").open("wb") as a, (output / "candidate.jsonl").open("wb") as b:
            for index, request in enumerate(spec["requests"]):
                # One interleaved smoke observation; notebook owns timed repetitions.
                results = {}
                for name, endpoint, stream in (("native", native, a), ("candidate", candidate, b)) if index % 2 == 0 else (("candidate", candidate, b), ("native", native, a)):
                    reply, raw, sample = endpoint.request(request)
                    stream.write(raw)
                    stream.flush()
                    results[name] = reply
                    samples.append(dict(case_id=request["case_id"], endpoint=name, smoke_only=True, **sample))
                native_replies.append(results["native"])
                candidate_replies.append(results["candidate"])
                comparisons.append(compare_replies(request, results["native"], results["candidate"]))
    finally:
        try:
            if native:
                native.close()
        finally:
            candidate.close()
    native_validation = validate_native_matrix(spec, native_replies, inputs["spec_sha256"])
    candidate_checks = [dict(case_id=r["case_id"], **c) for r in candidate_replies for c in source_checks(r, spec)]
    for check in candidate_checks:
        check["passed"] = check["passed"] and check.get("file_oid_matches", False)
    report = dict(version=1, experiment="independent extension SQL vs real GraphMcpModule",
                  inputs=inputs, cases=comparisons, native_validation=native_validation,
                  candidate_source_validation=candidate_checks, limitations=LIMITS,
                  identity_mapping=__doc__, field_coverage=FIELD_COVERAGE, speed_claims=[],
                  summary=dict(requests=len(comparisons), semantic_matches=sum(c["semantic_equal"] for c in comparisons),
                               full_api_matches=sum(c["full_api_equal"] for c in comparisons),
                               raw_identity_cases=sum(c["raw_identity_present"] for c in comparisons),
                               raw_identity_matches=sum(c["raw_identity_equal"] is True for c in comparisons),
                               unsupported=sum(not c["candidate_supported"] for c in comparisons),
                               native_validation_failures=sum(not c["passed"] for c in native_validation["checks"]),
                               candidate_source_failures=sum(not c["passed"] for c in candidate_checks)))
    report["by_tool"] = {tool: dict(requests=len(cases), semantic_matches=sum(c["semantic_equal"] for c in cases),
                                   full_api_matches=sum(c["full_api_equal"] for c in cases))
                         for tool in sorted({c["tool"] for c in comparisons})
                         for cases in [[c for c in comparisons if c["tool"] == tool]]}
    if report["summary"]["native_validation_failures"] or report["summary"]["candidate_source_failures"]:
        for case in comparisons:
            case["matched_semantic_work"] = False
            case["timing_eligibility"] = "ineligible: reference or candidate source validation failed"
    # Native provenance is read here by the coordinator ONLY, after candidate exit.
    provenance_path = Path(args.corpus) / ".retrieval/native/provenance.json"
    native_provenance_bytes = provenance_path.read_bytes()
    if sha256(native_path.read_bytes()) != native_sha:
        raise ValueError("native executable changed during the session")
    provenance = dict(inputs=inputs, native_provenance=json.loads(native_provenance_bytes),
                      native_provenance_sha256=sha256(native_provenance_bytes),
                      native_executable=str(native_path), native_executable_sha256=native_sha,
                      sql_ready=candidate.ready, native_ready=native.ready,
                      sql_startup_wall_ns=candidate.startup_wall_ns, native_startup_wall_ns=native.startup_wall_ns,
                      commands=dict(native=native.command, candidate=candidate.command),
                      host_load=os.getloadavg(), timing_boundaries=__doc__)
    write_json(output / "report.json", report)
    write_json(output / "provenance.json", provenance)
    write_json(output / "smoke-samples.json", samples)
    manifest = [dict(path=str(p.relative_to(output)), bytes=p.stat().st_size, sha256=sha256(p.read_bytes()))
                for p in sorted(output.rglob("*")) if p.is_file()]
    write_json(output / "manifest.json", manifest)
    print(dumps(dict(output=str(output), **report["summary"], speed_claims=[])))
    return 1 if report["summary"]["native_validation_failures"] or report["summary"]["candidate_source_failures"] else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    for command in ("compare", "serve-sql"):
        p = sub.add_parser(command)
        p.add_argument("--corpus", required=True)
        p.add_argument("--spec", help="optional byte-identical copy of the shared spec")
        p.add_argument("--extension", required=True)
        p.add_argument("--extension-sha256", default=APPROVED_EXTENSION_SHA256)
        if command == "compare":
            p.add_argument("--native", required=True)
            p.add_argument("--out", required=True)
        else:
            p.add_argument("--output", help="new directory for persistent typed DB/facts/provenance")
    args = parser.parse_args()
    if args.command == "serve-sql":
        serve_sql(args)
        return 0
    return compare(args)


if __name__ == "__main__":
    sys.exit(main())
