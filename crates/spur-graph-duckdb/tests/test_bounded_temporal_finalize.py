"""Small cached-pair fixtures: zero native parsing/matching, no full-run writes."""

import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import types
import unittest

import duckdb


ROOT=Path(__file__).resolve().parent
PROGRAM=ROOT.parent/'examples/bounded_temporal_finalize.py'
ORIGINAL=ROOT.parent/'examples/derive_temporal_pairs.py'
SYMBOL='STRUCT(entity_name VARCHAR,symbol_kind VARCHAR,enclosing_scope VARCHAR,byte_start UBIGINT,byte_end UBIGINT,line_start UBIGINT,line_end UBIGINT,anchor_hash VARCHAR,tokens VARCHAR[])'
CHANGE=f'STRUCT(stable_symbol_id VARCHAR,change_kind VARCHAR,previous_stable_symbol_id VARCHAR,symbol {SYMBOL},file_path VARCHAR)'


def load(path,name):
    m=types.ModuleType(name); m.__file__=str(path)
    exec(compile(path.read_bytes(),str(path),'exec'),m.__dict__)
    return m


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def q(value):
    return "'"+str(value).replace("'","''")+"'"


class AvailabilityTests(unittest.TestCase):
    def test_finalizer_exists(self):
        self.assertTrue(PROGRAM.is_file(),'bounded temporal finalizer is missing')


class FinalizerTests(unittest.TestCase):
    def setUp(self):
        self.module=load(PROGRAM,'bounded_fixture')
        self.original=load(ORIGINAL,'original_fixture')
        self.temp=tempfile.TemporaryDirectory(prefix='bounded-finalize-fixture-')
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve()
        self.metadata=self.root/'metadata'; self.metadata.mkdir()
        self.inputs={'version':1,'row_limit':4,'byte_budget':18530894,'fixture':'cached pair outputs, no native calls'}
        source_id=hashlib.sha256(json.dumps(self.inputs,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        self.source=self.root/('gen-'+source_id)
        (self.source/'pairs').mkdir(parents=True); (self.source/'receipts').mkdir()
        (self.source/'.lock').touch()
        (self.source/'inputs.json').write_text(json.dumps(self.inputs))
        (self.source/'current-status.json').write_text(json.dumps({'complete':False,'state':'failed','error':'observed global fold fixture failure'}))
        self.commits=[f'{i+1:040x}' for i in range(6)]
        def change(sid,kind='modified',previous=None,name='shared',tokens=None):
            return {'stable_symbol_id':sid,'change_kind':kind,'previous_stable_symbol_id':previous,
                    'file_path':'shared.rs','symbol':{'entity_name':name,'symbol_kind':'function','enclosing_scope':None,
                    'byte_start':0,'byte_end':10,'line_start':1,'line_end':1,'anchor_hash':'anchor-'+name,
                    'tokens':['a','b'] if tokens is None else tokens}}
        zero='0'*16; one='0'*15+'1'; maximum='f'*16
        payloads=[[change(zero,'added',name='root',tokens=[])], [change(one)],
                  [change('0'*15+'2','renamed_from_symbol',zero,name='branch')], [change(one)],
                  [change(one,'renamed_from_symbol',maximum,name='conflicting',tokens=['b','a'])],
                  [change(one)], [change(maximum,'added'),change(maximum,'added',zero)], []]
        comparisons=[(0,0,None),(1,1,0),(2,2,0),(3,3,1),(4,3,2),(5,4,3),(6,5,4)]
        locations=[(0,0,0),(1,1,1),(2,2,2),(3,3,3),(4,3,3),(5,4,3),(6,3,3),(7,0,0),(1,6,5)]
        with duckdb.connect(':memory:') as c:
            c.execute('CREATE TABLE commits(commit_oid VARCHAR,ordinal BIGINT,parents VARCHAR[],author_time BIGINT,summary VARCHAR)')
            parents=[[],[0],[0],[1,2],[3],[4]]
            c.executemany('INSERT INTO commits VALUES(?,?,?,?,?)',[(oid,i,[self.commits[x] for x in parents[i]],i,'commit'+str(i)) for i,oid in enumerate(self.commits)])
            c.execute('CREATE TABLE comparisons(comparison_id BIGINT,commit_oid VARCHAR,parent_oid VARCHAR,parent_index BIGINT)')
            c.executemany('INSERT INTO comparisons VALUES(?,?,?,?)',[(idx,self.commits[commit],self.commits[parent] if parent is not None else None,1 if idx==4 else 0) for idx,commit,parent in comparisons])
            c.execute('CREATE TABLE symbol_pairs(pair_id BIGINT,language VARCHAR,path VARCHAR,old_path VARCHAR,kind VARCHAR,old_blob_oid VARCHAR,new_blob_oid VARCHAR)')
            c.executemany('INSERT INTO symbol_pairs VALUES(?,?,?,?,?,?,?)',[(i,'Rust','file'+str(i)+'.rs','file'+str(i)+'.rs','A' if i in (0,7) else 'M',None if i in (0,7) else f'{i:040x}',f'{i+1:040x}') for i in range(8)])
            c.execute('CREATE TABLE parse_tasks(language VARCHAR,blob_oid VARCHAR,path VARCHAR,byte_length BIGINT)')
            c.executemany('INSERT INTO parse_tasks VALUES(?,?,?,?)',[('Rust',f'{i+1:040x}','file.rs',1) for i in range(8)])
            c.execute('CREATE TABLE changes(change_id BIGINT,comparison_id BIGINT,commit_oid VARCHAR,parent_oid VARCHAR,parent_index BIGINT,status VARCHAR,path VARCHAR,old_path VARCHAR,language VARCHAR,old_blob_oid VARCHAR,new_blob_oid VARCHAR,gitlink BOOLEAN)')
            c.execute('CREATE TABLE change_pairs(change_id BIGINT,pair_id BIGINT)')
            for cid,(pair,comparison,commit) in enumerate(locations):
                parent=next(p for idx,co,p in comparisons if idx==comparison)
                c.execute('INSERT INTO changes SELECT ?,?,?,?,? ,kind,path,CASE WHEN kind=\'A\' THEN NULL ELSE old_path END,language,old_blob_oid,new_blob_oid,false FROM symbol_pairs WHERE pair_id=?',
                          [cid,comparison,self.commits[commit],self.commits[parent] if parent is not None else None,1 if comparison==4 else 0,pair])
                c.execute('INSERT INTO change_pairs VALUES(?,?)',[cid,pair])
            c.execute(f'CREATE TABLE pair_values(pair_id BIGINT,status VARCHAR,changes {CHANGE}[],diagnostics VARCHAR[],old_status VARCHAR,new_status VARCHAR,old_source VARCHAR,new_source VARCHAR,old_binary BOOLEAN,new_binary BOOLEAN,native_invoked BOOLEAN)')
            for i,payload in enumerate(payloads):
                c.execute('INSERT INTO pair_values VALUES(?,?,?,?,?,?,?,?,?,?,?)',[i,'completed',payload,[],'completed','completed','logical-cache','logical-cache',False,False,True])
            for table in ('commits','comparisons','changes','parse_tasks','symbol_pairs','change_pairs'):
                c.execute('COPY '+table+' TO '+q(self.metadata/(table+'.parquet'))+' (FORMAT PARQUET)')
            c.execute('COPY (SELECT comparison_id,count(*) supported_occurrences,sum(array_length(v.changes))::BIGINT raw_candidates FROM changes ch JOIN change_pairs cp USING(change_id) JOIN pair_values v USING(pair_id) GROUP BY comparison_id) TO '+q(self.root/'comparison-counts.parquet')+' (FORMAT PARQUET)')
            # Include zero-change comparisons explicitly in the resource basis.
            c.execute('COPY (SELECT x.comparison_id,coalesce(s.supported_occurrences,0)::BIGINT supported_occurrences,coalesce(s.raw_candidates,0)::BIGINT raw_candidates FROM comparisons x LEFT JOIN read_parquet(?) s USING(comparison_id)) TO '+q(self.root/'counts.parquet')+' (FORMAT PARQUET)',[str(self.root/'comparison-counts.parquet')])
            costs=dict(c.execute('SELECT pair_id,coalesce(o.byte_length,0)+coalesce(n.byte_length,0) FROM symbol_pairs p LEFT JOIN parse_tasks o ON p.language=o.language AND p.old_blob_oid=o.blob_oid LEFT JOIN parse_tasks n ON p.language=n.language AND p.new_blob_oid=n.blob_oid').fetchall())
            for idx,ids in enumerate((list(range(4)),list(range(4,8)))):
                path=self.source/'pairs'/f'{idx:06}.parquet'
                c.execute('COPY (SELECT * FROM pair_values WHERE pair_id BETWEEN ? AND ?) TO '+q(path)+' (FORMAT PARQUET)',[ids[0],ids[-1]])
                receipt={'generation':source_id,'batch_id':idx,'pair_ids':ids,'rows':len(ids),'input_bytes':sum(costs[i] for i in ids),'byte_budget':18530894,'row_limit':4,'oversized':False,
                         'file':str(path.relative_to(self.source)),'file_bytes':path.stat().st_size,'sha256':digest(path),'native_invocations':len(ids),'elapsed_seconds':0.0}
                (self.source/'receipts'/f'{idx:06}.json').write_text(json.dumps(receipt))
            self.baseline=self.root/'baseline.duckdb'
            c.execute('ATTACH '+q(self.baseline)+' AS baseline')
            for table in ('commits','comparisons','changes','parse_tasks','symbol_pairs','change_pairs'):
                c.execute(f'CREATE TABLE baseline.{table} AS SELECT * FROM {table}')
        with duckdb.connect(str(self.baseline)) as c:
            self.original.materialize_views(c,[str(p) for p in sorted((self.source/'pairs').glob('*.parquet'))])
        anchor=self.root/'semantic-metadata.duckdb'
        with duckdb.connect(str(anchor)) as c:
            for name in ('commits','comparisons','changes','parse_tasks','symbol_pairs','change_pairs'):
                c.execute('CREATE TABLE '+name+' AS SELECT * FROM read_parquet('+q(self.metadata/(name+'.parquet'))+')')
        self.inputs['metadata']={'path':str(anchor),'bytes':anchor.stat().st_size,'sha256':digest(anchor)}
        source_id=hashlib.sha256(json.dumps(self.inputs,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        changed=self.root/('gen-'+source_id); self.source.rename(changed); self.source=changed
        (self.source/'inputs.json').write_text(json.dumps(self.inputs))
        for path in (self.source/'receipts').glob('*.json'):
            value=json.loads(path.read_text()); value['generation']=source_id; path.write_text(json.dumps(value))
        with duckdb.connect(str(self.baseline)) as c:
            self.original.materialize_views(c,[str(p) for p in sorted((self.source/'pairs').glob('*.parquet'))])
        self.config={'source_generation':str(self.source),'metadata':{name:str(self.metadata/(name+'.parquet')) for name in ('commits','comparisons','changes','parse_tasks','symbol_pairs','change_pairs')},
                     'comparison_counts':str(self.root/'counts.parquet'),'output':str(self.root/'output'),
                     'memory_limit':'128MB','threads':1,'max_partition_candidates':5,'max_output_bytes':16000000,'min_free_bytes':0}
        self.configuration=self.root/'configuration.json'; self.write_config()

    def write_config(self):
        self.configuration.write_text(json.dumps(self.config))

    def run_finalizer(self):
        with contextlib.redirect_stdout(io.StringIO()):
            return self.module.Finalizer(self.configuration).run()

    def test_partitioned_native_views_match_original_and_preserve_parent_duplicates(self):
        original_hashes={p:digest(p) for p in self.source.rglob('*') if p.is_file()}
        result=self.run_finalizer()
        self.assertTrue(result['complete']); self.assertEqual(result['unique_pair_emissions'],8)
        self.assertEqual(result['raw_candidates'],9); self.assertEqual(result['native_matching_calls'],0)
        self.assertEqual(result['native_extraction_calls'],0)
        with duckdb.connect(result['database'],read_only=True) as c:
            c.execute('ATTACH '+q(self.baseline)+' AS expected (READ_ONLY)')
            for table in ('folded_pointers','symbol_snapshots','temporal_edges'):
                self.assertEqual(c.execute(f'SELECT count(*) FROM ((SELECT * FROM {table}) EXCEPT ALL (SELECT * FROM expected.{table}))').fetchone()[0],0,table)
                self.assertEqual(c.execute(f'SELECT count(*) FROM ((SELECT * FROM expected.{table}) EXCEPT ALL (SELECT * FROM {table}))').fetchone()[0],0,table)
            self.assertEqual(c.execute('SELECT count(*) FROM symbol_snapshots WHERE key_commit=? AND key_stable_symbol_id=?',[self.commits[3],'0'*15+'1']).fetchone()[0],2)
            self.assertEqual(c.execute('SELECT tokens FROM symbol_snapshots WHERE key_commit=? AND key_stable_symbol_id=?',[self.commits[3],'0'*15+'1']).fetchall(),[(['a','b'],),(['a','b'],)])
            self.assertEqual(c.execute('SELECT count(*) FROM commits').fetchone()[0],6)
        self.assertEqual({p:digest(p) for p in self.source.rglob('*') if p.is_file()},original_hashes)
        plan=json.loads((Path(result['generation_directory'])/'plan.json').read_text())
        self.assertTrue(all(batch['candidate_rows']<=5 for batch in plan['batches']))

    def test_restart_verifies_checkpoints_and_does_zero_new_projection_or_fold(self):
        first=self.run_finalizer(); second=self.run_finalizer()
        self.assertEqual(first['generation'],second['generation'])
        self.assertEqual(second['new_thin_shards_this_run'],0)
        self.assertEqual(second['new_fold_partitions_this_run'],0)
        self.assertTrue(second['complete'])

    def test_interrupted_units_are_retained_and_only_missing_receipts_recompute(self):
        first=self.run_finalizer(); gen=Path(first['generation_directory'])
        (gen/'result.json').rename(gen/'result.before-interruption.retained')
        thin=gen/'receipts/thin-000000.json'; fold=gen/'receipts/fold-000001.json'
        thin_value=json.loads(thin.read_text()); fold_value=json.loads(fold.read_text())
        retained=[thin_value['emissions'],thin_value['pair_state'],fold_value['folded']]
        thin.rename(thin.with_suffix('.interrupted-retained'))
        fold.rename(fold.with_suffix('.interrupted-retained'))
        result=self.run_finalizer()
        self.assertEqual(result['new_thin_shards_this_run'],1)
        self.assertEqual(result['new_fold_partitions_this_run'],1)
        records=[json.loads(p.read_text()) for p in (gen/'orphans').glob('*/*/quarantine.json')]
        hashes={entry['retained']['sha256'] for record in records for entry in record['files']}
        self.assertTrue(all(item['sha256'] in hashes for item in retained))
        self.assertTrue(result['complete'])

    def test_missing_receipt_from_completed_generation_fails_closed(self):
        result=self.run_finalizer(); gen=Path(result['generation_directory'])
        receipt=gen/'receipts/thin-000000.json'
        receipt.rename(receipt.with_suffix('.retained'))
        with self.assertRaisesRegex(ValueError,'completed generation.*missing checkpoint'):
            self.run_finalizer()

    def test_fault_after_data_publish_before_receipt_recovers_same_config(self):
        for phase,filename in (('thin','emissions-000001.parquet'),('fold','part-000001.parquet')):
            with self.subTest(phase=phase):
                self.config['output']=str(self.root/('publish-fault-'+phase)); self.write_config()
                job=self.module.Finalizer(self.configuration); original=job.copy
                def interrupted(c,query,path):
                    value=original(c,query,path)
                    if path.name==filename: raise ValueError('injected failure after data publication')
                    return value
                job.copy=interrupted
                with self.assertRaisesRegex(ValueError,'injected failure'):
                    with contextlib.redirect_stdout(io.StringIO()): job.run()
                first_verified=job.generation/'receipts'/('thin-000000.json' if phase=='thin' else 'fold-000000.json')
                before=digest(first_verified)
                resumed=self.run_finalizer()
                self.assertTrue(resumed['complete']); self.assertEqual(digest(first_verified),before)
                self.assertEqual(resumed['new_thin_shards_this_run'],1 if phase=='thin' else 0)
                self.assertEqual(resumed['new_fold_partitions_this_run'],3 if phase=='thin' else 2)
                self.assertTrue(list((job.generation/'orphans').glob('*/*/quarantine.json')))

    def test_corrupt_fold_checkpoint_cannot_reuse_old_completion(self):
        first=self.run_finalizer(); gen=Path(first['generation_directory'])
        fold=next((gen/'folds').glob('*.parquet'))
        with fold.open('ab') as f: f.write(b'corrupt')
        with self.assertRaisesRegex(ValueError,'checksum'):
            self.run_finalizer()
        self.assertFalse(json.loads((gen/'current-status.json').read_text())['complete'])
        self.assertTrue((gen/'result.json').exists())

    def test_source_receipt_corruption_is_rejected_before_finalizing(self):
        receipt=self.source/'receipts/000000.json'
        value=json.loads(receipt.read_text()); value['sha256']='0'*64; receipt.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError,'checksum'):
            self.run_finalizer()

    def test_cached_fold_count_is_verified_without_trusting_old_result(self):
        result=self.run_finalizer(); gen=Path(result['generation_directory'])
        (gen/'result.json').rename(gen/'result.retained.json')
        path=next((gen/'receipts').glob('fold-*.json'))
        value=json.loads(path.read_text()); value['rows']+=1; path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError,'fold.*count'):
            self.run_finalizer()

    def test_cached_emission_count_is_verified_without_trusting_old_result(self):
        result=self.run_finalizer(); gen=Path(result['generation_directory'])
        (gen/'result.json').rename(gen/'result.retained.json')
        path=next((gen/'receipts').glob('thin-*.json'))
        value=json.loads(path.read_text()); value['emitted_rows']+=1; path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError,'thin.*count'):
            self.run_finalizer()

    def test_parsed_source_inputs_cannot_bind_later_bytes(self):
        job=self.module.Finalizer(self.configuration)
        job.prepare()
        path=self.source/'inputs.json'; value=json.loads(path.read_text()); value['fixture']='changed'
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError,'source inputs.*changed'):
            job.prepare()

    def test_fresh_invocation_source_manifest_failure_invalidates_prior_success(self):
        result=self.run_finalizer()
        path=self.source/'inputs.json'; value=json.loads(path.read_text()); value['fixture']='changed'
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError,'source generation'):
            self.run_finalizer()
        self.assertFalse(json.loads((self.root/'output/current-status.json').read_text())['complete'])
        self.assertFalse(json.loads((Path(result['generation_directory'])/'current-status.json').read_text())['complete'])

    def test_same_id_metadata_export_change_cannot_claim_old_semantics(self):
        path=self.metadata/'changes.parquet'; temporary=self.root/'changed.parquet'
        with duckdb.connect() as c:
            c.execute("COPY (SELECT * REPLACE ('changed-path.rs' AS path) FROM read_parquet(?)) TO "+q(temporary)+' (FORMAT PARQUET)',[str(path)])
        temporary.replace(path)
        with self.assertRaisesRegex(ValueError,'metadata export.*equivalence'):
            self.module.Finalizer(self.configuration).prepare()

    def test_preflight_failure_after_success_invalidates_latest_attempt(self):
        result=self.run_finalizer(); path=self.source/'receipts/000000.json'
        value=json.loads(path.read_text()); value['sha256']='0'*64; path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError,'checksum'):
            self.run_finalizer()
        self.assertFalse(json.loads((Path(result['generation_directory'])/'current-status.json').read_text())['complete'])
        self.assertFalse(json.loads((self.root/'output/current-status.json').read_text())['complete'])
        self.assertTrue((Path(result['generation_directory'])/'result.json').exists())

    def test_post_unit_disk_admission_rejects_overshoot(self):
        job=self.module.Finalizer(self.configuration)
        original=job.accounting
        def too_large_after_projection():
            observed=original()
            if any(job.generation.glob('thin/emissions-*.parquet')):
                observed['output_bytes']=self.config['max_output_bytes']+1
            return observed
        job.accounting=too_large_after_projection
        with self.assertRaisesRegex(ValueError,'disk'):
            with contextlib.redirect_stdout(io.StringIO()): job.run()
        self.assertFalse((job.generation/'result.json').exists())
        self.assertFalse(json.loads((job.generation/'current-status.json').read_text())['complete'])

    def test_final_database_overshoot_cannot_publish_completion(self):
        job=self.module.Finalizer(self.configuration); original=job.accounting
        def too_large_after_database():
            observed=original()
            if (job.generation/'temporal.duckdb').exists(): observed['output_bytes']=self.config['max_output_bytes']+1
            return observed
        job.accounting=too_large_after_database
        with self.assertRaisesRegex(ValueError,'disk'):
            with contextlib.redirect_stdout(io.StringIO()): job.run()
        self.assertFalse((job.generation/'result.json').exists())
        self.assertFalse(json.loads((job.generation/'current-status.json').read_text())['complete'])

    def test_noncompleted_source_with_changes_is_rejected(self):
        receipt_path=self.source/'receipts/000000.json'
        receipt=json.loads(receipt_path.read_text()); source=self.source/receipt['file']; other=self.root/'changed-source.parquet'
        with duckdb.connect() as c:
            c.execute("COPY (SELECT * REPLACE('matching_error' AS status) FROM read_parquet(?)) TO "+q(other)+' (FORMAT PARQUET)',[str(source)])
        other.replace(source)
        receipt.update(sha256=digest(source),file_bytes=source.stat().st_size); receipt_path.write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError,'noncompleted.*changes'):
            self.run_finalizer()

    def test_complete_commit_larger_than_declared_cap_is_not_split_or_waived(self):
        self.config['max_partition_candidates']=4; self.write_config()
        with self.assertRaisesRegex(ValueError,'whole commit.*candidate'):
            self.module.Finalizer(self.configuration).prepare()
        self.assertFalse((self.root/'output').exists())

    def test_no_space_admission_is_honest_and_preserves_source(self):
        finalizer=self.module.Finalizer(self.configuration)
        finalizer.free_bytes=lambda:0
        with self.assertRaisesRegex(ValueError,'disk'):
            finalizer.run()
        self.assertTrue((self.source/'pairs/000000.parquet').exists())
        self.assertFalse(json.loads((Path(finalizer.plan['generation_directory'])/'current-status.json').read_text())['complete'])

    def test_select_list_zip_and_explain_avoid_payload_hash_group(self):
        source=self.source/'pairs/000001.parquet'
        with duckdb.connect() as c:
            rows=c.execute(self.module.narrow_sql(source)).fetchall()
            self.assertEqual([(r[0],r[1]) for r in rows],[(4,1),(5,1),(6,1),(6,2)])
            plan=c.execute('EXPLAIN '+self.module.narrow_sql(source)).fetchone()[1]
            self.assertNotIn('HASH_GROUP_BY',plan); self.assertNotIn('DELIM_SCAN',plan)
            self.assertEqual(c.execute('SELECT unnest(x),generate_subscripts(x,1) FROM (VALUES (NULL::INTEGER[]),([]::INTEGER[]),([1,2])) t(x)').fetchall(),[(1,1),(2,2)])

    def test_minmax_variant_predicate_matches_distinct_with_null_children(self):
        cases=[[('modified',None),('modified',None)],[('renamed_from_symbol',None),('renamed_from_symbol',0)],
               [('renamed_from_symbol',2**64-1),('renamed_from_symbol',2**64-1)],[(None,None),(None,None)],[(None,None),('modified',None)]]
        with duckdb.connect() as c:
            for rows in cases:
                c.execute('CREATE OR REPLACE TEMP TABLE variants(kind VARCHAR,previous_id UBIGINT)')
                c.executemany('INSERT INTO variants VALUES(?,?)',rows)
                old,new=c.execute('SELECT count(DISTINCT struct_pack(kind:=kind,previous_id:=previous_id))>1,min(struct_pack(kind:=kind,previous_id:=previous_id)) IS DISTINCT FROM max(struct_pack(kind:=kind,previous_id:=previous_id)) FROM variants').fetchone()
                self.assertEqual(old,new,rows)


if __name__=='__main__': unittest.main()
