"""Embed the fixture and shared components in one portable notebook design cell."""
import json
from pathlib import Path
here=Path(__file__).resolve().parent
client="""
import { h, hydrate } from 'https://esm.sh/preact@10.24.3';
import { useState } from 'https://esm.sh/preact@10.24.3/hooks';
import { View, initial } from './view.ts';
const d=__DATA__;
function App(){
 const [s,set]=useState(initial());
 const act=(type,value)=>set(old=>{
  let next={...old,notice:''};
  if(type==='file'){
   next.file=value;
   next.symbol=d.files.find(f=>f.id===value).symbols[0]||'';
   next.question=value==='stats'?'incoming':value==='rewrite'?'contract':'config';
   next.evidence=d.contexts[next.question].rows[0].evidence;
   next.lens='Changes + context'; next.mode='map';
  }else if(type==='symbol'){next.symbol=value;next.question=old.file==='stats'?'dependencies':'contract';next.evidence=value==='HasBaseColumnSource'?'native:HasBaseColumnSource':value==='GetValuePreservingColumn'?'native:GetValuePreservingColumn':d.contexts[next.question].rows[0].evidence;next.notice='Selected '+value;
  }else if(type==='coverage'){
   next.notice='Coverage: 18,342 tracked head entries, 17,289 scanned UTF-8 blobs, 1,053 binary metadata entries. The selected context map and C++ linking are partial.';
  }else if(type==='copy'){
   const ref='duckdb/duckdb#28731@'+d.head+':'+value;
   if(navigator.clipboard?.writeText) navigator.clipboard.writeText(ref).catch(()=>{});
   next.notice='Reference: '+ref;
  }else {next[type]=value;if(type==='question')next.evidence=d.contexts[value].rows[0].evidence;}
  return next;
 });
 return h(View,{s,d,act});
}
hydrate(h(App),document.getElementById('app'));
document.documentElement.dataset.interactive='ready';
"""
cell="""// Design artifact only: captured evidence, no live app providers or AI calls.
import { h } from 'https://esm.sh/preact@10.24.3';
import { render as ssr } from 'https://esm.sh/preact-render-to-string@6.5.11';
import * as esbuild from 'npm:esbuild@0.24.0';
const viewSource=__VIEW__;
const css=__CSS__;
const data=__DATA__;
const clientSource=__CLIENT__;
const buildDir=await Deno.makeTempDir({prefix:'spur-review-design-'});
await Deno.writeTextFile(buildDir+'/view.ts',viewSource);
await Deno.writeTextFile(buildDir+'/client.ts',clientSource);
const view=await import('file://'+buildDir+'/view.ts');
const baseline=ssr(h(view.View,{s:view.initial(),d:data}));
const httpPlugin={name:'http',setup(b){
 b.onResolve({filter:/^https?:\\/\\//},a=>({path:a.path,namespace:'http-url'}));
 b.onResolve({filter:/.*/,namespace:'http-url'},a=>({path:new URL(a.path,a.importer).toString(),namespace:'http-url'}));
 b.onLoad({filter:/.*/,namespace:'http-url'},async a=>{const r=await fetch(a.path);if(!r.ok)throw Error('Build dependency '+r.status);return {contents:await r.text(),loader:'js'};});
}};
const bundle=await esbuild.build({plugins:[httpPlugin],entryPoints:[buildDir+'/client.ts'],bundle:true,minify:true,format:'iife',write:false,legalComments:'none',supported:{'template-literal':false}});
await esbuild.stop();
const doc='<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Spur Graph Review | interactive design</title><style>'+css+'</style></head><body><div id="app">'+baseline+'</div><script>'+bundle.outputFiles[0].text.replaceAll('</script','<\\/script')+'</script></body></html>';
await Deno.writeTextFile('/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-18-spur-review-ux/prototype.html',doc);
({[Symbol.for('Jupyter.display')](){return {'text/html':doc}}});
"""
data=json.loads((here/'design-data.json').read_text())
client=client.replace('__DATA__',json.dumps(data))
for token,value in [('__VIEW__',(here/'view.ts').read_text()),('__CSS__',(here/'style.css').read_text()),('__DATA__',data),('__CLIENT__',client)]:
    cell=cell.replace(token,json.dumps(value),1)
(here/'design-cell.ts').write_text(cell)
print(json.dumps({'cell_bytes':len(cell.encode()),'fixture_records':len(data['evidence'])}))
