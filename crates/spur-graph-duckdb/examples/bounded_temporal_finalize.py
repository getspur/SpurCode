"""Finalize immutable matched pairs using bounded, complete-commit thin folds.

Default --configuration FILE is read-only planning. --execute is a separately
reviewed action. No native extension is loaded and no extraction/matching runs.
The old failed generation/state is retained. Source payloads never enter grouped
state: SELECT-list UNNEST narrows one source shard, then complete-commit numeric
candidate batches use arg_min and a nullable min/max variant predicate.
"""

import argparse
import bisect
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import resource
import shutil
import sys
import time
import uuid

import duckdb


TABLES=('commits','comparisons','changes','parse_tasks','symbol_pairs','change_pairs')
THIN='pair_id BIGINT,emitted_ordinal BIGINT,stable_id UBIGINT,change_kind VARCHAR,previous_id UBIGINT'
FOLD='commit_ordinal UINTEGER,parent_ordinal UINTEGER,stable_id UBIGINT,pair_id BIGINT,emitted_ordinal BIGINT,change_id BIGINT,change_kind VARCHAR,previous_id UBIGINT'


def require(value,message):
    if not value: raise ValueError(message)


def q(value): return "'"+str(value).replace("'","''")+"'"
def sql_list(values): return '['+','.join(q(value) for value in values)+']'
def canonical(value): return json.dumps(value,sort_keys=True,separators=(',',':'))


def digest(path):
    with Path(path).open('rb') as stream: return hashlib.file_digest(stream,'sha256').hexdigest()


CONTROLLER_PATH=Path(__file__).resolve()
CONTROLLER_SHA=digest(CONTROLLER_PATH)


def descriptor(path):
    path=Path(path).resolve(strict=True)
    return {'path':str(path),'bytes':path.stat().st_size,'sha256':digest(path)}


def verify(item):
    require(descriptor(item['path'])==item,'checkpoint checksum/descriptor mismatch: '+item['path'])


def stage_json(path,value):
    temporary=path.with_name(path.name+'.'+uuid.uuid4().hex+'.tmp')
    with temporary.open('x') as stream:
        stream.write(json.dumps(value,indent=2,sort_keys=True)+'\n'); stream.flush(); os.fsync(stream.fileno())
    return temporary


def write_json(path,value,immutable=True):
    if immutable and path.exists():
        require(canonical(json.loads(path.read_text()))==canonical(value),'immutable manifest conflicts: '+str(path)); return
    temporary=stage_json(path,value)
    temporary.rename(path)


def narrow_sql(path):
    # SELECT-list UNNEST and generate_subscripts zip by index. The correlated
    # lateral WITH ORDINALITY form hashes the entire payload in DuckDB 2.0.
    return ('SELECT pair_id,emitted_ordinal,change.stable_symbol_id stable_symbol_id,change.change_kind change_kind,'
            'change.previous_stable_symbol_id previous_stable_symbol_id FROM ('
            'SELECT pair_id,unnest(changes) AS change,generate_subscripts(changes,1)::BIGINT emitted_ordinal '
            'FROM read_parquet('+q(path)+") WHERE status='completed') e ORDER BY pair_id,emitted_ordinal")


class Finalizer:
    def __init__(self,configuration):
        self.configuration_path=Path(configuration).resolve(strict=True)
        raw=self.configuration_path.read_bytes()
        self.config=json.loads(raw)
        self.configuration={'path':str(self.configuration_path),'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest(),'snapshot':self.config}
        self.started=time.perf_counter(); self.plan=None
        self.source=Path(self.config['source_generation']).resolve()
        self.source_inputs=None; self.source_inputs_sha=None; self.source_id=None

    @contextmanager
    def connection(self,database=':memory:'):
        c=duckdb.connect(str(database),config={'memory_limit':self.config['memory_limit'],'threads':1,'temp_directory':''})
        try: yield c
        finally: c.close()

    def metadata_views(self,c):
        for name in TABLES:
            c.execute('CREATE OR REPLACE VIEW '+name+' AS SELECT * FROM read_parquet('+q(self.config['metadata'][name])+')')

    def free_bytes(self):
        path=Path(self.config['output']).resolve()
        while not path.exists(): path=path.parent
        return shutil.disk_usage(path).free

    def accounting(self):
        used=sum(p.stat().st_size for p in self.generation.rglob('*') if p.is_file()) if self.generation.exists() else 0
        usage=resource.getrusage(resource.RUSAGE_SELF)
        return {'output_bytes':used,'free_disk_bytes':self.free_bytes(),'wall_seconds':time.perf_counter()-self.started,
                'cpu_user_seconds':usage.ru_utime,'cpu_system_seconds':usage.ru_stime,
                'max_rss_bytes':usage.ru_maxrss if sys.platform=='darwin' else usage.ru_maxrss*1024}

    def admit(self):
        values=self.accounting()
        require(values['free_disk_bytes']>self.config['min_free_bytes'],'disk admission failed: free space does not exceed configured reserve')
        require(values['output_bytes']<self.config['max_output_bytes'],'disk admission failed: measured output budget exhausted')

    def status(self,state,complete=False,**details):
        value={'generation':self.plan['generation'],'generation_directory':str(self.generation),'configuration_sha256':self.configuration['sha256'],
               'source_generation':str(self.source),'state':state,'complete':complete,**self.accounting(),**details}
        write_json(self.generation/'current-status.json',value,False)
        write_json(self.generation.parent/'current-status.json',value,False)
        print(json.dumps(value,sort_keys=True),flush=True)

    def metadata_equivalence(self,c):
        anchor=self.source_inputs.get('metadata')
        require(isinstance(anchor,dict),'metadata export equivalence requires the original semantic metadata descriptor')
        verify(anchor)
        c.execute('ATTACH '+q(anchor['path'])+' AS semantic_input (READ_ONLY)')
        proof={}
        try:
            for table in TABLES:
                schema=[(r[0],r[1]) for r in c.execute('DESCRIBE semantic_input.'+table).fetchall()]
                exported=[(r[0],r[1]) for r in c.execute('DESCRIBE '+table).fetchall()]
                require(schema==exported,'metadata export schema equivalence failed: '+table)
                left=c.cursor(); right=c.cursor(); count=0
                try:
                    left.execute('SELECT * FROM semantic_input.'+table)
                    right.execute('SELECT * FROM '+table)
                    while True:
                        old=left.fetchmany(duckdb.__standard_vector_size__)
                        new=right.fetchmany(duckdb.__standard_vector_size__)
                        require(old==new,f'metadata export exact typed-row equivalence failed: {table} near physical row {count}')
                        count+=len(old)
                        if not old: break
                finally: left.close(); right.close()
                proof[table]={'rows':count,'schema':[list(row) for row in schema]}
        finally: c.execute('DETACH semantic_input')
        verify(anchor)
        return {'source_metadata':anchor,'method':'exact ordered typed-row stream equality, stronger than row multiset equality',
                'batch_rows':duckdb.__standard_vector_size__,'tables':proof}

    def preflight_failure(self,error):
        output=Path(self.config['output']).resolve()
        if output.is_relative_to(self.source):
            print('preflight failed without writing inside immutable source: '+str(error),file=sys.stderr); return
        output.mkdir(parents=True,exist_ok=True)
        latest=output/'current-status.json'; previous=json.loads(latest.read_text()) if latest.exists() else {}
        value={'complete':False,'state':'failed','phase':'preflight','error':str(error),
               'configuration_sha256':self.configuration['sha256'],'source_generation':str(self.source)}
        if previous.get('configuration_sha256')==self.configuration['sha256'] and previous.get('source_generation')==str(self.source):
            old=Path(previous.get('generation_directory','')).resolve()
            if old.parent==output and old.name=='gen-'+str(previous.get('generation')) and old!=self.source:
                value.update(generation=previous['generation'],generation_directory=str(old))
                write_json(old/'current-status.json',value,False)
        write_json(latest,value,False)

    def batches(self,c):
        reader=c.cursor()
        reader.execute('SELECT p.pair_id,coalesce(o.byte_length,0)+coalesce(n.byte_length,0) input_bytes FROM symbol_pairs p '
                       'LEFT JOIN parse_tasks o ON p.language=o.language AND p.old_blob_oid=o.blob_oid '
                       'LEFT JOIN parse_tasks n ON p.language=n.language AND p.new_blob_oid=n.blob_oid ORDER BY p.pair_id')
        ids=[]; total=0; limit=self.source_inputs['row_limit']; budget=self.source_inputs['byte_budget']
        while chunk:=reader.fetchmany(limit):
            for pair,cost in chunk:
                if ids and (len(ids)>=limit or total+cost>budget): yield ids,total; ids=[]; total=0
                ids.append(pair); total+=cost
                if total>budget: yield ids,total; ids=[]; total=0
        if ids: yield ids,total
        reader.close()

    def prepare(self):
        require(digest(CONTROLLER_PATH)==CONTROLLER_SHA,'controller bytes changed after loading')
        require(digest(self.configuration_path)==self.configuration['sha256'],'parsed configuration changed')
        require(self.config['threads']==1,'bounded finalization requires one explicitly configured DuckDB thread')
        for key in ('max_partition_candidates','max_output_bytes'):
            require(type(self.config[key]) is int and self.config[key]>0,key+' must be an explicit positive policy')
        require(type(self.config['min_free_bytes']) is int and self.config['min_free_bytes']>=0,'min_free_bytes must be explicit')
        require(not Path(self.config['output']).resolve().is_relative_to(self.source),'output must not be inside the source generation')
        if self.source_inputs is None:
            source_bytes=(self.source/'inputs.json').read_bytes()
            self.source_inputs_sha=hashlib.sha256(source_bytes).hexdigest()
            self.source_inputs=json.loads(source_bytes)
            self.source_id=hashlib.sha256(canonical(self.source_inputs).encode()).hexdigest()
        require(self.source.name=='gen-'+self.source_id,'source generation does not bind its inputs')
        require(digest(self.source/'inputs.json')==self.source_inputs_sha,'parsed source inputs changed')
        metadata={name:descriptor(self.config['metadata'][name]) for name in TABLES}
        sources=[]
        with self.connection() as c:
            self.metadata_views(c)
            metadata_proof=self.metadata_equivalence(c)
            require(c.execute('SELECT count(*) FROM (SELECT ordinal FROM commits GROUP BY ordinal HAVING ordinal IS NULL OR ordinal<0 OR ordinal>4294967295 OR count(*)<>1)').fetchone()[0]==0,'commit ordinals must be unique UINTEGER values')
            require(c.execute('SELECT count(*) FROM (SELECT commit_oid FROM commits GROUP BY commit_oid HAVING commit_oid IS NULL OR count(*)<>1)').fetchone()[0]==0,'commit SHA mapping must be bijective')
            require(c.execute('SELECT count(*) FROM changes ch LEFT JOIN commits p ON ch.parent_oid=p.commit_oid WHERE ch.parent_oid IS NOT NULL AND p.commit_oid IS NULL').fetchone()[0]==0,'parent ordinal mapping is incomplete')
            for idx,(ids,cost) in enumerate(self.batches(c)):
                receipt_path=self.source/'receipts'/f'{idx:06}.json'
                receipt=json.loads(receipt_path.read_text())
                expected={'generation':self.source_id,'batch_id':idx,'pair_ids':ids,'rows':len(ids),'input_bytes':cost,
                          'row_limit':self.source_inputs['row_limit'],'byte_budget':self.source_inputs['byte_budget'],
                          'oversized':cost>self.source_inputs['byte_budget']}
                require(all(receipt.get(k)==v for k,v in expected.items()),'source receipt expected pair IDs/budget mismatch')
                file=(self.source/receipt['file']).resolve(strict=True)
                require(file.is_relative_to(self.source),'source pair file escapes generation')
                item=descriptor(file)
                require(item['sha256']==receipt['sha256'] and item['bytes']==receipt['file_bytes'],'source checkpoint checksum mismatch')
                actual=[row[0] for row in c.execute('SELECT pair_id FROM read_parquet(?) ORDER BY pair_id',[str(file)]).fetchall()]
                require(actual==ids,'source checkpoint actual pair IDs differ from metadata')
                sources.append({'batch_id':idx,'receipt':descriptor(receipt_path),'file':item,'pair_id_min':ids[0],'pair_id_max':ids[-1],'rows':len(ids)})
            expected_receipts={f'{idx:06}.json' for idx in range(len(sources))}
            require({p.name for p in (self.source/'receipts').glob('*.json')}==expected_receipts,'source checkpoint receipt coverage differs from metadata')
            comparison_counts=descriptor(self.config['comparison_counts'])
            c.execute('CREATE TEMP VIEW comparison_work AS SELECT * FROM read_parquet('+q(comparison_counts['path'])+')')
            require(c.execute('SELECT count(*) FROM ((SELECT comparison_id FROM comparisons) EXCEPT ALL (SELECT comparison_id FROM comparison_work))').fetchone()[0]==0,'comparison count basis omits IDs')
            require(c.execute('SELECT count(*) FROM ((SELECT comparison_id FROM comparison_work) EXCEPT ALL (SELECT comparison_id FROM comparisons))').fetchone()[0]==0,'comparison count basis has duplicate/extra IDs')
            require(c.execute('SELECT count(*) FROM comparison_work WHERE raw_candidates IS NULL OR raw_candidates<0').fetchone()[0]==0,'comparison counts must be nonnegative')
            rows=c.execute('SELECT m.ordinal,coalesce(sum(w.raw_candidates),0)::BIGINT candidates FROM commits m LEFT JOIN comparisons x USING(commit_oid) LEFT JOIN comparison_work w USING(comparison_id) GROUP BY m.ordinal ORDER BY m.ordinal').fetchall()
        partitions=[]; start=None; end=None; count=0; work=0
        for ordinal,candidates in rows:
            require(candidates<=self.config['max_partition_candidates'],f'whole commit {ordinal} exceeds candidate ceiling: {candidates}')
            if count and work+candidates>self.config['max_partition_candidates']:
                partitions.append({'partition_id':len(partitions),'ordinal_start':start,'ordinal_end':end,'candidate_rows':work,'commit_count':count})
                start=None; count=0; work=0
            if start is None: start=ordinal
            end=ordinal; count+=1; work+=candidates
        if count: partitions.append({'partition_id':len(partitions),'ordinal_start':start,'ordinal_end':end,'candidate_rows':work,'commit_count':count})
        policy={k:self.config[k] for k in ('memory_limit','threads','max_partition_candidates','max_output_bytes','min_free_bytes')}
        policy.update(temp_directory='',compression='ZSTD',row_group_size=duckdb.__standard_vector_size__,
                      partitioning='complete contiguous commit-ordinal batches; all parents remain together',
                      encoding='bijective UINTEGER commit/nullable parent ordinals and roundtrip-validated 16-lowercase-hex UBIGINT stable IDs',
                      fold='arg_min(change_id,emitted_ordinal); min/max nullable variant inequality => modified/previous NULL')
        require(digest(self.configuration_path)==self.configuration['sha256'],'parsed configuration changed during planning')
        require(digest(self.source/'inputs.json')==self.source_inputs_sha,'parsed source inputs changed during planning')
        inputs={'version':1,'controller':descriptor(CONTROLLER_PATH),'duckdb_version':duckdb.__version__,
                'configuration':self.configuration,'source_generation':{'directory':str(self.source),'generation':self.source_id,
                'inputs':descriptor(self.source/'inputs.json'),'current_status':descriptor(self.source/'current-status.json')},
                'source_inputs':self.source_inputs,'metadata':metadata,'metadata_equivalence':metadata_proof,'pair_sources':sources,
                'resource_basis':{'comparison_counts':comparison_counts},'policy':policy}
        generation=hashlib.sha256(canonical(inputs).encode()).hexdigest()
        self.generation=Path(self.config['output']).resolve()/('gen-'+generation)
        self.plan={'generation':generation,'generation_directory':str(self.generation),'inputs':inputs,'batches':partitions,
                   'raw_candidates':sum(row[1] for row in rows),'commits':len(rows),
                   'max_commit_candidates':max((row[1] for row in rows),default=0),'mode':'read_only_plan',
                   'disk_note':'full folded cardinality/compression is not yet known; each bounded unit checks measured disk admission; no unbounded spill or payload grouping'}
        return self.plan

    def copy(self,c,query,path):
        self.admit(); require(not path.exists(),'uncommitted output retained; refusing overwrite: '+str(path))
        temporary=path.with_name(path.name+'.'+uuid.uuid4().hex+'.tmp')
        c.execute('COPY ('+query+') TO '+q(temporary)+f' (FORMAT PARQUET,COMPRESSION ZSTD,ROW_GROUP_SIZE {duckdb.__standard_vector_size__})')
        with temporary.open('rb') as stream: os.fsync(stream.fileno())
        temporary.rename(path)
        self.admit()
        return descriptor(path)

    def quarantine_uncommitted(self,paths,unit):
        found=[]
        for path in paths:
            if path.exists(): found.append(path)
            found.extend(path.parent.glob(path.name+'.*.tmp'))
        if not found: return
        require(not (self.generation/'result.json').exists(),'completed generation has a missing checkpoint receipt; refusing implicit repair')
        directory=self.generation/'orphans'/unit/uuid.uuid4().hex
        directory.mkdir(parents=True)
        moved=[]
        for path in sorted(set(found)):
            require(path.resolve().is_relative_to(self.generation.resolve()),'uncommitted path escapes owned generation')
            before=descriptor(path); target=directory/path.name
            path.rename(target)
            after=descriptor(target)
            require(before['sha256']==after['sha256'] and before['bytes']==after['bytes'],'orphan quarantine changed bytes')
            moved.append({'original_path':str(path),'retained':after})
        write_json(directory/'quarantine.json',{'generation':self.plan['generation'],'unit':unit,
                                              'reason':'no completed receipt; preserve bytes and recompute only this unit','files':moved})

    def thin(self,source):
        idx=source['batch_id']; receipt_path=self.generation/'receipts'/f'thin-{idx:06}.json'
        expected={'generation':self.plan['generation'],'batch_id':idx,'source_shard':source['file'],'source_receipt':source['receipt']}
        if receipt_path.exists():
            receipt=json.loads(receipt_path.read_text()); require(all(receipt.get(k)==v for k,v in expected.items()),'thin checkpoint input mismatch')
            verify(receipt['emissions']); verify(receipt['pair_state'])
            expected_ids=json.loads(Path(source['receipt']['path']).read_text())['pair_ids']
            with self.connection() as c:
                c.execute('CREATE TEMP VIEW keys AS SELECT * FROM read_parquet('+q(receipt['emissions']['path'])+')')
                c.execute('CREATE TEMP VIEW state AS SELECT * FROM read_parquet('+q(receipt['pair_state']['path'])+')')
                schema=[(r[0],r[1]) for r in c.execute('DESCRIBE keys').fetchall()]
                require(schema==[('pair_id','BIGINT'),('emitted_ordinal','BIGINT'),('stable_id','UBIGINT'),('change_kind','VARCHAR'),('previous_id','UBIGINT')],'thin checkpoint schema mismatch')
                ids=[r[0] for r in c.execute('SELECT pair_id FROM state ORDER BY pair_id').fetchall()]
                count=c.execute('SELECT count(*) FROM keys').fetchone()[0]
                require(ids==expected_ids==receipt['source_pair_ids'] and len(ids)==receipt['pair_rows'] and count==receipt['emitted_rows'],'thin checkpoint count/coverage mismatch')
                require(c.execute("SELECT count(*) FROM state WHERE status<>'completed' AND coalesce(raw_change_count,0)>0").fetchone()[0]==0,'noncompleted native source must not contain changes')
                require(c.execute('SELECT count(*) FROM (SELECT pair_id,emitted_ordinal FROM keys GROUP BY ALL HAVING count(*)<>1)').fetchone()[0]==0,'thin checkpoint duplicate pointer')
                require(c.execute('SELECT count(*) FROM state s LEFT JOIN (SELECT pair_id,count(*) n,min(emitted_ordinal) lo,max(emitted_ordinal) hi FROM keys GROUP BY pair_id) k USING(pair_id) WHERE coalesce(k.n,0)<>s.emitted_count OR (s.emitted_count>0 AND (k.lo<>1 OR k.hi<>s.emitted_count))').fetchone()[0]==0,'thin checkpoint ordinal/count mismatch')
            return receipt,False
        self.quarantine_uncommitted([self.generation/'thin'/f'emissions-{idx:06}.parquet',
                                     self.generation/'thin'/f'pair-state-{idx:06}.parquet'],f'thin-{idx:06}')
        with self.connection() as c:
            c.execute('CREATE TEMP TABLE raw_keys AS '+narrow_sql(source['file']['path']))
            require(c.execute("SELECT count(*) FROM raw_keys WHERE stable_symbol_id IS NULL OR NOT regexp_full_match(stable_symbol_id,'[0-9a-f]{16}') OR (previous_stable_symbol_id IS NOT NULL AND NOT regexp_full_match(previous_stable_symbol_id,'[0-9a-f]{16}')) OR change_kind IS NULL").fetchone()[0]==0,'stable IDs must roundtrip exactly as 16 lowercase hex digits')
            c.execute("CREATE TEMP TABLE keys AS SELECT pair_id,emitted_ordinal,('0x'||stable_symbol_id)::UBIGINT stable_id,change_kind,CASE WHEN previous_stable_symbol_id IS NOT NULL THEN ('0x'||previous_stable_symbol_id)::UBIGINT END previous_id FROM raw_keys")
            require(c.execute("SELECT count(*) FROM raw_keys r JOIN keys k USING(pair_id,emitted_ordinal) WHERE r.stable_symbol_id<>lower(lpad(hex(k.stable_id),16,'0')) OR r.previous_stable_symbol_id IS DISTINCT FROM CASE WHEN k.previous_id IS NOT NULL THEN lower(lpad(hex(k.previous_id),16,'0')) END").fetchone()[0]==0,'stable ID encoding is not bijective')
            c.execute("CREATE TEMP TABLE state AS SELECT pair_id,status,array_length(changes)::BIGINT raw_change_count,CASE WHEN status='completed' THEN coalesce(array_length(changes),0) ELSE 0 END::BIGINT emitted_count,coalesce(array_length(diagnostics),0)::BIGINT diagnostic_count FROM read_parquet("+q(source['file']['path'])+')')
            require(c.execute("SELECT count(*) FROM state WHERE status IS NULL OR status NOT IN ('completed','binary_skipped','parse_failed','matching_error')").fetchone()[0]==0,'source matching status is not terminal')
            require(c.execute("SELECT count(*) FROM state WHERE status<>'completed' AND coalesce(raw_change_count,0)>0").fetchone()[0]==0,'noncompleted native source must not contain changes')
            pairs=c.execute('SELECT pair_id FROM state ORDER BY pair_id').fetchall()
            require(len(pairs)==source['rows'],'source pair-state coverage mismatch')
            require(c.execute('SELECT count(*) FROM (SELECT pair_id,emitted_ordinal FROM keys GROUP BY ALL HAVING count(*)<>1)').fetchone()[0]==0,'duplicate emission pointer')
            require(c.execute('SELECT count(*) FROM state s LEFT JOIN (SELECT pair_id,count(*) n,min(emitted_ordinal) lo,max(emitted_ordinal) hi FROM keys GROUP BY pair_id) k USING(pair_id) WHERE coalesce(k.n,0)<>s.emitted_count OR (s.emitted_count>0 AND (k.lo<>1 OR k.hi<>s.emitted_count))').fetchone()[0]==0,'emission ordinality/count differs from native source')
            count=c.execute('SELECT count(*) FROM keys').fetchone()[0]
            emissions=self.copy(c,'SELECT * FROM keys ORDER BY pair_id,emitted_ordinal',self.generation/'thin'/f'emissions-{idx:06}.parquet')
            state=self.copy(c,'SELECT * FROM state ORDER BY pair_id',self.generation/'thin'/f'pair-state-{idx:06}.parquet')
        receipt=dict(expected,source_pair_ids=[row[0] for row in pairs],pair_id_min=source['pair_id_min'],pair_id_max=source['pair_id_max'],
                     pair_rows=len(pairs),emitted_rows=count,emissions=emissions,pair_state=state)
        write_json(receipt_path,receipt); return receipt,True

    def occurrence_index(self):
        receipt_path=self.generation/'receipts/occurrences.json'
        if receipt_path.exists():
            receipt=json.loads(receipt_path.read_text()); require(receipt['generation']==self.plan['generation'],'occurrence input mismatch'); verify(receipt['occurrences'])
            with self.connection() as c:
                self.metadata_views(c)
                rows=c.execute('SELECT count(*) FROM read_parquet(?)',[receipt['occurrences']['path']]).fetchone()[0]
                require(rows==receipt['rows']==c.execute('SELECT count(*) FROM change_pairs').fetchone()[0],'occurrence checkpoint count mismatch')
            return receipt
        self.quarantine_uncommitted([self.generation/'thin/occurrences.parquet'],'occurrences')
        with self.connection() as c:
            self.metadata_views(c)
            query=('SELECT m.ordinal::UINTEGER commit_ordinal,p.ordinal::UINTEGER parent_ordinal,cp.pair_id,ch.change_id '
                   'FROM changes ch JOIN change_pairs cp USING(change_id) JOIN commits m USING(commit_oid) '
                   'LEFT JOIN commits p ON ch.parent_oid=p.commit_oid')
            item=self.copy(c,query,self.generation/'thin/occurrences.parquet')
            rows=c.execute('SELECT count(*) FROM read_parquet(?)',[item['path']]).fetchone()[0]
            require(rows==c.execute('SELECT count(*) FROM change_pairs').fetchone()[0],'numeric occurrence projection lost/duplicated metadata rows')
        receipt={'generation':self.plan['generation'],'rows':rows,'occurrences':item}
        write_json(receipt_path,receipt); return receipt

    def working_views(self,c,thin,occurrences):
        self.metadata_views(c)
        c.execute('CREATE OR REPLACE VIEW pair_state AS SELECT * FROM read_parquet('+sql_list(row['pair_state']['path'] for row in thin)+')')
        c.execute('CREATE OR REPLACE VIEW thin_occurrences AS SELECT * FROM read_parquet('+q(occurrences['occurrences']['path'])+')')

    def fold(self,batch,thin,occurrences):
        idx=batch['partition_id']; receipt_path=self.generation/'receipts'/f'fold-{idx:06}.json'
        expected={'generation':self.plan['generation'],'partition_id':idx,'ordinal_start':batch['ordinal_start'],'ordinal_end':batch['ordinal_end'],'expected_candidates':batch['candidate_rows']}
        if receipt_path.exists():
            receipt=json.loads(receipt_path.read_text()); require(all(receipt.get(k)==v for k,v in expected.items()),'fold checkpoint input mismatch'); verify(receipt['folded'])
            with self.connection() as c:
                c.execute('CREATE TEMP VIEW folded AS SELECT * FROM read_parquet('+q(receipt['folded']['path'])+')')
                schema=[(r[0],r[1]) for r in c.execute('DESCRIBE folded').fetchall()]
                require(schema==[('commit_ordinal','UINTEGER'),('parent_ordinal','UINTEGER'),('stable_id','UBIGINT'),('pair_id','BIGINT'),('emitted_ordinal','BIGINT'),('change_id','BIGINT'),('change_kind','VARCHAR'),('previous_id','UBIGINT')],'fold checkpoint schema mismatch')
                rows=c.execute('SELECT count(*) FROM folded').fetchone()[0]
                unscoped=c.execute('SELECT count(*) FROM (SELECT commit_ordinal,stable_id FROM folded GROUP BY ALL)').fetchone()[0]
                require(rows==receipt['rows'] and unscoped==receipt['unscoped_keys'] and rows-unscoped==receipt['cross_parent_key_repetitions'] and receipt['actual_candidates']==batch['candidate_rows'],'fold checkpoint count mismatch')
                require(c.execute('SELECT count(*) FROM (SELECT commit_ordinal,parent_ordinal,stable_id FROM folded GROUP BY ALL HAVING count(*)<>1)').fetchone()[0]==0,'fold checkpoint duplicate scoped key')
                require(c.execute('SELECT count(*) FROM folded WHERE commit_ordinal<? OR commit_ordinal>?',[batch['ordinal_start'],batch['ordinal_end']]).fetchone()[0]==0,'fold checkpoint ordinal routing mismatch')
            return receipt,False
        self.quarantine_uncommitted([self.generation/'folds'/f'part-{idx:06}.parquet'],f'fold-{idx:06}')
        with self.connection() as c:
            self.working_views(c,thin,occurrences)
            where=f"o.commit_ordinal BETWEEN {batch['ordinal_start']} AND {batch['ordinal_end']} AND s.emitted_count>0"
            actual=c.execute('SELECT coalesce(sum(s.emitted_count),0) FROM thin_occurrences o JOIN pair_state s USING(pair_id) WHERE '+where).fetchone()[0]
            require(actual==batch['candidate_rows'],'actual whole-commit candidate count differs from resource basis')
            require(actual<=self.config['max_partition_candidates'],'whole commit batch exceeds candidate budget')
            c.execute('CREATE TEMP TABLE occ AS SELECT o.* FROM thin_occurrences o JOIN pair_state s USING(pair_id) WHERE '+where)
            ids=[row[0] for row in c.execute('SELECT DISTINCT pair_id FROM occ ORDER BY pair_id').fetchall()]
            files=[]
            for row in thin:
                at=bisect.bisect_left(ids,row['pair_id_min'])
                if at<len(ids) and ids[at]<=row['pair_id_max']: files.append(row['emissions']['path'])
            if files:
                c.execute('CREATE TEMP TABLE keys AS SELECT * FROM read_parquet('+sql_list(files)+') SEMI JOIN (SELECT DISTINCT pair_id FROM occ) p USING(pair_id)')
            else: c.execute('CREATE TEMP TABLE keys('+THIN+')')
            c.execute('CREATE TEMP TABLE candidates AS SELECT o.*,k.emitted_ordinal,k.stable_id,k.change_kind,k.previous_id FROM occ o JOIN keys k USING(pair_id)')
            require(c.execute('SELECT count(*) FROM candidates').fetchone()[0]==actual,'narrow pointer cache omitted/duplicated candidate rows')
            c.execute('CREATE TEMP TABLE folded AS WITH grouped AS (SELECT commit_ordinal,parent_ordinal,stable_id,'
                      'arg_min(struct_pack(pair_id:=pair_id,emitted_ordinal:=emitted_ordinal,change_id:=change_id,kind:=change_kind,previous_id:=previous_id),struct_pack(change_id:=change_id,emitted_ordinal:=emitted_ordinal)) chosen,'
                      'min(struct_pack(kind:=change_kind,previous_id:=previous_id)) IS DISTINCT FROM max(struct_pack(kind:=change_kind,previous_id:=previous_id)) conflict '
                      'FROM candidates GROUP BY commit_ordinal,parent_ordinal,stable_id) '
                      "SELECT commit_ordinal,parent_ordinal,stable_id,chosen.pair_id pair_id,chosen.emitted_ordinal emitted_ordinal,chosen.change_id change_id,CASE WHEN conflict THEN 'modified' ELSE chosen.kind END change_kind,CASE WHEN conflict THEN NULL ELSE chosen.previous_id END previous_id FROM grouped")
            rows=c.execute('SELECT count(*) FROM folded').fetchone()[0]
            unscoped=c.execute('SELECT count(*) FROM (SELECT commit_ordinal,stable_id FROM folded GROUP BY ALL)').fetchone()[0]
            item=self.copy(c,'SELECT * FROM folded ORDER BY commit_ordinal,parent_ordinal,stable_id',self.generation/'folds'/f'part-{idx:06}.parquet')
        receipt=dict(expected,actual_candidates=actual,rows=rows,unscoped_keys=unscoped,cross_parent_key_repetitions=rows-unscoped,folded=item)
        write_json(receipt_path,receipt); return receipt,True

    def final_views(self,c,thin,occurrences,folds):
        self.working_views(c,thin,occurrences)
        files=[row['folded']['path'] for row in folds]
        if files: c.execute('CREATE OR REPLACE VIEW folded_encoded AS SELECT * FROM read_parquet('+sql_list(files)+')')
        else:
            c.execute('CREATE TABLE empty_fold('+FOLD+')')
            c.execute('CREATE VIEW folded_encoded AS SELECT * FROM empty_fold')
        c.execute("CREATE OR REPLACE VIEW folded_pointers AS SELECT m.commit_oid,p.commit_oid parent_oid,lower(lpad(hex(f.stable_id),16,'0')) stable_symbol_id,pair_id,emitted_ordinal,change_id,change_kind,CASE WHEN previous_id IS NOT NULL THEN lower(lpad(hex(previous_id),16,'0')) END previous_stable_symbol_id FROM folded_encoded f JOIN commits m ON f.commit_ordinal=m.ordinal LEFT JOIN commits p ON f.parent_ordinal=p.ordinal")
        c.execute('CREATE OR REPLACE VIEW pair_results AS SELECT * FROM read_parquet('+sql_list(row['file']['path'] for row in self.plan['inputs']['pair_sources'])+')')
        c.execute('CREATE OR REPLACE VIEW pair_emissions AS SELECT pair_id,unnest(changes) change,generate_subscripts(changes,1)::BIGINT emitted_ordinal FROM pair_results WHERE status=\'completed\'')
        c.execute('CREATE OR REPLACE VIEW symbol_changes AS SELECT f.*,list_extract(p.changes,f.emitted_ordinal).symbol symbol,list_extract(p.changes,f.emitted_ordinal).file_path file_path FROM folded_pointers f JOIN pair_results p USING(pair_id)')
        c.execute("CREATE OR REPLACE VIEW symbol_snapshots AS SELECT stable_symbol_id key_stable_symbol_id,commit_oid key_commit,rtrim(to_base64(encode(file_path)),'=') file_path_b64,symbol.entity_name entity_name,symbol.symbol_kind symbol_kind,symbol.enclosing_scope enclosing_scope,symbol.byte_start::BIGINT byte_range_start,symbol.byte_end::BIGINT byte_range_end,symbol.line_start::BIGINT line_range_start,symbol.line_end::BIGINT line_range_end,symbol.anchor_hash anchor_hash,symbol.tokens tokens FROM symbol_changes")
        c.execute("CREATE OR REPLACE VIEW temporal_edges AS SELECT 'commit' source_kind,NULL::VARCHAR source_path_b64,NULL::VARCHAR source_stable_symbol_id,commit_oid source_commit,'file' target_kind,rtrim(to_base64(encode(path)),'=') target_path_b64,NULL::VARCHAR target_stable_symbol_id,NULL::VARCHAR target_commit,'touches' relation,parent_oid parent,CASE WHEN gitlink THEN 'modified' WHEN left(status,1)='A' THEN 'added' WHEN left(status,1)='D' THEN 'deleted' WHEN left(status,1) IN ('R','C') THEN 'renamed_from_file' ELSE 'modified' END change_kind,CASE WHEN NOT gitlink AND left(status,1) IN ('R','C') THEN rtrim(to_base64(encode(old_path)),'=') END rename_prev_path_b64,NULL::VARCHAR rename_prev_stable_symbol_id,NULL::VARCHAR rename_prev_commit FROM changes "
                  "UNION ALL SELECT 'commit',NULL,NULL,commit_oid,'snapshot',NULL,stable_symbol_id,commit_oid,'touches',parent_oid,change_kind,NULL,previous_stable_symbol_id,CASE WHEN previous_stable_symbol_id IS NOT NULL THEN commit_oid END FROM folded_pointers "
                  "UNION ALL SELECT 'snapshot',NULL,previous_stable_symbol_id,commit_oid,'snapshot',NULL,stable_symbol_id,commit_oid,'touches',parent_oid,change_kind,NULL,previous_stable_symbol_id,commit_oid FROM folded_pointers WHERE change_kind='renamed_from_symbol'")
        c.execute('CREATE OR REPLACE VIEW pair_occurrences AS SELECT ch.change_id,ch.commit_oid,ch.parent_oid,cp.pair_id,s.status,s.raw_change_count change_count FROM changes ch JOIN change_pairs cp USING(change_id) JOIN pair_state s USING(pair_id)')
        c.execute('CREATE OR REPLACE VIEW pair_diagnostics AS SELECT pair_id,unnest(diagnostics) diagnostic FROM pair_results')
        c.execute("CREATE OR REPLACE VIEW diagnostics AS SELECT ch.commit_oid,ch.parent_oid,ch.change_id,p.pair_id,s.status,p.diagnostic FROM changes ch JOIN change_pairs cp USING(change_id) JOIN pair_diagnostics p USING(pair_id) JOIN pair_state s USING(pair_id) UNION ALL SELECT commit_oid,parent_oid,change_id,NULL::BIGINT,'gitlink','gitlink: file='||path||' commit='||commit_oid||' skipped submodule recursion; file-level touch retained' FROM changes WHERE gitlink")

    def run(self):
        try: self.prepare()
        except Exception as error:
            self.preflight_failure(error); raise
        self.generation.mkdir(parents=True,exist_ok=True)
        for name in ('thin','folds','receipts'): (self.generation/name).mkdir(exist_ok=True)
        with (self.source/'.lock').open('rb') as source_lock,(self.generation/'.lock').open('a') as output_lock:
            fcntl.flock(source_lock,fcntl.LOCK_SH|fcntl.LOCK_NB); fcntl.flock(output_lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            try:
                write_json(self.generation/'inputs.json',self.plan['inputs']); write_json(self.generation/'plan.json',self.plan)
                self.status('narrowing'); thin=[]; new_thin=0
                for source in self.plan['inputs']['pair_sources']:
                    self.admit(); receipt,new=self.thin(source); thin.append(receipt); new_thin+=new
                    self.status('thin_checkpoint',source_shard=source['batch_id'],unique_pair_emissions=sum(row['emitted_rows'] for row in thin))
                occurrences=self.occurrence_index(); folds=[]; new_fold=0
                for batch in self.plan['batches']:
                    self.admit(); receipt,new=self.fold(batch,thin,occurrences); folds.append(receipt); new_fold+=new
                    self.status('fold_checkpoint',partition_id=batch['partition_id'],folded_rows=sum(row['rows'] for row in folds))
                database=self.generation/'temporal.duckdb'
                self.admit()
                with self.connection(database) as c:
                    self.final_views(c,thin,occurrences,folds)
                    counts={name:c.execute('SELECT count(*) FROM '+name).fetchone()[0] for name in ('commits','comparisons','changes','symbol_pairs','pair_occurrences')}
                    rows=sum(row['rows'] for row in folds); renames=c.execute("SELECT count(*) FROM folded_encoded WHERE change_kind='renamed_from_symbol'").fetchone()[0]
                    diagnostic_rows=c.execute('SELECT coalesce(sum(s.diagnostic_count),0) FROM thin_occurrences o JOIN pair_state s USING(pair_id)').fetchone()[0]+c.execute('SELECT count(*) FROM changes WHERE gitlink').fetchone()[0]
                    counts.update(symbol_snapshots=rows,temporal_edges=counts['changes']+rows+renames,diagnostics=diagnostic_rows)
                    statuses=dict(c.execute('SELECT status,count(*) FROM pair_state GROUP BY status').fetchall())
                    c.execute('CHECKPOINT')
                self.admit()
                raw=sum(row['actual_candidates'] for row in folds)
                require(raw==self.plan['raw_candidates'],'final candidate reconciliation failed')
                result={'complete':True,'state':'completed_with_errors' if statuses.get('parse_failed',0) or statuses.get('matching_error',0) else 'completed',
                        'generation':self.plan['generation'],'generation_directory':str(self.generation),'database':str(database),
                        'source_generation':str(self.source),'source_semantic_generation_id':self.source_id,'counts':counts,
                        'unique_pair_emissions':sum(row['emitted_rows'] for row in thin),'raw_candidates':raw,'folded_rows':rows,
                        'scoped_unique_keys':rows,'unscoped_unique_keys':sum(row['unscoped_keys'] for row in folds),
                        'cross_parent_key_repetitions':sum(row['cross_parent_key_repetitions'] for row in folds),
                        'partition_count':len(folds),'occurrence_index':occurrences['occurrences'],'pair_statuses':statuses,
                        'native_extraction_calls':0,'native_matching_calls':0,'policy':self.plan['inputs']['policy']}
                # Stage success records first, then admit their measured bytes.
                # No complete marker is published until the last disk check.
                target=self.generation/'result.json'
                if target.exists():
                    require(json.loads(target.read_text())==result,'immutable result conflicts')
                    pending_result=None
                else: pending_result=stage_json(target,result)
                value={'generation':self.plan['generation'],'generation_directory':str(self.generation),
                       'configuration_sha256':self.configuration['sha256'],'source_generation':str(self.source),
                       'state':result['state'],'complete':True,'counts':counts,**self.accounting()}
                pending_status=stage_json(self.generation/'current-status.json',value)
                pending_latest=stage_json(self.generation.parent/'current-status.json',value)
                self.admit()
                if pending_result: pending_result.rename(target)
                pending_status.rename(self.generation/'current-status.json')
                pending_latest.rename(self.generation.parent/'current-status.json')
                print(json.dumps(value,sort_keys=True),flush=True)
                return dict(result,new_thin_shards_this_run=new_thin,new_fold_partitions_this_run=new_fold,resources=self.accounting())
            except Exception as error:
                self.status('failed',False,error=str(error)); raise


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--configuration',type=Path,required=True); parser.add_argument('--execute',action='store_true')
    args=parser.parse_args(); job=Finalizer(args.configuration)
    result=job.run() if args.execute else job.prepare()
    print(json.dumps(result,sort_keys=True),flush=True)


if __name__=='__main__': main()
