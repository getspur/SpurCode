"""Copy completed driver receipts, logs and probes; retain originals and binaries."""
import hashlib
import json
from pathlib import Path
import shutil
import sys


source = Path(sys.argv[1]).resolve(strict=True)
target = Path(sys.argv[2]).absolute()
receipt = json.loads((source / 'receipt.json').read_bytes())
assert receipt['status'] in ('passed', 'failed')
target.mkdir(parents=True)
inventory = {}
for pattern in ('*.stdout', '*.stderr', 'receipt.json', 'unittest-summary.json',
                'source-probe/*.json', 'source-probe-final/*.json'):
    for path in source.glob(pattern):
        destination = target / path.relative_to(source)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open('xb') as output, path.open('rb') as stream:
            shutil.copyfileobj(stream, output)
        inventory[str(path.relative_to(source))] = hashlib.sha256(destination.read_bytes()).hexdigest()
sidecar = source / 'spur_graph.duckdb_extension.json'
if sidecar.exists():
    with (target / 'package-sidecar.json').open('xb') as output:
        output.write(sidecar.read_bytes())
    inventory['package-sidecar.json'] = hashlib.sha256(sidecar.read_bytes()).hexdigest()
with (target / 'archive.json').open('x') as output:
    json.dump(dict(original=str(source), files=inventory,
                   binary_location=str(source / 'spur_graph.duckdb_extension')), output, indent=2)
    output.write('\n')
print(json.dumps(dict(status=receipt['status'], files=len(inventory), archive=str(target))))
