# Root review of S6A attempt 2

Decision: request changes; no approval or POST success is claimed.

Canonical execution issue: bd-3jqy. Source tracking issue: bd-48r6.
Plan: 805bc9eb-cc70-4d4c-8784-f35776e70a5d, task s6-owned-runtime.
Reviewed delegation: dabde37662ce4d7e.
Protected authored tip: b5e839e893ba091108c4248422ec6c4ebbb0aa5d,
branch spur/review-evidence/embedding-s6a-attempt2.
Source ancestry starts at c778e3ce230370db2634cce4f78eb7ee1aa7a8fb.
Preserve every authored test/fix/evidence commit; engine wrapper b35ba574 is not a substitute for that history.

## Mode clarification

Root's earlier unconditional warm wording was too broad. Local warm inference is required when the selected backend is local: InProcess, or Auto after the external sidecar is found unavailable. Off, Sidecar and reachable Auto must not launch a local helper. Skipping local warm is valid in these modes; document that it does not mean a local model was warmed. No new requirement to warm the external sidecar is introduced.

The existing owned PRE's generic warm row is scoped to a selected local backend. The new mode-aware PRE supplements it rather than weakening cancellation, inference, or cleanup requirements.

## Findings from exact source

1. Local result publication is not synchronized with shutdown. In owned_runtime.rs lines 299-306, infer completes before oneshot send, and rx.await returns without a final admission/cancellation check. The state check at lines 413-430 occurs before both boundaries. A queued reply can be consumed after shutdown has won closure. Add deterministic tests for closure between infer and send, and between send and caller delivery. Define publication at the public operation's completion gate, serialized with closure. Results already committed before closure remain valid.
2. External sidecar completion has the same unresolved boundary: lines 188-192 return Some(result) without checking closure at a shared publication gate. Biased select alone does not serialize shutdown against a future becoming ready during polling. Add a deterministic interleaving test and the smallest correction.
3. Owner close already gathers errors through model.close and CleanupOwner.wait_for_cleanup (lines 465-477), but no owner-level reap-report-failure test proves first and repeated shutdown preserve the error. Use the existing test-only supervisor fault facility through the actual owned runtime cleanup path. A clearly labeled injected error is sufficient; do not induce rare OS failures, interfere with unrelated children, fake native release, or prefill the owner's error.
4. Workspace formatting failed in two inherited files, in_process.rs and tests/embedding/dependency_boundary.rs. Formatting-only corrections there are authorized. Preserve the prior failure log.

This is source-review evidence. These race interleavings have not yet been reproduced by root runtime tests and must be established with failing assertions before fixes. Existing subprocess evidence remains useful but does not cover these narrow delivery boundaries.

## Worker evidence reviewed

The committed logs report nine owned-runtime subprocess tests, 39 client tests, two graph cancellation tests and a no-default analyst library check passing. Root has inspected the source and evidence; these are worker observations, not a fresh independent root rerun. The whole S6 analyst migration remains unfinished, as do B host wiring and C real-helper acceptance.

## Supplemental executed PRE

mode-pre.json.gz preserves the exact Jev input, OPEN interpretation and provenance, three catalog cards, eight complete family requests, solver responses, and pinned reloaded lowered requests. Jev selected mutually_consistent; root added catalog uniqueness and exact five-row cardinality. Each compiled request has 55 variables, within the 64-variable bound.

One SAT: sol_fb61115ea12e47a1.
Seven UNSAT controls: sol_affec400f4094b5d, sol_c504a47db00a46e6,
sol_264ab622294b4cb0, sol_150a170d819d495c, sol_944d55b1ae0f4cbf,
sol_9ab5cc4a21fe4530, sol_3ae50e01f5374815.

These are declared routing scenarios. The solver does not evaluate network reachability, execute inference, prove race freedom or establish complete S6 acceptance. verify.py independently evaluates the finite tuples, uniqueness and coverage, checks receipt/model equality, and binds the reviewed source hashes.

## Retry boundary

Continue only S6A: preserve prior history and add focused TDD for publication and owner cleanup; test mode behavior; fix inherited formatting. Small test-only barriers or failpoint forwarding are allowed, compiled out of production. Do not do core/CLI host wiring, packaging, broad ownership research or native lifecycle redesign. B and C remain dependent on root approval; the original packaging task bd-3pcp remains blocked on bd-1ktd. Supply actual observations for root Jev POST.
