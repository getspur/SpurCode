"""Publish the seven file-facts tables as one coherent app snapshot.

After closing the pipeline writer, run with the pinned DuckDB runtime:
  python publish_snapshot.py --database facts.duckdb --output snapshots \
      --version UNIQUE_VERSION --schema main --source-label commit=CALLER_LABEL

Readers load current.json ONCE and read only the paths in that manifest. Each
version contains seven Parquet files plus the same manifest.json, with SQL
column names/types, row counts, SHA-256 and byte lengths. Caller labels are
recorded as supplied claims, never as independently verified source identities.
No extension is needed to publish or consume these typed stored tables.

A read-only connection and one explicit transaction cover every export and its
metadata. A private staging directory is finalized before os.replace atomically
advances current.json. Export/pointer failures propagate; the previous current
manifest and all its version files remain readable. A pointer failure can leave
an unreferenced COMPLETE version, which is retained and cannot be overwritten.
Interrupted staging directories are unreferenced and may be removed explicitly.

This protocol requires a local POSIX filesystem with atomic same-filesystem
rename and advisory flock support. Cooperating publishers serialize on the
persistent .publish.lock inode (do not delete it). Readers need no lock. There
is no garbage collection; old versions stay readable. Files and directories
are synced before the pointer swap; atomic visibility does not promise survival
of sudden power loss after that swap. Legacy COPY exports in file_facts.sql
remain non-atomic and must not be used as this publication's current dataset.
"""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile

import duckdb


TABLES = tuple('file_facts_' + name for name in (
    'cache', 'nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes', 'occurrences'))


def quote_identifier(value):
    return '"' + value.replace('"', '""') + '"'


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write_manifest(path, content):
    with path.open('xb') as stream:
        stream.write(content)
        stream.flush()
        os.fsync(stream.fileno())


def publish_snapshot(database, output, *, version, schema='main', source_labels=None):
    """Publish a fresh version or raise; existing versions are never reused."""
    if not isinstance(version, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', version):
        raise ValueError('version must be a nonempty safe path component starting with a letter or digit')
    labels = dict(source_labels or {})
    if any(not isinstance(k, str) or not isinstance(v, str) for k, v in labels.items()):
        raise ValueError('source labels must map strings to strings')
    database = Path(database).resolve(strict=True)
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    versions = output / 'versions'
    versions.mkdir(exist_ok=True)
    final = versions / version
    # Keep this inode permanently: unlinking it would permit competing locks.
    with (output / '.publish.lock').open('a+b') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if os.path.lexists(final):
            raise FileExistsError(f'snapshot version already exists: {final}')
        staging = Path(tempfile.mkdtemp(prefix='.staging-' + version + '-', dir=versions))
        pointer = None
        try:
            tables = {}
            con = duckdb.connect(str(database), read_only=True)
            try:
                con.execute('BEGIN TRANSACTION')
                catalog = con.execute('SELECT current_database()').fetchone()[0]
                for table in TABLES:
                    relation = '.'.join(map(quote_identifier, (catalog, schema, table)))
                    filename = table + '.parquet'
                    path = staging / filename
                    columns = con.execute('DESCRIBE SELECT * FROM ' + relation).fetchall()
                    count = con.execute('SELECT count(*) FROM ' + relation).fetchone()[0]
                    # COPY paths are SQL literals, identifiers are quoted separately.
                    destination = str(path).replace("'", "''")
                    con.execute(f"COPY {relation} TO '{destination}' (FORMAT PARQUET)")
                    with path.open('rb') as stream:
                        checksum = hashlib.file_digest(stream, 'sha256').hexdigest()
                        os.fsync(stream.fileno())
                    tables[table] = {
                        'path': (Path('versions') / version / filename).as_posix(),
                        'schema': [[c[0], c[1]] for c in columns],
                        'row_count': count, 'sha256': checksum, 'bytes': path.stat().st_size,
                    }
                con.execute('COMMIT')
            finally:
                con.close()
            manifest = {
                'format_version': 1, 'version': version,
                'source': {'database': str(database), 'schema': schema,
                           'duckdb_version': duckdb.__version__},
                'source_labels': {'provenance': 'caller_supplied',
                                  'independently_verified': False, 'values': labels},
                'tables': tables,
            }
            content = (json.dumps(manifest, indent=2, sort_keys=True) + '\n').encode()
            write_manifest(staging / 'manifest.json', content)
            sync_directory(staging)
            # No existing destination under this publisher lock; never replace one.
            os.rename(staging, final)
            sync_directory(versions)
            with tempfile.NamedTemporaryFile(prefix='.current-', dir=output, delete=False) as stream:
                pointer = Path(stream.name)
                stream.write(content)
                stream.flush()
                os.fsync(stream.fileno())
            sync_directory(output)
            # Last fallible publication operation. Do not report failure after commit.
            os.replace(pointer, output / 'current.json')
            pointer = None
            return manifest
        finally:
            if staging.exists():
                shutil.rmtree(staging)
            if pointer is not None:
                pointer.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--database', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--version', required=True, help='fresh immutable version name; reuse is an error')
    parser.add_argument('--schema', default='main')
    parser.add_argument('--source-label', action='append', default=[], metavar='KEY=VALUE',
                        help='caller supplied claim, not independently certified; repeat for more labels')
    args = parser.parse_args()
    labels = {}
    for label in args.source_label:
        key, separator, value = label.partition('=')
        if not separator or not key or key in labels:
            parser.error('source labels require unique nonempty KEY=VALUE entries')
        labels[key] = value
    publish_snapshot(args.database, args.output, version=args.version,
                     schema=args.schema, source_labels=labels)
    print(args.output / 'current.json')


if __name__ == '__main__':
    main()
