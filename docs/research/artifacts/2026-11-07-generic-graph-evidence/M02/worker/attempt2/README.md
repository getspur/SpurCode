# M02 attempt 2: native constructor conservation

The three-file migration replaces eighteen explicit typed `None` fields with nine `..GraphIndexArtifact::empty()` tails. Every other source byte is preserved, including all counted 413 assertion macros, explicit legacy versions/hashes/IDs/selectors, temporal vectors and capacity expressions. No observer remains in source.

- Original PRE: `4a411dbd24f306a27b48b7e11b2ab381b7e39797`.
- Required recovered checkpoint: `f161471f542a8aadafffb00ac29ae195db82dc3c`, obtained by fast-forward. Attempt-1, ROOT PRE and recovery files remain unchanged.
- Source migration: `076aff8d53dcb50075b59083adb1025c5387c783` (9 insertions, 18 deletions).
- Native before checkpoints: `5fae8024064289e673834d62a877c77f0fecdb0d`, then `6b8d5e273`.
- Final native checks ran with fixed HEAD: `d256b1db743cf167d63a01fe3c4be0f50fbc8fde`.
- Source-only unified diff: [source-only.diff](source-only.diff); original context-bearing bytes: [source-only.diff.gz](source-only.diff.gz).

The current-source PRE verifier passed before source edits. The historical attempt-1 verifier ran once at recovery and was preserved verbatim. It is a historical source gate and must not be rerun after migration. Both complete live PRE envelopes exactly match the archived reloaded values: `sol_741613e11a004a45` SAT/pass and `sol_a33aa22358694b4b` UNSAT/fail. Fingerprint `a7991096b4283dccc5a146cdcad1ca9111bd74164b9e6cd5d56ef8e64a548df3`: 125 bindings, nine sites/cases, ten false controls, 33 variables, eight predicates. Accepted M01 POST pins were untouched. This PRE remains prospective.

There are 19 genuine native observations before and 19 after, covering all nine site kinds. All 17 observation predicates pass and ten independent mutations of actual exports are rejected. [native-conservation.json.gz](native-conservation.json.gz) contains every explicit before/after join. Keys include actual arguments, test/thread identity, native fixture header/source/commit identities, and occurrence for repeats. Core wire/Debug values and skipped node IDs are retained in full. Large benchmark payloads use native digest+length bindings. Rust computes Git blob identities, canonical BLAKE3 and typed Rust capability; Python only authenticates, decodes, joins and mutates copied exports.

| Constructor site kind | Before / after invocations | Actual wire bytes |
|---|---:|---:|
| `code_file_symbols_uses_symbol_uri_selector_for_legacy_empty_qualified_name` | 1 / 1 | 632 |
| `code_subgraph_table_format_returns_node_edge_tables_and_unresolved_edges` | 1 / 1 | 998 |
| `synthetic_history_artifact` | 2 / 2 | 31888798 |
| `synthetic_resolve_artifact` | 2 / 2 | 32342149 |
| `write_graph_with_empty_snapshots` | 1 / 1 | 578 |
| `write_graph_without_commit_index` | 1 / 1 | 433 |
| `write_temporal_fixture_artifact_for_backend` | 4 / 4 | 4240 |
| `write_temporal_resolution_fixture_artifact` | 1 / 1 | 8941 |
| `write_worker_graph_fixture` | 6 / 6 | 602 |

The temporal helper has four named cases, including Parquet/in-memory backends. The worker fixture has six named owning tests. Each benchmark factory is invoked twice, first by its original assertion route and then by Criterion. Both actual factories use 50,000 snapshots/eight-chain arguments, preserve their native commit-index identities and selectors, and run observation outside timed sections. The history route retains 1,000 assertion iterations and 250ms; resolve retains ten assertion iterations and a five-times speedup bound. Local and native history override variables are unset. The remote release test proxy is supported: the history filter reports one Criterion Success and resolve reports two, with real factory exports in both phases. Custom Criterion routes do not report libtest ignore/filter counts.

All commands use `scripts/spur-cargo`, required remote/no-local-fallback/full-sync flags, and stable source/HEAD identities. [command-results.json](command-results.json) records exact argv/environment/timestamps, named outcomes, counts and raw log hashes. Every complete raw command log is losslessly compressed. Before/after observer patches are preserved byte-exact in `observer.patch.gz` and `after-observer.patch.gz`; temporary-directory replay reproduces every compiled source SHA256. Native readbacks independently match local before/after/final source identities and record compiler/executable hashes. Compiler: Rust 1.94.1 (`e408947bfd200af42db322daf0fadfe7e26d3bd1`), aarch64, LLVM 21.1.8.

| Native route | Before pass/fail/ignore/filter | Final pass/fail/ignore/filter |
|---|---:|---:|
| Full `code_graph_e2e` | 44 / 5 / 0 / 0 | 44 / 5 / 0 / 0 |
| Owning core observation cases | 8 / 0 / 0 / 41 | 8 / 0 / 0 / 41 |
| Full `worker_server_dispatch` | 21 / 1 / 0 / 0 | 21 / 1 / 0 / 0 |
| `spur-graph --lib` | — | 589 / 0 / 3 / 0 |

Every libtest route reports zero measured cases. Both package all-target checks exit 0. Local scoped formatting, full source/evidence diff checks and frozen PRE replay pass. The three pre-existing ignored library tests are CodeRankEmbed, NomicEmbedTextV15 and JinaEmbeddingsV2BaseCode model-download smoke tests; their exact names are in `verification.json`. No assertions, ignores, thresholds, sizes, capacities or filters were weakened. The eight exact observation cases are explicitly named in command metadata; required final suites are unfiltered.

The full suites retain baseline RED. Three core real-tool fixture failures report `tuple owner absent or ambiguous in graph`: `code_resolve_prefers_real_submit_plan_mcp_tool_registration`, `code_search_candidate_rows_include_enclosing_scope`, and `code_search_recovers_macro_bodied_callees_for_tool_definitions`. Two freshness assertions return `fresh` instead of `not_needed`: `code_search_ignores_untracked_non_source_scratch_file` and `code_search_unrelated_edit_does_not_rebuild`. Worker `tools_list_returns_curated_worker_tools_including_code_graph_reads` reports actual 34 versus expected 31. These failures reproduce before/after; full raw details remain archived. `bd-16eq` was independently read back open and remains an analyst overlay risk; this task does not establish it as the cause of every listed failure.

The first sandboxed remote attempt exited 200 before Rust because EC2 was unreachable. The network-enabled retry reached AWS, started the configured builder and ran native tests; no local fallback, VM-token/target repair, quota/budget increase or source-stability bypass occurred. Tooling corrections are preserved separately: two temporary observer serializer statements were caught and removed before migration, patch-context whitespace was preserved via gzip, and the packet parser now uses authoritative libtest failure footers when nocapture status output interleaves. These are separate from native compiler/assertion outcomes.

Canonical issue: `bd-1zi2`, dispatch `f567f55354a44a99`. ROOT INTENT 4706 and worker INTENT 4716 are durable. ACTION/runtime 4717, baseline-risk 4722 and migration ACTION 4726 are durable with exact successful identical replays. Later optional issue/progress refreshes did not return; their requests and unreturned status are archived and do not establish a new receipt/state. See `record-status.json` for the final RECORD acknowledgment; unreturned calls are never claimed durable.

Replay packet verification:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M02/worker/attempt2/verify.py
```

[ROOT POST candidates](root-post-candidates.json) bind the final 125 source files (only the approved three differ), native joins, mutations and named command results. ROOT must perform a fresh actual-source/native-bound Jev POST and independent review before approval or M03. No self-approval, issue closure, main merge or successor dispatch was performed.
