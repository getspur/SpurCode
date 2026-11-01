# R5 source collector

Actual issue: bd-280. Dependencies R2C/R3/R4 are approved. Preserve graph schema v11 and the R4 structural API.

Use tree-sitter over exact supplied source bytes; recollect R3 identities first. Lower Option<u8> into separately versioned bool presence/u8 payload bindings. R4 scalar versions, assignments, phi inputs and guarded flow remain authoritative. Extend derivations/site variants for typed primitive results and direct helper bindings. Retain returns, implicit arithmetic/index operations and literal loop templates in a versioned optional extraction extension. Existing authored R4 facts remain structurally checkable; automatic acceptance requires fresh canonical recollection and expected-build pinning.

Branches merge reaching versions, lexical blocks restore shadows, early returns remove continuation edges, and short-circuit RHS executes only on its own branch. Opaque effects invalidate live values; unsupported syntax never supplies a safe conclusion. Helper resolution is limited to unambiguous same-file top-level non-generic free functions with bool/u8 parameters/results. Loops remain templates with an explicit exploration obligation for R6. R3 rejects cfg/parse uncertainty before body identity exists; report this as a typed collection obligation error rather than fabricate an R3 owner.

PRE discovers no family for Rust guard/value equations. Six exact typed requests were preflighted, persisted and reloaded; exact ownership tuples additionally execute data_integrity.mutually_consistent. POST must project newly collected facts and exact spans, then compare independent finite and ordinary Rust oracles. No fixture/function name selects collector behavior.
