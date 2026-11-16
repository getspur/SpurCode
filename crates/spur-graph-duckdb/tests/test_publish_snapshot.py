"""Real DuckDB snapshot publication and fault tests; use the pipeline test runtime."""

import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

import duckdb

import test_file_facts_pipeline as pipeline


PUBLISHER = Path(__file__).resolve().parents[1] / 'examples' / 'publish_snapshot.py'
TABLES = tuple('file_facts_' + name for name in (
    'cache', 'nodes', 'edges', 'spans', 'macro_invocations', 'macro_scopes', 'occurrences'))


def load_publisher():
    if not PUBLISHER.exists():
        return None
    spec = importlib.util.spec_from_file_location('publish_snapshot', PUBLISHER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PublishSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.publisher = load_publisher()
        self.assertIsNotNone(self.publisher, 'supported coherent snapshot publisher must exist')
        self.fixture = pipeline.FileFactsPipelineTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.use_facts_schema()
        self.fixture.stage([('blob', 'one.rs', pipeline.RUST, 'v1'), ('bad', 'bad.unknown', pipeline.RUST, 'v1')])
        self.fixture.con.execute("INSERT INTO source_lineage VALUES "
            "('root','repo','c0',NULL,'blob','one.rs','v1'),"
            "('repeat','repo','c1','c0','blob','one.rs','v1'),"
            "('repeat','repo','c1','c0','blob','one.rs','v1')")
        self.fixture.run_pipeline()
        self.expected = {t: self.fixture.rows(f'SELECT * FROM {t} ORDER BY ALL') for t in TABLES}
        self.schemas = {t: [[c[0], c[1]] for c in self.fixture.rows(f'DESCRIBE {t}')] for t in TABLES}
        self.fixture.con.close()
        self.database = Path(self.fixture.database)
        self.output = Path(self.fixture.output.name) / 'published'

    def publish(self, version, **kwargs):
        return self.publisher.publish_snapshot(self.database, self.output,
            version=version, schema='facts', source_labels={'commit': 'caller-claim'}, **kwargs)

    def manifest(self):
        return json.loads((self.output / 'current.json').read_text())

    def published_bytes(self):
        manifest = self.manifest()
        paths = [self.output / item['path'] for item in manifest['tables'].values()]
        paths += [self.output / 'current.json', self.output / 'versions' / manifest['version'] / 'manifest.json']
        return {str(p): p.read_bytes() for p in paths}

    def test_all_seven_tables_have_schema_count_checksum_and_supplied_provenance(self):
        self.publish('first')
        m = self.manifest()
        self.assertEqual(m['version'], 'first')
        self.assertEqual(set(m['tables']), set(TABLES))
        self.assertEqual(m['source']['schema'], 'facts')
        self.assertEqual(m['source']['database'], str(self.database.resolve()))
        self.assertEqual(m['source']['duckdb_version'], duckdb.__version__)
        self.assertEqual(m['source_labels'], {'provenance': 'caller_supplied',
                                            'independently_verified': False,
                                            'values': {'commit': 'caller-claim'}})
        self.assertEqual(json.loads((self.output / 'versions/first/manifest.json').read_text()), m)
        with duckdb.connect() as reader:
            for table, item in m['tables'].items():
                with self.subTest(table=table):
                    path = self.output / item['path']
                    self.assertEqual(path.parent, self.output / 'versions/first')
                    self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), item['sha256'])
                    self.assertEqual(path.stat().st_size, item['bytes'])
                    self.assertEqual(item['row_count'], len(self.expected[table]))
                    self.assertEqual(item['schema'], self.schemas[table])
                    query = 'SELECT * FROM read_parquet(?)'
                    self.assertEqual(reader.execute(query + ' ORDER BY ALL', [str(path)]).fetchall(),
                                     self.expected[table])
                    actual = reader.execute('DESCRIBE ' + query, [str(path)]).fetchall()
                    self.assertEqual([[c[0], c[1]] for c in actual], item['schema'])

    def test_every_export_and_metadata_read_uses_one_read_only_transaction(self):
        connect = duckdb.connect
        transaction_ids, calls = [], []

        class ObservedConnection:
            def __init__(self, real):
                self.real = real
            def execute(self, sql, *args, **kwargs):
                calls.append(sql)
                if sql.lstrip().upper().startswith(('COPY ', 'SELECT ', 'DESCRIBE ')):
                    transaction_ids.append(self.real.execute('SELECT current_transaction_id()').fetchone()[0])
                return self.real.execute(sql, *args, **kwargs)
            def close(self):
                self.real.close()

        def observed_connect(*args, **kwargs):
            self.assertTrue(kwargs.get('read_only'), 'publisher must open source read-only')
            return ObservedConnection(connect(*args, **kwargs))

        with patch.object(self.publisher.duckdb, 'connect', side_effect=observed_connect):
            self.publish('consistent')
        self.assertGreaterEqual(len(transaction_ids), len(TABLES))
        self.assertEqual(len(set(transaction_ids)), 1, 'all tables and metadata need one DB snapshot')
        self.assertEqual(sum(s.lstrip().upper().startswith('COPY ') for s in calls), len(TABLES))
        self.assertEqual(sum(s.lstrip().upper().startswith('BEGIN') for s in calls), 1)

    def test_late_export_failure_preserves_previous_manifest_and_checksummed_files(self):
        self.publish('previous')
        before = self.published_bytes()
        with duckdb.connect(str(self.database)) as c:
            c.execute('DROP TABLE facts.facts.file_facts_occurrences')
        with self.assertRaises(duckdb.Error):
            self.publish('broken')
        self.assertEqual(self.published_bytes(), before)
        self.assertFalse((self.output / 'versions/broken').exists())
        self.assertEqual({p.name for p in (self.output / 'versions').iterdir()}, {'previous'})

    def test_pointer_failure_preserves_previous_and_never_reuses_finalized_version(self):
        self.publish('previous')
        before = self.published_bytes()
        with patch.object(self.publisher.os, 'replace', side_effect=OSError('injected pointer failure')):
            with self.assertRaisesRegex(OSError, 'injected pointer failure'):
                self.publish('finalized')
        self.assertEqual(self.published_bytes(), before)
        orphan = self.output / 'versions/finalized'
        self.assertTrue((orphan / 'manifest.json').exists())
        orphan_bytes = {p.name: p.read_bytes() for p in orphan.iterdir()}
        with self.assertRaises(FileExistsError):
            self.publish('finalized')
        self.assertEqual({p.name: p.read_bytes() for p in orphan.iterdir()}, orphan_bytes)
        self.publish('next')
        self.assertEqual(self.manifest()['version'], 'next')
        for path, data in before.items():
            if not path.endswith('/current.json'):
                self.assertEqual(Path(path).read_bytes(), data)

    def test_first_failure_has_no_current_manifest(self):
        with duckdb.connect(str(self.database)) as c:
            c.execute('DROP TABLE facts.facts.file_facts_occurrences')
        with self.assertRaises(duckdb.Error):
            self.publish('broken')
        self.assertFalse((self.output / 'current.json').exists())

    def test_duplicate_version_and_path_traversal_are_rejected_without_mutation(self):
        self.publish('stable')
        before = self.published_bytes()
        for version, error in [('stable', FileExistsError), ('../escape', ValueError),
                               ('.', ValueError), ('', ValueError), ('a/b', ValueError)]:
            with self.subTest(version=version), self.assertRaises(error):
                self.publish(version)
            self.assertEqual(self.published_bytes(), before)

    def test_concurrent_publisher_is_rejected_without_changing_current(self):
        self.publish('stable')
        before = self.published_bytes()
        with (self.output / '.publish.lock').open('a+b') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.publish('concurrent')
        self.assertEqual(self.published_bytes(), before)

    def test_cli_can_publish_quoted_schema_without_extension(self):
        quoted_schema = 'facts "quoted\''
        with duckdb.connect(str(self.database)) as c:
            ident = '"' + quoted_schema.replace('"', '""') + '"'
            c.execute('CREATE SCHEMA ' + ident)
            for table in TABLES:
                c.execute(f'CREATE TABLE {ident}.{table} AS SELECT * FROM facts.facts.{table}')
        result = subprocess.run([sys.executable, str(PUBLISHER), '--database', str(self.database),
            '--output', str(self.output), '--version', 'cli', '--schema', quoted_schema,
            '--source-label', 'commit=unverified=label'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.manifest()['source_labels']['values'], {'commit': 'unverified=label'})
        self.assertEqual(self.manifest()['source']['schema'], quoted_schema)


if __name__ == '__main__':
    unittest.main()
