# U4 attempt 2: NULL pair paths and occurrence-independent Review reads

Both review gaps are fixed by keeping every accepted pair in one route and
binding published Review refs to their exact immutable source context.

Commits `31e5eb148` and `8f89b31a1` contain failing-first regressions and raw RED
evidence; `6df9ef629` contains the consumer fix. The branch retains original
`b359a4123` → `0342d3f1b` → `8427e33e9` → `a135b0bfb` ancestry. The pre-applied
tree was byte-identical to `a135b0bfb`, so the worker branch was attached to
that existing commit before rework; none of those commits/evidence were rewritten.
[rework.patch](rework.patch) is the production/test diff from attempt 1;
[unified.patch](unified.patch) includes the complete U4 diff from dispatched
base `1d23aee62`. Receipt files are additional committed evidence.
Both exported patches use zero context; apply with `git apply --unidiff-zero`.

Pair classification uses NULL-safe, complementary predicates. Ordinary A calls
translate absent `old_path` to the existing native empty-path sentinel. Ordinary
and notebook D calls with absent `logical_path` pass the validated `old_path`
as the native path. Published pair columns retain the exact caller values.
Notebook comparisons explicitly retain native NULL as `null_input`, without
dereferencing it or claiming success. Required-side validation is unchanged.

`read_notebook_source` requires the published `sources` full key and finds
documents in Explore or the exact Review before/after side, independently of
occurrence rows. It verifies native document version/path/hash against frozen
source bytes, then original cell identity/hash/coordinate/range. Explore remains
occurrence-based. No current-run, alternate-path or opposite-side substitution
is performed. Consumer v2/native v1 and all table schemas remain unchanged;
historical consumer v1 stays readable without synthetic notebook facts.

## Exact RED/GREEN and host execution

All commands ran as **HOST** under the authorized infrastructure recovery;
there was no Rust rebuild or remote compiler/test run. The preserved U3 remote
receipt and streams still record infrastructure exit **200 before compilation**,
SHA256 `0f74fe35a860681aa0525ada9d2647798e7d6f554e23ebd0fc55a5a1420dad4a`.
This is not a local retry of a remote compiler/assertion failure. Any later Rust
work must continue using `scripts/spur-cargo`; U5 remains gated.

```sh
export SPUR_GRAPH_EXTENSION=/private/tmp/spur-notebook-u3-bd-lnhs-attempt2-final/spur_graph.duckdb_extension
export GRAPH_PYTHON=/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python
export RECEIPTS=docs/research/artifacts/2026-11-17-notebook-u4-bd-qwpo-attempt2
```

Each phase invokes `$GRAPH_PYTHON "$RECEIPTS/capture.py" PHASE host COMMAND...`.
The capture preserves exact argv/cwd/environment, HEAD and source SHA256s,
package/sidecar hashes, raw stdout/stderr and exit; it refuses overwrites.

| Phase | Exact COMMAND after capture | Exit | Result |
| --- | --- | --- | --- |
| `review-red-final` | `$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests -p test_app_consumer.py -k test_review_ -v` | 1 | Six meaningful failures: lost ordinary rows, notebook D crash, and missing Review refs for M/empty and R/empty/base-only/head-only lineage |
| `review-green-final` | Identical command to `review-red-final` | 0 | Four tests pass, including all subcases and before/after/context/historical guards |
| `consumer-green` | `$GRAPH_PYTHON -m unittest discover -s crates/spur-graph-duckdb/tests -p test_app_consumer.py -v` | 0 | All **22** consumer/fault tests pass without skips, including frozen **28 anchors** |
| `native-readback` | `$GRAPH_PYTHON docs/research/artifacts/2026-11-17-notebook-u4-bd-qwpo/inspect_consumer.py` | 0 | Fresh real-input/native-child execution; **46/24 symbols, 17/11 functions, 70 refs**, no-ops, 50 symbol changes/10 cell events, actual v1 execution/readback, ordinary schema equality and failed-publication guards |
| `verification` | `$GRAPH_PYTHON "$RECEIPTS/verify_receipts.py"` | 0 | [verification.stdout](verification.stdout): exact source/package hashes, 325 native inputs, 37 preserved + 37 fresh immutable files; archived v1/v2 reader compatibility and 70 refs re-read from each real run |

Initial `review-red` retains five meaningful failures before adding the exact
M/no-lineage case. The supplementary RED restores the exact prior consumer
bytes, then restores the fix only after committing the added regression.
`review-green-attempt1` exit1 caught the ordinary native API's NULL old-path
behavior; `review-green-attempt2` exit0 confirms the sentinel fix. Both remain
raw intermediate receipts, not the final RED/GREEN claim.

Catalog PRE/POST cardinality and source-context foreign-key requests/results
are retained in `solver-pre.json`, `solver-source-pre.json`, `solver-post.json`.
Every request uses `persist:false` due the known full store. Missing routes or
Explore-only source contexts fail; complete routes/Review contexts pass; wrong
snapshot/extractor tuples fail. Two preflight request-shape errors are retained
in `solver-pre-invalid.json` and are not proof. These are finite relational
checks, not proofs of SQL execution, parser correctness or arbitrary notebooks.

## Identities, public surface and limits

- Package SHA256: `ed42b39bd3fda2d79ffce5c482c3c869643340f689eded84e9894e709714e35b`.
- Sidecar SHA256: `7f6cd25b5db413e29ca23b5bfa36e78b2bacfb17b4c43ed7c7537f0b900cbb66`.
- Extractor: `source-sha256-v1:6ff57778ccc62a9cbd78f39549836fd9a7c91f55c3f94830443b239e8a17f2c2`.
- Consumer SHA256: `8f3520e55826d1151f44c1daf3d22f9971dd0217dc41323933168770c9c7e6b1`.
- Tests SHA256: `de354d45eaba87be8b40f2f2b1e845e8246f369e71fa81cc8e8d79d91793d6d0`.
- Frozen real-input JSON SHA256: `bc0e5f23ba34c20149d5c8168f4a3a1fa8b3af3c8759adaef418f2343686b608`.
- Source identity remains **sha256:utf8-v1**; exact base/head source hashes and
  snapshots are retained in `native-readback.stdout` and `verification.stdout`.
- Historical package `28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122`
  and all original immutable evidence are unchanged.

Public signatures are unchanged:

```python
run_consumer(fixture, output, package, *, expected_extractor_id, timeout=None, fault=None)
read_result(output, snapshot_id, *, run_id=None)
read_notebook_source(output, snapshot_id, blob_oid, source_ref, *, run_id=None)
```

No native/Rust, publisher, generic cache, file-facts pipeline, fixture-source or
prior-artifact writes. No beads closure or merge. Existing native limits remain:
bounded leading single-line Evcxr preparation, explicit partial/anonymous/unknown
coverage, no runtime/cross-cell execution equivalence, caller-asserted snapshot
labels and unsigned local package trust. U5's broader release/UI gates remain
outstanding.
