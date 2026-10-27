# Beads storage growth: investigation and proposed automatic maintenance

Date: 2026-10-27. Tracking issue: `bd-2md`.

Status: approved and implemented in `spur-pm` under `bd-28k`. The investigation below describes the baseline before implementation. See the [verification record](../../research/2026-10-27-beads-maintenance-verification.md) and the executed implementation section in the notebook.

## Evidence

The [executed Notebook MCP analysis](../../research/2026-10-27-beads-maintenance-analysis.ipynb) contains filesystem inventory, read-only SQLite health checks, DuckDB aggregation, code-index schema/freshness checks, a retention sizing model, and an isolated WAL experiment. The [inventory](../../research/2026-10-27-beads-storage-inventory.json) contains file metadata only.

At 04:05 UTC the checkout's `.beads` contained:

| Category | Bytes | Files |
|---|---:|---:|
| Live SQLite database | 364,544 | 1 |
| WAL | 115,392 | 1 |
| Shared-memory sidecar and lock | 32,768 | 2 |
| Live `issues.jsonl` | 7,076 | 1 |
| `.br_history` | 0 | 1 |
| **Total** | **519,780** | **6** |

The initial pre-investigation total was 515,660 bytes. Tracking this investigation writes to beads, and other processes can also change these files; this is a point-in-time inventory, not a consistent filesystem snapshot. No large snapshot problem is reproduced in this checkout. The previously active notebook project's `tcinvest_rest/.beads` directory was absent. No broader filesystem scan was performed.

SQLite reported 89 pages of 4,096 bytes, zero freelist pages, WAL mode, and `auto_vacuum=0`. At the health check there were three issues, two dirty issues, 24 events, and 15 comments. Last export metadata was `2026-10-27T03:22:37.028155+00:00`. Vacuum would have no freelist space to recover in this sample.

The first knowledge pack reported an analyst/exact-graph mismatch. The later notebook attachment to `.spur/analyst.duckdb` reported hash `8599245794571718b3153894d70d281c091e04ef3300eff4d66c0f3eb9fe6d59`, matching the exact-graph hash from that pack. Behavior claims below are corroborated with current source bytes, not inferred from symbol names or heuristic call edges.

## Findings

### 1. Backup retention has no byte budget

`crates/spur-pm/Cargo.toml:27` pins `getspur/beads_rust` at `47b3b39a54b04c7d51d735c032c82e891c2a0db8`. The exact revision was verified in the local Cargo checkout after the external package graph reported that revision was not indexed.

In that revision, `src/sync/history.rs:16-29` defaults to enabled history, 100 files, and 30 days. `backup_before_export` copies the previous JSONL and calls `rotate_history` (`:45-148`). It already deduplicates identical newest backups **and still rotates on that deduplication path**. There is no byte limit. This is not an unbounded file-count leak, but full copies multiply the size of the live export.

The notebook models 100 distinct recent copies:

| Export size | Existing history | Proposed additional 256 MiB budget* |
|---|---:|---:|
| 10 MiB | 1,000 MiB | 250 MiB / 25 copies |
| 50 MiB | 5,000 MiB | 250 MiB / 5 copies |
| 100 MiB | 10,000 MiB | 200 MiB / 2 copies |
| 500 MiB | 50,000 MiB | 1,000 MiB / 2 copies |

\*Protect the newest two available backups from additional byte-based pruning. This is a soft budget: protected copies may exceed it, and upstream age/count rotation may leave fewer than two copies. The model excludes live SQLite/JSONL, temporary export files, WAL, and quarantine. It is a scenario calculation, not measured savings.

### 2. Normal writes do not schedule JSONL export

`BeadsCrateAdapter::write` in `adapter.rs:427-459` commits the mutation and requests a checkpoint. `validate_and_commit` similarly only requests a checkpoint. `batch` delegates to `write`.

`auto_flush` exists at `adapter.rs:258-287`, but a workspace-wide source search found no production invocation of that adapter method. Its direct invocations are tests. Startup calls the upstream `sync::auto_flush` at `init.rs:606`. Thus long-lived processes can retain dirty rows until a later startup or another external export operation. The observed dirty rows and older export are consistent with this behavior; they do not prove all dirty rows were created by this adapter.

The upstream `src/sync/mod.rs:1849-1889` returns early when no issues are dirty and otherwise uses `ExportConfig::default()`. Therefore calling it periodically would not, by itself, maintain history on a clean store or expose an adapter-specific history policy.

### 3. Background WAL checkpointing does not shrink the physical file

`beads_db.rs:310-324` runs a passive checkpoint on checkpoint messages or a 30-second receive timeout. `checkpoint_passive` at `:364-387` has no size threshold or truncation. Startup has a separate best-effort `TRUNCATE` helper with zero busy timeout and passive fallback (`wal_checkpoint.rs:3-23`), but the quiet startup path can skip that sequence.

The notebook's temporary SQLite experiment kept the writer open: a 1,203,072-byte WAL remained 1,203,072 bytes after a successful passive checkpoint of all 292 frames, then became zero bytes after `TRUNCATE`. This demonstrates allocated-size behavior using the notebook's SQLite 3.53.1, not a Rust integration test. A large WAL can be reusable capacity rather than an outstanding replay backlog; pinned readers may also prevent truncation. Report both sizes and checkpoint frame counts.

### 4. Quarantine preserves recovery data and has no retention

`init.rs:150-253` moves recognized recovery siblings into `quarantine/<timestamp>-<pid>/` with a manifest. Moving files within `.beads` does not reclaim their bytes. There is no quarantine expiration policy here. Startup's `sweep_stale_jsonl_temps` at `:363` removes only recognized aged export temporaries while holding `.write.lock`; it does not maintain history or quarantine.

`snapshot.rs` defines an in-memory CAS value/version token. It does not write snapshot files. No general disk-snapshot writer was found in this adapter or in the pinned SQLite storage implementation.

## Proposed scope

Implement the first increment within `spur-pm`, using the existing connection actor and `.write.lock`. Do not add an independent database writer or invoke `br` subprocesses. Keep archive/compaction of canonical issue data separate from redundant-file maintenance.

| Setting | Proposed default | Purpose |
|---|---:|---|
| Flush interval | 30 seconds | Coalesce dirty exports during long sessions |
| Filesystem maintenance interval | 15 minutes | Bound repeated directory scans |
| History byte budget | 256 MiB | Bound redundant copies in addition to upstream 100-file/30-day policy |
| Protected recent history | 2 available files | Preserve recent rollback copies during additional byte pruning |
| WAL truncate threshold | 64 MiB | Avoid truncation on every write |
| Stale export temporary age | Existing 1 hour | Preserve current behavior |
| Quarantine deletion | Disabled | Recovery retention needs an explicit policy |

These values are starting defaults, not measurements of an affected large store. Accept a typed `MaintenancePolicy` through `AdapterConfig`, validate its values, and expose a structured maintenance report. Runtime configuration plumbing outside the crate can be a later change if needed.

### Actor scheduling and locking

Use monotonic deadlines checked after each job as well as while idle. A `recv_timeout(30s)` alone resets under continual traffic and is not a periodic scheduler. Coalesce due work and preserve the existing foreground result even if maintenance fails after a committed mutation. Log maintenance failures and retry with backoff; retain dirty state when export fails.

Foreground jobs acquire `.write.lock` **before** entering the writer queue. Actor maintenance must use `try_blocking_write_lock_once` and skip immediately on contention. Waiting for that lock inside the actor can deadlock behind a foreground caller that owns the lock and is waiting for its queued job. Never call the async adapter `auto_flush()` from inside a writer closure.

Attempt startup maintenance even when initialization's flush fast path is taken, without adding a blocking scan to the open path. For multiple adapters/processes sharing the directory, use a lock-protected, atomically replaced maintenance timestamp to throttle successful filesystem sweeps across processes. Treat a malformed or future timestamp conservatively, without indefinitely suppressing cleanup. In-process scheduling uses `Instant`; persisted timestamps use wall time. Flush can cheaply check dirty state independently.

### JSONL history

Retain the pinned exporter's current copy/deduplication and age/count behavior for the first increment. Add a separate byte-based cleanup pass that also runs when dirty count is zero. Only manage exact `issues.YYYYMMDD_HHMMSS.jsonl` regular files directly within `.br_history`. Preserve unrelated export stems, malformed names, directories, and symlinks, and reject a symlinked history directory. Do not recursively delete arbitrary `.beads` paths.

Build a deterministic oldest-first deletion plan using filename timestamps, with a stable filename tie-breaker. Protect the newest two available entries from this **additional** pruning, then delete older candidates until the budget is met or only protected files remain. Report remaining over-budget bytes. Upstream age/count pruning can remove old protected candidates before this pass; do not promise that two backups always exist or call them verified recovery points without validating their contents.

Expose dry-run planning and execution as separate functions. Run execution under the write lock, revalidate candidate identity/type before unlinking, tolerate concurrent disappearance, and surface permission errors. Do not prune on a failed export in a way that sacrifices the immediate pre-export recovery copy. An earlier successful standalone cleanup remains valid.

An oversized individual live `issues.jsonl` cannot be fixed safely by backup pruning. Diagnose issue/comment payload sizes; any archival, summarization, or deletion of canonical records requires a separate explicit feature. Compression could reduce backup bytes but requires compatible list/restore support in the pinned dependency; renaming files to `.gz` would bypass its current restore interface.

### WAL, temporary files, and quarantine

Keep routine passive checkpointing. When the WAL exceeds the threshold and maintenance is due, attempt `TRUNCATE` on the existing actor-owned checkpoint connection with zero busy timeout. Busy readers cause a skip and later retry, not a forced reader interruption. Record busy/log/checkpointed frame counts and physical bytes. Never unlink live `beads.db-wal` or `beads.db-shm`.

Reuse the strict temporary-file sweep under the write lock during periodic maintenance. Quarantine initially contributes byte/count diagnostics only. Future pruning should use trusted manifests, a separate age/byte policy and an explicit opt-in, with unrecognized files preserved. Routine automatic `VACUUM` is out of scope: it can be expensive, cannot remove live issue/event history, and the measured database has no free pages to reclaim.

## Verification required before implementation is accepted

- Retention planning: count/age behavior stays with upstream; byte pruning is deterministic, preserves recent available copies, reports protected-over-budget cases, and is idempotent.
- Path safety: invalid timestamps, other stems, nested directories, file/directory symlinks, disappearance, and permission failures never cause broad deletion.
- Runtime scheduling: dirty data exports without reopen; clean stores still receive retention checks; continual jobs cannot starve deadlines; multiple adapters throttle sweeps under one lock.
- Locking: a queued foreground job holding the flock cannot deadlock maintenance; busy maintenance skips without changing mutation results.
- Export failures preserve dirty state and recovery files; a valid export remains readable and retains all live issue data.
- WAL integration: keep a writer open, demonstrate passive allocation retention, attempt truncation with and without a pinned reader, and verify readable data afterward.
- Dry runs report the same candidate set as execution on unchanged fixtures and leave all bytes intact.
- Metrics report files/bytes reclaimed, bytes retained by category, over-budget protected bytes, busy skips, errors, duration, dirty count, and last successful export/maintenance.

The initial investigation ran nine notebook cells without changing Rust. The approved implementation subsequently passed the full crate suite, three durability simulations, focused strict lint checks, and three additional implementation-evaluation notebook cells. No live storage cleanup was executed as part of this investigation or evaluation.

## Implemented API and reporting

`AdapterConfig.maintenance` accepts the typed policy above. Existing callers using `Default` receive automatic startup/periodic maintenance without extra wiring. `adapter.maintain_now(true).await` previews cleanup without flushing/deleting; `maintain_now(false)` requests an immediate best-effort pass. Callers must inspect the returned `errors` and `skipped_busy` fields. `maintenance_report()` returns the latest non-dry-run report.

Flush-only or busy runs retain the latest storage-size sample with `usage_measured_epoch_ms`; that timestamp makes its age explicit. `last_export_epoch_ms` covers successful exports performed by this actor's maintenance runtime, not historical startup/manual exports. The 256 MiB limit applies only to recognized redundant history, and protected copies can exceed it. Canonical issue/event data and quarantine are retained.
