# code_* versus extension SQL retrieval, 2026-11-15

The current extension reproduces source content and the tested symbol searches well. Its file-local facts do not yet reproduce repository caller retrieval. Keep native `code_*` as the compatibility reference for interactive graph retrieval; use DuckDB for analysis and experiments over materialized facts. This experiment does not authorize a production backend cutover.

The Rust notebook ran **87 requests** against the actual production `GraphMcpModule` handlers and an independent SQL adapter over `spur_graph_file_facts`. It found **61 strict semantic matches**, **4 exact full API body/error matches**, and **15/15 successful source reads with identical UTF-8 bytes, ranges and file OIDs**. These are counts for a bounded request matrix, including errors, empty results and repeated requests; they are not repository-wide precision/recall estimates.

| Operation | Synthetic semantic matches | Real-source semantic matches | Combined |
|---|---:|---:|---:|
| `code_symbol_search` | 16/18 | 6/6 | 22/24 |
| `code_read_symbol` | 12/13 | 4/7 | 16/20 |
| `code_callers` | 4/12 | 2/8 | 6/20 |
| `code_callees` | 11/15 | 6/8 | 17/23 |
| Total | 43/58 | 18/29 | 61/87 |

The 26 semantic mismatches comprise 12 graph binding/count/unresolved cases, 11 error-shape or error-message cases, and three missing `context_lines` fields. All mismatches are retained. Matching error replies or empty traversals do not demonstrate successful symbol binding.

**What the differences mean.** For the real `register` function, native callers with unresolved edges enabled returns four rows, including repeated calls; SQL returns none. Native counts report four calls and four unresolved edges even with unresolved rows hidden. For fully qualified `impl BytesExtractor::extract_file_facts`, native finds one unresolved inbound caller from `ParseFileFacts::exec`; SQL returns none. The candidate deliberately does not infer inbound edges from unresolved labels or add a repository resolver. A qualified label in an extension fact is evidence, not a resolved cross-file edge.

All six successful real source snippets match bytes, ranges and file OIDs. Three context-2 replies omit the API's `context_lines: 2` field; these remain semantic failures. The other three context-0 source replies match the full native API body exactly. The synthetic `read_source_format` reply is the fourth exact full API match.

Native freshness/provenance fields are absent from most SQL replies. The comparison therefore reports full API equality separately from content equality. Raw identity-bearing replies match in 43/52 cases, but a missing/reordered row can also cause an identity-list difference. The remaining nine cases must not be characterized as nine ID-algorithm failures. Extension textual `stable_key` values already reproduce many native symbol IDs; extension numeric IDs remain local to each extracted file.

**The fairness boundary.** Each backend receives the same complete immutable source corpus, including all of its resolution context. Native uses `build_facts(root, None)` and published graph artifacts, then actual `GraphMcpModule::dispatch` under `with_worktree_root_for_request`. SQL independently extracts every shared source with the approved extension, expands all five typed collections into tables, and executes search/read/caller/callee queries. It receives no native answers, node/edge tables, ID maps or resolutions. Parent-only validation reads native provenance after candidate execution; it never supplies facts to SQL.

Independent review found native discovery could skip a supplied ignored/`target` source while SQL extracts it. The parent added a required two-way path/blob equality gate for these runs. It checks the complete spec/source manifests, native indexed manifest, actual `files.parquet` path set, hashes of every native artifact, and SQL complete manifests/counts. Both six-file cohorts pass before and after comparison. A skipped-source fault is rejected. The standalone example command requires this parent gate for arbitrary custom corpora; native production discovery policy was preserved.

Search results, ambiguity lists, unresolved samples, limits and truncation remain ordered. Traversal rows are compared as multisets preserving duplicates, with observed order retained separately. Semantic identity uses independently obtained path, kind, qualified name and line range; source-format replies lack kind and use their visible path/name/range identity. Only explicitly named graph metadata and ID representation are separated. Missing/null/type differences, source whitespace, edge kinds, binding/confidence, counts and every other API field remain observable. Full API comparison preserves original IDs, metadata and ordering. Fields exposed only in raw extension facts have no complete `code_*` counterpart and are not claimed as validated against that API.

**Corpora and provenance.** The synthetic corpus has Cargo.toml and five complete Rust files: 35 symbols, 59 edges, Unicode/CRLF, ambiguity, scoped duplicates, imports, local/cross-file calls and negative controls. Its 58 requests are not 58 independent scenarios: two caller request pairs repeat identical selectors. Real-engine traversal coverage is calls/production paths; dynamic/reference edge kinds, tests/benches and ID/URI request round-trips remain unvalidated. Comparator fault tests exercise sensitivity to additional fields; that does not establish engine support for them.

The real corpus has six complete source files from `b5e1246cc9706f9938bcff6f6a4233d8a6c12199`, totaling 444,678 bytes, with 1,138 native symbols and 5,946 edges. Paths are `crates/spur-graph-duckdb/src/{lib,file_facts}.rs` and `crates/spur-graph/src/{extract/tree_sitter,extract/mod,extract/languages,schema}.rs`. This is a bounded isolated snapshot, not the full workspace or the DuckDB project's temporal history. Its 29 requests include the original 25 plus four fully qualified method traversal requests added before candidate evaluation. The original four incomplete method selectors remain as negative cases; no difficult cases were removed. Native source/graph bytes and Git HEAD are identical between the initial and extended request sets. Forty-one real positive/negative controls passed, including nonempty local traversal, qualified unresolved traversal, exact source/context ranges and five expected errors. Synthetic ambiguity controls passed separately.

| Artifact | Frozen identity |
|---|---|
| Native implementation commit | `e2114ff6bef2c33075940c48a90dc523add5eb1c` |
| SQL comparator commit | `7bfed9391736f60de5934941f36901ef4bc0841f` |
| Native optimized release binary SHA-256 | `f897e4d526038e3df5203d741e7f0513af8900f8de6908cce4c86e800e8bcfaa` |
| Approved extension build commit | `f0218c673ed478f854b64df6223d1817ced3586e` |
| Extension SHA-256 | `e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028` |
| SQL comparator SHA-256 | `3d7f35655c60636fe101f1c147a6d834cbb95fc13bfc1ba0952c93e92c884fc7` |
| Synthetic spec SHA-256 | `6a82c682c4b595c55135527dbc58811c28a725d058eb6f1b1bd68a84fcf2b836` |
| Real extended spec SHA-256 | `6e6717b2425c97536d1e0951fd12144b2fab80d7c17e0db25512daa3e8473ddc` |
| Synthetic native graph hash | `e82b98f3fc121631ddcf333bcac5eaa61b179696f78049d36b388eeba80b6457` |
| Real native graph hash | `b2925314053137ce53ac8aebe3ca208cf57fc9d23034137962ba1d959b5f03da` |

The extension and oracle use unchanged production MCP/extraction Rust source: `git diff f0218c673..7bfed9391 -- crates/spur-graph/src crates/spur-graph-duckdb/src` is empty. The oracle was built optimized on macOS ARM64 using `SPUR_REMOTE=0 scripts/spur-cargo ... --release --locked`. All 58 declared reference replies exactly match the previous debug oracle, excluding timing. Two additional malformed-input/recovery smoke lines are preserved outside the 58-case shared spec. Python 3.14.5 loads native DuckDB 2.0.0; the application's 1.4.4 and managed notebook's 1.5.5 engines were not upgraded.

**Warm latency diagnostics.** Only the four positive source reads with exact full body/error equality were timed. Each has 30 paired observations, alternating native/SQL order, after both persistent local JSONL processes warmed the full matrix. Client timing includes JSON encode, pipes and decode; server dispatch timing is separate. Every measured pair must still reproduce the original complete body/error. All 120 pairs passed. Construction, materialization, startup and warmup are excluded; native cold first-query time is unavailable.

| Exact positive source request | Native client p50 / p95 ms | SQL client p50 / p95 ms |
|---|---:|---:|
| Synthetic `read_source_format` | 107.595 / 130.592 | 3.232 / 5.640 |
| Real `source_offset`, context 0 | 88.138 / 136.334 | 2.899 / 4.051 |
| Real `register`, context 0 | 89.578 / 122.988 | 3.079 / 3.880 |
| Real `extract_file_facts`, context 0 | 90.791 / 148.796 | 2.934 / 4.539 |

This measures these frozen-snapshot retrieval pipelines. Native still performs production freshness/metadata work; SQL serves prevalidated materialized facts and emits additional diagnostic envelopes/query logs. API payloads match for these requests, but freshness guarantees, implementation language and internal work differ. These numbers do not isolate DuckDB engine throughput, explain the native cost, establish changed-worktree correctness, or justify graph/full-history speed claims. A separate profile is needed to attribute native latency before optimizing it. p50 is the median and p95 is nearest rank `ceil(0.95*n)`; raw observations, response sizes and host load are retained.

**Verification and artifacts.** All 70 combined regression/real-engine tests passed without skips in 35.801 seconds, including 20 retrieval-comparator tests and the existing 50 tests. Remote native checks and two native unit tests passed; an independent reviewer reran 14 comparator fault tests and reproduced the synthetic score from retained replies. Comparator faults cover omitted/extra/duplicate rows, changed bytes/types/nulls/ranges/kinds/evidence/counts/ranking and unsupported fields. They test the comparator rather than forcing result parity.

Notebook: `/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`, cells ending `067`–`074` plus analysis cell `a50c6b8d-1b23-49f4-a22c-3932e920fbc1`. Native corpus preparation executions 131/132, comparison execution 133, controls/timing execution 134. The experiment's cells succeeded durably; an unrelated older failed cell remains in the notebook and is not part of this result.

Evidence is under [artifacts/2026-11-15-duckdb-retrieval-parity](artifacts/2026-11-15-duckdb-retrieval-parity): `summary.json`, both `comparison-*-r0/{report,provenance}.json` and full `native.jsonl`/`candidate.jsonl`, corpus equality receipts, `real-positive-negative-controls.json`, test logs, notebook receipts and `evidence-manifest.json`. Small reviewable records are committed. Executables, nested corpus Git repositories, SQL databases, Parquet, raw fact dumps and larger query traces remain local with hashes rather than being committed.

Reproduction uses `examples/retrieval_native.rs`, `examples/retrieval_parity.py`, the committed synthetic fixture, and the retained `freeze_real_corpus.py` / `extend_real_requests.py`. Use fresh output directories. Run `validate_corpus.py --corpus ... --out ...` before `retrieval_parity.py compare`, then rerun the guard with `--comparison ...` before interpreting results. `time_matched_reads.py` explicitly gates eligibility; `analyze_results.py` recomputes every comparison from raw replies. These scripts live next to the receipts and expect the approved runtime/executable paths recorded above.

Next work should preserve the frozen failing cases while adding repository binding and inbound unresolved-label behavior, followed by API context/error/provenance compatibility and wider edge/context/ID selector coverage. Batch parsing alone does not implement those retrieval semantics. The extension is useful now for analysis over explicit fact tables and exact source experiments; parity evidence does not support replacing interactive native graph retrieval yet.
