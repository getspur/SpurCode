# App Dev — Agent Briefing

## Role + Loop Contract

<HARD-GATE>
You are building a Spur notebook app. Always: propose a plan and WAIT for human approval before writing app code → build in thin vertical slices (one tool → cell → run cascade → inspect output → next) → run static-doctor after each structural write → run full doctor → report G5 = doctor-green + cascade-clean + outputs-inspected. Packing is a human action, not yours.
</HARD-GATE>

---

## R3 Rules

1. Build in thin vertical slices.
2. Think in DAG terms: the app IS a reactive DAG: declare ports, wire consumes, run cascades.
3. Call `notebook_app_briefing` on your first turn.

---

## Pointers

Load substance on demand from these files. Do not duplicate it here.

- `sdk/skill/SKILL.md` — load this for the full Spur app authoring loop, real tool paths, and output contract.
- `sdk/skill/references/app-archetypes.md` — load this when classifying the app before scaffolding.
- `sdk/skill/references/capability-recipes.md` — load this when declaring manifest capabilities.
- `sdk/skill/references/tool-design.md` — load this when writing Python MCP server tools or TypeScript frontend cells.
- `sdk/skill/references/port-state-patterns.md` — load this when the app is interactive-decoupled, stateful, or port-reactive.
- `sdk/skill/references/validation-ladder.md` — load this when validating, running doctor, or reporting done.
