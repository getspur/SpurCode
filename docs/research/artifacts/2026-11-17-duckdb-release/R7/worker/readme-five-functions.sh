"$GRAPH_PYTHON" - <<'PYTHON'
import os
from pathlib import Path
import duckdb

package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
with duckdb.connect(config={'allow_unsigned_extensions': True}) as con:
    con.execute("LOAD '" + str(package).replace("'", "''") + "'")
    print(con.execute('SELECT spur_graph_info()').fetchone()[0])
    print(con.execute("SELECT spur_graph_symbols('src/a.rs', 'fn a() {}')").fetchone()[0])
    print(con.execute("SELECT spur_graph_file_facts('src/a.rs', 'fn a() {}')").fetchone()[0])
    print(con.execute("SELECT spur_graph_file_facts_result('bad.unknown', 'text')").fetchone()[0])
    print(con.execute("SELECT spur_graph_symbol_changes('src/a.rs','src/a.rs','M', "
        "spur_graph_symbols('src/a.rs','fn a() { let x = 1; }'), "
        "spur_graph_symbols('src/a.rs','fn a() { let x = 2; }'))").fetchone()[0])
PYTHON
