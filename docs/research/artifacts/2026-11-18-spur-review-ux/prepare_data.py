"""Build the design fixture from committed experimental evidence, without new collection."""
import json
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTEXT = HERE.parent / '2026-11-18-pr-repository-context-map'
POC = HERE.parent / '2026-11-17-review-extensions-poc'
relations = json.loads((CONTEXT / 'relationships.json').read_text())
native = json.loads((CONTEXT / 'native-incoming-paths.json').read_text())
HEAD = 'd29b26bb92ac940a0807be0f43cc385b31623839'
STATS = 'src/optimizer/relation_statistics/relation_statistics_helper.cpp'
REWRITE = 'src/function/aggregate/distributive/first_last_any.cpp'
TEST = 'test/sql/optimizer/statistics/distinct_on_stats_propagation.test'
evidence = {}

def source(path, line, end=None):
    lines = (CONTEXT / 'pinned-context' / path).read_text().splitlines()
    lo, hi = max(1, line - 2), min(len(lines), end or line + 3)
    return [{'n': n, 'text': lines[n-1]} for n in range(lo, hi+1)]

def rel(predicate):
    idx, r = next((i, x) for i, x in enumerate(relations) if predicate(x))
    key = f'context:{idx}'
    if key not in evidence:
        evidence[key] = dict(r, id=key, location='Exact source witness',
                             lines=source(r['witness_path'], r['witness_line']),
                             artifact='relationships.json', artifact_index=idx)
    return key

def exact(key, path, line, title, quality, limit, artifact, index=None):
    evidence[key] = dict(id=key, relation=title, evidence_class=quality,
                         witness_path=path, witness_line=line, revision=HEAD,
                         witness_text='\n'.join(x['text'] for x in source(path,line)),
                         lines=source(path,line), caveat=limit, artifact=artifact,
                         artifact_index=index, location='Source span with retained native path')
    return key

client = rel(lambda x: x['relation']=='references_aggregate_stats_api' and x['witness_line']==623)
extractor = rel(lambda x: x['relation']=='references_aggregate_stats_api' and 'extractor.cpp' in x['source_path'])
wrapper = exact('native:wrapper',STATS,693,'Wrapper calls changed internal helper','native_bound_syntax',
                'Syntax binding from the retained two-file graph. The wrapper body is unchanged; compiler resolution is not claimed.',
                'native-incoming-paths.json',6)
helpers = exact('native:helpers',STATS,627,'Internal helper calls uniqueness proof','native_bound_syntax',
                'Syntax binding, not compiler proof. Open the frozen path record for graph-scoped identities.',
                'native-incoming-paths.json',next(i for i,x in enumerate(native) if x['seed']=='HasUniqueGroups' and x['call_hops']==1))
# The source line shown for each native path is a source context anchor, not a claimed call-site coordinate.
for key in ['native:wrapper','native:helpers']:
    evidence[key]['location']='Source context anchor; exact graph path is in the retained receipt'
def native_leg(target):
    idx, record = next((i,x) for i,x in enumerate(native) if x['seed']==target and x['call_hops']==1 and x['nodes'][0]['name']=='ExtractAggregationStatsInternal')
    lines=(CONTEXT/'pinned-context'/STATS).read_text().splitlines()
    line=next(i+1 for i,x in enumerate(lines) if i+1>=558 and target+'(' in x)
    return exact('native:'+target, STATS, line, 'Internal implementation calls '+target,
                 'native_bound_syntax','Retained syntax binding, not compiler resolution.', 'native-incoming-paths.json',idx)
base_call=native_leg('HasBaseColumnSource')
value_call=native_leg('GetValuePreservingColumn')
write = rel(lambda x:x['relation']=='writes_shared_identity_contract' and x['source_path']==REWRITE and x['witness_line']==57)
read = rel(lambda x:x['relation']=='reads_shared_identity_contract' and x['source_path']==STATS)
cte = rel(lambda x:x['relation']=='reads_shared_identity_contract' and 'physical_recursive' in x['source_path'])
callback = rel(lambda x:x['relation']=='dispatches_possible_registered_rewrite')
skip = rel(lambda x:x['relation']=='skips_test_in_mode' and x['source_path'].endswith('stats_dependent_tests.json'))
inherit1=rel(lambda x:x['relation']=='inherits_skip_tests' and x['source_path'].endswith('/disable_optimizer.json'))
inherit2=rel(lambda x:x['relation']=='inherits_skip_tests' and x['source_path'].endswith('/verify_serializer.json'))
direct=rel(lambda x:x['relation']=='skips_test_in_mode' and x['source_path'].endswith('verify_aggregate_state_export.json'))
dependency=rel(lambda x:x['relation']=='calls_dependency_in_shared_syntax_graph' and x.get('source_symbol')=='ExtractAggregationStatsInternal')

def node(id, title, sub, state, x, y, ev):
    return dict(id=id,title=title,sub=sub,state=state,x=x,y=y,evidence=ev)
def edge(a,b,label,ev,kind='candidate'):
    return dict(a=a,b=b,label=label,evidence=ev,kind=kind)
def row(title,body,ev,kind='candidate'):
    return dict(title=title,body=body,evidence=ev,kind=kind)

contexts={
 'incoming':dict(title='Who reaches this change?', subtitle='Unchanged clients, an unchanged wrapper and the modified implementation.',
   caveat='External API references are candidates. The wrapper-to-internal leg is syntax-bound.',
   nodes=[node('client','Relation manager','relation_manager.cpp','Unchanged',20,38,client),
          node('extractor','Statistics extractor','relation_statistics_extractor.cpp','Unchanged',20,171,extractor),
          node('wrapper','ExtractAggregationStats','Two overloads inside modified file','Unchanged body',278,102,wrapper),
          node('internal','ExtractAggregationStatsInternal','relation_statistics_helper.cpp','Modified',536,102,helpers)],
   edges=[edge('client','wrapper','API reference',client),edge('extractor','wrapper','API reference',extractor),edge('wrapper','internal','Calls',wrapper,'syntax')],
   rows=[row('Join-order client → public wrapper → changed internal','The first leg references an overloaded API. The second has a retained syntax binding.',client),
         row('Unchanged wrapper inside a modified file','Identical body bytes; moved from base lines 554 / 562 to head 688 / 696.',wrapper,'syntax'),
         row('Changed internal → new uniqueness proof','Follow the new helper without treating syntax matching as compiler resolution.',helpers,'syntax')]),
 'dependencies':dict(title='What does this change depend on?', subtitle='A bounded view of retained syntax bindings from the selected statistics change.',
   caveat='These are syntax matches. C++ receiver, qualifier and overload resolution still have known gaps.',
   nodes=[node('internal','ExtractAggregationStatsInternal','Modified statistics implementation','Modified',20,104,helpers),
          node('unique','HasUniqueGroups','New singleton-group proof','Added',278,38,helpers),
          node('source','HasBaseColumnSource','New base-column proof','Added',278,171,base_call),
          node('value','GetValuePreservingColumn','New expression proof','Added',536,104,value_call)],
   edges=[edge('internal','unique','Calls',helpers,'syntax'),edge('internal','source','Calls',base_call,'syntax'),edge('internal','value','Calls',value_call,'syntax')],
   rows=[row('Internal implementation → HasUniqueGroups','Retained native syntax path. Open its source and binding receipt.',helpers,'syntax'),
         row('Statistics reader → aggregate identity contract','The statistics guard reads the unchanged metadata API.',read,'contract'),
         row('Repository dependency binding receipt','Cross-file shared Rust graph candidate; function anchor is not an exact call site.',dependency,'syntax')]),
 'contract':dict(title='Which shared contracts deserve review?', subtitle='The changed producer and statistics reader share metadata with unchanged code.',
   caveat='Shared contract participation does not establish runtime impact on the recursive-CTE consumer.',
   nodes=[node('rewrite','RewriteOrderedFirst','first_last_any.cpp','Modified',20,38,write),
          node('stats','Statistics helper','relation_statistics_helper.cpp','Modified',20,171,read),
          node('identity','single_value_identity','aggregate_function.hpp','Unchanged',278,102,write),
          node('cte','Recursive CTE consumer','physical_recursive_cte_keyed.cpp','Unchanged',536,102,cte)],
   edges=[edge('rewrite','identity','Writes',write,'contract'),edge('stats','identity','Reads',read,'contract'),edge('cte','identity','Reads',cte,'contract')],
   rows=[row('Changed rewrite → shared identity contract','The rewrite propagates HasSingleValueIdentity into the replacement aggregate.',write,'contract'),
         row('Unchanged recursive-CTE consumer → same contract','A concrete review neighbor. Runtime propagation from this PR is not established.',cte,'contract'),
         row('Unchanged dispatcher → possible rewrite callback','Inspect one dispatcher and all four registration witnesses.',callback)]),
 'config':dict(title='Where does test selection change?', subtitle='Two unchanged configurations inherit a changed shared skip list.',
   caveat='Declared exclusions only. This experiment did not execute the DuckDB test suite.',
   nodes=[node('disable','disable_optimizer.json','Inherits shared list','Unchanged',20,38,inherit1),
          node('serializer','verify_serializer.json','Inherits shared list','Unchanged',20,171,inherit2),
          node('shared','stats_dependent_tests.json','Shared skip list','Modified',278,102,skip),
          node('test','distinct_on_stats_propagation.test','New statistics regression','Added',536,102,skip)],
   edges=[edge('disable','shared','Inherits',inherit1,'config'),edge('serializer','shared','Inherits',inherit2,'config'),edge('shared','test','Excludes',skip,'config')],
   rows=[row('disable_optimizer → shared skip list → new test','Unchanged configuration inherits the newly declared exclusion.',inherit1,'config'),
         row('verify_serializer → shared skip list → new test','A second unchanged configuration inherits the exclusion.',inherit2,'config'),
         row('verify_aggregate_state_export → new test','The modified execution configuration directly excludes the test.',direct,'config')])
}
files=[dict(id='stats',name='relation_statistics_helper.cpp',path=STATS,status='Modified',
            symbols=['ExtractAggregationStatsInternal','HasUniqueGroups','HasBaseColumnSource','GetValuePreservingColumn']),
       dict(id='rewrite',name='first_last_any.cpp',path=REWRITE,status='Modified',symbols=['RewriteOrderedFirst']),
       dict(id='shared',name='stats_dependent_tests.json',path='test/configs/stats_dependent_tests.json',status='Modified',symbols=[]),
       dict(id='export',name='verify_aggregate_state_export.json',path='test/configs/verify_aggregate_state_export.json',status='Modified',symbols=[]),
       dict(id='test',name='distinct_on_stats_propagation.test',path=TEST,status='Added',symbols=[])]
components=[]
for target,caption in [('duckdb_aggr_distr','Aggregate functions'),('duckdb_optimizer_relation_statistics','Relation statistics'),('duckdb_optimizer_join_order','Join ordering'),('duckdb_optimizer','Optimizer'),('duckdb_operator_set','Set execution'),('test_optimizer','Optimizer tests')]:
    r=next(x for x in relations if x.get('component')==target)
    ev=rel(lambda x:x is r)
    members=[x['source_path'] for x in relations if x.get('component')==target]
    components.append(dict(target=target,caption=caption,members=members,evidence=ev))
data=dict(head=HEAD,base='629e395a854d72dcb1f414e1a0451c68ac0e9f1f',files=files,contexts=contexts,evidence=evidence,
          components=components,people=json.loads((POC/'stakeholder-candidates.json').read_text()),
          input_refs=['2026-11-18-pr-repository-context-map/relationships.json','2026-11-18-pr-repository-context-map/native-incoming-paths.json','2026-11-17-review-extensions-poc/stakeholder-candidates.json'])
for f in files:
    f['diff']=subprocess.check_output(['git','-C','/Users/kevintruong/project/duckdb','diff','--no-ext-diff', '--no-color',data['base'],HEAD,'--',f['path']],text=True)
(HERE/'design-data.json').write_text(json.dumps(data,indent=2)+'\n')
print(json.dumps({'files':len(files),'contexts':len(contexts),'evidence':len(evidence),'components':len(components)}))
