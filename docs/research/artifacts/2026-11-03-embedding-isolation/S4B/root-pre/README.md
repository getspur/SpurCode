# S4B root PRE and concrete handoff

Canonical issue bd-1ydx; task embedding-nomic-inference. Use this ID and actual new delegation for audits. Old bd-2dhg is cancelled historical context. Root performs Jev inline; no worker-local Jev tool is required.

This PRE is grounded at reviewed candidate ae0da00331472c6a7b715197dd8c87d14bcaa719. Root independently reran S4A's15 scoped tests and captured the normal-build real descriptor plus11 identity/error controls. S4A does not compute product vectors yet.

Two fresh Jev gates OPEN, inspected and executed through catalog verification:
- Resource cache/byte witness SAT sol_8a2d65cfffc04b58.
- Post-Ready cache/reuse/close lifecycle SAT sol_2302dcd25d1d4ee0.

Six negatives reject two entries in one slot, artifact budget one byte low, environment release before cache drop, Closed before checked release, reload instead of reuse, and abnormal exit. All eight complete requests/results/reloads are in pre-proof.json.gz and pinned for this worker review. Prospective only: cache_entries is a Rust cache entry count, not numeric native-owner counts or RSS; real inference and native close require your GREEN and root POST.

## Implement S4B now

Preserve the approved S4A metadata behavior and original tests. First reproduce product_nomic_matches_registered_reference_and_closes RED on current Unavailable, then implement the real normal worker. Returning only another RED/reference run does not complete this production task.

The current ResolvedArtifacts contains files: Vec<(PathBuf, File)>, total_bytes, profile and identity. Handles are rewound after hashing, but cache contents remain mutable. Extend model_artifacts.rs as part of S4B to consume bounded bytes from those handles, revalidate the exact profile/file identity, and freeze the exact bytes passed into the model/tokenizer constructor. Role names/order come from the declared FILES table, not canonical blob basenames. Atomic path replacement and same-inode content mutation must not let unverified bytes enter a session. Budget/checked conversion errors reject before native load. Do not hash a path once then let a registered loader silently resolve a different main revision.

The exact locked FastEmbed5.8.0 source is indexed and verified. The existing registered reference calls TextEmbedding::try_new with NomicEmbedTextV15 and max_length8192. Its Mean pooling, output key/quantization and normalization must stay compatible. The public try_new_from_user_defined accepts UserDefinedEmbeddingModel and InitOptionsUserDefined, commits model.onnx_file from memory and loads TokenizerFiles from bytes. This can consume verified owned bytes; preserve the registered model's actual pooling/quantization/output key, not just a guessed subset. Relevant exact selectors are archived in S4A/final-review/seam-proof.json.gz, including:
- pkg:fastembed@5.8.0::impl TextEmbedding::try_new
- pkg:fastembed@5.8.0::impl TextEmbedding::get_default_pooling_method
- pkg:fastembed@5.8.0::impl TextEmbedding::try_new_from_user_defined
- pkg:fastembed@5.8.0::impl TextEmbedding::embed
- pkg:fastembed@5.8.0::transformer_with_precedence
- pkg:fastembed@5.8.0::normalize.

Create one finite droppable ModelCache (Option<TextEmbedding> is sufficient), load once per pinned identity, and reuse for a second batch in the same generation. Normal macOS FastEmbed features must use the existing pinned load-dynamic/managed environment; client remains native-free and worker has no graph/analyst/DuckDB/PGQ dependency. Root supplied one cache slot and actual548026095-byte artifact budget as explicit test witnesses; no new shipped defaults or RSS claim.

Preserve already formatted query/document strings exactly; never add a second prefix. The inherited real reference test uses two public already-formatted inputs, finite768-element rows and its declared tolerance. Keep actual row/dimension/finite-value checks in the wire/session. Native input/output intermediates and cached sessions drop before checked RuntimeOwner::close. Shutdown publishes Closed only after successful checked release; EOF/error paths clear cache too. No forced successful exit, forgotten owner or native abort swallowing.

Use the real client and actual product Nomic model. Demonstrate two batches with one actual model-load event, same generation, and two independent normal worker lifecycles with raw exit0. Same PID alone is insufficient reuse evidence. Keep test-only instrumentation out of normal product packages. Other models remain explicit Unavailable until S4C, preserving their IDs and compatibility work.

Scope explicitly includes worker src/model_artifacts.rs in addition to src/models.rs, src/main.rs and src/runtime.rs, worker Cargo.toml/Cargo.lock, necessary native/model tests and a test-support-only independent reference probe. Model_artifacts changes are required by the verified-byte load seam; do not stop for a scope question about this named file. No graph/analyst/client production migration in S4B.

The supplied PRE is already executed. Run verify.py and reload the8 receipts through get_solve_result, comparing its lowered request/status to the archive. Native owner counts were not measured in S4A; do not invent zeros. Complete production code and real GREEN, then supply actual vector/reference/cache/wait/source/artifact exports, independent expected policy and exact POST candidates to root. Leave your issue in_progress as GREEN/root Jev POST review pending.
