# M4 checked batching and shared-defaults local acceptance

Canonical issue `bd-2hgz`, delegation `baa51dff3b96439b`, plan
`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`. Implementation is ready for ROOT review;
the issue remains in progress. **ROOT must execute the complete old stage-A Jev
POST and supply shipped-packaging PRE before approval.** Packaging/default CLI
and fresh Notebook acceptance remain separate obligations.

Requested routing: codex / profile codex / gpt-6.1-sol / xhigh. The worker tool
context and selected model environment keys expose no resolver receipt; actual
resolved model is unverified and requires ROOT confirmation. No fallback or
replacement model selection was performed.

The current M4 source-bound PRE passed before edits/builds: 51 source hashes,
19 reference cases, 113 chunks and five reloaded receipts (three SAT, two expected
UNSAT). Complete worker reloads in `pre-reloads.json.gz` exactly match all five
archived root requests/results, including models and lowerings. The inspected capacity catalog request was also executed directly and
passed. INTENT audit `bd167d64-2358-48fd-9f42-c6f18ab76d29` persisted as comment
3456 and replayed idempotently. [Entry evidence](entry-evidence.json) retains the
initial incorrect plan-task selector's issue-not-found error and the corrected
canonical request, acceptance and exact replay. No canonical authorization
rejection occurred.

Test commits: `7deb780e9`, fixture call correction `242a9397f`. The first run
`red.json` is **compile-only**, not assertion RED. `red-assertion.json` observed
65 rows rejected as text count; six separate fresh-process `red-*.json` runs
observed the remaining behavioral assertion failures without the first panic's
poisoned test mutex. Fix: `eb41a96d2`. Supplemental oracle/real acceptance and
formatting milestone: `612b6397f`.

The neutral planner preserves indices, every input byte and order while checking
requested rows, protocol rows, UTF-8 per-text/total sizes and compact serialized
request bodies. It uses serde serialization for strings and envelope overhead,
reserving the actual maximum u64 request ID length. The four-byte wire prefix is
outside the body cap. Invalid individuals fail before helper service launch;
there is no truncation. The model handle now respects its batch argument and
uses one admitted logical request and effective deadline across chunks. Existing
exchange validation, generation/request correlation, helper reuse, restart and
owned cleanup remain authoritative. Later failures return an error for the
logical batch; already validated replies and outer graph per-row progress are
preserved. The original single-frame API stays strict. All 14 policy defaults
and native/model/analyst ownership source bytes remain unchanged.

| Executed check | Observed result |
|---|---|
| Seven new batching groups | 7 passed, none ignored |
| Independent root PRE oracle through actual Rust writer | 1 test, 19 cases / 113 chunks |
| Neutral protocol/policy/discovery/supervisor | 47 passed, none ignored |
| Graph store/model identity/progress/publication/disabled behavior | 66 passed, 3 existing download tests ignored |
| Graph discovery/compatibility/cancellation | 12 passed, none ignored |
| Analyst modes, sockets, owned runtime and dependency boundary | 29 passed, none ignored |
| Analyst embedding units | 9 passed, 1 existing real-sidecar test ignored; one nested subprocess also passed |
| Ordinary helper private sibling layout | 1 explicit real test passed |
| Ordinary helper three-model batched references | 1 explicit real test passed |
| Neutral/graph/analyst all-target compilation; local formatting | exit 0 |

[run.py](run.py) and each stage's JSON preserve exact argv, environment, source
head, timestamps, exit and hashes of compressed stdout/stderr. All Cargo work
used `scripts/spur-cargo`, `SPUR_REMOTE=0`, `TMPDIR=/private/tmp`, the authorized
target and ORT archive, one writer. The first neutral suite hit sandbox EPERM in
process inspection; `neutral-access` reran with authorized access and passed.
The initial failure remains retained. Existing graph/analyst dead-code and
unchanged worker warnings are recorded; this is not a warning-free lint claim.
One manually provisioned historical graph reference-executable test was explicitly
skipped; the new three-model comparison executed the pinned reference arrays.

The 32 compressed [case captures](cases/) contain actual envelopes, body byte
counts, identities, generations, ordered inputs, result/error observations and
cleanup events. They cover 141 emitted embedding chunks and 423 independent
row/raw/body comparisons. `observations.json` records exact outcomes. Ten later
chunk faults, cancellation and close returned no partial logical rows while
preserving the first validated reply. Graph regression tests separately exercise
Parquet publication and preservation of prior per-row results. The new real-model
test returns actual vectors; it does not claim new real Parquet publication.

The normal helper SHA256 is
`44287e5c1f3d535debf3ff71a9d1ee8b821efc4b0dc76be02c35f6405b8a9935`.
Compiler records show ordinary worker/neutral artifacts with `features=[]` and
`profile.test=false`. [run_normal.py](run_normal.py) used private copies of the
15 pinned model artifacts and refs in `/private/tmp/M4 normal layout ü 078hsjfb`.
The archive SHA is `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`;
the private runtime dylib SHA is
`2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`.
Shared source model/ref bytes were verified unchanged after inference. Sibling
and absolute discovery used child-scoped runtime cache configuration; parent
cache environment stayed unchanged.

| Actual model profile | Shape | Maximum reference difference |
|---|---|---:|
| NomicEmbedTextV15 | 3 × 768, finite and ordered | 1.0058284e-7 |
| CodeRankEmbed | 3 × 768, finite and ordered | 0 |
| JinaEmbeddingsV2BaseCode | 3 × 768, finite and ordered | 0 |

[normal-observation.json](normal-observation.json) binds exact revisions, hashes,
pooling/normalization/prefix identities, build profiles and library/symbol
inspection. [normal-batch-actual.json](normal-batch-actual.json) contains actual
vectors, inputs, reference paths, helper events and checked close facts; all
comparisons use tolerance 1e-4. Graph and neutral paths completed eight checked
helper lifecycles including the one-model smoke. Four neutral raw reaped exit-0
statuses are exported; graph checked-close success is recorded separately.
The first real batch-size comparison incorrectly required exact equality;
its arrays and actual 1.02e-7 delta remain in `real-exact-equality-failure.json.gz`
and failed `normal-batched` logs. Test-only tolerance correction passed in
`normal-batched-tolerance`; no production change followed that failure.

Run before review or rebuilding shared artifacts:

```sh
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M4/verify.py
```

The 56-file source/test fingerprint is
`6825ea859167883e642872dd427e423ee20457641eb5aababb33317f3be59824`.
[ROOT candidates](root-post-candidates.json) preserve **all seven original
stage-A requirements and nine negative controls** (ten requests), rebound to
this source and normal artifact, plus actual chunk-capacity and one-byte-short
requests. [Worker numeric POST](worker-numeric-post.json) executed those last
two directly: SAT/pass and expected UNSAT/fail. These are finite numeric checks,
not ROOT Jev receipts, default selection, performance optimality or native safety
proof. ROOT must freshly compile, inspect, execute, persist/reload the complete
stage-A POST and provide packaging PRE before approving this node.

The historical M3 verifier fails its cwd-relative cache projection in this new
worktree; current M4's gate passes. That exact failure was reported and old
captures were preserved. [Signal schema evidence](risk-schema-gap.json) records
the intended rich signal and the exposed schema's missing required fields;
progress requested ROOT mediation without claiming a durable signal. Execution
is macOS ARM64 only; Linux/non-Unicode execution and final shipped CLI/native
inspection, performance/containment and Notebook gates remain explicit limits.
Main/user files, notebooks, graph/CURRENT, external notebook source and global
caches/services were not modified. No installation, publication or push occurred.
