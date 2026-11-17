# C++ repository extraction gap lab

[Proposal](../../../superpowers/specs/2026-11-18-cpp-repository-resolution.md)
· [Live notebook](/Users/kevintruong/project/spur-notebook/2026-11-17-cpp-extraction-gap-lab.ipynb)

This is before evidence and research scaffolding. Production extractor and
extension code are unchanged.

## What to inspect

- `experimental-notebook.ipynb`: immutable copy of the executed live notebook.
- `verification.json`: assertions covering the native repository consumer,
  compiler-grounded counterexamples, core/extension parity and finite Solve policy.
- `notebook-replay.json`: document-order execution receipts, including semantic
  output checks for native Rust panics rather than relying only on tool status.
- `source-identities.json`: current source/query/harness hashes and worktree HEAD.
- `repository-summary.json`, `repository-validation.json`: all 4,924 C++/header
  files per revision, complete diagnostics and composite-key checks.
- `repository-manifest.json.gz`: complete pinned base/head Git inventories.
  `repository-manifest-identity.json` identifies the uncompressed manifest.
- `repository-unresolved-candidates.json`: 349 original unresolved call facts
  joined to the complete head callable-name inventory. Candidates are not edges.
- `rust-full-repository-summary.json` and
  `rust-full-repository-seed-witnesses.txt`: existing shared Rust builder over
  the same 4,924 verified head sources, with raw binding witnesses.
- `fixtures.json`, `fixtures/`, `compiler-ground-truth.json`: 14 syntax-valid
  C++17 cases, source hashes, Clang commands and compiler reference targets.
- `rust-core-probes.json`, `fixture-extension-results.json`: real extractor
  outputs and exact native-extension parity for those fixtures.
- `real-syntax-counts.json`: syntactic call-expression counts compared with
  emitted call facts; those are different denominators.
- `solver-observed.json`, `solver-proposed.json`: exact bounded models and raw
  solver responses, including the initial server artifact-quota failures.
- `consumer-timing.json`: measured query batching experiment, not a release SLO.
- `capture-manifest.json`: final capture identities and checksums.

## Reproduce

Use the saved DuckDB **2.0.0** notebook runtime and the locally verified extension
package identified by `repository-identities.json`. Keep the pinned DuckDB Git
objects available in `/Users/kevintruong/project/duckdb`. The notebook configures
the `cpp-core` Rust profile against the actual `spur-graph` package with default
features disabled. It also executes a project example through `scripts/spur-cargo`:

```sh
SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo run -p spur-graph \
  --no-default-features --example cpp_gap_probe -- \
  crates/spur-graph/examples/cpp_gap_probe_result.json
```

The example embeds the 14 fixture inputs and two pinned real sources in adjacent
JSON files. It compares single-file extraction with the existing shared builder.
The notebook separately calls the shared builder directly over the full pinned
head source inventory.

`repo_extension_probe.py` is the research SQL consumer. Its exact invocation,
script hash and package hash are in `repository-launch.json`. The main local
cache lives at `.spur/artifacts/cpp-gap-repository-index/repository.duckdb`.
The notebook detaches its read-only SQL attachment before launching the writer
and waits for worker success before opening results. An identical rerun reuses
the 4,926 identity-checked extraction keys. A different package/revision requires
a separate output directory.

The original per-file SQL pilot was intentionally stopped after 735 completed
inputs; its interruption receipt is retained. The final consumer batches rows
by source directory, retaining the same native function and source identities.
`repository-worker.log` includes the resumed full extraction and cached replays;
cached replay timings are not cold indexing timings.

The shared Rust scope initially invalidated an assumption that all 366 facts
would remain calls. The failed assertion is retained in
`rust-whole-scope-assumption-failed.json`; the final probe separately measures
353 calls and 13 constructions. A notebook tool status of `succeeded` alone
did not establish that the Rust assertion passed. Final execution verifies
outputs and persisted measurements.

Compiler validation is limited to the controlled fixtures. Whole-repository
bindings remain syntax heuristics, including the 53 cross-file changed-file
bindings. Other source languages, system/generated files, compilation-database
semantics, production uplift and a new full extension release are not claimed.
