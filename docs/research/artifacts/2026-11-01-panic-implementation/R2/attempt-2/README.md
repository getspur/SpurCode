# R2 attempt 2: conservative trait receiver dispatch

Rationale: nominal receiver evidence must not select an inherent method when unsupported Rust trait/autoref dispatch can select another candidate.

Assigned execution issue: **bd-382**, verified `in_progress` before edits.
R1 is approved; R3 remains pending on R2. Root owns review and R8 owns full
regression acceptance. This work neither closes the issue nor advances the DAG.

## Changes and history

- `88cba3459`: preserve the staged attempt-one overlay and evidence. The original
  attempt branch ending at `e42495ee5` remains available; no prior evidence was overwritten.
- `1dfb818d4`: commit independent regression tests and fresh SOLVE PRE before RED.
- `57a6a3025`: minimal resolver guard plus scoped formatting, after runtime RED.
- [unified.diff](unified.diff): attempt-two code/test delta; [combined.diff](combined.diff):
  complete R2 code/test delta against the R1 baseline `6520e079b`.

The `.diff` views use zero context (`git diff --unified=0`; apply with
`git apply --unidiff-zero`). Exact original context patches and captured log
bytes are preserved in [raw-output.json](raw-output.json). Readable log copies
remove trailing spaces and surplus final blank lines; this addresses the final
staged whitespace check without discarding original command output.

The guard refuses inherent resolution when any observed trait implementation
has the same nominal owner (including reference implementations), or its owner
cannot be established exactly, including bounded generic and qualified owners.
It also refuses resolution when a lexical import leaves extension-trait
visibility unsupported. A known, different local nominal owner does not block
the receiver. No trait target is guessed, and no trait solver is introduced.

Reference layers are deliberately over-approximated together **for rejection**.
The resolver never uses that erasure to choose between inherent and trait
candidates. Thus all four reviewed receiver forms are refused, including
by-value trait methods and trait implementations for references. Original
source text and nominal receiver metadata remain in exported GraphFacts;
unsupported dispatch retains its source call observation and nominal scope.
Public GraphFacts and graph schema v11 remain compatible.

## Runtime verification

Exact commands, exit codes, and logs are in [commands.json](commands.json).
All Cargo invocations used `SPUR_REMOTE=0 scripts/spur-cargo`.

RED command:

```sh
SPUR_REMOTE=0 SPUR_R2_EXPORT_DIR=docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-2/red-exported scripts/spur-cargo test -p spur-graph --test rust_receiver_resolution
```

[red.log](red.log) exits **101** with five runtime assertion failures, covering
all four root cases and ten renamed, bounded, default-method, imported-trait,
and constructor variants. All fourteen incorrectly resolve an inherent method.
Seven other tests pass. This is genuine runtime RED, not compilation RED.
The relative export destination initially wrote this run's files under the
crate working directory; those files were moved into this authorized evidence
directory and the empty directories created by the run were removed. All
subsequent exports use an absolute path.

GREEN command (the export directory is set to the absolute evidence path):

```sh
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_receiver_resolution
```

[green-final.log](green-final.log): **12 passed** after formatting.
[green-regression.log](green-regression.log): **56 passed** across receiver,
lexical, call-range, constructs, macro query, macro invocation, and panic-site
integration tests. [resolver-unit.log](resolver-unit.log): **66 passed**.
Crate check, scoped formatting check, and diff check pass. The existing
`FileOidMatch::as_bool` dead-code warning remains in the build logs.

[verify_runtime.py](verify_runtime.py) independently compiles and executes all
18 hand-authored fixture programs: fourteen select trait methods and four
select inherent methods. The initial default-method control lacked a `Sized`
bound; the test commit corrected that fixture before graph RED. Its initial
compile error is retained in `runtime-controls-initial.json`, separately from
the successful final [runtime-controls.json](runtime-controls.json).

The previous Alpha, borrowed Beta, custom unwrap/len, constructor, interval,
shadow, namespace, unsupported-type, and eleven physical Option-site controls
all pass. No snapshot was changed; generated graph exports are evidence only
and are never test inputs. `cases.json` contains manually authored source and
expected outcomes, committed before RED.

## Fresh SOLVE evidence

Catalog rules, all version 1, executed through `solve_rules`:

- `data_integrity.mutually_consistent`: exact typed receiver/target/provenance
  tuples with required cells.
- `data_integrity.foreign_key`: call owner presence.
- `data_integrity.unique`: unambiguous owner observation.
- `data_integrity.cardinality`: observation coverage.

The matching implemented family owns request validation; no generic fallback
or generic preflight was used. The four catalog responses are retained as
`catalog-0.json` through `catalog-3.json`. Every receipt contains its exact
family request, response and `get_solve_result` reload, including the persisted
lowered request. No `.spur/solver` file was inspected.

PRE: five passing batches over 18 independent intended tuples plus seven
rejected mutations, all persisted/reloaded before project test or production
edits. First PRE: `sol_8bf5b2df20324f93`.

POST: twelve passing batches over **51 fresh actual exported observations**
(18 new, 33 retained), plus seven rejected mutations. First POST:
`sol_56d9a02d16454024`; last retained POST: `sol_82c1f3e1ec804cf4`.
All **31** PRE/POST IDs and raw verdicts are in [receipt-index.json](receipt-index.json).
Pass means `sat/pass`; rejected controls mean `unsat/fail` with family
diagnostics. There are no unknown or timeout results.

The seven controls inject a false inherent target, absent receiver evidence,
missing owner, duplicate observation, missing call, incompatible nominal type,
or discarded positive target. POST controls mutate fresh exported facts rather
than replaying PRE rows. The full family requests are in `post-requests.json`.

[project_post.py](project_post.py) obtains supplied rows from actual Calls edges,
`scope_text`, `receiver_text`, target definitions, and owner spans. Only the
allowed tuples come from the independent expectations. Source and export
SHA-256 values bind every row to the graph export in `post-projection.json`.
The final export run reproduces the same supplied relation and provenance.

```sh
python3 docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-2/project_post.py
python3 docs/research/artifacts/2026-11-01-panic-implementation/R2/attempt-2/verify_evidence.py
```

[retained-verification.json](retained-verification.json) verifies all 31 receipt
reloads, 51 observations, fourteen changed false targets, and eighteen Rust
runtime controls. Detailed source/solver evidence stays local; tracker records
contain compact local paths and commit references only.

## Bounds and remaining obligations

The source lane intentionally sacrifices precision when trait visibility or
implementation ownership is uncertain, including imports unrelated to the
observed method and unknown generic impl owners. It does not establish actual
trait applicability, default-method selection, receiver candidate ordering,
or user-defined autoderef. It never emits a guessed trait target. These are
bounded checks over the supplied source fixtures, not a proof of complete
Rust dispatch or compiler equivalence across arbitrary external/generated
implementations. Compiler authority remains a separate obligation.

Graph orientation failed on the analyst database query; exact symbol search
failed because this worktree has no graph artifact. Bounded raw fallback was
limited to the three approved R2 source/test files and local R2 review/evidence
files. No graph CURRENT/publication change, notebook access, global config
change, original CGA change, unrelated user-file edit, or upstream message was
performed. `preparation.json` records the retrieval failures and assumptions.
Root review and R8 complete regression acceptance remain outstanding.

Compact completion audit `63609889-0561-4017-898d-470bf1cac3d9` was recorded on
`bd-382` as comment `1099` and reloaded through the idempotent audit call.
The issue was rechecked as `in_progress`; no task or plan state was changed.
