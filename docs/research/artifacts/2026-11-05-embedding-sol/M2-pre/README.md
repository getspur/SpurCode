# M2 entry gate: shared local helper policy

Root prepared and executed this PRE before approving M1. Canonical task: `bd-7c53`, plan task `shared-policy` in `dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`. Use the existing explicit routing: `agent=codex`, `profile=codex`, `model=gpt-6.1-sol`, `effort=xhigh`.

Before implementation or builds:

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M2-pre/verify_pre.py
```

The 40-file source fingerprint must be `baafa46271a6b66399c914df92d3b557add71cd93cb2312dfb16d743a5d49ae3`. Reload these six current receipts through `get_solve_result` and compare statuses and exact archived requests. The verifier checks complete persisted lowerings against an independent finite/arithmetic oracle, including variable domains, known fixed values, all request/limit pairs and the aggregate cold phases.

| Case | Receipt | Expected |
|---|---|---|
| Complete declared defaults | `sol_0b9080503f4c4314` | SAT |
| Captured demand and composed phase budgets | `sol_be2b367b69404ad6` | SAT |
| Old uncomposed first-request timeout | `sol_e1c38cd239134016` | UNSAT |
| Absent required default value | `sol_37fd236f0d6c481b` | UNSAT |
| Largest model budget one byte short | `sol_6d6a5bd1ace940e9` | UNSAT |
| Composed deadline one millisecond short | `sol_3f60773716dc4ecf` | UNSAT |

Both Jev1.14.0 interpretations opened at weakest confidence0.98, with exact supplied facts, verification mode and no unknowns. Root completed the inspected catalog bindings: 14 finite default-value memberships, 14 cardinalities, one request-within-limit binding over 20 separately unit-bound subjects, and one aggregate-capacity binding over the two millisecond cold phases. Across six requests, all 90 compound catalog predicates match the independent oracle. A malformed synthetic absence was first rejected by request validation; the corrected `present=false,value=null` case produced the recorded UNSAT, not a validation error.

Implement the exact configurable default proposal in [policy.json](policy.json), preserving the existing explicit JSON override format:

| Setting | Default | Basis |
|---|---:|---|
| max_texts | 64 | Existing graph default batch |
| max_text_bytes | 65,536 | Reviewed per-item UTF-8 admission budget, reused from the measured experiment |
| max_total_text_bytes | 4,194,304 | 64 × 65,536 |
| max_frame_bytes | 33,554,432 | Smallest binary capacity covering the computed25,168,231-byte fully escaped request |
| max_dimensions | 768 | All three registered model profiles and captures |
| max_identity_bytes | 64 | Maximum current identity field length, including SHA256 hex |
| max_pending | 1 | Existing serial helper |
| artifact_budget | 644,080,771 | Largest exact five-file total among the three captured revisions |
| queue_capacity | 1 | One queued caller in addition to one active call |
| max_restarts | 0 | Explicit retry remains caller-controlled |
| startup_timeout_ms | 120,000 | Existing real-fixture phase allowance; covers captured descriptor/Hello times |
| request_timeout_ms | 240,000 | Hello120,000 + first inference120,000 under the same request deadline |
| shutdown_timeout_ms | 30,000 | Existing real-fixture cleanup budget |
| shutdown_poll_interval_ms | 20 | Existing cleanup polling interval |

These are explicit, configurable operating choices and checked relationships. The solver does not uniquely select a product policy. The per-text admission budget does not follow from an8192-token limit, and the4096-byte body eligibility test does not bound headings, symbols or queries. The32MiB frame includes control-byte escaping that the earlier16MiB experiment did not cover at its full4MiB raw allowance. Actual serialized-size checks remain mandatory for later batching and explicit overrides.

The fresh read-only inventory hashes15 cache files. Model totals and revisions are Nomic548,026,095 (`e9b6763023c676ca8431644204f50c2b100d9aab`), CodeRank548,431,020 (`151669b173750250e611e1be00c812f112ea6020`), and Jina644,080,771 (`516f4baf13dec4ddddda8631e019b5737c8bc250`). The default artifact budget is a byte-admission cap for these revisions, not an RSS bound or guarantee for future revisions. Larger future artifacts/identity fields need a supported explicit override and must fail clearly rather than bypass validation. No fresh native inference is claimed here; timing inputs are exact historical captures bound by hashes.

Scope: add one validated neutral policy/configuration module in `spur-embedding`; replace only the analyst `Timeouts::from_env` configuration adapter/projection. Policy reading must not discover/launch a helper or initialize native/model state. Preserve the JSON override's required fields, unknown-field rejection, zero/representational/deadline/queue-overflow checks and any currently valid independent text/total/frame caps. Overrides need not equal or exceed defaults. An absent override uses defaults; an invalid supplied override fails. Preserve Off/Auto/Sidecar and external socket behavior. Do not modify `State`, jobs, publication/completion, cancellation, shutdown, native ownership or the reviewed helper. Graph consumes the policy in M3, so helper discovery is not an M2 blocker.

Use the already committed default-local-policy assertion RED, then meaningful configuration/validation regressions, a fix commit and scoped GREEN. Do not duplicate tests solely for counts. Keep process environment changes child-scoped. Preserve all historical evidence and local main/user notebooks. After implementation, export the actual policy values with new source identities and POST negatives to `docs/research/artifacts/2026-11-05-embedding-sol/M2/`; root supplies a fresh actual-output Jev POST and M3 PRE before approval. Do not run the old M1 PRE's current-source checker or require its now-retired live receipts; their exact historical records remain committed at `c4253c5b7`.
