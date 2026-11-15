"""Recreate the exact immutable tracked-file inventory without reading a worktree."""
import argparse
import json
from pathlib import Path, PurePosixPath
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--repo', default='/Users/kevintruong/project/SpurCode')
parser.add_argument('--revision', default='61ba3d200439a541dce4c1617ca9277be7d438af')
parser.add_argument('--out', type=Path, default=Path(__file__).resolve().parent / 'tracked-inventory.json')
args = parser.parse_args()
raw = subprocess.check_output(['git', '-C', args.repo, 'ls-tree', '-r', '-z', '-l', args.revision])
rows = []
for record in raw.split(b'\0'):
    if not record:
        continue
    metadata, path = record.split(b'\t', 1)
    mode, kind, oid, size = metadata.decode().split()
    path = path.decode('utf-8')
    rows.append({'path': path, 'mode': mode, 'type': kind, 'git_blob_oid': oid,
                 'bytes': int(size) if size != '-' else None, 'suffix': PurePosixPath(path).suffix.lower()})
data = (json.dumps({'repo': args.repo, 'revision': args.revision, 'files': rows}, indent=2) + '\n').encode()
if args.out.exists():
    assert args.out.read_bytes() == data, 'Existing inventory differs; choose a fresh output path'
else:
    args.out.write_bytes(data)
print(json.dumps({'files': len(rows), 'revision': args.revision, 'inventory_verified': True}))
