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
import signal
import subprocess
import sys
import tempfile
import time
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
        self.assertEqual(m['contract_version'], 2)
        self.assertEqual(m['notebook_contract_version'], 1)
        self.assertEqual(m['artifact'], self.artifact)
        self.assertEqual(m['artifact']['binary']['sha256'],
                         hashlib.sha256(self.package.read_bytes()).hexdigest())
        self.assertEqual(m['fixture_sha256'], hashlib.sha256(FIXTURE.read_bytes()).hexdigest())
        self.assertEqual(m['source_identity_scheme'], 'sha256:utf8-v1')
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
                  'macro_invocations', 'macro_scopes', 'occurrences')} | {
                      'sources', 'explore', 'review', 'notebook_explore', 'notebook_review'}
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
            actual = self.rows(second, table, 'ORDER BY ALL')
            if table == 'review':
                # The native pair matcher preserves evidence but does not
                # promise list order across processes. Compare multiplicity;
                # exported results themselves must remain untouched.
                for a, b in zip(actual, expected[table]):
                    a, b = copy.deepcopy(a), copy.deepcopy(b)
                    ar, br = a.pop('result'), b.pop('result')
                    self.assertEqual(a, b)
                    if ar is None or br is None:
                        self.assertEqual(ar, br)
                    else:
                        self.assertCountEqual(ar['changes'], br['changes'])
                        self.assertCountEqual(ar['diagnostics'], br['diagnostics'])
                self.assertEqual(len(actual), len(expected[table]))
            else:
                self.assertEqual(actual, expected[table], table)
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
        for label, scope in (('remote_call', None), ('new', 'Remote')):
            self.assertTrue(any(e['target_label'] == label and e['target_node_id'] is None
                                and e['scope_text'] == scope
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

    def test_unsupported_fixture_scheme_preserves_completed_pointer_and_files(self):
        previous = self.run_consumer()
        before = {p.relative_to(self.output): p.read_bytes()
                  for p in self.output.rglob('*') if p.is_file()}
        changed = copy.deepcopy(self.fixture)
        changed['source_identity_scheme'] = 'git:sha1-v1'
        bad = Path(self.tmp.name) / 'unsupported-scheme.json'
        bad.write_text(json.dumps(changed))
        with self.assertRaisesRegex(self.app.ChildFailure, 'unsupported source identity scheme') as error:
            self.app.run_consumer(bad, self.output, self.package,
                expected_extractor_id=self.artifact['extractor_id'], timeout=60)
        self.assert_reaped(error.exception)
        self.assertEqual({p.relative_to(self.output): p.read_bytes()
                          for p in self.output.rglob('*') if p.is_file()}, before)
        self.assertEqual(self.app.read_result(self.output, 'base')['manifest'], previous)

    def test_reader_rejects_coherent_stored_result_with_unsupported_source_scheme(self):
        changed = self.run_consumer()
        changed['source_identity_scheme'] = 'git:sha1-v1'
        # Keep both manifests coherent so this checks the declared scheme,
        # rather than merely detecting a pointer/immutable-manifest mismatch.
        (self.output / 'current.json').write_text(json.dumps(changed))
        (self.output / 'runs' / changed['run_id'] / 'manifest.json').write_text(json.dumps(changed))
        with self.assertRaisesRegex(ValueError, 'unsupported source identity scheme'):
            self.app.read_result(self.output, 'base')

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

    def test_reader_checks_the_coherent_snapshot_manifest(self):
        m = self.run_consumer()
        path = self.output / m['snapshot_manifest']['path']
        path.write_bytes(path.read_bytes() + b' ')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            self.app.read_result(self.output, 'base')

    def test_interrupting_parent_reaps_native_child_before_cleanup(self):
        script = '''import importlib.util, sys
from pathlib import Path
s=importlib.util.spec_from_file_location('app',sys.argv[1])
m=importlib.util.module_from_spec(s); s.loader.exec_module(m)
m.run_consumer(sys.argv[2],sys.argv[3],sys.argv[4],expected_extractor_id=sys.argv[5],
               fault='wait_after_candidate')
'''
        parent = subprocess.Popen([sys.executable, '-c', script, str(CONSUMER), str(FIXTURE),
            str(self.output), str(self.package), self.artifact['extractor_id']],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        child_pid = None
        try:
            deadline = time.monotonic() + 60  # explicit test budget
            reached_candidate = False
            while time.monotonic() < deadline and parent.poll() is None:
                candidates = list(self.output.glob('runs/.staging-*/candidate.json'))
                if candidates:
                    child_pid = json.loads(candidates[0].read_text()).get('native_pid')
                    reached_candidate = True
                    break
                time.sleep(0.01)
            self.assertTrue(reached_candidate, 'native child must reach the blocking fault seam')
            parent.send_signal(signal.SIGINT)
            stdout, stderr = parent.communicate(timeout=60)
            self.assertNotEqual(parent.returncode, 0, stdout + stderr)
            self.assertIsNotNone(child_pid, 'candidate must record the native child identity')
            with self.assertRaises(ProcessLookupError):
                os.kill(child_pid, 0)
            self.assertFalse(list(self.output.glob('runs/.staging-*')))
            self.assertFalse((self.output / 'current.json').exists())
        finally:
            if parent.poll() is None:
                # Let the harness reap its private child even if a test
                # assertion fails, before using the final parent-only fallback.
                parent.send_signal(signal.SIGINT)
            try:
                parent.communicate(timeout=60)
            except subprocess.TimeoutExpired:
                parent.kill()
                parent.communicate()
            if child_pid is not None:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        self.assertEqual(self.run_consumer()['status'], 'complete')

    def notebook_fixture(self):
        from test_notebook import code, notebook
        left = [code(source=':dep serde = "1" # café\r\nfn helper() -> i32 { 1 }\r\n'),
                code('p', 'python', ['def alpha():\n', '    return 1\n']),
                code('j', 'javascript', 'function beta() { return 2; }'),
                code('i', 'python', 'import os\n'), code('e', 'rust', ''),
                dict(id='m', cell_type='markdown', metadata={}, source='# hi')]
        right = copy.deepcopy(left)
        right[0]['source'] = right[0]['source'].replace('"1"', '"2"')
        right[1]['metadata']['tag'] = 'changed'
        right = [right[1], right[0], *right[2:], code('added')]
        uncertain = [code('unknown', 'not-a-language'), dict(code('anon'), id=None),
                     code('command', source=':load external.rs\nfn hidden() {}')]
        uncertain[1].pop('id')
        return self.frozen_fixture([
            ('base', {'mixed.ipynb': notebook(left, {'profile': 'before'}),
                      'partial.ipynb': notebook(uncertain), 'null.ipynb': None}),
            ('head', {'mixed.ipynb': notebook(right, {'profile': 'after'}),
                      'partial.ipynb': notebook(uncertain), 'null.ipynb': None})],
            [dict(pair_id='edit', base_snapshot_id='base', head_snapshot_id='head',
                  logical_path='mixed.ipynb', old_path='mixed.ipynb', kind='M'),
             dict(pair_id='partial-noop', base_snapshot_id='base', head_snapshot_id='head',
                  logical_path='partial.ipynb', old_path='partial.ipynb', kind='M')])

    def frozen_fixture(self, versions, pairs):
        fixture = dict(format_version=1, source_identity_scheme='sha256:utf8-v1',
                       snapshots=[], lineage=[], pairs=pairs)
        for sid, files in versions:
            sources = []
            for path, source in files.items():
                blob = 'null-source' if source is None else 'sha256:' + hashlib.sha256(source.encode()).hexdigest()
                sources.append(dict(logical_path=path, source=source, blob_oid=blob))
                fixture['lineage'].append(dict(snapshot_id=sid, occurrence_id=sid + path,
                                              logical_path=path, blob_oid=blob))
            fixture['snapshots'].append(dict(snapshot_id=sid, repository='frozen-notebook',
                commit_oid='caller-' + sid, parent_oid=None if sid == 'base' else 'caller-base', sources=sources))
        return fixture

    def consume_notebooks(self, fixture):
        path = Path(self.tmp.name) / 'notebooks.json'
        path.write_text(json.dumps(fixture, ensure_ascii=False))
        try:
            return self.app.run_consumer(path, self.output, self.package,
                expected_extractor_id=self.artifact['extractor_id'], timeout=60)
        except self.app.ChildFailure as error:
            self.fail('notebook consumer must route original sources to notebook APIs: ' + str(error))

    def assert_notebook_tables(self, m):
        from test_notebook import DOCUMENT, COMPARISON
        self.assertEqual(m['contract_version'], 2)
        self.assertEqual(m['notebook_contract_version'], 1)
        self.assertEqual(dict(m['tables']['notebook_explore']['schema'])['document'], DOCUMENT)
        self.assertEqual(dict(m['tables']['notebook_review']['schema'])['result'], COMPARISON)
        for table in ('notebook_explore', 'notebook_review'):
            item = m['tables'][table]
            path = self.output / item['path']
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), item['sha256'])
            with duckdb.connect() as c:
                schema = c.execute('DESCRIBE SELECT * FROM read_parquet(?)', [str(path)]).fetchall()
                self.assertEqual([[v[0], v[1]] for v in schema], item['schema'])

    def test_notebook_inventory_refs_and_coverage_survive_native_child(self):
        m = self.consume_notebooks(self.notebook_fixture())
        self.assert_notebook_tables(m)
        rows = {r['logical_path']: r for r in self.rows(m, 'notebook_explore', 'WHERE snapshot_id=?', ['base'])}
        row = rows['mixed.ipynb']
        self.assertEqual(row['repository'], 'frozen-notebook')
        self.assertEqual(row['commit_oid'], 'caller-base')
        self.assertEqual(row['extractor_sha'], m['extractor_id'])
        self.assertEqual('sha256:' + row['document']['source_sha256'], row['blob_oid'])
        cells = {c['id']: c for c in row['document']['cells']}
        self.assertEqual(set(cells), {'r', 'p', 'j', 'i', 'e', 'm'})
        self.assertEqual(cells['e']['source']['original'], '')
        self.assertEqual(cells['i']['symbols'], [])
        self.assertEqual(cells['r']['source']['policy'], 'evcxr-leading-dep-mask-v1')
        self.assertNotEqual(cells['r']['source']['original_sha256'], cells['r']['source']['analysis_sha256'])
        self.assertEqual(rows['partial.ipynb']['status'], 'partial')
        partial = rows['partial.ipynb']['document']
        self.assertEqual(len(partial['cells']), 3)
        self.assertTrue(partial['diagnostics'] or any(c['diagnostics'] for c in partial['cells']))
        self.assertTrue(any(c['identity_quality'] == 'source_derived' for c in partial['cells']))
        self.assertEqual(rows['null.ipynb']['status'], 'null_input')
        self.assertIsNone(rows['null.ipynb']['document'])
        self.assertFalse(m['snapshots']['base']['coverage']['extraction_complete'])
        self.assertEqual(m['snapshots']['base']['coverage']['notebook_status_counts'],
                         {'complete': 1, 'partial': 1, 'null_input': 1})
        ref = cells['r']['symbols'][0]['source_ref']
        self.assertEqual(ref['line_start'], 2)
        text = self.app.read_notebook_source(self.output, 'base', row['blob_oid'], ref)
        self.assertIn('fn helper()', text)

    def test_notebook_review_preserves_structure_config_and_metadata(self):
        m = self.consume_notebooks(self.notebook_fixture())
        self.assert_notebook_tables(m)
        reviews = {r['pair_id']: r for r in self.rows(m, 'notebook_review')}
        r = reviews['edit']
        self.assertEqual((r['base_snapshot_id'], r['head_snapshot_id']), ('base', 'head'))
        self.assertEqual((r['base_commit_oid'], r['head_commit_oid']), ('caller-base', 'caller-head'))
        self.assertEqual('sha256:' + r['result']['before']['source_sha256'], r['old_blob_oid'])
        self.assertEqual('sha256:' + r['result']['after']['source_sha256'], r['new_blob_oid'])
        kinds = {c['kind'] for c in r['result']['cell_changes']}
        self.assertTrue({'added', 'moved', 'dependency_directives_changed', 'metadata_changed'} <= kinds, kinds)
        self.assertTrue(r['result']['document_changes'])
        self.assertEqual(reviews['partial-noop']['status'], 'partial')
        self.assertEqual(reviews['partial-noop']['result']['symbol_changes'], [])
        self.assertEqual(reviews['partial-noop']['result']['cell_changes'], [])
        for change in r['result']['symbol_changes']:
            for side, sid, blob in [('before', 'base', r['old_blob_oid']), ('after', 'head', r['new_blob_oid'])]:
                if change[side]:
                    self.assertTrue(self.app.read_notebook_source(self.output, sid, blob, change[side]['source_ref']))
        self.assertEqual(m['review_coverage']['notebook_pair_rows'], 2)
        self.assertEqual(self.rows(m, 'review'), [])

    def test_notebook_source_context_and_historical_run_guards(self):
        self.assertTrue(callable(getattr(self.app, 'read_notebook_source', None)), 'guarded source reader required')
        first = self.consume_notebooks(self.notebook_fixture())
        row = next(r for r in self.rows(first, 'notebook_explore') if r['snapshot_id'] == 'base' and r['logical_path'] == 'mixed.ipynb')
        ref = row['document']['cells'][0]['symbols'][0]['source_ref']
        for key, value in [('notebook_source_sha256', 'wrong'), ('cell_source_sha256', 'wrong'),
                           ('cell_id', 'wrong'), ('cell_language', 'Python'), ('coordinate_space', 'raw_json'),
                           ('cell_ordinal', 999), ('byte_start', -1), ('line_start', 999)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.app.read_notebook_source(self.output, 'base', row['blob_oid'], dict(ref, **{key: value}))
        with self.assertRaises(ValueError):
            self.app.read_notebook_source(self.output, 'head', row['blob_oid'], ref)
        second_fixture = self.notebook_fixture()
        second_fixture['snapshots'] = second_fixture['snapshots'][1:]
        second_fixture['lineage'] = [r for r in second_fixture['lineage'] if r['snapshot_id'] == 'head']
        second_fixture['pairs'] = []
        self.consume_notebooks(second_fixture)
        with self.assertRaises(ValueError):
            self.app.read_notebook_source(self.output, 'base', row['blob_oid'], ref)
        self.assertIn('fn helper()', self.app.read_notebook_source(self.output, 'base', row['blob_oid'], ref, run_id=first['run_id']))
        self.assertEqual(self.app.read_result(self.output, 'base', run_id=first['run_id'])['manifest'], first)

    def test_notebook_real_frozen_inputs_through_consumer_and_original_anchors(self):
        root = CRATE.parents[1] / 'docs/research/artifacts'
        inputs = json.loads((root / '2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json').read_text())
        anchors = json.loads((root / '2026-11-17-notebook-symbol-diagnosis-bd-nb4c/frozen-cell-model-input.json').read_text())['anchors']
        versions = []
        pairs = []
        for snapshot in inputs['snapshots']:
            self.assertEqual(hashlib.sha256(snapshot['source'].encode()).hexdigest(), snapshot['source_sha256'])
            sid, path = snapshot['snapshot_id'], snapshot['notebook_path']
            versions.append((sid, {path: snapshot['source']}))
            pairs.append(dict(pair_id=sid + '-noop', base_snapshot_id=sid, head_snapshot_id=sid,
                              logical_path=path, old_path=path, kind='M'))
        pairs.append(dict(pair_id='real-change', base_snapshot_id=versions[0][0], head_snapshot_id=versions[1][0],
            logical_path=inputs['snapshots'][1]['notebook_path'], old_path=inputs['snapshots'][0]['notebook_path'], kind='M'))
        m = self.consume_notebooks(self.frozen_fixture(versions, pairs))
        self.assert_notebook_tables(m)
        checked = 0
        for i, row in enumerate(sorted(self.rows(m, 'notebook_explore'), key=lambda r: [v[0] for v in versions].index(r['snapshot_id']))):
            symbols = [s for c in row['document']['cells'] for s in c['symbols']]
            self.assertEqual(len(symbols), [46, 24][i])
            self.assertEqual(sum(s['symbol']['symbol_kind'] == 'function' for s in symbols), [17, 11][i])
            for symbol in symbols:
                self.assertTrue(self.app.read_notebook_source(self.output, row['snapshot_id'], row['blob_oid'], symbol['source_ref']))
            for anchor in anchors:
                if anchor['snapshot_id'] == row['snapshot_id']:
                    matching = [s for s in symbols if s['symbol']['entity_name'] == anchor['entity_name'] and s['source_ref']['cell_id'] == anchor['cell_id']]
                    self.assertEqual(len(matching), 1)
                    for key in ('byte_start', 'byte_end', 'line_start', 'line_end'):
                        self.assertEqual(matching[0]['source_ref'][key], anchor[key])
                    checked += 1
        self.assertEqual(checked, 28)
        for review in self.rows(m, 'notebook_review'):
            if review['pair_id'].endswith('-noop'):
                self.assertEqual(review['result']['symbol_changes'], [])
                self.assertEqual(review['result']['cell_changes'], [])
            else:
                self.assertTrue(review['result']['symbol_changes'])
                self.assertTrue(review['result']['cell_changes'])

    def test_review_nullable_absent_ordinary_pairs_are_retained(self):
        fixture = self.frozen_fixture([
            ('base', {'deleted.rs': 'fn before() {}'}),
            ('head', {'added.rs': 'fn after() {}'})], [
                dict(pair_id='added', base_snapshot_id='base', head_snapshot_id='head',
                     logical_path='added.rs', old_path=None, kind='A'),
                dict(pair_id='deleted', base_snapshot_id='base', head_snapshot_id='head',
                     logical_path=None, old_path='deleted.rs', kind='D')])
        m = self.consume_notebooks(fixture)
        self.assertEqual(m['review_coverage']['pair_rows'], 2)
        rows = self.rows(m, 'review')
        self.assertEqual(len(rows), 2, 'every accepted ordinary A/D pair must survive routing')
        self.assertEqual(self.rows(m, 'notebook_review'), [])
        for row in rows:
            pair = next(p for p in fixture['pairs'] if p['pair_id'] == row['pair_id'])
            self.assertEqual({k: row[k] for k in pair}, pair)
            self.assertEqual(row['status'], 'ok')
            self.assertIsNotNone(row['result'], 'ok must carry the native comparison')
            self.assertEqual([c['change_kind'] for c in row['result']['changes']],
                             ['added' if pair['kind'] == 'A' else 'deleted'])

    def test_review_nullable_absent_notebook_pairs_are_retained(self):
        from test_notebook import notebook
        fixture = self.frozen_fixture([
            ('base', {'deleted.IPYNB': notebook()}),
            ('head', {'added.ipynb': notebook()})], [
                dict(pair_id='added', base_snapshot_id='base', head_snapshot_id='head',
                     logical_path='added.ipynb', old_path=None, kind='A'),
                dict(pair_id='deleted', base_snapshot_id='base', head_snapshot_id='head',
                     logical_path=None, old_path='deleted.IPYNB', kind='D')])
        fixture['lineage'] = []
        m = self.consume_notebooks(fixture)
        self.assert_notebook_tables(m)
        self.assertEqual(m['review_coverage']['pair_rows'], 2)
        self.assertEqual(m['review_coverage']['notebook_pair_rows'], 2)
        self.assertEqual(m['review_coverage']['notebook_status_counts'], {'complete': 2})
        rows = self.rows(m, 'notebook_review')
        self.assertEqual(len(rows), 2)
        self.assertEqual(self.rows(m, 'review'), [])
        for row in rows:
            pair = next(p for p in fixture['pairs'] if p['pair_id'] == row['pair_id'])
            self.assertEqual({k: row[k] for k in pair}, pair)
            self.assertEqual(row['status'], 'complete')
            present, absent, sid, blob = ('after', 'before', 'head', row['new_blob_oid']) if pair['kind'] == 'A' else (
                'before', 'after', 'base', row['old_blob_oid'])
            self.assertIsNone(row['result'][absent])
            self.assertIsNone(row['old_blob_oid'] if pair['kind'] == 'A' else row['new_blob_oid'])
            document = row['result'][present]
            self.assertEqual(document['path'], pair['logical_path'] or pair['old_path'])
            self.assertEqual([c['kind'] for c in row['result']['cell_changes']],
                             ['added' if pair['kind'] == 'A' else 'deleted'])
            ref = document['cells'][0]['symbols'][0]['source_ref']
            self.assertTrue(self.app.read_notebook_source(self.output, sid, blob, ref))

    def test_review_refs_resolve_without_complete_explore_lineage(self):
        from test_notebook import code, notebook
        old_source = 'fn f() -> i32 { 1 }'
        new_source = 'fn f() -> i32 { 2 }'
        fixture = self.frozen_fixture([
            ('base', {'old.ipynb': notebook([code(source=old_source)]),
                      'copy.ipynb': notebook([code(source=new_source)])}),
            ('head', {'new.ipynb': notebook([code(source=new_source)]),
                      'copy.ipynb': notebook([code(source=old_source)])})], [
                dict(pair_id='renamed', base_snapshot_id='base', head_snapshot_id='head',
                     logical_path='new.ipynb', old_path='old.ipynb', kind='R')])
        for kind, included in (('M', ()), ('R', ()), ('R', ('base',)), ('R', ('head',))):
            with self.subTest(kind=kind, lineage=included):
                current = copy.deepcopy(fixture)
                current['lineage'] = [r for r in fixture['lineage'] if r['snapshot_id'] in included]
                if kind == 'M':
                    current['pairs'][0].update(kind='M', old_path='new.ipynb')
                    current['snapshots'][0]['sources'][0]['logical_path'] = 'new.ipynb'
                # Duplicate occurrences must remain occurrences, not source ambiguity.
                current['lineage'] *= 2
                m = self.consume_notebooks(current)
                self.assertEqual(len(self.rows(m, 'notebook_explore')), len(current['lineage']))
                row = self.rows(m, 'notebook_review')[0]
                refs = []
                for side, sid, blob, expected in [('before', 'base', row['old_blob_oid'], old_source),
                                                 ('after', 'head', row['new_blob_oid'], new_source)]:
                    ref = row['result'][side]['cells'][0]['symbols'][0]['source_ref']
                    refs.append((sid, blob, ref, expected))
                    try:
                        text = self.app.read_notebook_source(self.output, sid, blob, ref)
                    except ValueError as error:
                        self.fail('published review refs must resolve without Explore occurrences: ' + str(error))
                    self.assertEqual(text, expected)
                    other_sid = 'head' if sid == 'base' else 'base'
                    for key, value in [('notebook_path', 'copy.ipynb'), ('notebook_source_sha256', 'wrong'),
                            ('cell_source_sha256', 'wrong'), ('cell_id', 'wrong'), ('cell_identity_quality', 'wrong'),
                            ('cell_language', 'Python'), ('coordinate_space', 'raw_json'), ('cell_ordinal', 999),
                            ('byte_start', -1), ('byte_end', 9999), ('line_start', 999), ('line_end', 999)]:
                        with self.subTest(side=side, key=key), self.assertRaises(ValueError):
                            self.app.read_notebook_source(self.output, sid, blob, dict(ref, **{key: value}))
                    with self.assertRaises(ValueError):
                        self.app.read_notebook_source(self.output, other_sid, blob, ref)
                    with self.assertRaises(ValueError):
                        self.app.read_notebook_source(self.output, sid, 'sha256:wrong', ref)
                for (sid, blob, ref, _), (_, other_blob, other_ref, _) in (refs, refs[::-1]):
                    with self.assertRaises(ValueError):
                        self.app.read_notebook_source(self.output, sid, blob, other_ref)
                    with self.assertRaises(ValueError):
                        self.app.read_notebook_source(self.output, sid, other_blob, ref)
                # A later current run cannot stand in for the requested review's sources.
                self.run_consumer()
                for sid, blob, ref, expected in refs:
                    with self.assertRaises(ValueError):
                        self.app.read_notebook_source(self.output, sid, blob, ref)
                    self.assertEqual(self.app.read_notebook_source(self.output, sid, blob, ref,
                                     run_id=m['run_id']), expected)
                target = self.output / m['tables']['notebook_review']['path']
                original = target.read_bytes()
                target.write_bytes(original + b'corrupt')
                with self.assertRaisesRegex(ValueError, 'size|checksum'):
                    self.app.read_notebook_source(self.output, *refs[0][:3], run_id=m['run_id'])
                target.write_bytes(original)
                changed = copy.deepcopy(m)
                changed['notebook_contract_version'] = 999
                stored = self.output / 'runs' / m['run_id'] / 'manifest.json'
                stored.write_text(json.dumps(changed))
                with self.assertRaisesRegex(ValueError, 'contract'):
                    self.app.read_notebook_source(self.output, *refs[0][:3], run_id=m['run_id'])
                stored.write_text(json.dumps(m))

    def test_notebook_contract_rejects_unknown_versions_and_preserves_v1_readback(self):
        self.assertTrue(callable(getattr(self.app, 'read_notebook_source', None)), 'guarded source reader required')
        m = self.run_consumer()
        def store(value):
            (self.output / 'current.json').write_text(json.dumps(value))
            (self.output / 'runs' / m['run_id'] / 'manifest.json').write_text(json.dumps(value))
        for field in ('contract_version', 'notebook_contract_version'):
            changed = dict(m, **{field: 999})
            store(changed)
            with self.assertRaisesRegex(ValueError, 'contract|unsupported'):
                self.app.read_result(self.output, 'base')
        changed = copy.deepcopy(m)
        del changed['tables']['notebook_explore']
        store(changed)
        with self.assertRaisesRegex(ValueError, 'contract|table'):
            self.app.read_result(self.output, 'base')
        legacy = copy.deepcopy(m)
        legacy['contract_version'] = 1
        legacy.pop('notebook_contract_version')
        legacy['tables'].pop('notebook_explore')
        legacy['tables'].pop('notebook_review')
        store(legacy)
        self.assertEqual(self.app.read_result(self.output, 'base')['manifest'], legacy)
        with self.assertRaisesRegex(ValueError, 'contract|notebook'):
            self.app.read_notebook_source(self.output, 'base', 'x', {})


if __name__ == '__main__':
    unittest.main()
