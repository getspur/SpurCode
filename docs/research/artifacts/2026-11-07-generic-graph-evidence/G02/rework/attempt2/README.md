# G02 attempt2 repair and native handoff

The repair preserves authoritative query ownership and unavailable coverage
without applying complete stored-capture rules to a validated query projection.
Full stored integrity, typed validation, exact reconstruction and original
saved-source validators remain active. Collector eligibility is unchanged.

Canonical issue: bd-ode3; runtime delegation: bb0a553fc28143a5;
assignee: spur-worker-bb0a553fc28143a5. The issue remains in_progress for ROOT.
Fresh INTENT dd023bed-9109-464f-95a5-acdc21471e07 persisted and replayed
idempotently as comment 4603 before the explicit fast-forward to 23705a89f.
There was no subdelegation, approval, close, integration merge or push.

Test checkpoint: a364e1a54. Minimal production fix: d863c8d9e.
The existing genuine ROOT RED was authenticated rather than rerun unchanged:
256c6c6ea has 1 pass/2 fail/0 ignored/10 filtered. All three final ROOT regression
function bodies remain byte-identical. The older contradictory physical-path
assertion is corrected. Current-cohort golden copies change only graph
manifest_version (empty/nonempty) and nonempty set origins[0].manifest_version;
both historical and current values are explicitly asserted. Every other exact
comparison and original forgery/mutation loop still executes. Frozen archives
and ROOT-review evidence are unchanged.

The repair PRE passed --current-source before edits: fingerprint
e3eb81c395a6fc90ed0c9d584cd0e05482f8f20d1c3c10a6e459fe04783c0694,
84 bindings/8 cases/9 false controls/33 variables, Jev open 0.96. Only current
sol_803d23cd91fb4603 and sol_b175348537b343ed were reloaded, pin omitted;
both full envelopes equal the archived reloaded objects. Their fresh objects
are retained in pre-reloaded.json.gz. Historical PRE was authenticated again
after edits without --current-source. This is policy PRE, not actual-output POST.

| Verification | Actual result |
| --- | --- |
| generic_evidence_contract | 18 passed, 0 failed/ignored/filtered |
| generic_evidence_providers | 13 passed, 0 failed/ignored/filtered |
| macro_invocation_artifacts | 8 passed, 0 failed/ignored/filtered |
| rust_evidence_contract | 18 passed, 0 failed/ignored/filtered |
| Remote all-target check |exit 0; existing overlay bench warnings |
| Standalone native retry | 13 passed, 0 failed/ignored/filtered |
| Local full fmt check |exit 0 |
| Complete diff check from plan base |exit 0 |

Commands, argv, environment, source SHA256/length bindings, HEAD and exact
raw-log hashes are in the matching JSON receipts. All builds used the existing
remote wrapper with full source sync and no local fallback, sequentially on
i-0d81ed4c6240d4317. Local formatting uses scripts/spur-cargo. G02-repair.patch
is the full unified source diff against the recovered ROOT checkpoint.

Fresh native replay consumes native-request.json through native-config.toml;
the request is byte-identical to the original worker input. The standalone
target avoids multi-target stderr interleaving. native-replayed.json.gz is
actual native JSON extracted unchanged from native-retry.log.gz, with no
removed bytes. It exports actual graph, original typed sets, extension tables,
decoded sets, opaque/unsupported statuses, source-bound mutation inputs,
native compact preimages/digests, Cargo/compiler/executable/config identity
and 25 compiled-source bindings, including rust_evidence_contract.rs.
Authentication confirms 3 providers, 11 original independent mutation cases,
complete typed roundtrips and distinct stored-vs-source rejection boundaries.
No Python BLAKE3 or fabricated native outputs were used.

An earlier standalone native run exited 101 without any captured assertion
summary or JSON. Its unchanged raw log and receipt are native-replay.*.
Concurrent local ENOSPC prevented a shell temporary file and a beads signal
write (Failed to write holder payload into write lock at
/Users/kevintruong/project/SpurCode/.beads/.write.lock). Disk later recovered
to 2.8 Gi without worker cleanup. Queue orphaned_at errors are also retained.
The native failure's cause is unconfirmed; it is not an assertion RED.
Checkpoint 4608 records the failure and uncertainty. The unchanged retry after
disk recovery passed and exported the authenticated packet. The lease-timing
risk was reported in comment 4605 with a durable typed signal; the canonical
lease was extended. No lock/cache/target deletion, token reset or limits change
was performed.

Replay authentication:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/rework/attempt2/verify_attempt.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/ROOT-review/rework-pre/verify.py
```

ROOT owns fresh actual-output Jev POST, independent review and local integration.
No G03, generic storage/backend/cache/runtime acceptance or final approval is claimed.
