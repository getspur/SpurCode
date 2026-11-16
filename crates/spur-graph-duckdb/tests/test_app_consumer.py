"""Real native headless acceptance; no skips, network, or repository scan.

Use the pinned requirements-test.txt runtime and SPUR_GRAPH_EXTENSION naming
an R4 packaged binary with its adjacent sidecar. All subprocess budgets below
are explicit test-caller inputs, not consumer defaults.
"""

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

import duckdb


CRATE = Path(__file__).resolve().parents[1]
CONSUMER = CRATE / 'examples/app_consumer.py'
FIXTURE = CRATE / 'tests/fixtures/app_consumer.json'


class AppConsumerTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(CONSUMER.is_file(), 'reusable headless app consumer must exist')
        spec = importlib.util.spec_from_file_location('app_consumer', CONSUMER)
        self.app = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.app)
        self.assertEqual(duckdb.__version__, '2.0.0')
        self.assertTrue(os.environ.get('SPUR_GRAPH_EXTENSION'), 'set the exact native package')
        self.package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
        self.artifact = json.loads(Path(str(self.package) + '.json').read_text())
        self.fixture = json.loads(FIXTURE.read_text())
        self.tmp = tempfile.TemporaryDirectory(prefix='spur-app-consumer-test-')
        self.addCleanup(self.tmp.cleanup)
        self.output = Path(self.tmp.name) / 'results'

    def run_consumer(self, **kwargs):
        return self.app.run_consumer(FIXTURE, self.output, self.package,
            expected_extractor_id=self.artifact['extractor_id'], timeout=60, **kwargs)

    def rows(self, manifest, table, where='', params=None):
        path = self.output / manifest['tables'][table]['path']
        with duckdb.connect() as c:
            cursor = c.execute('SELECT * FROM read_parquet(?) ' + where,
                               [str(path)] + list(params or []))
            names = [col[0] for col in cursor.description]
            return [dict(zip(names, row)) for row in cursor.fetchall()]

    def assert_reaped(self, error):
        self.assertIsNotNone(error.returncode)
        with self.assertRaises(ProcessLookupError):
            os.kill(error.pid, 0)
        self.assertFalse(list((self.output / 'runs').glob('.staging-*')))

    def test_manifest_binds_snapshot_schema_source_runtime_and_artifact(self):
        m = self.run_consumer()
        self.assertEqual(m['status'], 'complete')
        self.assertEqual(m['contract_version'], 1)
        self.assertEqual(m['artifact'], self.artifact)
        self.assertEqual(m['artifact']['binary']['sha256'],
                         hashlib.sha256(self.package.read_bytes()).hexdigest())
        self.assertEqual(m['fixture_sha256'], hashlib.sha256(FIXTURE.read_bytes()).hexdigest())
        self.assertEqual(m['facts_schema_version'], self.artifact['facts_schema_version'])
        self.assertEqual(m['extractor_id'], self.artifact['extractor_id'])
        self.assertEqual(m['runtime']['duckdb'], '2.0.0')
        self.assertEqual(m['runtime']['platform'], self.artifact['platform'])
        self.assertEqual(m['resolution'], {'scope': 'single_file', 'compiler_complete': False,
                                         'repository_complete': False})
        self.assertEqual(m['source_labels']['provenance'], 'caller_supplied')
        self.assertFalse(m['source_labels']['independently_verified'])
        self.assertEqual(m['process']['returncode'], 0)
        self.assert_reaped(type('Process', (), m['process'])())
        self.assertEqual(set(m['snapshots']), {'base', 'head'})
        for snapshot_id in m['snapshots']:
            result = self.app.read_result(self.output, snapshot_id)
            self.assertEqual(result['snapshot_id'], snapshot_id)
            self.assertEqual(result['manifest'], m)
        before = (self.output / 'current.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'snapshot'):
            self.app.read_result(self.output, 'another-snapshot')
        self.assertEqual((self.output / 'current.json').read_bytes(), before)

    def test_typed_exports_reopen_and_replay_without_added_duplicates(self):
        first = self.run_consumer()
        self.assertEqual(first['replay']['missing_keys'], 0)
        self.assertTrue(first['replay']['tables_unchanged'])
        tables = {'file_facts_' + n for n in ('cache', 'nodes', 'edges', 'spans',
                  'macro_invocations', 'macro_scopes', 'occurrences')} | {'sources', 'explore', 'review'}
        self.assertEqual(set(first['tables']), tables)
        with duckdb.connect() as c:
            for table, item in first['tables'].items():
                path = self.output / item['path']
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), item['sha256'])
                self.assertEqual(path.stat().st_size, item['bytes'])
                columns = c.execute('DESCRIBE SELECT * FROM read_parquet(?)', [str(path)]).fetchall()
                self.assertEqual([[v[0], v[1]] for v in columns], item['schema'])
                self.assertEqual(c.execute('SELECT count(*) FROM read_parquet(?)',
                                          [str(path)]).fetchone()[0], item['row_count'])
        self.assertIn('STRUCT(', dict(first['tables']['explore']['schema'])['facts'])
        self.assertIn('diagnostics VARCHAR[]', dict(first['tables']['review']['schema'])['result'])
        expected = {t: self.rows(first, t, 'ORDER BY ALL') for t in tables}
        second = self.run_consumer()
        self.assertNotEqual(first['run_id'], second['run_id'])
        for table in tables:
            self.assertEqual(self.rows(second, table, 'ORDER BY ALL'), expected[table], table)
            self.assertEqual(self.rows(first, table, 'ORDER BY ALL'), expected[table], table)

    def test_full_key_explore_preserves_lineage_and_same_bytes_at_different_paths(self):
        m = self.run_consumer()
        rows = self.rows(m, 'explore', 'WHERE snapshot_id=?', ['base'])
        expected_lineage = [r for r in self.fixture['lineage'] if r['snapshot_id'] == 'base']
        self.assertEqual(len(rows), len(expected_lineage))
        sources = {(s['blob_oid'], s['logical_path']): s for s in self.fixture['snapshots'][0]['sources']}
        for row in rows:
            self.assertIn((row['blob_oid'], row['logical_path']), sources)
            self.assertEqual(row['extractor_sha'], m['extractor_id'])
        a = next(r for r in rows if r['logical_path'] == 'src/local.rs')
        b = next(r for r in rows if r['logical_path'] == 'src/copy.rs')
        self.assertEqual(a['blob_oid'], b['blob_oid'])
        self.assertNotEqual(a['facts']['nodes'], b['facts']['nodes'])
        self.assertEqual(sum(r['occurrence_id'] == 'base-repeat' for r in rows), 2)
        cache = self.rows(m, 'file_facts_cache')
        keys = [(r['blob_oid'], r['logical_path'], r['extractor_sha']) for r in cache]
        self.assertEqual(len(keys), len(set(keys)))

    def test_diagnostics_and_coverage_do_not_claim_full_resolution(self):
        m = self.run_consumer()
        rows = self.rows(m, 'explore', 'WHERE snapshot_id=?', ['head'])
        by_path = {r['logical_path']: r for r in rows}
        for path, status, code in [('bad.unknown', 'extraction_error', 'unsupported_language'),
                                  ('../escape.rs', 'extraction_error', 'invalid_path'),
                                  ('bad.ipynb', 'extraction_error', 'malformed_input'),
                                  ('empty.rs', 'empty_source', None),
                                  ('null.rs', 'null_input', None)]:
            r = by_path[path]
            self.assertEqual((r['status'], r['error_code']), (status, code))
            self.assertEqual(bool(r['error_message']), bool(code))
        facts = by_path['src/local.rs']['facts']
        self.assertTrue(any(e['target_label'] == 'helper' and e['target_node_id'] is not None
                            for e in facts['edges']))
        for label in ('remote_call', 'Remote::new'):
            self.assertTrue(any(e['target_label'] == label and e['target_node_id'] is None
                                for e in facts['edges']), label)
        self.assertTrue(any(i['resolution']['kind'] == 'unresolved'
                            for i in facts['macro_invocations']))
        coverage = m['snapshots']['head']['coverage']
        self.assertFalse(coverage['extraction_complete'])
        self.assertGreater(coverage['unresolved_edges'], 0)
        self.assertEqual(coverage['source_rows'], len(self.fixture['snapshots'][1]['sources']))

    def test_review_retains_pair_identity_native_ambiguity_and_unavailable_inputs(self):
        m = self.run_consumer()
        reviews = {r['pair_id']: r for r in self.rows(m, 'review')}
        self.assertEqual(set(reviews), {p['pair_id'] for p in self.fixture['pairs']})
        local = reviews['local-edit']
        self.assertEqual((local['base_snapshot_id'], local['head_snapshot_id']), ('base', 'head'))
        self.assertEqual(local['status'], 'ok')
        self.assertTrue(local['result']['changes'])
        ambiguous = reviews['ambiguous-rename']['result']
        self.assertTrue(any(d.startswith('ambiguous_rename:') for d in ambiguous['diagnostics']))
        self.assertTrue(all(c['previous_stable_symbol_id'] is None for c in ambiguous['changes']))
        self.assertEqual(reviews['unavailable']['status'], 'unavailable')
        self.assertIsNone(reviews['unavailable']['result'])

    def test_mislabeled_source_and_wrong_extractor_fail_before_publication(self):
        changed = copy.deepcopy(self.fixture)
        changed['snapshots'][0]['sources'][0]['source'] += '// forged bytes'
        bad = Path(self.tmp.name) / 'forged.json'
        bad.write_text(json.dumps(changed))
        with self.assertRaisesRegex(self.app.ChildFailure, 'source.*identity') as error:
            self.app.run_consumer(bad, self.output, self.package,
                expected_extractor_id=self.artifact['extractor_id'], timeout=60)
        self.assert_reaped(error.exception)
        self.assertFalse((self.output / 'current.json').exists())
        with self.assertRaisesRegex(self.app.ChildFailure, 'extractor') as error:
            self.app.run_consumer(FIXTURE, self.output, self.package,
                expected_extractor_id='wrong-extractor', timeout=60)
        self.assert_reaped(error.exception)
        self.assertFalse((self.output / 'current.json').exists())

    def test_missing_full_lineage_key_and_conflicting_snapshot_are_rejected(self):
        for mutate in ('extractor', 'path', 'duplicate_snapshot', 'pair_snapshot'):
            fixture = copy.deepcopy(self.fixture)
            if mutate == 'extractor':
                fixture['lineage'][0]['extractor_sha'] = 'wrong-extractor'
            elif mutate == 'path':
                fixture['lineage'][0]['logical_path'] = 'src/missing.rs'
            elif mutate == 'duplicate_snapshot':
                fixture['snapshots'][1]['snapshot_id'] = 'base'
            else:
                fixture['pairs'][0]['base_snapshot_id'] = 'absent'
            path = Path(self.tmp.name) / (mutate + '.json')
            path.write_text(json.dumps(fixture))
            with self.subTest(mutate=mutate), self.assertRaises(self.app.ChildFailure) as error:
                self.app.run_consumer(path, self.output, self.package,
                    expected_extractor_id=self.artifact['extractor_id'], timeout=60)
            self.assert_reaped(error.exception)
            self.assertFalse((self.output / 'current.json').exists())

    def test_early_exit_and_interruption_cannot_publish_even_with_candidate_written(self):
        for fault in ('exit_after_candidate', 'interrupt_after_candidate'):
            with self.subTest(fault=fault), self.assertRaises(self.app.ChildFailure) as error:
                self.run_consumer(fault=fault)
            self.assertNotEqual(error.exception.returncode, 0)
            self.assertIn('candidate written', error.exception.stdout)
            self.assert_reaped(error.exception)
            self.assertFalse((self.output / 'current.json').exists())
        previous = self.run_consumer()
        before = (self.output / 'current.json').read_bytes()
        with self.assertRaises(self.app.ChildFailure) as error:
            self.run_consumer(fault='exit_after_candidate')
        self.assert_reaped(error.exception)
        self.assertEqual((self.output / 'current.json').read_bytes(), before)
        self.assertEqual(self.app.read_result(self.output, 'base')['manifest'], previous)
        self.assertEqual(self.run_consumer()['status'], 'complete')

    def test_explicit_timeout_reaps_child_and_next_healthy_run_succeeds(self):
        with self.assertRaises(self.app.ChildFailure) as error:
            self.app.run_consumer(FIXTURE, self.output, self.package,
                expected_extractor_id=self.artifact['extractor_id'], timeout=1,
                fault='wait_after_candidate')
        self.assert_reaped(error.exception)
        self.assertFalse((self.output / 'current.json').exists())
        self.assertEqual(self.run_consumer()['status'], 'complete')

    def test_reader_rejects_corrupt_exports_and_snapshot_relabeling(self):
        m = self.run_consumer()
        path = self.output / m['tables']['explore']['path']
        original = path.read_bytes()
        path.write_bytes(original + b'tampered')
        with self.assertRaisesRegex(ValueError, 'checksum|size'):
            self.app.read_result(self.output, 'base')
        path.write_bytes(original)
        pointer = self.output / 'current.json'
        changed = copy.deepcopy(m)
        changed['snapshots']['another-snapshot'] = changed['snapshots'].pop('base')
        pointer.write_text(json.dumps(changed))
        with self.assertRaisesRegex(ValueError, 'manifest|identity'):
            self.app.read_result(self.output, 'another-snapshot')


if __name__ == '__main__':
    unittest.main()
