# Experimental GitHub + DuckTails + Spur Graph + DuckPGQ POC

[Results report](../../2026-11-17-spur-review-extensions-poc.md)

Live notebook: `/Users/kevintruong/project/spur-notebook/2026-11-17-spur-review-extensions-poc.ipynb`.
The captured `experimental-notebook.ipynb` has 23 cells: 19 executed code cells and four markdown cells. It was read through notebook MCP after a fresh-kernel pass at document revision 150 and runtime generation 2.

## Reproduce

Open the live notebook in Spur Notebook, select DuckDB 2.0.0 and run in document order. The saved github-pat connection is attached and its managed setup cell restores the REST bridge. The connection resolves credentials through the existing notebook environment.

The first experiment installs DuckTails and DuckPGQ from community and Spur Graph from the existing rebuilt local artifact. Native binaries are kept under task-owned .spur/artifacts paths and are not committed here. Exact installed binary hashes and versions are in extension-binaries.json and extensions.json.

The notebook requires the local DuckDB Git object database and the captured PR metadata in pr-context.json. Its source comparison is pinned and does not change branches. The curated graph and two historical identity mappings reuse tracked artifacts from the prior PR #28731 simulation; source hashes are revalidated.

executed-queries.sql is a transcript of the eight SQL cells, **not a standalone driver**. Execute them interleaved with the Python setup/source cells as shown in the notebook.

## Evidence map

| Files | Meaning |
|---|---|
| extensions.json; extension-binaries.json | Actual installation/load/version receipts and SHA256 identities |
| github-inbox.json; pr-context.json | Real saved-connection inbox data and separately captured full PR metadata |
| github-sql-batch-probe.json; github-materialization-probes.json | Initial batch failure, unchanged successful retry and single-query materialization probe |
| comparison.json; changed-files.json; native-inputs.json | Pinned Git comparison, five-file diff and four extraction input identities |
| native-facts.json; native-symbols.json | Raw source facts and symbols for both C++ files at both revisions |
| raw-change-candidates.json; semantic-changes.json | Six raw changed function candidates, five normalized semantic seeds |
| node-aliases.json; graph-nodes.json | Explicit containment normalization and scoped vertex identities |
| resolved-calls.json; unresolved-calls.json; edge-profiles.json | 17 resolved and 349 unresolved call sites, with raw relation metadata |
| native-blast-radius.json; graph-parity.json | 13 bounded caller/seed pairs and exact recursive-SQL equivalence |
| curated-nodes.json; curated-edges.json; curated-propagation.json | Separate source-inspected cross-file layer, not native resolved calls |
| base-history.json; base-blame.json; github-identity-evidence.json | Base-pinned history/blame and previously verified GitHub identity evidence |
| stakeholder-candidates.json; component-rollup.json; review-join.json | Historical expertise, explicit component mappings and five combined review rows |
| arrow-seam.json; legacy-arrow-probe.json | Actual Arrow conversion and deprecated-alias behavior in this kernel |
| review-evidence-packet.json | Versioned export for future app/agent consumption; AI status is not_run |
| verification.json; fresh-run.json; notebook-context.json | Nine check groups and all 19 code cells passing after kernel restart |
| notebook-native.png; experimental-notebook.ipynb | Inspected native viewport and immutable executed notebook capture |
| capture-manifest.json | Capture identities and SHA256 inventory |

The graph is limited to two changed C++ files. A missing native path does not establish no impact. Broader propagation is explicitly curated, architecture alignment is not evaluated without an approved policy, and current reviewer assignment remains unknown.
