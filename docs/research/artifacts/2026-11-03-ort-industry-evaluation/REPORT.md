# ORT failure containment: industry evaluation with Jev and SOLVE

Date: 2026-11-03. Research and design comparison only. No production change or architecture migration is implemented by this artifact.

Recommendation: evaluate a separately built, supervised embedding helper as the preferred deployment for Spur's confirmed macOS native-library conflict and shutdown abort. Keep DuckDB/DuckPGQ in the host; keep FastEmbed/ORT in the helper. Maintain a small inference-only API and correct native cleanup inside that helper. This is a recommendation derived from the evidence and availability goal, not a universally optimal architecture proven by Z3.

## Grounded problem

The previous native controls established two separate failures: the embedding-enabled static build fails the unchanged DuckPGQ query with `prepare is null`; an official shared-ORT control makes that query pass but a process retaining the native ORT environment aborts during shutdown. Releasing the environment in a native control exits normally. These are inherited measured results, not new runtime tests in this research.

Source baseline examined: `37727b282d4d69d250bcd92a5aa7be4ab387a750`. It pins FastEmbed 5.8.0, ort/ort-sys 2.0.0-rc.10, DuckDB/libduckdb-sys 1.4.4, and the macOS bundled native ORT 1.22.0. The root Cargo patch selects `vendor/ort`; the registry source below is the original upstream contract, not a claim that the modified vendor tree is byte-identical.

The graph pack was used for orientation. Its historical local selector `graph://symbol/73d524b5fe29d405` identifies `shared_embed_model`. Candidate source was then read directly from the bound Git commit because its lifetime changes and vendored runtime work are newer than the graph evidence.

The seam is: `load_embedding_model` calls `ort_runtime::ensure_ort_runtime` on macOS, then creates a FastEmbed model. The runtime function calls `ort::init_from(path).commit()`. Both model caches use process-global cells in the examined source. The original registry contract `pkg:ort@2.0.0-rc.10::init_from` initializes the global dynamic-library path and returns an EnvironmentBuilder; `pkg:ort@2.0.0-rc.10::G_ENV` is `OnceLock<Environment>`, and `pkg:ort@2.0.0-rc.10::get_environment` returns `&'static Environment`. This explains why a narrow, explicit lifetime boundary matters; it does not replace the ongoing review of patched ownership.

Historical controls are documented in [G2 linkage](../2026-11-02-rust-gap-implementation/G2-linkage/README.md) and [shared-library control](../2026-11-02-rust-gap-implementation/G2-linkage/attempt-2/README.md). The lifecycle investigation is retained at the source baseline under `docs/research/artifacts/2026-11-02-rust-gap-implementation/G2-lifecycle/README.md`, readable with `git show` at that commit. These incomplete-stage statements must not be read as current approval.

## Industry guidance and applicability

| Primary source | Documented practice | Application to Spur |
|---|---|---|
| [Rust catch_unwind](https://doc.rust-lang.org/stable/std/panic/fn.catch_unwind.html) | Captures unwinding Rust panics, not process aborts; foreign exception catching is not a reliable recovery contract. | Use Result for ordinary API failures. Do not use catch_unwind as the native-abort boundary. |
| [Electron process model](https://www.electronjs.org/docs/latest/tutorial/process-model#the-utility-process) | Utility processes can host CPU-intensive and crash-prone components. | A separate process is a documented production pattern for protecting an interactive host. This is an architectural analogy, not a recommendation to add Electron to Spur. |
| [ONNX Runtime 1.22.0 C++ wrapper](https://github.com/microsoft/onnxruntime/blob/v1.22.0/include/onnxruntime/core/session/onnxruntime_cxx_api.h) | RAII releases owned native objects; borrowed objects must not outlive owners. | Release dependent resources in a controlled scope and honor native API contracts even inside a helper. Isolation does not repair an unsafe destructor. |
| [Rust Unix process status](https://doc.rust-lang.org/std/os/unix/process/trait.ExitStatusExt.html) | Reports terminating signals separately from normal exit codes. | Observe SIGABRT through process status. Shell code 134 alone is not a portable crash classifier. |
| [Amazon Builders' Library](https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/) | Timeouts also apply to calls across processes on one machine; retries need bounds and backoff. | Put deadlines, cancellation, bounded queues and restart budgets at the embedding protocol boundary. Choose numeric values from measured workloads. |
| [AWS retry guidance](https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/retry-backoff.html) | Retries should be idempotent; deterministic failures should fail promptly. | Retry transient worker failures only within a budget. A deterministic shutdown abort is not made correct by restarting forever. |

A correctly supported in-process ORT deployment remains a normal industry option. It can have lower IPC and packaging overhead, and should be considered if measurement justifies it and lifecycle/linkage acceptance passes. It offers no separate process boundary against a future native abort.

## Option comparison

| Option | Native abort containment | Separation of the conflicting native libraries | Judgment |
|---|---|---|---|
| catch_unwind or signal suppression in the host | No reliable continuation | No | Reject as the repair. |
| Fully validated in-process shared ORT with explicit ownership | Host can still terminate on native abort | Shared-ORT control removes the observed static RTTI conflict, subject to final acceptance | Viable when a shared process is acceptable. Current full acceptance remains incomplete. |
| A thread or a helper subcommand of the same ORT-linked executable | A thread shares the abort boundary; a real child contains execution failures | The main executable can still contain the original ORT linkage | Insufficient for the combined requirement unless the host dependency boundary is changed. |
| Separate supervised embedding binary; host neither links nor loads ORT | Contains helper process termination at the address-space boundary | Yes by construction, once artifact inspection verifies it | Preferred candidate for this availability goal. Costs include IPC, memory, packaging and supervision. |

A new helper process must not inherit an already initialized ORT runtime through a fork-only design. Start the separately built executable through the platform process-launch API. The literal word `fork` in an unrelated API name does not establish whether an executable is launched.

## Concrete target contract

- The host retains graph extraction, DuckDB/DuckPGQ, graph persistence, notebook interaction and bounded panic analysis. The helper receives bounded embedding batches and returns typed vectors.
- The host artifact and its dependency tree must be checked: neither FastEmbed/ORT static objects nor runtime ORT loading may remain in the host path. Merely passing `--no-*-embeddings` cannot establish this.
- Use a versioned framed protocol with request IDs, input/model/tokenizer identities, expected row count and vector dimension. Validate finite numeric outputs before graph-side publication. Keep graph database writes in the host.
- Start with a reusable helper and a bounded queue rather than reloading the model for each small batch. Model reuse and batch sizing remain hypotheses to benchmark, not supplied performance results.
- Ordinary failures become typed errors. Worker exits, malformed responses and deadlines fail the affected pending request. Restart only within a measured/configured budget, and make replay idempotent.
- A one-shot helper needs a valid complete response and normal successful exit for that invocation. For a reusable helper, a validated and committed response can complete its request before the helper's eventual exit; a later crash must not erase already committed results or certify shutdown as successful.
- Shutdown stops new work, resolves/drains pending work, releases sessions/values/allocators and other native owners in contract order, then releases the environment. Use a bounded shutdown wait; an exceeded deadline is a shutdown failure, with OS termination used only for containment.
- Keep the inference surface small. The earlier 48-identity ORT audit is useful evidence; a broad wrapper fork supporting editor, training, custom operators and every native API should not automatically become a prerequisite for this embedding-only process boundary.
- Evaluate a supported upstream fix or upgrade with the same pinned reproducer before committing to long-term maintenance of a wrapper fork. No untested upgrade is claimed to fix the issue here.

## Jev and SOLVE execution

Jev 1.14.0 opened the interpretation gate with weakest confidence 0.98 and selected `configuration.attribute_allowed_pair` in verify mode. The returned family, mode, subjects, domains, allowed tuples, facts and empty unknown set were inspected and matched the supplied input. The only execution additions were persistence and deliberately changed facts in named controls.

The Jev-derived compatibility request checks three failure classes (returned API error, unwinding Rust panic, native abort) against three mechanisms (Result handler, catch_unwind, supervised process). The allowed table is an explicit policy projection supported by the documentation, not something Z3 discovered.

| Case | Receipt | Expected and actual |
|---|---|---|
| Native abort / supervised process | `sol_7598125b0bb048e5` | SAT / pass |
| Native abort / catch_unwind | `sol_c59c5f786ed841f0` | UNSAT / fail |
| Rust unwind / catch_unwind | `sol_4b786b48350b489a` | SAT / pass |
| Returned error / Result | `sol_e65341f2ffe24957` | SAT / pass |

A second direct catalog model assembled ten `configuration.requires_any` bindings, discovered from the catalog, for embedding availability, process containment, native-library separation, native quiescence/release and six supervision obligations. It supplements the single Jev rule; Jev did not independently compile the entire multi-rule design.

| Prospective design scenario | Receipt | Expected and actual |
|---|---|---|
| Isolated helper with all declared obligations | `sol_e5e18e0275d54fb2` | SAT / pass |
| Hypothetical audited in-process deployment, containment/separation goals disabled | `sol_332c51ffdca940ba` | SAT / pass |
| In-process catch with host-containment goal enabled | `sol_e4d510b9cb494119` | UNSAT / fail |
| Helper while main still loads ORT | `sol_0a68f1f5ee9f48a3` | UNSAT / fail |
| Helper without declared cleanup contract | `sol_748848bf9c3942e5` | UNSAT / fail |
| Helper with unbounded restart | `sol_30f3aa47af454791` | UNSAT / fail |
| Helper with unchecked response | `sol_8da826a2692a4b2c` | UNSAT / fail |

All eleven receipts were persisted and reloaded through get_solve_result; IDs, statuses and nonempty compiled requests were checked. The current persistence surface returns the lowered generic query, so evidence.json separately preserves the exact catalog inputs as well as the returned lowered query. No solver-store files were read.

There are five positive passes and six expected policy rejections. No optimization weights, arbitrary timeout values or invented memory/latency measurements were supplied. These are finite policy-consistency checks over declared facts; they do not prove native memory safety, production containment, API completeness, performance or global architectural optimality.

## Acceptance experiment before migration

1. Build a minimal separate embedding helper exposing inference only, and a host artifact demonstrably free of ORT. Preserve both original failure reproducers.
2. Run the unchanged five-row DuckPGQ regression with default product embedding capability available through the helper; disabling product embeddings is not equivalent acceptance.
3. Verify real model outputs and repeated normal startup/inference/shutdown. A child exit 134 is recorded as a failure even if it printed plausible output.
4. Inject native abort before a reply, mid-frame, during inference and during shutdown. Check host graph/notebook continuity, typed failures, bounded restart and no uncommitted graph-side publication.
5. Test malformed/wrong-request/wrong-model/wrong-dimension/nonfinite responses, child hangs, queue pressure, parent cancellation and retained result ownership.
6. Measure cold/warm latency, peak resident memory, batch throughput, cancellation and packaging cost against a supported in-process baseline. Choose capacities/timeouts from those measurements.
7. Only then select or revise the implementation plan. This research does not approve the incomplete native repair or abandon its protected work.

No new ORT runtime tests or notebook execution were performed for this industry comparison. It reuses the existing native controls, adds fresh Jev/catalog evidence and defines the concrete next experiment.
