# spur-analyst

Shared Rust query layer over `.spur/analyst.duckdb`.

`spur-analyst` is the **warm tier** of SPUR worktree code intelligence: a DuckDB-backed secondary index over graph and documentation artifacts. It backs semantic evidence packs, documentation navigation, graph-path / risk / community enrichment, and read-only analytic SQL.

It does **not** replace the exact graph tools (`code_*` in `spur-graph` / `spur-mcp`). Those remain the source of truth for current working-tree source and call-graph impact. Analyst results are bounded evidence for orientation and ranking.

```
spur-graph (petgraph, hot)  ◄── code_callers / code_callees / code_subgraph
       │
       └─ Parquet artifact ─►  analyst: DuckDB + DuckPGQ + Onager (warm)
                                       │
                                       └─ spur-analyst (this crate)
                                              ├── knowledge_context_pack_2
                                              ├── doc_navigate
                                              └── query (read-only SQL)
```

> **Related, not the same:** `spur-context` is a separate DuckDB stack for **agent session / cost analytics**. Schema SQL that builds `.spur/analyst.duckdb` lives under `crates/spur-context/analyst/` and is compiled into `spur-cli` / graph build — this crate **consumes** that DB.

---

## What it provides

| Surface | Role |
|---|---|
| **Hybrid / BM25 search** | Rank documentation section bodies and code symbol token text; optional vector re-rank via embeddings + Lance |
| **Evidence packs** | `knowledge_context_pack` / `knowledge_context_pack_2` — structured, bounded packs for agents (not free-form answers) |
| **Doc navigation** | `doc_navigate` — BM25 FTS over sections, or one-hop `Contains` expansion from a root section id |
| **Graph reasoning** | DuckPGQ paths (recursive-SQL fallback), Onager-backed scorecard / community / temporal sections |
| **Read-only SQL** | `query` tool against `.spur/analyst.duckdb` (writes rejected; row cap 1000) |
| **Worktree overlay** | Merge-on-read overlay so dirty worktrees can see delta analytics without rewriting the base DB |

---

## Module layout

```
src/
├── api.rs          # Public DTOs: scopes, intents, candidates, paths, risk/community rows
├── db/             # Read-only connections, paths, freshness, extensions, SQL row serialization
├── search/         # Context candidates (BM25/hybrid), graph candidates, confidence helpers
├── embedding/      # Owned helper runtime, mode selection and sidecar protocol (feature `embed`)
├── pack/           # Evidence-pack assembly: request parse, staleness, impact, graph reasoning
├── paths/          # DuckPGQ/recursive path queries + risk/community enrichment
├── doc_nav/        # Documentation section artifact open + FTS / tree navigation
├── overlay/        # Base + delta merge-on-read DuckDB sessions
└── mcp/            # AnalystMcpModule + tools (knowledge_context, doc_navigate, query)
```

### `db`

- Opens **read-only** DuckDB connections with resource caps (default **4GB** memory, **4** threads).
- Discovers the DB at `<worktree>/.spur/analyst.duckdb` (falls back to a parent `.spur` for nested worktrees).
- Loads extensions best-effort: **ICU**, **Lance** (hybrid ANN), **DuckPGQ** (community extension).
- Freshness gate compares analyst `graph_content_hash` to the live graph pointer.

### `search`

- `query_context_candidates` — primary BM25/hybrid retrieval into `KnowledgeQueryResult`.
- `query_graph_candidates` / `merge_graph_candidates` — graph-scoped candidates merged into pack evidence.
- Hybrid confidence thresholds differ from BM25 (`hybrid-*` grounding vs raw BM25 scores).

### `embedding` (feature `embed`, default on)

- `OwnedEmbeddingRuntime` owns and reuses a supervised helper; hosts explicitly await `shutdown`.
- `SPUR_ANALYST_EMBED_MODE=auto` uses a reachable external sidecar and otherwise the local helper; `sidecar` uses the external service only; `off` skips embeddings. Unavailable embeddings fall back to BM25.
- `inprocess` is a compatibility alias for the local supervised helper. It does not load the native model into the analyst host.
- `warm_embed_model(&runtime).await` warms an explicit owned scope. The stateless compatibility query facade uses a bounded scope per call.

### `pack`

Orchestrates a pack request end-to-end:

1. Resolve analyst DB path; return an “unavailable” pack if missing.
2. Optionally embed the query.
3. Open a pooled pack connection (base DB ± worktree overlay).
4. Retrieve candidates → exact-graph impact context → graph-reasoning sections.
5. Assemble v1 or v2 JSON with staleness metadata and `recommended_next_tools`.

**v2 sections** (bounded by request budgets):

| Section | Backing | How to read |
|---|---|---|
| `graph_paths` | DuckPGQ / recursive SQL | Routes between grounded candidates or anchors |
| `risk_scorecard` | Onager + `v_symbol_scorecard` | PageRank, degree, churn, posture — signals, not proof |
| `community_context` | Components / Louvain | Local neighborhood / subsystem spread (IDs are build-local) |
| `temporal_context` | Scorecard temporal fields | Recent-change context |
| `caveats` | Pack + query errors | Why a section is partial or empty |

Popular sinks (callers > ~30) are counted but not expanded in impact neighbors.

### `paths`

- `query_context_paths` — multi-hop paths between stable symbol IDs (`max_hops` ≤ 6, `max_paths` ≤ 12).
- `query_symbol_risk_community` — batch risk + community rows (cap 40 IDs).

### `doc_nav`

Lance-backed section table over the doc artifact: full-text search, or expand children of a `root` section stable id (optional `as_of` commit, `file_glob`).

### `overlay`

`open_worktree_overlay(base_path, delta_dir)` attaches the base DB read-only and builds merge-on-read views over a delta directory so pack/graph-reasoning can reflect dirty worktree state without a full rebuild.

### `mcp`

`AnalystMcpModule` registers and dispatches:

| Tool | Purpose |
|---|---|
| `knowledge_context_pack_2` | Canonical evidence pack (+ graph reasoning) |
| `knowledge_context_pack` | Deprecated alias; routes to v2 behavior |
| `doc_navigate` | Doc section FTS / tree hop |
| `query` | Read-only DuckDB SQL |

Supports current-worktree-only mode or multi-project catalog mode (`with_local_projects` / `with_local_projects_for_analyst_server`).

---

## Public Rust API (selected)

Re-exported from the crate root (`lib.rs` + `api`):

**Types:** `KnowledgeSearchScope`, `KnowledgeQueryIntent`, `KnowledgeQueryOptions`, `KnowledgeCandidate`, `KnowledgeQueryResult`, `SymbolRiskScorecardRow`, `SymbolCommunityContextRow`, `SymbolRiskCommunityResult`, `KnowledgePathOptions`, `KnowledgePathResult`, …

**Functions:**

```rust
// Search
query_context_candidates(db_path, query, scope, options) -> KnowledgeQueryResult
query_graph_candidates(db_path, query, options) -> KnowledgeQueryResult

// Paths & scorecard
query_context_paths(db_path, sources, targets, options) -> KnowledgePathResult
query_symbol_risk_community(db_path, stable_ids) -> SymbolRiskCommunityResult

// MCP / ops
mcp::AnalystMcpModule::new()
mcp::ensure_analyst_db_ready(root) -> PathBuf
mcp::open_worktree_overlay(base, delta) -> duckdb::Connection
mcp::warm_embed_model()
```

Stability of exported types is guarded by `tests/public_api_exports.rs`.

---

## Building / refreshing the analyst DB

The DB is **built outside this crate** as a post-step of graph build:

```bash
# Recommended: graph build refreshes the analyst DB automatically
spur-cli graph build --workspace

# Manual refresh
spur-cli analyst build
```

Opt out: `--no-analyst` or `SPUR_GRAPH_SKIP_ANALYST=1`.

SQL sources and schema documentation: [`crates/spur-context/analyst/README.md`](../spur-context/analyst/README.md).

---

## Environment

| Variable | Default | Effect |
|---|---|---|
| `SPUR_ANALYST_DUCKDB_MEMORY_LIMIT` | `4GB` | Per-connection DuckDB memory cap |
| `SPUR_ANALYST_DUCKDB_THREADS` | `4` | DuckDB threads |
| `SPUR_GRAPH_SKIP_ANALYST` | unset | Skip analyst rebuild during graph build |
| `SPUR_EMBEDDING_MODEL` | `nomic` | Query/index model: `nomic`, `coderank`, or `jina-code` (all FP32/768); rebuild after changing it |

---

## Features

| Feature | Default | Notes |
|---|---|---|
| `embed` | **on** | Enables `fastembed` + HF model download path for query embeddings |

```toml
spur-analyst = { path = "crates/spur-analyst" }
# or without embeddings:
spur-analyst = { path = "crates/spur-analyst", default-features = false }
```

---

## MCP tool sketch

### `knowledge_context_pack_2`

```json
{
  "query": "delegation error handling",
  "intent": "explain",
  "scope": "all",
  "limit": 8,
  "include_tests": false,
  "max_symbol_bodies": 3,
  "graph_reasoning": {
    "paths": true,
    "communities": true,
    "risk": true,
    "max_path_hops": 4,
    "max_paths": 6
  }
}
```

Returns structured evidence (`primary_evidence`, `supporting_docs`, `impact`, `staleness`, `recommended_next_tools`, plus v2 graph sections). `answerable` / `confidence` describe retrieval quality, not behavioral correctness. Follow `graph://symbol/...` selectors into `code_read_symbol` / `code_callers` for exact grounding.

### `query`

```json
{
  "query": "SELECT entity_name, pagerank FROM v_symbol_scorecard ORDER BY pagerank DESC LIMIT 20",
  "allow_stale": false
}
```

Write statements are rejected. Results are capped at **1000 rows** without rewriting the SQL `LIMIT`.

### `doc_navigate`

Discovery only — metadata + ~200-char `lede`, never full section body.

- FTS: `{ "query": "overlay merge-on-read", "k": 20 }`
- Tree hop: `{ "root": "<stable_section_id>", "include_lede": true }`
- Each hit includes `next[]`. **Terminal body read:** `code_read_symbol({ stable_symbol_id })` (section symbols are graph `symbol_kind: "section"`). Use `doc_navigate` with `root` only for outline expansion.

---

## When to use analyst vs exact graph

| Question shape | Prefer |
|---|---|
| “Where does this concept live?” / orientation | `knowledge_context_pack_2` |
| Exact symbol body / callers / callees | `code_*` (exact graph) |
| Ranked hotspots, PageRank, co-change, multi-hop SQL | Analyst SQL / pack risk sections |
| Doc outline or section tree | `doc_navigate` |
| Full doc section body after a hit | `code_read_symbol` on the section `stable_symbol_id` |

**Trust rule:** treat pack hits as candidates; use `staleness` + `response_file_oids_match` / graph hash before high-impact claims; ground with exact graph tools.

---

## Tests

```bash
scripts/spur-cargo test -p spur-analyst
```

Notable suites under `tests/`:

- `pack/` — pack service, MCP characterization, query tool, snapshots of v1/v2 pack shape
- `search/` — hybrid / Lance session behavior
- `embedding/` — embed service + sidecar layout
- `overlay.rs`, `paths.rs`, `public_api_exports.rs`

---

## Dependencies (workspace)

Primary: `duckdb` (bundled), `lancedb` / `lance-index`, `spur-graph`, `spur-mcp`, `serde`/`serde_json`, `tokio`, optional `fastembed`.

---

## Further reading

- Workspace architecture: [`ARCHITECTURE.md`](../../ARCHITECTURE.md) — “Key crate internals → spur-analyst”
- Analyst DB schema & SQL tiers: [`crates/spur-context/analyst/README.md`](../spur-context/analyst/README.md)
- Worktree overlay design: `docs/superpowers/specs/2026-07-04-analyst-duckdb-worktree-overlay-merge-on-read-design.md`
- Knowledge pack plan: `docs/superpowers/plans/2026-06-07-knowledge-context-api.md`
- Hybrid embedding plan: `docs/superpowers/plans/2026-06-06-semantic-search-wave3-hybrid-embedding.md`
