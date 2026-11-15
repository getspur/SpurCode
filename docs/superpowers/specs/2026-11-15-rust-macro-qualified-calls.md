# Rust macro call qualification

User-approved repair for bd-38yz, rooted in `crates/spur-graph/src/extract`.

The notebook's compiler-backed fixture records `Vec::new()` inside `assert_eq!` as a call to an unrelated `LocalFactory::new()`. The Rust token-tree query captures a callee's leaf name, while the `macro_call` emission branch in `languages.rs` supplies neither receiver nor scope evidence. Bare-name resolution can consequently invent a local target. The DuckDB function shares this extractor; filtering its SQL output would leave the native defect intact.

Preserve explicit qualification for each Rust macro-body callee and feed it through the existing native qualified-call resolution rules. An unknown or external qualified call must remain unresolved rather than acquire an unrelated local target. A genuine local `LocalFactory::new()` and an ordinary helper call inside a macro must keep working. Scope evidence must belong to its own callee even when a macro contains several calls. Keep macro-derived confidence/evidence conventions and all five fact schemas compatible.

Required regression coverage includes the recorded compiler fixture, valid local constructor/helper controls, multiple qualifiers in one macro, representative turbofish/qualified paths and nested token trees. Use actual parser/public extraction paths and inspect target ownership; tests must distinguish two methods with the same leaf name. Avoid name blacklists, hard-coded Vec exceptions, arbitrary traversal caps, or a blanket removal of macro calls.

The production repair stays in `spur-graph` source/query files with native tests. Do not change the DuckDB adapter, compiler semantics, macro expansion scope, graph identity, historical database or unrelated user edits. Rebuild a separately named DuckDB 2.0 extension artifact afterward and validate the public SQL seam through notebook MCP. Preserve the original extension and original snapshot for before/after comparison.

Acceptance: a committed regression fails on the original wrong binding before production edits; the root fix makes it pass; valid controls and related native tests pass; independent review approves the source diff; a newly built extension preserves the compiler-backed helper control and removes the spurious constructor target in actual SQL. Any corpus comparison uses the same frozen input and reports changed bindings without treating a lower binding count as worse accuracy.
