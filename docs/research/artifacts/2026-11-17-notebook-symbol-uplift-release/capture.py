"""Exclusive, raw phase receipts. Run: capture.py PHASE MODE COMMAND [ARG...]."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def main():
    phase, mode, *argv = sys.argv[1:]
    receipt = HERE / (phase + '.json')
    if receipt.exists():
        raise ValueError('refusing to overwrite receipt')
    overrides = dict(PYTHONDONTWRITEBYTECODE='1', SPUR_SCCACHE_S3='0',
                     SPUR_NO_LOCAL_FALLBACK='1', RUSTC_WRAPPER='', CARGO_INCREMENTAL='0')
    if mode in ('host', 'remote'):
        overrides['SPUR_REMOTE'] = '0' if mode == 'host' else '1'
    record = dict(argv=argv, cwd=str(ROOT), mode=mode, environment=overrides,
                  started=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
    record['native_environment'] = {name: os.environ[name] for name in (
        'SPUR_GRAPH_LIBRARY', 'SPUR_GRAPH_BUILD_SCRIPT', 'SPUR_FILE_FACTS_ORACLE',
        'SPUR_RETRIEVAL_NATIVE', 'SPUR_HISTORY_CATALOG', 'SPUR_TEMPORAL_EXAMPLES_DIR')
        if name in os.environ}
    record['native_sha256'] = {name: sha(path) for name, path in record['native_environment'].items()
                               if Path(path).is_file()}
    paths = subprocess.check_output(['git', 'ls-files', 'crates/spur-graph/src',
                                    'crates/spur-graph-duckdb/src',
                                    'crates/spur-graph-duckdb/tests/test_notebook.py',
                                    'crates/spur-graph-duckdb/tests/test_contract.py', 'crates/spur-graph-duckdb/examples/app_consumer.py', 'crates/spur-graph-duckdb/tests/test_app_consumer.py'], cwd=ROOT, text=True).splitlines()
    paths += ['crates/spur-graph-duckdb/src/notebook.rs'] if (ROOT / 'crates/spur-graph-duckdb/src/notebook.rs').exists() else []
    paths += ['crates/spur-graph-duckdb/scripts/check_release.py', 'crates/spur-graph-duckdb/tests/test_release_driver.py']
    paths += [str(p.relative_to(ROOT)) for p in (HERE / 'check_native_notebook.py',) if p.exists()]
    record['source_sha256'] = {p: sha(ROOT / p) for p in sorted(set(paths))}
    package = os.environ.get('SPUR_GRAPH_EXTENSION')
    if package:
        record['package'] = dict(path=package, sha256=sha(package), sidecar_sha256=sha(package + '.json'))
    with (HERE / (phase + '.stdout')).open('xb') as out, (HERE / (phase + '.stderr')).open('xb') as err:
        result = subprocess.run(argv, cwd=ROOT, env=dict(os.environ, **overrides), stdout=out, stderr=err)
    record.update(exit=result.returncode, ended=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  stdout_sha256=sha(HERE / (phase + '.stdout')), stderr_sha256=sha(HERE / (phase + '.stderr')))
    receipt.write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(dict(phase=phase, exit=result.returncode)))
    return result.returncode

if __name__ == '__main__':
    sys.exit(main())
