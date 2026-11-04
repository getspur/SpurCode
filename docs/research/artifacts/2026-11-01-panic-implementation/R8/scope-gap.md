# R8 assertion acceptance gap

Assigned issue bd-gpd; baseline 1427759de0a3caa00104e21b2dbc17230d75e82d.

The shipped source collector in crates/spur-graph/src/panic_evidence/semantics.rs treats macro_invocation as unsupported (lines 1222 onward), and the primitive authority scan excludes all sources containing macros (lines 95 onward). Thus an actual source->canonical facts->solver assertion witness cannot be obtained for assert!. R8 cannot honestly clear that obligation or manufacture facts. Closing explicit assertion-macro replay requires root authorization to expand scope into collector code and collector regressions; potentially source authority and macro shadowing rules need design review. R8 will retain assert!/shadowed assert! as unknown and implement other commands and real bounds/overflow replay within the assigned files. This is an unresolved acceptance obligation, not a safety result.

Fresh orientation failed (analyst query prepare failure) and exact code_symbol_search failed (graph artifact absent); bounded raw fallback restricted to panic evidence example, API, tests, and solver receipt API. No CURRENT/publication operation was attempted.
