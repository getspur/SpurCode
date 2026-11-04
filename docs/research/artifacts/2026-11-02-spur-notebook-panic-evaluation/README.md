# spur-notebook panic detector experiment

The notebook's **native Rust cells** reproduced UTF-8 boundary panics in two unchanged source helpers from `../spur-notebook`.
The larger Rust matrix ran **4,352 calls**, catching **24 panic cases at those two helpers**.
These are function-level observations. The current automatic detector did **not** discover those bugs end to end: its original-file semantic runs remain unknown or incomplete.

Open [the executed notebook](../../../../2026-11-02-spur-notebook-panic-evaluation.ipynb).
The target snapshot is HEAD `7fb364638575da96d46a68c5ec6af0215fe97300`, including its recorded working-tree bytes.
All 721 captured files (689 Rust files) matched the live checkout at the final source-stability check.
The detector is the previously validated binary SHA-256
`984ee7bc02555b9c831addac41ce44d0bf323bf685f011c52078f815363eb5cd`.

## Actual native findings

| Source helper | Input and observation | Scope |
|---|---|---|
| `jute-notebook/src-tauri/src/state.rs:263`, `repair_object_store_location`, reached through unchanged `is_object_store_location` | `aaaaaaaé`: byte index **8** is inside é (bytes 7–9), causing a caught panic at the first prefix comparison | Exact helper, caller helper and scheme constant compiled in a Rust cell |
| `rest-table-gateway/src/adapter/manifest_adapter.rs:43`, `apply_header_value_prefix` | Prefix `Some("Basic ")`, value `aaaaaé`: byte index **6** is inside é (bytes 5–7), causing a caught panic | Exact helper compiled in a Rust cell |

The function bodies, signatures, constants and guards were copied byte-for-byte from captured source.
The notebook adds modules and call wrappers; it does not compile or run the entire Tauri application.
Exact file/segment hashes and byte/line ranges are retained in `replay-source-segments.json` and `native-fragment-manifest.json`.
Application call-path execution and exploitability are not established.

An initial smoke run had **2 caught panics and 9 normal controls**.
Four separately identified analyst-authored UTF-8 site models yielded **2 SAT** witnesses and **2 UNSAT** controls.
Their exact requests were preflighted, persisted and reloaded through MCP. The SAT witness strings were then replayed in Rust cells, reproducing both exact boundary payloads.
Those models were written from the source conditions; they are **not automatic detector-generated Rust facts**.

The native matrix uses 16 ASCII-prefix lengths and four scalars (`a`, `é`, `界`, `🦀`), plus an ASCII suffix.
It tests each string against the original Jute helper, original header helper and existing guarded object-store helper.
It also tests the original `bounded_text` function over 65 budgets per string.

| Family | Native calls | Caught panics |
|---|---:|---:|
| Jute prefix helper | 64 | 18 |
| Header prefix helper | 64 | 6 |
| Existing guarded object-store helper | 64 | 0 |
| Existing guarded `bounded_text` | 4,160 | 0 |
| **Total matrix** | **4,352** | **24** |

All finite predictions matched the observed results. Zero panics in a control domain is not a general safety proof.
The Rust kernel reported Rust **1.94.1**, host aarch64-apple-darwin, debug assertions enabled, unwind panics and 64-bit pointers.
No notebook dependency was installed; these source fragments use the standard library.

## What the actual automatic detector returned

Five complete original files were analyzed without deleting imports, attributes, tests or guards.
Their physical graph snapshots contain **1,846 method sites, 564 macro sites and 2,668 Calls** in total.
These counts have different grains and do not measure recall.

| Original file | Automatic semantic outcome |
|---|---|
| Jute `state.rs` | `unknown`: unevaluated `cfg/cfg_attr` |
| `manifest_adapter.rs` | `unknown`: unevaluated `cfg/cfg_attr` |
| Product `mcp/datasource/object_store.rs` | `unknown`: unevaluated `cfg/cfg_attr` |
| `dag/ai/inputs.rs::bounded_text` | `incomplete_semantics`, raw UNSAT, **8 obligations**, no witness |
| `driver_zeromq.rs::from_zmq_payload` | `incomplete_semantics`, raw UNSAT, **12 obligations**, no witness |

The two automatic typed requests were independently preflighted, solved and reloaded at the MCP endpoint.
They remained raw UNSAT with their original incomplete outcomes and obligations preserved.
The new MCP IDs are separate from the original CLI-local receipt IDs.

The available `u8`/Bool/Option subset does not model these general string/slice/runtime types.
File-level conditional-compilation uncertainty also prevents analysis of otherwise unconditional helpers.
Native Rust cells validate concrete execution; they do not supply the missing automatic semantic coverage.

## Snapshot and scope corrections

The registered project graph points to a missing older `/Volumes/Projects/spur-notebook` path.
This experiment therefore captured current source and built isolated task-local snapshots.

The first aggregate snapshot and five per-file snapshots used source roots nested inside the unrelated SpurCode Git worktree.
They failed with `missing macro source entry` at the artifact path-rebinding seam.
The same unchanged Jute file succeeded from `/private/tmp`.
Moving input roots and cwd outside that enclosing worktree made all five original-file snapshots succeed.
All six original failures are preserved; they are not represented as successful detection or as target compilation failures.
The detector implementation was not patched to change this outcome.

A corrected broad source-only snapshot requested **522 project-owned Rust source files** from the full archived inventory.
It produced no completed snapshot within the observation budget and was terminated after **645 seconds** (the decision threshold was 600 seconds).
This is an observed scalability limit of this run, not a compiler error or a project-wide no-panic verdict.

## Independent source review

The approved independent review, authored as `b4b0de765`, records **seven conditional production candidates and three protected controls**.
All 11 cited file identities were independently checked by root.
Intent, worker completion and orchestrator completion audits were verified in the tracker; the initial authorization failure and model-capacity attempt were not counted as completed review.

The review corroborates the two Unicode preconditions and records source callers.
Other candidates require malformed kernel frames, malformed Arrow/footer state, overflow configuration, theoretical event-counter exhaustion, or prior writer-lock poisoning.
They were not promoted to reproduced application bugs.
See [the review](independent-review/REPORT.md) and [candidate ledger](independent-review/candidates.json).

## Interpretation and next implementation priorities

The experiment validates direct Rust-cell reproduction on a real separate codebase.
It also exposes the distance between the existing bounded detector and useful automatic project analysis:

1. Resolve conditional compilation per body and crate configuration, preserving unknowns where configuration is unavailable.
2. Add typed `usize`, string lengths, UTF-8 boundaries, ranges and checked-`get` guards with exact value origins.
3. Preserve graph/source identities when snapshot inputs are nested under another worktree.
4. Measure and improve broad extraction before promising whole-project runs.
5. Keep source-bound solver witnesses and direct Rust replay as separate, linked evidence.

This study supplies two reproduced source-helper failures, not automatic discovery coverage or a panic-free project result.

## Evidence and recheck

Small summaries, native fragment files, solver records and native TSV/Parquet results remain directly readable.
Large immutable inputs and original detector request/output files are retained in `evidence.tar.gz`;
`bundle-manifest.json` binds each uncompressed member by path, size and SHA-256.
Private solver stores, targets and caches are excluded.
The original raw files remain in the active experiment workspace for the executed notebook.

Run `python3 verify.py` from this directory to check retained source identities, exact fragments, requests/receipts,
native payloads, all matrix rows, independent source bindings, scope corrections and the evidence archive.
The verifier is read-only and uses only the Python standard library.
It does not execute Rust, reload private solver storage or claim new detection coverage.

For direct reproduction in a fresh Rust kernel, run the notebook's Rust probe and native reproduction cells using the committed `native-rust/*.rs` fragments.
The solver-witness and matrix cells use the same helpers. Python cells prepared source/JSON artifacts; SQL cells aggregate measured outputs.
Re-running the full capture workflow requires a fresh output directory and the explicitly identified detector binary; it must not overwrite the recorded run.
