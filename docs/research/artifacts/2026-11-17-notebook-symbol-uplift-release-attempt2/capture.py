"""Capture an exclusive HOST verification receipt without rebuilding the candidate.

Run with the retained DuckDB 2 Python: capture.py PHASE COMMAND [ARG...].
"""
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PRIOR = HERE.with_name('2026-11-17-notebook-symbol-uplift-release')
RELEASE = Path('/private/tmp/spur-notebook-u5-bd-c1v3-final')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    phase, *argv = sys.argv[1:]
    receipt_path = HERE / (phase + '.json')
    if receipt_path.exists():
        raise ValueError('refusing to overwrite receipt')
    spec = importlib.util.spec_from_file_location('release_driver', ROOT / 'crates/spur-graph-duckdb/scripts/check_release.py')
    driver = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(driver)
    release = json.loads((RELEASE / 'receipt.json').read_bytes())
    overrides = dict(PYTHONDONTWRITEBYTECODE='1', SPUR_REMOTE='0',
        SPUR_NO_LOCAL_FALLBACK='1', SPUR_SCCACHE_S3='0', RUSTC_WRAPPER='', CARGO_INCREMENTAL='0',
        SPUR_GRAPH_EXTENSION=str(RELEASE / 'spur_graph.duckdb_extension'),
        SPUR_RELEASE_MANIFEST=release['release_trust']['manifest'],
        SPUR_RELEASE_MANIFEST_SHA256=release['release_trust']['manifest_sha256'],
        SPUR_RELEASE_SOURCE_IDENTITY=release['release_trust']['source_identity'])
    overrides.update(driver.native_environment(
        {k: v['path'] for k, v in release['native_artifacts'].items()}, RELEASE))
    source = json.loads((RELEASE / 'source-probe-final/spur_graph_source_identity.json').read_bytes())
    assert driver.verify_source(source) == release['release_trust']['source_identity']
    bound = [ROOT / 'crates/spur-graph-duckdb/docs/release-contract.md',
        ROOT / 'crates/spur-graph-duckdb/README.md',
        ROOT / 'crates/spur-graph-duckdb/examples/app_consumer.py',
        ROOT / 'crates/spur-graph-duckdb/scripts/check_release.py',
        ROOT / 'crates/spur-graph-duckdb/tests/test_contract.py',
        ROOT / 'crates/spur-graph-duckdb/tests/test_notebook.py',
        PRIOR / 'check_native_notebook.py', Path(__file__),
        *sorted(HERE.glob('*.py')), *sorted(HERE.glob('*.sql'))]
    record = dict(argv=argv, cwd=str(ROOT), mode='HOST; unchanged verified native artifact; no Rust execution',
        environment=overrides, started=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_sha256={str(p.relative_to(ROOT)): sha(p) for p in sorted(set(bound))},
        source_probe_sha256=sha(RELEASE / 'source-probe-final/spur_graph_source_identity.json'),
        source_identity=driver.verify_source(source), source_input_count=len(source['inputs']),
        release_receipt_sha256=sha(RELEASE / 'receipt.json'),
        package_sha256=sha(RELEASE / 'spur_graph.duckdb_extension'),
        sidecar_sha256=sha(RELEASE / 'spur_graph.duckdb_extension.json'),
        native_sha256={k: sha(v['path']) for k, v in release['native_artifacts'].items()})
    with (HERE / (phase + '.stdout')).open('xb') as out, (HERE / (phase + '.stderr')).open('xb') as err:
        result = subprocess.run(argv, cwd=ROOT, env=dict(os.environ, **overrides), stdout=out, stderr=err)
    record.update(exit=result.returncode, ended=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        stdout_sha256=sha(HERE / (phase + '.stdout')), stderr_sha256=sha(HERE / (phase + '.stderr')))
    with receipt_path.open('x') as stream:
        stream.write(json.dumps(record, indent=2) + '\n')
    print(json.dumps(dict(phase=phase, exit=result.returncode)))
    return result.returncode


if __name__ == '__main__':
    sys.exit(main())
