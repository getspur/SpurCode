# U2 attempt 1 root review

Request changes for an Important/P2 no-op regression. Comparing a byte-identical notebook containing two identical anonymous Python cells reports four false cell Deleted/Added events. The input remains Partial with identity diagnostics, and all symbols remain in inventory; the false structural events occur in the ambiguous-group branch of `notebook_symbols.rs:408-426`.

The private root regression compiled and failed as expected: actual cell-event count 4, expected 0, exit 101. `reproducer.patch` is a zero-context test-only patch against the reviewed worker HEAD. The isolated review checkout was restored to its exact committed bytes afterward. Duplicate explicit IDs take the same branch; the independent reviewer recorded this in bd-1n0t.

Other verification passed: all 15 new core tests and all 48 ordinary matcher tests passed independently. Exact frozen counts (46/24 symbols, 17/11 functions) and all 28 anchors are asserted in the passing core suite. Root verified all eight worker receipt log hashes/exits, RED/GREEN command/environment/test identity, current and committed source/fixture hashes, unchanged U1 bytes, public schema, patch, encoding mutation test and five equal-source offset witnesses.

The intended 55-to-50 real-change reduction is supported by identical original symbol slices. Core IDs and before/after SourceRefs have no other important review findings. Preserve this work and its original TDD history while adding no-op regressions and the targeted fix. Keep genuine anonymous source edits conservative and retain independent metadata/configuration events.

`review.json` records exact commands, environment, captured outputs and setup recovery. The initial root attempts failed before tests due to a cached worker path and then exhausted disk space. Only obsolete task-owned R5 build-cache deps were removed; sources, receipts and retained native package 28ddce83 are preserved. Host execution is explicitly authorized infrastructure recovery. SQL, consumer and native release gates remain pending.
