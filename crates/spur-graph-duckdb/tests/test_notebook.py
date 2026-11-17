"""Native DuckDB 2 notebook v1 contract; requires a freshly packaged candidate."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

import duckdb

ROOT = Path(__file__).resolve().parents[3]
SYMBOL = ('STRUCT(entity_name VARCHAR, symbol_kind VARCHAR, enclosing_scope VARCHAR, '
          'byte_start UBIGINT, byte_end UBIGINT, line_start UBIGINT, line_end UBIGINT, '
          'anchor_hash VARCHAR, tokens VARCHAR[])')
DIAGNOSTIC = ('STRUCT(code VARCHAR, side VARCHAR, cell_ordinal UBIGINT, '
              'byte_start UBIGINT, byte_end UBIGINT, message VARCHAR)')
REF = ('STRUCT(notebook_path VARCHAR, notebook_source_sha256 VARCHAR, cell_id VARCHAR, '
       'cell_ordinal UBIGINT, cell_identity_quality VARCHAR, cell_source_sha256 VARCHAR, '
       'cell_language VARCHAR, coordinate_space VARCHAR, byte_start UBIGINT, byte_end UBIGINT, '
       'line_start UBIGINT, line_end UBIGINT)')
NOTEBOOK_SYMBOL = f'STRUCT(id VARCHAR, symbol {SYMBOL}, source_ref {REF})'
DIRECTIVE = 'STRUCT("text" VARCHAR, byte_start UBIGINT, byte_end UBIGINT)'
SOURCE = ('STRUCT(original VARCHAR, original_sha256 VARCHAR, analysis VARCHAR, '
          'analysis_sha256 VARCHAR, "policy" VARCHAR, coordinate_map VARCHAR, '
          f'directives {DIRECTIVE}[])')
CELL = ('STRUCT(id VARCHAR, identity_quality VARCHAR, ordinal UBIGINT, cell_type VARCHAR, '
        f'"language" VARCHAR, metadata_json VARCHAR, "source" {SOURCE}, status VARCHAR, '
        f'diagnostics {DIAGNOSTIC}[], symbols {NOTEBOOK_SYMBOL}[])')
DOCUMENT = ('STRUCT(contract_version UINTEGER, path VARCHAR, source_sha256 VARCHAR, '
            f'metadata_json VARCHAR, cells {CELL}[], status VARCHAR, diagnostics {DIAGNOSTIC}[])')
CELL_REF = ('STRUCT(notebook_path VARCHAR, notebook_source_sha256 VARCHAR, cell_id VARCHAR, '
            'cell_ordinal UBIGINT, cell_source_sha256 VARCHAR, "language" VARCHAR, cell_type VARCHAR)')
CHANGE = f'STRUCT(kind VARCHAR, "before" {NOTEBOOK_SYMBOL}, "after" {NOTEBOOK_SYMBOL})'
CELL_CHANGE = f'STRUCT(kind VARCHAR, "before" {CELL_REF}, "after" {CELL_REF})'
DOCUMENT_CHANGE = ('STRUCT(kind VARCHAR, before_path VARCHAR, after_path VARCHAR, '
                   'before_metadata_json VARCHAR, after_metadata_json VARCHAR)')
COMPARISON = (f'STRUCT(contract_version UINTEGER, status VARCHAR, diagnostics {DIAGNOSTIC}[], '
              f'"before" {DOCUMENT}, "after" {DOCUMENT}, symbol_changes {CHANGE}[], '
              f'cell_changes {CELL_CHANGE}[], document_changes {DOCUMENT_CHANGE}[])')

def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()

def code(id='r', language='rust', source='fn helper() {}\n'):
    return dict(id=id, cell_type='code', metadata={'spur': {'code_type': language}}, source=source)

def notebook(cells=None, metadata=None):
    return json.dumps(dict(cells=cells if cells is not None else [code()],
                           metadata=metadata or {}), ensure_ascii=False)

class NotebookTests(unittest.TestCase):
    def setUp(self):
        self.assertEqual(duckdb.__version__, '2.0.0')
        extension = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
        self.con = duckdb.connect(config={'allow_unsigned_extensions': True})
        self.addCleanup(self.con.close)
        self.con.execute("LOAD '" + str(extension).replace("'", "''") + "'")

    def symbols(self, source, path='n.ipynb'):
        return self.con.execute('SELECT spur_graph_notebook_symbols(?::VARCHAR,?::VARCHAR)',
                                [path, source]).fetchone()[0]

    def changes(self, left, right, kind='M', path='n.ipynb', old_path=None):
        return self.con.execute('SELECT spur_graph_notebook_changes(' + ','.join(['?::VARCHAR'] * 5) + ')',
                                [path, old_path, kind, left, right]).fetchone()[0]

    def resolve(self, document, symbol):
        ref = symbol['source_ref']
        cell = document['cells'][ref['cell_ordinal']]
        source = cell['source']['original']
        self.assertEqual(ref['notebook_path'], document['path'])
        self.assertEqual(ref['notebook_source_sha256'], document['source_sha256'])
        self.assertEqual(ref['cell_id'], cell['id'])
        self.assertEqual(ref['cell_source_sha256'], digest(source))
        self.assertEqual(ref['cell_language'], cell['language'])
        self.assertEqual(ref['cell_identity_quality'], cell['identity_quality'])
        self.assertEqual(ref['coordinate_space'], 'decoded_code_cell_utf8')
        raw = source.encode()
        start, end = ref['byte_start'], ref['byte_end']
        text = raw[start:end].decode()
        self.assertTrue(text)
        self.assertEqual(ref['line_start'], raw[:start].count(b'\n') + 1)
        self.assertEqual(ref['line_end'], raw[:end].count(b'\n') + 1)
        for key in ('byte_start', 'byte_end', 'line_start', 'line_end'):
            self.assertEqual(ref[key], symbol['symbol'][key])
        return text

    def test_exact_schemas_and_registry_docs(self):
        self.assertEqual(self.con.execute("DESCRIBE SELECT spur_graph_notebook_symbols('n.ipynb','{}')").fetchone()[1], DOCUMENT)
        self.assertEqual(self.con.execute("DESCRIBE SELECT spur_graph_notebook_changes('n.ipynb',NULL,'M','{}','{}')").fetchone()[1], COMPARISON)
        rows = self.con.execute("SELECT function_name, parameter_types, description FROM duckdb_functions() "
                                "WHERE function_name LIKE 'spur_graph_notebook_%' ORDER BY function_name").fetchall()
        self.assertEqual([r[0] for r in rows], ['spur_graph_notebook_changes', 'spur_graph_notebook_symbols'])
        self.assertEqual([r[1] for r in rows], [['VARCHAR'] * 5, ['VARCHAR'] * 2])
        for _, _, description in rows:
            for term in ('contract_version=1', 'decoded_code_cell_utf8', 'partial', 'metadata_json'):
                self.assertIn(term, description)
        # Exact ordinary v1 structs remain unchanged.
        self.assertEqual(self.con.execute("DESCRIBE SELECT spur_graph_symbols('x.rs','')").fetchone()[1], SYMBOL + '[]')
        self.assertEqual(self.con.execute("DESCRIBE SELECT spur_graph_symbol_changes('x.rs','x.rs','M',[],[])").fetchone()[1],
                         f'STRUCT(changes STRUCT(symbol {SYMBOL}, stable_symbol_id VARCHAR, file_path VARCHAR, '
                         'change_kind VARCHAR, previous_stable_symbol_id VARCHAR)[], diagnostics VARCHAR[])')

    def test_original_source_preparation_metadata_and_inventory(self):
        original = ':dep serde = "1" # café\r\nfn helper() -> i32 { 1 }\r\n'
        cells = [code(source=original), code('p', 'python', ['def alpha():\n', '    return 1\n']),
                 code('j', 'javascript', 'function beta() { return 2; }'),
                 code('i', 'python', 'import os\n'), code('e', 'rust', ''),
                 dict(id='m', cell_type='markdown', metadata={}, source='# hi')]
        source = notebook(cells, {'profile': 'asserted-only'})
        doc = self.symbols(source)
        self.assertEqual(doc['contract_version'], 1)
        self.assertEqual(doc['status'], 'complete')
        self.assertEqual(doc['source_sha256'], digest(source))
        self.assertEqual(json.loads(doc['metadata_json']), {'profile': 'asserted-only'})
        self.assertEqual(len(doc['cells']), 6)
        prepared = doc['cells'][0]['source']
        self.assertEqual(prepared['original'], original)
        self.assertEqual(prepared['original_sha256'], digest(original))
        self.assertEqual(prepared['analysis_sha256'], digest(prepared['analysis']))
        self.assertEqual(len(prepared['analysis'].encode()), len(original.encode()))
        self.assertEqual(prepared['policy'], 'evcxr-leading-dep-mask-v1')
        self.assertEqual(prepared['coordinate_map'], 'identity')
        self.assertEqual(prepared['directives'][0]['text'], original.split('\r\n')[0])
        for cell in doc['cells']:
            for symbol in cell['symbols']:
                self.resolve(doc, symbol)
        self.assertEqual(doc['cells'][0]['symbols'][0]['source_ref']['line_start'], 2)
        self.assertEqual([len(c['symbols']) for c in doc['cells']][3:], [0, 0, 0])

    def test_null_absence_invalid_unsupported_and_legacy_guards(self):
        source = notebook()
        self.assertIsNone(self.symbols(source, None))
        self.assertIsNone(self.symbols(None))
        self.assertIsNone(self.changes(source, source, path=None))
        self.assertIsNone(self.changes(source, source, kind=None))
        for kind, left, right, missing in [('A', None, source, 'before'), ('D', source, None, 'after')]:
            result = self.changes(left, right, kind)
            self.assertEqual(result['status'], 'complete')
            self.assertIsNone(result[missing])
            self.assertTrue(result['symbol_changes'])
            self.assertTrue(result['document_changes'])
        for left, right, kind in [(None, source, 'M'), (source, None, 'M'),
                                  (source, source, 'A'), (source, source, 'D'),
                                  (None, None, 'M'), ('', source, 'M')]:
            result = self.changes(left, right, kind)
            self.assertEqual(result['status'], 'invalid')
            self.assertTrue(result['diagnostics'])
        for kind in ('T', 'C', '?', 'R90'):
            result = self.changes(source, source, kind)
            self.assertEqual(result['status'], 'partial')
            self.assertIn('unsupported_change_kind', [d['code'] for d in result['diagnostics']])
        for bad in ('', '{', '{}', '{"cells":{}}', notebook([dict(cell_type='code', source=[1])])):
            result = self.symbols(bad)
            self.assertEqual(result['status'], 'invalid')
            self.assertTrue(result['diagnostics'])
        for unsupported in (code(language='unknown'), code(source=':load secret.rs\n'), code(source=':dep foo = {\nversion="1"\n}\n')):
            result = self.symbols(notebook([unsupported]))
            self.assertEqual(result['status'], 'partial')
            self.assertTrue(result['diagnostics'])
        for sql, new_api in [("SELECT spur_graph_symbols('x.ipynb','{}')", 'spur_graph_notebook_symbols'),
                             ("SELECT spur_graph_symbol_changes('x.ipynb','x.ipynb','M',[],[])", 'spur_graph_notebook_changes'),
                             ("SELECT spur_graph_symbol_changes('x.rs','x.ipynb','R',[],[])", 'spur_graph_notebook_changes')]:
            with self.assertRaisesRegex(duckdb.InvalidInputException, new_api):
                self.con.execute(sql).fetchall()

    def test_duplicate_helper_noop_ambiguity_and_rename_refs(self):
        source = notebook([code('a'), code('b', source='fn helper() { let x = 1; }')])
        result = self.changes(source, source)
        self.assertEqual(result['status'], 'complete')
        for field in ('symbol_changes', 'cell_changes', 'document_changes'):
            self.assertEqual(result[field], [])
        ids = [c['symbols'][0]['id'] for c in result['after']['cells']]
        self.assertEqual(len(set(ids)), 2)
        changed = source.replace('let x = 1', 'let x = 2')
        result = self.changes(source, changed)
        self.assertTrue(result['symbol_changes'])
        for change in result['symbol_changes']:
            for side in ('before', 'after'):
                if change[side]:
                    self.resolve(result[side], change[side])
        for kind in ('R', 'R100'):
            result = self.changes(source, source, kind, path='new.ipynb', old_path='old.ipynb')
            self.assertEqual(result['status'], 'complete')
            self.assertTrue(result['symbol_changes'])
            for change in result['symbol_changes']:
                self.resolve(result['before'], change['before'])
                self.resolve(result['after'], change['after'])
                self.assertEqual(change['before']['source_ref']['notebook_path'], 'old.ipynb')
                self.assertEqual(change['after']['source_ref']['notebook_path'], 'new.ipynb')
        self.assertEqual(self.changes(source, source, 'R')['status'], 'invalid')
        for cells in ([code('a', source='fn f() {} fn f() {}')], [code('a'), code('a')],
                      [{k: v for k, v in code().items() if k != 'id'}] * 2):
            ambiguous = notebook(cells)
            result = self.changes(ambiguous, ambiguous)
            self.assertEqual(result['status'], 'partial')
            self.assertTrue(result['diagnostics'])
            self.assertEqual(result['symbol_changes'], [])
            self.assertEqual(result['cell_changes'], [])

    def test_structural_dependency_and_metadata_events(self):
        cells = [code('a', source=':dep serde = "1"\nfn helper() {}\n'), code('b', 'python', 'import os\n')]
        left = notebook(cells)
        right = notebook([cells[1], dict(cells[0], source=cells[0]['source'].replace('"1"', '"2"'))], {'profile': 'new'})
        result = self.changes(left, right)
        self.assertEqual(result['symbol_changes'], [])
        self.assertIn('dependency_directives_changed', [e['kind'] for e in result['cell_changes']])
        self.assertIn('moved', [e['kind'] for e in result['cell_changes']])
        self.assertEqual(result['document_changes'][0]['kind'], 'metadata_changed')
        self.assertEqual(json.loads(result['document_changes'][0]['after_metadata_json']), {'profile': 'new'})
        for replacement, event in [(dict(cells[0], cell_type='raw'), 'type_changed'),
                                    (code('a', 'python', 'def helper():\n    return 1\n'), 'language_changed'),
                                    (dict(cells[0], metadata={'spur': {'code_type': 'rust'}, 'profile': 'x'}), 'metadata_changed')]:
            result = self.changes(left, notebook([replacement, cells[1]]))
            self.assertIn(event, [e['kind'] for e in result['cell_changes']])
        output_only = notebook([dict(c, outputs=[{'text': 'changed'}]) for c in cells])
        self.assertEqual(self.changes(left, output_only)['symbol_changes'], [])
        self.assertEqual(self.changes(left, output_only)['cell_changes'], [])

    def test_real_frozen_sources_and_original_refs(self):
        fixtures = ROOT / 'docs/research/artifacts'
        inputs = json.loads((fixtures / '2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json').read_text())
        frozen = json.loads((fixtures / '2026-11-17-notebook-symbol-diagnosis-bd-nb4c/frozen-cell-model-input.json').read_text())
        docs = []
        anchors = 0
        for i, snapshot in enumerate(inputs['snapshots']):
            self.assertEqual(digest(snapshot['source']), snapshot['source_sha256'])
            doc = self.symbols(snapshot['source'], snapshot['notebook_path'])
            docs.append(doc)
            symbols = [s for c in doc['cells'] for s in c['symbols']]
            self.assertEqual(len(symbols), [46, 24][i])
            self.assertEqual(sum(s['symbol']['symbol_kind'] == 'function' for s in symbols), [17, 11][i])
            self.assertEqual(len(set(s['id'] for s in symbols)), len(symbols))
            for symbol in symbols:
                self.resolve(doc, symbol)
            for anchor in frozen['anchors']:
                if anchor['snapshot_id'] != snapshot['snapshot_id']:
                    continue
                matches = [s for s in symbols if s['symbol']['entity_name'] == anchor['entity_name']
                           and s['source_ref']['cell_id'] == anchor['cell_id']]
                self.assertEqual(len(matches), 1)
                for field in ('byte_start', 'byte_end', 'line_start', 'line_end'):
                    self.assertEqual(matches[0]['source_ref'][field], anchor[field])
                anchors += 1
            self.assertEqual(self.changes(snapshot['source'], snapshot['source'], path=snapshot['notebook_path'])['symbol_changes'], [])
        snapshots = inputs['snapshots']
        self.assertEqual(anchors, 28)
        result = self.changes(snapshots[0]['source'], snapshots[1]['source'], path=snapshots[1]['notebook_path'])
        self.assertEqual(result['status'], 'complete')
        self.assertTrue(result['symbol_changes'])
        self.assertTrue(result['cell_changes'])
        for change in result['symbol_changes']:
            for side in ('before', 'after'):
                if change[side]:
                    self.resolve(result[side], change[side])

    def test_varied_nested_batches_null_empty_large_selection_and_parquet(self):
        large = notebook([code('large', source='\n'.join(f'fn f{i}() {{ let x = {i}; }}' for i in range(2300))),
                          code('directives', source=''.join(f':dep d{i} = "1"\n' for i in range(25)) + 'fn dep() {}')])
        values = [None, notebook([]), notebook(), notebook([code('a'), code('b')]), large,
                  notebook([code(language='unknown')]), '{']
        self.con.execute('CREATE TEMP TABLE nb(i INTEGER, source VARCHAR)')
        count = 2 * duckdb.__standard_vector_size__ + 1
        selected = {i: (4 if i % 1024 == 4 else (i % 7 if i % 7 != 4 else 3)) for i in range(count)}
        self.con.executemany('INSERT INTO nb VALUES (?,?)', [(i, values[selected[i]]) for i in range(count)])
        self.con.execute("CREATE TEMP TABLE parsed AS SELECT i, spur_graph_notebook_symbols('n.ipynb',source) AS doc, "
                         "spur_graph_notebook_changes('n.ipynb',NULL,'A',NULL,source) AS diff FROM nb WHERE i%2=0")
        rows = self.con.execute('SELECT i, doc, diff FROM parsed ORDER BY i').fetchall()
        expected_counts = [None, 0, 1, 2, 2301, 0, 0]
        for i, doc, diff in rows:
            if doc is None:
                self.assertEqual(selected[i], 0)
                self.assertEqual(diff['status'], 'invalid')
                continue
            self.assertEqual(sum(len(c['symbols']) for c in doc['cells']), expected_counts[selected[i]])
            if doc['status'] == 'complete':
                self.assertEqual(len(diff['symbol_changes']), expected_counts[selected[i]])
        # Smaller subset preserves all shapes through independent readback.
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / 'nb.parquet').replace("'", "''")
            self.con.execute(f"COPY (SELECT * FROM parsed WHERE i<14) TO '{path}' (FORMAT PARQUET)")
            with duckdb.connect() as reader:
                actual = reader.execute(f"SELECT * FROM read_parquet('{path}') ORDER BY i").fetchall()
            self.assertEqual(actual, [r for r in rows if r[0] < 14])

if __name__ == '__main__':
    unittest.main()
