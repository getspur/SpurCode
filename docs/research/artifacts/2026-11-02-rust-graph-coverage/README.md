# Rust source and Spur graph coverage

Open [the interactive report](coverage.html), [the overview](coverage-overview.png), or the executed notebook at `../../../../2026-11-02-rust-graph-coverage.ipynb`. The report includes a clickable 689-file map, file/category filters, declaration line spans and per-symbol drilldowns. The notebook adds GGSQL and Perspective views.

Root task: **bd-33a**. Independent audit: **bd-l8h**, source lineage **bd-1xw**, approved plan `5d856f61-02d0-4341-811b-0e6cae45ce02`. Production code in both projects was left unchanged.

## Measured result

| Layer | Result |
|---|---|
| Physical Rust source | 689 tracked/nonignored Rust files; 366,382 physical lines; 13,351,580 bytes |
| Independent parser | Native Rust notebook, syn 2.0.118: 688 complete files parsed; one directive-bearing fragment excluded |
| Published file freshness | **651 source matches, 17 stale, 21 unindexed** |
| Published callable declarations | **11,517 / 12,682 verified (90.8%)**; 797 stale, 368 unindexed |
| Callables within source-matching files | **11,517 / 11,517 represented** |
| All declaration kinds | 23,197 / 26,507 represented; 2,144 stale; 790 unindexed; 376 without individual symbol correspondence |
| Current file-local extractor | **683 successful captures, six 30-second timeouts** |
| Current callables in successful files | **11,291 / 11,291 represented**; 1,391 further callable declarations unmeasured |
| Current method identities | **72,149 exact matches**; 9,797 further AST sites unmeasured |
| Current macro identities | **24,754 exact matches**; 3,608 further AST sites unmeasured |
| Free-call invocation identities | Not exposed by this capture API; no recall percentage claimed |

These are different eligible populations, not a before/after improvement estimate. Coverage is **physical source representation**, not executed line coverage or proof that code cannot panic. Callables include ordinary functions, methods and trait methods; nine foreign-function declarations are separate.

## Source and graph binding

Target: `../spur-notebook`, captured HEAD `2a18c60cdd09dcdf01e6a6e38fe4bbc9a3385b97`. Every file has a SHA-256 and Git blob OID in `source-manifest.json`. The denominator comes from `git ls-files --cached --others --exclude-standard -- '*.rs'`, restricted to existing files. Ignored build outputs and external dependency sources are excluded.

The published graph is **spur-graph-schema-v10**, hash `cbc715ac26ad217292466f09775b8eff130b0d49885af4d6817035fecd40bbc1`. Its `complete=true` describes artifact construction, not coverage of the current worktree. The registry points at unavailable `/Volumes/Projects/spur-notebook`; schema-discovered, read-only Parquet access was used. CURRENT was not rebuilt or replaced.

Current extraction used the previously tested binary, source commit `848a29f39511cbd27091beb6c171b97e179f353a`, SHA-256 `984ee7bc02555b9c831addac41ce44d0bf323bf685f011c52078f815363eb5cd`. Each file was extracted in an isolated private root. This measures syntax collection, with **no project-wide target-resolution claim**. Captured JSON header version is not a persisted-v11 completeness certificate; its method/macro rows are separate from published v10.

Successful captures retain compressed original stdout, request, duration, source identity and stdout/binary hashes. Six files exceeded the explicit 30-second budget at concurrency six. That is an experimental limit, not unsupported syntax. Their paths are in `summary.json`.

## What is counted

The independent syn visitor enumerates declarations, fields, variants and implementation blocks from physical source. Calls, macros, closures, async/const blocks and index operations are separate grains. Macro token trees remain opaque; no generated-item denominator is invented.

Symbol matching requires equal source identity, file, compatible kind, normalized raw-identifier name, identifier containment and exact declaration end. Implementation containers use file/kind/end containment. Represented symbol/site IDs are used at most once. Stale files are rejected before matching.

Method matching uses source identity, name, receiver start and exact call end. Eight expression attributes initially changed the full AST span; exact method-token positions independently verify their normalization. Nested/repeated sites remain distinct. See `calibration-verification.json` and `method-gap-review.json`.

The excluded `jute-notebook/src-tauri/src/assets/ports_bootstrap.rs` starts with notebook dependency directives. Its 115 method and 11 macro rows are excluded from numerator and denominator alike. Dividing raw capture counts by AST counts would exceed 100%.

Categories separate production/mixed paths, test files, examples, build tools, vendor and archived/research copies. Unit tests can remain in mixed files. The `cfg_context` / `test_context` fields are inherited **item annotation hints**, not exhaustive expression metadata or evaluated cfg formulas. False flags do not imply unconditional or production-only code. These flags do not drive top-line coverage. No captured file had an explicit generated-comment marker in its first 20 lines; no marker does not prove handwritten origin.

## Remaining gaps

- Published v10: 38 files need freshness/index coverage before current declarations can be trusted.
- Observed tuple fields and foreign declarations lack individual correspondence. Direct node inspection checked all **376 published** and **406 current** gaps (`unsupported-node-check.json`). Parent type text may still contain those bytes.
- Free-call identity remains unmeasured by this API. The inventory additionally records **8,808 body forms** and **4,722 indexing operations**, without inferring execution or panic.
- Six timeout files contain 3,031 declarations and remain unmeasured.
- Cargo configurations, macro expansions, trait dispatch, implicit compiler checks, value flow, branch feasibility and runtime reachability need separate evidence. No complete-panic-detection percentage is inferred.

Practical next priorities are refreshing the target's published graph with a compatible reader, exposing/persisting general invocation/body evidence, and separately investigating measured extraction timeouts. Those implementation changes were not made here.

## Verification and replay

`verification.json` verifies source bytes/OIDs, AST ranges/identifiers, capture hashes, source bindings, per-file bounds, unique represented identities, method tokens, parser exclusions and saved HTML. Identical external endpoint nodes repeated across independent captures are permitted; physical represented declaration IDs remain unique.

The offline browser check verified 689 initial rows, 21 unindexed rows, 175 test-file rows, empty search, keyboard selection, declaration drilldown and no horizontal overflow. Charts and all file rows are pre-rendered; HTML uses no external resources.

`audit/REPORT.md` records the independent audit with hashes and counterexamples. `target-protection.json` verifies unchanged target HEAD, Git status, source bytes, graph pointer and manifest. No solver store was read and no project build was needed.

Standalone verification (Python with pandas/pyarrow):
```sh
python verify.py .
```

This reads `evidence.tar.gz` when raw loose inputs are absent; it does not extract or parse notebooks. To regenerate tables in a disposable copy of this artifact directory:

```sh
python -m tarfile -e evidence.tar.gz .
python analyze.py .
python verify.py .
```

The notebook is the executable source for Rust inventory, capture commands, SQL views, Arrow port and HTML rendering. `inventory.evcxr.rs` and `*.cell.*` preserve exact cell sources; they require notebook kernel state and are not standalone entrypoints. Dependencies/versions are in `inventory-dependencies.txt` and `environment.json`. Re-running live source capture creates a new measurement; do not mix new inputs with old captures.

Raw sources, AST rows, published graph tables and original captures are bundled, with member hashes in `payload-manifest.json`. The integration commit uses a dedicated plan branch; the user's active checkout was preserved.
