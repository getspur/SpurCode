# Integrated issue reads: S7 acceptance evidence

Validated production commit `4cbd4f044` on approved S1–S6 snapshot `6c41c4e581862d293d94727935abc654b06f28e7`, in the orchestrator-created S7 worktree, for **bd-a96om**. The combined implementation passes **497 final tests**. Strict clippy passes for the touched production libraries with `--no-deps`; the broader attempt fails on an unchanged dependency warning. Formatting passes. Two TDD cycles remove measured recurring validation/serialization work while preserving both feed validations, integrity/replacement checks, existing query behavior, independent cursors, and derived retention bounds.

The initial optimized benchmark found a real regression despite zero payload-row loads. After the fixes, the representative admitted open query is about **31% faster** than equivalent direct summary SQL in this fixture; small open requests are approximately equal. Oversized results remain uncached and slower, and the feed adds write overhead. These are synthetic Rust measurements, **not live SPUR CPU/PID improvements**. The original **208/402** profile numbers are non-wait-classified observations, not invocation counts or CPU percentages. No live binary was installed/restarted. The worker did not operate Notebook MCP; the coordinating agent independently analyzed samples and solver cases in Python through [Notebook MCP](../../../spur-64019-performance-20261009.ipynb).

## Review artifacts

- [S7 unified diff](s7.patch): the four Rust files changed by this worker.
- [Combined S1–S7 code diff](integrated.patch): proposal commit `f54376d9d` through `4cbd4f044`, restricted to Cargo.lock, spur-pm and spur-core.
- [Exact commands, environments, exit statuses, durations and raw-log hashes](commands.json); complete logs are preserved in [logs](logs/), including failures.
- [Bounded measurement summary](measurement-summary.json), [baseline samples](baseline-samples.json), [first-fix samples](first-fix-samples.json), [final samples](final-samples.json), and [small/large work counters](work-counters.json).
- Reproduce statistics with `python3 docs/performance/2026-10-09-issue-reads-implementation/summarize.py`. No Notebook access is needed. Each read stage contains 248 request samples: 124 alternating-order pairs across four fixture/query combinations.
- [Exact catalog and typed POST requests and receipts](solver-post.json). Worker verification used the solver directly. The coordinating agent separately completed [S7 Jev compilation, solver execution and Python cross-checks](../2026-10-09-spur-64019/implementation-tdd/s7-jev-overhead-policy.json).
- [Preserved prior TDD refs](prior-reviews/preserved-tdd-refs.json), [approved S5 attempt 2](prior-reviews/s5-attempt-2-review.json), [approved S6 attempt 2](prior-reviews/s6-attempt-2-review.json). S5 attempt 1 required changes; it was not the approved implementation.

The 15 preserved raw logs match their recorded SHA-256 hashes and original `.spur/tmp/bd-a96om` copies. The statistics reproduce byte-for-byte, and both patch artifacts exactly match `git diff` for their stated ranges. Source whitespace validation (`git diff 6c41c4e581862d293d94727935abc654b06f28e7..4cbd4f044 --check -- crates/spur-pm crates/spur-core`) exits 0. The staged evidence check also exits 0 when excluding `*.patch`; the unrestricted check exits 2 only because literal blank unified-diff context lines contain their required space prefix. Those patch bytes are preserved.

## TDD and verification

S7 commits, in order:

| Commit | Intent and evidence |
|---|---|
| `da37a9019` | Test/diagnostics: warm request must avoid regenerating/comparing exact DDL and serializing unchanged retained payload; schema changes must still validate. RED: **2 DDL validations vs expected 0**, exit 101. Adds manual measurement harness and structural-edge acceptance coverage. |
| `f787530c0` | Cache exact DDL validation by connection/schema cookie; reuse unchanged boxed snapshot bytes. GREEN: **157 passed**, 3 manual tests ignored. |
| `81d17734f` | Test/diagnostics: equal validated revision must not execute delta SQL. RED: **2 delta queries vs expected 0**, exit 101. |
| `4cbd4f044` | Skip empty delta queries after integrity/epoch checks; reuse the single fixed clock statement. GREEN: **78 passed**, 2 manual tests ignored. |

The diagnostic events in the RED commits only count existing work; they do not implement the optimizations. Both RED gates are assertion failures from compiled tests. Each test commit follows its observed RED; each implementation commit follows its GREEN. The manual benchmark uses timings as evidence, never CI thresholds.

All remote commands used `SPUR_NO_LOCAL_FALLBACK=1 SPUR_RSYNC_IO_TIMEOUT=300`. Final evidence runs also use `SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0` to preserve complete output. Clippy additionally uses `SPUR_REMOTE=1`. Formatting is local through the wrapper.

```sh
# RED 1: exit 101; GREEN 1: exit 0
scripts/spur-cargo test -p spur-pm s7_unchanged_requests --lib -- --nocapture
scripts/spur-cargo test -p spur-pm beads_crate:: --lib -- --nocapture

# RED 2: exit 101; GREEN 2: exit 0
scripts/spur-cargo test -p spur-pm s7_unchanged_validated_revision --lib -- --nocapture
scripts/spur-cargo test -p spur-pm issue_ --lib -- --nocapture

# Baseline read + write benchmarks: exit 0
scripts/spur-cargo test --release -p spur-pm s7_ --lib -- --ignored --nocapture --test-threads=1
# Final read benchmark: exit 0, complete streaming output
scripts/spur-cargo test --release -p spur-pm s7_request_benchmark --lib -- --ignored --nocapture --test-threads=1

# Final suites: exit 0
scripts/spur-cargo test -p spur-pm -- --nocapture
scripts/spur-cargo test -p spur-core plan::reconciler --lib -- --nocapture
scripts/spur-cargo test -p spur-core --test reconciler_tick --test reconciler_shutdown --test reconciler_late_enable -- --nocapture
scripts/spur-cargo clippy --no-deps -p spur-pm -p spur-core --lib -- -D warnings
scripts/spur-cargo fmt -p spur-pm -p spur-core -- --check
```

| Final check | Result |
|---|---|
| spur-pm, all default targets and doctests | 298 unit + 54 integration passed; 5 ignored; no failures |
| spur-core plan::reconciler library tests | 103 passed |
| reconciler_tick / shutdown / late_enable | 40 / 1 / 1 passed |
| Production library clippy, `--no-deps`, PM + core | Exit 0 with `-D warnings` for selected libraries |
| PM + core formatting | Exit 0 |

The five default-suite ignores are the existing manual index-write benchmark, the two new manual benchmarks, live_smoke, and watermark_scan_perf. Both new manual benchmarks were executed separately; the new write measurement isolates status-index and feed cost. Non-default madsim feature targets and a broad workspace/release-core suite were not run.

Two operational limitations are preserved, not converted into passes:

- The first final core-unit attempt exited **255** while compiling `spur-core`, without a Rust diagnostic or test verdict. Read-only SSH inspection found no compiler/linker for this worktree; the same remote command then passed 103 tests. No shared builder state was killed or modified, and no local fallback was used. See `final-core-unit.log` and `final-core-unit-retry.log`.
- The command `scripts/spur-cargo clippy -p spur-pm -p spur-core --lib -- -D warnings` exited **101** on the unchanged `crates/spur-graph/src/mcp/mod.rs:908` unused `FileOidMatch::as_bool`. The scoped `--no-deps` command passed. Dependency-inclusive strict lint remains a baseline limitation; no warning suppression or unrelated source fix was introduced.

One final benchmark execution exited 0 but the wrapper's unchanged-checkout optimization printed only its last 30 lines. Its incomplete sample set was excluded from the final statistics. `final-release-stream.log` is the complete rerun with the supported streaming setting.

Prior accepted chains (original histories preserved in the linked refs, independently of squashed plan captures):

| Stage | Test commit(s) → accepted implementation |
|---|---|
| S1 index | `a63244b88`, `6e6652bd3` → `b3f07ee13` |
| S2 tracing | `aa4706355` → `1d1a17d93` |
| S3 projection | `64f716b60` → `9f3eac7d4` |
| S4 final review fix | `f2ad281df` → `f7b8456bd` |
| S5 final review fix | `039aeb470` → `1a569e5fc` |
| S6 final review fix | `4900ddad2` → `f99fe3df5` |

S6's original RED commits `524d77ce4` / `50a855c8e` remain under the attempt-1 ref. Its final captured tree `6c41c4e58` equals `f99fe3df5`; the retry baseline `2ce955ed2` equals the prior attempt's implementation tree. Prior histories/review receipts were inspected; prior RED runs were not re-executed here. The final combined suites were freshly executed on S7.

## Whole-request latency and admission

**Protocol:** optimized Cargo release build, rustc **1.94.1**, LLVM **21.1.8**, Rust-linked SQLite **3.51.3**, Linux ARM64 shared AWS builder `i-035f3692d8c17df4a`, kernel `6.1.0-53-cloud-arm64`. Provider flags include `-Ctarget-cpu=neoverse-v2 -Ctarget-feature=+lse`, clang and lld. Full machine/toolchain output is in the sample environment row. This is the standard release profile, not the dist/profiling LTO profile. Correctness tests are ordinary debug tests.

Each shape receives five warmup pairs and 31 timed pairs, alternating direct/reuse order. Timing begins before async submission to the existing reader queue and ends after common JSON output serialization. Reuse uses the real adapter `list_issues`; direct uses the same reader queue, a transaction, the production narrow query and batched labels. Direct does not inspect feed metadata. Reuse includes normalization, locking, metadata checks, retained-result decoding, cloning/allocation, and serialization costs incurred by that request. Outputs are byte-equal for every pair. Fixtures use one client and no concurrent writer during timing; no custom tracing subscriber is installed for these timings.

Measurement source stages: baseline production is `6c41c4e58`, with the new benchmark harness later committed in `da37a9019`; first-fix production is `f787530c0`; final production is `4cbd4f044`. All write/allocation samples come from the baseline run; later changes affect reads. Report tables and `measurement-summary.json` use nearest-rank p95, the sorted observation at rank `ceil(0.95 × n)` (one-based). For nine insert samples this is the maximum. The notebook separately labels its pandas linear-interpolation p95; medians agree exactly.

Actual direct query shape:

```sql
SELECT id, title, description, status, priority, issue_type, assignee
FROM issues WHERE 1=1
  -- open shape only:
  AND status IN (?)
  AND (is_template = 0 OR is_template IS NULL)
ORDER BY priority ASC, created_at DESC;
-- ? = 'open'; all_non_template omits only the status predicate.
-- Labels: production IN batches of at most 900 IDs, ORDER BY issue_id,label.
```

There is no text/label filter, timestamp, offset or limit in these timed shapes. Initial/delta cache loads also select raw rowid/priority/created_at ordering keys. Both snapshot validations remain after the fixes. Equal validated revisions avoid delta SQL; clock integrity, schema cookie and physical identity still receive checks.

| Fixture / returned shape | Direct median / p95, µs | Warm reuse median / p95, µs | Interpretation |
|---|---:|---:|---|
| 12 issues / 6 open, 14 labels | 56.957 / 133.618 | 56.482 / 84.621 | Approximately parity; median difference is within noise |
| 12 issues / all 12, 38 labels | 83.600 / 119.562 | 77.642 / 87.908 | Small observed reduction |
| 2,331 issues / 396 open, 943 labels | 1,601.309 / 1,669.767 | 1,107.385 / 1,122.441 | About 31% lower median for this admitted shape |
| 2,331 issues / all 2,331, 9,674 labels | 12,488.834 / 12,867.243 | 17,578.393 / 18,198.317 | Over budget, **not retained**, about 41% slower |

Before S7, the small open median was 432.905 µs reuse versus 57.597 µs direct; representative open was 1,943.045 versus 1,603.537 µs. The first fix alone improved the representative case but left a small-case regression (86.887 versus 58.918 µs); this drove the second TDD cycle. The final table uses matched pairs from the final run, not baseline timings from another execution.

The synthetic payload matches the supplied aggregate summary/raw-sort/label UTF-8 byte totals: **491,643 bytes** for 396 open issues and **3,939,064 bytes** for all 2,331 issues, with 943 / 9,674 labels. It uses uniform ASCII descriptions and synthetic labels; it does not reproduce private content, escaping distribution or the original largest-row/skew distribution.

Warm representative open retention is **571,964 bytes**, within the unchanged SQLite-derived **2,048,000-byte** budget. After 31 measured reuse calls, issue/label payload loads and full builds are zero. The oversized all-issue shape instead performs 31 builds / 31 explicit over-budget bypasses, loading 72,261 issue and 299,894 label rows. Its remaining one subscription is the earlier open query, not admission of the oversized query.

The byte counter covers boxed serialized keys/snapshots. SQLite state, fixed subscription slots, response/deserialization scratch, queued results and S6 pending-timer state are additional memory. One fixed prepared clock statement and one schema-cookie slot add constant metadata state. No TTL or new capacity constant was introduced. No process RSS or live caller-mix improvement is claimed.

## Index/feed write overhead and disk footprint

The same optimized Rust-linked SQLite build measures three modes: **0** retains the pre-existing priority/creation index; **1** also adds the S1 status/order index; **2** adds the issue feed to mode 1. Fixtures start with 2,331 issues / 9,674 labels. WAL and synchronous=NORMAL are matched, with a checkpoint before timed writes.

Nine fixture repetitions alternate mode order. Each repetition warms up one 200-row update transaction, then measures three more 200-row update transactions (27 samples per mode), followed by one 200-row insert transaction (9 samples per mode). Updates change status and priority. Inserts create **200 small, unlabeled issues with nine-byte descriptions**, unlike the representative seeded rows. These are **transaction batch timings, not single-edit latencies** or adapter/add_comment write timings. Setup, schema generation and fixture creation are excluded. The write path/schema did not change in S7, so the original write measurements remain applicable.

| Mode | Update batch median / p95, ms | Insert batch median / p95, ms |
|---|---:|---:|
| Existing index | 1.370 / 1.432 | 1.215 / 1.225 |
| + status/order index | 1.609 / 1.652 | 1.349 / 1.376 |
| + issue feed | 3.602 / 3.664 | 2.805 / 2.833 |

The feed costs about 2.24× the index-only update-batch median and 2.08× its insert-batch median in this workload. This cost must be considered with the unmeasured live read/write mix.

Page size is 4,096 bytes. The status index allocates **114,688 bytes**. After touching 200 existing IDs and inserting 200 IDs, the feed journal has **400 rows** and its table plus associated indexes allocate **36,864 bytes**, measured by freed pages when dropping that table after timing. This excludes the clock and trigger/schema pages. Total database allocation is 6,946,816 bytes index-only versus 7,041,024 bytes with feed; **Post-workload WAL file allocation** is 2,068,272 versus 2,183,632 bytes, following the initial checkpoint. This is file allocation, not cumulative write volume. Full per-round allocation rows are saved. Journal IDs, including deleted IDs, are deliberately not pruned; disk usage can grow with the number of IDs ever changed.

## Requirements matrix and test plan

Executed acceptance plan:

- Run every default spur-pm target and the reconciler library/tick/shutdown/late-enable tests.
- Exercise external SQL, corruption/replacement recovery, concurrent writes, paging, independent cursors, actual adapter comments and pending timers.
- Measure deterministic work at small/large sizes, then alternate equivalent optimized direct/reuse requests with representative payloads.
- Isolate index/feed write batches and allocation; rerun finite implemented-policy solver checks and strict scoped lint/format checks.

All tests named below passed in the final PM or core suites. The matrix follows the current task bodies, including S5/S6 review refinements, rather than treating the older proposal as an unqualified implementation specification.

| Contract | Implementation | Concrete evidence |
|---|---|---|
| Status/order index; preserve old indexes, ties and pages | `sort_index.rs` | `status_sort_plan_with_limit`, `status_sort_plan_without_limit`, `status_sort_preserves_rows_and_limit_windows_with_ties`, idempotence and initialization tests; isolated index write/allocation measurements above |
| Caller ancestry, privacy-safe query-shape/timing fields, errors/backpressure | `beads_db.rs`, `adapter.rs`, `issue_tracker.rs` | `submit_read_preserves_parent_span_and_dispatcher`, concurrent-reader/no-leak, error/cancelled-reply and backpressure tests |
| Seven fields, description, source/URL, AND labels, escaping, statuses/types/assignee/priority, since and paging | `summary_query.rs` | `summary_projection_matches_legacy_filter_and_page_matrix`, batched sorted labels, external-edit and projected/query error tests |
| Deliberate error-scope refinement | `summary_query.rs` module contract | `summary_projection_does_not_decode_unneeded_metadata`; unreturned metadata is not Rust-decoded, as explicitly approved in S3. Projected, label and SQL errors remain errors |
| Description-only external SQL with fixed timestamp/hash; all projected/filter/order fields | `issue_changes.rs` | `external_description_edit_with_fixed_timestamp_and_hash_is_visible`, `every_summary_filter_order_field_is_tracked_and_other_issue_fields_reach_hygiene` |
| Insert/delete/tombstone/ID replacement, label owner changes, REPLACE/rowid/unique victims, rollback/conflict safety | feed v3 triggers | Insertion/deletion/ID, label reparent, external_ref and hidden-rowid replacement, ignored/failed-conflict, structural replacement and rollback tests in `issue_changes_tests.rs` |
| Real adapter comments do not reload plain summaries; since remains correct | Distinct summary/timestamp/comment revisions | `adapter_comment_timestamp_is_unrelated_to_plain_summaries`, `issue_reads_unrelated_edit_and_adapter_comment_load_no_payload`, `timestamp_only_edits_signal_since_membership_entry_and_exit`, `issue_reads_pages_and_since_keep_sql_boundaries` |
| Both old/new membership and ordering; outside-window candidates | Full supported membership or direct page SQL | `issue_reads_old_and_new_membership_and_order_match_sql`, `issue_reads_pages_and_since_keep_sql_boundaries`; bounded membership POST |
| Unchanged work zero; relevant edit local at different database sizes | `IssueReads` work counters | `issue_reads_unchanged_work_is_zero_at_different_sizes`, `issue_reads_relevant_delta_loads_only_affected_payload`: 12/1,200 issue fixtures, zero/zero after warmup and one/one after edit; [counters](work-counters.json) |
| Fixed-cost validation/serialization and delta-query regression | S7 schema cookie, boxed-byte reuse, fixed clock SQL | `s7_unchanged_requests_do_not_revalidate_ddl_or_reserialize_payload`, `s7_unchanged_validated_revision_does_not_query_deltas`; two behavioral REDs and optimized actual-request measurements |
| Coherent watermark/delta/rows and coalesced readers | Feed transaction + adapter mutex | `watermark_delta_and_loaded_rows_share_one_snapshot`, `issue_reads_identical_concurrent_requests_coalesce` |
| Ordinary race stays dirty with original cursor; reset/error discards | Load then validation, both still present | `issue_reads_commit_during_load_leaves_snapshot_dirty`; validation generation, gap, schema, replacement and error-propagation/retry tests |
| Missing/corrupt clock, journal or triggers; exact restoration; schema and replacement recovery | Per-transaction clock/head/dirty checks; DDL check keyed by cookie/connection | `lost_or_corrupt_tracking_forces_reseed_and_recovers`, `exact_trigger_restoration_cannot_hide_an_untracked_edit`, `serialized_cursor_after_feed_restart_detects_clone_replacement`, cache/direct replacement and validation tests |
| Independent lagging consumers, timestamp-domain history and writer recovery | Per-ID domain revisions; no silent prune | `coalescing_retains_older_domain_changes_for_lagging_independent_readers`, `timestamp_summary_and_comment_revisions_survive_compaction_in_both_orders`, `repaired_journal_rejects_stale_writer_then_schema_read_allows_tracked_retry` |
| Retention and unsupported-shape observability | Pager-derived bytes + existing reader width; no TTL | `issue_reads_retention_obeys_database_budget_and_reader_width`, over-budget and unsupported-order bypass tests; representative admission and oversized failure-to-admit above |
| Pure structural edits do not load plain-summary payload, even after hygiene advances | Hygiene-only issue revision, independent summary cursor | `s7_structural_edits_advance_hygiene_without_summary_payload_loads`; raw mixed-case structural/replace feed tests |
| Unchanged hygiene avoids comments; one leaf remains local | Independent hygiene cursor | `hygiene_unchanged_skips_comments_and_leaf_edit_stays_local` |
| Pending grace expires with unchanged DB and no repeated comment reads | Cached successful empty audits for pending timers | `hygiene_pending_grace_expires_without_db_change_or_comment_reread`, `hygiene_pending_empty_audits_invalidate_on_comments_and_eligibility_exit` |
| Closed-parent audit, direct-audit precedence, eligible endpoints, mixed-case structural edges | Indexed parent expansion and graph-compatible predicates | `hygiene_closed_parent_changes_expand_to_child_and_direct_audit_wins`, `hygiene_review_excluded_parent_removes_inherited_plan`, mixed-case parent-comment and structural repair tests; PM endpoint/index tests |
| Partial failure, cancellation, overlap, self writes and concurrent comments cannot lose work | Staged cursor published only after success; serialized sweeps | `hygiene_partial_failure_and_comment_during_label_write_are_retried`, `hygiene_successful_write_does_not_ack_a_concurrent_comment`, `hygiene_overlapping_sweeps_serialize_and_cancelled_sweeps_retry` |
| Eligibility entry/exit/deletion, history gap, unsupported backend and errors | Candidate invalidation/reseed/default fallback | `hygiene_membership_reseed_unsupported_and_feed_errors`, pending invalidation tests and PM feed/hygiene recovery tests |
| Tick timers continue on unchanged data; cancellation/backoff preserved | Core tick/timer production flow retained | `hygiene_unchanged_database_still_runs_due_loop_on_tick`, `hygiene_unchanged_database_still_renews_expired_owner_lease`, `biased_select_cancel_preempts_pending_tick`, `cadence_backoff_formula`, integration cancellation/shutdown/late-enable tests |

The linked `work-counters.json` counters are cumulative and include cold loading. Totals of 7 / 601 issue rows mean 6 / 600 initial rows plus one changed row; the per-edit increment is one row.

Existing standalone clock/delta SQL work probes remain constant at 3 / 300 / 3,000 issues: clock `(rows=1, VM=54, scans=0, sorts=0)`; empty delta `(0,16,0,1)`. These probe the SQL explicitly. The final unchanged request skips that delta query altogether. They are work counts, not time or I/O proofs.

## Interfaces, solver scope and remaining limitations

**S7 adds no public API, dependency, schema version, TTL or capacity setting.** Internally it adds a connection-local validated-schema slot/helper, three privacy-safe diagnostic event sites, one fixed cached clock statement, and the unchanged-byte reuse branch. Existing SQLite/reader-derived bounds remain unchanged.

The combined implementation adds the crate-local summary query/summary reader queue, transactional issue feed/cursor/change types and direct-read helper; public `IssueReadWork` plus `BeadsCrateAdapter::issue_read_work`; and default-compatible `BeadsAdvanced::hygiene_changes` with `HygieneCursor`, `HygieneCandidate` and `HygieneBatch`. The default returns unsupported and preserves the full fallback. No IDs/exists API was added: that optional proposal item was outside the refined S1–S6 task contracts.

Required PRE receipts were reloaded: `sol_68e50056a49749b3` (declared lifecycle sat) and `sol_5d17396943d94ca6` (bounded old/new stale-reuse search unsat). The current workflow catalog was inspected before RED. S7 PRE verification `sol_4a5b269a2599446f` passed. Final implemented-policy POST results:

- **`sol_715ec684d377470c`** — summary lifecycle, sat/pass, 35 steps, including supported/unsupported, over-budget, unusual-order, race and validation-reset/error cases.
- **`sol_83ef1d0138b44ef6`** — hygiene lifecycle, sat/pass, 26 steps, including staged cursor, retries, concurrent writes, grace timers, parent expansion and fallback.
- **`sol_8fe1ea212d5047cd`** — conservative shipped old/new membership policy, unsat for stale reuse. One changing candidate; values 0–2, absence -1; complete coverage and matching identity/filter are explicit guards.
- **`sol_1090da184f144b96`** — feasible reuse witness, sat, both memberships absent. This avoids treating an infeasible reuse policy as a useful safety result.

Ten invalid summary variants and eleven invalid hygiene variants were rejected (unsat/fail); all exact requests and IDs are in `solver-post.json`. The root's existing S5/S6 policy artifacts were read at `approved_source_and_tests_verified`, including final S6 endpoint/case corrections. The coordinating agent also completed S7 Jev 1.13.0 compilation (gate open, weakest confidence 0.63, exact facts preserved): the supplied 30-step metadata/reuse policy passed and six invalid prefixes were rejected, with matching Python checks in the notebook. These finite supplied-policy checks do not prove the Rust implementation, trigger completeness, SQL equivalence, multi-row paging, all physical interleavings, unbounded liveness, or performance.

Reuse is admitted only on Unix for complete unpaginated memberships with no since/text query, ordinary supported ordering storage, and results within the derived budget. Limit—including zero—nonzero offset, since, text, exceptional ordering and oversized shapes retain SQL behavior; pages are not incrementally patched. Ties remain unspecified by the public SQL contract. Untouched metadata is deliberately not decoded under the approved S3 projection contract. Unsupported platforms do direct reads.

Every returned retained result still needs decoding/allocation and output serialization; an unrelated new revision can still require serializing the snapshot with its advanced cursor. The measured optimization does not mean zero CPU on reuse. Large over-budget results still pay loading/admission costs on every request, and the feed's disk journal retains deleted IDs indefinitely. S6 pending-timer memory and transient/result memory are outside the serialized subscription byte counter. Custom schemas/collations, arbitrary tracing configurations, high concurrency, cold caches, live queue contention and process RSS were not benchmarked.

The initial feed and original direct projection error/rollback semantics remain covered. An external writer whose prepared schema is stale after journal repair may need a schema-reading query or reconnect before retrying its failed statement; the feed does not silently retry another connection's writes. The protection model covers ordinary external SQL, not deliberate forging of both tracking data and schema metadata by a hostile administrator.

All previously reported severe S5/S6 review findings are represented by passing regression tests above. The tested production source has passed independent review. The coordinating agent corrected the report wording and is integrating the reviewed result; final integration is recorded in the [verification manifest](../2026-10-09-spur-64019/implementation-tdd/s7-final-verification.json).
