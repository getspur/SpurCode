# R2C: implicit prelude dispatch uncertainty

Rationale: implicit prelude, blanket, reference and off-file implementations can precede a lexical inherent candidate, so a colliding inherent target requires a supported receiver-order witness.

Execution issue **bd-3nb** was verified `in_progress` before edits and again
before handoff. Plan **04f4a253-7e5d-46f3-99ba-afcd3f016f85**, task R2C.
Original issue bd-382 and plan 670123f8-257e-43ea-bf28-a95c82978883 are lineage.
Root owns review and final notebook/plan integration; R3 remains gated and
R8 owns full regression acceptance. This work does not approve or close them.

## Review changes and preserved history

- Incoming preserved R2 attempt-3 base: **e77f7f418393ab23dd0af2d7cdb35b72a08f27be**.
- Approved R1 baseline: **6520e079b**.
- Failing-test/PRE commit: **1fc4d23af**.
- Resolver fix commit: **eaf03f4c5**.
- This evidence package is committed separately after the fix; obtain its ID
  with `git log -1 --format=%H -- docs/research/artifacts/2026-11-01-panic-implementation/R2C/README.md`.

[r2c-code.diff](r2c-code.diff) is the complete source/test delta against the
incoming preserved base. [cumulative-code.diff](cumulative-code.diff) is the
complete R2 source/test delta against 6520e079b. Both cover the three approved
Rust paths, use unified format with zero context, and pass reverse application
checks with `git apply --reverse --check --unidiff-zero`. Test inputs are in
[cases.json](cases.json), alongside preserved R2 inputs referenced by the tests.
Evidence files are represented by the commits rather than recursively included
in their own diffs.

All **626 tracked R1/R2 historical files** remain byte-identical to the incoming
base; [historical-sha256.json](historical-sha256.json) records those bytes.
No prior commit was amended, replaced or deleted. Earlier authored attempt
commits and their IDs remain recorded in the preserved R2 evidence.

## Implemented policy and primary grounding

Only the private lexical resolver changed in R2C. Physical Calls, exact owner,
nominal scope, receiver text and evidence IDs remain present. Unsupported
dispatch removes the target and the associated certainty/binding metadata;
it never substitutes a guessed trait target. Public GraphFacts and graph
schema remain unchanged. `tree_sitter.rs` has no R2C-only edit.

The [Rust Reference method lookup procedure](https://doc.rust-lang.org/reference/expressions/method-call-expr.html)
orders receiver candidates before looking for inherent and visible trait
methods at each candidate. An inherent method at a later candidate can lose
to a trait method at an earlier candidate. This is the ordering used by the
independent executable controls, rather than an assumption that inherent
methods always win.

The [standard prelude](https://doc.rust-lang.org/std/prelude/index.html) supplies
traits without a physical `use` declaration. [Into](https://doc.rust-lang.org/std/convert/trait.Into.html)
includes identity conversions, [TryInto](https://doc.rust-lang.org/std/convert/trait.TryInto.html)
has blanket conversion implementations, and [Clone](https://doc.rust-lang.org/std/clone/trait.Clone.html)
includes reference implementations. Thus the three root blockers require
neither source imports nor physical trait impls, derives or macro expansions.

[prelude-inventory.json](prelude-inventory.json) records the installed compiler's
trait declarations, documentation hashes and primary compiler source links:
**rustc 1.94.1**, commit **e408947bfd200af42db322daf0fadfe7e26d3bd1**.
It includes **34 traits, 117 receiver APIs and 107 distinct method names**.
The saved compiler re-export sources and hashes in
[prelude-sources.json](prelude-sources.json) ground the actual trait inventory,
including supertrait APIs; marker traits and associated functions contribute
no extra dot receiver methods. Derive macros such as Debug/Hash are not
mistaken for implicitly imported traits.

The policy conservatively uses the union of std/core preludes for editions
**2015, 2018, 2021 and 2024** on that compiler, including documented unstable
receiver APIs. This AST does not establish an edition, std/core configuration
or disabled prelude, so it never uses their presumed absence to authorize a
target. Rust 2021 adds conversion/collection traits, and Rust 2024 adds future
traits; older editions may therefore lose precision under the union.

For an inventoried method, a target remains unresolved unless all of these
ordering facts hold: lexical evidence establishes an owned nominal receiver;
the inherent receiver is exactly `&self`; and every inventoried trait method
with this spelling has a shared or mutable borrowed receiver. At the resulting
`&T` candidate the inherent method precedes trait methods. By-value and explicit
receivers (including `Future::poll`) remain uncertain. The ownership witness
uses the same lexical interval/pattern/constructor logic as nominal lookup,
with reference erasure disabled. An unknown witness never means owned.

This preserves grounded Alpha/Beta/custom methods, including owned custom
`len()` locals and shared inherent `clone()` on owned receivers. Reference
layers, identity conversions, mutable inherent methods and unsupported
receiver forms retain conservative unresolved results. The existing
trait/import/expansion guards remain in force. An out-of-line module declaration
now also blocks inherent dispatch: that unexamined source may implement a
visible local or prelude trait, even without an impl in this AST.

## Exact PRE → RED → fix → GREEN → POST record

All times below are **2026-11-01 UTC**. Exact argv, working directories, starts,
finishes and exit codes are in [commands.json](commands.json). Readable logs
normalize trailing whitespace; [raw-output.json](raw-output.json) retains the
original command output strings.

1. **14:52:51–14:53:49 PRE**, before new project tests or production edits:
   catalog `data_integrity` discovery and the four implemented rules
   `mutually_consistent`, `foreign_key`, `unique`, `cardinality`.
   The contract passed; eight negative controls failed. Every receipt was
   persisted and reloaded. Contract receipt **sol_c7a7ba38a8d44d24**;
   [receipt-index.json](receipt-index.json) lists every exact ID and expectation.
   No generic encoding was needed or used.
2. Independent cases were authored from source/runtime expectations, never
   from GraphFacts. Two mutable-reference control forms initially proved
   invalid under rustc; those diagnostics remain in
   [runtime-controls-initial.json](runtime-controls-initial.json) and
   [runtime.log](runtime.log). Their receiver forms were corrected **before**
   test commit/RED, and **14:58:47–14:59:04** ordinary Rust replay passed all
   **54 controls** across editions 2021/2024. Runtime output values distinguish
   identity/reference trait dispatch (0) from inherent controls (1), including
   renamed, borrowed, double-reference, local-borrow and parenthesized cases.
3. **14:59:18** commit **1fc4d23af** preserves tests and PRE evidence.
   **14:59:24–15:01:17 RED**: the real receiver integration suite returned 101,
   with three individual blocker failures and the aggregate adversary failure.
   All three blockers selected inherent NodeId 4; the aggregate exposes
   **23 false inherent targets**. [red.log](red.log) and `red-exported/` retain
   the observations. The 17 other tests passed.
4. Production edits followed RED. The first GREEN attempt exposed two cursor
   lifetime compile errors; [green.log](green.log) preserves them. The next
   attempt exposed the old custom `len()` local being lost;
   [green-final.log](green-final.log) preserves that real regression. Sharing
   the original lexical binding seam for the ownership witness repaired it.
5. **15:05:02–15:05:17 GREEN**: [green-shipped.log](green-shipped.log), all
   **21 receiver tests** passed. **15:05:33–15:05:50**: all **66 resolver units**
   passed. **15:05:54–15:06:07**: all **65 tests in seven affected integration
   suites** passed, including **15 lexical tests**. The fix was committed as
   **eaf03f4c5** at **15:06:24**.
6. Fresh GraphFacts from those shipped resolver outputs were exported into
   `R2C/exported/`. The projection script initially mishandled omitted optional
   `scope_text` JSON fields; its failure remains in [post-projection.log](post-projection.log).
   After correcting that evidence reader, [project_post.py](project_post.py)
   supplied **99 actual observations: all retained 72 plus 27 new cases**.
   [post-projection.json](post-projection.json) binds each observation to the
   exact source/export SHA-256, function node/span and physical Calls edge.
   [resolver-provenance.json](resolver-provenance.json) pins the shipped resolver
   and test bytes to SHA-256 hashes and git blobs in fix commit eaf03f4c5.
7. **POST**, after GREEN: **20 positive requests pass**, and **11 adversarial
   requests fail**, using the same four catalog families. Additional instances
   of owner uniqueness/coverage check the parent owner relation as well as
   call rows. Controls cover incompatible targets, absent nominal/receiver
   evidence, lost positive targets, missing/duplicate/extra call owners and
   missing/duplicate/extra parent owners. Every result was persisted/reloaded;
   [receipt-index.json](receipt-index.json) lists all **31 POST** and **9 PRE**
   receipt IDs. Per-receipt files retain the original family request, result,
   reloaded solver envelope and completion timestamp. First positive POST
   receipt **sol_cf40989a3f7e4a94**; incompatible control
   **sol_a982d84513e74694**; extra-owner control **sol_2e4692e3acf5498f**.

[verify_evidence.py](verify_evidence.py) independently re-evaluates **222 finite
family predicates**, checks their individual solver statuses, receipt reloads,
99 provenance chains, all 23 target removals and four new positives, old
dispatch/expansion export equality, compiler inventory, chronology and scope.
[verification.json](verification.json) and
[evidence-verification-final.log](evidence-verification-final.log) retain the result.

Compact audit **e3954c8c-9e41-41b9-a717-b3c79ca09473**, comment **1157**, was
written to bd-3nb and idempotently reloaded; [audit.json](audit.json) preserves
both responses. The audit leaves issue/plan/review state unchanged.

## Validation and replay

Every Cargo invocation used **`SPUR_REMOTE=0 scripts/spur-cargo`**. Crate check
and scoped formatting passed. Existing `mcp/mod.rs` dead-code warnings remain
in the logs and are unrelated to these changes. Full regression acceptance
continues to belong to R8.

```sh
SPUR_REMOTE=0 SPUR_R2_EXPORT_DIR="$PWD/docs/research/artifacts/2026-11-01-panic-implementation/R2C/exported" scripts/spur-cargo test -p spur-graph --test rust_receiver_resolution
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R2C/project_post.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-01-panic-implementation/R2C/verify_evidence.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R2C/verify_runtime.py
```

The first command regenerates exports from the resolver, the second rebuilds
the catalog requests from actual facts, the third verifies retained receipts
and independent finite predicates, and the fourth repeats ordinary Rust
controls. The verifier does not invoke Z3 again; fresh solves use the exact
requests in [post-requests.json](post-requests.json) through `solve_rules`,
followed by `get_solve_result`. Never run scripts that write older R1/R2 files.

## Bounds and unresolved obligations

The solver verifies finite supplied relations, presence, owners, uniqueness
and coverage. It does **not** prove arbitrary Rust semantics. Ordinary rustc
replay grounds the concrete dispatch expectations; conservative unresolved
controls can still execute the inherent method in that concrete program.

This is a physical-source subset, not a full trait solver or expansion engine.
It does not certify a complete crate/module/import environment, type aliases,
generic substitutions, arbitrary autoderef, mutable receiver availability,
explicit self types, cfg/expansion output, included-fragment context, external
crate implementations, future compiler/prelude additions or arbitrary Rust
equivalence. Those require compiler/build-context evidence before stronger
dispatch claims. Unknown applicability in the guarded lanes stays unresolved;
the union intentionally sacrifices resolution precision.

Graph orientation failed querying the analyst DB and exact symbol search
reported no worktree graph artifact. Bounded working-tree reads were used for
the approved source/evidence paths; compiler sources/documentation were read
for primary grounding. No notebook was accessed, no graph CURRENT/publication
was changed, and no global install/config, original CGA, unrelated user file
or upstream message was changed. Tracker audits contain compact references;
the detailed payloads stay in this local package.
