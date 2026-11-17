# U3 native notebook SQL contract

Rationale: expose the approved core cell-aware facts and comparisons as typed
DuckDB v1 results so notebook symbols retain original-cell identity and coverage.

Isolated base: `eaf45df7562d03d6bc54164d9b2d9f56e197a938`. U1/U2 were approved
before U3 began. No unrelated edits were present. Tests were committed before
implementation in `14d1cd28c`; corrected DESCRIBE quoting and the valid TOML
directive witness were committed separately in `cceffb0cb` with another actual
baseline missing-function RED. No core, build.rs, consumer, release driver or
release documentation file is modified by the U3 feature.

Public entry points, NULL/absence/status semantics, type aliases, U4/U5 commands
and limits are in [public-interface.md](public-interface.md). The full exact
native DESCRIBE types, registered function docs and real-fixture evidence are
in [native-interface.stdout](native-interface.stdout). U5 owns transferring
these exact schemas into release-contract.md.

## Reproducible TDD

Every phase uses `capture.py PHASE MODE COMMAND [ARG...]`, retaining argv, cwd,
environment overrides, HEAD, source hashes, package identity, raw stdout/stderr,
raw process exit and log hashes. Receipts are exclusive and never overwritten.
Rust phases use `scripts/spur-cargo`. Native tests use Python
`/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python`, DuckDB **v2.0.0**,
platform **osx_arm64**; engine path/hash and Python identity are independently
recorded in `native-interface.stdout`. Source hashes captured for a test are
the current checkout; the tested binary's own identity is separately bound by
its immutable package SHA/sidecar, package receipts and corresponding build
receipt. In particular final RED loads the original baseline even though the
working feature source is present.

| Phase | Exact command after wrapper / Python | Exit | Observed result |
| --- | --- | --- | --- |
| remote-baseline | `scripts/spur-cargo --workdir crates/spur-graph-duckdb build --locked --lib --target-dir target/notebook-u3 --message-format=json` | 200 | Both AWS routes unavailable before compilation; no local fallback |
| host-baseline-build | same build, absolute target `/private/tmp/spur-notebook-u3-bd-lnhs-target` | 0 | Fresh pre-U3 native library |
| baseline-package | `crates/spur-graph-duckdb/scripts/package_extension.py --library /private/tmp/spur-notebook-u3-bd-lnhs-target/debug/libspur_graph_duckdb.dylib --out /private/tmp/spur-notebook-u3-bd-lnhs-baseline/spur_graph.duckdb_extension` | 0 | Fresh separate RED package |
| native-red / native-red-final | `-m unittest discover -s crates/spur-graph-duckdb/tests -p test_notebook.py -v` | 1 | All seven tests fail because the notebook functions do not exist |
| provenance-red | `-m unittest discover -s crates/spur-graph-duckdb/tests -p test_contract.py -v` | 1 | Seven pass; one fails on missing new adapter source |
| host-final-build | `scripts/spur-cargo --workdir crates/spur-graph-duckdb build --locked --lib --examples --target-dir /private/tmp/spur-notebook-u3-bd-lnhs-target --message-format=json` | 0 | Fresh U3 candidate and ordinary native oracles |
| final-package | package helper as above, fresh `--out /private/tmp/spur-notebook-u3-bd-lnhs-final/spur_graph.duckdb_extension` | 0 | LOAD/manifest probes passed |
| native-green-final | Same exact unittest argv as native-red-final | 0 | **7 passed**, 108.257s, no skips |
| provenance-green | Same unittest argv as provenance-red | 0 | **8 passed**, exact old info schema and source identity checks |
| host-rust-tests | `scripts/spur-cargo --workdir crates/spur-graph-duckdb test --locked --lib --target-dir /private/tmp/spur-notebook-u3-bd-lnhs-target` | 0 | **2 passed**; infrastructure failures remain errors |
| native-interface | `docs/research/artifacts/2026-11-17-notebook-u3-bd-lnhs/inspect_candidate.py` | 0 | Actual native schemas, runtime, binary identity and real comparison |

All host phases intentionally use **SPUR_REMOTE=0**, under the user's confirmed
exit-200 infrastructure recovery authorization. This is HOST execution, never
remote success or a local retry of a remote compiler/assertion failure.
`remote-baseline.stderr` preserves both actual route failures. Baseline/native
packaging uses the existing package helper. No retained historical package was
used as GREEN, written or overwritten.

`native-green` retains the first candidate's two test expectation failures: the
expected schema omitted DuckDB's quotes around reserved identifiers, and the
fixture used a Rust `//` comment in a TOML dependency assignment. Its nested
batch test passed. The corrected exact DESCRIBE expectation and supported TOML
`# café` witness produced `native-red-final` against the baseline before final
GREEN. No production parser policy was changed to make that fixture pass.

## Candidate and concrete acceptance

Current U3 candidate package SHA256:
`0ea46655f5968dec09e3116e6941eb6fae6f2d2607a7e50b00049f9c55c66955`.
Baseline package SHA256:
`3409c6b1f268865007202da595cd5be4a33c134146d2abadcf35e787e8e0df59`.
Extractor source identity:
`source-sha256-v1:519ca134fe311f6b7579cdbd6336169474c805218bb12ab69fd57a0b3f381865`.
The native engine SHA256 is
`1e0c41d7d52834b43bf66c1e8b25d6aeee7911609515c9563383b3ffb28919b3`.
The retained historical package28ddce83 lives in the main checkout, outside
this isolated worktree; verify_receipts.py independently checks its bytes.

The native tests assert **46/24 symbols, 17/11 functions, all 28 independent
original-source function anchors**, unique canonical IDs, all 70 symbol refs
and every emitted before/after delta ref. Real same-source comparisons and
duplicate helpers across cells are no-ops. The real comparison is **50 changes**
(12 added, 4 modified, 34 deleted) and **10 cell events**, matching approved U2.
The five PoC offset-only modifications are intentionally absent: U2's retained
`offset-only-real-changes.json` shows equal original slices for
PERSPECTIVE_VERSION, EvidencePort, esc, normalizeValue and tableRows at shifted
offsets. The adapter does not change matching policy.

Coverage includes mixed Rust/Python/JS, string/list sources, metadata, empty and
imports-only cells, UTF8+CRLF :dep byte mapping, directive config changes,
unsupported/invalid inputs, M/A/D/R/R100 and unsupported kinds, ambiguous IDs,
same-cell duplicate symbols, before/after refs, legacy notebook guards, exact
ordinary schemas, and registry docs. A selected multi-chunk batch spans 4097
input rows with null/invalid/empty/small/large nested results, 2301-symbol cells
and 25 directives; independent Parquet readback preserves typed nested values.

The existing recursive build source inputs cover all new adapter/core files,
including notebook_model.rs, notebook_symbols.rs, notebook.rs and the preparation
policy bytes. The provenance regression mutates each in an isolated source
copy and observes a changed extractor ID. No build.rs patch is needed.
PRE/POST `persist:false` solver requests/results retain finite uniqueness and
SourceRef witnesses; expected outcomes are unsat/sat/unsat/sat/sat/unsat.
They prove only those declared finite snapshots, not parsing/runtime/vector
allocation or arbitrary notebook correctness.

## Legacy compatibility finding and scope signal

`native-regressions` ran 76 existing tests from test_extension, test_temporal,
test_file_facts, test_diagnostics, test_contract, test_package and
test_app_consumer. It exited **1**, with 2 failures and 16 errors, all rooted in
the preexisting U1 enriched notebook input-error classifier incompatibility.
`baseline-diagnostic-red` runs the unchanged native malformed-notebook
regression against the fresh **pre-U3** package and reproduces the same fatal
InternalException: `parse .ipynb JSON: [(None, InvalidJson)]`. The old SQL adapter
recognizes only the exact former `parse .ipynb JSON` message. Consumer child
and interruption checks fail downstream of the invalid notebook fixture.

Beads audit **7364** and scope-drift signal
`ce735b48-09b8-4520-98fb-b4687ebd4ec6` request the single additional path
`crates/spur-graph-duckdb/src/file_facts.rs`. A concrete narrow proposed fix and
core-to-diagnostic regression are in `proposed-legacy-diagnostics.patch`.
At this receipt stage, that file is unchanged and scope authorization is
pending. No complete legacy/consumer GREEN or release readiness is claimed.

U4 owns consumer routing/schema/publication; U5 owns the complete fresh release
driver and release docs; root owns final notebook-runtime readback. No frontend,
deployment, merge, beads closure, compiler semantics or full Evcxr support is
claimed. Exact U3 tests, artifact checks and schema handoff remain usable while
the legacy diagnostic scope question is resolved.
