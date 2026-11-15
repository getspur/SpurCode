# Approved SpurCode cache refresh

The user approved loading the corrected extension in a fresh DuckDB 2.0 process and refreshing cached facts after bd-38yz. Track this operational experiment in bd-1i5s. Use extension SHA256 `be0548ce3a1217cf0b50110ad02419223c895daa8d2a932d7a81747da4c38892` from source `4d32f19d4`.

Re-ingest the same frozen SpurCode revision `61ba3d200439a541dce4c1617ca9277be7d438af`: 4,404 inventory records, 4,098 admitted sources, including 1,726 Rust files. Preserve all original bytes, admission decisions, the baseline database and extension, earlier evidence and existing user edits. The original database SHA256 is `c5506839a23ef728e4bec81d44e70083ea671709d38caa3d3ba7d89ddb8bc11e`.

Create a separate persistent cache containing original and corrected extractor versions. Seed original facts from the authenticated frozen snapshot; run the corrected artifact on all admitted sources through the existing typed SQL workflow. Preserve five independent child collections, ordinals and snapshot lineage. This is a file-snapshot refresh, not a full temporal-history ingestion.

Acceptance: exact input/corpus equality and source-byte hashes; new versioned keys; zero new extraction/child refresh on replay; typed children exactly equal cached native facts; bounded structural audit and Rust before/after metrics; recorded Vec false binding absent in refreshed facts, with positive controls retained. Publish only after verification and attach the completed database read-only for notebook SQL. Fresh native processes perform extraction; do not imply the already loaded managed SQL function was hot-replaced. Treat local-target rates as diagnostics, not precision or compiler expansion.
