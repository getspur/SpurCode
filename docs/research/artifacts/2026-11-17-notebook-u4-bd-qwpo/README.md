# U4 notebook consumer routing and immutable readback — bd-qwpo

Rationale: preserve notebook identity, coverage and original cell source references through typed native extraction and immutable publication while keeping ordinary table schemas readable.

Base: `1d23aee62d80b1964d322fb5af742feddbccbacb`; isolated worker worktree
`06f06a07-413e-4d6f-9fdf-1e7645d097fc`, initially clean. U1/U2/U3 were approved
before U4 started. Test commits `b359a4123` and `0342d3f1b` precede implementation
`8427e33e9`. No publisher, graph cache/pipeline, Rust, fixture or old evidence
changes. No merge or beads closure. [unified.patch](unified.patch) contains the
complete production/test diff from that base; this directory retains receipts
and executable verification helpers separately.

Consumer contract **v2** adds `notebook_explore` and `notebook_review`; native
notebook payloads remain contract **v1**. The original `sources`, graph-only
`explore`, ordinary-pair `review`, and file-facts table schemas remain unchanged.
Snapshot coverage adds notebook status counts and considers partial/invalid/NULL
notebook coverage incomplete. Review coverage separately counts notebook pairs,
statuses and diagnostics. Notebook routing uses either pair path, so a rename
cannot enter the legacy notebook guard through its old path. Native typed
invalid/partial results remain observable instead of becoming unavailable rows.

`sha256:utf8-v1` is unchanged. Fixture source bytes and lineage keys are validated
before extraction; native document/cell hashes and original-source references
are checked before snapshot/occurrence labels join notebook facts. Distinct
source/path/extractor keys are extracted once; repeated lineage rows remain
occurrences. Metadata, diagnostics, all cells and original/prepared sources,
symbols, before/after references and all three typed change lists survive in
Parquet. Native errors still abort the child. Existing parent publication,
locking, cleanup, timeout and hash checks are unchanged.

Exact current table schemas, row counts, fixture/source identities, immutable
manifest/file hashes, function refs, native identity and failed child exits are
in [native-readback-final.stdout](native-readback-final.stdout). It records an
actual v1 run produced by the byte-exact base consumer in a private crate-shaped
harness, then independently reads it after v2 publication and compares every
pre-existing table schema. It also reopens the earlier real notebook run by
explicit immutable run ID. V1 reads do not synthesize notebook facts. New v2
reads reject unknown consumer/native contract versions or absent notebook tables.

## Exact validation

All executions here are **HOST**, using DuckDB Python **2.0.0**, osx_arm64 and
the approved U3 candidate. This Python-only task changes no native extractor
input and does not rebuild Rust. The 325 native source inputs are independently
rehashed and match the packaged extractor. No remote compiler/test success is
claimed. The original U3 [remote-baseline receipt](../2026-11-17-notebook-u3-bd-lnhs/remote-baseline.json)
and its raw streams are preserved and rehashed: exit **200 before compilation**,
both routes failed infrastructure readiness. Root authorized deliberate host
execution after EC2 health/SSM ConnectionLost checks; no remote compiler or
assertion failure was retried locally. Any later Rust work still uses
`scripts/spur-cargo`; U5 owns the complete fresh release-driver run.

```sh
export SPUR_GRAPH_EXTENSION=/private/tmp/spur-notebook-u3-bd-lnhs-attempt2-final/spur_graph.duckdb_extension
export GRAPH_PYTHON=/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python
export RECEIPTS=docs/research/artifacts/2026-11-17-notebook-u4-bd-qwpo
```

Each execution invokes `$GRAPH_PYTHON "$RECEIPTS/capture.py" PHASE host COMMAND...`.
Capture retains exact argv/cwd/environment, source HEAD/SHA256s, package/sidecar
hashes, raw stdout/stderr and exit. Receipts refuse overwrite.

| Phase | Exact COMMAND after capture | Exit | Evidence |
| --- | --- | --- | --- |
| `native-red` | `$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests -p test_app_consumer.py -k notebook -v` | 1 | Five meaningful assertion failures: legacy notebook guard / absent guarded reader |
| `native-green-attempt3` | Identical argv to RED | 0 | Five notebook tests pass, including 28 independent original byte/line anchors |
| `consumer-green` | `$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests -p test_app_consumer.py -v` | 0 | All **19** tests pass, no skips; ordinary tables/results, replay, child exit/interruption/timeout, parent interruption, forged keys/source identities, corruption and old-pointer guarantees |
| `native-readback-final` | `$GRAPH_PYTHON "$RECEIPTS/inspect_consumer.py"` | 0 | **46/24** symbols, **17/11** functions, **70** resolved symbol refs, no-op correctness, actual v1/v2 readback, schema equality and notebook child-failure guards |
| verifier | `python3 "$RECEIPTS/verify_receipts.py"` | 0 | [verification.json](verification.json): all phase hashes, 325 native inputs, 37 immutable files, old evidence/package and solver results independently checked |

Initial implementation attempts remain raw: `native-green-attempt1` exit1
caught deprecated DuckDB lambda syntax; `native-green-attempt2` exit1 caught a
duplicate projection block. Both are failed development runs, not meaningful
RED or GREEN. `native-readback` exit1 reached the archived-v1 harness but lacked
its publisher symlink; fixing that evidence-only setup produced
`native-readback-final` exit0. No test or production assertion was weakened.

Finite catalog uniqueness and source-reference PRE/POST requests/results are
retained in [solver-pre.json](solver-pre.json) and [solver-post.json](solver-post.json).
Cell-qualified/structured identities and coherent reference tuples pass;
flattened/delimiter-colliding keys and wrong before-source tuples fail with
family diagnostics. Every request uses `persist:false` because persistence is
known full. These snapshots complement runtime tests; they do not prove parser
or execution behavior for arbitrary notebooks.

## Native identity and limits

- Package SHA256: `ed42b39bd3fda2d79ffce5c482c3c869643340f689eded84e9894e709714e35b`.
- Sidecar SHA256: `7f6cd25b5db413e29ca23b5bfa36e78b2bacfb17b4c43ed7c7537f0b900cbb66`.
- Extractor: `source-sha256-v1:6ff57778ccc62a9cbd78f39549836fd9a7c91f55c3f94830443b239e8a17f2c2`.
- Historical `28ddce83` package is unchanged and independently rehashed.

Real comparison preserves approved U2/U3 output: **50** symbol changes and
**10** cell events. The PoC's 55 was an observation of its transport matcher;
U2's source-grounded correction is unchanged here. No consumer rematching or
delta-count normalization occurs.

V2 extraction requires a notebook-capable native candidate; historical v1
*results* remain readable without loading native code. Graph-cache replay is
the existing graph-only replay check; notebook projections are regenerated from
frozen sources in each private child, not stored in a new cross-run cache.
Native notebook limitations remain: bounded leading single-line Evcxr `:dep`
masking, explicit partial/anonymous/unsupported coverage, no cross-cell runtime
resolution or execution-equivalence claims. Caller repository/commit/profile
labels remain assertions. Package trust remains unsigned/local. Full native
release suite/docs and notebook UI/runtime review belong to U5/root.
