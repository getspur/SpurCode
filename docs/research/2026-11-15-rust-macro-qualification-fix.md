# Rust macro call qualification: bd-38yz

Implementation `4d32f19d4` is integrated into `main` with its ordered test-first history. Independent review `bd-1ytb` reports no critical, important or minor findings. The rebuilt extension passes the checks below. Earlier `9a05457df` and `99760f547` candidates remain preserved; the historical directory named `final-99760f547` is superseded.

The native Rust extractor previously discarded the path of calls inside macro token trees. A recorded `Vec::new()` could therefore bind to an unrelated local `new`. The fix carries each callee's own qualifier into the existing resolver, which can keep an unmatched qualified call unresolved. It applies to both native graph extraction and the DuckDB extension because both use the same extractor.

The regression controls require the `Vec` call to remain recorded, the actual local constructor and helper to remain resolved, and qualifiers to stay separate within mixed and nested macro bodies. They also cover leading paths and turbofish arguments. Resolved macro-body calls retain their existing heuristic metadata. No adapter, fact schema or compiler-expansion behavior changes.

The first candidate passed its native checks, but a compiler-valid notebook probe exposed a regression for `Factory /* qualifier */ ::new()` and `Factory // qualifier\n::new()`. A follow-up removed trailing trivia, then independent review exposed internal trivia in paths such as `outer :: local::deep()`. The final implementation normalizes significant syntax tokens around path separators while preserving literal contents and generic keyword boundaries. Both superseded candidates and their evidence remain available.

## Evidence and scope

All receipts are under [the artifact directory](artifacts/2026-11-15-rust-macro-qualification-fix/). `native-initial/commands.json` preserves the initial test-first evidence. `baseline.json` verifies the old extension against all five stored fact collections on identical source inputs. `scope-probe-baseline.json` and `scope-probe-candidate.json` preserve the six-case before/after experiment that found the comment regression.

The frozen project source revision is `61ba3d200439a541dce4c1617ca9277be7d438af`. The 52-file selection exhausts the files containing the original 120 `macro_body_singleton` calls targeting `new`, plus the six prior extraction-contract files. This checks a constructor-review population; it is not a random sample or a full-corpus quality measurement.

The original snapshot and extension are immutable inputs. Their SHA256 hashes, along with the five existing user-edited files, are recorded in `preserved-inputs.json`.

## Source and build identity

The ordered initial test commits are `08fed2401` and `3427630ba`, followed by native fix `9a05457df`. The trailing-comment regressions were committed as `645330c24` before fix `5fa07540e`; the integration lineage preserves these as `ee4c7da17` and `99760f547`. The final normalization test commit is `102ff51ee`, followed by fix `4d32f19d4`. Its first mixed fixture incorrectly expected an ambiguous bare name to resolve; that fixture was corrected to use a singleton helper, and the expected baseline failures were observed again before final GREEN. Only the Rust edge query, native language extractor and new macro regression test change in production/test code.

The corrected host-native artifact is [build-4d32f19d4/spur_graph.duckdb_extension](artifacts/2026-11-15-rust-macro-qualification-fix/build-4d32f19d4/spur_graph.duckdb_extension), built for DuckDB **2.0.0**, `osx_arm64`, `C_STRUCT_UNSTABLE`. Its SHA256 is `be0548ce3a1217cf0b50110ad02419223c895daa8d2a932d7a81747da4c38892`. The sibling `build-command.json` and `build-manifest.json` record the exact command, source hashes and native oracle identity. Host-native packaging uses `SPUR_REMOTE=0`; correctness tests run remotely through `scripts/spur-cargo`.

## Validation

| Check | Result |
|---|---|
| Initial native test-first regression | Five intended assertion failures before the root fix |
| Comment follow-up test-first regression | 9 passed, 2 intended assertion failures before the token-boundary fix |
| Internal-path normalization test-first regression | 14 passed, 6 intended assertion failures on unchanged production bytes |
| Corrected native focused and related checks | 77 passed, 0 failed; format and diff checks pass |
| Rebuilt DuckDB SQL/native contract and pipeline | 31 passed, 0 failed |
| Three core compiler fixtures | Compile and run with Rust 1.94.1; MIR identifies the distinct `Vec`, local constructor and helper calls |
| Rebuilt extension scope controls | 6/6 pass, including block and line comments, whitespace and `Self` |
| Rebuilt extension multi-segment controls | 7/7 local paths bind; the qualified `std::cmp::max` remains unresolved despite a local `max` |
| Final independent review | 66 fresh native/SQL fixtures agree; no critical, important or minor findings |
| 52 frozen files | Nodes, spans, macro invocations and macro scopes exactly match the frozen facts |
| Real SpurCode constructor control | Retains `Vec::new` as scope `Vec`, target NULL; no `EvidenceEpoch::new` binding |

The selected-file call counts are 34,591 → 34,224, with local targets 5,206 → 5,039. Fifty edge lists change. `build-4d32f19d4/validation-from-notebook.json` retains every comparison query, parameters and result, including caller/name delta diagnostics. These are extraction changes, not a precision measurement or proof that every changed binding was wrong. The previous whole-project 12.48% figure has not been recomputed by this repair.

The notebook is `/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`. Rust cell `ef8529f1-7aa7-4009-a566-7ba98b17902a` (execution 160) invokes the new artifact in fresh native DuckDB 2.0.0 processes and saves all three checks under `build-4d32f19d4/`. SQL cell `a6d91056-65d9-49cc-ad72-42cffd74cfe3` (execution 22) displays those saved receipts. `notebook-normalization.json` and `notebook-normalization-summary-sql.json` preserve source, output and execution provenance. Earlier cells retain the experiments that found both regressions. The retained managed SQL connection still has the baseline extension loaded; the summary cell does not reload it.

Load the corrected artifact in a fresh DuckDB 2.0.0 process configured to allow the local unsigned extension. A process with the earlier `spur_graph` already loaded needs a restart before selecting this build. Re-ingest with a new extractor-version cache key; existing cached facts keep their old bindings. Macro-body calls remain heuristic, and this repair does not add compiler macro expansion, full name/type resolution or a speed claim.

The complete reviewed source tree is `c0e390e7f5b00b7817a7b5b59a70ccd01bb1c43c`, identical for the normalized reviewer branch and ordered integration tip `4d32f19d4`. [Final review](artifacts/2026-11-15-rust-macro-qualification-fix/final-review/result.json) records the fresh 66-fixture checks, source and artifact identities, all three TDD histories, and limitations. Its earlier important finding was independently reproduced before normalization and verified resolved afterward. Existing `crate`/`self`/`super` resolution limits remain; no broader Rust name-resolution claim is made.

The automated plan merger applied the implementation but treated the intentionally empty read-only review diff as a conflict. Its partial branch had the exact verified source tree. The parent was fast-forwarded to the original ordered commits after approval, preserving that same tree and all seven protected input hashes. This was an integration bookkeeping issue; no source conflict remains. `delivery.json` records the final source, binary, review and verification identities.
