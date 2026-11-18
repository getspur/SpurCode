"""Read-only prototype checks and browser interaction receipt, not production app tests."""
import hashlib
import json
import re
from pathlib import Path
from playwright.sync_api import sync_playwright

HERE=Path(__file__).resolve().parent
html=(HERE/'prototype.html').read_text()
data=json.loads((HERE/'design-data.json').read_text())
results=[]
def check(name, condition):
    assert condition, name
    results.append(name)

check('No external script, stylesheet, font or resource URLs',not re.search(r'<script[^>]*src=|<link[^>]*href=|url\(\s*[\'\"]?https?://',html,re.I))
check('No prohibited dash glyphs or entities',not re.search('[\u2013\u2014]|&[mn]dash;|&#821[12];',html))
check('Five files and four context questions',len(data['files'])==5 and len(data['contexts'])==4)
for c in data['contexts'].values():
    check('Evidence exists: '+c['title'],all(x['evidence'] in data['evidence'] for x in c['nodes']+c['edges']+c['rows']))
for e in data['evidence'].values():
    check('Pinned source witness: '+e['id'],bool(e['lines']) and e['revision']==data['head'])
native=json.loads((HERE.parent/'2026-11-18-pr-repository-context-map/native-incoming-paths.json').read_text())
for e in data['evidence'].values():
    if e['id'].startswith('native:'):
        path=native[e['artifact_index']]['nodes']
        check('Native receipt matches displayed leg: '+e['id'],len(path)==2 and
              (path[0]['name']=='ExtractAggregationStatsInternal' if e['id']!='native:wrapper' else path[1]['name']=='ExtractAggregationStatsInternal'))

with sync_playwright() as pw:
    browser=pw.chromium.launch(executable_path='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless=True)
    context=browser.new_context(viewport={'width':1500,'height':1100})
    page=context.new_page();errors=[];requests=[]
    page.on('pageerror',lambda e:errors.append(str(e)))
    page.on('request',lambda r: requests.append(r.url))
    page.goto((HERE/'prototype.html').as_uri())
    page.wait_for_function("document.documentElement.dataset.interactive==='ready'")
    check('Client island hydrates',page.locator('h1').count()==1)
    check('Unchanged wrapper is visible',page.get_by_text('Unchanged body',exact=True).count()==1)
    page.screenshot(path=str(HERE/'01-incoming-context.png'),full_page=True)
    page.get_by_role('button',name='Join-order client → public wrapper → changed internal',exact=False).click()
    check('Candidate path opens exact unchanged client source', 'relation_manager.cpp' in page.locator('.inspector').inner_text() and 'Reference candidate' in page.locator('.inspector').inner_text())
    page.get_by_label('Include candidates').uncheck()
    check('Candidate filter explains hidden paths', 'Candidate relationships hidden' in page.locator('.paths').inner_text())
    check('Candidate filter removes external candidate nodes',page.locator('.graph-node').count()==2)
    page.get_by_label('Include candidates').check()
    page.get_by_role('button',name='Path list',exact=True).click()
    check('Path list works without map',page.locator('svg').count()==0 and page.locator('.path').count()==3)
    page.get_by_role('button',name='HasUniqueGroups',exact=True).click()
    check('Symbol selection persists in heading',page.locator('.selection-heading h2').inner_text()=='HasUniqueGroups')
    page.get_by_role('button',name='Architecture',exact=True).click()
    check('CMake architecture and missing policy are explicit','Alignment not evaluated' in page.locator('.main-view').inner_text() and 'duckdb_aggr_distr' in page.locator('.main-view').inner_text())
    page.get_by_role('button',name='Inspect CMake',exact=True).first.click()
    check('Component witness opens CMake source','CMakeLists.txt' in page.locator('.inspector').inner_text())
    page.screenshot(path=str(HERE/'02-architecture.png'),full_page=True)
    page.get_by_role('button',name='People',exact=True).click()
    check('People keep unknown assignment separate from history','Unknown in captured response' in page.locator('.main-view').inner_text() and page.get_by_role('heading',name='lnkuiper',exact=True).count()==1)
    page.screenshot(path=str(HERE/'03-people.png'),full_page=True)
    page.get_by_role('button',name='Changes + context',exact=True).click()
    check('Selection survives lenses',page.locator('.selection-heading h2').inner_text()=='HasUniqueGroups')
    page.get_by_role('button',name='first_last_any.cpp Modified',exact=True).click()
    check('Rewrite chooses shared-contract neighborhood','Which shared contracts deserve review?' in page.locator('.main-view').inner_text())
    page.get_by_role('button',name='Unchanged recursive-CTE consumer → same contract',exact=False).click()
    check('Shared contract retains runtime caveat','not a proven runtime flow' in page.locator('.inspector').inner_text())
    page.screenshot(path=str(HERE/'04-shared-contract.png'),full_page=True)
    page.get_by_role('button',name='Unchanged dispatcher → possible rewrite callback',exact=False).click()
    page.get_by_text('4 callback registrations',exact=True).click()
    check('Callback detail exposes all registrations',page.locator('.registration').count()==4)
    page.get_by_role('button',name='stats_dependent_tests.json Modified',exact=True).click()
    check('Configuration view distinguishes declared from executed','Declared exclusions only' in page.locator('.scope-note').inner_text())
    page.get_by_role('button',name='disable_optimizer → shared skip list → new test',exact=False).click()
    check('Inherited config opens unchanged witness','disable_optimizer.json' in page.locator('.inspector').inner_text())
    page.screenshot(path=str(HERE/'05-config-context.png'),full_page=True)
    page.get_by_role('button',name='Source diff',exact=True).click()
    check('Frozen source diff is real and visible','distinct_on_stats_propagation.test' in page.locator('.diff-view').inner_text())
    page.get_by_role('button',name='AI review',exact=True).click()
    page.get_by_role('button',name='Preview evidence packet',exact=True).click()
    check('Packet preview does not claim AI execution','No AI job has run' in page.locator('.packet').inner_text())
    page.get_by_role('button',name='Changed rewrite → shared identity contract',exact=False).click()
    check('AI packet citation opens exact source','first_last_any.cpp' in page.locator('.inspector').inner_text())
    page.screenshot(path=str(HERE/'06-review-packet.png'),full_page=True)
    page.get_by_role('button',name='Repository',exact=True).click()
    check('Repository setup keeps saved connection reference',page.locator('input').nth(2).input_value()=='@github-pat')
    page.screenshot(path=str(HERE/'07-repository-setup.png'),full_page=True)
    page.get_by_role('button',name='Open captured PR',exact=True).click()
    page.get_by_role('button',name='Inbox',exact=True).click()
    page.get_by_role('button',name='#28731 Preserve aggregate domains',exact=False).click()
    check('Inbox opens captured review',page.locator('h1').count()==1)
    page.get_by_role('button',name='Changes + context',exact=True).click()
    page.get_by_role('button',name='relation_statistics_helper.cpp Modified',exact=True).click()
    page.get_by_role('button',name='Path list',exact=True).focus()
    page.keyboard.press('Enter')
    check('Keyboard activates path-list control',page.get_by_role('button',name='Path list',exact=True).get_attribute('aria-pressed')=='true')
    page.set_viewport_size({'width':390,'height':844})
    check('Narrow layout has no document horizontal overflow',page.evaluate('document.documentElement.scrollWidth <= innerWidth'))
    page.screenshot(path=str(HERE/'08-narrow.png'),full_page=True)
    page.set_viewport_size({'width':768,'height':1024})
    check('Intermediate layout has no document horizontal overflow',page.evaluate('document.documentElement.scrollWidth <= innerWidth'))
    page.set_viewport_size({'width':1100,'height':900})
    page.get_by_role('button',name='Architecture',exact=True).click()
    check('Intermediate architecture fits document width',page.evaluate('document.documentElement.scrollWidth <= innerWidth'))
    check('No client JavaScript errors',not errors)
    check('Rendering does not request external resources',not any(u.startswith(('https://','http://')) for u in requests))
    static=browser.new_context(java_script_enabled=False,viewport={'width':1500,'height':1100})
    still=static.new_page();still.goto((HERE/'prototype.html').as_uri())
    check('Scripts-off baseline contains map, paths and source',still.locator('.graph-node').count()==4 and still.locator('.path').count()==3 and 'Wrapper calls' in still.locator('.inspector').inner_text())
    still.screenshot(path=str(HERE/'09-scripts-off.png'),full_page=True)
    browser.close()

receipt={'status':'passed','checks':results,'check_count':len(results),'browser':'Isolated local headless Google Chrome via Playwright',
 'prototype_sha256':hashlib.sha256(html.encode()).hexdigest(),
 'limits':['Captured-data design prototype, not native app/DAG validation.','Prototype offers a curated PR neighborhood; controls do not perform live repository queries.','No live GitHub, current people refresh, AI generation or publication.','Keyboard activation and layout probes are checks, not a complete accessibility audit.']}
(HERE/'verification.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({'status':receipt['status'],'checks':len(results),'sha256':receipt['prototype_sha256']}))
