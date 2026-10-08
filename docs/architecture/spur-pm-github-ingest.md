# spur-pm: GitHub Issue & PR Ingestion — Design Spec (Phase 1)

Status: Approved direction. Implementation-ready.

Builds on:
- [`docs/architecture/spur-pm-beads-source-of-truth.md`](spur-pm-beads-source-of-truth.md) — invariants this spec preserves.
- [`docs/research/github-ingestion-rust.md`](../research/github-ingestion-rust.md) — research and tradeoff analysis.

Scope: punch-list items 1–9 from the research doc. Out of scope (deferred to a Phase 2 spec): ETag incremental polling, webhook receiver, failure-state TUI plumbing, optional upstream `beads_rust` schema PR.

Acceptance: an open-source contributor with `gh auth login` already done can run `spur pm ingest github <owner>/<repo>` on a real upstream repo and see its issues + PRs land in `.beads/beads.db` with full provenance, surviving renames and re-runs, with dependency hints surfaced as structured comments for the brain to act on.

## 0. Invariants (re-stated; the spec must preserve all of these)

I-1. Beads is the only PM store used for dependency-aware orchestration.
I-2. GitHub is an ingest source and sync target, never a peer issue authority.
I-3. The graph engine reads only Beads state through `BeadsCrateAdapter`.
I-4. `PmService::advanced()` remains Beads-gated.
I-5. Remote sync failures must not mutate local truth backwards without explicit conflict handling.
I-6. PR creation remains separate from issue authority.
I-7. New remote PM support must not extend `PmBackendInner` with a peer `IssueTracker` backend.
I-8. Dependency extraction from remote Markdown produces **hints**, not edges. Only the brain mutates the local Beads DAG.

Any reviewer who finds a section of this spec contradicting one of these should treat the contradiction as a bug in the spec, not the invariant.

## 1. Architecture at a glance

```
                                ┌──────────────────────────────┐
spur pm ingest github X/Y ─────►│  CLI (crates/spur/src/cli/)  │
                                └──────────────┬───────────────┘
                                               │
                                               ▼
                                ┌──────────────────────────────┐
                                │  PmService::sync_target()    │  ◄── new accessor
                                └──────────────┬───────────────┘
                                               │ &dyn ExternalPmSync
                                               ▼
                                ┌──────────────────────────────┐
                                │  crates/spur-pm/src/         │
                                │    sync.rs    ←── new trait  │
                                │    ingest/                   │
                                │      mod.rs    apply_delta() │
                                │      watermark.rs            │
                                │      dep_hints.rs            │
                                │      github/                 │
                                │        mod.rs    GitHubSync  │
                                │        client.rs  octocrab   │
                                │        auth.rs   gh + device │
                                │        mapping.rs            │
                                │        graphql/              │
                                │          ingest_repo.graphql │
                                │          types.rs            │
                                └──────────────┬───────────────┘
                                               │
                                               ▼
                                ┌──────────────────────────────┐
                                │   .beads/beads.db            │
                                │   (sole source of truth)     │
                                │                              │
                                │   issues  ←── provenance:    │
                                │              external_ref,   │
                                │              source_system,  │
                                │              source_repo     │
                                │   comments ←── spur-sync v1, │
                                │              spur-dep-hint,  │
                                │              spur-import     │
                                │   dependencies               │
                                └──────────────────────────────┘
                                               ▲
                                               │ via BeadsCrateAdapter
                                               │ + .beads/.write.lock
                                               │
                                ┌──────────────────────────────┐
                                │  GitHub GraphQL/REST         │
                                │  via octocrab                │
                                └──────────────────────────────┘
```

Three rules govern the layout:

R-1. The CLI and `PmService` are the only callers allowed to drive ingest. Brain agents go through `PmService::sync_target()`; they never touch `octocrab` directly.
R-2. All writes go through `BeadsCrateAdapter::write` (the existing `.beads/.write.lock` discipline). The ingest module composes adapter methods; it does not open `SqliteStorage` directly.
R-3. **There is no second store.** All ingest state — provenance, sync watermark, link state, imported-comment idempotency markers — lives in `.beads/beads.db` via primitives `beads_rust` already provides (provenance columns, comments). Preserves invariant I-1 literally, not just in spirit.

## 2. Public types — `crates/spur-pm/src/sync.rs` (new module)

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The contract Spur uses to talk to any external PM. Separate from
/// `IssueTracker` so external systems cannot become peer authorities (I-7).
#[async_trait]
pub trait ExternalPmSync: Send + Sync {
    /// Stable provenance tag — "github", "linear", "plane".
    fn source_system(&self) -> &'static str;

    /// Per-instance scope, e.g. "getspur/SpurCode".
    fn source_repo(&self) -> &str;

    /// Bulk pull. `since=None` means full repo state.
    async fn fetch_changes_since(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> SyncResult<RemoteDelta>;

    /// Fetch a single remote node by stable id. `if_none_match` is the
    /// REST-only fast path; GraphQL implementations ignore it.
    async fn fetch_one(
        &self,
        remote_id: &str,
        if_none_match: Option<&str>,
    ) -> SyncResult<FetchOneOutcome>;

    /// Project local Beads mutations onto the remote.
    /// `Vec` order is preserved; outcomes align positionally.
    async fn push_mutations(
        &self,
        diff: Vec<LocalMutation>,
    ) -> SyncResult<Vec<PushOutcome>>;

    /// Compare local watermarks against the remote (cheap; uses ETag/
    /// updated_at). Used by the apply step before any push.
    async fn detect_conflicts(
        &self,
        watermarks: &[SyncWatermark],
    ) -> SyncResult<Vec<RemoteConflict>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDelta {
    pub nodes: Vec<RemoteNode>,
    /// Remote IDs known to be deleted/inaccessible.
    pub deletions: Vec<RemoteRef>,
    /// Server-time cursor for the next `fetch_changes_since` call.
    pub watermark: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteNode {
    pub remote_id: String,           // e.g. GitHub node_id: "I_kwDO..."
    pub remote_number: Option<u64>,  // e.g. issue #42 (display only)
    pub kind: RemoteKind,
    pub title: String,
    pub body: String,
    pub state: RemoteState,
    pub labels: Vec<String>,         // raw remote names; mapping happens later
    pub assignees: Vec<String>,      // remote logins
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub html_url: String,
    pub etag: Option<String>,        // REST poll path only
    pub dep_hints: Vec<DepHint>,
    pub comments: Vec<RemoteComment>,
    /// Anything we didn't map; preserved for forward-compat.
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RemoteKind { Issue, PullRequest }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteState {
    Open,
    Closed { reason: Option<String> },
    Draft,                               // PRs only
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRef {
    pub source_system: String,
    pub remote_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteComment {
    pub remote_id: String,               // GitHub comment node_id
    pub author: String,                  // login
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepHint {
    pub kind: DepHintKind,
    pub remote_keyword: String,          // verbatim, e.g. "Closes"
    pub remote_ref: String,              // canonical form, "owner/repo#42"
    pub remote_node_id: Option<String>,  // when GraphQL gave us the resolved node_id
    pub raw_span: String,                // exact source text
    pub source: DepHintSource,
}

// Note: there is no `resolved_beads_id` field on DepHint itself. Hint
// comments are append-only sentinels; local resolution to a beads_id
// is computed at read time via `BeadsAdvanced::list_dep_hints`, which
// returns `Vec<ResolvedDepHint>` (DepHint + live lookup). See §5.6.

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DepHintKind {
    Closes, Fixes, Resolves,
    DependsOn, Blocks, BlockedBy,
    TaskList,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DepHintSource { Body, TimelineItem }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncWatermark {
    pub beads_id: String,
    pub remote_id: String,
    pub last_synced_at: DateTime<Utc>,
    pub last_synced_etag: Option<String>,
    pub last_synced_remote_updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FetchOneOutcome {
    Unchanged,                           // 304 / etag match
    Updated(RemoteNode),
    Gone,                                // 404 / repo private / transferred
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalMutation {
    pub beads_id: String,
    pub remote_id: String,
    pub kind: LocalMutationKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LocalMutationKind {
    StatusChange { from: String, to: String },
    LabelsAdded(Vec<String>),
    LabelsRemoved(Vec<String>),
    CommentAdded { body: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PushOutcome {
    Pushed { new_etag: Option<String>, new_remote_updated_at: DateTime<Utc> },
    Conflict(RemoteConflict),
    Skipped { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConflict {
    pub beads_id: String,
    pub remote_id: String,
    pub local_updated_at: DateTime<Utc>,
    pub remote_updated_at: DateTime<Utc>,
    pub watermark_remote_updated_at: DateTime<Utc>,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ConflictReason {
    RemoteMovedSinceLastSync,
    LocalAndRemoteBothMutated,
}

pub type SyncResult<T> = std::result::Result<T, SyncError>;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("authentication required: {0}")]
    NeedsAuth(String),
    #[error("remote not found: {0}")]
    Gone(String),
    #[error("rate limited; retry after {retry_after_s}s")]
    RateLimited { retry_after_s: u64 },
    #[error("transient network error: {0}")]
    Transient(String),
    #[error("malformed remote response: {0}")]
    Malformed(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

`SyncError` is the *only* error type that crosses the `ExternalPmSync` boundary; each variant maps to a documented retry / surfacing strategy in §6.

## 3. Public types — extension of `spur-pm` PM types

The existing `Issue`, `IssueCreate`, and `IssueUpdate` (`crates/spur-pm/src/types.rs`) currently hide the provenance fields that `beads_rust` already supports. We surface them as optional fields — old callers compile unchanged.

```rust
pub struct Issue {
    // ... existing fields ...

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
}

pub struct IssueCreate {
    // ... existing fields ...

    /// Provenance. When `external_ref` is `Some`, `create_issue` is
    /// idempotent: if a row already exists with this `external_ref` it
    /// returns the existing id rather than creating a duplicate (relies
    /// on `beads_rust`'s UNIQUE partial index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
}

pub struct IssueUpdate {
    // ... existing fields ...

    /// Set, clear, or leave the external_ref alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<Option<String>>,
}
```

**Wiring touchpoints (the five hardcoded `None` sites become passthroughs):**

- `crates/spur-pm/src/beads_crate/issue_tracker.rs:325` — `create_issue` populates `beads_rust::model::Issue.external_ref / source_system / source_repo` from `IssueCreate`.
- `crates/spur-pm/src/beads_crate/issue_tracker.rs:465` — `update_issue` threads `update.external_ref` into `beads_rust::storage::sqlite::IssueUpdate.external_ref`.
- `crates/spur-pm/src/beads_crate/issue_tracker.rs` `br_to_pm_issue` — copy the three provenance fields off the `beads_rust::model::Issue` onto the returned `Issue`.
- `crates/spur-pm/src/beads_crate/adapter.rs:570` — snapshot conversion mirrors the same three fields.
- `crates/spur-pm/src/graph_engine/snapshot.rs:397` — same.
- `crates/spur-pm/src/test_workspace.rs:116` — default to `None`; tests opt in.

**Idempotency contract on `create_issue`:** when `external_ref` is `Some`, the adapter does a `find_by_external_ref` first and returns the existing id if found (status quo today: a UNIQUE constraint violation surfaces as an error). This is the *primary* idempotency guarantee for ingest re-runs.

## 4. State storage — `.beads/beads.db` only

**There is no sidecar database.** Every fact ingest cares about — provenance, sync watermark, link state, imported-comment idempotency — is stored in `.beads/beads.db` using primitives that `beads_rust` already provides:

- **Provenance** uses the existing `external_ref` (UNIQUE partial index), `source_system`, and `source_repo` columns on `issues`.
- **Watermark + link state** live in a per-issue `spur-sync v1` sentinel comment, written via `BeadsAdvanced::add_comment`.
- **Imported-comment idempotency** uses an embedded `<!-- spur-import gh:<node_id> -->` marker on the first line of each imported comment.
- **Sync run history** is out of scope for Phase 1 — `IngestReport` is the only artifact, returned to the CLI.

This preserves invariant I-1 literally. One file, one write lock, one set of backups, one consistent crash boundary. The cost paid: an extra `list_comments` per issue during ingest (~sub-ms each in WAL mode). The cost not paid: a two-store consistency problem.

### 4.0 Sentinel-comment pattern: consistent with existing conventions

A reviewer can reasonably ask: "Isn't storing structured key-value records inside the `comments` column a hidden schemaless store, violating I-1 in spirit even if it preserves it in letter?" Three reasons this is not a novel co-option:

1. **The pattern already exists.** The Spur codebase uses `spur-audit v1` comments to record brain decisions (approvals, rejections, completions) and `spur-signal v1` comments for worker→brain communication (scope drift, blocked, risk). Those sentinels are documented in `AGENTS.md` and load-bearing for retry, review, and lineage. `spur-sync v1` is consistent with that idiom, not a departure.
2. **`BeadsAdvanced::list_comments` + `add_comment` are first-class trait methods**, designed for exactly this kind of structured per-issue annotation. Reading and appending sentinels is a supported API, not a workaround.
3. **The append-only audit trail is a feature, not a bug.** Every state transition leaves a record. Lineage of "when did we first see this remote node, and what version each time?" comes for free. The Phase 2 column migration in §4.5 preserves these audit comments after switching reads to columns.

What stays true: the read path *is* an N+1 scan today. That's a real cost, captured as R-4 in §13 with a tight (<500ms-for-1k-issues) acceptance gate. If we miss the gate in practice, §4.5 fast-tracks. Until then, **the column-vs-comment choice is a performance trade with a fallback, not a correctness compromise.**

### 4.1 Per-issue sync watermark — `spur-sync v1` sentinel comment

Body format (one comment per ingested issue per ingest run; append-only):

```
spur-sync v1
source_system:                  github
remote_id:                      I_kwDOExample123
remote_number:                  42
remote_etag:                    W/"abc123def456"
remote_updated_at:              2026-05-10T12:00:00Z
last_synced_at:                 2026-05-12T03:00:00Z
last_synced_remote_updated_at:  2026-05-10T12:00:00Z
state:                          active
```

Reads — to load the current watermark for issue `bd-XYZ`:

```rust
let comments = adv.list_comments("bd-XYZ").await?;
let latest = comments.iter().rev()
    .find(|c| c.body.starts_with("spur-sync v1\n"))
    .map(parse_sync_comment);
```

Newest by `created_at` wins. If none exists, the issue has never been synced — treat as first-sight.

Writes — on every successful ingest/update of the issue, **append** a new sentinel via `BeadsAdvanced::add_comment`. Append-only by design: prior watermarks form a per-issue audit trail of when each remote version was observed. Phase 2 may add a compaction pass that prunes all but the newest `spur-sync v1` comment per issue.

### 4.2 Per-imported-comment idempotency — embedded marker

Each remote comment is imported into Beads as:

```
<!-- spur-import gh:IC_kwDOComment123 -->
imported from https://github.com/owner/repo/issues/42#issuecomment-123 by gh:alice (2026-05-10T12:00:00Z):

<remote comment body verbatim>
```

The first line is an HTML comment so it renders unobtrusively in any Markdown viewer (including `bd show`). The `<!-- spur-import gh:<id> -->` marker is the dedup key.

Idempotency check before importing a remote comment:

```rust
let comments = adv.list_comments(beads_id).await?;
let already_imported: HashSet<&str> = comments.iter()
    .filter_map(|c| c.body.lines().next()
        .and_then(parse_import_marker))   // returns Some("IC_kwDO...") on match
    .collect();
if !already_imported.contains(remote_comment_node_id) {
    adv.add_comment(beads_id, &format_import(remote_comment)).await?;
}
```

For an issue with N existing comments and M new remote comments, this is one `list_comments` call plus O(N) parse. Acceptable for Phase 1. Phase 2 can add `BeadsAdvanced::find_comment_by_external_marker` for a SQL `LIKE` shortcut.

### 4.3 `link.state` lifecycle

Encoded as the `state:` field of the latest `spur-sync v1` comment. Same state machine, no separate storage:

```
        ingest first sight
            │
            ▼
       ┌─────────┐  401  ┌────────────┐
       │ active  │◄──────│ needs_auth │
       └──┬──────┘       └────────────┘
          │ 404 / repo private
          ▼
   ┌──────────────┐
   │ disconnected │
   └──────────────┘
```

- `active`: ingest is working; last sync succeeded.
- `needs_auth`: a 401 was observed against this remote node.
- `disconnected`: a 404 / private-repo error was observed.

Every state transition emits a new `spur-sync v1` comment with the updated `state:` field. `BeadsAdvanced::list_comments` filtered for the sentinel is the source-of-truth query. Phase 1 wires `active` and `disconnected` transitions; `needs_auth` lands with the Phase 2 failure-state spec — no schema change required, only new code that writes the new state value.

### 4.4 Identity Registry

The doc's "Identity Registry mapping `beads_id <-> (source_system, remote_id, remote_version_hash)`" maps to:

| Doc field | Implementation |
|---|---|
| `beads_id` | `issues.id` |
| `source_system` | `issues.source_system` (existing column) |
| `remote_id` | encoded in `issues.external_ref` as `"github:<node_id>"` (UNIQUE indexed) |
| `remote_version_hash` | `remote_etag` field of the latest `spur-sync v1` comment |
| watermark | `last_synced_remote_updated_at` field of the latest `spur-sync v1` comment |

Lookup by remote id: `find_by_external_ref("github:<node_id>")` — already exposed at the `beads_rust` storage layer; Phase 1 surfaces it through `spur-pm` (see PR-1).

Lookup by local id → current watermark: `list_comments(beads_id)` + filter.

### 4.5 Future upstream optimization (Phase 2 or later, not Phase 1)

Once the design is proven in production, an optional upstream `beads_rust` PR can add indexed columns for hot watermark fields:

```sql
ALTER TABLE issues ADD COLUMN remote_etag TEXT;
ALTER TABLE issues ADD COLUMN last_synced_at DATETIME;
ALTER TABLE issues ADD COLUMN last_synced_remote_updated_at DATETIME;
ALTER TABLE issues ADD COLUMN link_state TEXT;
```

Migration is one-time and fully reversible: scan all `spur-sync v1` comments, project the latest per issue into the new columns, then stop emitting new sentinels (existing ones remain as audit history). Reads switch from comment-scan to column-read. **Until then, the sentinel approach is correct and complete — it is not a workaround, just a slower read path that needs no upstream coordination.**

## 5. Ingest flow — `crates/spur-pm/src/ingest/mod.rs`

### 5.1 Entry point

```rust
pub struct IngestOptions {
    pub since: Option<DateTime<Utc>>,
    pub label_namespace: String,      // default "gh"
    pub auto_label: Option<String>,   // default Some("spur-managed"), like GitHubAdapter today
    pub dry_run: bool,
}

pub struct IngestReport {
    pub run_id: i64,
    pub source_system: &'static str,
    pub source_repo: String,
    pub ingested: usize,    // newly created
    pub updated: usize,     // existed, mutated
    pub unchanged: usize,   // existed, no diff
    pub conflicts: Vec<RemoteConflict>,
    pub deletions: Vec<RemoteRef>,
    pub dep_hints_added: usize,
    pub comments_added: usize,
}

pub async fn apply_remote_delta(
    pm: &PmService,
    sync: &dyn ExternalPmSync,
    delta: RemoteDelta,
    opts: &IngestOptions,
) -> SyncResult<IngestReport>;
```

`apply_remote_delta` is the only mutation entry point. The CLI calls `sync.fetch_changes_since(opts.since).await?` then this function.

### 5.2 Apply sequence (per node)

> **Concurrency contract:** the entire apply loop runs under an **exclusive process-level lock** `.beads/.spur-ingest.lock` (separate from beads's `.write.lock`; held for the duration of the whole run, not per-node). A second concurrent `spur pm ingest` against the same `.beads/` fails fast with "another ingest run is in progress (pid=N)" rather than racing on watermark reads. The per-node `adapter.write` closure additionally **re-reads the watermark inside the closure** as defense-in-depth — if the lock were ever bypassed, the in-closure re-read still catches the staleness before the write commits.

```
acquire .beads/.spur-ingest.lock (exclusive, blocking with 30s timeout)
if fail: exit with "another ingest run is in progress (pid=N)"

For each node in delta.nodes:
  # Cheap-path preview (read-only, no lock acquired beyond beads's
  # routine read snapshot). Lets us skip the write lock entirely for
  # unchanged nodes — the cheap-path is advisory; the in-closure re-read
  # is the source of truth.
  preview_existing = pm.find_by_external_ref("github:" + node.remote_id).await?
  if let Some(e) = preview_existing:
    preview_wm = read_latest_sync_comment(e.id)?
    if preview_wm.last_synced_remote_updated_at == node.updated_at
        AND (node.etag.is_none() || preview_wm.remote_etag == node.etag):
      report.unchanged += 1; continue

  # Mutating path: every read is re-done inside adapter.write so that
  # the watermark seen at decision time is the watermark held under the
  # write lock. Defense in depth in case the process lock is bypassed.
  adapter.write(|s| {
    existing = s.find_by_external_ref("github:" + node.remote_id)?
    watermark = match existing {
      Some(e) => parse_latest_sync_comment(s.list_comments(e.id)?),
      None    => None,
    };

    if existing.is_none() {                            # NEW
      create = mapping::to_issue_create(node, opts)
                  with external_ref  = "github:<remote_id>"
                       source_system = "github"
                       source_repo   = "<owner>/<repo>"
      beads_id = s.create_issue(create)?  # idempotent on external_ref
      write_sync_sentinel(s, beads_id, node, state=Active, now=Utc::now())
      apply_dep_hints(s, beads_id, node.dep_hints)
      import_comments(s, beads_id, node.comments)
      report.ingested += 1
    }
    else if watermark.is_some() {                      # EXISTING WITH WM
      let wm = watermark.unwrap();
      diff = mapping::diff_against_local(existing.unwrap(), node)
      if diff.is_empty() {
        # remote.updated_at moved but mapped fields didn't (e.g. a
        # reaction). Still refresh the sentinel so the next sync can
        # short-circuit on the cheap path.
        write_sync_sentinel(s, existing.id, node, state=wm.state, now)
        report.unchanged += 1
      } else if is_field_level_conflict(&diff, &existing, &wm, node) {  # §5.4
        report.conflicts.push(make_conflict(existing, wm, node, diff))
        # Do not write. Beads + watermark untouched.
      } else {
        s.update_issue(existing.id, diff.to_issue_update())?
        write_sync_sentinel(s, existing.id, node, state=Active, now)
        apply_new_dep_hints(s, existing.id, node.dep_hints)
        import_new_comments(s, existing.id, node.comments)
        report.updated += 1
      }
    }
    else {                                             # EXISTING WITHOUT WM
      # Issue created by ingest in a prior version, or by a manual
      # `bd` op with the same external_ref. We have no prior reference
      # point, so we cannot detect conflict here. Adopt the remote as
      # baseline; record the first sentinel.
      diff = mapping::diff_against_local(existing.unwrap(), node)
      if !diff.is_empty() {
        s.update_issue(existing.id, diff.to_issue_update())?
        report.updated += 1
      } else {
        report.unchanged += 1
      }
      write_sync_sentinel(s, existing.id, node, state=Active, now)
      apply_dep_hints(s, existing.id, node.dep_hints)
      import_new_comments(s, existing.id, node.comments)
    }

    Ok(())
  })?

For each ref in delta.deletions:
  adapter.write(|s| {
    existing = s.find_by_external_ref("github:" + ref.remote_id)?
    if let Some(e) = existing {
      write_sync_sentinel(s, e.id, /*etag*/ None, state=Disconnected, now)
      s.add_comment(e.id, "spur-audit v1\nkind:disconnected\n...")
      report.deletions.push(ref)
    }
    Ok(())
  })?

release .beads/.spur-ingest.lock
```

`write_sync_sentinel` is a thin helper over `BeadsAdvanced::add_comment` that formats the `spur-sync v1` body per §4.1.

### 5.3 Comment ingestion

Per-comment idempotency uses the embedded marker from §4.2 — no second store needed. The flow for each `RemoteComment` on an issue:

```
existing_markers = scan_import_markers(adv.list_comments(beads_id).await?)
for rc in node.comments:
    if existing_markers.contains(rc.remote_id): continue
    adv.add_comment(beads_id, format_import(rc)).await?
```

`format_import` produces the body documented in §4.2, with the `<!-- spur-import gh:<id> -->` first line. `scan_import_markers` is a 10-line helper over `Vec<Comment>` that returns `HashSet<String>` of GitHub comment node_ids.

For an issue with N existing Beads comments and M new remote comments, this is one `list_comments` call plus M inserts. Bounded scan; acceptable for Phase 1.

### 5.4 Conflict detection — field-level three-way merge

A naive timestamps-only detector (`remote.updated_at > watermark.last_synced_remote_updated_at && local.updated_at > watermark.last_synced_at`) is wrong: GitHub bumps `Issue.updated_at` for unrelated activity (added comments, reactions, label edits by bots) and Beads bumps `Issue.updated_at` for any column change. Two disjoint edits — local priority bump + remote comment add — would falsely conflict and the remote comment would be silently dropped.

Phase 1 detects conflict at **field level**, by intersecting the mutated-fields sets on both sides:

```rust
/// Returns true iff local and remote both mutated the SAME mapped field
/// since the last successful sync. Disjoint mutations are not conflicts.
fn is_field_level_conflict(
    diff: &MappedDiff,
    local: &Issue,
    watermark: &SyncWatermark,
    remote: &RemoteNode,
) -> bool {
    // Fields the remote changed (computed by diff_against_local):
    let remote_changed: FieldSet = diff.remote_changed_fields();

    // Fields the local moved since last sync. Phase 1 conservatively
    // marks the whole "user-mutable" field set as local-changed when
    // local.updated_at > watermark.last_synced_at, because Beads does
    // not record per-field change timestamps. Refined in Phase 2 once
    // beads_rust exposes per-field history (or we add a sentinel-based
    // local change log).
    let local_might_have_changed_any_field =
        local.updated_at > watermark.last_synced_at;
    if !local_might_have_changed_any_field {
        return false;  // remote-only changes: take remote, no conflict.
    }
    let local_changed: FieldSet = FieldSet::user_mutable();

    !remote_changed.intersection(&local_changed).is_empty()
}
```

`FieldSet` enumerates the mapped fields the diff can touch (`title`, `description`, `status`, `priority`, `assignee`, `labels`). `MappedDiff::remote_changed_fields()` is computed during `mapping::diff_against_local` (a small extension over what diff already produces).

Phase 1 behavior:

- **Disjoint mutations** (e.g., remote adds a comment, local bumps priority) → not a conflict. Apply remote-side updates; preserve local field. Both `update_issue` and `import_comments` proceed.
- **Same-field collision** (e.g., both sides edited `title`) → `RemoteConflict { fields: ["title"], ... }`. Skip the write. Beads + watermark untouched. Report counted.
- **Remote-only changes** → apply (cheap path or normal update).
- **Local-only changes** → diff is empty on the remote side; we still refresh the sentinel so the next sync short-circuits, but no Beads write occurs.

Phase 2 refinement: with per-field local change tracking, `local_changed` becomes a real subset instead of "all user-mutable fields when updated_at moved." This narrows the false-conflict surface further — important once push direction (`push_mutations`) is wired and human edits are common.

When a conflict is surfaced, the Beads issue and the local watermark are preserved as-is. Conflict resolution UX (TUI / brain decision flow) lands in Phase 2.

### 5.5 Locking & transaction discipline

One lock: `.beads/.write.lock`, held by `BeadsCrateAdapter::write`. The apply step composes adapter methods; per-node it runs:

```
adapter.write(|s| {
    create_or_update_issue
    add_or_skip_imported_comments
    add_dep_hint_sentinels
    add_sync_sentinel
})
```

Every per-node update is one BEGIN/COMMIT in `beads_rust`. If the process crashes mid-batch, partial nodes are persisted but each persisted node is internally consistent. Re-running ingest is safe because:

1. `find_by_external_ref` returns the partially-applied issue.
2. The latest `spur-sync v1` sentinel records the remote state we'd persisted.
3. Re-ingest of an unchanged remote short-circuits via §5.2 (a).

**No two-store consistency problem exists.** This is the architectural reason for the §4 redesign.

### 5.6 Dependency hints

`dep_hints::extract(body, timeline) -> Vec<DepHint>` runs after the node is materialized in Beads. Hints are persisted as **immutable** structured comments with sentinel `spur-dep-hint v1`:

```
spur-dep-hint v1
kind:           closes
remote_keyword: Closes
remote_ref:     owner/repo#42          # canonical form, see below
remote_node:    I_kwDO...              # source issue/PR node_id (containing the hint)
source:         body | timeline_item
raw_span:       Closes #42
```

**Note: no `resolved_beads_id` field.** Resolution to a local `beads_id` is deferred to **read time**, not write time, for three reasons:

1. The append-only sentinel comment cannot be mutated later — `BeadsAdvanced` exposes `add_comment` but no `update_comment` or `remove_comment`. A write-time resolution that needs to be updated later (because a referenced node lands afterwards in the same batch, or in a future ingest run) cannot fix itself without appending a second hint comment, which would duplicate the hint in every UI that lists them.
2. The `external_ref` UNIQUE partial index makes a live lookup cheap: `find_by_external_ref("github:" + remote_node_id)` is O(log N) on a B-tree, executed at most once per hint per query.
3. The set of "resolvable" hints grows over time as more repos are ingested. A hint written today as `unresolved` may resolve next month; deferring resolution lets that happen automatically with no migration.

**Canonical `remote_ref` form.** The extractor normalizes all refs to `<owner>/<repo>#<number>`:
- Cross-repo refs (`owner2/repo2#7`) → preserved as written.
- Bare numeric refs (`#42` in body text) → expanded to `<current source_repo>#42`.
- Refs sourced from `timelineItems` (already-resolved by GitHub) include both the canonical form AND the source node_id (preferred when present).

**Resolution at read time.** New `BeadsAdvanced` helper (lands with the dep-hints PR):

```rust
pub struct ResolvedDepHint {
    pub hint: DepHint,                       // parsed from the sentinel
    pub resolved_beads_id: Option<String>,   // live lookup; None if unresolvable
}

#[async_trait]
impl BeadsAdvanced for BeadsCrateAdapter {
    /// List dep hints on an issue, with live resolution against the
    /// current external_ref index. Reads only; never mutates.
    async fn list_dep_hints(&self, beads_id: &str)
        -> Result<Vec<ResolvedDepHint>>;
}
```

`list_dep_hints` scans the issue's comments for `spur-dep-hint v1` bodies, parses each, and for every hint attempts:
- If `remote_node_id` is present: `find_by_external_ref("github:" + node_id)` → resolved.
- Else: derive `(source_repo, number)` from `remote_ref`, then `list_issues(filter { source_repo, external_ref LIKE "github:%" })` and walk results checking the number. Phase 1 accepts that this is the slow path; Phase 2 can add an indexed `(source_repo, remote_number)` lookup helper.

Hints **never** mutate the local DAG. The brain consumes `list_dep_hints` and decides whether to call `IssueTracker::add_dependency`. The grep gate from A-8 verifies no automated edge creation.

## 6. Failure model and `SyncError` mapping

| `SyncError` | Source observable | Apply-step behavior | Surfaced to user |
|---|---|---|---|
| `NeedsAuth` | 401 | Abort run cleanly; mark in-flight links → no state change; set link.state=`needs_auth` only after the Phase 2 spec lands | "Authentication failed. Run `gh auth login` or `spur pm reauth github`." (exit 1) |
| `Gone(remote_id)` | 404 on a previously-known node | Mark link.state=`disconnected`; append one `spur-audit kind:disconnected` comment; Beads issue untouched | Counted in `IngestReport.deletions`. |
| `RateLimited{retry_after_s}` | 403 with `Retry-After` or `x-ratelimit-remaining=0` | Sleep `min(retry_after_s, 600)`; retry once; on second hit, abort with `RateLimited` propagated to CLI | "GitHub rate limit hit; retrying in N seconds." (single live update line) |
| `Transient` | Network timeout, 5xx | Three-attempt exponential backoff (250ms, 1s, 5s); on third failure, propagate | "Network error; partial progress saved. Re-run to resume." (exit 2) |
| `Malformed` | JSON parse failure / schema mismatch | Abort run; do not mutate further; log the response excerpt at WARN | "GitHub returned an unexpected response. Please file a bug." (exit 3) |
| `Other` | anything else | Abort, propagate | Default message + cause chain. |

Crucially: every variant except `Malformed` leaves the system in a **resumable** state. Re-running `spur pm ingest github X/Y` is always safe.

## 7. GitHub adapter — `crates/spur-pm/src/ingest/github/`

### 7.1 `auth.rs`

```rust
pub async fn resolve_token() -> Result<GitHubToken, AuthError>;

pub struct GitHubToken {
    pub token: String,
    pub source: TokenSource,
}

pub enum TokenSource {
    GhCli,        // shelled out to `gh auth token`
    DeviceFlow,   // ran OAuth Device Flow
    EnvVar,       // SPUR_GITHUB_TOKEN (escape hatch for CI)
}
```

Resolution order:

1. `SPUR_GITHUB_TOKEN` env var — for CI and power users. Skipped if empty.
2. `gh auth token` — `Command::new("gh").args(["auth", "token"]).output()`. Trim stdout; ignore stderr. If the binary is missing or the call returns non-zero, fall through.
3. OAuth Device Flow — prints the user code + URL, polls the token endpoint. Stores the resulting token in the OS keyring (via the `keyring` crate, behind a config flag — Phase 2 wires the persistence; Phase 1 may hold the token in process only and ask again next run).

No PAT prompt. If all three fail, return `AuthError::NoTokenSource` and the CLI prints a one-paragraph remediation.

### 7.2 `client.rs`

Wraps `octocrab::Octocrab`. Two responsibilities:

R-7.2.1 Rate-limit governor. Every response runs through `Governor::observe(headers)`, which extracts `x-ratelimit-remaining`, `x-ratelimit-reset`, and (for GraphQL) the `rateLimit { remaining resetAt cost }` block from the JSON. When `remaining` drops below a configurable floor (default 50 for REST, 100 points for GraphQL), `Governor::throttle().await` sleeps until reset. Implementation uses `tokio::time::sleep_until` with a `Notify` so concurrent requests share the same wake.

R-7.2.2 GraphQL execution. Wraps `octocrab.graphql::<T>(query)` so the client returns `Result<T, SyncError>` instead of `octocrab::Error`. Concrete error mapping table:

| `octocrab::Error` shape | `SyncError` |
|---|---|
| `GitHubError { status: 401, .. }` | `NeedsAuth` |
| `GitHubError { status: 403, headers: rate-limit-remaining=0 }` | `RateLimited { retry_after_s }` |
| `GitHubError { status: 403, headers: retry-after=N }` | `RateLimited { retry_after_s: N }` |
| `GitHubError { status: 404, .. }` | `Gone(remote_id)` |
| `Hyper(_)` / connection reset | `Transient(...)` |
| `Serde(_)` / `Json(_)` | `Malformed(...)` |
| anything else | `Other(...)` |

> **Crate-API note:** Verify the exact variant names against the current `octocrab` release at implementation time — they've moved between versions. The mapping *intent* is fixed even if the syntax shifts.

### 7.3 GraphQL ingest query

GitHub GraphQL **connection cursors are connection-specific** opaque strings — an `endCursor` returned by `issues` cannot be passed to `pullRequests`. The query takes a separate `$issueCursor` and `$prCursor`, and the two connections paginate independently on the client side.

`ingest/github/graphql/ingest_repo.graphql`:

```graphql
query IngestRepo(
  $owner: String!,
  $repo: String!,
  $issueCursor: String,
  $prCursor: String,
  $issuePageSize: Int!,
  $prPageSize: Int!,
  $commentsFirst: Int!,
  $timelineFirst: Int!,
) {
  repository(owner: $owner, name: $repo) {
    issues(first: $issuePageSize, after: $issueCursor,
           orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        id number title body url state stateReason
        createdAt updatedAt
        author { login }
        assignees(first: 10) { nodes { login } }
        labels(first: 30)    { nodes { name } }
        comments(first: $commentsFirst) {
          pageInfo { hasNextPage endCursor }
          nodes { id author { login } body createdAt updatedAt }
        }
        timelineItems(first: $timelineFirst,
            itemTypes: [CROSS_REFERENCED_EVENT, CLOSED_EVENT]) {
          nodes {
            __typename
            ... on CrossReferencedEvent {
              source { __typename
                ... on Issue       { id number repository { nameWithOwner } }
                ... on PullRequest { id number repository { nameWithOwner } }
              }
            }
            ... on ClosedEvent { stateReason }
          }
        }
      }
    }
    pullRequests(first: $prPageSize, after: $prCursor,
                 orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        id number title body url state isDraft
        createdAt updatedAt
        author { login }
        assignees(first: 10) { nodes { login } }
        labels(first: 30)    { nodes { name } }
        comments(first: $commentsFirst) {
          pageInfo { hasNextPage endCursor }
          nodes { id author { login } body createdAt updatedAt }
        }
        closingIssuesReferences(first: 10) {
          nodes { id number repository { nameWithOwner } }
        }
      }
    }
    rateLimit { cost remaining resetAt nodeCount }
  }
}
```

Pagination strategy:

P-1. **Independent cursors.** Issues and PRs paginate as two separate cursored loops. The query is called with one cursor advancing per call; the other connection still re-fetches its first page each call but the client discards it (cheap because GitHub deduplicates work for identical sub-queries and the inner node counts dominate cost). When the issues loop completes, subsequent calls pass `$issuePageSize: 0`; same trick for PRs.

P-2. **Cost-aware default page sizes.** Defaults: `$issuePageSize = 25`, `$prPageSize = 25`, `$commentsFirst = 30`, `$timelineFirst = 20`. A single page now requests `25 × (10 + 30 + 30 + 20) ≈ 2,250` nodes per connection, ~45 rate-limit points per call (GitHub bills roughly 1 point per 100 nodes). That sits well under the 5% floor of 250 points/hour budget.

P-3. **Adaptive shrink.** The client observes `rateLimit.cost` on every response. If a page's cost exceeds the configurable floor (default `rate_limit_floor_graphql_points = 100`, see §9), halve `*PageSize` on the next iteration. If even `*PageSize = 1` busts budget, abort cleanly with `SyncError::RateLimited` and let the user re-run with `--since` to narrow.

P-4. **Inner-connection truncation.** Nodes with `comments.pageInfo.hasNextPage = true` or `timelineItems.pageInfo.hasNextPage = true` trigger a follow-up `IngestNodeExtras($id, $commentsCursor, $timelineCursor)` query (defined alongside `ingest_repo.graphql`) to drain the remaining pages. Phase 1: emit a warning if any node exceeds the per-page limits; the follow-up query lands as a small in-Phase-1 PR if smoke tests hit truncation, otherwise defers to Phase 2.

`graphql_client::GraphQLQuery` generates the Rust types into `ingest/github/graphql/types.rs`. The `#[derive(GraphQLQuery)]` macro consumes the `.graphql` file at compile time; the two connection cursors map to two `Option<String>` fields on the generated `Variables` struct, which the client populates from the issues and PR pagination loops.

### 7.4 `mapping.rs`

Pure functions, no I/O. `to_issue_create(node, opts) -> IssueCreate`, `to_remote_node(graphql_response_node) -> RemoteNode`, `diff_against_local(local: &Issue, remote: &RemoteNode) -> IssueUpdate`.

The mapping table (full canonical version, supersedes §3.3 of the research):

```
GitHub                          → Beads
─────────────────────────────────────────────────────────────────
node.id                         → external_ref = "github:" + node.id
"github"                        → source_system
"<owner>/<repo>"                → source_repo
node.title                      → title
node.body                       → description
node.url                        → url

# state
issue.state = OPEN              → Status::Open
issue.state = CLOSED,
  stateReason = COMPLETED       → Status::Closed
  stateReason = NOT_PLANNED     → Status::Closed + label "gh:not-planned"
  stateReason = REOPENED        → Status::Open
pr.state    = OPEN, isDraft=t   → Status::Draft
pr.state    = OPEN, isDraft=f   → Status::Open
pr.state    = MERGED            → Status::Closed + label "gh:merged"
pr.state    = CLOSED            → Status::Closed

# people
node.assignees[0].login         → assignee = "gh:" + login
node.assignees[1..].login       → labels: "gh:also-assigned:<login>"   (Phase 1: log + drop; revisit in Phase 2)

# labels
node.labels[].name              → labels: "gh:" + name
                                  AND infer IssueType (see below)
                                  AND infer Priority (see below)

# heuristics
labels contains "bug"           → IssueType::Bug
        "enhancement"|"feature" → IssueType::Feature
        "documentation"|"docs"  → IssueType::Docs
        "question"              → IssueType::Question
        "chore"                 → IssueType::Chore
        (PR with none above)    → IssueType::Feature
        (Issue with none above) → IssueType::Task

labels matching /^p[0-4]$|^priority[-/:]p?[0-4]/i  → Priority(0..4)
                                                     (label-driven; default Backlog)

# kind tag
PR                              → label "gh:pull-request"
Issue                           → label "gh:issue"

# timing
node.createdAt, node.updatedAt  → created_at, updated_at
```

Label namespace `gh:` is configurable via `IngestOptions.label_namespace`. The default makes collisions with Spur's own labels structurally impossible.

`diff_against_local` only emits an `IssueUpdate` for fields that actually changed — the goal is to keep beads_rust's audit history clean.

### 7.5 `dep_hints.rs`

Pure functions. Two extractors compose into the final `Vec<DepHint>`:

E-1. Body extractor: regex-driven over `RemoteNode.body`.

```
PATTERN_CLOSING:
  (?i)\b(?P<kw>close[sd]?|fix(?:es|ed)?|resolve[sd]?)\s+
  (?P<ref>(?:[\w.-]+/[\w.-]+)?#\d+)
  → DepHint { kind: closes|fixes|resolves, remote_ref: <ref>, source: Body }

PATTERN_DEPENDS:
  (?im)^\s*(?P<kw>depends\s+on|blocked\s+by|blocks)\s*:?\s+
  (?P<ref>(?:[\w.-]+/[\w.-]+)?#\d+)
  → DepHint { kind: depends-on|blocked-by|blocks, remote_ref: <ref>, source: Body }

PATTERN_TASKLIST:
  (?m)^\s*-\s*\[\s*[ x]\s*\]\s*(?P<ref>(?:[\w.-]+/[\w.-]+)?#\d+)
  → DepHint { kind: task-list, remote_keyword: "- [ ]", remote_ref: <ref>, source: Body }
```

E-2. Timeline extractor: walks `timelineItems.nodes`:

- `ClosedEvent.stateReason = COMPLETED` with `closer` pointing to a PR → DepHint { kind: closes, source: TimelineItem, remote_ref: "<pr.repo>#<pr.number>" } **on the issue side**.
- `CrossReferencedEvent` from a PR → DepHint { kind: closes (if the PR has a matching `closingIssuesReferences`) else depends-on, source: TimelineItem }.

Timeline extraction is preferred over body extraction when both produce a hint for the same `(remote_id, kind)` pair — it's GitHub's already-resolved view. Deduplicate at the end of extraction by `(remote_ref, kind)`.

This module emits `DepHint` records only — it never resolves `remote_ref` to a `beads_id`. Resolution is deferred to read time via `BeadsAdvanced::list_dep_hints`, per §5.6.

## 8. CLI surface — `crates/spur/src/cli/`

New subcommand:

```
spur pm ingest github <owner>/<repo>
    [--since <iso8601>]
    [--label-namespace <prefix>]      # default "gh"
    [--page-size <N>]                 # default 25 (see §7.3 P-2 for cost math)
    [--dry-run]
    [--json]                          # machine-readable report

spur pm ingest github --help          # standard help
```

Behavior:

- `--dry-run`: runs `fetch_changes_since` and the mapping step, prints the report, *does not* call `apply_remote_delta`.
- `--json`: prints `IngestReport` as JSON to stdout; exits 0 even if conflicts were detected (caller is responsible for inspecting the JSON).
- Default (human) output: a progress line per page, a final summary block, and a non-zero exit code if there were any conflicts or errors.

Wiring: the CLI calls `PmService::sync_target("github")` which (Phase 1) constructs a `GitHubSync` lazily from the existing `BeadsCrateAdapter`'s cwd. The `PmBackendInner` enum is **not** extended; `sync_target` is a separate accessor on `PmService` that returns `Option<Arc<dyn ExternalPmSync>>` and is gated on Beads being the active backend (preserves I-1, I-7).

```rust
impl PmService {
    pub fn sync_target(&self, source_system: &str) -> Option<Arc<dyn ExternalPmSync>> {
        match (&self.backend, source_system) {
            (PmBackendInner::Beads { .. }, "github") => Some(self.github_sync.clone()?),
            _ => None,
        }
    }
}
```

`self.github_sync` is `Option<Arc<GitHubSync>>` populated by `PmService::try_new_with_actor` when GitHub config is present (token resolvable, repo detected). When `None`, the CLI prints the auth/repo-detection error from §7.1 and exits.

## 9. Configuration

Lives in the existing Spur config file (no new file required). Schema:

```toml
[pm.github]
# Optional explicit repo when running `spur pm ingest github` without an arg.
default_repo = "getspur/SpurCode"

# Token resolution preference; overrides the default order from §7.1.
auth_preference = ["env", "gh_cli", "device_flow"]

# Rate-limit floors.
rate_limit_floor_rest = 50
rate_limit_floor_graphql_points = 100

# Default ingest options.
[pm.github.ingest]
label_namespace = "gh"
page_size = 25
auto_label = "spur-managed"

# Persisted token cache (Phase 2 wires this; Phase 1 may set it but ignores).
[pm.github.cache]
keyring_service = "spur.github"
```

All keys are optional; defaults match the constants in this spec.

## 10. Test plan

T-1 **Unit — mapping (`mapping_test.rs`):** fixture JSON for 12 representative issues and 6 PRs (open, closed-completed, closed-not_planned, draft, merged, reopened, multi-assignee, with all four label types, with priority labels, with no body). For each, assert that `to_issue_create` produces the expected `IssueCreate` and `to_remote_node` round-trips through `RemoteNode` losslessly except for declared drops.

T-2 **Unit — dep_hints (`dep_hints_test.rs`):** fixture bodies covering each pattern from §7.5. Add fuzz tests that confirm:
   - No panic on arbitrary UTF-8 bodies (proptest with arbitrary strings, 1000 iterations).
   - No extraction from inside fenced code blocks (\`\`\` … \`\`\`) — Phase 1 may accept this limitation; document it.
   - Deduplication when body and timeline emit the same hint.

T-3 **Unit — watermark (`watermark_test.rs`):** round-trip a `spur-sync v1` sentinel through `format_sync_sentinel` → `parse_sync_sentinel`. Cases: every state value, every optional field present/absent, malformed bodies returning `Err` cleanly, oldest-vs-newest selection across multiple sentinels on the same issue. Plus the marker parser from §4.2: extract marker from various leading-line shapes, reject lookalikes inside body text.

T-4 **Integration — apply_remote_delta (`apply_test.rs`):** in-memory `MockSync` returning a scripted `RemoteDelta`. Tests use a tempdir-backed `BeadsCrateAdapter`. Cases:
   - **Fresh repo:** 0 existing issues → N created, each with a `spur-sync v1` sentinel.
   - **Re-ingest unchanged:** cheap-path short-circuit hits; `unchanged == N`.
   - **Re-ingest with mutated remote:** titles/labels/state change → updated with correct diffs; new sentinel appended.
   - **Re-ingest with deletions:** simulated 404 surfaces as `Gone` → sentinel `state: disconnected`.
   - **Disjoint conflict (regression for the timestamps-only bug):** local edits `priority`, remote adds a comment (bumping `updated_at` but not touching mapped fields). Field-level detector in §5.4 must classify as **not a conflict** and the new remote comment must land via `import_comments`. The previous timestamps-only logic would have flagged this and dropped the comment.
   - **Same-field conflict:** both sides edit `title` since last sync → `RemoteConflict { fields: ["title"], ... }` returned; Beads issue and sentinel untouched.
   - **Idempotency under partial-batch crash:** simulate by aborting after writing N/2 nodes; re-run, verify the remaining N/2 land cleanly and no duplicates appear (relies on `external_ref` UNIQUE index + the in-closure watermark re-read).
   - **Concurrent ingest runs (regression for the TOCTOU bug):** spawn two `apply_remote_delta` invocations against the same tempdir at the same time. The second must fail fast with "another ingest run is in progress" rather than racing on watermark reads. After the first completes, the second can be re-run and lands cleanly.
   - **Comment dedup:** import the same RemoteComment twice in two separate runs; second run is a no-op via §4.2 marker scan.
   - **Manual marker removal:** seed a Beads issue with an imported comment whose `<!-- spur-import gh:IC_… -->` first line has been manually stripped (simulating a human edit). Re-ingest the same `RemoteDelta`. Verify the system imports the comment again **without panicking or corrupting Beads** — the imported comment appears twice (documented limitation; the marker is fragile by design because we chose comments over a sidecar table). The test pins the failure mode as "duplicate, never corrupt."
   - **Recovery branch:** pre-create an issue manually with the right `external_ref` but no sentinel; first ingest must adopt it and produce the first sentinel without conflict.
   - **Single-store invariant (A-9):** after the run completes, assert the `.beads/` directory contents against the allowlist from A-9. Fail if any unexpected filename appears.

T-5 **Integration — auth (`auth_test.rs`):** mock `gh` binary via `PATH` prepending a shell script that prints a known token to stdout and a warning to stderr. Verify `resolve_token` returns the token cleanly.

T-6 **Smoke — live GitHub (`live_smoke.rs`, ignored by default):** runs against a real public read-only repo (e.g. `octocat/Hello-World`) with the developer's own token. Expects exit 0 and a non-empty report. Not run in CI; documented for contributors to verify locally.

T-7 **Snapshot — CLI output (`cli_snapshot.rs`):** insta snapshots of the human and `--json` outputs against scripted `MockSync` results.

All Phase 1 PRs must add to T-1 through T-5; T-6 is optional; T-7 lands with the CLI PR.

## 11. Acceptance criteria

Phase 1 is complete when, with all PRs merged on a clean checkout:

A-1. `cargo test -p spur-pm` and `cargo test -p spur` pass.
A-2. `cargo test -p spur-pm --test apply_test -- --ignored` (live smoke) passes against a known public repo when `SPUR_GITHUB_TOKEN` or `gh auth status` is set.
A-3. `spur pm ingest github octocat/Hello-World` on a fresh `.beads/` lands ≥1 issue with all three provenance fields populated and exactly one `spur-sync v1` sentinel comment per ingested issue. Idempotent on re-run (`bd list --label gh:issue` count is stable).
A-4. `bd show <ingested-id>` displays `external_ref`, `source_system`, `source_repo`. (Requires the provenance fields to be surfaced through whatever CLI / display path is wired; coordinate with `beads_rust` upstream if needed.)
A-5. Dep hints appear as `spur-dep-hint v1` sentinel comments queryable via `BeadsAdvanced::list_comments`.
A-6. Re-running ingest after a Beads-side title change without a corresponding remote-side change does *not* overwrite the local title (three-way merge per §5.4: local-moved-only → update is dropped). The reverse (remote-moved-only) takes remote. Both-moved → conflict, counted in `IngestReport.conflicts`, Beads untouched.
A-7. No clippy warnings on touched files.
A-8. No code path turns a `DepHint` into an `add_dependency` call — verified by a grep gate in CI.
A-9. **Single-store invariant.** After a full ingest run, `.beads/` contains only:
   - `beads.db` (plus `beads.db-wal`, `beads.db-shm` from SQLite WAL mode)
   - `.write.lock` (beads_rust's existing write lock)
   - `.spur-ingest.lock` (the new ingest-run flock from §5.2; released after the run completes, so absence is expected when no run is in flight)
   - any other files `beads_rust` itself creates upstream

   Specifically: **no `external_links.db`, no separate sync database, no per-repo ingest cache files.** Verified by an integration test that asserts the exact filename set against an allowlist (not a raw `ls`); the test fails if any unexpected filename appears, which catches a regression that accidentally re-introduces a sidecar.

## 12. Sequencing — implementation order

Each entry is one PR. They land in order; each PR is reviewable in isolation.

PR-1. **Surface provenance + `find_by_external_ref` on `spur-pm` types.** Touches `types.rs`, the five hardcoded `None` sites (issue_tracker.rs:325, 465; adapter.rs:570; snapshot.rs:397; test_workspace.rs:116), the adapter create/update/get paths. Adds `IssueTracker::find_by_external_ref` and threads it to `beads_rust`'s existing storage method. Tests: T-1 fixtures pass; existing tests unchanged. (~350 lines.)

PR-2. **`crates/spur-pm/src/sync.rs` + types.** Trait + all data types from §2. `MockSync` impl behind `#[cfg(test)]` for downstream tests. No GitHub yet. Tests: trait compiles, `MockSync` round-trips through the type model. (~400 lines.)

PR-3. **`ingest/watermark.rs` + `dep_hints.rs` + `ingest/mod.rs::apply_remote_delta`.** All state lives in `beads.db` via §4. Sentinel format helpers, marker scanner, three-way conflict detection, the per-node apply flow. Driven by `MockSync`. Tests: T-2, T-3, T-4. (~700 lines.)

PR-4. **`ingest/github/{auth,client,mapping}.rs` + GraphQL query.** No CLI yet. Tests: T-1 mapping fixtures, T-5 auth shim. Pins `octocrab` version and verifies the device-flow surface. (~700 lines.)

PR-5. **`PmService::sync_target` + CLI subcommand.** End-to-end wiring per §8. Tests: T-7 CLI snapshot; manual T-6 live smoke documented in the PR description. (~400 lines.)

Total: five PRs, ≈2,550 LoC. Each PR independently mergeable; behaviors are compile-only callsites until PR-5 exposes the CLI.

## 13. Risks and what we accept

R-1. **octocrab API drift.** Variant names and helper signatures have changed across releases; the spec flags this. Mitigation: PR-4 pins an exact `octocrab` version in `Cargo.toml` and verifies the device-flow API surface against the current docs before merging.

R-2. **GraphQL cost spikes on large repos.** A `pullRequests(first: 50)` query on a 50k-PR repo will exceed per-page budgets. Mitigation: §7.3 P-2 (adaptive page sizing). If even `pageSize=1` busts budget, the run aborts cleanly with `RateLimited`; user re-runs with `--since` to narrow.

R-3. **Comment / timeline truncation at 100 items per node.** Phase 1 accepts truncation with a warning. Mitigation: §7.3 P-3 follow-up `IngestNodeExtras` query, deferred. Real-world impact small for typical OSS issues.

R-4. **Watermark scan cost.** Phase 1 reads watermarks by scanning all comments per issue and filtering for `spur-sync v1`. For an issue with 200 comments this is fine; for a pathological 10k-comment thread it isn't. Mitigation: §4.5 future column migration. Acceptance: measure on `octocat/Hello-World` and a moderately-busy upstream (e.g. ~50-comment issues). With SQLite in WAL mode and N issues averaging C comments each, watermark reads should complete in **<500ms wall time per 1k issues** (≈0.5ms per issue). If we miss this gate, fast-track §4.5. The previous "30s for 1k issues" gate was a typo for "30s for 1M issues" and undersold the bar; this corrects it.

R-5. **Dep hint regex misfires.** Body parsing is heuristic; fenced code blocks aren't excluded in Phase 1. Mitigation: hints are never edges (I-8); brain has final say. Accept the false-positive risk in exchange for never blocking the DAG on misparses.

R-6. **Multi-assignee data loss.** Phase 1 drops `assignees[1..]` with a log line. Mitigation: re-visit in Phase 2 alongside the `also-assigned` label scheme. Accept.

R-7. **Token caching out of scope.** Phase 1 may ask for the device flow on every run if `gh` is not installed. Mitigation: keyring persistence in Phase 2. Accept short-term friction.

R-8. **Append-only sentinel growth.** Each ingest of an issue appends a new `spur-sync v1` comment. A weekly-polled issue accumulates ~52 sentinels per year. Mitigation: Phase 2 compaction pass that keeps only the newest sentinel per issue. Until then, accept — sentinels are short (≤300 bytes each).

## 14. What this spec does not cover (Phase 2+)

- ETag-based REST incremental polling (`spur pm sync github`). Adds a `fetch_one` REST path; the sentinel already carries `remote_etag` so no schema change is needed.
- Webhook receiver + smee.io relay. Adds `ingest/github/webhook.rs`; pushes deltas through `apply_remote_delta` unchanged.
- TUI surfacing of `state: needs_auth` / `state: disconnected` sentinels and the conflict review flow.
- Push direction (`push_mutations`): projecting Beads status/comment/label changes back to GitHub. The trait method exists; Phase 1 returns `Skipped { reason: "phase-2" }`.
- Sentinel compaction pass (per R-8).
- **Upstream `beads_rust` PR** adding indexed `remote_etag` / `last_synced_at` / `last_synced_remote_updated_at` / `link_state` columns to `issues`. One-time migration projects the latest sentinel per issue into the columns; reads switch from comment-scan to column-read. Optional optimization, not a correctness fix.
- Linear and Plane sync targets — they reuse `ExternalPmSync` unchanged.

Each of those gets its own design doc, layered on this one.

---

*Spec authored: 2026-05-12. Builds on research in `docs/research/github-ingestion-rust.md`. Companion to `docs/architecture/spur-pm-beads-source-of-truth.md`. Implementation epic: TBD — file with the punch list in §12 as PR descriptions.*
