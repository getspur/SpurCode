import { h } from 'https://esm.sh/preact@10.24.3';
export const initial = () => ({screen:'review',lens:'Changes + context',file:'stats',symbol:'ExtractAggregationStatsInternal',question:'incoming',mode:'map',evidence:'native:wrapper',candidates:true,packet:false,notice:''});
const el=(tag,props,...children)=>h(tag,props,...children.flat());
const btn=(text,fn,props={})=>el('button',{type:'button',onClick:fn,...props},text);
const badge=(text,kind='')=>el('span',{class:'badge '+kind},text);
const mono=text=>el('span',{class:'mono'},text);
const label=q=>({incoming:'Incoming clients',dependencies:'Dependencies',contract:'Shared contracts',config:'Tests + config'}[q]);
const quality=q=>({native_bound_syntax:'Syntax-bound',source_reference_candidate:'Reference candidate',callback_dispatch_candidate:'Possible callback',shared_contract_reference:'Shared contract',build_declaration:'Build declaration',config_value:'Declared config'}[q]||q);

function FileList({s,d,act}){
 return el('aside',{class:'files', 'aria-label':'Changed files and symbols'},
  el('div',{class:'pane-title'},el('strong',{},'Changed files'),badge('5')),
  el('p',{class:'small muted'},'Select a change to explore repository context.'),
  el('nav',{},d.files.map(f=>el('div',{class:'file-group'},
   btn(el('span',{},el('span',{class:'file-name'},f.name),el('span',{class:'file-status'},f.status)),()=>act('file',f.id),{class:'file '+(s.file===f.id?'selected':''),'aria-current':s.file===f.id?'true':undefined,title:f.path}),
   s.file===f.id&&f.symbols.map(name=>btn(el('span',{},name),()=>act('symbol',name),{class:'symbol '+(s.symbol===name?'selected':''),title:name,'aria-pressed':s.symbol===name}))
  ))),
  el('div',{class:'file-footer'},el('strong',{},'Context includes unchanged files'),el('p',{class:'small muted'},'Tracked repository catalog. Semantic linking remains partial.'),btn('Inspect coverage',()=>act('coverage'),{class:'text-button'})));
}

function Graph({s,d,act}){
 const c=d.contexts[s.question],filtered=c.edges.filter(e=>s.candidates||e.kind!=='candidate'),ids=new Set(filtered.flatMap(e=>[e.a,e.b]));
 return el('div',{class:'graph-region',role:'region','aria-label':'Context map. Equivalent relationships are listed below.',tabIndex:0},
 el('svg',{viewBox:'0 0 786 296',role:'img','aria-label':c.title},
 el('defs',{},el('marker',{id:'arrow',viewBox:'0 0 8 8',refX:'7',refY:'4',markerWidth:'6',markerHeight:'6',orient:'auto-start-reverse'},el('path',{d:'M0 0 L8 4 L0 8',fill:'#7c8d84'}))),
 filtered.map(e=>{const a=c.nodes.find(n=>n.id===e.a),b=c.nodes.find(n=>n.id===e.b);const reverse=a.x>b.x;const x1=a.x+(reverse?0:210),x2=b.x+(reverse?210:0),y1=a.y+43,y2=b.y+43;return el('g',{},
  el('path',{d:`M${x1} ${y1} C${(x1+x2)/2} ${y1},${(x1+x2)/2} ${y2},${x2} ${y2}`,class:'edge '+e.kind,'marker-end':'url(#arrow)'}),
  el('text',{x:(x1+x2)/2,y:(y1+y2)/2-8,'text-anchor':'middle',class:'edge-label'},e.label==='API reference'?'API ref.':e.label));}),
 c.nodes.filter(n=>ids.has(n.id)).map(n=>el('g',{class:'graph-node',onClick:()=>act('evidence',n.evidence)},
  el('rect',{x:n.x,y:n.y,width:210,height:86,rx:5,class:n.title===s.symbol?'selected-node':n.state==='Modified'||n.state==='Added'?'changed-node':'neutral-node'}),
  el('foreignObject',{x:n.x+10,y:n.y+10,width:190,height:68},el('div',{xmlns:'http://www.w3.org/1999/xhtml'},el('div',{class:'node-title'},n.title),el('div',{class:'node-sub'},n.sub),el('div',{class:'node-state'},n.state)))))));
}

function Paths({s,d,act}){
 const c=d.contexts[s.question],rows=c.rows.filter(r=>s.candidates||r.kind!=='candidate');
 return el('div',{class:'paths'},el('div',{class:'section-line'},el('strong',{},'Review paths'),el('span',{class:'small muted'},'Select a path to inspect its witness')),
 rows.length? rows.map(r=>btn(el('span',{},el('span',{class:'path-title'},r.title),el('span',{class:'path-body'},r.body),badge(r.kind==='candidate'?'Candidate':r.kind==='syntax'?'Syntax-bound':r.kind==='contract'?'Shared contract':'Declared config',r.kind)),()=>act('evidence',r.evidence),{class:'path '+(s.evidence===r.evidence?'active':''),'aria-pressed':s.evidence===r.evidence})):el('p',{},'No paths in this filtered selection. Candidate relationships are hidden.'),
 !s.candidates&&el('p',{class:'small muted'},'Candidate relationships hidden. This does not imply no impact.'));
}

function Changes({s,d,act}){
 const f=d.files.find(f=>f.id===s.file),c=d.contexts[s.question];
 return el('section',{class:'main-view'},
  el('div',{class:'selection-heading'},el('div',{},el('div',{class:'eyebrow'},'SELECTED CHANGE'),el('h2',{},s.symbol||f.name),el('div',{class:'small mono path-wrap'},f.path)),badge(f.status,'change')),
  el('div',{class:'view-switch',role:'group','aria-label':'Context presentation'},['map','paths','diff'].map(m=>btn({map:'Context map',paths:'Path list',diff:'Source diff'}[m],()=>act('mode',m),{class:s.mode===m?'chosen':'','aria-pressed':s.mode===m}))),
  s.mode==='diff'?el('div',{class:'diff-view'},el('div',{class:'section-line'},el('strong',{},'Frozen Git diff'),mono('629e395 → d29b26b')),el('pre',{},f.diff.split('\n').map(line=>el('span',{class:line.startsWith('+')?'addition':line.startsWith('-')?'deletion':line.startsWith('@@')?'hunk':''},line+'\n')))):
  el('div',{},el('div',{class:'question-tabs',role:'group','aria-label':'Context question'},['incoming','dependencies','contract','config'].map(q=>btn(label(q),()=>act('question',q),{'aria-pressed':s.question===q,class:s.question===q?'chosen':''}))),
   el('div',{class:'context-heading'},el('h3',{},c.title),el('p',{class:'muted'},c.subtitle)),
   el('div',{class:'filter-line'},el('label',{},el('input',{type:'checkbox',checked:s.candidates,onChange:e=>act('candidates',e.currentTarget.checked)}),' Include candidates'),el('span',{class:'small muted'},'Captured PR neighborhood · frozen head')),
   s.mode==='map'&&el(Graph,{s,d,act}),
   el('div',{class:'scope-note'},el('strong',{},'Evidence limit. '),c.caveat),el(Paths,{s,d,act})));
}

function Architecture({s,d,act}){
 return el('section',{class:'main-view'},el('div',{class:'eyebrow'},'SAME PR SNAPSHOT'),el('h2',{},'Where the change sits'),el('p',{class:'muted'},'Build-declared components and the contracts between them.'),
 el('div',{class:'scope-note'},el('strong',{},'Alignment not evaluated. '),'No approved architecture policy is attached to this run.'),
 el('div',{class:'architecture-flow'},el('div',{},'Aggregate functions',mono('duckdb_aggr_distr')),el('span',{},'→'),el('div',{},'Shared identity contract',mono('single_value_identity')),el('span',{},'←'),el('div',{},'Statistics / execution',mono('Readers in unchanged code'))),
 el('p',{class:'small muted'},'Arrows show contract participation, not newly introduced architectural dependencies.'),
 el('table',{class:'component-table'},el('thead',{},el('tr',{},el('th',{},'Component'),el('th',{},'Captured members'),el('th',{},'Evidence'))),el('tbody',{},d.components.map(c=>el('tr',{},el('td',{},el('strong',{},c.caption),el('div',{class:'small mono'},c.target)),el('td',{},c.members.map(p=>el('div',{class:'small path-wrap'},p))),el('td',{},btn('Inspect CMake',()=>act('evidence',c.evidence),{class:'text-button'})))))),
 el('div',{class:'next-step'},el('h3',{},'Review the boundary before judging alignment'),el('p',{},'Confirm component membership, then add a versioned policy for allowed dependencies and exceptions. Existing base dependencies remain distinct from new edges.')));
}

function People({s,d,act}){
 const candidates=d.people.filter(p=>p.github_login);
 return el('section',{class:'main-view'},el('div',{class:'eyebrow'},'PARTICIPATION AND EXPERTISE'),el('h2',{},'Who can add context?'),el('p',{class:'muted'},'Historical knowledge is separate from current review responsibility.'),
 el('div',{class:'people-status'},el('div',{},el('strong',{},'Current reviewers'),el('p',{},'Unknown in captured response')),el('div',{},el('strong',{},'Declared ownership'),el('p',{},'No CODEOWNERS found in captured base/head'))),
 el('h3',{},'Historical expertise candidates'),candidates.map(p=>el('article',{class:'person'},el('div',{class:'person-initial'},p.github_login[0].toUpperCase()),el('div',{},el('h3',{},p.github_login),el('p',{},p.author_name),el('p',{class:'small mono path-wrap'},p.file_path),el('p',{},`${p.base_region_lines} inspected base-region lines; ${p.history_commits} sampled file commits.`),el('p',{class:'small muted'},'Base-pinned history and blame. This is not an assignment, ownership claim or availability signal.'),el('details',{},el('summary',{},'Inspect historical evidence'),el('p',{class:'small path-wrap'},'Identity witness: ',p.evidence_url),el('p',{class:'small'},'Source: stakeholder-candidates.json. Sampling scope is inspectable in the original history/blame receipts.'))))),
 el('div',{class:'next-step'},el('strong',{},'Useful review questions'),el('p',{},'Statistics: does the uniqueness proof preserve total and current domain evidence?'),el('p',{},'Aggregate rewrite: is identity metadata preserved across rewrite and rebinding?')));
}

function AI({s,d,act}){
 return el('section',{class:'main-view'},el('div',{class:'eyebrow'},'EVIDENCE BEFORE GENERATION'),el('h2',{},'AI review'),badge('Not run'),el('p',{class:'muted'},'Prepare one packet from the same changes, context, components and people evidence.'),
 el('table',{class:'readiness'},el('tbody',{},[['Changes','Frozen Git diff and semantic seeds'],['Context','Partial linking; candidates retained'],['Architecture','Build membership ready; policy missing'],['People','Historical evidence; current requests unknown'],['Execution','DuckDB test suite not run'],['AI provider','Not configured in this design prototype']].map(([a,b])=>el('tr',{},el('th',{},a),el('td',{},b))))),
 btn('Preview evidence packet',()=>act('packet',!s.packet),{class:'primary','aria-expanded':s.packet}),
 s.packet&&el('section',{class:'packet',id:'packet'},el('h3',{},'Packet preview'),el('p',{},'Design preview over captured evidence. No AI job has run.'),el('p',{class:'small mono'},'Source: 629e395 → d29b26b'),el('h4',{},'Review questions, not generated defect findings'),
 d.contexts.contract.rows.slice(0,2).map(r=>btn(el('span',{},r.title,el('span',{class:'path-body'},'Inspect supporting source before drawing a conclusion.')),()=>act('evidence',r.evidence),{class:'path'})),
 el('h4',{},'Known limits travel with the packet'),el('p',{},'C++ bindings are incomplete. Contract proximity is not runtime propagation. Architecture has no approved policy. People evidence is historical.'),el('p',{class:'small muted'},'Production findings require citations, counter-evidence, validation steps and a human disposition.')));
}

function Inspector({s,d,act}){
 const e=d.evidence[s.evidence];
 return el('aside',{class:'inspector','aria-label':'Selected evidence'},el('div',{class:'pane-title'},el('strong',{},'Evidence'),badge('Frozen head')),
 el('h3',{},quality(e.evidence_class)),el('p',{class:'relation-name'},e.relation.replaceAll('_',' ')),
 el('div',{class:'evidence-location'},el('div',{class:'small mono path-wrap'},e.witness_path),el('strong',{},'Line '+e.witness_line)),
 el('p',{class:'small muted'},e.location||'Exact source witness'),
 el('pre',{class:'source'},e.lines.map(l=>el('span',{class:l.n===e.witness_line?'current-line':''},el('span',{class:'line-number'},l.n),el('span',{},l.text),'\n'))),
 el('h4',{},'What this establishes'),el('p',{},e.caveat|| (e.evidence_class==='build_declaration'?'Declared build membership. Architecture policy alignment has not been evaluated.':e.evidence_class==='config_value'?'Declared configuration relationship. This is not an executed test result.':'A retained source witness for this relationship.')),
 e.registration_witnesses&&el('details',{},el('summary',{},'4 callback registrations'),e.registration_witnesses.map(w=>el('div',{class:'registration'},mono(w.path+':'+w.line),el('pre',{},w.text)))),
 el('details',{class:'provenance'},el('summary',{},'Provenance and query scope'),el('dl',{},el('dt',{},'Revision'),el('dd',{class:'mono path-wrap'},e.revision),el('dt',{},'Evidence ID'),el('dd',{class:'mono'},e.id),el('dt',{},'Receipt'),el('dd',{class:'mono path-wrap'},e.artifact+' ['+e.artifact_index+']'),e.witness_blob_oid&&[el('dt',{},'Source blob'),el('dd',{class:'mono path-wrap'},e.witness_blob_oid)]),el('p',{class:'small muted'},'Graph identities and source anchors remain scoped to the originating experiment.')),
 btn('Copy reference',()=>act('copy',e.id),{class:'secondary'}),el('p',{class:'small muted'},'Source and path selection share this review run.'));
}

function Setup({s,d,act}){
 return el('main',{class:'setup-page'},el('div',{class:'eyebrow'},'REPOSITORY SETUP · DESIGN PREVIEW'),el('h2',{},'Bring your repository into the review'),el('p',{class:'muted'},'Connect an existing checkout and inspect which evidence is available.'),
 el('label',{class:'field'},'Local repository',el('input',{readOnly:true,value:'/Users/kevintruong/project/duckdb'})),el('label',{class:'field'},'Upstream',el('input',{readOnly:true,value:'duckdb/duckdb'})),el('label',{class:'field'},'Saved GitHub connection',el('input',{readOnly:true,value:'@github-pat'})),
 el('div',{class:'setup-status'},[['Source objects','Captured comparison available'],['Working tree','Review reads pinned commits'],['Runtime','DuckDB 2.0 experiment captured'],['Providers','DuckTails / Spur Graph / DuckPGQ probed'],['Context catalog','18,342 tracked head files'],['Linking','Partial C++ resolution']].map(([a,b])=>el('div',{},el('strong',{},a),el('span',{},b)))),
 el('p',{class:'small muted'},'Fields are a setup preview. This artifact does not save settings, fetch objects or validate a runtime.'),btn('Open captured PR',()=>act('screen','review'),{class:'primary'}));
}
function Inbox({s,d,act}){
 return el('main',{class:'setup-page'},el('div',{class:'eyebrow'},'DUCKDB / CAPTURED INBOX'),el('h2',{},'Pull requests'),el('p',{class:'muted'},'A saved example is available. Live GitHub collection is not wired in this artifact.'),btn(el('span',{},el('strong',{},'#28731 Preserve aggregate domains for proven singleton groups'),el('span',{class:'path-body'},'5 changed files · pinned source snapshot · partial context')),()=>act('screen','review'),{class:'inbox-row'}));
}

export function View({s,d,act=()=>{}}){
 return el('div',{class:'app'},el('header',{class:'topbar'},el('div',{class:'brand'},el('span',{class:'brand-mark'},'S'),el('strong',{},'Spur Graph'),el('span',{class:'muted'},'Review')),el('nav',{'aria-label':'App navigation'},['review','inbox','setup'].map(name=>btn({review:'Review',inbox:'Inbox',setup:'Repository'}[name],()=>act('screen',name),{'aria-current':s.screen===name?'page':undefined,class:s.screen===name?'chosen':''}))),badge('Interactive design')),
 el('div',{class:'prototype-note'},'Captured evidence from PR #28731. Navigation is interactive; collection, AI execution and publication are not connected.'),
 s.screen==='setup'?el(Setup,{s,d,act}):s.screen==='inbox'?el(Inbox,{s,d,act}):el('div',{},
  el('section',{class:'pr-header'},el('div',{},el('div',{class:'eyebrow'},'DUCKDB / PULL REQUEST #28731'),el('h1',{},'Preserve aggregate domains for proven singleton groups'),el('div',{class:'snapshot'},mono('629e395 → d29b26b'),el('span',{},'5 changed files'),el('span',{},'Partial linking'))),btn('Review packet',()=>act('lens','AI review'),{class:'primary'})),
  el('nav',{class:'lens-tabs','aria-label':'Review lens'},['Changes + context','Architecture','People','AI review'].map(l=>btn(l,()=>act('lens',l),{'aria-current':s.lens===l?'page':undefined,class:s.lens===l?'chosen':''}))),
  el('div',{class:'workspace'},el(FileList,{s,d,act}),s.lens==='Architecture'?el(Architecture,{s,d,act}):s.lens==='People'?el(People,{s,d,act}):s.lens==='AI review'?el(AI,{s,d,act}):el(Changes,{s,d,act}),el(Inspector,{s,d,act}))),
 el('footer',{class:'app-footer'},el('span',{},'Selection persists across review lenses.'),el('span',{},'Solid: syntax / declarations · Dashed: candidates · Dotted: shared contracts')),
 el('div',{class:'live',role:'status','aria-live':'polite'},s.notice||''),el('noscript',{},el('div',{class:'scripts-note'},'Static design preview. Enable active content in the notebook to try navigation and evidence selection.')));
}
