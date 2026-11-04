# S6 root PRE: analyst migration

Canonical issue `bd-3cu4`, task `embedding-analyst-routing`, plan `b3135858-c6f8-4c51-a360-3f31a0766440`. User routing remains agent/profile codex, model gpt-6.1-terra, effort xhigh. This PRE is prospective, grounded in S5 candidate `c06d12b2489c8f49af1c0be1f3d28a9e1428b64a` and the existing analyst mode/runtime source hashes in `../../S5/final-review/source-sha256.json`.

S5 supplies `EmbeddingModelSelection::model_id` and an owned `EmbeddingModelHandle` returned by `load_embedding_model`; it supports embed and explicit close through the neutral client. It requires caller-declared helper configuration until S7 completes packaging. Analyst still expects the former native loader/type. S6 must migrate that caller/cache/jobs and the external socket service, preserving Auto/Sidecar/Off behavior and socket v1. The existing inprocess selection may be a documented local-helper alias; it cannot execute native inference in the host.

The four OPEN Jev interpretations cover all sixteen prospective route/lifecycle cases in disjoint four-row partitions. The first two larger requests exceeded the catalog compiler's 64-variable limit and were validation rejections, not solver verdicts. Their inputs, interpretations and exact error remain in `proof.json.gz`. The smaller requests preserve every original row and allowed tuple; no gate threshold was lowered and no requirement removed.

Four intended models passed; twelve corruptions were rejected. All sixteen receipts are persisted, reloaded and pinned for worker review. They are requirements, not actual migrated outputs. Full Jev provenance, current catalog, exact family/lowered requests and responses are archived. The S5 root verifier also validates this PRE.

| Case | Result | Receipt |
|---|---|---|
| analyst_routes_part1 | sat | sol_ec0431c40df84852 |
| analyst_routes_part2 | sat | sol_7ce2da62dc204207 |
| analyst_lifecycle_part1 | sat | sol_8eacb51cca954613 |
| analyst_lifecycle_part2 | sat | sol_82ec2d47d6524974 |
| off_starts_child | unsat | sol_a9e10ac422804390 |
| inprocess_native | unsat | sol_82239fcd961e4336 |
| auto_native_fallback | unsat | sol_fb64a2c76a4543f0 |
| serve_native | unsat | sol_34efe60ce96b463b |
| socket_v1_removed | unsat | sol_1e58f883bacf4f67 |
| missing_helper_success | unsat | sol_1a5e2699f683477e |
| error_exit0_success | unsat | sol_7d98fc5d434142af |
| abort_success | unsat | sol_fef7921b62a44c05 |
| wrong_generation_success | unsat | sol_64ee1e1eb4cf40f5 |
| wrong_model_success | unsat | sol_b30099b8b1654c8d |
| cancelled_warmup_success | unsat | sol_2fbcb112e32c46bf |
| detached_job | unsat | sol_1ded908de11d469a |

Implement through committed assertion tests, observed RED, minimal fix and actual GREEN. Worker-local Jev absence is already bridged. Preserve external socket v1 translation, explicit unavailable/error results, correct model/generation validation, owned cancellation/teardown, and no process in off mode. Native FastEmbed/ORT/HF dependencies must disappear from normal/build host closure, including default product embed features. Keep public modes, source formatting and selected model/vector identity stable. Current timeouts and cache behavior are existing source policy; any new bounds must be explicit or grounded rather than copied from experiments.

Root will run a new POST from actual mode/socket/process/dependency outputs before approval. Keep independent expected policy and exact corruptions separate from observations. S7 owns deployment configuration and packaging; S8 owns final CLI/PGQ/notebook acceptance.

The eight legacy in-process wrapper failures were reproduced on the pre-S5 base and remain a separate preserved target. Do not restart that broad ownership audit, conceal failures or restore native fallback to make old callers compile. The graph-isolation review does not claim the old seam is fixed or the whole CLI is already native-free.
