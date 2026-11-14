# DuckDB source ingestion, 2026-11-14

The native extension successfully extracted and persisted the supported source
files in DuckDB commit `8d3673cfd16c231f5e941386dec4c7744e7c21b6`.
Rust notebook cells ran an isolated DuckDB 2.0.0 process; the notebook's existing
SQL runtime was preserved. These are single-run observations on macOS ARM64,
10 logical CPUs and 24 GiB RAM.

| Measurement | Result |
| --- | ---: |
| Tracked entries inventoried | 18,195 |
| Supported source paths | 6,168 |
| Distinct blob/language tasks | 6,129 |
| Distinct input bytes | 81,283,539 |
| Completed tasks with zero symbols | 686 |
| Unique blob symbol rows | 181,228 |
| Path-associated symbol rows | 181,251 |
| Ingestion errors / missing supported paths | 0 / 0 |

Unsupported source formats and data assets remain inventoried. SQLLogicTest
`.test` files are outside the current language registry. Symbol-free inputs
remain completed cache rows. Tree-sitter extraction does not establish semantic
validity of the source.

| Run | Wall time | Parse/materialization phase | Peak process RSS |
| --- | ---: | ---: | ---: |
| Debug, first ingestion | 186.14 s | 166.32 s | 900 MiB |
| Release, fresh extraction keys | 36.56 s | 30.83 s | 1,124 MiB |
| Final guarded release resume, zero reparses | 0.52 s | 0 s | 418 MiB |

Release used the same 15 planned batches, cached source blobs and a warm OS
cache. The caller also changed from individual inserts to bulk SQL staging.
The two full runs therefore do not isolate compiler optimization alone. The
earlier debug resume took 25.14 s with individual task inserts and 2.36 s after
bulk staging. Later warm observations ranged from 0.52 to 1.53 s. No throughput
confidence interval or full temporal speedup is claimed.

The release run still spends about 84% of wall time in parsing/materialization.
Whole-blob AST extraction, symbol anchors and tokens are more work than scanning
commit metadata. The adapter batches its SQL interface and deduplicates blobs;
it does not turn Tree-sitter into a columnar AST parser. History ingestion can
reuse these content-addressed facts, but full temporal matching and history
ingestion have not been implemented by this prototype.

Validation established exact equality of every cached symbol field, nested token
list and empty result across debug/release, plus bidirectional `EXCEPT ALL`
equality of the persisted HEAD view and Parquet export. All symbol byte/line
ranges passed bounds checks. A separate 3,067-blob fixture matched 55,998 symbols
exactly against a direct Rust extractor oracle. Twelve SQL/caller integration
tests pass; an independent review found no remaining blocking defects.

Local evidence:

- Notebook: `/Users/kevintruong/project/spur-notebook/duckdb-temporal-performance.ipynb`
- Dataset: `docs/research/artifacts/2026-11-14-duckdb-extension-ingestion/`
- Extension and source/solver evidence: `docs/research/artifacts/2026-11-14-spur-graph-duckdb-v2/`
- Release artifact SHA-256: `db1cffdecfcee42568bcf37a7391901cb150ae7543241a73393ccef6fcfd1d4b`

The native extension requires exactly DuckDB 2.0.0 with its pinned unstable C
struct ABI. Its ordinary Parquet output was also read successfully by DuckDB
1.4.4. The extension itself correctly rejects that older runtime.
