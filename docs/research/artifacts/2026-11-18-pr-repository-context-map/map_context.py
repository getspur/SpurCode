"""Map a frozen PR change set into the complete tracked file inventory.

Relationship classes retain their evidence semantics. Text, build/config and
callback relationships are not presented as compiler-resolved call edges.
"""
import hashlib
import gzip
import json
from pathlib import Path, PurePosixPath
import re
import subprocess

import duckdb

ROOT = Path('/Users/kevintruong/project/SpurCode')
REPO = Path('/Users/kevintruong/project/duckdb')
CACHE = ROOT / '.spur/artifacts/cpp-gap-repository-index'
ART = Path(__file__).parent
BASE = '629e395a854d72dcb1f414e1a0451c68ac0e9f1f'
HEAD = 'd29b26bb92ac940a0807be0f43cc385b31623839'
PRIOR = ROOT / 'docs/research/artifacts/2026-11-17-review-extensions-poc'


def save(name, value):
    (ART / (name + '.json')).write_text(json.dumps(value, indent=2) + '\n')


def main():
    manifest = json.loads((CACHE / 'repository-manifest.json').read_text())
    head = {r['logical_path']: r for r in manifest if r['snapshot'] == 'head'}
    base = {r['logical_path']: r for r in manifest if r['snapshot'] == 'base'}
    changes = json.loads((PRIOR / 'changed-files.json').read_text())
    seeds = json.loads((PRIOR / 'semantic-changes.json').read_text())
    changed = {r['file_path'] for r in changes}
    assert changed == {p for p, r in head.items() if p not in base or base[p]['blob_oid'] != r['blob_oid']}
    assert len(head) == 18342 and len(changed) == 5
    relationships = []
    texts = {}
    coverage = {'tracked_entries': len(head), 'text_blobs_scanned': 0, 'non_utf8_blobs': 0, 'non_blobs': 0}
    symbols = {r['entity_name'] for r in seeds}
    symbols.update(['ExtractAggregationStats', 'GetDirectRewriteCallback', 'SetDirectRewriteCallback',
                    'HasSingleValueIdentity', 'SetSingleValueIdentity'])
    pattern = re.compile(r'\b(' + '|'.join(re.escape(x) for x in sorted(symbols)) + r')\b')
    refs = []
    paths = sorted(head)
    reader = subprocess.Popen(['git', '-C', str(REPO), 'cat-file', '--batch'], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    try:
        for path in paths:
            item = head[path]
            if item['git_kind'] != 'blob':
                coverage['non_blobs'] += 1
                continue
            reader.stdin.write((item['blob_oid'] + '\n').encode())
            reader.stdin.flush()
            oid, kind, length = reader.stdout.readline().decode().strip().split()
            data = reader.stdout.read(int(length))
            assert oid == item['blob_oid'] and kind == 'blob' and reader.stdout.read(1) == b'\n'
            item['source_sha256'] = hashlib.sha256(data).hexdigest()
            try:
                source = data.decode('utf-8')
            except UnicodeDecodeError:
                coverage['non_utf8_blobs'] += 1
                continue
            coverage['text_blobs_scanned'] += 1
            matches = []
            for line_number, line in enumerate(source.splitlines(), 1):
                for match in pattern.finditer(line):
                    matches.append({'path': path, 'line': line_number, 'symbol': match.group(), 'text': line.strip()})
            refs.extend(matches)
            if matches or path in changed or path.endswith('CMakeLists.txt') or path in ('Makefile', 'test/helpers/test_config.cpp') or path.startswith('test/configs/'):
                texts[path] = source
    finally:
        reader.stdin.close()
        assert reader.wait(timeout=10) == 0

    def edge(source, target, relation, evidence_class, witness_path, line, text, **extra):
        assert source in head and target in head and witness_path in head, (source, target, witness_path)
        if line is not None:
            assert 0 < line <= len(texts[witness_path].splitlines()), (witness_path, line)
        if evidence_class != 'native_bound_syntax':
            assert text.strip() in texts[witness_path], (witness_path, line, text)
        relationships.append({'source_path': source, 'target_path': target, 'relation': relation,
            'evidence_class': evidence_class, 'source_changed': source in changed, 'target_changed': target in changed,
            'witness_path': witness_path, 'witness_line': line, 'witness_text': text.strip(),
            'witness_blob_oid': head[witness_path]['blob_oid'], 'revision': HEAD, **extra})

    # Build declarations: literal source membership and literal subdirectory hierarchy.
    components = []
    interesting_sources = changed | {r['path'] for r in refs}
    for path, source in texts.items():
        if not path.endswith('CMakeLists.txt'):
            continue
        directory = PurePosixPath(path).parent
        for match in re.finditer(r'add_library(?:_unity)?\s*\(\s*([^\s()]+)\s+([^)]*)\)', source, re.S):
            if source.splitlines()[source.count('\n', 0, match.start())].lstrip().startswith('#'):
                continue
            target_name, body = match.group(1), match.group(2)
            for token in re.findall(r'[^\s"()]+', body):
                candidate = str(directory / token)
                if candidate in interesting_sources:
                    line = source.count('\n', 0, match.start()) + 1
                    edge(candidate, path, 'listed_in_build_target', 'build_declaration', path, line, match.group(), component=target_name)
                    components.append({'file_path': candidate, 'changed': candidate in changed, 'target': target_name, 'cmake_path': path, 'line': line})
        for match in re.finditer(r'add_subdirectory\s*\(\s*([\w./-]+)', source):
            if source.splitlines()[source.count('\n', 0, match.start())].lstrip().startswith('#'):
                continue
            child = str(directory / match.group(1) / 'CMakeLists.txt')
            if child in head:
                edge(child, path, 'included_by_parent_build', 'build_declaration', path,
                     source.count('\n', 0, match.start()) + 1, match.group())

    # Configuration relationships are parsed JSON values, not feature-name matches.
    configs = {}
    for path, source in texts.items():
        if path.startswith('test/configs/') and path.endswith('.json'):
            try:
                configs[path] = json.loads(source)
            except json.JSONDecodeError:
                pass
    test_path = 'test/sql/optimizer/statistics/distinct_on_stats_propagation.test'
    for path, config in configs.items():
        inherited = config.get('inherit_skip_tests')
        if inherited in head:
            line = next(i for i, s in enumerate(texts[path].splitlines(), 1) if 'inherit_skip_tests' in s)
            edge(path, inherited, 'inherits_skip_tests', 'config_value', path, line, texts[path].splitlines()[line - 1])
        for group in config.get('skip_tests', []):
            if test_path in group.get('paths', []):
                line = next(i for i, s in enumerate(texts[path].splitlines(), 1) if test_path in s)
                edge(path, test_path, 'skips_test_in_mode', 'config_value', path, line,
                     texts[path].splitlines()[line - 1], reason=group.get('reason'), on_init=config.get('on_init'))
    for line_number, line in enumerate(texts['Makefile'].splitlines(), 1):
        for config in ['test/configs/verify_aggregate_state_export.json', 'test/configs/disable_optimizer.json', 'test/configs/verify_serializer.json']:
            if config in line:
                edge('Makefile', config, 'lists_test_configuration', 'build_declaration', 'Makefile', line_number, line)

    # The configuration's effective test selection can change while the inheriting file does not.
    def skip_chain(path, seen=()):
        assert path not in seen, 'configuration inheritance cycle'
        config = configs[path]
        if any(test_path in group.get('paths', []) for group in config.get('skip_tests', [])):
            return [path, test_path]
        inherited = config.get('inherit_skip_tests')
        if inherited in configs:
            tail = skip_chain(inherited, seen + (path,))
            return [path] + tail if tail else []
        return []
    config_effects = []
    for path in configs:
        chain = skip_chain(path)
        if chain:
            config_effects.append({'configuration': path, 'configuration_changed': path in changed,
                'test': test_path, 'effective_skip': True, 'chain': chain,
                'on_init': configs[path].get('on_init'), 'evidence_class': 'parsed_config_inheritance',
                'caveat': 'Declared effective selection; no claim that the test suite was executed'})
    save('effective-test-selection', config_effects)

    # Explicit API references into wrappers that forward to the changed internal helper.
    api_target = 'src/optimizer/relation_statistics/relation_statistics_helper.cpp'
    for ref in refs:
        if ref['symbol'] == 'ExtractAggregationStats' and ref['path'].endswith('.cpp') and ref['path'] != api_target:
            if ref['path'].startswith(('src/', 'test/')):
                qualified = 'RelationStatisticsHelper::ExtractAggregationStats' in ref['text']
                edge(ref['path'], api_target, 'references_aggregate_stats_api' if qualified else 'unqualified_api_name_candidate', 'source_reference_candidate',
                     ref['path'], ref['line'], ref['text'], target_api='RelationStatisticsHelper::ExtractAggregationStats',
                     caveat='Source reference; overload/declaration-to-definition binding is not compiler-proven')

    # Callback registration plus dispatcher/getter are a possible execution relationship.
    aggregate_path = 'src/function/aggregate/distributive/first_last_any.cpp'
    callback_registration = list({(r['path'], r['line']): r for r in refs
        if r['path'] == aggregate_path and 'SetDirectRewriteCallback' in r['text'] and 'RewriteOrderedFirst' in r['text']}.values())
    callback_dispatch = [r for r in refs if r['symbol'] == 'GetDirectRewriteCallback' and '()(input)' in r['text']]
    for dispatch in callback_dispatch:
        edge(dispatch['path'], aggregate_path, 'dispatches_possible_registered_rewrite', 'callback_dispatch_candidate',
             dispatch['path'], dispatch['line'], dispatch['text'],
             registration_witnesses=callback_registration,
             caveat='Possible callback flow from registration and dispatcher; not a direct bound call or runtime trace')

    contract_header = 'src/include/duckdb/function/aggregate_function.hpp'
    for ref in refs:
        if ref['symbol'] in ('HasSingleValueIdentity', 'SetSingleValueIdentity') and ref['path'].endswith('.cpp'):
            edge(ref['path'], contract_header,
                 'reads_shared_identity_contract' if ref['symbol'].startswith('Has') else 'writes_shared_identity_contract',
                 'shared_contract_reference', ref['path'], ref['line'], ref['text'],
                 contract='AggregateFunction.properties.single_value_identity',
                 contract_definition={'path': contract_header, 'field_line': 353, 'reader_line': 401, 'writer_line': 402},
                 caveat='Shared contract participation; not a proven runtime flow between these files')

    loader = 'test/helpers/test_config.cpp'
    for config in ['test/configs/stats_dependent_tests.json', 'test/configs/verify_aggregate_state_export.json']:
        edge(config, loader, 'interpreted_by_config_loader', 'configuration_schema_consumer', loader, 621,
             texts[loader].splitlines()[620], caveat='Loader schema support; does not assert that this configuration was executed')

    # Reuse the previously measured shared-builder target-file evidence. Keep its precision limitation.
    witness_file = ROOT / 'docs/research/artifacts/2026-11-17-cpp-extraction-gap-lab/rust-full-repository-seed-witnesses.txt'
    all_symbols = json.loads((PRIOR / 'native-symbols.json').read_text())
    expression = re.compile(r'^(.*?) :: (.*?) -> (.*?) :: (.*?) \[bind=(.*?), cross_file=(true|false)\]$')
    for number, line in enumerate(witness_file.read_text().splitlines(), 1):
        match = expression.match(line)
        assert match, line
        source, caller, target, callee, method, cross = match.groups()
        if cross == 'true':
            seed = next((r for r in seeds if r['entity_name'] == caller and r['file_path'] == source), None)
            candidate_lines = {r['line_start'] for r in all_symbols if r['side'] == 'after'
                               and r['entity_name'] == caller and r['file_path'] == source}
            source_line = seed['line_start'] if seed else (next(iter(candidate_lines)) if len(candidate_lines) == 1 else None)
            edge(source, target, 'calls_dependency_in_shared_syntax_graph', 'native_bound_syntax', source,
                 source_line, line, source_symbol=caller, target_symbol=callee,
                 changed_symbol_seed=seed is not None, bind_method=method,
                 evidence_artifact=str(witness_file.relative_to(ROOT)), evidence_artifact_line=number,
                 witness_location='Core receipt is the binding witness; repository line is function start when unique, not the call site')

    # Retained native facts give exact intra-file forwarder paths. Their wider
    # incoming API references stay candidates, preserving the missing binding seam.
    retained_path = ROOT / 'docs/research/artifacts/2026-11-17-cpp-extraction-gap-lab/rust-core-probes.json'
    graph_identity = 'retained-two-file-head:' + hashlib.sha256(retained_path.read_bytes()).hexdigest()
    retained = json.loads(retained_path.read_text())['real_selected_repository']
    native_nodes = {n['node_id']: n for n in retained['nodes']}
    native_spans = {s['span_id']: s for s in retained['spans']}
    native_files = {n['file_id']: n['label'] for n in retained['nodes'] if n['kind'] == 'file'}
    calls = [e for e in retained['edges'] if e['relation'] == 'calls' and e.get('target_node_id') is not None]
    def symbol(node_id):
        node = native_nodes[node_id]
        span = native_spans[node['source_span_id']]
        source_path = native_files[node['file_id']]
        return {'node_id': node_id, 'graph_identity': graph_identity, 'scoped_node_id': graph_identity + ':' + str(node_id),
                'node_stable_key': node['stable_key'], 'repo_revision': HEAD, 'source_blob_oid': head[source_path]['blob_oid'],
                'name': node['label'], 'path': source_path,
                'line_start': span['start_line'], 'line_end': span['end_line'],
                'byte_start': span['start_byte'], 'byte_end': span['end_byte']}
    seed_maps = []
    native_paths = []
    for seed in seeds:
        ids = [n['node_id'] for n in retained['nodes'] if n['kind'] in ('function', 'method')
               and native_files.get(n.get('file_id')) == seed['file_path'] and n['label'].split('::')[-1] == seed['entity_name']
               and native_spans[n['source_span_id']]['start_byte'] < seed['byte_end']
               and native_spans[n['source_span_id']]['end_byte'] > seed['byte_start']]
        assert ids, seed['entity_name']
        seed_maps.append({'seed': seed['entity_name'], 'path': seed['file_path'], 'captured_nodes': [symbol(i) for i in ids]})
        frontier = [(i, [i], []) for i in ids]
        seen = set(ids)
        # Same two-call-hop neighborhood used in the original POC; no unbounded path enumeration.
        for depth in (1, 2):
            next_frontier = []
            for target, path_ids, path_edges in frontier:
                for call in calls:
                    if call['target_node_id'] != target or call['source_node_id'] in path_ids:
                        continue
                    source_id = call['source_node_id']
                    native_paths.append({'seed': seed['entity_name'], 'direction': 'incoming', 'call_hops': depth,
                        'evidence_class': 'native_bound_syntax', 'nodes': [symbol(i) for i in [source_id] + path_ids],
                        'bind_methods': [call.get('bind_method')] + path_edges,
                        'scope': 'Retained two-file native graph; cross-file API references are separate candidate evidence'})
                    if source_id not in seen:
                        next_frontier.append((source_id, [source_id] + path_ids, [call.get('bind_method')] + path_edges))
                        seen.add(source_id)
            frontier = next_frontier
    save('seed-node-mapping', seed_maps)
    save('native-incoming-paths', native_paths)
    save('callback-registrations', callback_registration)
    wrappers = [r for r in all_symbols if r['entity_name'] == 'RelationStatisticsHelper::ExtractAggregationStats']
    wrapper_identities = []
    for after in [r for r in wrappers if r['side'] == 'after']:
        before = [r for r in wrappers if r['side'] == 'before' and r['body_sha256'] == after['body_sha256']]
        assert len(before) == 1
        wrapper_identities.append({'name': after['entity_name'], 'path': after['file_path'],
            'before_lines': [before[0]['line_start'], before[0]['line_end']],
            'head_lines': [after['line_start'], after['line_end']], 'body_sha256': after['body_sha256'], 'body_unchanged': True})
    save('unchanged-forwarder-identities', wrapper_identities)

    # Persist a relational map whose vertex inventory is every tracked file.
    files = [{**r, 'changed': p in changed, 'change': next((c['status'] for c in changes if c['file_path'] == p), 'unchanged'),
              'unchanged_blob_verified': p in base and base[p]['blob_oid'] == r['blob_oid']}
             for p, r in sorted(head.items())]
    tracked_path = CACHE / 'context-tracked-files.json'
    tracked_data = (json.dumps(files, indent=2) + '\n').encode()
    tracked_path.write_bytes(tracked_data)
    (ART / 'tracked-files.json.gz').write_bytes(gzip.compress(tracked_data, mtime=0))
    save('changed-files', changes)
    save('changed-symbol-seeds', seeds)
    save('relationships', relationships)
    save('source-references', refs)
    save('declared-components', components)
    save('scan-coverage', coverage)
    save('identities', {'repository': str(REPO), 'base': BASE, 'head': HEAD,
        'retained_native_graph_identity': graph_identity,
        'node_id_rule': 'Native numeric IDs must retain graph_identity; they are not IDs in the full repository graph',
        'retained_native_graph_artifact': str(retained_path.relative_to(ROOT)),
        'core_binding_witness_artifact': str(witness_file.relative_to(ROOT))})
    with duckdb.connect(str(CACHE / 'change-context.duckdb')) as con:
        con.execute('CREATE OR REPLACE TABLE tracked_files AS SELECT * FROM read_json(?)', [str(tracked_path)])
        con.execute('CREATE OR REPLACE TABLE relationships AS SELECT * FROM read_json(?)', [str(ART / 'relationships.json')])
        con.execute('CREATE OR REPLACE TABLE changed_symbols AS SELECT * FROM read_json(?)', [str(ART / 'changed-symbol-seeds.json')])
        columns = con.execute("SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_name IN ('tracked_files','relationships','changed_symbols') ORDER BY table_name,ordinal_position").fetchall()
        save('schema', [{'table': a, 'column': b, 'type': c} for a, b, c in columns])
    print(json.dumps({'coverage': coverage, 'changed_files': len(changes), 'changed_symbols': len(seeds),
                      'relationships': len(relationships), 'declared_components': components}, indent=2))


if __name__ == '__main__':
    main()
