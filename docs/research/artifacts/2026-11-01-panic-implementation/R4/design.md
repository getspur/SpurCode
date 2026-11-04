# R4 contract boundary

Assigned issue bd-1f8; plan 04f4a253-7e5d-46f3-99ba-afcd3f016f85. R2C and R3 approved before edits. Baseline 4417055a4e48278a3d9a58b717cad4f32d789a91.

Implement a separately versioned JSON contract without modifying graph v11. The canonical R3 source/body/callsite set is revalidated, then exact copies bind the semantic facts. Explicit build configuration has no serde defaults; its hash binds facts and primitive authority. An independent expected build comparison is required because a self-contained document cannot authenticate a fully rehashed replacement build.

Bool and u8 domains, expressions, value versions with mandatory origins, guarded assignments and phi predecessor inputs carry source/body spans and ordered control-flow references. Flow predecessors form a bounded DAG (loops are unrolled by a future producer); guard expressions must be boolean, present and available on incoming paths. Definitions have deterministic identity ordering; control flow is ordered by body then explicit evaluation order. Unknown obligations cover opaque, mutation, cfg, parse, callee and exploration limits; completeness is a structural gate for the declared scope, not certification of arbitrary Rust semantics. R5 owns canonical collection and R6 owns interpretation.

PRE uses catalog data_integrity unique, foreign_key, conditional_required, mutually_consistent exact tuple and value_range. Requiredness is independent of NULLS DISTINCT/MATCH SIMPLE semantics. Positive and adversarial requests and reloads are local. POST will project actual test-serialized facts and invalid variants, not these manual PRE fixtures.

Scope: semantic_facts.rs, mod.rs exports, panic_semantic_contract.rs, this evidence directory only. No notebook, graph publication, external producer, global config or protected user file edits. Build attempts use SPUR_REMOTE=0 scripts/spur-cargo. Cold build interrupted (130) to reuse the existing R3 scratch target due to 3.6 GiB free disk; it is not a RED assertion result.
