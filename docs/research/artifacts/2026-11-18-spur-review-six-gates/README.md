# Six-gate grounding and repair evidence

[Report and repair contracts](../../2026-11-18-spur-review-six-gates.md) · [Results and exact Solve inputs](results.json) · [Captured lab](experimental-notebook.ipynb) · [Reference app](app/app.ipynb)

Live lab: `/Users/kevintruong/project/spur-notebook/2026-11-18-spur-review-six-gates-lab.ipynb`. Task `bd-2gmj`; source investigations `bd-xea7`, `bd-eg02`, `bd-2uao`.

All notebook authoring, profile changes, execution, app scaffolding and inspection used notebook MCP. The connection metadata correction used the app-owned supported daemon control API because the attachment MCP surface could not retain those fields. The correction reused the existing credential reference with no supplied secret values. Exact restoration booleans are in `results.json`.

`app/` is a generated native reference package containing the captured HTML view, not the finished named-runtime application. Its manifest is embedded in `app.ipynb`. Doctor Level 2 and the display/cascade are verified; host browser interactions, live port refresh and pack are not. Generated writer locks are excluded from the capture/commit.

The lab uses a prepared existing Python 2.0 interpreter and the isolated project fixture from the preceding evaluation. Profile definitions retain their exact machine-local paths in the receipts; use those only on this host. No environment installation or extension rebuild was performed.

The two origin cells must run A then B in the same named Python process. The grammar cell deliberately reproduces failure and restores a working connection setting. The rejected named Python producer was never inserted. The two named Deno cells retain no execution/output after MCP startup rejection. SQL inbox uses the separately managed factory connection; it is explicitly distinguished from named Python.

R4 and reference-package R5 qualify. R1/R2/R3 and automatic R6 remain unqualified. `verify_evidence.py` checks evidence consistency and records the negative results; it does not claim runtime implementation acceptance.
