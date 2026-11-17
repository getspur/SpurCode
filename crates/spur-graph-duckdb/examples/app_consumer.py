"""Headless Explore/Review over caller-frozen sources; no repository or SQL service.

  python app_consumer.py --fixture tests/fixtures/app_consumer.json --output OUT \
      --package /path/to/spur_graph.duckdb_extension --extractor-id SOURCE_ID

run_consumer returns a completed manifest; read_result(OUT, snapshot_id) checks
the immutable manifest and every typed Parquet file without loading native code.
Read current.json once, then retain that manifest's immutable paths. Snapshot,
repository, commit and parent names are caller claims. Source bytes are checked
against sha256:utf8-v1 IDs (NULL has the explicit null-source sentinel); these
are not Git object IDs. Extractor/source identity comes from the verified local
unsigned package, distinct from its binary checksum. Native resolution and
pair matching retain the extension's single-file, pair-local limitations.
Ambiguous pair result lists retain native order, which can vary across fresh
processes; replay preserves evidence multiplicity without promising byte equality.

All DuckDB work runs in one isolated child. Only the parent can publish the
outer current.json, after exit 0 and checksummed candidate validation. Explicit
fault seams exercise death after candidate creation. On timeout or interruption
the parent kills the child process group and reaps it before cleaning its own
staging directory. timeout is an optional caller budget; there is no invented
production time, row, memory or retry cap. This is a small POSIX acceptance
harness, not a production supervisor. Atomic publication requires local rename
and flock semantics; a pointer failure can leave an unreferenced immutable run.
Old runs stay readable. Unsigned/local checksums do not authenticate a publisher
or defend against an adversarial concurrent filesystem writer.

Consumer contract v2 adds notebook_explore/document and notebook_review/result
typed tables using native notebook contract v1. Ordinary table schemas stay v1;
review contains ordinary pairs, notebook_review contains notebook pairs (either
path ending in .ipynb). Graph-only explore remains available for all sources.
Notebook coverage supplements graph coverage; complete is not runtime analysis.
read_result also accepts historical v1 manifests without inventing notebook
facts. An explicit run_id reads a retained immutable run independently of the
current pointer. read_notebook_source binds a native SourceRef to an exact
snapshot and blob within that run, without loading the extension.
Review source reads use the exact before/after snapshot context independently
of Explore occurrences. For A/D, a NULL absent-side path means absence; a D
with no logical_path supplies its validated old_path only to the native call.
Ordinary A calls map a NULL old_path to that API's empty-path absence sentinel.
The published pair paths retain the caller's values. Native NULL notebook
comparisons remain visible with null_input status.
"""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import uuid


CRATE = Path(__file__).resolve().parents[1]
SOURCE_IDENTITY_SCHEME = 'sha256:utf8-v1'
KEY = 'blob_oid, logical_path, extractor_sha'
SUCCESS = ('ok', 'empty_source', 'empty_facts')
FAULTS = (None, 'exit_after_candidate', 'interrupt_after_candidate', 'wait_after_candidate')
NOTEBOOK_TABLES = ('notebook_explore', 'notebook_review')


def checksum(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class ChildFailure(RuntimeError):
    def __init__(self, pid, returncode, stdout, stderr):
        self.pid, self.returncode = pid, returncode
        self.stdout, self.stderr = stdout, stderr
        super().__init__(f'native child {pid} exited {returncode}: {stderr}')


def validate_tables(manifest, root, prefix):
    """Validate only ordinary files within this run, including before rename."""
    for item in [*manifest['tables'].values(), manifest['snapshot_manifest']]:
        relative = Path(item['path']).relative_to(prefix)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('result path escapes immutable run')
        path = root / relative
        if path.resolve(strict=True) != path.absolute() or not path.is_file():
            raise ValueError('result path must be an ordinary file within the run')
        if 'bytes' in item and path.stat().st_size != item['bytes']:
            raise ValueError('result size mismatch: ' + item['path'])
        if checksum(path) != item['sha256']:
            raise ValueError('result checksum mismatch: ' + item['path'])


def read_result(output, snapshot_id, *, run_id=None):
    """Bind an exact requested snapshot to one verified immutable result."""
    output = Path(output).resolve(strict=True)
    manifest = None if run_id is not None else json.loads((output / 'current.json').read_bytes())
    run_id = run_id if manifest is None else manifest['run_id']
    if not isinstance(run_id, str) or uuid.UUID(run_id).hex != run_id:
        raise ValueError('invalid result run identity')
    prefix = Path('runs') / run_id
    root = output / prefix
    stored = json.loads((root / 'manifest.json').read_bytes())
    if manifest is None:
        manifest = stored
    if stored != manifest or manifest['run_id'] != run_id:
        raise ValueError('current manifest disagrees with immutable run identity')
    if manifest['status'] != 'complete' or manifest['contract_version'] not in (1, 2):
        raise ValueError('unsupported or incomplete result manifest')
    if manifest['contract_version'] == 2:
        if manifest.get('notebook_contract_version') != 1:
            raise ValueError('unsupported notebook contract version')
        if not all(t in manifest['tables'] for t in NOTEBOOK_TABLES):
            raise ValueError('notebook contract tables are absent')
    if manifest.get('source_identity_scheme') != SOURCE_IDENTITY_SCHEME:
        raise ValueError('unsupported source identity scheme')
    if snapshot_id not in manifest['snapshots']:
        raise ValueError('requested snapshot is absent: ' + str(snapshot_id))
    if manifest['artifact']['extractor_id'] != manifest['extractor_id']:
        raise ValueError('result extractor identity mismatch')
    validate_tables(manifest, root, prefix)
    return {'snapshot_id': snapshot_id, 'manifest': manifest}


def resolve_source_ref(document, ref):
    """Resolve original decoded-cell bytes; every native identity field matters."""
    ordinal = ref['cell_ordinal']
    if type(ordinal) is not int or not 0 <= ordinal < len(document['cells']):
        raise ValueError('source reference cell ordinal mismatch')
    cell = document['cells'][ordinal]
    prepared = cell['source']
    if prepared is None:
        raise ValueError('source reference has no decoded cell source')
    expected = dict(notebook_path=document['path'], notebook_source_sha256=document['source_sha256'],
        cell_id=cell['id'], cell_ordinal=cell['ordinal'], cell_identity_quality=cell['identity_quality'],
        cell_source_sha256=prepared['original_sha256'], cell_language=cell['language'],
        coordinate_space='decoded_code_cell_utf8')
    if any(ref.get(k) != v for k, v in expected.items()):
        raise ValueError('source reference context mismatch')
    raw = prepared['original'].encode('utf-8')
    if hashlib.sha256(raw).hexdigest() != prepared['original_sha256']:
        raise ValueError('original cell source identity mismatch')
    start, end = ref['byte_start'], ref['byte_end']
    if (type(start) is not int or type(end) is not int or not 0 <= start <= end <= len(raw)
            or ref['line_start'] != raw[:start].count(b'\n') + 1
            or ref['line_end'] != raw[:end].count(b'\n') + 1):
        raise ValueError('source reference range mismatch')
    try:
        return raw[start:end].decode('utf-8')
    except UnicodeDecodeError as error:
        raise ValueError('source reference is not on UTF-8 boundaries') from error


def validate_notebook_document(path, source, document):
    """Validate native provenance before attaching caller snapshot labels."""
    if source is None and document is None:
        return
    if (document is None or source is None or document['contract_version'] != 1
            or document['path'] != path
            or document['source_sha256'] != hashlib.sha256(source.encode('utf-8')).hexdigest()):
        raise ValueError('native notebook source identity mismatch')
    for cell in document['cells']:
        if cell['source'] is not None:
            prepared = cell['source']
            if hashlib.sha256(prepared['original'].encode()).hexdigest() != prepared['original_sha256']:
                raise ValueError('native original cell source identity mismatch')
        for symbol in cell['symbols']:
            resolve_source_ref(document, symbol['source_ref'])


def read_notebook_source(output, snapshot_id, blob_oid, source_ref, *, run_id=None):
    """Read a range only from the explicitly requested immutable source context."""
    import duckdb
    manifest = read_result(output, snapshot_id, run_id=run_id)['manifest']
    if manifest['contract_version'] != 2:
        raise ValueError('historical consumer contract has no notebook source references')
    paths = {table: str(Path(output) / manifest['tables'][table]['path'])
             for table in ('sources', *NOTEBOOK_TABLES)}
    context = [snapshot_id, blob_oid, source_ref['notebook_path'], manifest['extractor_id']]
    with duckdb.connect() as reader:
        sources = reader.execute('SELECT DISTINCT source FROM read_parquet(?) '
            'WHERE snapshot_id=? AND blob_oid=? AND logical_path=? AND extractor_sha=?',
            [paths['sources'], *context]).fetchall()
        # A review document is bound to its own published side. An occurrence
        # in Explore is optional; neither the current run nor the opposite
        # side is a substitute for this exact snapshot/blob/path/extractor key.
        rows = reader.execute('SELECT DISTINCT document FROM ('
            'SELECT document FROM read_parquet(?) '
            'WHERE snapshot_id=? AND blob_oid=? AND logical_path=? AND extractor_sha=? '
            'UNION ALL SELECT result."before" FROM read_parquet(?) '
            'WHERE base_snapshot_id=? AND old_blob_oid=? AND old_path=? AND extractor_sha=? '
            'UNION ALL SELECT result."after" FROM read_parquet(?) '
            'WHERE head_snapshot_id=? AND new_blob_oid=? AND logical_path=? AND extractor_sha=?'
            ') WHERE document IS NOT NULL',
            [paths['notebook_explore'], *context, paths['notebook_review'], *context,
             paths['notebook_review'], *context]).fetchall()
    if len(sources) != 1 or sources[0][0] is None:
        raise ValueError('source reference has no matching snapshot source')
    if len(rows) != 1 or rows[0][0] is None:
        raise ValueError('source reference has no matching snapshot source')
    document = rows[0][0]
    if 'sha256:' + document['source_sha256'] != blob_oid:
        raise ValueError('source reference notebook hash mismatch')
    validate_notebook_document(source_ref['notebook_path'], sources[0][0], document)
    return resolve_source_ref(document, source_ref)


def run_consumer(fixture, output, package, *, expected_extractor_id, timeout=None, fault=None):
    """Run one private native attempt and publish only a healthy verified result."""
    if fault not in FAULTS:
        raise ValueError('unknown child fault')
    if timeout is not None and timeout <= 0:
        raise ValueError('timeout must be a positive caller budget')
    package = Path(package).resolve(strict=True)
    fixture_bytes = Path(fixture).read_bytes()
    output = Path(output).resolve()
    runs = output / 'runs'
    runs.mkdir(parents=True, exist_ok=True)
    # Keep the inode: unlinking an advisory lock permits competing publishers.
    with (output / '.consumer.lock').open('a+b') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run_id = uuid.uuid4().hex
        staging = Path(tempfile.mkdtemp(prefix='.staging-', dir=runs))
        pointer = None
        try:
            (staging / 'fixture.json').write_bytes(fixture_bytes)
            request = dict(run_id=run_id, package=str(package),
                           expected_extractor_id=expected_extractor_id, fault=fault,
                           fixture_sha256=hashlib.sha256(fixture_bytes).hexdigest())
            write_json(staging / 'request.json', request)
            process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                '--child', str(staging / 'request.json')], cwd=staging,
                start_new_session=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                stdout, stderr = process.communicate(timeout=timeout)
            except BaseException as error:
                if process.poll() is None:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass  # exited between poll and signal; still reap below
                stdout, stderr = process.communicate()
                if isinstance(error, subprocess.TimeoutExpired):
                    raise ChildFailure(process.pid, process.returncode, stdout,
                                       stderr + '\ncaller timeout expired') from error
                raise
            if process.returncode != 0:
                raise ChildFailure(process.pid, process.returncode, stdout, stderr)
            candidate = json.loads((staging / 'candidate.json').read_bytes())
            if (candidate['status'] != 'candidate' or candidate['run_id'] != run_id
                    or candidate['native_pid'] != process.pid
                    or candidate['fixture_sha256'] != request['fixture_sha256']
                    or candidate['extractor_id'] != expected_extractor_id):
                raise ValueError('native candidate request identity mismatch')
            prefix = Path('runs') / run_id
            validate_tables(candidate, staging, prefix)
            candidate['status'] = 'complete'
            candidate['process'] = dict(pid=process.pid, returncode=process.returncode)
            write_json(staging / 'manifest.json', candidate)
            sync_directory(staging)
            os.rename(staging, runs / run_id)
            sync_directory(runs)
            pointer = output / ('.current-' + run_id)
            write_json(pointer, candidate)
            sync_directory(output)
            # Last fallible publication action; failures retain the old pointer.
            os.replace(pointer, output / 'current.json')
            pointer = None
            return candidate
        finally:
            if staging.exists():
                shutil.rmtree(staging)
            if pointer is not None:
                pointer.unlink(missing_ok=True)


def validate_fixture(fixture, extractor_id):
    """Check frozen bytes and complete lineage keys before native extraction."""
    if fixture['format_version'] != 1:
        raise ValueError('unsupported fixture format')
    if fixture.get('source_identity_scheme') != SOURCE_IDENTITY_SCHEME:
        raise ValueError('unsupported source identity scheme')
    snapshots, sources, keys = {}, [], set()
    for snapshot in fixture['snapshots']:
        sid = snapshot['snapshot_id']
        if not isinstance(sid, str) or not sid or sid in snapshots:
            raise ValueError('snapshot identity must be unique and nonempty')
        snapshots[sid] = snapshot
        paths = set()
        for source in snapshot['sources']:
            path, text, blob = (source[k] for k in ('logical_path', 'source', 'blob_oid'))
            if not isinstance(path, str) or path in paths or (text is not None and not isinstance(text, str)):
                raise ValueError('snapshot sources require distinct string paths and string/NULL bytes')
            paths.add(path)
            expected = 'null-source' if text is None else 'sha256:' + hashlib.sha256(text.encode()).hexdigest()
            if blob != expected:
                raise ValueError('source byte identity mismatch: ' + path)
            sources.append((sid, snapshot['repository'], snapshot['commit_oid'], snapshot['parent_oid'],
                            blob, path, text, extractor_id))
            keys.add((sid, blob, path, extractor_id))
    lineage = []
    for row in fixture['lineage']:
        sid, blob, path = (row[k] for k in ('snapshot_id', 'blob_oid', 'logical_path'))
        extractor = row.get('extractor_sha', extractor_id)
        if (sid, blob, path, extractor) not in keys:
            raise ValueError('lineage full source/extractor key has no matching snapshot source')
        snapshot = snapshots[sid]
        lineage.append((sid, row['occurrence_id'], snapshot['repository'], snapshot['commit_oid'],
                        snapshot['parent_oid'], blob, path, extractor))
    pair_ids, pairs = set(), []
    for pair in fixture['pairs']:
        if pair['pair_id'] in pair_ids or pair['kind'] not in ('A', 'D', 'M', 'R'):
            raise ValueError('review pair requires unique identity and supported change kind')
        pair_ids.add(pair['pair_id'])
        for side, path in [('base', pair['old_path']), ('head', pair['logical_path'])]:
            sid = pair[side + '_snapshot_id']
            if sid not in snapshots:
                raise ValueError('review pair names an absent snapshot')
            if pair['kind'] != {'base': 'A', 'head': 'D'}[side]:
                if not any(k[0] == sid and k[2] == path for k in keys):
                    raise ValueError('review pair path has no matching snapshot source')
        pairs.append(tuple(pair[k] for k in ('pair_id', 'base_snapshot_id', 'head_snapshot_id',
                                            'logical_path', 'old_path', 'kind')))
    return snapshots, sources, lineage, pairs


def notebook_projections(c, snapshots, extractor_id):
    # Materialize routing boundaries: the ordinary throwing APIs must never
    # see a notebook row, even if DuckDB changes expression evaluation order.
    c.execute("CREATE TEMP TABLE notebook_inputs AS SELECT * FROM staged_sources "
              "WHERE ends_with(lower(logical_path), '.ipynb')")
    c.execute('CREATE TEMP TABLE notebook_documents AS SELECT blob_oid, logical_path, extractor_sha, '
              'spur_graph_notebook_symbols(logical_path, source) document FROM notebook_inputs')
    for path, source, document in c.execute('SELECT logical_path, source, document '
            'FROM notebook_inputs JOIN notebook_documents USING (' + KEY + ')').fetchall():
        validate_notebook_document(path, source, document)
    c.execute("CREATE TABLE notebook_explore AS SELECT l.*, coalesce(d.document.status, 'null_input') status, "
              "d.document FROM source_lineage l JOIN notebook_documents d USING (" + KEY + ")")
    c.execute('CREATE TEMP TABLE snapshot_context(snapshot_id VARCHAR, repository VARCHAR, '
              'commit_oid VARCHAR, parent_oid VARCHAR)')
    if snapshots:
        c.executemany('INSERT INTO snapshot_context VALUES (?,?,?,?)',
            [(sid, s['repository'], s['commit_oid'], s['parent_oid']) for sid, s in snapshots.items()])
    c.execute("CREATE TEMP TABLE notebook_pair_inputs AS SELECT p.*, b.blob_oid old_blob_oid, "
              "h.blob_oid new_blob_oid, b.source left_source, h.source right_source "
              "FROM pairs p LEFT JOIN sources b ON p.base_snapshot_id=b.snapshot_id AND p.old_path=b.logical_path "
              "LEFT JOIN sources h ON p.head_snapshot_id=h.snapshot_id AND p.logical_path=h.logical_path "
              "WHERE coalesce(ends_with(lower(p.logical_path), '.ipynb'), false) "
              "OR coalesce(ends_with(lower(p.old_path), '.ipynb'), false)")
    c.execute('CREATE TEMP TABLE notebook_comparisons AS SELECT * EXCLUDE(left_source,right_source), '
              'spur_graph_notebook_changes(CASE WHEN kind=\'D\' AND logical_path IS NULL '
              'THEN old_path ELSE logical_path END,old_path,kind,left_source,right_source) result '
              'FROM notebook_pair_inputs')
    for path, old_path, left, right, result in c.execute(
            'SELECT i.logical_path,i.old_path,i.left_source,i.right_source,r.result '
            'FROM notebook_pair_inputs i JOIN notebook_comparisons r USING (pair_id)').fetchall():
        if result is None:
            continue  # Native NULL is absence, not a successful comparison.
        if result['contract_version'] != 1:
            raise ValueError('unsupported native notebook comparison contract')
        # Invalid side/kind results may intentionally have no parsed document.
        for side_path, source, document in [(old_path, left, result['before']), (path, right, result['after'])]:
            if document is not None:
                validate_notebook_document(side_path, source, document)
        for change in result['symbol_changes']:
            for side in ('before', 'after'):
                if change[side] is not None:
                    resolve_source_ref(result[side], change[side]['source_ref'])
    c.execute('CREATE TABLE notebook_review AS SELECT r.* EXCLUDE(result), '
              'b.repository base_repository,b.commit_oid base_commit_oid,b.parent_oid base_parent_oid, '
              'h.repository head_repository,h.commit_oid head_commit_oid,h.parent_oid head_parent_oid, '
              '?::VARCHAR extractor_sha,coalesce(r.result.status,\'null_input\') status,r.result FROM notebook_comparisons r '
              'JOIN snapshot_context b ON r.base_snapshot_id=b.snapshot_id '
              'JOIN snapshot_context h ON r.head_snapshot_id=h.snapshot_id',
              [extractor_id])


def native_candidate(request, staging):
    # These imports and every native operation stay in the isolated child.
    import duckdb
    sys.path.insert(0, str(CRATE / 'scripts'))
    from verify_package import verify_package, runtime_contract
    from publish_snapshot import publish_snapshot, TABLES

    package = Path(request['package'])
    artifact = verify_package(package)
    if artifact['extractor_id'] != request['expected_extractor_id']:
        raise ValueError('loaded package extractor disagrees with requested extractor identity')
    fixture_path = staging / 'fixture.json'
    if checksum(fixture_path) != request['fixture_sha256']:
        raise ValueError('frozen fixture identity changed before extraction')
    fixture = json.loads(fixture_path.read_bytes())
    snapshots, sources, lineage, pairs = validate_fixture(fixture, artifact['extractor_id'])
    database = staging / 'facts.duckdb'

    def connect():
        c = duckdb.connect(str(database), config={'allow_unsigned_extensions': True})
        c.execute("LOAD '" + str(package).replace("'", "''") + "'")
        if c.execute('SELECT spur_graph_info()').fetchone()[0] != {
                k: artifact[k] for k in ('extension_version', 'facts_schema_version',
                'duckdb_api_version', 'extractor_id', 'supported_languages', 'resolution_scope')}:
            c.close()
            raise ValueError('working connection extractor metadata mismatch')
        return c

    pipeline = (CRATE / 'examples/file_facts.sql').read_text().split('-- LEGACY NON-ATOMIC', 1)[0]
    c = connect()
    try:
        c.execute('CREATE TABLE sources(snapshot_id VARCHAR, repository VARCHAR, commit_oid VARCHAR, '
                  'parent_oid VARCHAR, blob_oid VARCHAR, logical_path VARCHAR, source VARCHAR, extractor_sha VARCHAR)')
        if sources:
            c.executemany('INSERT INTO sources VALUES (?, ?, ?, ?, ?, ?, ?, ?)', sources)
        c.execute('CREATE TABLE staged_sources AS SELECT DISTINCT blob_oid, logical_path, source, extractor_sha FROM sources')
        c.execute('CREATE TABLE source_lineage(snapshot_id VARCHAR, occurrence_id VARCHAR, repository VARCHAR, '
                  'commit_oid VARCHAR, parent_oid VARCHAR, blob_oid VARCHAR, logical_path VARCHAR, extractor_sha VARCHAR)')
        if lineage:
            c.executemany('INSERT INTO source_lineage VALUES (?, ?, ?, ?, ?, ?, ?, ?)', lineage)
        c.execute(pipeline)
        before = {t: c.execute('SELECT * FROM ' + t + ' ORDER BY ALL').fetchall() for t in TABLES}
    finally:
        c.close()
    # A fresh writer connection reuses immutable cache keys, preserving caller
    # occurrence multiplicity while adding no rows on replay.
    c = connect()
    try:
        c.execute(pipeline)
        replay = dict(missing_keys=c.execute('SELECT count(*) FROM file_facts_missing').fetchone()[0],
                      tables_unchanged=all(c.execute('SELECT * FROM ' + t + ' ORDER BY ALL').fetchall()
                                           == before[t] for t in TABLES))
        if replay['missing_keys'] or not replay['tables_unchanged']:
            raise ValueError('cache replay changed the frozen snapshot')
        c.execute('CREATE TABLE explore AS SELECT o.* EXCLUDE(extraction_status), '
                  'c.status, c.error_code, c.error_message, c.facts FROM file_facts_occurrences o '
                  'JOIN file_facts_cache c USING (' + KEY + ')')
        # Physically filter recoverable failures before the throwing symbols API.
        c.execute("CREATE TEMP TABLE symbol_inputs AS SELECT s.* FROM sources s JOIN file_facts_cache c "
                  "USING (" + KEY + ") WHERE c.status IN ('ok','empty_source','empty_facts') "
                  "AND NOT ends_with(lower(s.logical_path), '.ipynb')")
        c.execute('CREATE TEMP TABLE symbols AS SELECT snapshot_id, blob_oid, logical_path, extractor_sha, '
                  'spur_graph_symbols(logical_path, source) symbols FROM symbol_inputs')
        c.execute('CREATE TABLE pairs(pair_id VARCHAR, base_snapshot_id VARCHAR, head_snapshot_id VARCHAR, '
                  'logical_path VARCHAR, old_path VARCHAR, kind VARCHAR)')
        if pairs:
            c.executemany('INSERT INTO pairs VALUES (?, ?, ?, ?, ?, ?)', pairs)
        c.execute("CREATE TEMP TABLE ordinary_pairs AS SELECT * FROM pairs "
                  "WHERE NOT (coalesce(ends_with(lower(logical_path), '.ipynb'), false) "
                  "OR coalesce(ends_with(lower(old_path), '.ipynb'), false))")
        c.execute("CREATE TEMP TABLE pair_inputs AS SELECT p.*, b.blob_oid old_blob_oid, h.blob_oid new_blob_oid, "
                  "b.symbols old_symbols, h.symbols new_symbols, "
                  "(p.kind='A' OR b.symbols IS NOT NULL) AND (p.kind='D' OR h.symbols IS NOT NULL) available "
                  "FROM ordinary_pairs p LEFT JOIN symbols b ON p.base_snapshot_id=b.snapshot_id AND p.old_path=b.logical_path "
                  "LEFT JOIN symbols h ON p.head_snapshot_id=h.snapshot_id AND p.logical_path=h.logical_path")
        c.execute("CREATE TABLE review AS SELECT * EXCLUDE(old_symbols,new_symbols,available), "
                  "CASE WHEN available THEN 'ok' ELSE 'unavailable' END status, "
                  "CASE WHEN available THEN spur_graph_symbol_changes("
                  "CASE WHEN kind='D' AND logical_path IS NULL THEN old_path ELSE logical_path END,"
                  "CASE WHEN kind='A' AND old_path IS NULL THEN '' ELSE old_path END,kind,"
                  "coalesce(old_symbols,[]),coalesce(new_symbols,[])) ELSE NULL END result FROM pair_inputs")
        notebook_projections(c, snapshots, artifact['extractor_id'])
        coverage = {}
        for sid, snapshot in snapshots.items():
            counts = dict(c.execute('SELECT c.status,count(*) FROM sources s JOIN file_facts_cache c '
                                   'USING (' + KEY + ') WHERE s.snapshot_id=? GROUP BY c.status', [sid]).fetchall())
            unresolved = c.execute('SELECT count(*) FROM sources s JOIN file_facts_edges e USING (' + KEY + ') '
                                   'WHERE s.snapshot_id=? AND e.target_node_id IS NULL', [sid]).fetchone()[0]
            notebook_counts = dict(c.execute('SELECT coalesce(d.document.status,\'null_input\'), count(*) '
                'FROM sources s JOIN notebook_documents d USING (' + KEY + ') '
                'WHERE s.snapshot_id=? GROUP BY ALL', [sid]).fetchall())
            coverage[sid] = dict(repository=snapshot['repository'], commit_oid=snapshot['commit_oid'],
                parent_oid=snapshot['parent_oid'], coverage=dict(source_rows=len(snapshot['sources']),
                status_counts=counts, notebook_status_counts=notebook_counts,
                extraction_complete=all(k in SUCCESS for k in counts)
                    and all(k == 'complete' for k in notebook_counts),
                unresolved_edges=unresolved))
        review_coverage = dict(pair_rows=len(pairs), unavailable_pairs=c.execute(
            "SELECT count(*) FROM review WHERE status='unavailable'").fetchone()[0],
            pairs_with_diagnostics=c.execute(
            'SELECT count(*) FROM review WHERE len(result.diagnostics)>0').fetchone()[0])
        review_coverage.update(notebook_pair_rows=c.execute('SELECT count(*) FROM notebook_review').fetchone()[0],
            notebook_status_counts=dict(c.execute('SELECT status,count(*) FROM notebook_review GROUP BY status').fetchall()),
            notebook_pairs_with_diagnostics=c.execute('SELECT count(*) FROM notebook_review '
                'WHERE len(result.diagnostics)>0 OR list_has_any(list_transform(result."before".cells, '
                'lambda cell: len(cell.diagnostics)>0), [true]) OR list_has_any(list_transform(result."after".cells, '
                'lambda cell: len(cell.diagnostics)>0), [true])').fetchone()[0])
        native_version, platform = runtime_contract(c)
    finally:
        c.close()
    snapshot = publish_snapshot(database, staging / 'snapshot', version='facts',
                                source_labels={'fixture_sha256': request['fixture_sha256']})
    prefix = Path('runs') / request['run_id']
    tables = {t: dict(item, path=(prefix / 'snapshot' / item['path']).as_posix())
              for t, item in snapshot['tables'].items()}
    # No writer remains. These additional app projections come from that same
    # private immutable database, in one read-only transaction.
    with duckdb.connect(str(database), read_only=True) as reader:
        reader.execute('BEGIN TRANSACTION')
        for table in ('sources', 'explore', 'review', *NOTEBOOK_TABLES):
            path = staging / (table + '.parquet')
            reader.execute("COPY " + table + " TO '" + str(path).replace("'", "''") + "' (FORMAT PARQUET)")
            columns = reader.execute('DESCRIBE ' + table).fetchall()
            with path.open('rb') as stream:
                os.fsync(stream.fileno())
            tables[table] = dict(path=(prefix / path.name).as_posix(), schema=[[v[0], v[1]] for v in columns],
                row_count=reader.execute('SELECT count(*) FROM ' + table).fetchone()[0],
                sha256=checksum(path), bytes=path.stat().st_size)
        reader.execute('COMMIT')
    if checksum(package) != artifact['binary']['sha256']:
        raise ValueError('package artifact changed during extraction')
    candidate = dict(contract_version=2, notebook_contract_version=1, status='candidate', run_id=request['run_id'], native_pid=os.getpid(),
        fixture_sha256=request['fixture_sha256'], extractor_id=artifact['extractor_id'],
        facts_schema_version=artifact['facts_schema_version'], artifact=artifact,
        artifact_manifest_sha256=checksum(Path(str(package) + '.json')),
        runtime=dict(duckdb=duckdb.__version__, native_version=native_version, platform=platform),
        source_labels=dict(provenance='caller_supplied', independently_verified=False),
        source_identity_scheme=SOURCE_IDENTITY_SCHEME, snapshots=coverage,
        resolution=dict(scope=artifact['resolution_scope'], compiler_complete=False, repository_complete=False),
        review_coverage=review_coverage, replay=replay, tables=tables,
        snapshot_manifest=dict(path=(prefix / 'snapshot/versions/facts/manifest.json').as_posix(),
                               sha256=checksum(staging / 'snapshot/versions/facts/manifest.json')))
    write_json(staging / 'candidate.json', candidate)
    print('candidate written', flush=True)
    if request['fault'] == 'exit_after_candidate':
        os._exit(23)  # explicit fault exit, never a production resource limit
    if request['fault'] == 'interrupt_after_candidate':
        os.kill(os.getpid(), signal.SIGTERM)
    if request['fault'] == 'wait_after_candidate':
        while True:
            signal.pause()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--child', type=Path, help=argparse.SUPPRESS)
    parser.add_argument('--fixture', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--package', type=Path)
    parser.add_argument('--extractor-id')
    parser.add_argument('--timeout', type=float, help='optional explicit caller time budget in seconds')
    args = parser.parse_args()
    if args.child:
        native_candidate(json.loads(args.child.read_bytes()), args.child.parent)
    else:
        if not all((args.fixture, args.output, args.package, args.extractor_id)):
            parser.error('--fixture, --output, --package and --extractor-id are required')
        run_consumer(args.fixture, args.output, args.package,
                     expected_extractor_id=args.extractor_id, timeout=args.timeout)
        print(args.output / 'current.json')


if __name__ == '__main__':
    main()
