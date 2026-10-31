# Panic evidence pilot — 2026-11-01

The integrated P1/P2/P3 sidecar was accepted after test-first development, SOLVE receipts, 1,170 graph tests and fresh Notebook MCP capture. The published v11 graph was preserved.

- `summary.json`: measured build/configuration and source-frontier results.
- `verification.json`: root acceptance gates, exact source tip, test log hashes and preserved state.
- `workspace-frontier.json`: the original seven labels mapped to three exact callers, two closure sites and two macro-token cases. All workspace compiler targets remain Unknown.
- `configuration-analysis.json`: five captured builds and 45 executable controls (32 panic exits, 13 normal exits).
- `typed-analysis.json`: canonical standard/custom callees, generic multiplicity, Unicode span and conditional compiler checks.
- `capture-summary.json`: raw export hashes and capture IDs.
- `capture.tar.gz`: exact compiler exports, input inventories, identities, full requests/responses, witnesses, command records, stdout/stderr and seven fixture inputs. Build targets and caches are excluded.
- `workspace-subset.tar.gz`: exact five-file subset artifact, sources, request, runner identity, time output and process sample. The full debug snapshot took 103.27s; the sample is from the pre-existing macro-validation path before method collection.
- `verification.tar.gz`: worker reports, referenced test logs, Beads comment snapshot (complete PRE/POST receipts), root integration logs and the independent reviewer contract.
- `evidence-index.json`: SHA256 and byte counts for payloads and archive members. Archive members are read in memory; no extraction is needed.

The fresh capture is initiated by cell `3178a0a9-0c59-4398-83d5-34511ea780c9` in `2026-10-31-panic-graph-experiment.ipynb`. A final replay cell verifies the durable payloads and reads the archives after temporary capture directories are gone. The capture recipe is `scripts/experiments/panic-evidence-capture.py`; the JSON boundary is `crates/spur-graph/examples/panic_evidence.rs`.

This is a build-bound evidence pilot. It does not claim whole-workspace compiler coverage or panic freedom. Priorities supported by the measured gaps are compiler closure ownership, expansion-to-invocation mapping and reuse of existing artifacts to avoid snapshot extraction cost.
