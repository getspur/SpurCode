## Unreleased

### Added
- **`spur-graph` content-hash invalidation substrate (bd-jvers, v2.1).**
  Replaces mtime/size per-file invalidation with a filtered-content hash
  over `(path, content_oid)` pairs, where `content_oid` is the git blob OID
  (from `git ls-files -s` for clean files; locally computed via
  `sha1("blob " || decimal(size) || "\0" || bytes)` for dirty / untracked /
  fs-mode files; `gitlink:<oid>` for submodules). Cache key derivation
  reads only the git index + dirty file bytes — does not depend on `HEAD`.
  - **Canonical artifact** at `<git_common_dir>/spur-graph/artifacts/<manifest_version>/<graph_content_hash>.json`
    (immutable per key, content-addressed).
  - **Per-worktree** `.spur/graph-index.json` retained as a full artifact,
    hardlinked to the canonical (with `fs::copy` fallback across filesystems).
    Existing direct consumers (`crates/spur-tui/src/mentions/code_graph/source.rs`)
    are source-compatible — no API changes.
  - **Optional pointer sidecar** at `.spur/graph-index.pointer.json` carries
    provenance (`indexed_commit_oid`, `source_kind`, `manifest_version`).
  - **Bucket reuse.** Within a worktree, unchanged paths (same `content_oid`
    as the previous artifact) are cloned wholesale — no tree-sitter parsing
    or symbol extraction. This is the per-build extraction-saving mechanism.
  - **Cross-worktree dedup.** Two worktrees with identical filtered content
    derive the same canonical key regardless of commit history, so the
    second writer skips the canonical JSON write and hardlinks the existing
    artifact. (Each worktree still computes its own discovery + hash;
    pre-extraction shortcircuit is tracked as a followup, bd-5yful.)
  - **Dirty == committed (I2).** A worktree's dirty `content_oid` equals
    the blob OID git would assign if the file were committed, so
    `graph_content_hash` before and after committing identical bytes
    matches and every bucket is reused on the next build.
  - `fs2` exclusive lock with 5s timeout + worktree-only fallback on
    contention; atomic write via tmp + rename + best-effort `fsync_dir`.
  - Schema bumped to `spur-graph-schema-v4`: `GraphFileManifestEntry` swaps
    `mtime_nanos`/`size_bytes` for `content_oid`; `GraphIndexArtifact` gains
    `graph_content_hash` and value-level `tombstones` (no commit OID).
  - Design spec: `docs/architecture/spur-graph-git-invalidation.md`.
- **`spur-graph` benchmark gate.** Criterion bench at
  `crates/spur-graph/benches/incremental.rs` covers four scenarios:
  clean cold (cache miss), clean warm (cache hit via hardlink), 1k-file
  change set over baseline, and dirty unstaged mods overlay. Env-knob
  scaling via `BENCH_FILE_COUNT` / `BENCH_CHANGE_SET` / `BENCH_DIRTY_MODS`.

### Fixed
- **`spur-graph` cache lock-timeout no longer leaves a ghost pointer**
  (bd-jvers followup). On `fs2` lock contention, `write_with_dedup`
  previously wrote the worktree artifact and ALSO wrote a pointer sidecar
  whose `canonical_artifact_path` referenced a canonical file that was
  never created — any pointer-following reader hit a deterministic
  missing-file failure. Fix: on lock-timeout, write the worktree artifact
  (still authoritative) and remove any pre-existing pointer so it cannot
  be stale. Convergent finding from claude-code and kimi reviews. Test
  `lock_timeout_writes_worktree_only` extended to stage a stale pointer
  before the lock-contended write and assert no pointer exists after.

- **Experimental TUI Insights view (`--features analytics`, default OFF).**
  `Alt+a` from any view opens a 4-tab Insights surface (Overview / Timeline /
  Breakdown / Live) backed by `spur-context::AnalyticsEngine`'s DuckDB store.
  - Overview: KPI cards (Today / 7d / 30d / Cache hit), 7d cost sparkline,
    cost-provenance gauge (native / priced / unpriced), top-3 agent / model /
    project lists.
  - Timeline: BarChart with `D`/`W`/`M` granularity toggle (no re-query).
  - Breakdown: Pivot table over Agent / Model / Project (`A` / `M` / `P`).
  - Live: per-session burn rate, hourly projection, refresh interval drops to
    5s while this tab is active (60s otherwise).
  Refresh is driven by an async `tokio::spawn` task with a signal channel and
  30s timeout; the view never blocks the UI thread.
- **Five well-known agents are first-class in analytics**: Claude Code,
  Codex, Gemini, OpenCode, Kimi. Kiro remains a Phase-2 stub
  (no token data via filesystem; ACP `UsageUpdate` capture pending).
- **`spur-context` Gemini extractor (R4).** Parses `~/.gemini/tmp/<uuid>/chats/session-*.json`
  with `tokens.{input,output,cached,thoughts,tool}` folding. Wires into
  `AnalyticsEngine` alongside existing Claude / Codex / OpenCode / Kimi paths.
- **OpenCode model-prefix strip (R1).** `anthropic/claude-opus-4-5` is now
  stored as `claude-opus-4-5` so the pricing registry's `LIKE` matcher works.
  Eliminates spurious `cost_source='unpriced'` rows for OpenCode users.
- **Kimi `kimi-for-coding` pricing (R2).** Registered at $0.60 input /
  $2.50 output / $0.15 cache-read per million tokens. Kimi events now surface
  with correct `cost_source='priced'` instead of `'unpriced'`.
- **OpenCode SQLite mtime in cache staleness (R3).** `newest_agent_mtime`
  now includes `~/.local/share/opencode/opencode.db`, fixing permanently-stale
  cache for OpenCode-only users.
- **Dashboard cost-source switch (when feature on).** Dashboard's total cost
  reads from a periodically-refreshed `LiveCostCache` populated via
  `AnalyticsEngine::live_session_snapshot` instead of `ExecutorLineage`. The
  status bar shows a `via analytics` pill while the cache has data for the
  active session. Single source of truth for cost when feature is enabled.
- **CI matrix for `spur-tui`.** New workflow runs
  `cargo check + cargo test --lib` for `--no-default-features`, default, and
  `--features analytics` configurations on every PR and `main` push.
- **P0 cost-correctness fixes harvested onto this branch.**
  - P0.1: Claude event dedup on `(sessionId, requestId, message.id)`.
  - P0.2/3/4: `cost_source` column + Codex cache double-count fix.
  - P0.6: `include_str!` per-report SQL with synced cache columns.
  - P0.8: `SessionRow::models` aggregates across mid-session model switches.

### Changed
- **Project renamed to SpurCode for the public open-source identity.** The
  repository at `getspur/SpurCode` is now branded "SpurCode" in top-level
  documentation (README, ARCHITECTURE); the `spur` binary, `@getspur/spur-cli`
  npm package, `.spur/` paths, crate names, and protocol sentinels
  (e.g. `[[spur-loop v1]]`) are unchanged. Added community health files:
  `SECURITY.md` (private vulnerability reporting policy),
  `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), issue templates
  (bug / feature / contact links), and a pull request template
  (`.github/`).
- **Read-only future metadata saves now fail visibly.** When
  `.spur/session_metadata.json` was written by a future SpurCode version,
  `SessionMetadataStore::save()` now returns `Err(ReadOnlyFutureSchema)`
  instead of silently accepting and discarding in-session edits. App
  save paths route through `App::persist_metadata`, which surfaces the
  refusal as a dismissible warning banner.
- **Peer mailbox reconciler now emits `WorkerPeerMessageAuditFailed`** on
  non-terminal transition errors during startup reconciliation. The
  `WorkerPeerMailboxReconciled.audit_failed_emitted` counter, which was
  always `0` prior to this release, may now report non-zero values when
  the persistent ledger introduces transition failures. Dashboards and
  alerts that filter `audit_failed_emitted == 0` as a "no anomalies"
  signal should switch to alerting on the
  `WorkerPeerMessageAuditFailed { transition_kind: "reconcile_to_delivered" }`
  event directly. (bd-cpf.5b)

### Added
- **Tolerant input-history deserialize.** A malformed `ProtectedRange` or
  `InputHistoryEntry` in `.spur/session_metadata.json` no longer aborts
  the entire load: bad rows are skipped with `tracing::warn!`, while
  remaining valid history is preserved.
- **`schema_version` field on `SessionMetadata`.** Persisted as the
  on-disk JSON key `"version"` for backward compatibility. Files written
  by a future SPUR version are loaded read-only; in-memory mutations are
  not persisted until SPUR understands that schema.
- **Read-only mode banner.** Future-version metadata now shows a top-row
  warning at first paint and after every refused save attempt. `Esc`
  dismisses the current banner without clearing read-only mode.
- **`SessionMetadataStore::is_read_only()` getter.** Callers can poll
  whether metadata is in read-only mode before enabling write-oriented UI.
- **Worker heartbeat watchdog configuration.** New `[worktree]` config keys:
  `worker_heartbeat_watchdog_enabled` (bool, default `false`),
  `worker_heartbeat_timeout_secs` (u64, default `90`),
  `worker_heartbeat_initial_grace_secs` (u64, default `60`). See
  `docs/architecture.md` Risk #23 for operational guidance and the
  no-runtime-toggle rollback constraint. (bd-arch.23)
- **`DelegationAbortReason` enum** distinguishing `BrainRequested` from
  `WorkerHeartbeatTimeout`. Stage-2 will extend with `ResourceLimitExceeded`
  / `SandboxTerminated` for cgroup-based termination. (bd-arch.23)
- **Peer mailbox production wire-up (Stage-1).** The peer mailbox subsystem
  (hardened in bd-cpf.1–7) is now constructed and attached when
  `peer_mailbox_enabled = true` is set in config. A long-lived reconciler
  task drains stranded peer messages and emits audit events. Startup
  reconcile runs at brain session boundaries. Default is `false`; no
  behavioral change for existing deployments. Operators who opt in should
  monitor for `WorkerPeerMessageUndeliverable` events and be aware that
  the in-memory ledger does not prune entries (Risk #22). To disable,
  set `peer_mailbox_enabled = false` and restart SPUR — runtime toggle
  is not supported. (bd-arch.21)
- **Peer mailbox drain lifecycle events.** `WorkerPeerMessageDrainStarted`
  and `WorkerPeerMessageDrainTimedOut` add symmetric observability to the
  post-prompt ack drain. `DrainStarted` carries the candidate-set size
  and the cap/quiet-window limits in effect; `DrainTimedOut` carries the
  same payload shape as `WorkerPeerMessageDrainCappedOut` plus
  `quiet_window_ms`, so dashboards can reuse panel queries across both
  exit events. `DrainTimedOut` is emitted only when the quiet-window
  exit leaves remaining non-terminal messages; clean-exit drains
  (`remaining_messages == 0`) emit no exit event. Diagnostic-only —
  message loss continues to be tracked per-message via
  `WorkerPeerMessageIgnored`. (bd-cpf.7)
- **`WorkerPeerMailboxReconciled.inflight_already_delivered` counter.**
  Tracks benign idempotent races during startup reconciliation where an
  entry was already in `Delivered` state when the reconciler attempted
  to advance it. Always 0 in Stage-1; becomes non-zero under Stage-2
  crash-loop or concurrent-reconcile scenarios. (bd-cpf.5c)
- **Spur Way skill bundle.** Six bundled skills harden brain-worker-beads
  collaboration: `spur-way` (beads-first invariant), `beads-lifecycle`
  (status state machine), `worker-signals` (`[[spur-signal v1]]` protocol),
  `brain-review-gate` (beads-aware approval checklist), `plan-task-discipline`
  (DAG integrity rules), and `worker-mention-routing` (user `@`-mention
  overrides algorithmic selection). All compile into `spur-core` via
  `include_str!` and render across the seven agent adapters. See
  `docs/superpowers/specs/2026-04-22-superpower-skill-hardening-spur-way.md`.
- **Role-gated skill rendering + Kimi adapter.** Skills now carry a `role`
  field (`brain | worker | both`). Brain-only skills (`brain-review-gate`,
  `brain-delegation`) no longer leak into worker agent directories. Worker
  skills (`test-driven-development`, `systematic-debugging`) are tagged
  explicitly. New `Adapter::Kimi` renders to `.kimi/skills/`, closing the
  gap where Kimi workers accidentally relied on `.claude/skills/` fallback.
  See `docs/superpowers/specs/2026-04-22-multi-agent-skill-embedding-research.md`.

### Fixed
- **Risk #4 (worktree orphaning under unclean shutdown).** New
  `WorktreeAuthority` actor sweeps dead-session worktrees safely under
  multi-process operation. Branch namespace migrated to
  `spur/worker/v2/{agent}/{brain_session_id}/{worker_session_id}` so
  sweep enumeration can be precisely scoped to v2 worktrees. Pre-v2
  branches are NOT auto-cleaned; operators reclaim legacy debt via the
  separate `spur-worktree-gc-legacy.sh` script (deferred). The actor
  uses a `SessionLivenessProbe` over advisory `flock(2)` and a
  `SelfHeldSet` to skip self-owned sessions. See
  `docs/superpowers/specs/2026-04-26-worktree-authority-design.md` for
  the full invariants I-1..I-7.
- **Worker child processes now die with their orchestrator** via
  `kill_on_drop(true)` on the `tokio::process::Command` spawn paths in
  `crates/spur-acp/src/connection/{native,stdio_adapter,cli_wrap_adapter,stream_json_adapter}.rs`.
  Closes Risk #4's hard prerequisite.
- **`spur-bot/tests/runtime_flow.rs` compile errors absorbed.** 12
  inline `AgentSessionReady` event initializers were missing the
  `fs_unsafe: bool` field added in the single-attach invariant work
  (commit `84e91895`). Fixed inline so the workspace smoke test gate
  passes. This is cross-stream cleanup — not part of the
  WorktreeAuthority design but required for plan closure.
- **Architecture Risk #23 (semaphore indefinite wait).** Permit acquire is now
  cancellable: `cancel_delegation` arriving while a task is queued for a
  permit short-circuits immediately without acquiring. A heartbeat-based
  watchdog (default-off) detects silent worker hangs and releases the held
  permit after `worker_heartbeat_timeout_secs` (default 90s, configurable).
  Watchdog is gated behind `worker_heartbeat_watchdog_enabled` (default
  `false`) until a v1 `_spur/heartbeat` emitter lands; operators may opt
  in early if their workers emit heartbeats. Watchdog firings map to
  `DelegationStatus::Timeout`, preserving the `Timeout` (worker-hang)
  vs `TimedOut` (review-gate) semantic split. Brain-initiated cancellations
  continue to map to `DelegationStatus::Cancelled`. (bd-arch.23)
- **Architecture Risk #21.** The peer mailbox reconciler is now spawned
  at orchestrator boot and aborted on shutdown via `Orchestrator::drop`.
  Previously the receiver was dropped immediately after construction,
  causing stranded messages to be silently lost — but the surrounding
  wire-up was also missing in production, so the entire subsystem (62
  tests, bd-cpf.1–7 hardening) was inert. (bd-arch.21)

## v1.1.8 — 2026-05-13

Spur 1.1.8 ships fresh defaults for the agents you actually run and makes two long-standing rough edges disappear: `/model` now feels instant on every agent, and long GitHub fetches stop looking like a hang.

- **Fresh out-of-the-box agent versions.** `spur init` now seeds new repos with `claude-agent-acp 0.33.1` (up from 0.26.0) and `codex-acp 0.14.0` (up from 0.11.1). New users get the latest ACP features and fixes on first run — no manual version-pinning, no stale prompts about deprecated flags. Existing `.spur/config.toml` files are preserved as-is; bump the pinned versions there when you're ready.
- **`/model` feels instant on every agent.** Some agents (including older Claude Code and Kimi builds) don't emit a `config_option_update` after a model switch, so the status bar used to keep showing the previous model until you reconnected. Spur now applies an optimistic override the moment you pick a model — the label flips immediately and reconciles with the agent's confirmation when it arrives.
- **GitHub ingest shows live progress.** `spur pm` ingesting a large GitHub repo's issues used to look frozen for minutes on the first run. The PM layer now surfaces per-page progress as fetches stream in, so you can see it working and estimate how much longer it'll take instead of wondering whether to kill it.

## v1.1.7 — 2026-05-13

Spur 1.1.7 makes everyday agent-switching and navigation feel right. Changing models works everywhere, the picker puts what you actually want at the top, and the code graph is there when you need it without any setup.

- **`/model` now works on every supported agent.** Switching models in Gemini CLI and Kimi CLI was broken before — both now behave like Codex, Claude Code, and OpenCode. Pick a model, get that model.
- **Status bar reflects the model you're actually on.** After a `/model` switch, the label updates live instead of showing the old name until you restart the session.
- **Smarter `@mention` picker.** Files no longer get buried under issues and workers. Results are grouped under clear section headers and ordered so the thing you're most likely reaching for is at the top.
- **Code graph just works.** Spur now auto-discovers the project's code graph at the worktree root — no environment variable to set, no "run `spur graph build`" hint when the graph is already there. Rebuilds are also faster and symbol names stay stable across runs, so jumping between files is more reliable.

## v0.4.5 — 2026-04-19

Spur 0.4.5 focuses on getting around faster and trusting what you see. A new universal palette (Ctrl+K) jumps you between sessions, workers, traces, and commands from anywhere. The `@mention` and `/slash` pickers now share one consistent interface. Agent output renders markdown and mermaid diagrams inline, and streaming stays smooth under bursty traffic.

### Added
- **Universal palette (Ctrl+K).** Fuzzy-search and jump to any session, worker, trace entry, or command from anywhere in the TUI. A `[Ctrl+K: go]` badge in the status bar reminds you it's there.
- **Unified completion for `@mention` and `/slash`.** Both now flow through the same picker — same keys, same preview, same ranking. One interface to learn.
- **Rerun recent prompts with Ctrl+R.** Session picker surfaces your recent prompts; pick one to rerun against the current session.
- **Auto-resume landing.** Launching spur drops you straight back on the last session you were working in.
- **Markdown and mermaid in agent output.** Rich content renders inline in the trace view — no more raw source for formatted replies or diagrams.
- **Dashboard view.** New top-level view for at-a-glance status across sessions and workers.
- **Skills installer across adapters.** `.spur/skills/` installs to Cursor, Codex, Kiro, Gemini, and OpenCode in one step, with edits you own preserved on upgrade.
- **Delegation visibility in the timeline.** Delegation plans now appear in the TUI timeline as they happen; `list_available_workers` returns richer descriptors (tier, cost, suitability hints).

### Improved
- **Smoother streaming.** A new scroll anchor and per-frame drain cap keep long, bursty outputs readable instead of jittery.
- **Faster palette and pickers.** Single-pass reranking and cached search patterns.
- **Session metadata persists across restarts.**

### Preview features (opt-in)
- **Brain delegation framework.** A new orchestration model for deciding which worker handles a task. Opt in via your config:
  ```toml
  [brain.delegation]
  framework = "v1"
  ```
  Release builds default to `legacy` for 0.4.5; the v1 framework will become default once it stabilizes.

## [2026-07-09]

### Added
- **`spur-tui`: Explore pool table view.** The manage lens now renders explore-pool entries in a denser table layout, which makes larger pools easier to scan and compare without dropping back to raw detail views. (`3c1bfec39`)
- **`spur-tui`: Browse catalog filter in Explore.** Explore browser sessions can now narrow the catalog with an in-view filter, reducing the amount of scrolling needed to find the right item in larger pools. (`3ef14e861`)
- **`spur-tui`: Accept-all clean gate cards.** Clean gate cards can now be accepted in one pass, which removes repetitive confirmation work when a review batch has no blocking findings. (`5420dedad`)
- **Telemetry performance and usage events.** Spur now emits slow-frame, view-opened, MCP tool/request, ACP request, and LLM request telemetry so operators can observe UI and request latency end to end. (`e71f8f288`)

### Fixed
- **`spur-core`: OpenCode MCP schema compatibility.** Tool schemas now inline `$ref` definitions before they are exposed through MCP, which fixes tool ingestion for OpenCode clients that could not follow the previous schema shape. (`2f8cc2c33`)
- **`spur-context`: Worktree project attribution and scan resilience.** Context collection now collapses SPUR worktree checkouts back to their origin repo and skips unreadable filesystem entries instead of aborting the scan, which fixes fragmented or missing project rollups. (`a50655ed6`, `cce1c81bc`)
- **`spur-analyst`: Database hardening.** Analyst scratch lifecycle ownership, nested Arrow value serialization, and incomplete-database blocking now prevent partially-built analyst stores from surfacing as live query failures. (`4ed1f4a1c`)
- **`spur-tui`: Session detail UX regressions.** Session detail rendering and input handling regressions are corrected so navigation and readback behave consistently again in the detail pane. (`4d6f26706`)

### Changed
- **`spur-tui`: Explore view UX refresh.** Scroll-follow, preview caching, render-path caching, browse presentation, gate interaction feedback, and search filtering all landed together, making the Explore surface steadier under frequent updates. (`12b626b4c`)
- **`spur-tui`: Loop browser polish.** The loop browser now documents `g`/`G`, masks notices correctly, shows governor placeholders, and wraps confirmations cleanly so long-running loop inspection reads more predictably. (`36f09d00a`)
- **`spur-tui`: Session picker polish and render-frame cleanup.** Session picker theme tokens, preview styling, key handling, and render-path cleanup reduce visual churn and keep filtering responsive while moving through long session lists. (`e560f9995`)

## [2026-07-11]

### Added
- **`spur-core`: Autonomous L3 loop ownership.** Spur now lets the project runtime hold L3 loop leadership under the stable `spur-loop-runtime` owner, with dedicated reviewer dispatch and failover semantics for unattended project loops. This removes the need for an active brain session to keep L3 execution moving while preserving a separate maker/checker review boundary. (`e359653a3`)

### Fixed
- **`spur-context-service`: Queue repair and cursor hardening for external indexing.** Queued index jobs now recover terminal and stale dispatch paths without leaking running capacity, and the queue cursor protocol is hardened so backlog draining does not strand work after retries or partial failures. (`7dd56ef6f`, `7986ca2cd`)
- **`spur-context-service`: Legacy crates.io warm-hit preservation.** Existing crates.io cache hits are now preserved for legacy records, which avoids unnecessary reindexing when a package was already warm in the catalog. (`16fceaf59`)
- **`spur-core`: Codex role TOML compatibility.** Generated Codex agent role files now use a schema-safe managed marker format, keeping SPUR-managed profiles compatible with Codex’s supported TOML shape while still detecting edited files correctly. (`0884f5404`)
- **`spur-core`: Exact global catalog verification during migration.** Global catalog migration now verifies the fully merged catalog state instead of only checking for surviving entries, which prevents incomplete or stale merged catalogs from being accepted as successful migrations. (`806618c72`)

### Changed
- **`spur-context-service`: Bounded external-index admission.** `external_index` now enqueues accepted work behind per-owner and optional global backlog caps, then dispatches it through a correctness drainer instead of starting execution inline. This adds explicit backpressure controls while ensuring queued indexing jobs still start and complete under the configured limits. (`edae14a2e`, `9ca1a9568`, `e8eac0eba`)
