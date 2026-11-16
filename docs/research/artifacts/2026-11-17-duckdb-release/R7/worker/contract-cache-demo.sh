export FACTS_DEMO="$RELEASE_RUN/cache-demo"
mkdir "$FACTS_DEMO"
"$GRAPH_PYTHON" - <<'PYTHON'
import hashlib, json, os
from pathlib import Path
import duckdb

root = Path.cwd()
crate = root / 'crates/spur-graph-duckdb'
package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
manifest = json.loads(Path(str(package) + '.json').read_bytes())
assert hashlib.sha256(package.read_bytes()).hexdigest() == manifest['binary']['sha256']
source = 'fn helper() {} fn entry() { helper(); remote_call(); }'
blob = 'sha256:' + hashlib.sha256(source.encode()).hexdigest()
extractor = manifest['binary']['sha256']
workflow = (crate / 'examples/file_facts.sql').read_text().split('-- LEGACY NON-ATOMIC', 1)[0]
database = Path(os.environ['FACTS_DEMO']) / 'facts.duckdb'
with duckdb.connect(str(database), config={'allow_unsigned_extensions': True}) as con:
    con.execute("LOAD '" + str(package).replace("'", "''") + "'")
    con.execute('CREATE TEMP TABLE staged_sources AS SELECT ?::VARCHAR blob_oid, '
                '?::VARCHAR logical_path, ?::VARCHAR source, ?::VARCHAR extractor_sha',
                [blob, 'src/demo.rs', source, extractor])
    con.execute('CREATE TEMP TABLE source_lineage AS SELECT * EXCLUDE(source), '
                "'demo'::VARCHAR snapshot_id FROM staged_sources")
    con.execute(workflow)
    con.execute(workflow)
    assert con.execute('SELECT count(*) FROM file_facts_missing').fetchone()[0] == 0
    print(con.execute('SELECT logical_path,status,error_code FROM file_facts_cache').fetchall())
PYTHON
"$GRAPH_PYTHON" crates/spur-graph-duckdb/examples/publish_snapshot.py \
  --database "$FACTS_DEMO/facts.duckdb" --output "$FACTS_DEMO/snapshots" \
  --version demo-v1 --source-label source_scheme=sha256:utf8-v1
