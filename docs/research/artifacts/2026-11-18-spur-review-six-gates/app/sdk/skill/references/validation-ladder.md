# Validation Ladder — what "done" means

Load this at **Step 4**. The goal is not "files were created." It is "the app satisfies a
graded, honestly-reported contract." Run the lowest-cost checks first, climb as far as
feasible, and **always state which level you reached and what you did not run.**

## Minimum repo contract

Before climbing the ladder, the repo should satisfy the parts of this contract that apply
to its archetype (see `app-archetypes.md`):

- **Shape** matches the chosen archetype; a reader can see where the server and frontend
  live.
- **Manifest** parses; `schema: "spur.app/v1"`; `entry_notebook` exists; declared
  capabilities are known and grantable.
- **Tools** each map to one intent, with clear descriptions and accurate behavior cues
  (see `tool-design.md`).
- **Frontend** (if any) renders from its last expression; capability overlays it uses are
  declared.
- **Skill** at `manifest.skill` exists and every HARD-GATE tool name is live.

## The ladder

### Level 0 — static contract review (free)

- chosen archetype is stated and the repo shape matches it
- `headless` ⇒ no frontend cells; `react-widget` ⇒ anywidget present; `interactive-decoupled`
  ⇒ declared `ports.read` correspond to real DAG source cells
- manifest shape and capability keys are valid (`deny_unknown_fields`)

### Level 1 — syntax / typecheck (cheap)

- Python server: import/syntax-check; unit-test tools with `FakePortStore`
  (see `tool-design.md`)
- frontend cells: TypeScript typecheck

### Level 2 — `notebook_app_doctor` (the gate)

```json
{ "path": "/abs/path/to/my-app" }
```

Returns `{ "ok": bool, "level_reached": number | null, "findings": [{ check, level, ladder_level, message, location? }] }`,
where `level ∈ {"pass","warn","fail"}` is severity, `ladder_level` is the validation
ladder level for that finding, and `ok` is true only when no finding is `fail`.
`level_reached` is `null` when Level 0 fails, `0` when static checks pass but a
doctor-level check fails, and `2` when doctor is green.
Doctor checks: manifest parses; capabilities known and grantable; declared
`ports.read` exist as DAG sources; plugin spawns and `tools/list` succeeds; skill
file exists and every HARD-GATE tool name is in the live surface; port store
reachable; deprecated `runtime.features` warns.

### Level 3 — host loop (when feasible)

- open the app in app mode; confirm capability grants
- the frontend renders; `callTool` round-trips
- for `interactive-decoupled`: a port update re-runs the render without remounting; the
  capture loop produces `duration_sec`
- exercise retries to confirm idempotent tools stay safe

## The gate and the reporting rule

- **Doctor must be green before packing.** Any `fail`-level finding blocks the pack. This
  is the mechanism that stops skills drifting to phantom tools.
- **Report what you ran and what you didn't.** End every validation with an explicit
  line, e.g. *"Reached Level 2 (doctor green); did not run Level 3 host loop."* "Repo
  shape looks right," "syntax is valid," "server starts," and "host integration was
  exercised" are different claims — never let one stand in for another.
