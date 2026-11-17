"""Execute the exact retained public SQL example, statement by statement."""
import hashlib
import json
import os
from pathlib import Path
import duckdb

here = Path(__file__).resolve().parent
sql_path = here / 'documented_api_example.sql'
package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
with duckdb.connect(config={'allow_unsigned_extensions': True}) as con:
    con.execute("LOAD '" + str(package).replace("'", "''") + "'")
    results = []
    for statement in con.extract_statements(sql_path.read_text()):
        results.append(dict(sql=statement.query, rows=con.execute(statement.query).fetchall()))
    assert len(results) == 7
    assert results[-2]['rows'][0][0]['cells'][0]['symbols'][0]['symbol']['entity_name'] == 'f'
    assert results[-1]['rows'][0][0]['status'] == 'complete'
    print(json.dumps(dict(sql_sha256=hashlib.sha256(sql_path.read_bytes()).hexdigest(),
        package_sha256=hashlib.sha256(package.read_bytes()).hexdigest(), results=results), indent=2))
