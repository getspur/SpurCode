---
name: spur-review-evidence
description: "Use when working with the spur-review-evidence Spur App — drives the app notebook frontend."
---

# spur-review-evidence — Spur App Skill

Archetype: `vanilla-widget` with no MCP server.
Authoring reference: `sdk/skill/references/app-archetypes.md`.

<HARD-GATE>
Operate the notebook ONLY through MCP tools (`notebook_run_cell`).
Never ask the user to paste code or open files.
</HARD-GATE>

## The loop

1. Open `app.ipynb` in app mode.
2. Build the app surface as frontend cells whose outputs carry `text/html`.
3. Run `notebook_app_doctor` before every pack; fix every `fail` finding.
4. Pack with `notebook_app_pack` only — never hand-roll a `.spurapp`.
