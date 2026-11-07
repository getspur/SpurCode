# G03 worker evidence

Canonical assignment: `bd-3t8y`, delegation `6d483939abde4385`,
plan `7c25f4ce-dee1-45fa-91fb-6e8b16db82bb`. Starting clean commit:
`bb1f602b3b2f35943a3c4546adad8d9883528709`.

The initial `root-pre/verify.py --current-source` exited 0 before any edits.
`entry-pre-summary.json` records that initial result. A later current-source
recheck after the test edits correctly rejected the changed provider test;
`pre-frozen-replay.log` authenticates the frozen PRE epoch independently.
`entry-live-receipts.json` contains both complete live solver envelopes, equal
in full to `root-pre/proof.json.gz`'s `reloaded` entries. PRE is prospective
policy, not implementation acceptance. ROOT owns the fresh independent POST.

INTENT UUID `57e63df4-78dd-4b0b-9b38-ad63797a24a5` persisted on the canonical
assignment as comment `4629`. Its identical replay is recorded as idempotent.
The first replay transport was cancelled; the unchanged retry succeeded.
Exact request, responses and that cancelled response are preserved separately.

Test-first checkpoints: `6c772c4d9`, `342ee01bc`, and `9bb7cafdb`. Evidence/input checkpoint:
`227e70bb9`. No production implementation preceded these tests.

`red.log` is the original sandbox network failure (exit 200), with local fallback
forbidden. `red-network-retry.log` is the network-enabled retry: the builder
started successfully, but a test checkpoint advanced during upload, so the
source-stability guard rejected it (exit 1). Neither is compiler RED. The stable
retry is preserved separately in `red-stable.log` and its command record.

`native-request.json` supplies the exact source fixtures and optional provider
replay cases. `native-config.json` describes the binding. Both files are embedded as exact byte literals
by the existing provider integration target, so the remote executable must use
those bytes even though the remote build excludes the research archive. The
first actual compilation (`red-stable.log`) reported 39 missing-API E0599 errors
and two missing include-file errors. Test-only checkpoint `9bb7cafdb` corrected
the include mechanism without changing the archived input bytes; `red-api.log`
records the subsequent API-only compiler RED. `G03_NATIVE_JSON` is the native export line.
The export includes before/after values and serialized bytes, captured-empty and
absent lanes, owned-value replacements, native canonical/capability digests,
full/delta reader output, actual metadata rejection errors and provider replay
candidates. Native output exists only after the target executes successfully.

`run.py` invokes the required commands with remote builds and no local fallback,
archives exact stdout/stderr, exits, times, result counts and source identities,
and extracts native JSON without computing native graph behavior. `replay.py`
authenticates archives and hashes; it never implements graph logic or BLAKE3.


The default-sync Parquet and all-target attempts also exposed pre-existing
`include_str!`/`include_bytes!` dependencies on frozen research artifacts. Their
complete errors remain in `parquet.log` and `check.log`. The documented remote
helper setting `SPUR_SYNC_FULL=1` includes these inputs without changing any
archive or out-of-scope test. Full-sync retries use separate `parquet-full` and
`check-full` logs/command records. No failed remote run was rerun locally.

For native replay, `post-candidates.json` joins four actual native request cases
into the existing `ReplayRequest` shape. ROOT can inspect/mutate those cases,
copy the result to `native-request.json`, run `bind_inputs.py --write`, format
through `scripts/spur-cargo`, and execute the provider target remotely.
`bind_inputs.py` without `--write` verifies exact byte equality. Such a changed
request requires a fresh executable and output; the existing native archive
continues to describe only its original bound request.


Final native verification on the unchanged implementation source:

| Target | Passed | Failed | Ignored | Filtered |
|---|---:|---:|---:|---:|
| generic_evidence_providers | 16 | 0 | 0 | 0 |
| canonical_hash (lib) | 5 | 0 | 0 | 587 |
| macro_invocation_parquet | 9 | 0 | 0 | 0 |
| parquet_roundtrip | 9 | 0 | 0 | 0 |
| parquet_schema_invariants | 4 | 0 | 0 | 0 |
| rust_evidence_parquet | 18 | 0 | 0 | 0 |

Total: 61 passed, 0 failed, 0 ignored, 587 filtered. All-target check exited 0
with full sync; local formatting and complete base diff checks exited 0.
No ignored tests were added; no required suite reported an ignore. Existing
warnings are retained in unchanged MCP/Lance code and the overlay benchmark.
`verification-summary.json` joins exact command/status/count/source records.

Implementation commit: `9022ea0be` (`feat(spur-graph): G03 prepare typed evidence
accessors and drafts`). `implementation.diff` contains the complete unified
three-file source/test diff from the dispatch base. No other production/test
path changed.

Actual native export: `native-actual.json`, SHA-256
`85818a0b4b929432d742d33f098e70b3691d8637950a26fd09d5adfd392356a5`.
The executable SHA-256 is
`0fb5b8bbcf996ffb9efea2ce155b778595d287444fd9fcbe51e8a9f8d3b7ae7f`.
`native-index.json` retains request/config and all compiled source hashes plus
full compiler identity. Re-run archive authentication with `python3
.../G03/worker/replay.py` from this checkout. No Python code implements graph
behavior or BLAKE3.

G03 is ready for ROOT's fresh independent POST and review only. The canonical
issue remains in_progress. No self-approval, closure, merge, push, PR, successor
work, generic-carrier migration or native subagent delegation occurred.


Completion audit UUID `84f19e7a-f39e-432e-8814-9c4f58250a1d` persisted on
canonical `bd-3t8y` as comment `4633`; its identical replay returned
`recorded=true`, `idempotent=true`, the same comment ID and the same complete
audit. Exact request/response/replay files are preserved. Evidence commit
`2c5b950a4` is the audit's committed evidence anchor; the subsequent audit archive
commit additionally protects the persisted receipt and SHA-256 inventory.
`SHA256SUMS.json` hashes all archive files except itself and the replay-output
log, which is produced after the inventory. All claim-bearing command/native
and audit files are covered.


Final archive check correction: once the saved patch became tracked, Git's
complete base diff check flagged its single-space blank context lines. This
was an artifact-format issue; all Rust source and native test outputs were
unchanged. `implementation.diff` now uses valid zero-context unified format
(`git diff --unified=0`), retaining every source/test change. For application,
use `git apply --unidiff-zero`. The original check output and successful retry
are retained in `archive-diff-before-fix*` and `archive-diff-after-fix*`.
The correction does not alter the previously executed 61 tests, native output,
compiler identity or runtime command statuses. Completion comment 4633 records
the earlier evidence anchor; a follow-up audit records this archive correction.


Archive correction commit: `f9a33c14e`. Completion addendum UUID
`911e4885-aaad-4fcd-ad16-b2366e3df345` persisted as comment `4635` on the same
canonical issue and replayed identically (`recorded=true`, `idempotent=true`).
The exact correction request/response/replay are archived. Final full-base diff,
archive hashes, native compiled bindings and all audit replays authenticate.
