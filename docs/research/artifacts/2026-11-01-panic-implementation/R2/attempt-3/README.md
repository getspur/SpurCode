# R2 attempt 3: expansion uncertainty blocks inherent dispatch

Rationale: a source-only nominal receiver must not select an inherent method when an unexpanded derive, attribute, or macro can supply a competing trait implementation.

Assigned execution issue **bd-382** was verified `in_progress` before edits.
The issue remains in progress for root review; R3 stays pending and R8 owns
complete regression acceptance. Public GraphFacts and graph schema v11 remain
compatible. No snapshots changed.

## Implementation and history

- `35f8862b2`: preserve the orchestrator's staged attempt-two overlay and both
  prior attempts' local evidence before making this rework. The original
  attempt branches and their authored commits remain unchanged and available:
  attempt one ended at `e42495ee5`, attempt two at `285a7cf50`.
- `08dc3b09d`: commit fresh PRE evidence and the independent regression fixtures
  before executing runtime RED.
- `816f81f9d`: add the conservative expansion guard after runtime RED; includes
  scoped formatting of the new tests.

[unified.diff](unified.diff) is the attempt-three source/test delta against
`35f8862b2`; [combined.diff](combined.diff) is the complete R2 source/test delta
against R1 baseline `6520e079b`. Both use zero context; apply with
`git apply --unidiff-zero`. The reverse patch check passes against shipped code.

The existing `trait_dispatch_uncertain` scan now rejects unresolved macro
invocations at any syntax position, including item, statement, nested block,
and expression positions. It also rejects attributes outside the exact known
non-generating lint/doc set: `allow`, `warn`, `deny`, `forbid`, `expect`, `doc`.
Derives, procedural attributes, qualified attributes, inner attributes, and
`cfg_attr` therefore cannot justify inherent dispatch. An unused macro
definition is skipped, and its token trees are never treated as ordinary Rust.
Safe lint/doc argument tokens are skipped as well. No trait target is guessed.

This deliberately conservative guard applies across the physical source tree,
including expansions unrelated to the method name or receiver. Physical Calls,
receiver text and nominal scope remain exported even when the target becomes
unresolved. The source-only lane cannot establish expansion effects; compiler
confirmation remains a separate obligation.

## RED, GREEN and ordinary Rust controls

Every Cargo command used **`SPUR_REMOTE=0 scripts/spur-cargo`**.
[commands.json](commands.json) retains exact argv, working directory, and exit
codes. [raw-output.json](raw-output.json) retains original log bytes as JSON
strings; readable `.log` copies strip trailing spaces and surplus final blank
lines so evidence does not create whitespace failures.

Runtime RED, from test commit `08dc3b09d` before the production edit:

```sh
SPUR_REMOTE=0 SPUR_R2_EXPORT_DIR="$PWD/docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-3/red-exported" scripts/spur-cargo test -p spur-graph --test rust_receiver_resolution
```

[red.log](red.log): exit **101**, **13 passed / 3 failed**. Both independent
root reproductions select the wrong inherent `NodeId(4)`; the aggregate test
reports all fifteen additional mismatches. This is runtime assertion RED,
not a compile error. The 21 hand-authored cases include seventeen uncertainty
cases and four positive controls. Generated graphs are evidence only and
never become test inputs.

After the fix and scoped formatting:

```sh
SPUR_REMOTE=0 SPUR_R2_EXPORT_DIR="$PWD/docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-3/exported" scripts/spur-cargo test -p spur-graph --test rust_receiver_resolution --test rust_lexical_calls --test calls_range_resolution_edges --test constructs_edges --test rust_macro_token_tree_query --test rust_macro_invocations --test panic_method_sites
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --lib extract::tree_sitter::tests
SPUR_REMOTE=0 scripts/spur-cargo check -p spur-graph
SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -- --check --config skip_children=true crates/spur-graph/src/extract/rust_bindings.rs crates/spur-graph/src/extract/tree_sitter.rs crates/spur-graph/tests/rust_receiver_resolution.rs
```

- [green.log](green.log): **16 receiver tests pass** before formatting.
- [green-regression.log](green-regression.log): **60 integration tests pass**
  after formatting, including all 16 receiver tests and all 15 lexical tests.
- [resolver-unit.log](resolver-unit.log): **66 resolver unit tests pass**.
- Crate check, scoped formatting check, source/evidence diff check, and reverse
  patch check pass. Existing unused-function/method warnings remain in logs.

The original Alpha, borrowed Beta, custom len/unwrap, eleven physical Option
sites, constructor, shadowing, namespace, unsupported-type, and ambiguous
receiver controls remain covered. The four first-review trait precedence
cases and their ten adversaries remain unresolved as required.

[verify_runtime.py](verify_runtime.py) independently compiles and executes
**15 ordinary Rust controls** under `rustc 1.94.1` (toolchain details in
[toolchain.log](toolchain.log)). Derived Clone returns the original value;
expanded macro trait methods return 2. The include-statement control supplies
a valid block expression; its generated trait is scoped inside that block,
so that particular runtime selects the inherent method. Its graph stays
unresolved because the unexpanded source cannot establish the supplied file's
effects. The initial harness supplied items where include! expected an
expression; the error is preserved in `runtime-controls-initial.json` and
`runtime-initial.log`. No production change was made to satisfy this harness.
Procedural-attribute fixtures test unavailable expansion authority; they are
not claimed to be standalone compilable programs without those procedures.

## Fresh catalog SOLVE PRE and POST

The implemented `data_integrity` family, version 1, owns these four predicates:

- `data_integrity.mutually_consistent`: complete exact typed receiver/target
  tuples, including receiver text for the expansion and trait fixtures.
- `data_integrity.foreign_key`: owner presence.
- `data_integrity.unique`: one observation per owner.
- `data_integrity.cardinality`: required call coverage.

Catalog discovery, rule guidance, initial task state and graph retrieval
failures are retained in [preparation.json](preparation.json). Family requests
are validated by `solve_rules`; no generic fallback, generic preflight, or
`.spur/solver` filesystem read was used.

All **33** persisted receipt IDs, exact requests, responses, and
`get_solve_result` reloads are retained locally. The complete ID list is
[receipt-index.json](receipt-index.json); request lists are
[pre-requests.json](pre-requests.json) and [post-requests.json](post-requests.json).

- PRE: **11 receipts**, all persisted/reloaded before project test edits.
  Five passing batches cover 21 independently authored intended tuples;
  six failing controls inject false targets, missing receiver evidence,
  missing owners, duplicates, missing calls, or discarded positive targets.
  First: `sol_4e242c0c599d4349`; last: `sol_5e347a3bf61e4bdc`.
- POST: **22 receipts** check **72 actual exported observations**: 21 new,
  18 prior trait cases and 33 earlier receiver cases. Fifteen passing batches
  cover the supplied relation; seven failing controls also check an incorrect
  nominal receiver. First: `sol_d4cc813551f946d7`; final:
  `sol_2d22bdc5d53941c0`.

Pass means `sat/pass`; rejected controls mean `unsat/fail`. There are no
unknown or timeout results. Two requests were rejected before solving:
`preflight-family-limit.json` retains the six-row batch that exceeded the
64-variable family bound; batching was reduced without dropping rules.
`post-control-preflight-error.json` retains a duplicate enum label caused by
the renamed Ember fixture; the control now declares a distinct incompatible
nominal label. Neither validation error was counted as a solver verdict.

[project_post.py](project_post.py) reads fresh GraphFacts Calls, receiver text,
nominal scope, owner nodes/spans, and target definitions. Only allowed tuples
come from independent expectations; actual supplied rows come from exports.
Source and export SHA-256 hashes bind the rows in
[post-projection.json](post-projection.json). The checker verifies all receipt
reloads and independently evaluates all **132 family predicates**. It confirms
**17 removed false targets**, **4 preserved new positives**, byte-identical
prior trait-case edges, and all 15 ordinary Rust controls.

```sh
python3 docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-3/project_post.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-3/verify_evidence.py
```

[retained-verification.json](retained-verification.json) records the final
result. The initial checker assumed a `source_span_id` on Calls; public edges
instead carry `evidence_id`. The checker was corrected to verify existing
owner-span and physical caller syntax without changing that public contract;
its initial error remains in `evidence-verification-initial.log`.

## Scope and remaining obligations

Graph orientation failed querying the analyst database, and exact symbol
search failed because the worktree has no graph artifact. Bounded raw reads
were limited to approved R2 files, R2 evidence and the root's supplied local
review inputs. No notebook access, graph CURRENT/publication change, global
installation/config change, original CGA change, unrelated user-file change,
or upstream message was performed. All final edits are in approved R2 paths.

The guard intentionally sacrifices resolution precision around any unsupported
expansion and attributes beyond the narrow proven non-generating set. It does
not expand macros, solve traits, determine applicability or visibility across
external sources, or prove arbitrary Rust compiler equivalence. SOLVE covers
the supplied finite tuples, and runtime tests cover the concrete fixtures.
Root approval and R8 full regression acceptance remain outstanding.

Compact worker audit `9951a6ae-655e-4ddd-ad5f-1132a3acbca2` was recorded on
**bd-382**, comment **1108**, and reloaded by the same idempotent request.
Only local paths and commit references were sent to the tracker. The actual
execution issue was freshly rechecked as `in_progress`; no issue, plan, or
DAG status was changed.
