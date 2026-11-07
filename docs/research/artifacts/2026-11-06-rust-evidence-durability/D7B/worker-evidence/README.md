# D7B native query evidence

Canonical issue: bd-2bur; delegation 1248d718b56c44a2; dispatch base deef4ad0aae9ed129b8c72b56b74bcd87cb4ba5b. Implementation source: 98ffa5e64dd6252f929b8a33bdab7a47432ea0dc. ROOT owns fresh Jev POST, approval and integration. D8–D10 remain outside this change.

The query implementation and replay packet are ready for independent review. Completion audit state is recorded in the completion receipt files. Worker MCP audit, issue and progress calls stopped responding during final packaging. Entry INTENT comment 4197 was persisted and replayed; successful progress through comment 4230 and the risk signal are recorded in beads. `audit-transport.json` preserves the unconfirmed progress UUID, and `completion-audit-request.json` preserves the exact completion request for an idempotent retry. No authorization rejection was received. The worker has made no close, approval, merge or push operation.

The implementation validates the complete recorded envelope and actual source/owner context before selection, caches the native physical source domain even without an evidence carrier, and forwards explicitly through `&T`. The disk helper freezes five compressed tables, validates their shared checksum and graph projections, drops full decoded query rows, then prunes immutable bytes by file path. It retains whole selected file groups and referenced whole origin inventories. Public fields and availability variants are preserved. The original eight-test file remains SHA-256 `09ffb8a2c2beb09a116dc5e9411a6777cdf4bd2b3751aae2e8c967473eae779e`.

| Verification | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: |
| Original query target | 8 | 0 | 0 | 0 |
| Dedicated D7B target | 6 | 0 | 0 | 0 |
| Query-client parity | 7 | 0 | 0 | 0 |
| Query-client unit selection, library | 26 | 0 | 0 | 566 |
| Macro target | 13 | 1 | 0 | 0 |
| Exact failing macro case on accepted predecessor | 0 | 1 | 0 | 13 |

The full `query_client::tests` command also visits 115 filtered integration targets: aggregate filtered count 1,428, zero additional executed tests. `summary.json` retains every target count. Graph all-target check, scoped local formatting and complete diff checks exited zero. Existing dead-code warnings remain in raw logs. This is no claim of full project test coverage.

The macro failure is `parquet_macro_queries_reject_missing_source_manifest_rows`. Its fixture removes a source manifest and attempts to persist the resulting artifact; existing Rust stored validation rejects the stale graph content hash before the macro query runs. The same failure was genuinely reproduced on all 803 exact accepted-predecessor source inputs, verified again on the builder with a native executable hash. `predecessor-macro-{command,identity}.json` and the complete log establish the baseline. This is separate from bd-16eq. Risk signal b35256fe-3485-4555-ab16-055482952017 requests ROOT attention; no unrelated fixture or assertion was changed.

`implementation.patch.gz` contains the exact unified source/test diff (`gzip -dc implementation.patch.gz`). TDD history is retained in `commits.json`, `run-source-bindings.json.gz`, `red-native-identity.json` and complete compressed logs. The original seed reached runtime RED before functionality: 0 passed, 1 failed, 7 filtered, exit 101. New regressions were committed at 66a79eac9. The freeze assertion was strengthened at 9c7e2e71b after an equality-only version passed on two unavailable results; its meaningful assertion then reached RED. The implementation at 085220b4a had a decoder-diagnostic compile error; 81ded1ea6 corrected it. The native replay hook is 3e195e9d1. Runtime regressions then exposed wrong edge projection column names; 98ffa5e64 uses actual `source_stable_id`/`target_stable_id` columns. All attempts remain visible.

All builds used `scripts/spur-cargo`, remote execution, no local fallback, disabled fresh-output capture and the explicitly bound cloud helper. Two uploads exhausted the default 60-second inactivity timeout while waiting for source admission. The successful retries used the wrapper's existing per-run `SPUR_RSYNC_IO_TIMEOUT=180` option, based on the observed 99-second source-phase wait. No infrastructure, shared locks, other jobs or shared targets were changed. Source stayed fixed during each upload; baseline snapshots were automatically restored before final checks. Logs and intermediate packets were kept outside the checkout until runs ended.

`native-packet.tar.gz` preserves original core/Parquet artifacts, raw fixture bytes, 20 ordered native requests and exact memory/disk/frozen/cold results. It includes call Unknown, invalid UTF-8, healthy zero, legacy, declaration-only Unknown, present-empty and absent-empty profiles. The declaration-only Unknown profile is an explicit stored-envelope test variation; syntax authentication remains a separate API. A bound physical notebook-like filename appears in the native domain as unavailable because the accepted collector has no evidence file for it; it wins over the virtual-spelling classification without fabricating evidence.

The native IO unit fixture reads five evidence files totaling 23,988 compressed bytes at first successful query. Its counters are `[0,0]` before, `[5,23988]` frozen and after replacement/source edit, `[10,24023]` after a separate corrupt cold client, and unchanged after deletion plus another existing-context query. Counters instrument successful file-backed `get_bytes` reads at snapshot loading. Thus the existing context adds zero evidence reads; the cold client's five reads are kept distinct. These observations impose no performance quota.

`independent-replays/` contains 12 source/executable-bound native invocations: filtered-out body and source corruption, wrong existing OID, rehashed owner substitution, warm then corrupt, six D5-backed groups and an independent IO replay. Driver requests and complete stdout are retained. `d5-native-inputs.tar.gz` preserves the exact D5 persisted inputs used by PRE, raw source bytes and a documented carrier-removed legacy variation. Nineteen D5-backed observations plus the original native custom-default control match the 20 prospective PRE result values after availability spelling normalization. The virtual spelling observation uses the D5 baseline graph; a known notebook-owner fixture remains an independent ROOT replay input. `d5-native-observations.json` records this context explicitly. These native observations are inputs for ROOT POST, not a worker-authored POST receipt.

Run the packet verifier from any directory:

```sh
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7B/worker-evidence/verify.py
```

`source-bindings.json` binds 804 graph/config inputs and the source commit; `native-identity.json` verifies the same inputs on the builder and records target, toolchain executable, native ELF and config hashes. The three complete live PRE envelopes are retained and compared exactly with the archived envelopes. The verifier does not rerun historical source-bound PRE derivations against modified code.

ROOT can run arbitrary independent mutations using an absolute JSON request on the builder:

```sh
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7B/worker-evidence/replay.py \
  --request /absolute/remote/request.json \
  --remote-output /absolute/remote/fresh-output \
  --capture /absolute/local/capture
```

The request has `artifact`, `disk_dir`, `filters`, and optional `before`/`after` mutation arrays. The hook copies the supplied disk before mutation. Supported operations are `replace_string` (`table`, `column`, optional `file_path`, `replacement`), `rehash_replace` (`old`, `new`), `write` (`table`, `text`) and `delete` (`table`). Supplied cases export actual outcomes without assuming memory/disk equality. `independent-requests.json` and `d5-replay-requests.json` contain executable examples. Omit `--request` for the generated fixture/lifecycle packet; use `--io` for the IO unit replay. The driver checks source and executable hashes before and after execution. A rebuild is needed if the recorded builder/executable is unavailable.

Stored integrity validation does not authenticate mutable current source bytes, prove complete panic detection or establish call-target correctness. Full syntax/source authentication remains the existing explicit API. ROOT still needs fresh actual-output POST and independent review, including disposition of the demonstrated macro baseline failure and the completion-audit receipts.
