# Verified native cache; notebook attachment blocked

`cache.duckdb` is published and verified, SHA256 `ad6f7d3874b4829e9754df2ac7ab7ed0bafc7004d241561a2ad8e52fdd81172b`. See `refresh-summary.json` and the research report. Both extractor versions and all frozen sources are retained. The original snapshot and extension remain unchanged.

`phase-*.json`, `workflow.sql`, `source-manifest.jsonl`, `structural-*.json`, `parquet-manifest.json` and `native-readback.json` are completed native evidence. `refresh-preflight.py` and `refresh-ingest.py` retain exact script versions used by the phase receipts. Timings are one observed ingest/replay, not a comparative engine benchmark.

Notebook MCP returned `Transport closed`. The fresh worker has only read-only notebook capabilities; `worker-notebook-receipts.json` records that block. There are no successful notebook refresh run or attachment receipts. The first two insert outcomes are unverified.

After reconnecting the full Notebook MCP service, open the specified notebook and obtain its context before mutation. Inspect the attempted cells by IDs recorded in the worker receipt. **Do not run `notebook-refresh.rs` after publication**: it is the original unexecuted ingestion proposal, retained for provenance. Instead invoke `inspect_published.py --out <new receipt path>` from a Rust cell, then attach this database read-only under a separate SQL alias, discover its schema and query its current views. Do not hot-reload a second binary over an already loaded `spur_graph` function.

Database and Parquet binaries stay local; text evidence is committed. The cache is frozen-revision data and does not update the live `code_*` index.
