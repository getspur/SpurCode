# Rust graph gap closure: grounded proposal

The measured declaration gaps can be closed, but the first change should repair ownership before adding nodes. The production extractor currently gives the named fields in `enum E { A { x:u8 }, B { x:u8 } }` the qualified names **`E::x`, `E::x`**. This is an identity-quality defect beyond the 458 missing-node count.

A source-bound experimental sidecar recovered **all 458 missing declarations**: 446 tuple fields, nine foreign functions and three extern blocks. All matched the previous gap inventory by exact path, kind and byte range, acquired a parent identity, and survived Parquet round-trip. **No production nodes or tables were changed.** Adding equivalent records to the graph could close the measured 26,049/26,507 physical declaration correspondence gap; it would not establish complete Rust semantics or panic detection.

## What ran

[Executed notebook](../../../../2026-11-02-rust-gap-closure-grounding.ipynb) · [Rendered report](report.html) · [Structural candidates](captures/structural-candidates.json) · [Solver verification](solver/verification.json)

Inputs are pinned to graph `fcf40b9cd5785e09a0dbf3e45167f6e5f599348a44a1e184c7ff78d3e8e5cfec`, schema v11, manifest `588b0718b0ca5800de3507f603cb809ed90bfa932c63f04865902ef417c4e8c0`. All **689 Rust source hashes** still match the independently inventoried corpus. The AST baseline excludes one directive-bearing bootstrap fragment; this remains a physical source inventory, not a Cargo-selected program.

| Experiment | Observed result | Design consequence |
| --- | --- | --- |
| Native syn 2.0.118 structural probe | Exactly 446 tuple fields, 9 foreign functions, 3 extern blocks; 136 source files; 11 held-out records | Owner-aware capture and explicit provenance can account for every measured gap. |
| Repeated types in one tuple owner | Three real groups contain six equal-typed fields; type-only IDs collide | Use an explicit physical ordinal and real parent; type text is not a field identity. |
| Conditional tuple field, native Rust | Physical field 2 becomes compiled projection `.1` | Keep physical ordinal separate from compiler-selected ordinal. |
| Existing R3 public call collector | Six sites, five bodies, two source bindings including a zero-site file | Reuse the current collector. Persist sources, bodies and sites together. |
| Repeated `leaf()` | Four sites across function/closure/async bodies; one coarse call edge | Summary edges cannot reconstruct invocation counts or deferred-body ownership. |
| Existing canonical validator | Accepts Parquet round-trip; rejects omission, duplicate, stale OID, wrong existing body and missing zero-site source | Existence/FK checks are insufficient; retain canonical correspondence checks. |
| Existing detector, `u8 + 1` | SAT at 255 with overflow checks on; bounded UNSAT for 0..254 or checks off | Bind each result to its exact domain and effective build. |
| Existing replay command | Persisted detector witness reproduces “attempt to add with overflow” | Candidate and native reproduction are separate evidence stages. |
| Macro-generated bounds check | Compiler MIR contains generated `hidden` and an implicit bounds assertion; 1,2,3 panic | The physical graph has only `generated` and `entry`; source detection remains incomplete. |
| `cfg(feature="trip")` | Native panic at 1 only with feature on; source collector returns cfg unknown in both configurations | Supplying a build declaration does not yet make the collector evaluate cfg. |
| Enum ownership regression | Both named payload fields have `E::x` | Fix the two-pass owner seam before generating additional field identities. |

Five native configurations exercised **524 inputs**, with **five panic cases**. A separate tuple-layout control and the detector's own witness replay also passed. These fixtures test the proposed boundaries; they are not a whole-application spur-notebook scan or new production vulnerability claims.

## SOLVE evidence and its limits

Catalog rules were discovered first. Eighteen `solve_rules` checks used uniqueness, foreign keys, cardinality, exact allowed tuples, and attribute compatibility. **Seven valid contracts passed; eleven invalid contracts were rejected.** In catalog verify mode, SAT means the contract passes and UNSAT means rejection.

Seven generic queries were schema-checked before execution. Five were exported from the actual detector, rather than independently reimplemented. Two manually normalized the generated bounds fixture: `0 <= i <= 3 && i >= 1` gives **i=1**; adding `i<1` is UNSAT. All four native inputs verify that small model. The latter is an explicit manual normalization, not automatic macro translation.

Together with the production detector's persisted witness receipt, **26 receipts were reloaded and matched**, comprising ten SAT and sixteen UNSAT results. [Exact requests](solver/catalog-requests.json), [catalog results](solver/catalog-results.json), [generic requests](solver/detector-generic-requests.json), [reloaded receipts](solver/reloaded-receipts.json), and [normalization assumptions](solver/normalization.json) are retained.

Two useful counterexamples guide the design:

- A site assigned to a **different existing body** passes a foreign-key-only rule. Exact source/site/body correspondence rejects it.
- Both the macro-generated case and the deferred-call case return raw **UNSAT** while the detector correctly reports **`incomplete_semantics`**. The reporting contract rejects promoting those results to bounded safety, cfg unknown to safety, source coverage to global panic freedom, and function-pointer address flow to a call observation.

The initial finite model exceeded the backend's 64-variable limit, and one attribute domain had duplicate labels. Those were validation errors, not UNSAT evidence. The corrected model projects exact site/body/source tuples into 63 variables; byte ranges are checked by the production canonical validator and the source joins. [Initial validation errors](solver/initial-validation-errors.json) remain recorded.

## Recommended implementation order

Each task should run PRE on grounded source exports, commit a genuinely failing regression, implement the smallest change, run GREEN and POST on actual reloaded output, and retain adversarial cases. This is a proposal; no implementation plan has been dispatched.

| Task | Concrete change | Required acceptance evidence |
| --- | --- | --- |
| **G1 — repair ownership, then tuple fields** | Assemble Rust definition candidates before assigning owners/IDs in both extraction paths. Preserve variant ownership for named and unnamed fields. Reuse `Field`, record physical ordinal, type/full spans and attributes. | `E::A::x` and `E::B::x`; `E::T::0/1`; all 446 source rows match individually; graph and byte extractors agree. Repeated/nested types, Unicode, cfg fields and insert/reorder edits retain honest identity semantics. |
| **G2 — foreign declarations and extern structure** | Reuse `Function` with explicit foreign/body-absent provenance. Add a non-callable extern container with a versioned representation; keep it transparent to Rust lexical names. Persist ABI explicitness, qualifiers, attributes/link spelling and exact source ownership. | Nine declarations plus three actual block nodes; distinguish bodyful `extern "C" fn`, required trait methods and `external://` placeholders. `m::imported` lookup never gains a synthetic extern path segment. No fabricated safe/empty body or linked implementation. |
| **G3 — persist existing invocation/body evidence** | Add versioned source-manifest, body and callsite tables around R3, including explicit availability. Wire full builds, unchanged-file reuse, deltas/deletions, overlays, hashes, checksums, query clients and caches. | Six fixture sites/five bodies survive memory→Parquet→overlay. Zero-site source remains represented; legacy absence means unavailable. Replacing a file removes all old sites/bodies. Changed extractor/provenance invalidates reused evidence. |
| **G4 — configured compiler evidence** | Capture each effective rustc invocation, sources/generated inputs, package/target/edition, cfg/features, compiler and ordered flags. Join expanded/implicit/trait observations with provenance; retain unresolved observations. | Feature on/off and overflow on/off remain different builds on identical source. Generated `hidden`/bounds operation receives compiler evidence. Traits, casts, drop glue and expansion lineage retain their categories. |
| **G5 — bounded analysis and witness replay** | Feed supported R4–R8 typed facts, versions, origins and guards from validated evidence. Publish candidate, reproduced, complete-bounded, incomplete, timeout and unknown outcomes distinctly. | Reproduce 255; reject the same receipt after source/build changes; retain raw UNSAT plus incomplete status for opaque calls/expansions. Narrow domains only with explicit assumptions or proven caller guards. |

G1 precedes G2's owner/namespace work. G3 can use the existing supported R3 lane independently, but **current R3 rejects cfg-bearing sources**: persist an unavailable/unknown status rather than counting those files as zero sites. Separating raw physical syntax inventory from configured semantic eligibility needs an explicit future contract, not a relaxed validator.

G4 and G5 can build on G3's evidence envelope. Existing R1/R2 receiver/self-call work and R3–R8 APIs should be reused, rather than replaced by another detector.

Preserve the existing node identity contract where feasible: name new fields by ordinal under the correct parent and retain explicit ordinal metadata. The experimental parent-plus-ordinal SHA-256 key demonstrates uniqueness; it is **not** a proposed silent replacement for production stable IDs. Correcting old ownership can change existing IDs and requires deliberate temporal/cache migration.

New extern kind/provenance and new persisted tables require a coordinated schema/capability decision across writer, readers and CLI. Bump extractor compatibility for code-only ownership changes too, so unchanged-file reuse cannot keep stale omissions. Adding a table only to the writer is insufficient.

## What remains compiler-dependent

Foreign declarations have no local body; this follows the [Rust Reference's external-block contract](https://doc.rust-lang.org/reference/items/external-blocks.html). A node does not prove linkage, FFI soundness or unwind behavior.

Effective cfg is fixed per compilation, as described in the [Rust Reference](https://doc.rust-lang.org/reference/conditional-compilation.html). MIR is useful for generated/implicit operations, but its human-readable format is unstable; a production importer needs a pinned producer contract, not the textual checks used by this experiment. See the [compiler guide](https://rustc-dev-guide.rust-lang.org/mir/index.html).

No new CGA capture was performed here. Existing pinned CGA/compiler-validation infrastructure remains necessary for traits, expansions and implicit checks; the previously isolated CGA virtual-call/drop-glue ICE still prevents treating historical partial exports as a complete CLI graph. The current experiment independently used native rustc/MIR and runtime controls.

The practical target is **complete physical declaration correspondence plus auditable bounded panic findings**, with unsupported regions visible. Complete project panic freedom remains unestablished.

## Reproduction and workflow

The notebook contains native Rust and Python cells. A small adapter links the already compiled spur-graph rlib and calls the production R3 collector/validator; it does not implement a substitute collector or modify production code. [Lineage](collector-lineage.json) binds the adapter, rlib and detector binary. [Native captures](captures/) record exact arguments, outputs and hashes. [Structural audit](audits/structure.md) and [root call-evidence audit](audits/call-evidence.md) identify the code seams.

A transport disconnect interrupted final verification. Root reconnected through the official MCP proxy to the existing writer app's confirmed socket, without overriding writer leases; final verification then succeeded in the original notebook. The optional second worker audit was cancelled after two dispatch-lease expiries; the root completed that audit and the experiments directly. The approved structural worker report is preserved unchanged.

No spur-notebook source write, graph rebuild command, CURRENT selection change or production Rust implementation was performed by this research. During observation, live temporal shard metadata and embedding-sidecar row counts changed. The 689 source hashes, target HEAD/status/CURRENT and the Rust symbol projection still match. Both manifest versions and the changed fields are retained; no conclusion here depends on the temporal or embedding sidecars.

Run saved-evidence verification with `python3 verify.py`; optionally pass `--live-root /path/to/spur-notebook` to recheck all 689 source hashes. It needs PyArrow and the sibling frozen new-build-check artifacts. It checks saved receipts and native observations rather than re-running Z3/CGA or claiming whole-project panic freedom.
