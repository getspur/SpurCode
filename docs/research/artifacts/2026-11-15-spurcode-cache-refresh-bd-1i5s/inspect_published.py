"""Fresh DuckDB 2 readback of the published cache, usable from a Rust cell."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

import duckdb

ART = Path(__file__).resolve().parent
FIX = ART.parent / "2026-11-15-rust-macro-qualification-fix"
EXT = FIX / "build-4d32f19d4/spur_graph.duckdb_extension"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--out", type=Path, required=True)
args = parser.parse_args()
assert not args.out.exists(), "Use a new receipt path"
published = json.loads((ART / "published.json").read_text())
db = Path(published["database"])
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert duckdb.__version__ == "2.0.0"
assert sha(db) == published["database_sha256"]
assert sha(EXT) == published["extension_sha256"]
c = duckdb.connect(config={"allow_unsigned_extensions": "true"})
c.execute("LOAD '" + str(EXT).replace("'", "''") + "'")
c.execute("ATTACH '" + str(db).replace("'", "''") + "' AS refreshed (READ_ONLY)")
receipts = []


def q(name, sql, params=()):
    cursor = c.execute(sql, list(params))
    columns = [field[0] for field in cursor.description]
    rows = [dict(zip(columns, row)) for row in cursor.fetchall()]
    receipts.append(dict(name=name, sql=sql, params=list(params), rows=rows))
    return rows


q("schema", """SELECT table_name,column_name,data_type FROM information_schema.columns
 WHERE table_catalog='refreshed' AND table_schema='main'
 AND table_name IN ('refresh_meta','inventory','sources','facts','nodes','edges','file_facts_cache','validation_controls')
 ORDER BY table_name,ordinal_position""")
q("runtime", "SELECT version() version")
loaded = q("loaded_extension", "SELECT extension_name,loaded,extension_version FROM duckdb_extensions() WHERE extension_name='spur_graph'")
assert len(loaded) == 1 and loaded[0]["loaded"]
meta = q("cache_provenance", "SELECT * FROM refreshed.main.refresh_meta")
assert len(meta) == 1 and meta[0]["extractor_sha"] == published["extension_sha256"]
counts = q("corpus", """SELECT (SELECT count(*) FROM refreshed.main.inventory) inventory,
 (SELECT count(*) FROM refreshed.main.sources) sources,
 (SELECT count(*) FROM refreshed.main.facts) current_facts,
 (SELECT count(*) FROM refreshed.main.file_facts_cache) versioned_cache_keys""")[0]
assert counts == dict(inventory=4404, sources=4098, current_facts=4098, versioned_cache_keys=8196)
calls = q("rust_before_after", """SELECT CASE WHEN e.extractor_sha=m.extractor_sha THEN 'corrected' ELSE 'baseline' END data_version,
 count(*) recorded_calls,count(*) FILTER(WHERE e.target_node_id IS NOT NULL) local_targets,
 count(*) FILTER(WHERE e.target_node_id IS NULL) unresolved,
 100.0*count(*) FILTER(WHERE e.target_node_id IS NOT NULL)/count(*) local_target_pct
 FROM refreshed.main.file_facts_edges e JOIN refreshed.main.inventory i ON i.path=e.logical_path
 CROSS JOIN refreshed.main.refresh_meta m WHERE i.suffix='.rs' AND e.relation='calls'
 GROUP BY data_version ORDER BY data_version""")
sample = "crates/spur-acp/src/capability_evidence.rs"
parity = q("fresh_native_cache_parity", """SELECT s.logical_path,
 spur_graph_file_facts(s.logical_path,s.source) IS NOT DISTINCT FROM f.value all_five_collections_same
 FROM refreshed.main.sources s JOIN refreshed.main.facts f USING(logical_path)
 WHERE s.logical_path=?""", [sample])
assert len(parity) == 1 and parity[0]["all_five_collections_same"]
vec = q("real_vec_control", """SELECT src.label caller,e.target_label,e.scope_text,e.target_node_id
 FROM refreshed.main.edges e JOIN refreshed.main.nodes src
 ON src.logical_path=e.logical_path AND src.node_id=e.source_node_id
 WHERE e.logical_path=? AND e.relation='calls' AND e.target_label='new'
 AND src.label='rejected_evidence_demotes_while_inconclusive_evidence_preserves_prior_routes'""", [sample])
assert vec and all(row["scope_text"] == "Vec" and row["target_node_id"] is None for row in vec)
c.close()
assert sha(db) == published["database_sha256"]
report = dict(all_checks_passed=True, duckdb_version=duckdb.__version__, pid=os.getpid(),
              python=sys.executable, extension=str(EXT), extension_sha256=sha(EXT),
              database=str(db), database_sha256=sha(db), reader_sha256=sha(__file__),
              receipts=receipts,
              limits="Local-target percentage is not precision; retained managed notebook SQL function is separate.")
args.out.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(dict(all_checks_passed=True, corpus=counts, rust_calls=calls,
                     real_vec=vec, fresh_native_cache_parity=True)))
