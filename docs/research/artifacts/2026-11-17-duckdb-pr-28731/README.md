# PR #28731 review evidence

The active notebook is `/Users/kevintruong/project/spur-notebook/2026-11-17-duckdb-pr-28731-review-simulation.ipynb`. `review-simulation.ipynb` is its immutable capture after notebook MCP revision 76. It contains 21 cells, including 14 executed code cells and no saved error outputs.

Read the [assessment](../../2026-11-17-duckdb-pr-28731-review-simulation.md) for the four review journeys and capability gaps.

## Reproduce

Open the notebook in Spur Notebook, select DuckDB **2.0.0**, and run from the top. Python and SQL cells use the managed kernel's shared DuckDB connection. Local extension binaries and the DuckDB Git object database must be present at the paths shown in the notebook. Binary hashes are in `extension-runtime.json`; binaries are not committed here.

`queries.sql` provides the standalone DuckTails diff/history/blame recipes. The final verification cell executed this file and checked its 116-line blame total. The notebook binds exact Git blob bytes to the native Spur Graph scalar functions and records both Git object IDs and SHA256 identities.

The simulation is pinned to base `629e395a854d72dcb1f414e1a0451c68ac0e9f1f` and head `d29b26bb92ac940a0807be0f43cc385b31623839`. It does not depend on the repository's checked-out branch. Checkout drift observed during the investigation is recorded in verification.json.

## Evidence files

| File | Meaning |
|---|---|
| extension-runtime.json | Actual loaded extensions, runtime version, binary hashes and diff schema |
| changed-files.json | DuckTails base/head file diff |
| source-manifest.json | Native extraction input Git blobs, source hashes and symbol counts |
| native-symbols.json | Raw native symbols, tokens, spans and exact body hashes for both revisions |
| changed-function-candidates.json | Six unique-key body-change/addition candidates before presentation normalization |
| semantic-changes.json | Five semantic functions after collapsing the overlapping template/function representation |
| facts-summary.json | Four successful native diagnostic extractions and fact counts |
| source-evidence.json | Twenty source-inspected points, revision-pinned links, hashes and excerpts |
| impact-edges.json | Fourteen structural source paths, one inferred eligibility edge, one unproven candidate |
| graph-provenance.json | Graph freshness limits; empty temporal data is unavailable history |
| file-history.json | 65 base-pinned file-history rows, fixed lower date and per-file cap |
| base-blame.json | 158 attribution lines from explicit base-revision semantic regions |
| cochange-sample.json; cochange.json | Eight non-merge commits retained from 12 recent distinct history commits; historical co-change only |
| github-snapshot.json | PR/current review snapshot, capture time, unknown assignment fields and first workflow page |
| github-identity-evidence.json | Commit API evidence mapping the two primary technical candidates to GitHub logins |
| stakeholders.json | Confirmed participants and historical expertise candidates, with evidence and review questions |
| verification.json | Executed consistency checks, view counts, source/extension validation and checkout drift |
| stakeholder-view-native.png | Full-resolution native screenshot inspected for rendering and table usability |
| capture-manifest.json | File hashes, final PR revision check and notebook capture identity |

No PR build, upstream regression suite or benchmark was executed here. No GitHub comments, messages or review requests were sent. Architecture and impact annotations are source-inspected enrichment rather than automatic whole-repository analysis. This is an analysis notebook, without app frontend cells or published ports.
