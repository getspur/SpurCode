# ROOT review: correct the owner oracle before accepting B

Decision: request bounded verification rework. The reviewed B production implementation is preserved. C remains pending; B approval, fresh B POST and C PRE are still required.

## Independent review

Canonical issue `bd-2x4s` was in progress with risk signal `8ffbf4ef-fb2f-41a7-ab69-d64291882852`. ROOT read INTENT 3707, risk 3720/3722, handoff 3729 and completion 3731. Authored `91a587c92` and normalized `b85c6903de516271ef9581424c78f81735baebbe` have the same tree `422db9387d2fed9f2aa48c8ea44d9847a08c4b49`. All 81 changed paths fit the assigned scope.

The unchanged B integrity verifier passed with hashes: 373 selected executions, six explicitly named integration ignores, all 446 old tuple records unchanged, nine adversarial mutations rejected, and the original owner gate still rejected. These are the worker's recorded executions verified by ROOT; no new full suite is claimed. `original-B-integrity.json` retains the exact independent command/result.

## Cause and Rust notebook experiment

The supplied `rest-table-gateway-ext/src/native_registration.rs` bytes have Git blob OID `543d7d4432a0199706c961c67d6f022faad978d8`. Extern block [2865,3363] is inside method `get` [2798,4706] in `impl Api`. The historical syn visitor tracks Item functions/modules/structs/enums, but has no `visit_impl_item_fn` owner stack. Its output therefore incorrectly falls back to the file for this block.

A new notebook ran Rust with syn 2.0.118 over all 136 supplied inputs:

[Notebook](/Users/kevintruong/project/spur-notebook/rust-foreign-owner-root-review-20261106.ipynb)

Cell `cb7644b5-9410-49c2-9ca5-dde7593d4958` succeeded with durable output. MCP readback confirmed the exact source matches the saved `notebook-source.rs`. The legacy visitor reproduces all twelve original foreign owner records. Adding impl-method and trait-default-method tracking changes exactly six lexical rows: the block and its five declarations. Separate impl-method, trait-default-method and free-function source controls produce six expected owner rows. The parser reads supplied source only; it never reads B graph exports.

Only one historical candidate requires an addendum to its explicit structural owner: the block's `owner` and `owner_id`. Its five functions retain their exact block structural owner; their lexical owner changes transitively. The replacement method ID is derived from the independently parsed method range and the preexisting accepted G1 symbol, with its stable-ID inputs checked.

`reconcile.py` preserves the original inventory and rejection, applies that single candidate correction to an additional copy, then runs B's unchanged strict join. All twelve foreign rows pass. `correction-diff.json` records the two changed candidate fields and the prior G1 owner; `corrected-frozen-join.json` contains the actual joined output. No production source changed.

## Fresh amended PRE for rework

Run `python3 -B docs/research/artifacts/2026-11-05-rust-foreign-resume/ROOT-B-owner-review/verify_pre.py` before editing. Reload and compare all five full receipts in `amended-proof.json.gz`. This binds 224 files to fingerprint `213f07c1133728a45c50f81440e9ecc72cfb0e86e94beb750966badc7bfbbd36`.

| Case | Status | Receipt |
| --- | --- | --- |
| Corrected source group 0 | SAT | sol_3f75112f37d4425a |
| Corrected source group 1 | SAT | sol_6b6fac45a7bb4466 |
| Corrected source group 2 | SAT | sol_f8e50d641d314a12 |
| Obsolete file owner group 1 | UNSAT | sol_d39a3c0967df4ce0 |
| Obsolete file owner group 2 | UNSAT | sol_66651c6a9f2f44ed |

All five receipts were persisted, pinned and reloaded. Fifteen predicates, exact lowerings and models passed the independent oracle. Jev's open candidate was inspected; explicit unique/cardinality bindings were added from the current catalog. Original file-owner values remain valid typed-domain values, so negative outcomes express the ownership conflict rather than malformed inputs.

ROOT released retention for four already-closed A POST receipts only after checking all seven archived A receipts/33 predicates and exact live envelopes for the four releases. Complete live envelopes and operations are retained. No solver files or quotas were edited; the eight original B PRE receipts remain untouched.

This amended PRE authorizes an evidence/oracle repair, not a B POST or current acceptance. Keep the original B PRE, independent inventory, first handoff and failed join unchanged. Preserve unsupplied held-out rows and recovered-source Unknown status.

## Required rework

- Recover the reviewed code and complete first-attempt evidence; verify the amended entry packet and canonical assignment, then record/replay INTENT.
- Add an executable oracle regression that rejects the old file-owner inference for a foreign block inside an impl method, and exercise the corrected method visitor. Preserve its actual RED/GREEN outputs. Existing production RED/fix history already establishes the extraction change; do not fabricate a new production failure.
- Add a focused production projection regression for the exact nested-method source shape: graph and byte results must agree on method lexical scope, block direct parent, full spans and stable-ID inputs. Include the preexisting method identity.
- Complete an accepted frozen join through an explicitly versioned addendum or new verifier. Do not silently rewrite the old inventory, synthesize observed nodes from expected rows, relax exact owner checks, or change production to file ownership. Keep all 136 source/zero-row bindings, 446 unchanged tuple rows and same-file/same-ABI wrong-existing-owner adversaries.
- Update only B's scoped tests/evidence unless a newly observed production failure warrants a separately grounded PRE. Retain original raw logs, hash manifests and rejection records as historical evidence; place new attempts separately and distinguish reused verification from fresh runs.
- Return actual source/output-bound POST inputs and current binaries/commands. ROOT will run fresh whole-B POST and prepare C PRE before approval. No main merge, push, notebook-project rebuild or broad ORT work.

`verification.json` proves the source reconciliation and strict join. `amended-verification.json` proves the fresh rework PRE. Neither asserts Cargo cfg activation, expansion, linkage, local foreign execution, compiler-grade dispatch or complete panic detection.
