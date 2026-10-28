# SpurCode Architecture

SpurCode is a Rust workspace for running agentic software work under explicit process
control. Most work is brain-led: a long-lived brain session reasons about a
task, calls MCP tools to inspect or delegate work, workers execute in isolated
git worktrees, and the orchestrator routes results through review, merge,
retry, or rejection paths. The same control plane also owns unattended L3
project loops, where a repository-scoped runtime can schedule maker and
reviewer delegations without an active brain ACP session.

The current workspace contains 20 member crates under `crates/`, plus `xtask`.
`crates/spur-context-service` also lives in the repository but is excluded from
the main workspace because it tracks a newer DuckDB stack than the graph/analyst
path used by the core workspace.

## System overview

```mermaid
flowchart LR
    USER[User] --> CLI[spur-cli]
    USER --> TUI[spur-tui]
    USER --> BOT[spur-bot]

    CLI --> CORE[spur-core orchestrator]
    TUI --> CORE
    BOT --> CORE

    CORE -->|ACP sessions| ACP[spur-acp]
    ACP --> BRAIN[Brain agent]
    ACP --> WORKERS[Worker agents]

    BRAIN -->|MCP tool calls| MCP[spur-mcp registry/server]
    MCP --> CORE
    MCP --> PM[spur-pm]
    MCP --> GRAPH[spur-graph]
    MCP --> ANALYST[spur-analyst]

    CORE --> WT[spur-worktree]
    CORE --> COST[spur-cost]
    CORE --> BLOB[spur-blob-store]
    CORE --> TELEMETRY[spur-telemetry]

    GRAPH --> ANALYST
    CORE --> FUNNEL[Event funnel]
    FUNNEL --> TUI
    FUNNEL --> LINEAGE[Lineage projections]
    FUNNEL --> SINK[Event sink]
```

At a high level, the workspace is split into five concerns:

1. Control plane: session lifecycle, delegation, review, plan scheduling, and
   unattended loop ownership.
2. Frontends: CLI, TUI, Telegram, and non-TUI bridges.
3. Execution infrastructure: worktree isolation, artifact storage, telemetry,
   licensing, and cost tracking.
4. Project intelligence: PM adapters, graph indexing, analyst queries, and
   local session analytics.
5. Supporting services: external package indexing and test/tooling crates.

## Core execution loop

The load-bearing path is:

1. Work enters `spur-core` either from a frontend (`spur-cli`, `spur-tui`,
   `spur-bot`) or from the loop scheduler when a durable L3 generation becomes
   due.
2. For ad-hoc, interactive, and L1/L2 loop work, `spur-core` creates or
   resumes a brain session through `spur-acp`. For unattended L3 work, the
   project runtime owns the generation directly under the stable
   `spur-loop-runtime` owner.
3. Brain-owned flows reason in ACP and cause side effects through MCP tool
   calls exposed by `spur-mcp`. Project-owned L3 flows instantiate persisted
   plan templates directly and skip the brain ACP channel.
4. Executable tasks become isolated worker runs: `spur-core` provisions a
   worktree through `spur-worktree`, starts worker sessions through `spur-acp`,
   and collects notifications, diffs, and artifacts.
5. All emitted `SpurEvent`s pass through a single event funnel inside
   `spur-core`, which stamps sequence numbers and timestamps before
   broadcasting an ordered runtime stream to subscribers.
6. Runtime consumers such as executor lineage, TUI state, and durable event
   sinks consume that stream. Durable plan snapshots still originate from
   beads-backed projection in `spur-mcp`; `spur-core` caches and reflects those
   already-shaped snapshots through `PlanSnapshotUpdated` events.
7. Review closes the loop: interactive work can merge, retry, reject, or
   preserve worktrees, while unattended L3 work adds an independent reviewer
   delegation that must submit a durable verdict before approval.

That produces two orchestration modes:

- Brain-owned runtime: ad-hoc runs, interactive sessions, and L1/L2 loops keep
  a brain session in the planner role and use MCP as the brain's side-effect
  surface.
- Project-owned L3 runtime: at most one elected interactive SpurCode process per
  repo holds `.spur/loop-runtime.lock` at a time, while additional interactive
  processes remain passive standbys. That leader schedules due generations,
  reconciles durable beads state under `spur-loop-runtime`, and dispatches a
  separate reviewer worker before unattended work is accepted; without a
  leader, or when advisory locking is unsafe, L3 work stays parked.

Two protocol planes stay deliberately separate:

- ACP (`spur-acp`) is the outbound control channel from SpurCode to agent sessions.
  It owns transports, session/config types, notifications, and compatibility
  adapters such as `stdio`, `cli_wrap`, `stream_json`, and native ACP.
- MCP is the inbound tool channel from the brain back into SpurCode. `spur-mcp`
  provides the JSON-RPC server, tool registry, and module interface used by
  domain crates such as `spur-core`, `spur-pm`, `spur-graph`, and
  `spur-analyst`.

Within `spur-core`, the most important subsystems are:

- `orchestrator`, `orchestrator/loop_runtime`, `scheduler`, `plan`,
  `plan/loops`, `plan/reconciler`, and `server/handlers` for session, plan,
  and loop flow
- `event_funnel`, `event_sink`, `event_replay`, `notification_pump`, and
  `notification_drain` for ordered event capture, replay, and delivery
- `lineage`, `plan_projection`, and `review_sink` for read models and review
  coordination
- `worker_server`, `peer_mailbox`, and `continuation_bridge` for runtime
  coordination with agents, reviewers, and worker tools
- `agent_profiles`, `skills`, `session_synopsis`, `outcome_materializer`,
  `explore`, and `server` for higher-level orchestration support

## Workspace map

| Area | Crates | Responsibility |
|---|---|---|
| Entry points and UX | `spur-cli`, `spur-tui`, `spur-interactive`, `spur-bot` | User-facing command entrypoint, ratatui dashboard, non-TUI bridge, and Telegram runtime |
| Control plane | `spur-core`, `spur-acp`, `spur-mcp` | Orchestration, agent session transport, MCP registry/server, plan scheduling/reconciliation, unattended loop runtime, review, and event flow |
| Execution infrastructure | `spur-worktree`, `spur-blob-store`, `spur-telemetry`, `spur-cost` | Worktree isolation, delegation artifacts, tracing/logging, pricing and cost tracking |
| PM and governance | `spur-pm`, `spur-license`, `spur-license-admin` | Issue/PR adapters, beads and GitHub backends, graph triage/planning tools, license policy and admin surface |
| Code and session intelligence | `spur-graph`, `spur-analyst`, `spur-context` | Worktree code graph, DuckDB-backed semantic/indexed queries, and local agent-session analytics |
| External package indexing support | `spur-context-source`, `spur-context-fetcher` | Source URL validation/rate limiting and archive fetch/normalization for external package indexing |
| Testing and tooling | `spur-test-madsim`, `xtask` | Simulation-heavy tests and repository automation |

Additional repository-local crate:

- `crates/spur-context-service`: AWS-backed external package indexing and
  serving service. It is intentionally excluded from the main workspace because
  it builds against a newer DuckDB/DuckLake stack than the main graph/analyst
  workspace.

## Key crate internals

The workspace is broad, but a few crates define most of the architectural
surface area.

### `spur-acp`

`spur-acp` is the agent communication substrate. Its top-level modules are
organized around adapters and domain types:

- `adapter`, `connection`, and `protocol` handle transport/runtime behavior
- `config`, `agents`, and `registry` load and select configured agents
- `domain`, `types`, `ext`, and `spur_agent_caps` define shared event and
  capability types
- `session_*`, `orphan_*`, `process_inspector`, and `profile_strategy` support
  lifecycle, liveness, and compatibility behavior around agent sessions

This crate is the common type boundary used by the CLI, TUI, orchestrator, and
MCP modules.

### `spur-mcp`

`spur-mcp` is intentionally small and generic. It provides:

- the JSON-RPC server/runtime in `server`
- tool registry and module composition in `registry`
- wire-level response/error types in `response`
- tool metadata/schema definitions in `tools` and `tool_schemas`

Domain crates plug into it by implementing `ToolModule`; `spur-mcp` itself does
not own plan logic, PM logic, or graph logic.

### `spur-pm`

`spur-pm` is the project-management facade used by the brain and orchestrator.
Its current structure includes:

- `service`, `adapter`, and `advanced` trait layers
- `beads_crate/` for the in-process beads backend
- `github` for GitHub-backed operations
- `graph`, `graph_engine`, and `mcp` for PM-adjacent planning/triage tools
- `sync`, `ingest`, `poll_cursor`, and locking utilities for reconciler-style
  flows

The beads path is no longer just a CLI shellout abstraction; the crate now owns
an in-process adapter with explicit WAL plus flock concurrency behavior.

### `spur-graph`

`spur-graph` is the worktree code-intelligence substrate. Its main modules are:

- `extract/` for tree-sitter extraction across supported languages
- `identity` and `content_hash` for stable symbol IDs and freshness hashes
- `schema` for the typed graph artifact model
- `store/` for Parquet artifacts and pointer files
- `query_client`, `selector`, `search`, `traversal`, and `validation` for
  navigation and verification
- `temporal` for rename/history tracking
- `mcp` for the `code_*` tool surface

This crate is where "what code exists, what calls what, and is the answer
fresh?" becomes a first-class workspace capability.

### `spur-analyst`

`spur-analyst` is a DuckDB-backed secondary index over graph and documentation
artifacts. Its module layout reflects that:

- `db`, `search`, and `embedding` for indexed query building
- `doc_nav` for documentation section search/navigation
- `pack` and `paths` for evidence packs, graph paths, and ranked context
- `mcp` for the semantic retrieval surface

In practice, this crate backs `knowledge_context_pack_2`, `doc_navigate`, and
read-only SQL over `.spur/analyst.duckdb`.

### `spur-context`

`spur-context` is separate from the code graph. It builds a local DuckDB
analytics database over agent session data and enriches it with cost data. Its
main modules are:

- `engine` for the blocking DuckDB engine and schema/view lifecycle
- `async_engine` for async access
- `reporter` for higher-level reports and aggregates
- `live` for polling-based session snapshots
- `extractors` for source-specific normalization

It currently normalizes multiple local agent data sources into a single query
surface and is consumed primarily by `spur-cli` and `spur-tui`.

## Code and session intelligence

SpurCode has two distinct intelligence stacks:

### 1. Worktree code intelligence

This path is optimized for understanding the repository itself.

- `spur-graph` builds and incrementally refreshes a multi-language fact graph
  over the current worktree.
- `spur-analyst` loads graph and documentation artifacts into DuckDB for
  semantic evidence packs, doc search, path analysis, and risk/community
  queries.
- MCP tool surfaces expose both precise graph navigation (`code_*`) and
  semantic retrieval (`knowledge_context_pack_2`, `doc_navigate`, SQL queries).

The repository's own retrieval guidance is graph-first: orient with a knowledge
pack, follow with exact `code_*` reads/callers/callees, and use analyst SQL when
the question is set-shaped or ranked.

### 2. Local session analytics

This path is optimized for understanding agent usage and cost over time.

- `spur-context` normalizes heterogeneous agent logs into DuckDB views and
  caches
- `spur-cost` provides pricing data and cost bookkeeping
- `spur-tui` and `spur-cli` consume the resulting reports for insights and cost
  views

These two stacks are related, but they solve different problems: one answers
questions about code structure, the other answers questions about agent work and
spend.

## External package indexing

The repository also contains a second code-intelligence plane for code outside
the current worktree.

- `spur-context-source` validates agent-supplied source URLs, infers whether a
  fetch should use git or an archive flow, rejects unsafe/private targets, and
  applies lightweight caller rate limiting.
- `spur-context-fetcher` is the fetch/archive normalization runtime used by the
  external indexing flow; it validates sources, fetches git or tarball content,
  normalizes it into an archive, and persists it to object storage.
- `spur-context-service` layers an AWS service over that substrate. Its ingest
  plane admits caller-scoped jobs into bounded DynamoDB queues, drains them
  through Step Functions-backed workers that fetch, graph-build, and publish
  bronze / silver / gold artifacts with Aurora as an ingest-only catalog, and
  short-circuits warm hits when cached package content can be reused. Its
  serving plane answers external package queries from immutable DuckLake
  snapshots on S3 and never queries Aurora directly.

This service crate is intentionally adjacent to the main workspace rather than
fully inside it.

## Frontends and external repos

SpurCode currently presents itself through several frontends:

- `spur-cli` is the main binary entry point
- `spur-tui` is the ratatui dashboard and interactive session UI
- `spur-interactive` is the bridge layer for non-TUI clients
- `spur-bot` is the Telegram runtime and command surface

The notebook frontend is no longer developed in this repository. `jute-notebook`
and the notebook-oriented UI source now live in the standalone
`getspur/spur-notebook` repository. This workspace consumes that work as an
external artifact and keeps `scripts/spur-pnpm` only as a compatibility wrapper
that forwards to a standalone notebook checkout when `SPUR_NOTEBOOK_REPO` is
set.

## Build, test, and development model

The project-standard cargo entry point is `scripts/spur-cargo`, not bare
`cargo`.

Architecturally, that script is part of the workspace contract:

- heavy compile commands (`build`, `check`, `test`, `doc`, `clean`, and cross
  builds) prefer a remote cloud-build path by default
- `clippy` is local-first unless `SPUR_REMOTE=1` forces remote execution
- `fmt` stays local
- `run` is remote by default and syncs worktree-relative output files back after
  execution
- `SPUR_REMOTE=0` or `SPUR_REMOTE=1` override routing per invocation

That wrapper lets the workspace keep large Rust, DuckDB, and cross-build
workloads out of fragile local `target/` directories while preserving a normal
developer-facing cargo interface.

## Where to drill down

This file is the top-level map. The deeper subsystem docs are:

- `docs/spur-core-architecture.md`
- `docs/architecture-spur-mcp.md`
- `docs/tui-architecture.md`
- `docs/spur-pm-beads-crate-architecture.md`
- `docs/spur-brain-worker-collaboration.md`
- `crates/spur-context/ARCHITECTURE.md`
- `crates/spur-graph/ARCHITECTURE.md`
- `crates/spur-context-service/docs/ARCHITECTURE.md`

Read those when you need subsystem-level invariants, state machines, concurrency
contracts, or service deployment detail.
