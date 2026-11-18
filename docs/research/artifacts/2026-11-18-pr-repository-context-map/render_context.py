"""Render two small evidence maps; no edge is presented as a runtime trace."""
from html import escape
import json
from pathlib import Path

ART = Path(__file__).parent
relationships = json.loads((ART / 'relationships.json').read_text())


def render(name, height, title, nodes, edges, note):
    elements = [f'<svg xmlns="http://www.w3.org/2000/svg" width="1180" height="{height}" style="max-width:100%;height:auto" viewBox="0 0 1180 {height}" role="img" aria-label="{escape(title)}">',
        '<defs><marker id="arrow" markerWidth="8" markerHeight="8" refX="7" refY="3" orient="auto"><path d="M0,0 L0,6 L8,3 z" fill="#64748b"/></marker></defs>',
        f'<rect width="1180" height="{height}" fill="#f8fafc"/>',
        f'<text x="28" y="32" font-family="system-ui,sans-serif" font-size="21" fill="#0f172a">{escape(title)}</text>']
    positions = {node['id']: node for node in nodes}
    for source, target, label, candidate in edges:
        a, b = positions[source], positions[target]
        x1, y1 = a['x'] + 160, a['y'] + 44
        x2, y2 = b['x'] + 160, b['y'] + 44
        if abs(x2-x1) > abs(y2-y1):
            x1 += 160 if x2 > x1 else -160
            x2 += -160 if x2 > x1 else 160
        else:
            y1 += 44 if y2 > y1 else -44
            y2 += -44 if y2 > y1 else 44
        dash = ' stroke-dasharray="7 5"' if candidate else ''
        elements.append(f'<path d="M{x1},{y1} L{x2},{y2}" fill="none" stroke="#64748b" stroke-width="1.7"{dash} marker-end="url(#arrow)"/>')
        mx, my = (x1+x2)/2, (y1+y2)/2
        short = {'qualified reference':'API ref', 'qualified references':'API refs',
                 'native syntax calls':'calls', 'possible callback dispatch':'callback',
                 'propagates metadata':'metadata', 'shared-contract reader':'reader',
                 'shared-contract producer':'writer', 'inherits_skip_tests':'inherits',
                 'declared skip':'skip', 'lists configuration':'lists'}.get(label,label)
        elements.append(f'<g><title>{escape(label)}</title><text x="{mx}" y="{my-7}" text-anchor="middle" font-family="system-ui,sans-serif" font-size="12" fill="#334155" paint-order="stroke" stroke="#f8fafc" stroke-width="5">{escape(short)}</text></g>')
    for node in nodes:
        color = '#fff7df' if node.get('changed') else '#ffffff'
        border = '#d7a432' if node.get('changed') else '#94a3b8'
        x, y = node['x'], node['y']
        elements.append(f'<g><title>{escape(node.get("witness", ""))}</title><rect x="{x}" y="{y}" width="320" height="88" rx="10" fill="{color}" stroke="{border}"/>')
        for index, line in enumerate(node['lines']):
            size = 14 if index == 0 else 12
            elements.append(f'<text x="{x+12}" y="{y+25+index*20}" font-family="system-ui,sans-serif" font-size="{size}" fill="#0f172a">{escape(line)}</text>')
        elements.append('</g>')
    elements.append(f'<text x="28" y="{height-24}" font-family="system-ui,sans-serif" font-size="12" fill="#475569">{escape(note)}</text></svg>')
    (ART / name).write_text('\n'.join(elements))


def main():
    assert any(r['relation'] == 'dispatches_possible_registered_rewrite' for r in relationships)
    assert any(r['source_path'].endswith('physical_recursive_cte_keyed.cpp') and r['relation'] == 'reads_shared_identity_contract' for r in relationships)
    assert sum(r['relation'] == 'references_aggregate_stats_api' for r in relationships) == 10
    render('code-context.svg', 830, 'PR changes in their unchanged code context', [
        {'id':'join','x':28,'y':75,'lines':['Join-order client (unchanged)','join_order/relation_manager.cpp','References at lines 623 and 856']},
        {'id':'extractor','x':28,'y':205,'lines':['Statistics extractor (unchanged)','relation_statistics_extractor.cpp','Qualified API reference at line 124']},
        {'id':'tests','x':28,'y':335,'lines':['Existing unit tests (unchanged)','test/optimizer/relation_statistics.cpp','Seven qualified API references']},
        {'id':'wrapper','x':430,'y':205,'lines':['Public forwarders (bodies unchanged)','ExtractAggregationStats overloads','Changed file, unchanged wrapper bodies']},
        {'id':'stats','x':832,'y':205,'changed':True,'lines':['CHANGED statistics implementation','ExtractAggregationStatsInternal','and 3 added helper functions']},
        {'id':'dispatch','x':28,'y':535,'lines':['Optimizer dispatcher (unchanged)','optimizer/aggregate_rewrite.cpp:58','Invokes selected rewrite callback']},
        {'id':'rewrite','x':430,'y':535,'changed':True,'lines':['CHANGED aggregate rewrite','first_last_any.cpp:19–59','RewriteOrderedFirst']},
        {'id':'contract','x':832,'y':535,'lines':['Shared contract (unchanged header)','AggregateFunction.single_value_identity','aggregate_function.hpp:353,401–402']},
        {'id':'cte','x':832,'y':705,'lines':['Another reader (unchanged)','physical_recursive_cte_keyed.cpp:42','Review candidate; runtime impact unproven']},
        {'id':'minmax','x':430,'y':705,'lines':['Other producers (unchanged)','minmax.cpp:380,404','Also set the identity property']},
    ], [('join','wrapper','qualified reference',True),('extractor','wrapper','qualified reference',True),
        ('tests','wrapper','qualified references',True),('wrapper','stats','native syntax calls',False),
        ('dispatch','rewrite','possible callback dispatch',True),('rewrite','contract','propagates metadata',True),
        ('stats','contract','reads contract',True),('contract','cte','shared-contract reader',True),
        ('minmax','contract','shared-contract producer',True)],
        'Amber = changed symbols. Dashed = candidate / source-contract context; solid = retained bound syntax. No runtime-coverage claim.')
    render('configuration-context.svg', 580, 'A changed skip list affects unchanged configurations', [
        {'id':'disable','x':28,'y':80,'lines':['Unchanged configuration','disable_optimizer.json','Inherits skip tests from shared list']},
        {'id':'serializer','x':28,'y':220,'lines':['Unchanged configuration','verify_serializer.json','Inherits skip tests from shared list']},
        {'id':'statsconfig','x':430,'y':150,'changed':True,'lines':['CHANGED shared skip list','stats_dependent_tests.json','Adds the new regression path']},
        {'id':'newtest','x':832,'y':150,'changed':True,'lines':['ADDED SQL regression','distinct_on_stats_propagation.test','[statistics] grouping / identity tests']},
        {'id':'make','x':28,'y':410,'lines':['Unchanged test matrix declaration','Makefile:580','Lists aggregate-state-export config']},
        {'id':'verify','x':430,'y':410,'changed':True,'lines':['CHANGED mode skip list','verify_aggregate_state_export.json','Adds the new regression path']},
    ], [('disable','statsconfig','inherits_skip_tests',False),('serializer','statsconfig','inherits_skip_tests',False),
        ('statsconfig','newtest','declared skip',False),('verify','newtest','declared skip',False),
        ('make','verify','lists configuration',False)],
        'These are parsed configuration/build declarations. Two unchanged modes inherit the shared list; aggregate-state-export also skips the new test. No test execution claim.')
    print('Rendered two source-backed context diagrams')


if __name__ == '__main__':
    main()
