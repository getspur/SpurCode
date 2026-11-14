"""Derive parent-scoped temporal facts from immutable, typed native parse caches.

Usage: derive_temporal_pairs.py --metadata META.duckdb --parse-inputs CONFIG.json
       --extension FILE --output DIR --byte-budget BYTES [--row-limit N]

CONFIG is {"sources": [{"cache_glob": "...", "flags_glob": "...",
"expected_extractor_sha": "..."}]}. flags_glob is optional for caches with an
inline binary_prefix column. Producers must have disjoint (blob_oid, language)
keys. Output is gen-<content hash>/temporal.duckdb plus immutable pair Parquet,
checksummed batch receipts, inputs.json and result.json. Existing generations
and orphaned files are retained. Restart reuses only verified checkpoints.
current-status.json is authoritative for the latest verification attempt;
result.json retains immutable completion evidence from a successful attempt.

The byte budget bounds logical old+new source bytes per pair batch, not RSS.
A pair larger than the budget runs alone and is recorded as oversized.
"""

import argparse
import fcntl
import glob
import hashlib
import json
import os
from pathlib import Path
import sys
import time
import uuid

import duckdb


VERSION = 1


def quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def sql_list(values):
    return "[" + ",".join(quote(value) for value in values) + "]"


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def descriptor(path):
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": digest(path)}


def files_for(pattern, base):
    pattern = Path(pattern)
    if not pattern.is_absolute():
        pattern = base / pattern
    files = sorted({str(Path(path).resolve()) for path in glob.glob(str(pattern), recursive=True) if Path(path).is_file()})
    require(files, f"no Parquet files match {pattern}")
    return [descriptor(path) for path in files]


def write_new_json(path, value):
    with path.open("x") as stream:
        stream.write(json.dumps(value, indent=2, sort_keys=True) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def publish_current_status(generation, generation_id, state, complete, error=None):
    status = {"generation": generation_id, "state": state, "complete": complete, "updated_at_unix": time.time()}
    if error is not None:
        status["error"] = str(error)
    path = generation / "current-status.json"
    temporary = generation / ("current-status-" + uuid.uuid4().hex + ".tmp")
    write_new_json(temporary, status)
    os.replace(temporary, path)


def columns(connection, expression):
    return dict((row[0], row[1]) for row in connection.execute(f"DESCRIBE SELECT * FROM {expression}").fetchall())


def no_rows(connection, query, message):
    row = connection.execute("SELECT * FROM (" + query + ") bad LIMIT 1").fetchone()
    require(row is None, f"{message}: {row}")


def index_inputs(connection, inputs, symbol_type):
    """Read only keys, statuses, byte counts, flags and provenance globally."""
    connection.execute("CREATE OR REPLACE TABLE cache_index(blob_oid VARCHAR,language VARCHAR,extractor_sha VARCHAR,"
                       "representative_path VARCHAR,status VARCHAR,error VARCHAR,input_bytes BIGINT,filename VARCHAR,"
                       "source_id BIGINT,binary_prefix BOOLEAN)")
    for source_id, source in enumerate(inputs["sources"]):
        files = [item["path"] for item in source["cache_files"]]
        relation = f"read_parquet({sql_list(files)}, filename=true)"
        schema = columns(connection, relation)
        required = {"blob_oid", "language", "extractor_sha", "representative_path", "symbols", "status", "error", "input_bytes", "parsed_at"}
        require(required <= schema.keys(), f"cache schema missing fields: {sorted(required - schema.keys())}")
        require(schema["symbols"] == symbol_type, "cache symbols schema differs from the native typed contract")
        for name in ("blob_oid", "language", "extractor_sha", "representative_path", "status", "error"):
            require(schema[name] == "VARCHAR", f"cache schema {name} must be VARCHAR")
        require(schema["input_bytes"] in ("BIGINT", "UBIGINT", "INTEGER", "UINTEGER"), "cache input_bytes must be integral")
        inline = "binary_prefix" in schema
        if inline:
            require(schema["binary_prefix"] == "BOOLEAN", "inline binary_prefix must be BOOLEAN")
        flags = source["flag_files"]
        require(inline or flags, "missing binary flags: supply flags_glob or inline binary_prefix")
        connection.execute(f"CREATE OR REPLACE TEMP TABLE source_index AS SELECT blob_oid,language,extractor_sha,"
                           f"representative_path,status,error,input_bytes::BIGINT input_bytes,filename,"
                           f"{('binary_prefix' if inline else 'NULL::BOOLEAN')} binary_prefix FROM {relation}")
        no_rows(connection, "SELECT blob_oid,language FROM source_index WHERE blob_oid IS NULL OR language IS NULL "
                "OR representative_path IS NULL OR status IS NULL OR input_bytes IS NULL OR input_bytes<0",
                "invalid required cache metadata")
        no_rows(connection, "SELECT blob_oid,extractor_sha FROM source_index WHERE extractor_sha IS NULL OR extractor_sha<>"
                + quote(source["expected_extractor_sha"]), "extractor provenance mismatch")
        no_rows(connection, "SELECT blob_oid,status FROM source_index WHERE status NOT IN ('completed','binary_skipped','encoding_error','extension_error') "
                "OR (status IN ('encoding_error','extension_error') AND error IS NULL) "
                "OR (status='binary_skipped' AND error IS NOT NULL)", "invalid parse status/error")
        if flags:
            relation = f"read_parquet({sql_list(item['path'] for item in flags)})"
            flag_schema = columns(connection, relation)
            require(flag_schema.get("blob_oid") == "VARCHAR" and flag_schema.get("binary_prefix") == "BOOLEAN",
                    "binary flags schema must contain blob_oid VARCHAR and binary_prefix BOOLEAN")
            connection.execute(f"CREATE OR REPLACE TEMP TABLE source_flags AS SELECT blob_oid,binary_prefix FROM {relation}")
            no_rows(connection, "SELECT blob_oid FROM source_flags WHERE blob_oid IS NULL OR binary_prefix IS NULL", "NULL binary flag")
            no_rows(connection, "SELECT blob_oid FROM source_flags GROUP BY blob_oid HAVING count(*)<>1", "duplicate binary flag")
            no_rows(connection, "SELECT s.blob_oid FROM source_index s LEFT JOIN source_flags f USING(blob_oid) "
                    "WHERE f.binary_prefix IS NULL", "missing binary flag")
            if inline:
                no_rows(connection, "SELECT s.blob_oid FROM source_index s JOIN source_flags f USING(blob_oid) "
                        "WHERE s.binary_prefix IS DISTINCT FROM f.binary_prefix", "conflicting binary flags")
            else:
                connection.execute("UPDATE source_index s SET binary_prefix=f.binary_prefix FROM source_flags f WHERE s.blob_oid=f.blob_oid")
        no_rows(connection, "SELECT blob_oid FROM source_index WHERE binary_prefix IS NULL", "missing binary flag")
        no_rows(connection, "SELECT blob_oid,status FROM source_index WHERE status='binary_skipped' AND NOT binary_prefix",
                "binary_skipped requires a true binary flag")
        # Only skipped rows need this payload-validity check; keep ordinary facts
        # out of the global thin index and its scans.
        no_rows(connection, f"SELECT blob_oid FROM read_parquet({sql_list(files)}) WHERE status='binary_skipped' AND symbols IS NOT NULL",
                "binary_skipped requires NULL symbols")
        connection.execute("INSERT INTO cache_index SELECT * EXCLUDE(binary_prefix),?::BIGINT,binary_prefix FROM source_index", [source_id])
    no_rows(connection, "SELECT blob_oid,language FROM cache_index GROUP BY blob_oid,language HAVING count(*)<>1", "duplicate cache key")
    no_rows(connection, "SELECT blob_oid FROM cache_index GROUP BY blob_oid HAVING count(DISTINCT binary_prefix)>1", "conflicting binary flags")


def validate_metadata(connection):
    for table, key in (("commits", "commit_oid"), ("comparisons", "comparison_id"), ("changes", "change_id"), ("symbol_pairs", "pair_id")):
        no_rows(connection, f"SELECT {key} FROM {table} GROUP BY {key} HAVING {key} IS NULL OR count(*)<>1", f"duplicate/NULL {table} identity")
    no_rows(connection, "SELECT language,blob_oid FROM parse_tasks GROUP BY language,blob_oid HAVING count(*)<>1", "duplicate parse task")
    no_rows(connection, "SELECT change_id FROM change_pairs GROUP BY change_id HAVING count(*)<>1", "duplicate change pair")
    no_rows(connection, "SELECT pair_id FROM symbol_pairs WHERE language IS NULL OR path IS NULL OR old_path IS NULL "
            "OR kind IS NULL OR kind NOT IN ('A','M','D','R')", "invalid symbol pair")
    no_rows(connection, "SELECT language,path,old_path,kind,old_blob_oid,new_blob_oid FROM symbol_pairs "
            "GROUP BY ALL HAVING count(*)<>1", "duplicate symbol pair")
    no_rows(connection, "SELECT c.change_id FROM changes c LEFT JOIN comparisons x USING(comparison_id) "
            "WHERE x.comparison_id IS NULL OR c.commit_oid<>x.commit_oid OR c.parent_oid IS DISTINCT FROM x.parent_oid",
            "change comparison provenance mismatch")
    no_rows(connection, "SELECT change_id,status FROM changes WHERE status IS NULL OR left(status,1) NOT IN ('A','M','D','R','C','T') "
            "OR path IS NULL OR commit_oid IS NULL OR gitlink IS NULL", "invalid file change")
    no_rows(connection, "SELECT c.change_id FROM changes c ANTI JOIN change_pairs cp USING(change_id) "
            "WHERE c.language IS NOT NULL AND NOT c.gitlink", "missing change pair")
    no_rows(connection, "SELECT cp.change_id FROM change_pairs cp LEFT JOIN changes c USING(change_id) "
            "LEFT JOIN symbol_pairs p USING(pair_id) WHERE c.change_id IS NULL OR p.pair_id IS NULL OR c.gitlink "
            "OR c.language IS DISTINCT FROM p.language OR c.path<>p.path OR c.old_blob_oid IS DISTINCT FROM p.old_blob_oid "
            "OR c.new_blob_oid IS DISTINCT FROM p.new_blob_oid OR p.kind<>CASE WHEN left(c.status,1) IN ('R','C') THEN 'R' "
            "WHEN left(c.status,1)='T' THEN 'M' ELSE left(c.status,1) END "
            "OR p.old_path<>CASE WHEN left(c.status,1) IN ('R','C') THEN c.old_path ELSE c.path END", "change pair provenance mismatch")
    connection.execute("CREATE OR REPLACE TEMP TABLE required_keys AS SELECT language,old_blob_oid blob_oid FROM symbol_pairs "
                       "WHERE old_blob_oid IS NOT NULL UNION SELECT language,new_blob_oid FROM symbol_pairs WHERE new_blob_oid IS NOT NULL")
    no_rows(connection, "SELECT k.* FROM required_keys k ANTI JOIN cache_index c USING(language,blob_oid)", "missing cache key")
    no_rows(connection, "SELECT k.* FROM required_keys k ANTI JOIN parse_tasks t USING(language,blob_oid)", "missing parse task metadata")
    no_rows(connection, "SELECT c.blob_oid,c.language,c.input_bytes,t.byte_length FROM cache_index c JOIN parse_tasks t USING(language,blob_oid) "
            "WHERE t.byte_length IS NULL OR t.byte_length<0 OR c.input_bytes<>t.byte_length", "cache input_bytes mismatch")


def batches(connection, row_limit, byte_budget):
    reader = connection.cursor()
    reader.execute("SELECT p.pair_id,coalesce(o.byte_length,0)+coalesce(n.byte_length,0) input_bytes FROM symbol_pairs p "
                   "LEFT JOIN parse_tasks o ON p.language=o.language AND p.old_blob_oid=o.blob_oid "
                   "LEFT JOIN parse_tasks n ON p.language=n.language AND p.new_blob_oid=n.blob_oid ORDER BY p.pair_id")
    batch, total = [], 0
    while chunk := reader.fetchmany(row_limit):
        for pair_id, cost in chunk:
            if batch and (len(batch) >= row_limit or total + cost > byte_budget):
                yield batch, total
                batch, total = [], 0
            batch.append(pair_id)
            total += cost
            if total > byte_budget:
                yield batch, total
                batch, total = [], 0
    if batch:
        yield batch, total
    reader.close()


def evaluate_batch(connection, pair_ids, symbol_type):
    connection.execute("CREATE OR REPLACE TEMP TABLE batch_pairs AS SELECT * FROM symbol_pairs WHERE pair_id IN (SELECT unnest(?::BIGINT[]))", [pair_ids])
    connection.execute("CREATE OR REPLACE TEMP TABLE batch_keys AS SELECT language,old_blob_oid blob_oid FROM batch_pairs WHERE old_blob_oid IS NOT NULL "
                       "UNION SELECT language,new_blob_oid FROM batch_pairs WHERE new_blob_oid IS NOT NULL")
    filenames = [r[0] for r in connection.execute("SELECT DISTINCT filename FROM cache_index SEMI JOIN batch_keys USING(language,blob_oid)").fetchall()]
    if filenames:
        oids = [r[0] for r in connection.execute("SELECT DISTINCT blob_oid FROM batch_keys").fetchall()]
        languages = [r[0] for r in connection.execute("SELECT DISTINCT language FROM batch_keys").fetchall()]
        # Select files and push key filters into the Parquet scan before projecting facts.
        connection.execute(f"CREATE OR REPLACE TEMP TABLE batch_payload AS SELECT blob_oid,language,symbols FROM "
                           f"read_parquet({sql_list(filenames)}, union_by_name=true) WHERE blob_oid IN {sql_list(oids)} "
                           f"AND language IN {sql_list(languages)} AND (blob_oid,language) IN (SELECT blob_oid,language FROM batch_keys)")
    else:
        connection.execute(f"CREATE OR REPLACE TEMP TABLE batch_payload(blob_oid VARCHAR,language VARCHAR,symbols {symbol_type})")
    no_rows(connection, "SELECT c.blob_oid,c.language FROM batch_keys k JOIN cache_index c USING(blob_oid,language) "
            "LEFT JOIN batch_payload p USING(blob_oid,language) WHERE c.status='completed' AND p.symbols IS NULL", "completed cache has NULL symbols")
    connection.execute(f"CREATE OR REPLACE TEMP TABLE ready AS SELECT p.*,"
                       f"CASE WHEN p.old_blob_oid IS NULL THEN []::{symbol_type} ELSE os.symbols END old_symbols,"
                       f"CASE WHEN p.new_blob_oid IS NULL THEN []::{symbol_type} ELSE ns.symbols END new_symbols,"
                       "coalesce(o.status,'absent') old_status,coalesce(n.status,'absent') new_status,"
                       "o.error old_error,n.error new_error,o.filename old_source,n.filename new_source,"
                       "coalesce(o.binary_prefix,false) old_binary,coalesce(n.binary_prefix,false) new_binary "
                       "FROM batch_pairs p LEFT JOIN cache_index o ON p.language=o.language AND p.old_blob_oid=o.blob_oid "
                       "LEFT JOIN cache_index n ON p.language=n.language AND p.new_blob_oid=n.blob_oid "
                       "LEFT JOIN batch_payload os ON p.language=os.language AND p.old_blob_oid=os.blob_oid "
                       "LEFT JOIN batch_payload ns ON p.language=ns.language AND p.new_blob_oid=ns.blob_oid")
    connection.execute("CREATE OR REPLACE TEMP TABLE matched AS SELECT pair_id,TRY(spur_graph_symbol_changes(path,old_path,kind,old_symbols,new_symbols)) result "
                       "FROM ready WHERE NOT old_binary AND NOT new_binary AND old_status IN ('completed','absent') AND new_status IN ('completed','absent')")
    connection.execute("CREATE OR REPLACE TEMP TABLE evaluated AS SELECT r.pair_id,"
                       "CASE WHEN old_binary OR new_binary THEN 'binary_skipped' "
                       "WHEN old_status NOT IN ('completed','absent') OR new_status NOT IN ('completed','absent') THEN 'parse_failed' "
                       "WHEN m.result IS NULL THEN 'matching_error' ELSE 'completed' END status,"
                       "CASE WHEN m.result IS NULL THEN [] ELSE m.result.changes END changes,"
                       "CASE WHEN old_binary OR new_binary THEN ['binary_blob: file='||path||' side='||"
                       "CASE WHEN old_binary AND new_binary THEN 'left,right' WHEN old_binary THEN 'left' ELSE 'right' END] "
                       "WHEN old_status NOT IN ('completed','absent') OR new_status NOT IN ('completed','absent') THEN list_filter(["
                       "CASE WHEN old_status NOT IN ('completed','absent') THEN 'parse_failed: file='||old_path||' side=left status='||old_status||' error='||old_error||' source='||old_source END,"
                       "CASE WHEN new_status NOT IN ('completed','absent') THEN 'parse_failed: file='||path||' side=right status='||new_status||' error='||new_error||' source='||new_source END],lambda d: d IS NOT NULL) "
                       "WHEN m.result IS NULL THEN ['matching_error: file='||path||' pair_id='||r.pair_id::VARCHAR] ELSE m.result.diagnostics END diagnostics,"
                       "old_status,new_status,old_source,new_source,old_binary,new_binary,"
                       "(NOT old_binary AND NOT new_binary AND old_status IN ('completed','absent') AND new_status IN ('completed','absent')) native_invoked "
                       "FROM ready r LEFT JOIN matched m USING(pair_id)")


def verify_receipt(generation, receipt, expected):
    for key, value in expected.items():
        require(receipt.get(key) == value, f"checkpoint receipt mismatch: {key}")
    path = generation / receipt["file"]
    require(path.resolve().is_relative_to(generation.resolve()), "checkpoint file escapes generation")
    require(path.is_file() and path.stat().st_size == receipt["file_bytes"] and digest(path) == receipt["sha256"],
            f"checkpoint checksum mismatch: {path}")
    return path


def materialize_views(connection, shards):
    if shards:
        connection.execute(f"CREATE OR REPLACE VIEW pair_results AS SELECT * FROM read_parquet({sql_list(shards)})")
    else:
        result_type = connection.execute("SELECT return_type FROM duckdb_functions() WHERE function_name='spur_graph_symbol_changes'").fetchone()[0]
        changes_type = dict(duckdb.sqltype(result_type).children)["changes"]
        connection.execute(f"CREATE OR REPLACE TABLE empty_pair_results AS SELECT NULL::BIGINT pair_id,'completed'::VARCHAR status,"
                           f"[]::{changes_type} changes,[]::VARCHAR[] diagnostics,NULL::VARCHAR old_status,NULL::VARCHAR new_status,"
                           "NULL::VARCHAR old_source,NULL::VARCHAR new_source,false old_binary,false new_binary,false native_invoked WHERE false")
        connection.execute("CREATE OR REPLACE VIEW pair_results AS SELECT * FROM empty_pair_results")
    connection.execute("CREATE OR REPLACE VIEW pair_occurrences AS SELECT c.change_id,c.commit_oid,c.parent_oid,cp.pair_id,p.status,"
                       "array_length(p.changes) change_count FROM changes c JOIN change_pairs cp USING(change_id) JOIN pair_results p USING(pair_id)")
    connection.execute("CREATE OR REPLACE VIEW pair_emissions AS SELECT p.pair_id,u.ordinality emitted_ordinal,u.change "
                       "FROM pair_results p,unnest(p.changes) WITH ORDINALITY u(change,ordinality) WHERE p.status='completed'")
    connection.execute("CREATE OR REPLACE VIEW raw_symbol_changes AS SELECT c.commit_oid,c.parent_oid,c.change_id,e.pair_id,e.emitted_ordinal,"
                       "e.change.stable_symbol_id stable_symbol_id,e.change.change_kind change_kind,"
                       "e.change.previous_stable_symbol_id previous_stable_symbol_id,e.change.symbol symbol,e.change.file_path file_path "
                       "FROM changes c JOIN change_pairs cp USING(change_id) JOIN pair_emissions e USING(pair_id)")
    # The GROUP BY has no full symbol payload or token list, only thin keys and a pointer.
    connection.execute("CREATE OR REPLACE TABLE folded_pointers AS WITH folded AS ("
                       "SELECT commit_oid,parent_oid,stable_symbol_id,arg_min(struct_pack(pair_id:=pair_id,emitted_ordinal:=emitted_ordinal,"
                       "change_id:=change_id,kind:=change_kind,previous_id:=previous_stable_symbol_id),"
                       "struct_pack(change_id:=change_id,emitted_ordinal:=emitted_ordinal)) chosen,"
                       "count(DISTINCT struct_pack(kind:=change_kind,previous_id:=previous_stable_symbol_id)) variants "
                       "FROM raw_symbol_changes GROUP BY commit_oid,parent_oid,stable_symbol_id) "
                       "SELECT commit_oid,parent_oid,stable_symbol_id,chosen.pair_id pair_id,chosen.emitted_ordinal emitted_ordinal,chosen.change_id change_id,"
                       "CASE WHEN variants>1 THEN 'modified' ELSE chosen.kind END change_kind,"
                       "CASE WHEN variants>1 THEN NULL ELSE chosen.previous_id END previous_stable_symbol_id FROM folded")
    connection.execute("CREATE OR REPLACE VIEW symbol_changes AS SELECT f.*,e.change.symbol symbol,e.change.file_path file_path "
                       "FROM folded_pointers f JOIN pair_emissions e USING(pair_id,emitted_ordinal)")
    connection.execute("CREATE OR REPLACE VIEW symbol_snapshots AS SELECT stable_symbol_id key_stable_symbol_id,commit_oid key_commit,"
                       "rtrim(to_base64(encode(file_path)),'=') file_path_b64,symbol.entity_name entity_name,symbol.symbol_kind symbol_kind,"
                       "symbol.enclosing_scope enclosing_scope,symbol.byte_start::BIGINT byte_range_start,symbol.byte_end::BIGINT byte_range_end,"
                       "symbol.line_start::BIGINT line_range_start,symbol.line_end::BIGINT line_range_end,symbol.anchor_hash anchor_hash,symbol.tokens tokens FROM symbol_changes")
    connection.execute("CREATE OR REPLACE VIEW temporal_edges AS "
                       "SELECT 'commit' source_kind,NULL::VARCHAR source_path_b64,NULL::VARCHAR source_stable_symbol_id,commit_oid source_commit,"
                       "'file' target_kind,rtrim(to_base64(encode(path)),'=') target_path_b64,NULL::VARCHAR target_stable_symbol_id,NULL::VARCHAR target_commit,"
                       "'touches' relation,parent_oid parent,CASE WHEN gitlink THEN 'modified' WHEN left(status,1)='A' THEN 'added' "
                       "WHEN left(status,1)='D' THEN 'deleted' WHEN left(status,1) IN ('R','C') THEN 'renamed_from_file' ELSE 'modified' END change_kind,"
                       "CASE WHEN NOT gitlink AND left(status,1) IN ('R','C') THEN rtrim(to_base64(encode(old_path)),'=') END rename_prev_path_b64,"
                       "NULL::VARCHAR rename_prev_stable_symbol_id,NULL::VARCHAR rename_prev_commit FROM changes "
                       "UNION ALL SELECT 'commit',NULL,NULL,commit_oid,'snapshot',NULL,stable_symbol_id,commit_oid,'touches',parent_oid,"
                       "change_kind,NULL,previous_stable_symbol_id,CASE WHEN previous_stable_symbol_id IS NOT NULL THEN commit_oid END FROM symbol_changes "
                       "UNION ALL SELECT 'snapshot',NULL,previous_stable_symbol_id,commit_oid,'snapshot',NULL,stable_symbol_id,commit_oid,'touches',"
                       "parent_oid,change_kind,NULL,previous_stable_symbol_id,commit_oid FROM symbol_changes WHERE change_kind='renamed_from_symbol'")
    connection.execute("CREATE OR REPLACE VIEW diagnostics AS SELECT c.commit_oid,c.parent_oid,c.change_id,p.pair_id,p.status,d.diagnostic "
                       "FROM changes c JOIN change_pairs cp USING(change_id) JOIN pair_results p USING(pair_id),unnest(p.diagnostics) d(diagnostic) "
                       "UNION ALL SELECT commit_oid,parent_oid,change_id,NULL::BIGINT,'gitlink','gitlink: file='||path||' commit='||commit_oid||"
                       "' skipped submodule recursion; file-level touch retained' FROM changes WHERE gitlink")


def run(args):
    require(duckdb.__version__ == "2.0.0", "this derivation requires DuckDB 2.0.0")
    require(args.byte_budget > 0, "byte-budget must be positive")
    require(0 < args.row_limit <= duckdb.__standard_vector_size__, "row-limit must be positive and no larger than the runtime vector size")
    configuration = json.loads(args.parse_inputs.read_text())
    require(isinstance(configuration.get("sources"), list) and configuration["sources"], "parse-inputs requires nonempty sources")
    sources = []
    for source in configuration["sources"]:
        require(isinstance(source.get("expected_extractor_sha"), str) and source["expected_extractor_sha"], "expected extractor SHA is required")
        sources.append({"expected_extractor_sha": source["expected_extractor_sha"],
                        "cache_files": files_for(source["cache_glob"], args.parse_inputs.resolve().parent),
                        "flag_files": files_for(source["flags_glob"], args.parse_inputs.resolve().parent) if source.get("flags_glob") else []})
    inputs = {"version": VERSION, "script_sha256": digest(__file__), "duckdb_version": duckdb.__version__,
              "metadata": descriptor(args.metadata), "matcher": descriptor(args.extension), "sources": sources,
              "byte_budget": args.byte_budget, "row_limit": args.row_limit}
    generation_id = hashlib.sha256(canonical(inputs).encode()).hexdigest()
    generation = args.output.resolve() / ("gen-" + generation_id)
    generation.mkdir(parents=True, exist_ok=True)
    for directory in ("pairs", "receipts"):
        (generation / directory).mkdir(exist_ok=True)
    with (generation / ".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        publish_current_status(generation, generation_id, "verifying", False)
        try:
            manifest = generation / "inputs.json"
            if manifest.exists():
                require(json.loads(manifest.read_text()) == inputs, "generation input manifest mismatch")
            else:
                write_new_json(manifest, inputs)
            database = generation / "temporal.duckdb"
            connection = duckdb.connect(str(database), config={"allow_unsigned_extensions": True})
            connection.execute(f"LOAD {quote(args.extension.resolve())}")
            symbol_type = connection.execute("SELECT return_type FROM duckdb_functions() WHERE function_name='spur_graph_symbols'").fetchone()[0]
            connection.execute(f"ATTACH {quote(args.metadata.resolve())} AS frozen (READ_ONLY)")
            print(json.dumps({"state": "indexing", "generation_directory": str(generation)}), flush=True)
            for table in ("commits", "comparisons", "changes", "parse_tasks", "symbol_pairs", "change_pairs"):
                connection.execute(f"CREATE OR REPLACE TABLE {table} AS SELECT * FROM frozen.{table}")
            index_inputs(connection, inputs, symbol_type)
            validate_metadata(connection)
            publish_current_status(generation, generation_id, "running", False)
            shards, reused, new, native_calls = [], 0, 0, 0
            for batch_id, (pair_ids, source_bytes) in enumerate(batches(connection, args.row_limit, args.byte_budget)):
                require(len(pair_ids) <= args.row_limit and (source_bytes <= args.byte_budget or len(pair_ids) == 1),
                        "pair batch exceeds the row/byte contract")
                expected = {"generation": generation_id, "batch_id": batch_id, "pair_ids": pair_ids,
                            "rows": len(pair_ids), "input_bytes": source_bytes, "byte_budget": args.byte_budget,
                            "row_limit": args.row_limit, "oversized": source_bytes > args.byte_budget}
                receipt_path = generation / "receipts" / f"{batch_id:06}.json"
                if receipt_path.exists():
                    receipt = json.loads(receipt_path.read_text())
                    shard = verify_receipt(generation, receipt, expected)
                    reused += 1
                else:
                    phase = time.perf_counter()
                    evaluate_batch(connection, pair_ids, symbol_type)
                    actual_ids = [r[0] for r in connection.execute("SELECT pair_id FROM evaluated ORDER BY pair_id").fetchall()]
                    require(actual_ids == pair_ids, "pair batch completion accounting mismatch")
                    calls = connection.execute("SELECT count(*) FROM evaluated WHERE native_invoked").fetchone()[0]
                    native_calls += calls
                    shard = generation / "pairs" / f"{batch_id:06}-{uuid.uuid4().hex}.parquet"
                    connection.execute(f"COPY evaluated TO {quote(shard)} (FORMAT PARQUET)")
                    with shard.open("r+b") as stream:
                        os.fsync(stream.fileno())
                    receipt = dict(expected, file=str(shard.relative_to(generation)), file_bytes=shard.stat().st_size,
                                   sha256=digest(shard), native_invocations=calls, elapsed_seconds=time.perf_counter()-phase)
                    write_new_json(receipt_path, receipt)
                    new += 1
                    print(json.dumps({"state": "checkpoint", "batch_id": batch_id, "rows": len(pair_ids),
                                      "input_bytes": source_bytes, "oversized": expected["oversized"]}), flush=True)
                shards.append(str(shard))
            materialize_views(connection, shards)
            no_rows(connection, "SELECT pair_id FROM pair_results GROUP BY pair_id HAVING count(*)<>1", "duplicate completed pair")
            no_rows(connection, "SELECT pair_id FROM symbol_pairs ANTI JOIN pair_results USING(pair_id)", "missing completed pair")
            statuses = dict(connection.execute("SELECT status,count(*) FROM pair_results GROUP BY status").fetchall())
            counts = {table: connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
                      for table in ("commits", "comparisons", "changes", "symbol_pairs", "pair_occurrences", "symbol_snapshots", "temporal_edges", "diagnostics")}
            connection.execute("CHECKPOINT")
            connection.close()
            summary = {"complete": True, "state": "completed_with_errors" if statuses.get("parse_failed", 0) or statuses.get("matching_error", 0) else "completed",
                       "generation": generation_id, "generation_directory": str(generation), "database": str(database),
                       "current_status_file": str(generation / "current-status.json"),
                       "counts": counts, "pair_statuses": statuses, "batches": len(shards),
                       "byte_budget": args.byte_budget, "row_limit": args.row_limit,
                       "budget_scope": "logical old+new source bytes, not RSS; oversized pairs run alone"}
            result = generation / "result.json"
            if result.exists():
                require(json.loads(result.read_text()) == summary, "completed generation summary mismatch")
            else:
                write_new_json(result, summary)
            publish_current_status(generation, generation_id, summary["state"], True)
            return dict(summary, new_batches=new, reused_batches=reused, native_invocations_this_run=native_calls)
        except Exception as error:
            publish_current_status(generation, generation_id, "failed", False, error)
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--parse-inputs", type=Path, required=True)
    parser.add_argument("--extension", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--byte-budget", type=int, required=True)
    parser.add_argument("--row-limit", type=int, default=duckdb.__standard_vector_size__)
    args = parser.parse_args()
    try:
        print(json.dumps(run(args), sort_keys=True), flush=True)
    except (ValueError, OSError, duckdb.Error, KeyError) as error:
        raise SystemExit(f"temporal derivation failed: {error}") from error


if __name__ == "__main__":
    main()
