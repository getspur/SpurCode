# D8 empty-capability repair handoff

Canonical issue `bd-3eyg`, delegation `657667b3cc3a4f3e`, plan
`ad663d7c-9434-46f1-ac9c-63b95c499a81`, task `D8-empty-capability-repair`.
Worker evidence is ready for ROOT's actual-output solver POST and review.
The canonical issue remains `in_progress`; this packet is not approval.

Production commit `47d9619acadf3c1f276552a73355f1db0d5887a2` adds eight lines
only to `crates/spur-graph/src/rust_evidence/query.rs`. When the resulting
known domain is empty and both input availabilities are `Unavailable`, the
aggregate stays `Unavailable` before evaluating the vacuous `all(Complete)`.
Captured empty coverage still returns `Complete`. Nonempty domains, unknown
custom domains, group replacement, deletion, original OID validation and
whole origin retention keep their existing paths. No public carrier or
availability policy is added. The query-client seam is unchanged.

Both ROOT tests are byte-identical to regression commit `ac713352e75f71936a157ac52f5ccd4921cbf9e5`.
The complete original 12-test D8 file and its seed from `0d3072d8e` are
byte-identical too. The existing compiled ROOT assertion RED (1 passed,
1 failed, 0 ignored, 0 filtered, exit 101) was reused as instructed. No new
cold RED or test rewrite was performed. `production.diff.gz` is the exact
repair diff; `implementation.diff.gz` also includes the authorized metadata
recovery from dispatch base `b264c7add` through `287d3020d`.

Entry PRE verified fingerprint
`240cc8548007f0f46214d457fca854e556a5d1e45db18f4be7cdbb3826c5070b`,
2,541 bindings, six independently checked predicates and 68 case payloads.
Both complete current envelopes, `sol_b33fce6d9f5b4d6f` (SAT/pass intended
contract) and `sol_4e43fd28124942d0` (UNSAT/fail counterexample), were reloaded
and exactly compared to the archived PRE. INTENT UUID
`2677c669-ff37-466a-8a54-a2c6505b71f9` was persisted as comment `4347` and
idempotently replayed before the recovery fast-forward, source edit or build.
Focused GREEN and native progress audits `4348` and `4349` were also persisted
and replayed. Exact requests, responses and PRE envelope copies are under
`audit/`. The prior attempt's wrong-target request remains preserved unchanged
in `dispatch-recovery/`; it was not reused as this worker's audit.

| Required executable / filter | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: |
| `rust_evidence_empty_domain_root_review` | 2 | 0 | 0 | 0 |
| `rust_evidence_overlays` | 12 | 0 | 0 | 0 |
| `overlay_client` | 12 | 0 | 0 | 0 |
| `macro_invocation_queries` | 13 | 1 | 0 | 0 |
| `mcp_overlay_notfound_retry` | 4 | 0 | 0 | 0 |
| `rust_evidence_queries` | 8 | 0 | 0 | 0 |
| `rust_evidence_d7b` | 6 | 0 | 0 | 0 |
| `rust_evidence_pruning_d7b` | 3 | 0 | 0 | 0 |
| `rust_evidence_pruning_root_review` | 1 | 0 | 0 | 0 |
| `query_client::tests` | 26 | 0 | 0 | 566 |

The macro target is **not GREEN**. Its sole failure is the preserved
`parquet_macro_queries_reject_missing_source_manifest_rows` baseline
`bd-xb5p`: `stale graph content hash` before the macro query. Exact predecessor
artifacts and hashes are referenced by `known-macro-baseline.json`; no other
test failed. The combined regression run used `--no-fail-fast` and exited
101. The focused and unit wrappers exited 0. Graph `check -p spur-graph
--all-targets`, local scoped formatting and full diff checks exited 0.
All-target warnings are preserved in its full log; no warning cleanup was
included.

All builds used `scripts/spur-cargo` with the prescribed full-sync,
no-local-fallback environment and explicit escalation. Each run's complete
source hashes, Git revision/status, command, environment, start/end and exit
are under `checks/`; source and Git stayed immutable during every wrapper
run. Logs were kept outside the checkout until each run finished. The first
GREEN provider attempt had an SSH banner timeout; the wrapper's configured
retry recovered. That transport event is not an assertion result. The normal
builder layout used this worker's own target; the older ROOT target was not
copied, linked, unlocked or changed.

The new executable SHA-256 is
`86229db1ac56849ab547e6140d1f9571b49310f43306144e1c666960df8b62ea`.
Its capture asserts that the executable timestamp follows this build's start,
checks source hashes before/after execution, and exports current compiler,
config and executable identities. `native-identity.json.gz` binds 2,542 files
including the ROOT regression. Compilation occurred with the repair as the
only dirty source at local HEAD `287d3020d`; the verifier independently binds
those exact compiled bytes to clean production commit `47d9619ac`. The old
source epoch or historical executable is not used as proof of this fix.

`native-packet.tar.gz` contains 17 fresh cases, real base/delta artifact JSON,
separate raw source bytes, both Parquet directories and full memory/disk/
reloaded outputs. Fresh capture passed 12/0/0/0. All 68 original payloads match
ROOT's prior fresh capture exactly; 423 observations independently satisfy
whole-file and whole-original-inventory selection. `matrix-packet.tar.gz`
reruns ROOT's 15 unchanged arbitrary-input requests on the new executable:
15/0/0/165, with all 300 non-counterexample outputs exactly preserved. Requests
and input archives are inputs, not substituted expected outputs.

| Empty-domain control | Required and observed |
| --- | --- |
| Both carriers absent, identity | `Unavailable` |
| Both carriers absent, only README shadowed | `Unavailable` |
| Captured empty carriers, only README shadowed | `Complete` |

`empty-controls.tar.gz` additionally materializes the same real capture inputs
with explicit absent/captured carriers in base/delta JSON files, separate empty
raw source directories, and **no request mutation fields**. All three are
replayed through the delivered entry point: 3/0/0/33. Source authentication is
recorded separately: captured empty inputs authenticate; legacy absent carriers
report evidence unavailable. `replay-smoke.tar.gz` preserves a separate
entry-point smoke invocation, 1/0/0/11. Across all 30 test executable
invocations: 118 passed, 1 known baseline failure, 0 ignored, 775 filtered.
Cargo check executes no tests.

To verify the finite packet and regenerate actual-output POST candidates:

```bash
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D8/empty-capability-repair/verify.py \
  --write-candidates /private/tmp/d8-empty-repair-post.json
```

`post-candidates.json.gz` retains complete actual observations, original group
and whole-origin digests, byte/OID bindings, all three empty results and three
independently tainted output controls. ROOT still owns fresh solver POST.
The verifier does not run historical PRE against changed source. Historical
ROOT evidence can be checked with its `verify.py` **without** `--current-source`.

Extract either input packet into a private directory, supply artifact/source/
Parquet paths and original filters in a request JSON, then run:

```bash
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D8/empty-capability-repair/replay.py \
  --request /private/tmp/independent-request.json \
  --capture /private/tmp/independent-observation
```

The entry point uses this packet's compressed current identity and the
preserved arbitrary-input native hook. It supports old sites/OIDs/carriers/
origins, JSON mutations, source directories and independent disk directories.
Explicit custom backend projections remain trusted input, separate from
source-authenticated stored artifacts. Rebinding such a projection is not
proof of stored-artifact corruption. Builder `i-083233d863b806894` is only an
availability hint; `--instance` can select the current builder, with all identity
checks retained. No cfg activation, macro expansion or call-target correctness
claim is made.
