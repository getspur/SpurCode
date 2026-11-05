# D3 ROOT review: request changes

D3 is not approved. ROOT reviewed canonical bd97a0e84891064653bdec1b4f97007cee0c0510; its tree exactly equals the preserved authored 4d71ec26f tree. Beads canonical bd-mq2h was in progress with persisted/replayed INTENT, RED and handoff, completion audit 3917, and no unresolved worker signal labels. The original test/fix/evidence history is preserved in spur/review-evidence/g3-d3-authored.

The production diff is within the assigned four seams. Its current native executable and all 364 source/query/manifest bindings matched. ROOT freshly ran the original suite: 14 passed, zero failed/ignored/filtered (the packet entry returns when no input is supplied, as documented by the worker). Five actual source packets were regenerated and freshly validated, exactly matching the worker values. The recorded remote graph all-target check passed against stable source bytes; the earlier network/upload failures remain explicit. No full-workspace success is inferred.

ROOT then added existing-public-API regressions using selected paths that discovery accepts. All five compiled and failed runtime assertions (exit 101, 0 passed, 5 failed, no skips/filters):

- selected parent component, tuple: build_facts_for_paths extracts inner/../entry.rs; selected discovery uses entry.rs, drops the original owner, and returns "tuple owner absent or ambiguous in graph".
- overlay parent component, tuple: the same owner error through extract_delta.
- absolute internal symlink, tuple: extraction retains alias.rs while discovery canonicalizes to entry.rs, producing the same owner error.
- selected parent component, named function: the builder succeeds but silently drops the named function. Canonical call collection can use optional definition references, so its success is not proof of graph symbol preservation.
- overlay parent component, unit struct: its artifact manifest uses entry.rs but shadowed contains inner/../entry.rs. The returned shadow key does not identify the emitted source.

The first three-assertion RED and expanded five-assertion RED are both retained. The test is crates/spur-graph/tests/rust_evidence_build_root_review.rs. The exact wrapper command, environment, successful compilation and runtime errors are in path-regressions.log/json.

The causal mismatch is at source normalization: build_facts_for_paths/language_groups_for_paths receives original spellings; the new discover_selected_entries normalizes/canonicalizes later, and extract_delta constructs shadow keys with a separate legacy helper. Worker tests passed normalized paths to the selected builder with already full, canonically spelled facts, so they did not exercise this integration boundary.

Rework authorization: narrowly share safe normalization/deduplication across selected extraction, discovery/composition and overlay shadow identities. The already assigned build/query/test seams may be extended to extract/tree_sitter.rs and extract/mod.rs (or one crate-local path helper) only for this causal repair. Preserve byte/OID authentication, exact selected domains, zero/Unknown lanes, canonical owner/ID semantics, missing-path behavior, symlink escape rejection and existing tests. Do not relax missing-owner validation or invent placeholder graph owners. No incremental transport, persistence/query/cache or cfg evaluation work is authorized here.

A fresh prospective supplemental PRE is in rework-pre/. It binds 15 current source/test/log/oracle files and the exact five failing cases. The five-row/five-field model has 55 lowering variables, within the current catalog bound. A symbolic absolute_internal_alias identifies the test pattern, not a fabricated absolute runtime filename. The intended canonical path/owner/shadow model is SAT; observed owner loss, divergent source identity and wrong shadow cases are UNSAT. These receipts check the finite prospective contract; current production still fails the five tests.

Fingerprint: 754ec020484f7a6e7882f69281d369e3873bd3c9711585e5419529f5b32d264a.

The first staged diff check detected extra blank lines at EOF in new reviewer documentation/scripts and the raw suite log. The exact original files/input fingerprint and check output are retained in pre-formatting.json.gz; the raw suite output is byte-preserved in root-suite.raw.log.gz. Removing script-only blank lines changes the source fingerprint from 4b8238d1 to 754ec020, while every executed solver request remains identical. The complete request/lowering/source verifier passed again against the final files; no production or test behavior changed.

| Case | Status | Persisted/reloaded/pinned receipt |
|---|---|---|
| Shared normalized contract | SAT | sol_bd5d68407f2a42ce |
| Observed owner loss | UNSAT | sol_b492b507b0a14d48 |
| Divergent source identity | UNSAT | sol_7c4480791c664259 |
| Observed shadow mismatch | UNSAT | sol_7cd4a03b7d8d4106 |

Jev opened; exact family/mode/facts/subjects/no-unknown bindings were inspected. It proposes mutually_consistent only; ROOT adds catalog unique/cardinality explicitly. The verifier checks 12 independent predicates, exact requests, complete lowerings/models and source hashes. Four already closed D2 POST envelopes were matched to archives and independently verified before releasing their pins; all eight current D3 original PRE receipts remain pinned. No local solver state or quota files were accessed.

Before any rework production edit, restore this ROOT addition packet/test unchanged, then run:

    python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/ROOT-review/rework-pre/verify.py

Reload and compare all four current complete envelopes with proof.json.gz. Preserve these pre-edit inputs after fixing code; they intentionally become stale. Produce fresh actual full/selected/overlay outputs and GREEN for the five ROOT failures and original required regressions. ROOT's actual-output POST/Jev and independent review are still required before approval. There is no current POST success claim.

The user’s five protected main files, old notebooks/evidence, external notebook, native model/runtime and live graph/CURRENT were unchanged.
