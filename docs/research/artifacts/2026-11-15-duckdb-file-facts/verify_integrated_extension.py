"""Package the isolated committed native build and verify its real-engine contracts."""
import hashlib
import json
import os
import shutil
import subprocess
import time
from pathlib import Path

ROOT = Path('/Users/kevintruong/project/SpurCode')
OUT = ROOT / 'docs/research/artifacts/2026-11-15-duckdb-file-facts/integration'
WORKTREE = Path('/private/tmp/spur-file-facts-integration-20261115')
TARGET = Path('/private/tmp/spur-graph-filefacts-native-target/release')
PYTHON = '/private/tmp/spur-graph-duckdb-v2-20261114/bin/python'
commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=WORKTREE, text=True).strip()
assert commit == 'f0218c673ed478f854b64df6223d1817ced3586e'
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=WORKTREE)
results = []
commands = [
    ('package', [PYTHON, str(WORKTREE / 'crates/spur-graph-duckdb/scripts/package_extension.py'),
                 '--library', str(TARGET / 'libspur_graph_duckdb.dylib'),
                 '--out', str(OUT / 'spur_graph.duckdb_extension')]),
]
for name, command in commands:
    with (OUT / (name + '.log')).open('w') as log:
        subprocess.run(command, cwd=WORKTREE, stdout=log, stderr=subprocess.STDOUT, check=True)
shutil.copy2(TARGET / 'examples/reference_file_facts', OUT / 'reference_file_facts')
shutil.copy2(TARGET / 'libspur_graph_duckdb.dylib', OUT / 'libspur_graph_duckdb.dylib')
manifest = {'commit': commit, 'integration_base': 'cfccd6129b9ce9ec5e5b448fcba7548245a30424',
            'branch': 'spur/file-facts-integration-20261115', 'artifacts': {}}
for name in ['spur_graph.duckdb_extension', 'reference_file_facts', 'libspur_graph_duckdb.dylib']:
    data = (OUT / name).read_bytes()
    manifest['artifacts'][name] = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
(OUT / 'artifact-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
env = os.environ | {'SPUR_GRAPH_EXTENSION': str(OUT / 'spur_graph.duckdb_extension'),
                    'SPUR_FILE_FACTS_ORACLE': str(OUT / 'reference_file_facts'),
                    'PYTHONPATH': str(WORKTREE / 'crates/spur-graph-duckdb/tests')}
start = time.perf_counter()
command = [PYTHON, '-m', 'unittest', 'test_file_facts_pipeline', 'test_file_facts',
           'test_extension', 'test_temporal', '-v']
with (OUT / 'real-engine-tests.log').open('w') as log:
    proc = subprocess.run(command, cwd=WORKTREE, env=env, stdout=log, stderr=subprocess.STDOUT)
manifest['real_engine_tests'] = {'command': command, 'exit': proc.returncode,
                                 'wall_seconds': time.perf_counter() - start}
(OUT / 'artifact-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps(manifest, indent=2))
print('\n'.join((OUT / 'real-engine-tests.log').read_text().splitlines()[-8:]))
raise SystemExit(proc.returncode)
