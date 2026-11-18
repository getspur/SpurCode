# Spur Graph Review UX design package

[Full specification](../../../superpowers/specs/2026-11-18-spur-review-app-ux.md) · [Interactive prototype](prototype.html) · [Verification](verification.json)

Live design notebook: [/Users/kevintruong/project/spur-notebook/2026-11-17-spur-review-app-design.ipynb](/Users/kevintruong/project/spur-notebook/2026-11-17-spur-review-app-design.ipynb). It contains the existing brief, refreshed specification, one Deno design cell, rendered HTML and handoff notes. [Captured notebook](design-notebook.ipynb) preserves this delivery independently of later edits to the live notebook.

## Try the design

Open `prototype.html` in a browser, or open the live notebook and enable active content for the artifact output. Its initial source, graph and paths remain visible when scripts are disabled.

1. Select a changed file or function in the left pane.
2. Switch between Context map, Path list and the actual pinned Source diff.
3. Select an incoming path, node or shared-contract witness. Read its exact source and limits in Evidence.
4. Hide candidate relations and observe the explanation; absence of those paths never becomes “no impact.”
5. Select `first_last_any.cpp` for the shared contract and possible callback flow. Open the dispatcher path and its four registrations.
6. Select `stats_dependent_tests.json` for inherited exclusions in unchanged configurations.
7. Switch to Architecture or People, then return. The selected change remains selected.
8. Preview the AI review packet and inspect a citation. This is captured evidence, not generated AI findings.
9. Inspect the Repository and Inbox design previews.

The prototype intentionally uses a curated PR neighborhood. It does not execute new graph queries when a question changes. Architecture and People show captured PR context while retaining the current file/symbol selection. Full filtering, expansion/pagination, review dispositions, persisted navigation history, source-side switching and job recovery are specified for the native implementation, not implemented in this design artifact.

## Grounding

The fixture is derived from the committed tracked-repository context experiment and prior extension POC. It includes five changed files, four context questions, 21 source-backed evidence records, six CMake components, the real base/head Git diffs and captured historical expertise candidates.

`prepare_data.py` selects source witnesses from `../2026-11-18-pr-repository-context-map/`, reads captured people evidence from `../2026-11-17-review-extensions-poc/` and reads immutable Git diffs from the local DuckDB object database. `design-data.json` is the resulting fixture. Native receipt indices are zero-based and retain graph-scoped provenance. A displayed source-context anchor is explicitly distinguished from an exact call-site coordinate.

`view.ts` is the shared stateless Preact view. `style.css` contains the light Tech Utility design tokens and responsive rules. `assemble_cell.py` embeds those files and the fixture in `design-cell.ts`. The notebook cell server-renders the baseline, bundles the client island at build time, emits exactly one `text/html` MIME result and saves the same HTML as `prototype.html`. Dependencies are bundled; the HTML requests no external resources.

This is a notebook design artifact. It is not a native frontend cell or packed app. The design notebook's unrelated default SQL runtime was not changed; the artifact runs in Deno and displays captured results of the DuckDB 2.0 experiments.

## Verification and critique

Final Deno execution succeeded, one output with only the `text/html` MIME key. The saved output was re-read through notebook MCP. Local headless Chrome passed **60 checks**, including source/receipt consistency, real interactions, candidate filtering, selection across lenses, callback registrations, CMake witnesses, configuration context, citations, keyboard activation, narrow/intermediate layouts, no external resource requests and a scripts-disabled baseline. This is not a complete accessibility audit or native app doctor run.

Screenshots:

- [Incoming clients and unchanged wrapper](01-incoming-context.png)
- [Architecture](02-architecture.png)
- [People](03-people.png)
- [Shared identity contract](04-shared-contract.png)
- [Inherited test exclusions](05-config-context.png)
- [Review packet](06-review-packet.png)
- [Repository setup](07-repository-setup.png)
- [Narrow viewport and keyboard focus](08-narrow.png)
- [Scripts-disabled baseline](09-scripts-off.png)

The visual review retained the engineering density and restrained neutral/green direction. It widened relationship gaps to keep labels readable, distinguished the selected symbol outline from other modified nodes, and checked wrapped C++ symbols and source text. The narrow preview deliberately stacks sections; the spec calls for full-width panel navigation in the native implementation. No invented performance/risk metrics, external fonts, decorative gradients or prohibited dash glyphs appear in the artifact.

To reproduce fixture and cell assembly from this workspace:

```bash
python3 docs/research/artifacts/2026-11-18-spur-review-ux/prepare_data.py
python3 docs/research/artifacts/2026-11-18-spur-review-ux/assemble_cell.py
```

Write/run the resulting cell through notebook MCP. The cell uses cached/build-time Deno dependencies and writes the HTML to this workspace's artifact directory; adjust that output path when replaying elsewhere. To rerun the captured-data browser checks, use `verify_design.py` with local Playwright and Chrome installed. No live GitHub access, token or AI provider is required.

The next production slice is captured typed run → selection → unchanged-wrapper path → exact evidence. Live collectors, reliable repository linking, versioned architecture policy and user-triggered AI execution remain implementation work.
