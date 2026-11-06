# D3R ROOT review: two remaining physical-path failures

D3R is not approved. Canonical bd-2dcu is in progress; INTENT3957, RED3959, implementation/native checkpoints3960–3965 and service completion3968 are durable. The worker's later final audit remains unconfirmed as reported. This alone is not the reason for requested changes.

ROOT preserved authored9d9802ca1 at spur/review-evidence/g3-d3r-authored and reviewed normalized310707bfd. Its scoped production diff shares a helper across extraction, discovery and overlay shadowing. The 365 native source/query/test bindings and both native executable hashes matched. ROOT freshly replayed original14 and ROOT8, all passed with no ignores/filters; actual outputs are retained here. Worker remote/local combined47 and local all-target compilation passed; remote all-target attempts failed in transfer/provisioning before compilation and remain unclaimed. No remote compiler/test failure was replaced by a local pass. No cloud configuration was changed.

Two new existing-API tests compiled and failed runtime assertions, exit101, zero passed/two failed/no skips:

1. root/link is a symlink to root/inner/deep. Both root/entry.rs and root/inner/entry.rs exist with different tuple owners/bytes. A relative link/../entry.rs selects root/entry.rs; its absolute equivalent canonicalizes to root/inner/entry.rs. The same existing filesystem request therefore selects different source bytes and graph owners.
2. A symlinked root alias points inside the actual worktree root. Selecting its absolute deleted.rs errors as escaping, instead of producing no active source and canonical shadow deleted.rs.

The testfile is crates/spur-graph/tests/rust_selected_path_semantics_root_review.rs. The complete native GraphIndexArtifact comparison and exact exception are preserved in path-semantics.log.gz, with the wrapper command and successful compilation. The assertions are ordinary API behavior, not new compiler-grade resolution requirements.

The helper currently canonicalizes existing absolute paths before lexical parent normalization, but collapses relative parents before resolving symlinks. Its absolute NotFound branch retains the unresolved spelling, then rejects it before resolving the existing root alias prefix. A correction must make existing relative/absolute spellings honor the same physical path and resolve safe existing ancestor aliases for missing suffixes. Keep the original nonexistent unused/../entry.rs contract, selected-only domain, symbol/tuple/macro retention, duplicates, missing-file shadows, and strict escape/owner/OID validation. Do not suppress either failure by rejecting valid aliases or weakening validation. Production changes by ROOT: zero.

A supplemental prospective PRE in rework-pre binds17 current source/test/log/oracle files. Fingerprint41c662aac9e6743fd38a812961f002bd0bc96e6a0e20737993b9ea31372f0fde. Three rows/five fields use33 lowering variables. Absolute-root strings are symbolic test-pattern markers, not invented runtime filenames. Fixture Git blob OIDs are independently derived from the exact source bytes and checked against the native assertion output.

| Case | Expected/actual | Persisted, reloaded, pinned receipt |
|---|---|---|
| Physical path and missing-alias contract | SAT | sol_7b80e6807c124635 |
| Observed relative wrong source | UNSAT | sol_80532aafae6844f3 |
| Observed missing alias rejection | UNSAT | sol_973a047136864c13 |

Jev opened with exact family/mode/facts/subject/no-unknown bindings inspected. ROOT explicitly added catalog unique/cardinality. The verifier checks nine independent finite predicates and complete requests/lowerings/models. Three remaining receipts from closed D2 POST were compared completely to live archives and independently checked before unpinning; current D3 PREs stay pinned. No solver state or quotas were accessed.

Before further production/test edits, restore this committed reviewer test/packet unchanged and run:

    python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D3/ROOT-final-review/rework-pre/verify.py

Then reload all three complete live envelopes. Keep the packet immutable after edits. Its SAT describes the intended finite contract, not a current runtime pass. The earlier754ec020 PRE was valid before the first repair and is intentionally stale afterward.

Requested rework is confined to the shared normalizer and focused tests/exports, with actual current-code GREEN and fresh ROOT POST/Jev still required. No D3 merge, D4 dispatch or complete panic-detection claim is made. Protected main files, old notebooks/evidence and live graph/CURRENT remain unchanged.
