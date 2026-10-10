# Incremental beads graph design

Issue: bd-potu6. User requirement: retain the graph, update only relevant changes, and avoid whole-graph reloads/scans during ordinary retrieval and small edits.

## Contract

- A graph scope is the full beads graph or one label-induced graph.
- Initialize a scope once. Unchanged reads do not load issues, labels, dependencies, or recompute a whole-graph hash.
- Apply committed changes to affected IDs and their incident edges. Independent database writers participate without calling Spur APIs.
- No-op updates and unrelated tables do not advance graph state. A changed field used by graph reports is relevant, including updated_at.
- Issue deletion, tombstones, template membership, label membership and edge insertion/removal/type/endpoints are covered.
- Read a coherent SQLite transaction, stage a complete delta, then publish it under one engine mutex. A failed read cannot advance the cursor or partially publish.
- Keep reports fresh: graph storage is retained, but time-dependent analysis and report serialization still run when requested.
- Cold initialization and detected schema/database replacement recovery are explicit full-build boundaries.

## Storage and publication

A graph-owned rusqlite connection is necessary: the pinned beads_rust exposes neither a raw read transaction nor a reliable revision token. Its adapter read_snapshot proxy only counts issues.

Install namespaced SQLite metadata and triggers transactionally on first graph use, after the beads adapter has initialized the schema. A singleton revision increments only on changes to graph-relevant columns/rows in issues, labels, dependencies. A coalescing table stores each affected issue ID's last node revision and edge revision, plus an indexed last revision. It is not consumed/deleted by a reader, so multiple engines/processes have independent cursors. Rollbacks also roll back these records. Deleted IDs remain recorded so a lagging reader cannot miss removal; storage grows with distinct IDs, not mutation count.

Each scope owns a mutable dense petgraph graph, by-ID lookup and incremental fingerprint. Refresh reads only change records after its cursor, hydrates changed nodes by primary key, and fetches incident edges only when relation/membership changes require it. Removing a dense graph node repairs the swapped node's index. Scope membership checks are indexed label queries. Filtered scopes never decode unrelated issue/dependency payloads.

Cold label queries start from the label index and look up matching issues/dependency owners by ID. Repeated unknown-label requests may initialize an empty view again, but must not scan either graph table; an EXPLAIN QUERY PLAN regression checks this separately from decoded-row counters. Before decoding an incident edge's payload, both endpoints must belong to the transaction's projected graph, including staged membership changes.

The projection uses the pinned beads model's status, issue type and dependency type parsing, including aliases, case normalization and empty-assignee handling. Fields unused by the graph (for example dependency metadata and dependency creation time) are not decoded or fingerprinted. Graph retrieval is not a validator of every database field. Valid projected nodes and edges remain comparable to the legacy full loader.

A single blocking task holds the engine's mutex through refresh and report computation. Concurrent requests therefore share initialization and publication, without cloning the entire graph. This serializes reports on that engine; parallel analytics is outside this change. No async runtime thread blocks on SQLite or the mutex.

Schema changes reset affected engine state through explicit recovery. File identity changes reopen the database. SQLite file replacement while a transaction is open follows SQLite's own supported-operation limits.

## Fingerprint compatibility

The current sorted whole-graph SHA requires an O(V+E) pass after every edit. The incremental path uses a versioned, order-independent fingerprint of domain-separated node/edge SHA-256 records; XOR removes an old record and adds the replacement. It is a cache/change fingerprint, not an authentication primitive. Include actual projected node fields, sorted labels, edge endpoints and type. The returned opaque data_hash gains a version prefix; consumers must compare it as an opaque string and invalidate old values once. The legacy standalone full loader remains available for tests/oracle comparison.

## Boundaries and tradeoffs

Implementation stays in spur-pm. No changes to the reconciler or code-index graph, and no changes to the running process. Scope caches retain one graph per queried nonempty label scope; this may duplicate nodes across scopes. Empty unknown labels are not retained. Global analytics such as ranking may still traverse the graph when requested; incremental graph maintenance does not claim incremental PageRank or constant-time serialization.

## Validation

- Fail first on repeated GraphEngine retrieval reaching the old adapter loader.
- Expose actual graph work counters: full builds, reuses, delta refreshes, changed IDs, loaded nodes and loaded edges.
- Assert one-node edits do not increase full-build count or load unrelated nodes; unchanged/comment/config/no-op writes do no graph work.
- Exercise external connections, transactions/rollback, deletion and reintroduction of nodes, templates/tombstones, label movement, and all dependency mutations.
- Compare projected nodes/edges against the existing full loader after mutation sequences; independently compare cold and incremental fingerprints.
- Force a malformed affected row and prove failed refresh leaves the cursor intact; repair and retry.
- Concurrent requests build once; filtered scope changes stay isolated.
- Run spur-pm tests and appropriate lint via scripts/spur-cargo. Measure work counts on a larger fixture rather than claiming live-process speedups.

## Jev and solve

jev_compile returned gate=open with weakest confidence 0.92, model jev-1.13.0, selecting workflow.transition_allowed in verify mode. The executed request adds initial_state_allowed and safety_invariant. The six-step intended trace passed (sol_fcdaa9daa6a84df7); a trace that full-scans on the final unchanged read failed (sol_32e16c5c377849db). The post-implementation Jev gate also opened (weakest 0.91); cached solver replays passed/failed respectively as sol_39e55e0ab1b84607 and sol_d7bb274166e844be. Exact inputs, provenance and results are saved in docs/performance/2026-10-09-incremental-graph/solver-{pre,post}.json. These validate declared lifecycle traces, not Rust, SQL trigger completeness, concurrency or unbounded safety.
