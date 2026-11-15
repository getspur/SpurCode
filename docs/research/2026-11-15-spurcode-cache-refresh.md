# Versioned SpurCode cache refresh — bd-1i5s

The corrected DuckDB 2.0 extension has re-ingested all **4,098 admitted frozen sources**, including **1,726 Rust files**. The separate [published cache](artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s/cache.duckdb) retains both extractor versions as **8,196 keys**. Current `facts`, `nodes`, `edges`, `spans` and macro views select the corrected version; `file_facts_*` tables retain both versions.

**Notebook attachment is blocked.** The parent Notebook MCP connection returned `Transport closed` before any refresh execution. Native work continued in fresh pinned DuckDB processes. A fresh-client worker (`bd-3uj7`) exposes only read-only notebook tools, with no open/context-pack, insert or run capabilities. Its [capability evidence](artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s/worker-notebook-receipts.json) confirms no notebook execution or attachment. Initial insert outcomes are unverified; no run was requested. Reconnect the full Notebook MCP connection to attach this already verified cache; do not re-ingest or run the unexecuted original ingestion proposal after publication.

## Identical-input comparison

| Recorded Rust calls | Original extractor | Corrected extractor |
|---|---:|---:|
| Total | 259,828 | 257,163 |
| Local targets | 32,414 | 32,253 |
| Unresolved | 227,414 | 224,910 |
| Local-target rate | 12.4752% | 12.5419% |

The higher percentage is **not a precision estimate**. Total calls decrease by 2,665 and local targets decrease by 161, so 2,504 fewer unresolved records does not mean 2,504 calls were newly resolved. The known real `Vec::new()` case remains recorded with scope `Vec` and NULL target; genuine local constructor/helper controls pass. The remaining 224,910 unresolved Rust calls and compiler-expansion limits remain.

All 4,404 inventory rows are preserved, including 305 admission rejects and one symlink. Every admitted source matches its frozen UTF-8 bytes, SHA256 and Git blob identity. All four non-edge collections match the original facts on **all 4,098 files**; only 952 Rust edge lists differ. Both versions' five typed child collections exactly match the raw cached lists in both directions, including ordinals. Both versions have 4,098 `ok` parents and no extraction-error/null-input parents.

The existing read-only structural audit executes 43 SQL receipts per database; all 21 structural invariants have zero affected rows on both baseline and refreshed data. Its unchanged `expected_ingestion_extension_sha256` field refers to the legacy baseline contract and explicitly does not authenticate ingestion. Corrected provenance is pinned separately by `refresh_meta`, the actual loaded artifact and the refresh/readback receipts.

## Replay and observed costs

The unchanged `crates/spur-graph-duckdb/examples/file_facts.sql` stages new keys, extracts once, expands five collections separately, maintains the key checkpoint and exports seven typed Parquet files. Original facts were seeded from the authenticated frozen snapshot; only the new version was parsed.

One ingest took **92.70 seconds**, including **82.69 seconds** in the native extraction/materialization statement. One fresh-process replay took **2.25 seconds**, including exports, with **zero missing keys, zero extractions, zero child refreshes and zero removals**. The six child/checkpoint table identities were unchanged within the replay connection. These are observations from this run with DuckDB defaults (10 threads, 19.1 GiB memory limit), not a native-versus-extension benchmark or a full temporal-history result.

All seven Parquet exports have matching row counts and complete column/type schemas. Full typed/raw value equality is verified in the database. A further fresh process loads the corrected binary, attaches the published database read-only and reproduces all five cached collections for the real source control. The published file is unchanged by readback.

## Identity and evidence

The input snapshot is revision `61ba3d200439a541dce4c1617ca9277be7d438af`, database SHA256 `c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e`. Original extractor SHA256 is `e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028`; corrected artifact SHA256 is `be0548ce3a1217cf0b50110ad02419223c895daa8d2a932d7a81747da4c38892`, built from native fix `4d32f19d4`.

The published cache SHA256 is **`ad6f7d3874b4829e9754df2ac7ab7ed0bafc7004d241561a2ad8e52fdd81172b`**. [Refresh summary](artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s/refresh-summary.json) links runtime, process, lineage and verification results. The artifact directory retains the exact SQL workflow, source manifest, phase receipts, statement timings, two structural audits, Parquet hashes/schemas and fresh native readback. Original snapshot, extension, earlier evidence and five existing user-edited files remain unchanged.

The native fallback commands and process IDs are recorded in `native-execution-summary.json`; exact script versions for every phase are archived. `inspect_published.py --out <new receipt path>` loads the corrected extension in another fresh DuckDB 2.0 process and verifies the published cache read-only. Notebook integration can invoke that inspection from a Rust cell and then attach `cache.duckdb` read-only for ordinary SQL. The cache does not replace the live `code_*` workspace index.

IDs are file-local. Join current `nodes`/`edges` by logical path and ID; joins over both cached versions must include the full versioned key as well. The cache is a file-snapshot refresh, not a cross-file resolver or temporal-history ingest.
