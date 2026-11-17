-- 1. Runtime metadata: no arguments.
SELECT spur_graph_info();

-- 2. Symbols: LIST<Symbol>.
SELECT spur_graph_symbols('src/a.rs', 'fn a() {}');

-- 3. Full single-file facts: Facts STRUCT with five typed lists.
SELECT spur_graph_file_facts('src/a.rs', 'fn a() {}');

-- 4. Diagnostic parent: Result STRUCT, including recoverable bad input.
SELECT spur_graph_file_facts_result('bad.unknown', 'text');

-- 5. Shared native pair matcher: changes plus ambiguity diagnostics.
SELECT spur_graph_symbol_changes('src/a.rs', 'src/a.rs', 'M',
    spur_graph_symbols('src/a.rs', 'fn a() { let x = 1; }'),
    spur_graph_symbols('src/a.rs', 'fn a() { let x = 2; }'));

-- 6/7. Pass original JSON, including cell IDs and metadata.
SELECT spur_graph_notebook_symbols('n.ipynb',
  '{"metadata":{},"cells":[{"id":"p","cell_type":"code","metadata":{"spur":{"code_type":"python"}},"source":"def f():\n    return 1\n"}]}');
SELECT spur_graph_notebook_changes('n.ipynb', NULL, 'A', NULL,
  '{"metadata":{},"cells":[]}');
