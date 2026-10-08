# Repository Guidelines

## Layout
Rust workspace under `crates/` + `xtask/`. Scope changes to one crate unless a cross-crate refactor is required.

| Area | Crates |
|------|--------|
| Core | `spur-cli`, `spur-core`, `spur-acp`, `spur-tui`, `spur-mcp` |
| Data | `spur-context`, `spur-cost`, `spur-graph`, `spur-analyst` |
| Infra | `spur-worktree`, `spur-pm`, `spur-blob-store`, `spur-telemetry` |
| Frontends | `spur-interactive`, `spur-bot` |
| Other | `spur-license`, `spur-license-admin`, `spur-test-madsim` |

Notebook UI lives in `getspur/spur-notebook` (external). Specs/plans: `docs/superpowers/{specs,plans}/`. Integration tests: `crates/spur-{acp,core,tui,cli}/tests`.

## Skills (foundation + catalog)

Default projection is **foundation only** (`skills.projection_mode = "catalog_only"`).

**Always on disk:** `skills-catalog`, `spur-way`, `code-explore`, `solve`, `brain-delegation`, `brain-review-gate`, `plan-task-discipline`, `worker-signals`, `beads-lifecycle`, `spur-analyst`.

**Everything else** — discover via catalog MCP, do not filesystem-walk skill dirs:

1. **`skill_navigate`** — FTS (`query`) or tree hop (`root`); metadata + short ledes only
2. **`skill_read`** — full `SKILL.md` / approved resource; use exact `skill_id` from a hit
3. **`skill_search`** — optional name/description cards; prefer navigate

Do not treat ledes as full instructions or request task-specific materialization. Catalog down → continue with foundation only. Rollback: `projection_mode = "all_active"`. Details: `crates/spur-cli/assets/skills/skills-catalog/SKILL.md`.

## Code retrieval

1. **`knowledge_context_pack_2`** (alias: `knowledge_context_pack`) — first call for orientation / “where is X”
2. **`code_*`** — exact symbols: search, resolve, read, callers/callees, subgraph (radius ≤ 2)
3. **`spur-analyst`** — SQL, aggregation, churn, reachability, graph algorithms
4. **Native tools** (`rg`, read file, …) — only when graph tools lack the shape, are stale, or you need raw working-tree bytes

## Build & test

**Always** `scripts/spur-cargo`; never bare `cargo` (sandbox `target/` EPERM / disk).

| Rule | Detail |
|------|--------|
| Remote default | `build` / `check` / `test` / `doc` / `clean` / `run` → AWS VM |
| Force local | `SPUR_REMOTE=0` (interactive TUI, live ports) |
| Force remote | `SPUR_REMOTE=1` (sandbox `clippy`, etc.) |
| Failures | Remote red = real failure; do not re-run locally to “make it pass” |
| `fmt` | Always local |

```bash
scripts/spur-cargo test -p spur-tui
scripts/spur-cargo clippy --workspace -- -D warnings   # SPUR_REMOTE=1 in sandbox
scripts/spur-cargo run -p spur-cli -- --help           # SPUR_REMOTE=0 if interactive
spur explore sync|list|add|remove|status
```

Notebook frontend: `SPUR_NOTEBOOK_REPO=/path/to/spur-notebook scripts/spur-pnpm …`. Release/cross builds (`e2e`, `coverage`, `zigbuild`, `xwin`, `xtask dist`): see `docs/` / Claude.md extras when needed.

## Style, test, commit

- Rust 2021 + `cargo fmt`; `snake_case` / `CamelCase` / `SCREAMING_SNAKE_CASE`; small crate-local diffs
- Bug fixes: failing `test(...)` commit, then `fix(...)`. ACP envelope changes: round-trip tests like `crates/spur-acp/tests/executor_events_roundtrip.rs`
- Commits: `<type>(<scope>): <sub-id> <short imperative>` (e.g. `fix(spur-tui): S1.c cap per-frame event drain at 8`). Types: `feat|fix|test|docs|refactor|chore`. Commit finished work with intent-focused messages
- Spec → plan → implement for non-trivial work. Re-check older plans vs current code; respect broadcast sizing, TUI drain caps, ACP sequencing, notification grace

## Collaboration signals

Beads are source of truth (`spur-way`: INTENT → ACTION → RECORD). Emit/consume `[[spur-signal v1]]` / `[[spur-audit v1]]` and labels via foundation skills (`worker-signals`, `brain-review-gate`, `beads-lifecycle`, `plan-task-discipline`) — not by inventing formats here. Label charset: `[A-Za-z0-9_:-]+`. Authoritative lists: skill bodies + `crates/spur-mcp/src/plan/labels.rs`.
