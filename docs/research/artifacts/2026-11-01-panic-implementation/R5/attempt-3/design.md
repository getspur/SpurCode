# R5 attempt 3: source dispatch authority

Issue bd-280 is in_progress. This rework preserves the approved physical-source bool/u8 subset and all previous evidence; unsupported dispatch remains unknown.

Root helper metadata is authoritative only for a caller function whose AST parent is source_file and whose unqualified target has no intervening lexical binding. Reuse that exact predicate for direct calls and contextual argument type inference. Inline/nested modules and nested functions retain a Callee obligation at the affected invocation. Implementing module resolution is outside this narrow repair.

Option::is_some requires autoref and can lose to a visible trait method taking self by value. Without compiler method-selection evidence, conservatively refuse this shortcut in a source containing trait declarations/trait implementations. Imports/macros already disable source primitive/prelude authority. Keep the obligation on the invocation in the analyzed caller. By-value inherent unwrap/checked_add retain their existing supported handling.

Fresh catalog discovery selects data_integrity.mutually_consistent for finite ownership/refusal tuples. No catalog rule models Rust guard/value evaluation; use typed generic equations with exact preflight for that lane. Six fresh PRE requests, expected SAT/UNSAT, were persisted and reloaded before new tests or production changes (pre.json).

Runtime RED covers the five exact review programs, module and contextual-type variants, default/blanket/raw/local/nested traits, and unambiguous positives. GREEN exports are inputs to independent finite/Rust checks and POST equations; generated facts are never collector inputs. Unknowns are an unresolved outcome, never a safety prediction. R6/R7/R8 retain their responsibilities.

Source retrieval: both orientation and exact graph reads failed (discovery.json). Bounded raw fallback is limited to the four R5 scoped files and prior/review evidence. No notebook or graph publication operations.
