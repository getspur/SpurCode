# Rust source / graph coverage audit

2026-11-02 · actual assigned bead **bd-l8h**, verified `in_progress`; plan `5d856f61-02d0-4341-811b-0e6cae45ce02`, task `COV-AUDIT`. `bd-1xw` is lineage only. Inputs: `/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-02-rust-graph-coverage`. Target capture HEAD: `2a18c60cdd09dcdf01e6a6e38fe4bbc9a3385b97`.

**Finding:** the captured coverage joins and bounded denominators reproduce; no represented-row false positive or many-to-one mapping was found. Expression-level cfg/test metadata has a concrete false negative. This audit validates the supplied physical AST and captured graph correspondence, not exhaustive Rust grammar support or compiler semantics.

| Independently checked quantity | Result |
| --- | --- |
| Manifest / frozen source bytes | 689 unique paths; 366,382 lines; 13,351,580 bytes; every length, SHA-256 and Git blob OID verified |
| Parser accounting | 688 parsed files; 1 excluded include fragment; all per-file AST row counts agree |
| Physical AST | 205,726 rows: 26,507 declarations, 165,689 callsites, 8,808 body rows, 4,722 index operations |
| Published source states | 651 exact-OID matches, 17 stale, 21 unindexed; 668 unique captured file manifests |
| Published declaration states | 23,197 represented; 2,144 stale; 790 unindexed; 376 without a matching symbol |
| Current file-local captures | 683 success, 6 timeouts at 30.006–30.007 s; all successful stdout hashes, source hashes and graph/site source OIDs verified |
| Current declaration states | 23,070 represented; 3,031 timeout/unmeasured; 406 without a matching symbol |
| Current measured method/macro sites | 72,149 / 24,754 represented; 9,797 / 3,608 timeout; zero ambiguous or unmatched eligible AST sites |
| Free-call sites | 55,381 total: 47,814 identity not exposed by capture, 7,567 timeout; no free-call recall claim |

Every declaration status and candidate count was independently reconstructed from captured node names, kinds, exact ends, identifier containment, and source-OID eligibility. All 23,197 published and 23,070 current represented IDs are individually unique; the joins do not reuse one symbol for two declarations. All 96,903 represented method/macro site IDs are unique. Every represented method also agrees on the exact method-token range, a stricter independent check than `analyze.py` uses. All 689 file-level counters reproduce and their numerators are bounded by their eligible denominators.

Published declaration presence is 23,197/26,507 = **87.513%** over all parsed declarations, or 23,197/23,573 = **98.405%** within exact-source files. Current presence is 23,070/26,507 = **87.034%**, or 23,070/23,476 = **98.271%** within successfully captured declarations. These have different eligible populations and are not a before/after improvement estimate. The 12,682 `callables` comprise functions, methods and trait methods; that label excludes the nine foreign-function declarations.

The parser-excluded `jute-notebook/src-tauri/src/assets/ports_bootstrap.rs` has notebook directives (`:dep ...`) before its Rust items: 12,254 bytes, 351 lines, OID `a7ee0fad81433612d66e221214c5bfdca81eafa9`. It contributes zero AST denominator rows, despite successful current extraction. **All** 115 extra current method rows and 11 extra macro rows belong to this fragment. Dividing raw capture totals by measured AST totals would yield impossible 72,264/72,149 = **100.159%** and 24,765/24,754 = **100.044%**. The actual correspondence-based counters avoid that error. Zero AST rows here means excluded/unmeasured, not an empty file or complete source coverage.

The published 376 correspondence gaps are 364 tuple fields, nine foreign functions and three extern blocks; current 406 gaps are 394 tuple fields plus the same nine functions and three blocks. Tuple fields and extern blocks have no correspondence rule in `analyze.py`; therefore label them **no supported correspondence**, rather than asserting extractor failure from that join alone. The inspected tuple and FFI samples below also lack corresponding current nodes. The other 52 of 446 tuple fields fall in current timeout files. Closures, async/const blocks and index operations are inventoried separately and are not declarations. Macro-generated items and Cargo cfg evaluation are outside this denominator. Its 689 paths include 30 vendor, 19 research/archive, 175 test, five build-tool and two example files, alongside 458 production/mixed files.

The receiver-start normalization is legitimate for the eight observed expression attributes: only the attribute/whitespace prefix is removed; file, source OID, name and exact call end remain fixed. All eight method-token spans also match. There are zero duplicate `(kind, file, core_start, end, name)` AST keys. Nested repeated calls retain different ends/tokens, as the `get` chain below demonstrates; normalization does not collapse them. Calibration independently agrees for Unicode/raw identifiers, a nested function, trait methods, `s.trim().len()`, and attributed `s.trim()`; the opaque macro's generated function is absent from physical AST declarations.

**Concrete counterexample:** `local.rs` bytes `[7431,7486)` contain `#[cfg(windows)]\n    command.creation_flags(0x0800_0000)` but `cfg_context=false`; the correctly matched normalized call is `[7451,7486)`. All eight normalized rows have this cfg false negative. `workspace_store.rs` bytes `[3194,3240)` contain `#[cfg(test)]\n        self.pause_before_write()` yet both `cfg_context` and `test_context` are false. Do not present those flags as exhaustive cfg/test classification. No top-line count change is indicated. Durable risk audit `568f4355-92c2-4b87-944d-8090ba852681` and signal `de256fb8-7eb8-4631-8068-706e17c454a5` were recorded on **bd-l8h**; detector changes remain root-owned.

Inspected raw byte slices (zero-based, half-open), against their frozen source OIDs:

| Source path | Exact Git blob OID | Slice / graph identity |
| --- | --- | --- |
| `app_gallery/code-graph-app/backend/rust/metrics.rs` | `7e59842702589db4cefbd5bf82b13a309dde5574` | function `[153,565)`, `build_rust_metrics` identifier `[160,178)`, published/current ID `43e0a83f4ff92de7` |
| `src/agent_registry.rs` | `66bb3fd798beece2496611fad46145e811ec6291` | `pub config: AgentConfig` `[3910,3933)`, ID `4c49d01d04105697`; `fmt` method `[3013,3219)`, ID `1368b3e4aeb35bf8`, both published/current |
| `jute-notebook/src-tauri/src/backend/local.rs` | `ea85b828f0e50e15d0d2fce98e584c766d61fa64` | normalized call `[7451,7486)`, method token `[7459,7473)`, site `0c9bbc77c0c99b32e740a86bcbd443f96b66699a70335495d7545a4e4f0696b7` |
| `src/sidebar_chat/workspace_store.rs` | `76866d864d399bdfe10e6469037f1e1cfa90c160` | attributed call `[3194,3240)`, normalized `[3215,3240)`, method token `[3220,3238)`; cfg/test counterexample |
| `src/cell_output.rs` | `b71855d390b32ba418619a587366dbef925916ad` | `output.get("data")?.get(key)`; calls `[645,663)` / `[645,673)`, method tokens `[652,655)` / `[665,668)`, distinct site IDs `936538fdddf94779e96b0bbc5363f4c9ad2259700952fee49c364f1e02439e91` / `7594ea846cd41b02793a090880cfcde92f5f88948e6343ece6b123b4d8637e5d` |
| `docs/reviews/2026-09-29-catalog-detach-fix/catalog-final-revision/catalog_tokens.before.rs` | `3161a0bd9bfbfa23b89920c2f7c3623c6ee5f55c` | unnamed tuple field `String` `[330,336)`, fresh published source; no correspondence |
| `rest-table-gateway-ext/src/generation_probe_tests.rs` | `dd55012af0c4e3e76a44bfadddf0f3c15b8b82df` | `dlopen` declaration `[3692,3750)`, enclosing extern block `[3615,3872)`; no matching node |

Published manifest explicitly says **`spur-graph-schema-v10`**, graph hash `cbc715ac26ad217292466f09775b8eff130b0d49885af4d6817035fecd40bbc1`. Current capture header advertises only `graph_index_version=4`; it does not certify persisted v11 schema completeness. Current extraction used source commit `848a29f39511cbd27091beb6c171b97e179f353a`, binary SHA-256 `984ee7bc02555b9c831addac41ce44d0bf323bf685f011c52078f815363eb5cd`. Its separately captured method/macro rows cannot be attributed to published v10. Neither capture establishes project-wide target resolution, receiver types, runtime reachability, execution coverage, generated-code coverage, or panic completeness.

Verification: read-only `python3 - <<'PY'` checks using `json`, `gzip`, `hashlib`, `collections` and `pyarrow.parquet` exited **0** for source hashes/lengths/lines, 683 capture hashes/OIDs, all 53,014 declaration status reconstructions, all 165,689 callsite statuses, represented symbol/site uniqueness and method tokens, parser accounting and 689 bounded file counters. Preliminary audit-harness failures (a Python Counter construction and an absent timeout `exit` key) were corrected before those successful checks; they were not artifact failures. PyArrow emitted sandbox CPU-probe warnings without affecting reads. No builds, parser reruns, recaptures, target edits, notebook operations, shellfs or solver-store access occurred. `git diff --check` is the report's final whitespace check. Calibration validation still being completed by root is not claimed here.

Root concurrently revised filename classification and added a generated-marker column. I reread the revised analyzer and tables: correspondence logic, coverage states and unique IDs are unchanged; source classes now count 175 test / 458 production-or-mixed files. The versions below bind that final read. Initial fully audited hashes were `d0ee0aee3522a19d48f91216c41037c98507a2d3ff3660e62adb9545281a779b` (`analyze.py`), `9496e4fe42e342b0e342771a44b6b3455b4e89165754e6d12c6142b56ba1f641` (symbol coverage), and `d79652ad045abfe17e4015fefedec09a4a5f5fd67e6c0749098ba796513d62a4` (file coverage). All other listed inputs remained byte-identical on recheck. Generated-marker absence is a heuristic, not proof of handwritten origin.

Input SHA-256 versions read (per-source OIDs above and all 689 verified against `source-manifest.json`; every successful gzip payload verified against `current-extractor-results.json`):

| Artifact | SHA-256 |
| --- | --- |
| `analyze.py` | `f64b8d3174956f38b0747d3fa607f833ade9dbb7af161e10b0bc8ab0b60a02d5` |
| `summary.json` | `5b0a70f81e9c675b3d2b5ee19a314181c39f33113e49a4dc82348a70869b6b60` |
| `source-manifest.json` | `e13dd39f31241d6c40aef5d4de4641d3792022b200a3e91f5d2fd2ad92910c54` |
| `rust-ast.json` | `5829cfe95b8b36aaa0bb86c93be318d2615b19e74a6a008c461d9d0779fbc09b` |
| `rust-parse-results.json` | `51609ea57b6a52c3bdee008cc54777c7484f87176e06e895f5898bcbdb4c3020` |
| `graph-manifest.json` | `635fbf42abace1eb8310b1c65025e7acffa2aec853ca61ce4dda03b848e768ec` |
| `graph-nodes.parquet` | `e80a7d41b48f6ad87451861da0649bebae1ba64e0954b1afc81e859badbb533f` |
| `graph-file_manifests.parquet` | `08a10fef1c5809d4058540aa5e4b55d1cb63d25fd815a25be05e55f69e194e47` |
| `symbol-coverage.parquet` | `794d2c879b99487f5cef4e7a575e3eb8e551bf34b7ec5f46b96045284b92e2ca` |
| `callsite-coverage.parquet` | `597e7cda32fbb8d0a857728cd89b5e257209fb9064907fa467c81b9cac55745e` |
| `file-coverage.parquet` | `bbb293500f8ea90516e1b25a65e822763cb6cb2d5a28136f23aa5e4cc28f1f88` |
| `current-extractor-results.json` | `8591345db8ee311bdcffeecfe9ad3f3cd9d9e33d62163c7a3eca8b2f3344964c` |
| `calibration.rs` | `461ca164771650e8ed81081e698567ad766f947190dbc970b416089c29f77235` |
| `calibration-ast.json` | `76acd2c8eae03c50513e369af95ee6af24bd92c111cc6ffaea3f61bc1dc8905b` |
| `calibration-current-extractor.json` | `344ca21db6017adc78fce555be6f129e2e9936e04b93e6b519a15754bdf5db57` |
