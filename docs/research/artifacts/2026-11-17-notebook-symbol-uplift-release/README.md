# U5: fresh notebook symbol release acceptance

Task **bd-c1v3**, plan U5; isolated worker base `c90ba9264fb99f620227f363bdbf146bd8b5e38a`.
Ready for root's independent package/notebook review. No merge, beads closure,
frontend edit, deployment, AWS reboot or resource change.

Rationale: release acceptance now exercises shared notebook core tests with
their frozen remote fixtures, validates the real native consumer boundary, and
publishes the exact notebook v1 contract without changing approved U1–U4 Rust
or consumer implementation bytes.

`unified.patch` contains the complete source/docs/tests/standalone tools diff
from the dispatched base; raw receipts are separate committed files. It uses
zero context (`git apply --unidiff-zero`) to preserve a compact review artifact.

## Candidate and execution identity

- New package: `/private/tmp/spur-notebook-u5-bd-c1v3-final/spur_graph.duckdb_extension`.
- SHA256: `c5f36a0f99213b570c1fe862561f3bb8882f40936c0afe6565f46ceca1878d48`.
- Sidecar SHA256: `187b64ec5da7b4fe5a3120df233b63983e62fb6c811c4ff77339a572f92b069d`.
- Raw library SHA256: `da4d546647b92af08ce5e2c4ff0b7687e7383f1979a53fcb2a9e5e89aa1170c6`.
- Extractor: `source-sha256-v1:6ff57778ccc62a9cbd78f39549836fd9a7c91f55c3f94830443b239e8a17f2c2`.
- Both source probes independently enumerate **325** inputs. New notebook
  source/preparation policy inputs are included; the approved native source ID
  is unchanged from U3, while this fresh optimized binary is distinct.
- Python 3.12.10, native DuckDB **2.0.0 / v2.0.0**, **osx_arm64**,
  Rust/Cargo 1.94.1. Executable/engine hashes are in [driver/receipt.json](driver/receipt.json).
- Release source HEAD `5545f6d1e`, with README/contract edits recorded as dirty.
  Documentation and acceptance artifacts are outside the native fingerprint;
  the exact final contract hash and acceptance/consumer hashes are retained in
  [native-names/evidence.json](native-names/evidence.json).

Original package `28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122`
and its sidecar remain unchanged. [verification-final.stdout](verification-final.stdout)
checks **281** original tracked notebook evidence files, the historical package,
four prior U3 packages/sidecars, every current native artifact, current sidecar,
source inventory and retained receipt stream. No historical evidence is rewritten.

## Meaningful RED → GREEN

The only automation change is five added driver lines: a focused core phase,
its separate target, and the cloud wrapper's existing `SPUR_SYNC_FULL=1` setting.
The existing release driver was read first; its packaging, native build,
full-suite and source-stability automation was retained.

| Regression | Test commit → fix commit | RED → GREEN |
|---|---|---|
| Missing focused notebook core release gate | `fe06191df` → `43a96744d` | Exit **1**, assertion that `core-notebook-tests` is absent → exit **0** |
| Missing remote docs fixture transport | `779b5a1e6` → `5545f6d1e` | Exit **1**, `SPUR_SYNC_FULL` is absent → exit **0** |

Both rows use the identical focused Python command for RED and GREEN:

```sh
$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests \
  -p test_release_driver.py \
  -k test_release_runs_focused_core_notebook_tests_before_adapter_tests -v
```

`GRAPH_PYTHON=/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python`.
The compiled source probe used by the orchestration unit test is named in
`driver-red.json`, `driver-green.json`, `fixture-sync-red.json` and
`fixture-sync-green.json`, with exact executable SHA256, argv, source hashes,
HEAD, overrides and raw `.stdout`/`.stderr` siblings. The unit tests substitute
unrelated orchestration phases but execute the real source probe. Final release
execution below exercises the actual remote commands and fresh native build.

## Exact final commands and results

Each phase was invoked through `capture.py PHASE MODE COMMAND...`; capture
refuses existing receipts and preserves raw exit/stdout/stderr and source hashes.
Common overrides are `PYTHONDONTWRITEBYTECODE=1`, `SPUR_SCCACHE_S3=0`,
`SPUR_NO_LOCAL_FALLBACK=1`, `RUSTC_WRAPPER=''`, `CARGO_INCREMENTAL=0`.
Remote mode sets `SPUR_REMOTE=1`; HOST mode sets `SPUR_REMOTE=0`.

```sh
# release-final (REMOTE Rust; separate deliberate HOST native build)
$GRAPH_PYTHON crates/spur-graph-duckdb/scripts/check_release.py \
  --out /private/tmp/spur-notebook-u5-bd-c1v3-final \
  --host-native --expect-platform osx_arm64

# native-names-acceptance (HOST)
$GRAPH_PYTHON docs/research/artifacts/2026-11-17-notebook-symbol-uplift-release/check_native_notebook.py \
  --release /private/tmp/spur-notebook-u5-bd-c1v3-final \
  --out /private/tmp/spur-notebook-u5-bd-c1v3-native-names

# verification (HOST)
$GRAPH_PYTHON docs/research/artifacts/2026-11-17-notebook-symbol-uplift-release/verify_receipts.py
```

| Gate | Exit / concrete result |
|---|---|
| Final complete release driver | **0**, every phase exits zero |
| Core notebook Rust | **REMOTE**, **61 passed**, 0 failed/ignored; 580 deliberately filtered by the documented focused selector |
| Adapter Rust | **REMOTE**, **4 passed**, 0 failed/ignored; no filters |
| Fresh `--release --lib --examples` build | **HOST**, exit **0**, library and all seven native oracles emitted into this run's exclusive target |
| Complete native Python discovery | **HOST**, **223 passed** in 379.261s, **0 skips**, expected failures, unexpected successes, failures or errors |
| Independent final source/artifact guard | **0**, fresh source-probe directory agrees; native binaries and package remain unchanged |
| Standalone notebook consumer acceptance | **0**, native results below |
| Published Python and SQL examples | **0** each; exact extracted bytes are `documented_api_example.py` / `.sql` |
| Retained receipt/preservation verification | **0**, [verification-final.stdout](verification-final.stdout) |

[driver/receipt.json](driver/receipt.json) retains every exact child argv, cwd
and raw exit, runtime/compiler identity, native paths/hashes, source status,
package/sidecar trust and skip summary. [driver/archive.json](driver/archive.json)
binds the archived streams, probes, summary and sidecar to the originals.
The initial replay and verification remain unchanged in `native/`,
`native-acceptance.*` and `verification.*`. The final stricter replay also checks
every symbol name against its original slice; `native-names-acceptance.*` and
`verification-final.*` retain its exact command and results.
Native target/object files and the binary remain in the original fresh `/private/tmp`
release directory. `native-names/archive.json` binds the portable replay evidence and
raw child streams; its immutable Parquet runs remain under the named native output.
Inner negative-control suites deliberately print failures/skips to prove their
rejection; the authoritative outer summary has zero skipped/expected-failure tests.

## Real inputs, ranges and source-grounded 55 → 50

The standalone replay uses tracked PoC `real-inputs.json`, SHA256
`bc0e5f23ba34c20149d5c8168f4a3a1fa8b3af3c8759adaef418f2343686b608`, and
independent frozen anchors, SHA256
`93623d013f4767ac621853e7855aa84a700b73d1814f84c6b3c02a555928fd83`.
It checks these exact files, every frozen source SHA, current compiled source
inventory, native engine, verified package/sidecar and loaded source identity.
Caller-frozen snapshot labels remain caller assertions.

| Snapshot | Symbols | Functions / independent anchors | Status |
|---|---|---|---|
| `git:cd24d5c4779d11a5b2dd1bbe55c93da12856ae41` | 46 | 17 / 17 | complete |
| `git:07db59f08dbced2c8bedd17290c78f57b428a2bc` | 24 | 11 / 11 | complete |

All **70** symbols have names checked against their original slices, checked cell
IDs, cell/document hashes, original UTF-8 byte and line bounds. All **28** function anchors match independently
frozen names/cells/ranges. Every cell source matches the decoded real JSON.
Both same-document comparisons retain the complete source documents and return
zero symbol, cell and document changes. Two distinct cells declaring `helper`
retain two functions, original refs and a zero-change comparison.

The real result is **50** symbol changes: **12 added, 4 modified, 34 deleted**,
plus **10** cell events. All **54** returned before/after delta refs resolve in
the exact published snapshot/source context. The PoC's **55** deltas were
12 added, 9 modified, 34 deleted. Five unchanged `wb-graph` symbol slices shifted:

| Symbol | Before bytes `[start,end)` | After bytes `[start,end)` |
|---|---|---|
| PERSPECTIVE_VERSION | [118,147) | [128,157) |
| EvidencePort | [240,391) | [329,480) |
| esc | [393,599) | [647,853) |
| normalizeValue | [757,1362) | [1026,1631) |
| tableRows | [1364,1550) | [1633,1819) |

`native-names/evidence.json.offset_only` retains the equal original text and both
fully hashed refs for each one. The standalone check independently selects
these slices from the frozen originals, verifies byte equality and confirms
none is emitted as a symbol edit. The cell source/move/metadata events remain.
This explains five fewer modifications without forcing an obsolete count or
claiming execution equivalence. Ten cell events are four additions, three
deletions, a move, a source change and a metadata change.

The replay also checks `:dep` with Unicode/CRLF, complete recovered `helper`
at original line 2, original/analysis hashes, exact byte masking and identity
coordinate mapping. A dependency-only edit has no symbol delta and retains a
configuration event. `:load`, multiline dependency syntax and a non-leading
header retain explicit partial diagnostics; raw-string directive text stays
unchanged. The exact native schemas and expanded public documentation aliases
are compared, and manifest entries retain schemas/counts for all **12** typed tables.

## Publication failures and retained history

The consumer runs real native children. `native-names/evidence.json.children` retains
argv/cwd/environment/PID/raw exit and checksums for every `.stdout`/`.stderr`:
healthy **0**, candidate-written failure **23**, candidate-written interruption
**-15**, forged-source rejection **1**, subsequent healthy publication **0**.
Every failing child is reaped; staging is removed. The old pointer and all
**21** original published files stay byte-identical on every failure. A later
healthy run publishes a distinct run ID, and both original and current runs
reopen successfully without relabeling snapshots.

## Public signatures and limits

The [release contract](../../../../crates/spur-graph-duckdb/docs/release-contract.md#notebook-symbols-and-changes-v1)
declares every ordered SQL v1 nested type, NULL/absence/status rule,
coordinate/preparation/identity limit and consumer migration. Public Rust:

```rust
normalize_notebook(bytes: &[u8]) -> NormalizedNotebook;
extract_notebook_symbols(path: &str, bytes: &[u8]) -> anyhow::Result<NotebookSymbolDocument>;
compare_notebook_sources(path: &str, old_path: Option<&str>, change_kind: &str,
    left_source: Option<&[u8]>, right_source: Option<&[u8]>) -> anyhow::Result<NotebookComparison>;
```

The first lives in `spur_graph::extract::notebook_model`; the latter two in
`spur_graph::extract::notebook_symbols`. SQL:

```text
spur_graph_notebook_symbols(path VARCHAR, source VARCHAR) -> Document
spur_graph_notebook_changes(path VARCHAR, old_path VARCHAR, kind VARCHAR,
                            left_source VARCHAR, right_source VARCHAR) -> Comparison
```

Consumer v2/native notebook v1 retains ordinary table/schema and historical
consumer v1 read compatibility. Python:

```python
run_consumer(fixture, output, package, *, expected_extractor_id, timeout=None, fault=None)
read_result(output, snapshot_id, *, run_id=None)
read_notebook_source(output, snapshot_id, blob_oid, source_ref, *, run_id=None)
```

Native source hashes and decoded-cell coordinates are verified; repository and
snapshot provenance remains caller supplied. Anonymous identities depend on
source/occurrence, public IDs include positions, and ambiguous identities have
explicit diagnostics. Preparation is bounded single-line Evcxr support, not a
kernel parser, dependency resolver, cross-cell runtime analysis or compiler.
Unsigned local native DuckDB 2.0/macOS ARM64 support is the observed platform;
no wider ABI, UI, deployment or signed-release claim follows.

Catalog discovery and exact finite PRE/POST requests/results are retained in
`solve-catalog.json`, `solver-pre.json`, `solver-unique-pre.json`, `solver-post.json`,
all with `persist:false` due the known full persistence store. They check finite
cardinality, uniqueness and complete source-context reference tuples; cached
solver outcomes are labelled by their raw envelopes. These are not proofs of
arbitrary parser behavior or execution. Actual remote/native gates establish
the concrete runtime claims above.

## Preserved unsuccessful attempts

- `restricted-remote/`: exit **200**, both AWS routes inaccessible from the
  restricted sandbox; local fallback disabled, no compilation. Prior root/U3
  disconnected-SSM evidence also remains unchanged.
- `ssm-health` / `ec2-health`: authorized read-only network-enabled queries
  observed **Online**, running/healthy on existing `i-053c907fa48c0ba69`.
- `upload-stopped/`: exit **1** before compilation because this worker committed
  the first fix during source upload. The wrapper correctly rejected the changed
  HEAD; a later stable checkout was used. No source failure is concealed.
- `remote-missing-fixtures/`: real compiler exit **101** for the two tracked
  `include_str!` JSON fixtures excluded by the default cloud `docs/*` filter.
  The existing full-sync option fixed this in the scoped release driver;
  meaningful Python RED/GREEN and actual **REMOTE** 61-test GREEN are retained.

The scope signal `d8d2820e-fc56-4d85-9069-9905a39c9315` and durable audit
`b295e369-a227-4e57-a734-3a0784e27e42` record that investigation and containment.
No out-of-scope file writes were needed. No remote compiler/assertion failure
was retried as HOST success; the final Rust gates ran remotely. Root retains
independent native notebook review, issue closure and merge authority.
