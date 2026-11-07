# G01 rework: original implementation passes; final replay pending

Canonical `bd-bohx == report_audit.task_id`; fresh delegation `52ed8b8342674b60`,
assignee `spur-worker-52ed8b8342674b60`, durable plan/task
`2271b771-c3d1-4438-85c1-dde54b37d58c/G01-generic-evidence-contract` match.
INTENT `909e9182-f63c-4958-8981-0ad85923cd9e` was persisted/replayed as comment
4510 before edits/builds. Checkpoint 4514 preserves transport failures. Keep
the issue **in_progress** for ROOT review.

Four production files remain byte-equal to original `b2bd69cb6`. Rework adds one
test-only serialized request replay and SHA256 source/Cargo/config/executable
capture. There are 18 authored tests; final executed counts require actual logs.
No production redesign, dependencies, parser, artifact-field, IO/cache/runtime,
adapter or schema/extractor changes were made.

## Actual verification

ROOT's completed original-source probe is **exit0, 17 passed, 0 failed**.
`root-original-command.json`, `root-original-generic.log.gz`,
`root-original-export.json.gz` and `root-original-provenance.json` preserve the
actual command, full native fixtures/results, compiler/executable identities,
source comparisons and uncompressed log SHA256. ROOT checkout was only read.
Its five files equal original `b2bd69cb6`; our production files match but the
replay test differs. This is original-source verification, not final replay
success. ROOT evidence was dirty when captured, not relabeled clean. The
sccache `Compilation failures` footer is historical cache statistics.

Own final focused suite, query regressions, all-target check and protected
original test-first attempt each exited **30 before rustc** after both native
180-second rsync attempts: zero executed tests, no compiler RED/GREEN in those
runs. Required local fmt passed at `b8fd45dd4`. The duplicate original probe
was intentionally canceled (`-15`), retaining its exact disconnect log.
Exact argv/env/status/times are in command JSONs and `.log.gz` files. Original
attempt1 exit200 logs, `commands.json` and `source-bindings.json` remain.
No local compile fallback, infra changes, target copying or other-job termination.

Historical PRE passed: fingerprint
`41f981e743d55ff88720be20f606fcb1ad818c25b72f709598d4144c85422d84`,
729 bindings/19 cases/13 false claims/eight current + eight retired predicates.
Retry used **no --current-source**. Both complete live envelopes were fetched
with pin omitted and match saved `reloaded` objects. All68 historical base files
were checked with `git show 87af0db9:path`; all5 carried-over source hashes
matched before rework. See `retry-entry-verification.json`,
`retry-pre-verification.json`, `retry-live-pre.json.gz`. No new solve/pin/POST.

## Test-only native replay

`replay_serialized_contract_request` deserializes graph, inventory, extensions
and filter, then calls actual integrity/checksum/manifest/capability,
registry/default-query and inventory operations. Default execution asserts a
real positive request plus finite synthetic mutations. `G01_REPLAY_EXPORT=`
contains full inputs/results and exact compact typed checksum/digest preimages.
No production test API. Select an independent supplied request through Cargo:

```sh
scripts/spur-cargo test --config 'env.G01_EVIDENCE_REQUEST="docs/research/artifacts/2026-11-07-generic-graph-evidence/G01/worker/supplied-positive-request.json"' -p spur-graph --test generic_evidence_contract -- --nocapture
```

Use the exact remote environment in `run_command.py`. The supplied positive
and negative requests derive from ROOT's actual synthetic fixture outputs;
they are inputs, not claimed replay results. Negative sets a nonnullable cell
to Null. ROOT can independently mutate any field. Synthetic contract fixtures
are never collector/disk/artifact/cache observations.

`reload_packet.py` refuses missing exports, requires actual passing summaries
and final compiled-source SHA256 equality, then saves full native request/results
and independent payload/null inputs. Its refusal is in `retry-packet-status.txt`;
no final packet was fabricated. Original docs/extractor are preserved as
`attempt1-README.md` and `attempt1-reload_packet.py`.

Original protected lineage: `b543a7fa1` -> `b2bd69cb6` -> `d9373fe3e`.
Current checkpoints: `b7470d77a` tests, `59a1fa6c4` unchanged production,
`b8fd45dd4` replay. `retry-source-bindings.json` binds committed source bytes
and local Cargo/config/wrappers separately from ROOT native identities.

## Archives and remaining verification

Original whitespace-bearing `fmt-initial.txt`, `source-only.diff` and
`full-review.diff` are exact `.gz` archives, with uncompressed hashes in
`retry-entry-verification.json` and `retry-archives.json`. No output was edited
to appear green. Final diffs use distinct compressed names, excluding diff
archives themselves to avoid recursion.

Remaining: final replay/default and supplied-request execution; genuine
missing-API compiler RED at protected `b543a7fa1`; passing query/all-target
checks; full final-source native packet. ROOT owns actual-output POST,
independent mutations, approval and later phases.
