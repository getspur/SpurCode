# Workspace panic graph experiment — 2026-11-01

The existing `spur-graph` panic-evidence executable compiled and ran successfully with the pinned CGA producer. Its single `panic_evidence::main` root reaches a graph containing 77,468 nodes and 246,520 observations. The full `spur-cli` attempt failed inside rustc/CGA while resolving `std::ptr::drop_glue::<S>`; it produced no `spur::main` export.

The [Notebook MCP experiment](../../../../2026-10-31-panic-graph-experiment.ipynb) contains the commands, live compiler receipts, SQL tables, directed path query, source matches, and finite-model receipts. This extends the [accepted fixture pilot](../2026-11-01-panic-evidence-pilot/README.md) with raw observations from actual workspace targets. The typed importer still accepts bounded fixtures only; these captures have not been relabeled as fixtures or certified through that API.

## Compilation and identity

| Attempt | Exact Cargo exit | Cargo-reported time | Interpretation |
|---|---:|---|---|
| `spur-graph --lib`, CGA input unset | 0 | 1m18s | Compile-only baseline; zero exports |
| `spur-graph --lib`, extraction enabled | 0 | 3m55s | 79,008 nodes, 250,319 observations, 13 async coroutine roots |
| `spur-cli --bin spur`, extraction enabled | 101 | No successful finish | Producer ICE in `spur-cli` library; dependency fragments only |
| Existing `spur-graph --example panic_evidence` | 0 | 3m30s | 77,468 nodes, 246,520 observations, one executable main root |

All four invocations use `SPUR_REMOTE=0 scripts/spur-cargo build --frozen --offline`, `--no-default-features`, and target `aarch64-apple-darwin`. Extraction additionally sets `CGA_INPUT='{"extract":true,"lint":null}'`. The source HEAD during compilation was `5c75446f1f01b0a5b70a48426f9b51dd822a2349`, including the five protected pre-existing dirty files recorded in [before.json](before.json).

The producer is CGA revision `2fc63108f0b674fe7a178191ffa15647a18879f1`, with the previously recorded macOS rpath build patch. Driver SHA256 is `5647b7f2c7f857dda0dd0204e91d870287988181ca5ec4d7792810a561886172`; nightly is `nightly-2026-10-18`, rustc commit `8006091eb71ae7d9e9bccb66c6122c9d68af4466`. Explicit flags are:

```text
-Zalways-encode-mir -Zhint-mostly-unused -Copt-level=0 -Cpanic=unwind
-Cdebug-assertions=on -Coverflow-checks=on -Cforce-frame-pointers=yes
```

The compiled example also executed its `snapshot` JSON command against a separate synthetic source directory with exit 0. This smoke check exercises the actual sidecar; it does not execute that source's Rust body or demonstrate a runtime panic. The binary hash and request/response hashes are in the capture records. No binary was installed.

## What changed in the seven-case frontier

The library capture has zero matches for the seven function names even before source-location filtering. This is a coverage limitation, not a safety result. The pinned producer explicitly uses `NoConstOrStatic`; ordinary library functions are not automatically roots, while public async coroutine bodies and executable entry roots have special handling. [cga-root-contract.json](cga-root-contract.json) preserves exact indexed source for that contract.

The successful example graph has source-bound observations in five function regions. A function-region match is a wider measure than an exact match for the originally collected `expect` or `unwrap` site.

| Original case | Observations in function region | Finding at the original site |
|---|---:|---|
| `run_full_walk_into_with_stats` | 0 | No emitted source match |
| `write_edges` | 0 | Macro-token witness remains; no compiler match |
| `rebuild_from_buckets` | 135 | Function region observed; collected closure `expect` has no exact match |
| `stable_file_id_from_path` | 10 | `Result<[u8; 8], TryFromSliceError>::unwrap` observed within recorded `format!` invocation span |
| `ExternalImportRegistry::write_to_buckets` | 21 | Exact call span → `Option<u64>::expect`, owned by the compiler closure |
| `compute_anchor_hash` | 19 | Exact call span → `Result<[u8; 8], TryFromSliceError>::expect` |
| `resolve_bare_pending_edge` | 92 | Exact call span → `Option<&PendingEdge>::expect`; eight additional `BuiltinAssert` observations |

Thus the result is **two exact function call matches, one exact closure call match, and one macro-token `unwrap` correlation**. The closure's outer graph caller remains null. The macro correlation does not establish an expansion provenance map. Source hashes, Git blob OIDs, UTF-8 byte ranges, one-based Unicode scalar coordinates, caller header locations, and raw graph edges are preserved in [example-frontier-correlations.json](example-frontier-correlations.json).

All eight implicit-check observations map to separate `*ambiguous_unresolved += 1` source ranges. The capture enables overflow checks, but CGA's `BuiltinAssert` export has no reason payload; the ledger retains that uncertainty. These are compiler observations, not eight demonstrated failures.

## Candidate paths need value-flow conditions

Notebook DuckDB/duckrouting found and independently checked a six-hop, directed `Called`-only path in the library capture:

```text
code_read_symbol coroutine
  -> code_graph_backend_value_allowing_source coroutine
  -> code_graph_backend_value_with_format coroutine
  -> open_code_search_backend_for_request coroutine
  -> resolve_artifact_location
  -> tracing::LevelFilter::current
  -> core::panicking::panic_fmt
```

[library-panic-path.json](library-panic-path.json) binds every hop to the raw export. `FnPtrCast` is address flow and is excluded from executable-call traversal. `VTableCalled` remains a separate observation kind. A structural path does not establish that its panic branch is feasible.

For the tracing branch, cached `tracing-core 0.1.36` source bytes were checked against the exact crate tarball whose SHA256 matches Cargo.lock. The private level value is initialized to OFF and the inspected typed setter writes six valid representations. [tracing-source-contract.json](tracing-source-contract.json) records the exact source selections and model boundary.

The one-step workflow model has three persisted, reloaded solver receipts:

| Query | Status | Receipt |
|---|---|---|
| Valid OFF → TRACE witness | SAT | `sol_c977ef0429054438` |
| Reach an invalid representation under typed-setter transitions | UNSAT | `sol_f84468625d604da4` |
| Add a hypothetical OFF → invalid write | SAT | `sol_681709bbd37a4ca9` |

The invalid-state query does not include a safety-invariant constraint that would assume the desired result. Replay independently enumerates the declared one-step transitions. This model checks a normalized contract; it is not a Rust memory-model proof, an unbounded result, or evidence of a real invalid write.

## Remaining gaps and effective next work

1. **Capture coverage:** record target roots and site-level denominators for every build. The example observes five regions but only three collected method sites exactly. Recovering a full CLI capture first requires isolating the producer's generic drop-glue ICE.
2. **Workspace import:** add an explicit workspace-target capture contract before accepting these observations through the typed importer. Bind target/features/configuration, source bytes, producer lineage and actual compile completion; retain unavailable states.
3. **Association:** preserve closure identities and add compiler-backed macro expansion provenance. The `format!` source correlation is useful evidence but does not fill that provenance gap by itself.
4. **Panic feasibility:** enrich implicit checks with producer-supplied reasons and value-flow guards. The tracing candidate shows why call reachability alone can overstate panic risk.

No workspace runtime panic or whole-workspace safety conclusion is claimed. The graph pointer and v11 manifest remained unchanged, as did all five protected user-file hashes. The initial library inventory covers 1,711 declared tracked build inputs; the CLI and example inventories cover 3,507 tracked non-notebook regular files. Before/after equality is recorded, while registry/git/native build effects are not claimed fully closed. Cargo can replay cached `cga-emit` stderr, so marker counts are not fresh compiler invocation counts.

## Replay and retained data

```bash
python3 docs/research/artifacts/2026-11-01-panic-workspace/replay.py
```

Replay requires Python's standard library and no compiler, network, live graph, or temporary build directory. It checks the [payload index](evidence-index.json), four selected verbatim raw exports, node/edge references and counts, archived source hashes/OIDs, all 277 frontier observations, exact caller/closure associations, the directed candidate path, compile receipts, sidecar smoke response, external source bytes, and the three finite-model receipts. Notebook cell `2e2a3e01-7aec-416e-87a6-4bc2b8929330` runs this replay. Following SQL cells inspect the retained Parquet data directly from this directory.

`capture-records.tar.gz` contains exact commands/environments, compiler logs, input inventories, export indices, observer diagnostics and the failed worker-audit request. The observer's assignment check rejected its audit RPC; root preserved the complete request through the authorized issue API and reloaded it exactly in Beads comment 1002. That repair is recorded separately from compiler evidence. The observer had an intentionally empty code diff.

The four retained raw exports are the successful library graph, successful example graph, and two dependency fragments from the failed CLI attempt. Other dependency exports have an index and worker reference-validation records; their raw bytes are not retained here and cannot be replayed from this dossier. The archive contains no target directory, compiler binary, Cargo cache, or raw notebook copy.
