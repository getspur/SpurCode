# S3 root review: native lifecycle and startup errors

Reviewed candidate `65189c90c146bc6bc3d972f5b88b076b729ef19c`, delegation `b94c48a20039451c`, assigned issue `bd-2hdm`. Test commit `9fb799554` precedes fix `7b1a825a4`; authored history is protected by `spur/review-evidence/embedding-s3-attempt2-authored`.

The normal helper validates Hello before native startup. Startup failure now returns a correlated, byte-bounded Unavailable frame and an unsuccessful process exit. It does not claim Ready or Closed after initialization fails.

| Independent verification | Result |
|---|---|
| Native/protocol suite | 6 passed |
| Protocol/client suite | 32 passed |
| Normal build and scoped formatting | Exit 0 |
| Normal startup failure, identity limits 1 and 256 | Correlated Error, raw wait 256 (exit 1) |
| Healthy S3 scaffold at both limits | Ready, product Unavailable, Closed; exit 0 |
| Actual fixture inference | Three runs produced 1,4,9,16,25,36 and exited 0 |
| Retained native output | Checked shutdown refused while retained; succeeded after drop, exit 0 |
| Active close | Admission closed, job joined, checked shutdown succeeded, exit 0 |
| Deliberately killed helper | Raw wait 9 / SIGKILL 9, preserved as abnormal termination |
| Catalog POST | 6 complete models passed; 16 corrupt controls rejected; all 22 receipts reloaded |

Five Jev interpretations opened. The native-results interpretation is **blocked at mode confidence** and is preserved verbatim. That complete, unambiguous finite model was checked separately with an explicit catalog verify request; no confidence threshold was lowered and the blocked result was not rerouted by rewriting its intent. This is not six open Jev gates.

`post-proof.json.gz` contains all Jev inputs/responses, exact family requests, SMT, solve results and persisted reloads. `project_post.py` derives observations from raw framed bytes, real fixture outputs, source hashes and raw process waits against separately declared finite policies. Missing rows, duplicates, stale sources/runtime, invented clean exits, early release and missing output drop are rejected.

The native runtime SHA256 is pinned to `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`. `native-observations.json` binds ten reviewed source files and the two independently rebuilt binaries. The earlier worker hashes matched before rebuilding, as recorded in `inherited-checks.json`. The dependency tree has no graph, analyst or database dependency.

Four numeric native owner counters were **not measured**. The evidence is checked Busy/success, source review of the actual managed release API, real inference and raw exit statuses on these concrete macOS ARM64 paths. A session-handle-drop event does not assert physical native session release. The loader guard catches only the wrapper's Rust loading panic before environment creation; native aborts remain process failures.

S3 intentionally returns Unavailable for product models. Real product loading, artifact identities and finite cache ownership belong to S4. Host migration, default CLI PGQ acceptance and packaging are still pending.

Recheck recorded evidence from this checkout:

```bash
python3 docs/research/artifacts/2026-11-03-embedding-isolation/S3/final-review/verify.py
```

Exact build/test invocations and full compressed logs are in `verification.json` and sibling `*.log.gz`. Local execution used `SPUR_REMOTE=0`, `TMPDIR=/private/tmp`, offline Rust dependencies, the isolated shared target `/private/tmp/rust-gap-g1-acceptance-target`, and the pinned `SPUR_ORT_ARCHIVE` from S3's native proof.

