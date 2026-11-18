---
name: app-dev
description: "Use when building, scaffolding, extending, or packaging a Spur notebook app — a .spurapp bundle with a Python MCP server and/or TypeScript frontend cells."
role: both
---
<!-- SPUR-MANAGED v=1 skill=app-dev sha256=0000000000000000000000000000000000000000000000000000000000000000 -->

# App Dev — Spur Notebook App Development

You are building a Spur notebook app. A Spur app is a `.spurapp` bundle that contains a
Jupyter notebook, an optional Python MCP server, optional TypeScript frontend cells,
widget assets, and a `spur-app.json` manifest. The host opens the bundle in *app mode*,
where the output pane renders the notebook's HTML outputs with full trust and the Python
server is started as an MCP plugin.

This skill is a lean orchestrator. Each step names a reference under `references/` to load
**only when you reach that step** — don't preload them all.

<HARD-GATE>
Tool-name verification. Every tool listed here MUST either exist in the live tool surface
(verified at task write time against `src/mcp/tools/`) or be explicitly marked PLANNED.

| Tool name                   | Status   | Path |
|-----------------------------|----------|------|
| `notebook_export_spur_app`  | EXISTS   | `src/mcp/tools/export_spur_app.rs` |
| `notebook_import_spur_app`  | EXISTS   | `src/mcp/tools/import_spur_app.rs` |
| `notebook_app_doctor`       | EXISTS   | `src/mcp/tools/notebook_app_doctor.rs` |
| `notebook_app_briefing`     | EXISTS   | `src/mcp/tools/notebook_app_briefing.rs` |
| `notebook_app_init`         | EXISTS   | `src/mcp/tools/notebook_app_init.rs` |
| `notebook_app_pack`         | EXISTS   | `src/mcp/tools/notebook_app_pack.rs` |
| `spur app init`             | PLANNED  | (CLI task U5 — not yet shipped) |
| `spur app pack`             | PLANNED  | (CLI task U5 — not yet shipped) |
| `notebook_insert_cell`      | EXISTS   | notebook MCP |
| `notebook_write_cell`       | EXISTS   | notebook MCP |
| `notebook_read_cell`        | EXISTS   | notebook MCP |
| `notebook_run_cell`         | EXISTS   | notebook MCP |
| `notebook_save`             | EXISTS   | notebook MCP |

Do NOT call planned tools. Use the "today's real paths" column from the front-door table
in §5.
</HARD-GATE>

---

## The loop

### 0. Classify the archetype — load `references/app-archetypes.md`

Before anything else, pick **one** primary archetype and state it: `headless`,
`vanilla-widget`, `react-widget`, or `interactive-decoupled`. It decides the scaffold
template, the typical capabilities, and what validation matters. Capabilities
(`ports`, `canvas_capture`, `artifacts_dir`, …) are a separate, composable axis — never
archetypes.

### 1. Declare capabilities — load `references/capability-recipes.md`

The manifest declares what the app needs; the host provisions it. Declare only what you
use; unknown capability keys are rejected (`deny_unknown_fields`). Never read an env var
the manifest did not declare.

### 2. Scaffold the app

**Today's real path:** `notebook_app_init` (exists) scaffolds a doctor-green app:

```json
{ "app_root": "/abs/path/to/my-app", "name": "my-app", "template": "minimal" }
```

Templates: `minimal` (Python server + Deno frontend cell + skill + pytest),
`frontend-only` (no server), `headless`, `react-widget`, and
`interactive-decoupled`. `react-widget` starts with a TSX/Vite anywidget project
under `frontend/`; `interactive-decoupled` adds Python/Rust/Go port cells,
`backend/` stubs, and a frontend-emitted `query` source port. Only `app_root` is
required. It refuses to overwrite.

Scaffold implementation lives in `src/spur_app/scaffold.rs` (`scaffold_app`,
`template_text_files`, and `notebook_for_template`). When the standard app/project
structure changes, update that scaffold so new users and agents start from a ready
layout instead of reconstructing the structure by hand.

### 3. Write server tools and frontend cells — load `references/tool-design.md`

One job per tool; descriptions start with "Use this when …"; partition returns
(model-visible vs port-delivered). If the archetype is `interactive-decoupled` (or the
app has long-lived/reactive state), also load `references/port-state-patterns.md`. If the
frontend uses Deno with anywidget/React/Perspective and a polyglot backend, also load
`references/anywidget-deno-polyglot.md` for the TSX source layout, build boundary, and
thin Deno loader-cell pattern.

### 4. Validate — load `references/validation-ladder.md`

Run the ladder lowest-cost first (static → typecheck → `notebook_app_doctor` → host
loop). **Doctor must be green before packing**, and you must state which level you reached
and what you did not run.

### 5. Pack and publish

**Today's real paths** — use the gated pack path for normal app packaging:

| Step | Today (agents) | Planned CLI |
|------|----------------|-------------|
| Pack to `.spurapp` | `notebook_app_pack { notebook_path, output_path }` | `spur app pack` |
| Install / test pack | `notebook_import_spur_app { path }` | same tool |
| Scaffold | `notebook_app_init { app_root }` (see §2) | `spur app init` |

```json
{
  "notebook_path": "/abs/path/to/my-app/app.ipynb",
  "output_path": "/abs/path/to/my-app.spurapp"
}
```

`notebook_export_spur_app` remains available as the lower-level canonical exporter; do
not use it for normal app packing when the doctor gate should apply.

Optional: `name`, `widget_assets`, `include_port_snapshots`. The packer discovers
dependency lock files from the notebook's parent directory (no `dependency_roots`
parameter). **Never simulate the packer yourself** — the fixture-lockstep invariant
guarantees only the canonical Rust packer (`spur_app::archive`) produces a valid archive.

---

## Output contract

When you build or extend an app, present your work in this order so the user can correct
course early:

1. the archetype you chose and why
2. the capability/manifest plan
3. scaffold result
4. tools + cells written
5. **validation level reached and what you did not run**
6. pack result

---

## References

Skill references (load on demand, per step):

- `references/app-archetypes.md` — classify before scaffolding (Step 0)
- `references/capability-recipes.md` — the four composable capabilities (Step 1)
- `references/tool-design.md` — server tools + frontend cells (Step 3)
- `references/port-state-patterns.md` — reactive/decoupled apps (Step 3, when stateful)
- `references/anywidget-deno-polyglot.md` — TSX source layout, Deno anywidget/React/Perspective frontends, and polyglot DAG backends (Step 3)
- `references/validation-ladder.md` — graded validation + the doctor gate (Step 4)

SDK docs:

- `sdk/docs/manifest.md` — `spur-app.json` field reference and capabilities
- `sdk/docs/python-sdk.md` — full Python SDK API
- `sdk/docs/typescript-sdk.md` — full TypeScript SDK API
- `sdk/docs/port-store.md` — port-store wire contract and fixtures
- `sdk/docs/dev-loop.md` — init/dev/doctor/pack/publish real paths vs planned
- `sdk/docs/versioning.md` — schema versioning, sdk_min, contract_version
- `sdk/schema/spur-app.schema.json` — JSON Schema for `spur-app.json`
- `app_gallery/spur-growth/` — reference react-widget gallery app
- `app_gallery/code-graph-workbench/` — server + ports gallery app
