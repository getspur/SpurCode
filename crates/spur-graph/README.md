# spur-graph

Multi-language **code-graph substrate** for SPUR: tree-sitter extraction, content-addressed stable IDs, incremental rebuild, Parquet artifacts, and the exact `code_*` MCP query surface.

`spur-graph` is the **hot / exact tier** of worktree code intelligence. It turns a git worktree into a typed fact graph (`GraphIndexArtifact`) that answers “where is X”, “what calls X”, “what changed”, and “is this answer fresh”.

It does **not** do ranked SQL analytics or evidence packs — that is `spur-analyst` over DuckDB. Agents and brains consume this crate’s facts through MCP (`code_*` tools); they do not normally load Parquet directly.

## Videos

### Four-beat explainer
[spur-graph-explainer.webm](https://github.com/user-attachments/assets/9c9f0999-bf13-4716-be1c-9d889db62ced)

<p align="center">
  <sub>
    <a href="./assets/spur-graph-explainer.webm">Watch the 48-second WebM explainer</a>
    — files become normalized facts, stable identities, incremental rebuilds, and freshness-aware <code>code_*</code> answers.
  </sub>
</p>

### Graph-first code exploration

[spur-graph-explore-code.webm](https://github.com/user-attachments/assets/95ed7a64-d8ce-444f-8cf5-b9db6890fae9)


<p align="center">
  <sub>
    <a href="./assets/spur-graph-explore-code.webm">Watch the 33-second WebM demo</a>
    — a Grok brain uses graph-backed code exploration to resolve current source, follow consumers, and assemble a concrete impact model.
  </sub>
</p>

```
Worktree + git (gix)
        │
        ▼
  discovery → tree-sitter extract → stable IDs → GraphIndexArtifact
        │                                      │
        │                                      ├─ Parquet + pointer  ─► spur-analyst (warm)
        │                                      └─ GraphQueryClient
        │                                              │
        └─ OverlayClient (dirty delta) ◄───────────────┤
                                                       ▼
                                              GraphMcpModule (code_*)
                                                       │
                                                       ▼
                                              Brain / workers
```

> **Deep dive:** [ARCHITECTURE.md](./ARCHITECTURE.md) is the C4 map (system → containers → components → key signatures). This README is the orientation layer.

---

## Responsibilities

| Owns | Does not own |
|---|---|
| Multi-language extraction into a uniform node/edge/span model | DuckDB analytics / PageRank / pack assembly (`spur-analyst`) |
| Content-addressed stable symbol/file IDs | Beads / PM state (`spur-pm`) |
| Incremental rebuild + dirty-worktree overlay | TUI / orchestration (`spur-tui`, `spur-core`) |
| Staleness contract on every query response | Session cost analytics (`spur-context`) |
| Exact call/reference graph for `code_*` tools | Free-form natural-language answers |

---

## Quick start

### Build the graph for a worktree

```bash
# From a SPUR checkout (or any repo you want indexed)
scripts/spur-cargo run -p spur-cli -- graph build

# Artifact default: <worktree>/.spur/graph/
# Override: SPUR_CODE_GRAPH_INDEX=/path/to/dir  or  --output
```

### Standalone code-graph MCP server

```bash
scripts/spur-cargo run -p spur-cli -- graph mcp
# Registers GraphMcpModule: code_resolve, code_symbol_search, code_read_symbol,
# code_callers, code_callees, code_subgraph, code_file_symbols, code_symbol_info,
# code_symbol_history
```

### Programmatic pipeline (library)

```rust
use spur_graph::{
    artifact_from_facts, artifact_from_facts_incremental, build_facts,
    load_artifact, resolve_worktree_root_from, write_artifact_parquet,
};

let root = resolve_worktree_root_from(std::env::current_dir()?);
let (facts, _counts) = build_facts(&root, None)?;
let artifact = artifact_from_facts(&facts, &root)?;
// write_artifact_parquet(...); load_artifact(...); artifact_from_facts_incremental(...)
```

---

## Supported languages

`Language` in `src/extract/languages.rs` (18 variants):

| Language | Grammar | Notes |
|---|---|---|
| Rust | `tree-sitter-rust` | Full tags + edge queries |
| Python | `tree-sitter-python` | + notebook fact queries |
| TypeScript / TSX / JavaScript | `tree-sitter-typescript` | JS/TSX share TSX grammar |
| C / C++ | `tree-sitter-c` / `cpp` | Inheritance + relation quality edges |
| Go | `tree-sitter-go` | |
| HCL / Terraform | `tree-sitter-hcl` | One grammar for `.hcl` and `.tf` |
| JSON | `tree-sitter-json` | Named keys only; lockfiles skipped |
| TOML | `tree-sitter-toml-ng` | Named tables/keys; lockfiles skipped |
| YAML | `tree-sitter-yaml` | `.yaml` and `.yml`; lockfiles skipped |
| Lua | `tree-sitter-lua` | |
| Shell | `tree-sitter-bash` | |
| SQL | `tree-sitter-sequel` | |
| Markdown | `tree-sitter-md` | Sections + inline fenced code |
| Jupyter notebook | specialized | Cells / ports via `extract/notebook.rs` |

Query sources live under [`queries/`](./queries/) — see [`queries/README.md`](./queries/README.md) for the `@definition.*` capture contract and the coverage matrix.

Specialized extractors also surface **MCP tool definitions** as `NodeKind::McpTool` nodes (`extract/mcp_tools.rs`).

---

## Typed fact model

Canonical in-memory and on-disk shape: **`GraphIndexArtifact`** (`src/schema.rs`).

```text
GraphIndexArtifact
├── header / manifest_version / graph_content_hash
├── file_manifests[]     path → content_oid + node ids
├── files[] / symbols[]  stable IDs + spans
├── edges[]              calls / refs / imports / …
├── tombstones[]         deleted paths
├── diagnostics[]
└── commits[] / symbol_snapshots[] / temporal_edges[]
```

**Node kinds** (selection): Module, Function, Class, Interface, Struct, Impl, Trait, Enum, Method, Field, Constant, TypeAlias, Macro, Section, McpTool, Cell, Port, Resource, External, …

**Relation kinds:** Imports, Calls, Constructs, Contains, Implements, Defines, References, Extends, Links, Touches, Produces, Consumes, Binds, Emits.

**Edge refinement** (`GraphEdgeKind`): `Calls`, `CallsDyn`, `ReferencesHof`, `ReferencesOther`, `ReferencesAddress`.

Schema / extractor / resolver versions gate incremental validity (`SCHEMA_VERSION` and friends in `store/build.rs`). A `manifest_version` mismatch forces a full rebuild.

---

## Stable identity & content hash

These two hashes are the load-bearing walls of the crate.

### Stable symbol ID

```rust
// src/identity.rs
pub fn stable_symbol_id_for(
    relative_path: &str,
    fqn: &str,
    kind: NodeKind,
    byte_range_start: u64,
) -> String  // 16-hex u64 prefix of SHA256(path ║ fqn ║ kind_discriminator ║ le_start)
```

- Same inputs → same ID across builds and consumers (`spur-analyst` re-derives the same scheme).
- External / dependency nodes: `stable_symbol_id_for_external_path` under synthetic path `external://`.
- Newtypes: `NodeId`, `EdgeId`, `FileId`, `SpanId`, …

### Graph content hash (staleness anchor)

```rust
// src/content_hash.rs
pub fn compute_graph_content_hash(entries: /* sorted (path, oid) */) -> String
// blake3 over every indexed file’s (path, content_oid)
```

Also: `git_blob_oid` — git-compatible SHA1 blob OID (`blob <len>\0…`), tested against `git hash-object`.

---

## Module layout

The crate root (`src/lib.rs`) is a **flat re-export**: every module is `pub` and glob-re-exported. The public API is the union of module roots.

```
src/
├── lib.rs              # re-exports
├── schema.rs           # GraphIndexArtifact, NodeKind, RelationKind, load_artifact
├── identity.rs         # stable_symbol_id_for*
├── content_hash.rs     # graph_content_hash, git_blob_oid
├── discovery.rs        # .gitignore-aware file walk
├── worktree.rs         # resolve_worktree_root_from
├── git.rs / git_walk.rs
├── extract/            # tree-sitter pipeline + languages + markdown/notebook/mcp_tools
├── store/              # build, parquet, pointer, cache, commit index, lance embeddings
├── graph/              # petgraph builder + bounded subgraph algorithms
├── query_client.rs     # GraphQueryClient + Parquet / InMemory / Overlay backends
├── selector.rs         # graph://symbol/<id>, file-qualified, bare-name resolution
├── search.rs           # SearchOptions / modes / filters
├── traversal.rs        # subgraph budgets
├── temporal.rs         # TemporalIndex, symbol history, rename chains
├── validation.rs       # span anchors, code-mention validation
├── locking.rs          # flock discipline
└── mcp/                # GraphMcpModule, RebuildCoordinator, response metadata
```

### Pipeline (containers)

1. **Worktree + git** — root resolution, dirty state, history walk.
2. **Discovery** — language-grouped files.
3. **Extraction** — `build_facts` / `build_facts_for_paths` → `GraphFacts { nodes, edges, spans }`.
4. **Identity** — stamp stable IDs.
5. **Store** — `artifact_from_facts` (full) or `artifact_from_facts_incremental` → Parquet + pointer.
6. **Query** — `GraphQueryClient` trait over persisted / in-memory / overlay backends.
7. **MCP** — tool dispatch + singleflight rebuild + staleness metadata on every response.

---

## Incremental rebuild

`artifact_from_facts_incremental(prev, root)` (`store/build.rs`):

1. **Manifest gate** — version mismatch → full rebuild (`BuildMode::Full`).
2. **Discover** current tree entries.
3. **Diff OIDs** against `prev.file_manifests[*].content_oid`.
4. **Re-extract only changed paths** via `build_facts_for_paths`.
5. **Compose** buckets + tombstones → new artifact with a fresh `graph_content_hash`.

`BuildStats` reports reused vs changed vs removed paths.

### Dirty overlay (no full rebuild)

`OverlayClient<B>` composes a base backend with an in-memory delta over edited files: shadows base symbols on changed paths, remaps stable IDs that shifted on edit, and merges caller/callee edges across the base/delta boundary. Served under `RebuildCoordinator` singleflight (`mcp/rebuild_singleflight.rs`), keyed by `{ head_oid, dirty_oid_set_hash }`.

---

## Query surface

### `GraphQueryClient` trait

```rust
// src/query_client.rs (abbreviated)
pub trait GraphQueryClient {
    fn search_symbols(&self, opts: &SearchOptions) -> anyhow::Result<SearchResult>;
    fn find_caller_edges(&self, sid: &str) -> Vec<OwnedCallerRecord>;
    fn find_callee_edges(&self, sid: &str) -> Vec<OwnedCalleeRecord>;
    fn resolve_selector(&self, selector: &str) -> anyhow::Result<CodeSelectorResolution>;
    fn symbol_by_id(&self, sid: &str) -> anyhow::Result<Option<GraphSymbolArtifact>>;
    fn symbols_by_file(&self, path: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>>;
    fn symbols_by_files(&self, paths: &[String]) -> anyhow::Result<Vec<GraphSymbolArtifact>>;
    fn temporal_index(&self) -> Arc<TemporalIndex>;
    fn symbol_history(...) -> ...;
    // file_manifest_by_path, file_exists, …
}
```

**Backends:** `ParquetClient` · `InMemoryClient` · `OverlayClient<B>`.

### Selectors

Prefer carrying `graph://symbol/<hex-id>` once resolved. Also accepted: bare hex id, `path::name`, qualified name, bare name (may be ambiguous → candidates).

### MCP tools (`GraphMcpModule`)

| Tool | Role |
|---|---|
| `code_symbol_search` / `code_search` | Name lookup (exact/prefix/substring + filters) |
| `code_resolve` | Exact canonical name → symbol |
| `code_read_symbol` | Source body by id or path+name |
| `code_symbol_info` | Metadata only |
| `code_file_symbols` | Symbols in one file (prefer filtered search on huge files) |
| `code_callers` / `code_callees` | Impact / behavior (read `counts_by_kind` first) |
| `code_subgraph` | Bounded neighborhood map |
| `code_symbol_history` | Temporal change chain |

Registered into `spur-mcp` via `ToolModule`; also available as `spur graph mcp` and the bundled `spur mcp` server.

---

## Staleness contract

Every `code_*` response carries **`GraphResponseMetadata`**:

| Field | Meaning |
|---|---|
| `graph_content_hash` | blake3 over indexed `(path, oid)` set |
| `indexed_head_oid` | git HEAD when the artifact was built |
| `worktree_head_oid` | git HEAD at query time |
| `worktree_dirty` | tracked edits **or** HEAD ahead of indexed (untracked files do not flip this) |
| `response_file_oids_match` | tightest per-answer signal: files in *this* response byte-identical to the index |
| `rebuild_status` | `NotNeeded` \| `Fresh` \| `StaleBudgetExceeded` \| `StaleRebuildFailed` |

Prefer `response_file_oids_match` when judging trust in a single answer; use `worktree_dirty` for “should I rebuild?” decisions.

---

## On-disk layout

Resolved by `resolve_artifact_location` (`store/pointer.rs`), priority: explicit override → `CURRENT` → pointer JSON.

| Path | Role |
|---|---|
| `.spur/graph/` | Default Parquet artifact directory |
| `.spur/graph/CURRENT` | Active artifact cursor |
| `.spur/graph-index.pointer.json` | `GraphIndexPointer` (hash, manifest, indexed commit, canonical path) |
| Lance tables (feature `embed`) | Sections / code-symbol embedding sidecars |

Env override: `SPUR_CODE_GRAPH_INDEX`.

---

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `embed` | **on** | Selectable FP32/768 embedding models + LanceDB section/symbol sidecars |
| `perf-gates` | off | Compile-time performance budgets / assertions |
| `test-support` | off | Rebuild counters and test-only hooks |

`unsafe_code = "deny"`; clippy profile keeps async-lock-holding and similar safety lints active.

---

## Dependencies (high level)

| Dep | Role |
|---|---|
| `tree-sitter` + language crates | Parse + query |
| `gix` | History, refs, blobs, dirty detection |
| `blake3` / `sha2` / `sha1` | Content hash + stable IDs + git blob OIDs |
| `parquet` + Arrow | Columnar artifact I/O |
| `petgraph` | In-memory subgraph BFS |
| `fastembed` + Lance (optional) | Embedding sidecars |
| `spur-mcp` | `ToolModule` for the MCP surface |
| `ignore` | `.gitignore`-aware discovery |

### Embedding model selection

Set the model in `.spur/config.toml` (repository) or `~/.spur/config.toml` (user). Changing the model
changes the stored model lineage and automatically invalidates incompatible vectors.

```toml
[graph]
embedding_model = "nomic"
```

| Value | Model | Input contract |
|---|---|---|
| `nomic` (default), `nomic-ai/nomic-embed-text-v1.5` | Nomic Embed Text v1.5, FP32/768 | `search_query:` / `search_document:` prefixes |
| `coderank`, `nomic-coderank`, `nomic-ai/CodeRankEmbed` | CodeRankEmbed, FP32/768 | Required code-search instruction on queries; raw documents |
| `jina-code`, `jina-embeddings-v2-base-code`, `jinaai/jina-embeddings-v2-base-code` | Jina Embeddings v2 Base Code, FP32/768 | Symmetric raw query/document text |

`SPUR_EMBEDDING_MODEL` still overrides the config file when set:

```bash
SPUR_EMBEDDING_MODEL=coderank spur explore sync
```

---

## Tests & benches

- **Integration tests:** `tests/` — per-language definition/edge corpora, incremental ingest, parquet round-trip, overlay client, temporal resolution, rename corpora, MCP module, perf gates.
- **Fixtures:** `tests/fixtures/` (sample, python, typescript, cpp, markdown, rename, semantic_benchmark, …).
- **Benches:** `benches/incremental`, `benches/parquet` (criterion); `capture-baselines` binary for baseline capture.

```bash
scripts/spur-cargo test -p spur-graph
scripts/spur-cargo bench -p spur-graph --bench incremental
```

---

## How to explore this crate

The crate is self-hosting — use the graph tools it powers (see `spurpower-code-explore`):

1. **Orient:** `knowledge_context_pack_2({ query: "stable symbol id", scope: "code" })`
2. **Read:** carry `graph://symbol/<id>` into `code_read_symbol` (modules here are often 1k–7k lines).
3. **Build path:** `code_callees` on `artifact_from_facts_incremental`.
4. **MCP router:** `code_subgraph radius=1` seeded at `GraphMcpModule::dispatch`.
5. **Ranked / path / SQL questions:** `spur-analyst` over `.spur/analyst.duckdb`, not hand-chained `code_*`.

**First files to open:** `src/lib.rs` → `schema.rs` → `identity.rs` + `content_hash.rs` → `store/build.rs` → `query_client.rs` → `mcp/mod.rs`.

---

## Related crates & docs

| Resource | Why |
|---|---|
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Full C4 architecture |
| [queries/README.md](./queries/README.md) | Tree-sitter query contract + coverage matrix |
| `crates/spur-analyst` | Warm DuckDB index + evidence packs |
| `crates/spur-mcp` | MCP tool registry / transports |
| `crates/spur-cli` `graph` command | Build, vector backfill, standalone MCP |
| `docs/architecture/spur-graph-git-invalidation.md` | Content-hash invalidation design |
| `docs/superpowers/specs/*spur-graph*` | Feature design specs (notebook, HCL, parquet, …) |

## Direct Rust macro evidence

Direct macro syntax is stored separately from runtime `Calls` edges. For example,
`unreachable!` in `parse_raw_diff` and two repeated `debug_assert!` sites in that
caller produce three distinct evidence rows. Existing call-shaped tokens inside
macro arguments (such as `input.len()`) retain their previous graph behavior.

`GraphIndexArtifact.macro_invocations` is optional: `None` means unavailable
(legacy or uncaptured); `Some` with zero rows means captured with zero sites.
The current domain is **`source_scope = physical_rust_files`**. Notebook Rust
function definitions and calls remain supported, but direct macro evidence for
embedded cells awaits a separate cell/source-location contract. Query a notebook
file or its function caller to obtain `UnsupportedSource`; zero macro rows alone
never establish coverage or safety. A real physical Rust filename containing
`#cell` remains supported.

Both `macro_invocations.parquet` and `macro_scopes.parquet` are required core
tables for captured evidence, even when empty. `manifest.json` records optional
row counts, completeness, schema/extractor versions, source scope and a shared
BLAKE3 checksum binding both tables. Footers bind those metadata to the graph
snapshot. Global site identity is `(graph_content_hash, site_id)`; the full site
digest binds version, physical path, exact source blob OID and coordinates.

| Table | Main columns |
| --- | --- |
| `macro_invocations.parquet` | `site_id`, `file_path`, `source_oid`, nullable `caller_stable_symbol_id`, `enclosing_scope_id`, `enclosing_scope_kind`, `raw_macro_path`, `normalized_macro_path`, `byte_range_start`, `byte_range_end`, `line_start`, `line_end`, `test_context`, `execution_phase`, `debug_assertions_candidate`, nullable `sink_family_candidate`, `attributes`, `resolution` |
| `macro_scopes.parquet` | `scope_id`, `kind`, `file_path`, `source_oid`, `byte_range_start`, `byte_range_end`, `line_start`, `line_end` |

Byte ranges are half-open UTF-8 offsets; lines are one-based and inclusive.
`attributes` and tagged `resolution` are JSON strings retaining raw text, owner
references and local-definition locations. Scopes include non-function const/type
sites and attribute owners. IDs beginning `macro-scope-v1:` identify evidence-only
scopes and do not become graph functions. Only an actual enclosing function has
`caller_stable_symbol_id`; outside-function sites keep it null.

Use `GraphQueryClient::macro_invocations(&MacroInvocationFilter)` for validated
queries. Filters `file_path`, `caller_stable_symbol_id` and `source_oid` are exact
and conjunctive; an expected `source_oid` requires an explicit `file_path`.
Results carry rows, exactly required scopes, snapshot bindings and availability:
`Complete`, `Partial`, `Unavailable` or `UnsupportedSource`. Complete means
captured for the supported source domain, with no result cap. Overlay results
retain separate base/delta snapshots, replace both rows and scopes for changed
paths, remove deleted paths, and preserve untouched scopes. An unknown base stays
partial globally and unavailable for untouched paths; supported recaptured paths
can be complete.

```rust,ignore
use spur_graph::{MacroInvocationFilter, query_client::{GraphQueryClient, ParquetClient}};
let client = ParquetClient::open(artifact_dir)?;
let result = client.macro_invocations(&MacroInvocationFilter {
    file_path: Some("src/diff.rs".into()),
    caller_stable_symbol_id: Some(caller_id), // an actual graph function stable ID
    source_oid: None, // or the expected exact source blob OID
})?;
// Inspect result.availability, result.source_scope and result.snapshots as well as rows.
```

`InMemoryClient` needs the full artifact's source manifests for nonempty evidence.
Slim loads validate then clear those manifests; use `ParquetClient` for persisted
directories. Its first macro query freezes both complete compressed evidence
tables and validates their shared checksum, stored source/scopes and actual graph
caller projection before filtering. It also verifies decoded supporting source
manifest count, including zero-site files. Full decoded evidence is then dropped;
compressed bytes and source context stay resident. Subsequent queries prune those
immutable in-memory Parquet bytes and perform no evidence-table disk IO. Caller-only
ownership lookup can separately hydrate graph caller data. Reopen the client to
observe later on-disk changes. The legacy full artifact reader still uses the
supporting source-table declared count as capacity; the focused macro-query count
check does not repair that separate full-reader integrity limitation.

### DuckDB queries

Validate the artifact through `ParquetClient` first; raw `read_parquet` SQL does
not enforce the shared checksum or API availability semantics. Run these queries
in one DuckDB session from the artifact directory, or replace the file paths with
absolute paths to a single validated snapshot. Check the declared domain first:

```sql
SELECT macro_evidence.complete, macro_evidence.source_scope,
       macro_evidence.schema_version, macro_evidence.extractor_version,
       graph_content_hash, row_counts.macro_invocations, row_counts.macro_scopes
FROM read_json_auto('manifest.json');

CREATE TEMP VIEW macro_sites AS SELECT * FROM read_parquet('macro_invocations.parquet');
CREATE TEMP VIEW macro_scopes AS SELECT * FROM read_parquet('macro_scopes.parquet');
CREATE TEMP VIEW graph_symbols AS SELECT * FROM read_parquet('nodes.parquet');

-- Every direct source candidate, including nullable non-function callers.
SELECT m.site_id, m.file_path, m.source_oid, m.raw_macro_path,
       m.normalized_macro_path, m.byte_range_start, m.byte_range_end,
       m.line_start, m.line_end, m.caller_stable_symbol_id,
       c.qualified_name AS caller, s.kind AS enclosing_kind,
       s.byte_range_start AS scope_start, s.byte_range_end AS scope_end,
       m.test_context, m.debug_assertions_candidate, m.execution_phase,
       m.sink_family_candidate, m.resolution, m.attributes
FROM macro_sites m
LEFT JOIN graph_symbols c
  ON c.stable_symbol_id = m.caller_stable_symbol_id AND c.file_path = m.file_path
JOIN macro_scopes s
  ON s.scope_id = m.enclosing_scope_id AND s.file_path = m.file_path
 AND s.source_oid = m.source_oid
ORDER BY m.file_path, m.byte_range_start, m.site_id;

-- Scoped caller join: the held-out fixture returns three distinct sites.
SELECT m.site_id, m.normalized_macro_path, m.line_start,
       m.debug_assertions_candidate, c.stable_symbol_id AS caller_id
FROM macro_sites m
JOIN graph_symbols c ON c.stable_symbol_id = m.caller_stable_symbol_id
WHERE c.file_path = 'src/diff.rs' AND c.entity_name = 'parse_raw_diff'
  AND m.file_path = c.file_path
ORDER BY m.byte_range_start, m.site_id;

-- Attribute-owner locations also survive independently of graph function symbols.
SELECT DISTINCT m.site_id, json_extract_string(a.value, '$.raw_text') AS attribute,
       s.scope_id, s.kind, s.file_path, s.source_oid,
       s.byte_range_start, s.byte_range_end, s.line_start, s.line_end
FROM macro_sites m, json_each(m.attributes) a, macro_scopes s
WHERE s.scope_id = json_extract_string(a.value, '$.owner.stable_symbol_id')
  AND s.file_path = m.file_path AND s.source_oid = m.source_oid
ORDER BY m.site_id, s.scope_id, attribute;
```

These are source candidates, not panic executions. Bare and std/core-looking
paths are candidate identities; aliases remain unresolved, and local macro
shadowing is only a lexical definition candidate. Raw `cfg`/`cfg_attr` evidence is
retained without evaluating a build. Absence of a test marker is unspecified,
not proof of production execution. Nested token-tree text and macro-definition
bodies do not become direct invocation rows. No expansion, namespace resolution,
MIR/CGA, cfg evaluation, panic feasibility or panic-freedom claim is provided.

Reproduce the held-out extraction, full/incremental artifact, scope, Parquet,
query, notebook-domain and row-group boundary checks with local builds:

```sh
env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_end_to_end
env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph
SPUR_REMOTE=0 scripts/spur-cargo check -p spur-core -p spur-analyst -p spur-tui -p spur-cli -p spur-mentions --tests
SPUR_REMOTE=0 scripts/spur-cargo check -p spur-graph --all-targets
SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -p spur-analyst -- --check
```

For reproducible SQL and cost inspection, set `M6_EVIDENCE_DIR=/private/tmp/m6-evidence`
on the held-out command and add `-- --nocapture`. It writes observed JSON and a
`parquet/` directory outside the fixture input. Timings separate full extraction,
artifact construction/validation, incremental rebuild, Parquet writing and first
versus subsequent filtered queries. `scope_verification_io_hash_s` separately
measures the existing extra scope/source rereads, hashes and identity/context
assertions (summed over incremental filters); it excludes query time. Other
roundtrip/oracle assertions, SQL and export copying also remain outside extraction
timers. The boundary test rewrites the same evidence into two-row
Parquet groups and asserts actual metadata and cross-group membership; it is a
correctness check with no speed threshold. Exact task receipts and the measured
bounded workload are in the [implementation plan](../../docs/superpowers/plans/2026-10-31-macro-invocation-evidence.md).

The final integration review passed the complete graph suite with default test
parallelism: **1,128 passed, 0 failed, 19 existing ignores**. Affected dependent
test compilation, graph all-target checks, formatting and the exact SQL oracle
also pass. The implementation plan preserves earlier schedule-sensitive failures
and the serial verification run; no timeout or snapshot fallback policy was
changed. Bounded timing observations and their source/IO separation are recorded
in that ledger and the experiment notebook.

## Experimental panic evidence sidecar

`spur_graph::panic_evidence` is opt-in, source/configuration-bound evidence.
It preserves individual direct method invocations and actual compiler instance
names. It does not change v11 facts, ordinary graph builds, Parquet or CURRENT.

Build and invoke the JSON runner locally:

```sh
SPUR_REMOTE=0 scripts/spur-cargo build -p spur-graph --no-default-features --example panic_evidence
SPUR_REMOTE=0 scripts/spur-cargo run -p spur-graph --no-default-features --example panic_evidence < request.json > response.json
```

One UTF-8 JSON request enters stdin and one JSON response leaves stdout;
validation errors use stderr and a nonzero exit. Commands and fields:

| `command` | Request fields | Response |
| --- | --- | --- |
| `snapshot` | `root`, `source_paths` | `graph`, `method_sites` (in-memory fixture snapshot; no graph writes) |
| `collect` | `graph`, `sources` | `method_sites` |
| `identity` | `identity` | `build_id` |
| `import` | `graph`, `sources`, `method_sites`, `inputs`, `identity`, `expected_build_id`, `raw_export` | `compiler_evidence` |
| `validate` | all import fields plus `compiler_evidence` | `valid`, `build_id` |
| `analyze` | `graph`, `sources`, `build` | `status: "collected"`, `facts`; or `status: "unknown"`, `obligation` |
| `solve` | `graph`, `sources`, `build`, `facts`, `domains`, `limits` | `report` (`panic-analysis-v1`) |
| `compiler_report` | all validate fields plus `build`, `config_binding` | `compiler_report` (`panic-compiler-validation-v1`) |
| `replay` | all solve fields plus `report`, `query_index`, `expected_request_identity`, `expected_panic_payload`, `execution_timeout_ms` | `replay` (`panic-fixture-replay-v1`) |

Sources are `{ "file_path": "src/lib.rs", "text": "exact UTF-8 source" }`;
inputs use `path` and `text`. `raw_export` is the complete, unchanged compiler
JSON encoded as a JSON string, including whitespace for checksum verification.
An independently retained expected build ID is required before import. The
snapshot command is for a bounded standalone fixture; callers with an existing
graph pass its artifact directly to `collect`/`import`.

These command names and required JSON fields are the experimental v1 contract.
Unknown fields and malformed requests are errors. The synchronous Rust
`execute(Value)` entrypoint remains available for all synchronous commands;
`execute_async(Value, &SolverService)` also accepts `solve` and `replay`.
The stdin runner uses the async entrypoint and persists requested solver receipts
under its current repository directory. API callers supply that directory through
`SolverService::with_repo_root` and reload receipts with `get_solve_result`.

`analyze` collects canonical facts from exact source bytes. Its required `build`
object has this complete shape (the graph hash and compiler strings are actual
values, and `cfg` is the complete sorted inventory of enabled compiler atoms):

```json
{
  "schema_version": "panic-build-config-v1",
  "graph_content_hash": "<graph.graph_content_hash>",
  "target": "<target triple>",
  "edition": "2021",
  "cfg": [{"key": "panic", "value": "unwind"}],
  "overflow_checks": true,
  "debug_assertions": true,
  "panic_strategy": "unwind",
  "rustc_verbose": "<exact rustc -vV stdout>",
  "codegen_flags": [],
  "features": []
}
```

Editions are `2015`, `2018`, `2021`, or `2024`; panic strategy is `unwind` or
`abort`. Cfg atoms use a null value for bare names. This build object declares
the intended model; collection and solving do not attest compiler execution.
Cfg/parse collection uncertainty returns `unknown` with `{kind, detail}` and
no facts. Opaque constructs that have a known body remain explicit obligations
inside collected facts. In particular, macro expansion (including `assert!`)
is unsupported by the source collector.

Pass the unchanged `facts` returned by `analyze` to `solve`, alongside the same
graph, source bytes, and independently expected build. Before narrowing any
domains, `solve` recollects and compares the entire canonical fact set. Missing
sites, implicit operations, coverage, or source bindings therefore fail closed.
`domains` is an array of the following records, exactly one per modeled parameter
of each explicit entry body; duplicate, missing, widened, or unrelated domains
are rejected:

```json
{
  "body_id": "<facts.bodies entry body_id>",
  "parameter_index": 0,
  "domain": {"kind": "u8", "minimum": 0, "maximum": 255}
}
```

U8 bounds are inclusive and within 0–255. Bool domains use
`{"kind":"bool","values":[false,true]}` (nonempty, unique, sorted).
Parameters absent from the supported translation retain their obligations;
domains cannot invent them. Entry IDs identify exact bodies, including source
identity, rather than resolving a function by its spelling. `limits` has no
implicit defaults:

```json
{
  "entry_body_ids": ["<entry body_id>"],
  "expected_build_hash": "<facts.build_hash>",
  "max_call_depth": 0,
  "max_loop_iterations": 0,
  "max_states": 4096,
  "timeout_ms": 30000,
  "persist": true
}
```

These are caller-chosen exploration budgets, subject to the typed solver's
caps (timeout at most 60,000 ms). A query returns its exact typed `request`,
request digest, source/build/limits-bound `request_identity`, optional solver
response and receipt binding, candidate inputs, obligations, and outcome:

| Query outcome | Meaning |
| --- | --- |
| `candidate_witness` | SAT with a validated model; `runtime_reproduced` remains false |
| `no_counterexample_within_complete_scope` | UNSAT with no open obligations, only for the declared model/domains/bounds |
| `incomplete_semantics` | UNSAT with unsupported semantics or an open exploration frontier |
| `unknown`, `timeout` | Solver did not establish SAT or UNSAT |
| `operational_error`, `invalid_model` | Solver invocation/request failure or rejected model; no safety conclusion |

Validation failures return Rust `Err`, or CLI stderr and a nonzero exit without
a successful JSON response. Compiler/execution timeouts and compilation failures
also return errors. Unknown and timeout never establish safety. The
`compiler_report` command authenticates the legacy import, recollects canonical
source semantics, then runs compiler correlation with
`config_binding: {"capture_build_id":"<expected_build_id>",
"semantic_build_hash":"<build.content_hash()>"}`. This independently retained
binding is a capture-procedure declaration. The report preserves all observations
and unknown obligations; semantic completeness remains `unestablished`.

`replay` is a separate operation for a trusted standalone fixture. It requires
a persisted complete candidate, the original entire `report`, a zero-based
`query_index`, its independently retained `expected_request_identity`, an exact
`expected_panic_payload` string, and `execution_timeout_ms` in 1–60,000. It
revalidates source/build/facts/domains, reloads original solver receipts, and
re-solves to reconstruct the report. A different model is rejected; rerun solve
if the solver selects another candidate. Altered report identities, candidates,
receipt bindings, and solver-produced reproduction claims are rejected before
compilation.

Replay supports one source file with a top-level synchronous function whose
entire signature is modeled primitive `u8`/`bool` inputs. It runs native ordinary
`rustc` with exact recorded compiler version, edition, target, overflow/debug
settings, `opt-level=0`, and `panic=unwind`; the effective compiler cfg must equal
the complete declared inventory. Additional codegen flags and features are
rejected. The fixture source and generated include/catch-unwind harness are
written to a fresh temporary directory. Compiler and execution subprocesses
have explicit per-process deadlines and a recorded allowlisted environment.
This executes trusted local fixture code; it is not a sandbox for hostile Rust.

The replay record binds graph/source identities, build/fact/request digests,
request identity, original receipt, candidate, harness and executable digests,
compiler arguments/environment, actual exit/stdout/stderr and panic payload.
`runtime_reproduced` becomes true only after successful execution catches the
exact expected string payload. Other outcomes are `panic_payload_mismatch`,
`returned`, or `execution_error`, with reproduction false. `replay_digest` hashes
the compact UTF-8 JSON record in emitted field order before adding that field.
These hashes support local evidence
integrity; they do not attest a producer or independently certify a supplied
record. Only this fixture/harness executed. No arbitrary workspace binary replay
or general Rust safety is established.

The standard-library-only capture script materializes the portable corpus from
`tests/fixtures/panic_evidence/corpus.json` in a **new** directory outside the
workspace. Use `--prepare-only --work-dir /private/tmp/panic-pilot-prepared` to
inspect the seven-input/five-configuration contract without a compiler. For
mandatory fresh acceptance, supply already-built pinned binaries and the exact
retained producer patch (omit `--producer-patch` only for a clean producer):

```sh
python3 scripts/experiments/panic-evidence-capture.py \
  --work-dir /private/tmp/panic-pilot-capture \
  --cga-source /private/tmp/spur-cga-pilot-20261031 \
  --cga-driver /private/tmp/spur-cga-pilot-20261031/target/release/cga-driver \
  --cargo-cga /private/tmp/spur-cga-pilot-20261031/target/release/cargo-cga \
  --producer-patch /private/tmp/panic-evidence-p3/producer.patch \
  --expected-driver-sha256 5647b7f2c7f857dda0dd0204e91d870287988181ca5ec4d7792810a561886172 \
  --expected-launcher-sha256 7f84bc715138c09744fd351859ca02b240ea768d9756069229e9250fa9447b7a \
  --example /private/tmp/panic-evidence-p1/target/debug/examples/panic_evidence
```

The producer is CGA `2fc63108f0b674fe7a178191ffa15647a18879f1` on
`nightly-2026-10-18`. The driver runs as `RUSTC` through the absolute repository
`scripts/spur-cargo --workdir` with `SPUR_REMOTE=0`. The launcher digest records
build lineage; the script bypasses its implicit `build-std`/Cargo subprocesses.
No MIR-text parser, installation or toolchain-default change occurs. Cargo home,
targets and output are isolated, ambient compiler settings are cleared, ancestor
Cargo configuration is rejected, and local path dependencies use a frozen lock.
Verbose compiler commands and the entire effective environment are retained.

Each opt0/panic=unwind debug/overflow pair gets an independent build; a fifth
build enables `extra`. All compiler JSON fragments are retained verbatim. The
bin fragment containing `panic_pilot::main` is imported verbatim, with its raw
checksum, exact source inventory, patched-producer lineage and expected build
ID. `summary.json` links identities, unchanged before/after inventories, logs,
25 corruption rejections, the cross-configuration rejection, and 45 runtime
controls against those same compiled executables. Expected panics must exit 101
and report a panic. A failing capture retains `blocked.json` and logs; no
mandatory gate is silently skipped.

The source corruption selects a physical `.rs` input whose identity kind is
`source`, then finds the request input by its path. Only those bytes change;
the expected identity and graph/source evidence stay fixed. The ownership
corruption changes only a site's caller to a different, unique existing
function or method in the same physical file. Capture fails if either witness
cannot be constructed. Each control retains its request and a
`negative-{source,ownership}.witness.json` with the selected path or caller
symbols and exact changed JSON leaf. Both nonzero exits and the intended
binding/ownership validation messages are required. Portable tests exercise
control construction with process I/O replaced; fresh pinned-compiler capture
remains a separate mandatory gate.

Fresh held-out evidence confirms repeated Option/Result calls, distinct Safe
methods, both generic instances, chains, dependency calls, cfg/feature contrast,
bounds checks and overflow/debug contrasts. A multibyte-prefix call verifies
one-based **scalar** columns and an **exclusive** end against actual bytes.
Compiler macro locations remain unmapped; bounds/addition have no method site,
and debug macro locations can be external. Receiver types, assertion reasons
and expansion maps are unavailable. Optimized MIR may inline/elide calls even
at opt0; absent associations do not establish safety or whole-program coverage.
The frozen corpus projection retains unchanged raw nodes/edges and full-export
checksums with truthful provenance; it is not a standalone CGA graph or a
substitute for fresh acceptance. Run the focused suite with:

```sh
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --no-default-features --test panic_method_sites --test panic_compiler_evidence --test panic_evidence_end_to_end
```

The [2026-11-01 notebook pilot evidence](../../docs/research/artifacts/2026-11-01-panic-evidence-pilot/README.md)
retains fresh five-configuration compiler captures, executable controls and
checksummed replay archives. The held-out fixture associates 17/18 declared AST
sites in each base build and 18/18 with its cfg feature enabled; macro token
trees are outside that denominator. On the original workspace frontier, seven
legacy method labels become three exact function-owned sites, two closure sites
and two macro-token cases. Workspace compiler targets remain unknown. The
notebook records these limits and the measured snapshot-construction cost.
