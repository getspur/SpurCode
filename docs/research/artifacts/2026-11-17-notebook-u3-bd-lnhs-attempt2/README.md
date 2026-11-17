# U3 attempt 2: legacy notebook diagnostic compatibility

Rework starts at original U3 commit `9dd6014ab`, preserving its test commits
`14d1cd28c` and `cceffb0cb`, sources and immutable receipts/packages. The staged
overlay matched that commit exactly; a soft reset restored its ancestry without
changing working bytes. The only additional production path is the explicitly
authorized `crates/spur-graph-duckdb/src/file_facts.rs` (scope mutation
`1cc24264-5159-4eab-95dc-e172a3f254a5`). No merge or beads closure.

## RED before production repair

`capture.py` exclusively records raw stdout/stderr, exact argv, environment,
HEAD and source hashes. `rust-red`: exit **101**, two actual core-to-diagnostic
regressions fail on malformed JSON and invalid notebook inventory; existing
internal error propagation checks pass. `native-diagnostic-red`: exit **1**,
unchanged legacy reproducer reports four errors on immutable prior U3 package
`0ea46655f5968dec09e3116e6941eb6fae6f2d2607a7e50b00049f9c55c66955` with the
required real native file-facts oracle.

All Rust execution here is explicitly **HOST**, `SPUR_REMOTE=0`, through
`scripts/spur-cargo`, under root's infrastructure-exit-200 recovery authorization.
Attempt 1 raw `remote-baseline` receipts retain both precompilation route failures;
no compiler or assertion failure is retried as a remote success. A separate target
was APFS-cloned from the earlier target for dependency reuse; current-worktree
crates were compiled and the new candidate will be packaged separately. Historical
packages and raw libraries remain untouched.

Catalog lookup is retained in `solve-catalog.json`: this repair is error-string
classification with no invented numeric bound and no applicable hard-rule family.
The unchanged U3 uniqueness/SourceRef solver requests/results remain in attempt 1;
no new symbolic correctness claim is made for parser or error classification.

## Repair and complete GREEN

Test commit: `61d65f537`. The following production repair recognizes only the
known `parse .ipynb JSON: [...]` and `invalid .ipynb notebook inventory: [...]`
contexts, requiring complete list delimiters; it preserves exact core Display
text, `status=extraction_error`, `error_code=malformed_input` and SQL NULL facts.
`NoTree`, setup errors, arbitrary internal errors and context near misses still
propagate. Actual malformed JSON and invalid inventory (including source/type/ID
failures) run through the public core extractor in the committed unit tests.

One-sentence rationale: restore the legacy row-diagnostic contract for shared
notebook normalization errors so malformed input does not invalidate DuckDB's
connection or break existing consumers.

The exact argv, working directory, environment, source SHA256s, package/sidecar
SHA256s and executable hashes are in each linked phase JSON; raw output is the
matching `.stdout` / `.stderr`. Tests, source and preparation policy are unchanged
after the final build. No hidden skips, expected failures or test selection
changes. The 76-test selection is the unchanged attempt-1 `regressions.py`.

| Receipt | Exact command after capture wrapper | Exit | Result |
| --- | --- | --- | --- |
| [rust-red](rust-red.json) | `scripts/spur-cargo --workdir crates/spur-graph-duckdb test --locked --lib --target-dir /private/tmp/spur-notebook-u3-bd-lnhs-attempt2-target` | 101 | 2 failed, 2 passed; actual JSON and inventory errors |
| [rust-green](rust-green.json) | Same exact argv as Rust RED | 0 | **4 passed**, no ignored tests |
| [native-diagnostic-red](native-diagnostic-red.json) | `$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests -p test_diagnostics.py -k test_invalid_paths_and_malformed_notebooks_preserve_native_diagnostics -v` | 1 | 4 errors on immutable package `0ea46655` with required native oracle |
| [native-diagnostic-green](native-diagnostic-green.json) | Same exact argv as native RED | 0 | **1 passed**, native connection remains usable |
| [host-build](host-build.json) | `scripts/spur-cargo --workdir crates/spur-graph-duckdb build --locked --lib --examples --target-dir /private/tmp/spur-notebook-u3-bd-lnhs-attempt2-target --message-format=json` | 0 | Current worktree library and all native examples rebuilt |
| [package](package.json) | `$GRAPH_PYTHON crates/spur-graph-duckdb/scripts/package_extension.py --library /private/tmp/spur-notebook-u3-bd-lnhs-attempt2-target/debug/libspur_graph_duckdb.dylib --out /private/tmp/spur-notebook-u3-bd-lnhs-attempt2-final/spur_graph.duckdb_extension` | 0 | Fresh LOAD/sidecar-verified package |
| [notebook-green](notebook-green.json) | `$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests -p test_notebook.py -v` | 0 | **7 passed**, 166.982s |
| [provenance-green](provenance-green.json) | `$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests -p test_contract.py -v` | 0 | **8 passed**, 24.584s |
| [regressions-green](regressions-green.json) | `$GRAPH_PYTHON docs/research/artifacts/2026-11-17-notebook-u3-bd-lnhs/regressions.py` | 0 | **76 passed**, 400.714s; ordinary API/temporal/file-facts/diagnostic/contract/package/consumer tests |
| [native-interface](native-interface.json) | `$GRAPH_PYTHON docs/research/artifacts/2026-11-17-notebook-u3-bd-lnhs/inspect_candidate.py` | 0 | Native identity, exact schemas/registry docs and frozen real fixtures |

`GRAPH_PYTHON=/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python`.
The shared GREEN environment sets `SPUR_GRAPH_EXTENSION` to the fresh attempt-2
package, `SPUR_GRAPH_LIBRARY` to its rebuilt raw library,
`SPUR_GRAPH_BUILD_SCRIPT` to the exact build-script executable from Cargo JSON,
and `SPUR_FILE_FACTS_ORACLE` to the newly built `debug/examples/reference_file_facts`.
All native tests require exactly DuckDB **2.0.0**, native platform **osx_arm64**.
The source HEAD recorded during GREEN is the test commit; the deliberately
uncommitted repair bytes are independently bound by source hashes and the native
extractor identity, then committed after verification. Existing core warnings
about unused code/imports remain outside this narrow repair.

## Exact final identity and handoff

- Packaged SHA256: `ed42b39bd3fda2d79ffce5c482c3c869643340f689eded84e9894e709714e35b`.
- Sidecar SHA256: `7f6cd25b5db413e29ca23b5bfa36e78b2bacfb17b4c43ed7c7537f0b900cbb66`.
- Raw library SHA256: `ec6f5fb18070de55175536384b8f1528f06c59123547a7e6a15e99098547e64a`.
- Extractor: `source-sha256-v1:6ff57778ccc62a9cbd78f39549836fd9a7c91f55c3f94830443b239e8a17f2c2`.
- Build-script SHA256: `6178e08e6e198106258d1c5a9e7b2d8724dc2e5b50c733002b74c326464d0165`.

[verification.json](verification.json) is exact output from
`python3 docs/research/artifacts/2026-11-17-notebook-u3-bd-lnhs-attempt2/verify_receipts.py`:
exit **0**. It checks all 10 execution receipts, binds all GREEN source/native
identities to this one artifact, independently recomputes the 325-input extractor
identity retained in [build-source-identity.json](build-source-identity.json),
checks actual native recompilation, and proves old/new SQL schemas compare equal.
It also verifies all **17** immutable attempt-1 receipts/logs, the three earlier
packages (`3409c6b1`, `790a1922`, `0ea46655`), prior raw library and historical
`28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122` bytes.
The first verifier run passed; redirecting its output to its own JSON directory
then exited 1 while trying to read the still-empty report. The helper now excludes
that report, and the identical checks/export exited 0. Native suites were unaffected.
Core, build.rs, metadata, original U3 adapter/tests and earlier evidence remain
unchanged. `scripts/spur-cargo --workdir crates/spur-graph-duckdb fmt --check`
and `git diff --cached --check -- crates/spur-graph-duckdb` both exited **0**.
The full staged whitespace check exited 2 on standard blank context lines in
unified patches and the verbatim Rust log's final blank line; those evidence
bytes are retained. Source and Markdown/SQL handoff checks pass separately.

[public-interface.md](public-interface.md) gives exact public Rust/SQL signatures,
field aliases, NULL/absence/status semantics, preparation limits and updated
runnable U4/U5 commands. [native-schema.sql](native-schema.sql) retains the full
expanded actual `DESCRIBE` strings and runnable queries for both new functions,
ordinary symbol/change functions and unchanged exact v1 info STRUCT.
[native-interface.stdout](native-interface.stdout) adds native registry docs,
engine identity and original-source function refs. The fresh package still
recovers **46/24 symbols, 17/11 functions and all 28 independent anchors**;
real comparison remains **50 symbol changes and 10 cell events**, directly from
approved U2. Large mixed nested lists, canonical refs, duplicate-helper no-op,
NULL/absence, M/A/D/rename, unsupported/error and old ordinary contracts pass.

[unified.patch](unified.patch) is the complete U3 source/test diff from approved
U2 base `eaf45df75`; [repair.patch](repair.patch) is just attempt-2 changes from
`9dd6014ab`. Original U3 TDD ancestry is preserved.

This resolves the confirmed P1 legacy/consumer compatibility blocker and is ready
for root review. Validation remains authorized **HOST** recovery, not remote
success. U4 notebook consumer routing/cache/publication, U5 complete release-driver
execution/release docs and root notebook-runtime readback remain downstream.
No deployment, full Evcxr/runtime dependency support, merge or beads closure is
claimed. Existing unsigned/local artifact trust is unchanged.
