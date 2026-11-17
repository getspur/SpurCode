"""Freeze the current public examples and retain the existing SQL runner bytes."""
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PRIOR = HERE.with_name('2026-11-17-notebook-symbol-uplift-release')


def main():
    readme = (ROOT / 'crates/spur-graph-duckdb/README.md').read_text()
    contract = (ROOT / 'crates/spur-graph-duckdb/docs/release-contract.md').read_text()
    examples = {
        'documented_api_example.py': readme.split("<<'PYTHON'\n", 1)[1].split('\nPYTHON', 1)[0] + '\n',
        'documented_api_example.sql': contract.split('```sql\n', 1)[1].split('\n```', 1)[0] + '\n',
        'run_documented_sql.py': (PRIOR / 'run_documented_sql.py').read_text(),
    }
    for name, source in examples.items():
        assert source == (PRIOR / name).read_text(), name
        with (HERE / name).open('x') as stream:
            stream.write(source)
    print(json.dumps({name: hashlib.sha256(source.encode()).hexdigest()
                      for name, source in examples.items()}, indent=2))


if __name__ == '__main__':
    main()
