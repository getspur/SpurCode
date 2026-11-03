# S4 concrete root handoff

Canonical issue **bd-2dhg**, task embedding-worker-models. Do not audit source stub bd-1p7l. Verify the actual dispatch and in_progress state; branch UUIDs are not delegation IDs. Preserve codex/profile codex/gpt-6.1-terra/xhigh.

The approved S3 candidate is `65189c90c146bc6bc3d972f5b88b076b729ef19c`. Root evidence commit `5a54022204146883e9a9c93d7d27383be4d593fb` independently reran 6 native and 32 client tests. Five POST Jev gates opened, one native-table mode gate remained blocked and was preserved with separate explicit catalog verification; all 6 catalog positives and 16 controls were reloaded. Read S3/final-review/README.md and preserve its narrow scope. Four numeric native owner counters are unmeasured.

Import only the S3/final-review/ and S4/root-pre/ evidence directories from the root evidence commit named in your task. The source/base SHA bindings are in grounding.json. A later composed HEAD can differ; compare the bound source files to candidate651, not the whole commit ID.

Root completed fresh Jev PRE before your RED. Read README.md, verify.py, grounding.json and pre-proof.json.gz here. Three gates OPEN and inspected; 3 catalog passes +10 expected rejects, all persisted/reloaded. Reload via get_solve_result; do not read .spur/solver. These are prospective finite policies, not completed S4 inference.

Implement S4 now with test-first RED -> minimal implementation -> real GREEN. Root Jev POST stays mandatory after actual measured exports. Worker-local Jev absence has already been bridged; do not stop just for it.

## Compatibility and identity

Grounded loader source is crates/spur-graph/src/store/lance_sections.rs SHA256 `f688b253ffd1667cf279b0ec154fdbc754dfe1eb5e129ebb5a5c7d9ec763f10b`.

- All three supported models have 768 dimensions and max_length8192.
- Nomic and Jina use the pinned existing FastEmbed registered behavior. Resolve their actual pooling, normalization and tokenizer behavior from that dependency before asserting them.
- CodeRank uses repo jamie8johnson/CodeRankEmbed-onnx, revision151669b173750250e611e1be00c812f112ea6020, onnx/model.onnx, Mean pooling, max_length8192, hf-hub from_env/progress/retries3 and all four existing tokenizer/config files.
- Preserve HOME/.spur/cache/fastembed, then XDG_CACHE_HOME/spur/fastembed fallback when HOME is absent. Tests/materialization use explicit isolated paths and may read existing verified cached assets without altering them.
- Existing graph callers ALREADY format embedding text. Preserve the exact preformatted bytes in worker Embed; never prepend again. Nomic query is "search_query: " + query, document is "search_document: title: " + trimmed title (empty becomes none) + " | text: " + body. CodeRank query uses "Represent this query for searching relevant code: "; CodeRank documents and Jina are raw. Identity describes this legacy profile. Compare product results against an independent reference using exactly these same input bytes.
- Hash all actual model/external-weight/tokenizer/config files used plus inference configuration, including max_length, pooling, normalization and input-format profile. Do not substitute a model name or expected label for an artifact fingerprint. Pin the actual identity in Hello and every batch.

## Concrete identity bootstrap decision

The strict S1 Hello needs artifact identity before launch; only the separate native helper may load/resolve native model assets. Add a bounded metadata/discovery mode to the helper executable (e.g. --describe-model) accepting a requested model, explicit caller limits and correlation IDs. It resolves/hashes actual files and returns an existing framed Reply identity, then exits normally. stdout remains framing only. No new public wire variants or fake placeholder Hello identity are needed. Normal embedding Hello remains strict; mismatched actual identity fails. This is within S4 scope. S5 will expose the neutral host integration.

Use deadlines/size limits for descriptor execution in tests; do not leave a hung discovery process. Metadata/config code can be shared inside the helper. If an exercised source constraint makes this design impossible, report the exact conflict, complete unrelated TDD, and ask root for a concrete adjustment. Do not silently accept unverified caller hashes.

## Cache and native ownership

A worker process pins one identity: a single finite droppable model entry is sufficient. PRE's one entry/one slot is this explicit witness, not a hidden product default. Reuse the loaded model for a second batch in the same generation; record real cache/load observations. No immortal OnceLock<TextEmbedding>. New byte budgets must be caller-declared, catalog checked, and tested against actual artifact sizes before materialization. Artifact byte totals and the legacy approximate MB display estimates do not prove a native RSS ceiling. State this limit honestly; product defaults/process memory policy are completed in S7/S8.

Stop admission, join actual work, drop cached TextEmbedding sessions and retained outputs, then call the real checked S3 environment release and publish Closed only after success. Native abort/timeout remains an abnormal child result. No abort swallowing, _exit, success process::exit, mem::forget, mocked C API frees or broad wrapper-audit restart.

## Real model acceptance and scope

Nomic assets are present at:
`/Users/kevintruong/.spur/cache/fastembed/models--nomic-ai--nomic-embed-text-v1.5/snapshots/e9b6763023c676ca8431644204f50c2b100d9aab`.
grounding.json records exact five-file SHA256/byte lengths; ONNX SHA256 is `147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965`, size547310275. This is real product data, not the tiny release-control model.

Run real Nomic inference through the actual worker and existing EmbeddingClient, compare vectors with an independent existing-loader reference, verify row count/dimensions/finiteness/normalization, prefix consistency, actual identity mismatch rejection, same generation/cache reuse and repeated normal load/infer/close exits. Keep the tiny ONNX fixture only as the native lifecycle control. Other models must preserve implementations and metadata contracts. Investigate/download needed missing assets as authorized, using isolated paths; record exact missing files and attempted commands if unavailable. Do not call skipped real-model acceptance GREEN.

Normal helper dependencies must exclude spur-graph/spur-analyst/DuckDB/PGQ; neutral client remains native-free. Preserve pinned ORT build/materialization behavior and the current managed inference-only wrapper. No host migration or global runtime edits yet.

Expanded planned writes authorize Cargo.lock; worker Cargo.toml; worker src/{main,models,runtime}.rs; tests/embedding_models.rs, tests/native_lifecycle.rs, tests/support/* and test fixtures; S4 evidence; plus ONLY the S3/S4 root evidence imports. S3 scaffold tests expecting arbitrary fake identities/Unavailable may be updated to real-identity behavior; never retain a production bypass for them. Keep genuine tiny native probe and cleanup regressions. No S1 protocol/client production edits or graph/analyst source changes in S4.

Use SPUR_REMOTE=0 TMPDIR=/private/tmp scripts/spur-cargo, one writer /private/tmp/rust-gap-g1-acceptance-target; CARGO_NET_OFFLINE=true can reuse Rust packages while model acquisition remains separately explicit. Pinned SPUR_ORT_ARCHIVE remains the approved S3 path/checksum; do not overwrite shared binaries while another build is active.

At GREEN give root actual framed responses, artifact/config/source/binary hashes, independent reference outputs/tolerances, cache observations, raw native wait statuses and exact POST intent/data plus negative controls. Do not infer numeric native owner zeros. Commit test then fix/evidence, durable completion audit on bd-2dhg; leave in_progress for root review.

