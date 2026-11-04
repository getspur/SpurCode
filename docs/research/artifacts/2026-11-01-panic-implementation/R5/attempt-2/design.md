# R5 attempt 2: source authority repair

Assigned issue `bd-280` was verified `in_progress`. Attempt-1 source and evidence were restored unchanged in `06987cb00`; authored history remains at `spur/evidence/r5-attempt-1-authored` (`0c27271bfb43a58d5fd2b42c7a0c2db5ccd62389`). No historical receipt is substituted for fresh PRE.

The review identifies two source-to-fact errors: spelling-based primitive authority and incomplete lexical value namespace shadowing. Small numeric values alone do not imply Rust's `u8` type. A source return/declaration, a proven operand/binding, an actual literal suffix or an actual generic type argument must supply authority. Unexpanded items/macros and primitive namespace shadows prevent certification. Default arrays and ranges require their own authority; LoopBound alone addresses exploration, not typing.

Normalize raw identifiers at every binding/reference boundary. Prehoist local items before evaluating the block, retaining an obligation instead of selecting an unrelated outer helper. Parse token structure excluding comments for primitive/generic types and constructor paths. Preserve schema v11, sidecar IR and public APIs.

TDD sequence: fresh catalog and PRE receipts -> compile and run new runtime regressions -> commit RED -> scoped production fix -> affected GREEN -> fresh collector exports -> independent finite and ordinary Rust oracles -> source/span-bound POST generic equations and catalog ownership checks -> commit evidence. Unknowns remain unresolved, never safe outcomes.

Orientation and exact symbol tools both failed (analyst DB prepare error; graph artifact absent). Bounded raw fallback is restricted to R5 source/tests, local evidence, and required build wrapper context. No graph publication changes or notebook access.

First local RED invocation was interrupted (exit 130) during dependency compilation to reuse `/private/tmp/r1-rework2-target-9ac34ff5`; it is not RED evidence. All Cargo invocations use `SPUR_REMOTE=0 scripts/spur-cargo`.
