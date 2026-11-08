# M06B four-operation migration — conservation blocked for ROOT review

The authorized mechanical source migration is present and all temporary capture
hooks are removed. **Whole-dataset conservation is false.** Both the first strict
comparison and the explicitly permitted observed-root view fail on the two G03
supplemental `cases[0/1].rust_negative_error` strings: metadata map-entry order
differs. Complete strings, raw bytes and failed comparator exits are retained.
No order exception, output rewriting, semantic repair or native rerun was used.
ROOT must decide this blocker before M06 acceptance or actual POST.

## Product scope and restoration

The final unified diff against `ca95a46f4bdbf18d4490467fbd282c4e36e2aae8`
is [final-source.diff.gz](final-source.diff.gz): exactly four operations in three files,
four insertions and six deletions.

| Site | Exact operation |
| --- | --- |
| S1 | `validate_rust_evidence_sources` obtains its owned Option through `rust_evidence_set()?`; absence text and subsequent validation are exact. |
| S2 | `validate_macro_invocations` obtains its owned Option through `macro_evidence_set()?`; None success and all subsequent checks/order are exact. |
| S3 | Existing search helper removes only the two explicit typed None members and adds its final empty tail. |
| S4 | Existing selector helper makes the same literal-tail change. |

[restoration.json](restoration.json) checks all 153 inputs, six migrated field
occurrences and the eight individually source-bound compatibility occurrences.
Every final input equals the base followed by only these four substitutions.
Consequently all original assertions, fixture expressions/order/IDs/spans,
GraphFacts, storage/serde/default/getter/setter internals and old wire controls
remain exact. Both support integration files are restored byte-for-byte.
[final-sources.json](final-sources.json) gives all actual final lengths/hashes;
`final/` freezes the three clean product files and both restored support files.

## Assignment, dependency and PRE

Canonical `bd-3qkw`; source tracker `bd-hx77`; delegation `4700ac6b542e416a`;
assignee `spur-worker-4700ac6b542e416a`; plan
`b9e69ee5-ca31-4374-8f8d-404c58599647` / `M06B-native-migration`.
Live entry confirmed in_progress, matching assignment/base/plan/task and no
supersession. Plan projection independently reported A approved and B dispatched.
Inherited HEAD was `2407d39cab150b418a41a7db3d0c1c92a4d81797`, A's evidence-only
commit. ROOT's binding review at `f4f447e00ad63921ca437b1915c5aa0eb55b9287`
is copied read-only in [entry/root-A-review.md](entry/root-A-review.md).

A's 71 manifest members authenticate against manifest SHA-256
`ba0e35d116b29cffc78f525a339d407495b6bb266aa74decf8af094cb71f333b`.
All 153 inherited source paths equaled both PRE and Git base before migration.
The original baseline packet and its scripts/expectations remain unchanged.

Current-source PRE verification passed before edits. Both complete live
`sol_e556e670e1904bad` and `sol_aa9e9b76e042467c` envelopes equal their archived
`reloaded` envelopes field for field; full responses are in `entry/`.
Fingerprint `a99d2c132816f945b50cee8c45476c3cc743a2669f69253db8931d9e187e0b4d`.
Jev remains **blocked at 0.48**; the documented direct catalog route is unchanged.
[historical-pre-verification.json](historical-pre-verification.json) checks the
bound baseline after migration and explicitly says current_source_verified=false.
It does not attest changed native/current bytes. No solver creation, retention,
quota, threshold or limit operation occurred; retired M05 IDs were not reloaded.

Graph orientation returned off-target graph `50a577...`; precise search/read
returned the expected validator from graph `24411aed...` with file OID matching
the base, but indexed/worktree HEAD and response-file freshness stayed unknown.
Full returns are archived in `entry/graph-*`. Current/base bytes were therefore
authenticated independently with bounded raw reads. No graph resync occurred.

## Actual native acquisition

| Command/phase | Exit | Actual result |
| --- | ---: | --- |
| First sandbox integration attempt | 200 | AWS endpoint transport failure before compilation; zero stdout/native records |
| Network-enabled matching integration acquisition | 0 | 16 provider + 9 macro + 19 Rust contract tests passed; no failures, ignores or filters |
| Matching serial library acquisition | 0 | 589 passed; 3 existing model-download ignores; no failures or filters |

The initial failure remains under `attempts/sandbox-transport/`. The retry uses
ordinary wrapper startup/network access; no manual VM/token/cache/resource action
or local fallback was used. Acquisition commands and environment are byte-for-byte
the copied A runner: remote=1, no-local-fallback=1, full-sync=1. All commands use
`scripts/spur-cargo`. Full raw streams, actual remote invocation/config and
source/HEAD stability are under `runs/*/command.json` and compressed raw files.
The stable acquisition and final-test HEAD is the inherited A commit; dirty
source variants are separately authenticated, not relabeled as a compiled commit.

There are 46 native records: four compiled-input records plus 42 observations,
S1=6, S2=9, S3=7, S4=20; one G03 supplemental record is indexed separately.
Every native compilation record authenticates all 153 actual compiler-included
byte streams. `source-inputs.tar.gz` is the unchanged base archive; `variant/`
contains every changed compiler input, including schema.rs. Native readbacks
match [variant-sources.json](variant-sources.json). No expected hashes populate
native fields. Actual compiler/executable/source correspondence is retained in
[native/provenance.json](native/provenance.json) and
[native/executable-lineage.json](native/executable-lineage.json).

[prepare_capture.py](prepare_capture.py) applies the four exact substitutions to
A's saved successful v2 capture variants. The appended integration probe variants
are byte-identical to A, and helper capture code changes only around the authorized
literal edits. [capture-conservation.json](capture-conservation.json) binds these
variants. [observer.patch.gz](observer.patch.gz) is capture-only relative to the
saved clean migration. No original test body, fixture or production validator was
instrumented. The v1 observer was never rerun and no recursion limit changed.

## Strict comparison and controls

The copied [compare.py](compare.py), [extract.py](extract.py) and
[controls.py](controls.py) retain A's exact bytes. Both raw/source bundles are
authenticated before comparison: record/reference identity, every frame and
diagnostic complement span, hash/length, multiplicity, actual source bindings,
compiler/executable presence and native command outcomes.

[comparison-strict.json](comparison-strict.json) retains the first exit 1.
[comparison-observed-roots.json](comparison-observed-roots.json) also retains
exit 1. Only the exact two authenticated per-case temporary-root spans use that
explicit view. Every other error byte and metadata entry order remains strict.
[comparison-all-differences.json](comparison-all-differences.json) lists **all**
two differing full fields before and after that view. Raw values are unchanged.

All 42 observations, including actual inputs, full artifact values, native serde
bytes, Debug, canonical/capability outcomes, presence, validation Results/errors
and ordering are exact at matching site/case/test/multiplicity. This is a
diagnostic fact recorded in [observation-comparison.json](observation-comparison.json),
**not acceptance of an equal subset**: the full supplemental-inclusive result
remains false. Ordered serialization and all non-transport fields remain covered.

The actual after data passes strict identity and rejects all 46 copied-native
output/reference mutations in [controls.json](controls.json). These controls
cover the same categories as A, including JSON-equivalent wire-byte changes,
presence, field/row order, owner/ID/path, canonical/capability/Debug, complete
Results/error fields, metadata diagnostic order and source/executable provenance.
[scope-controls.json](scope-controls.json) separately rejects eight copied-source
mutations. All controls are labeled synthetic and are not native Rust outputs.

## Coverage limits and final verification

All 15 declared wrapper cases emitted actual full Results with genuine fixture
inputs and explicit synthetic input mutations, using unchanged A probe logic.
Scope/caller reaches use valid nonempty prerequisites and their exact captured
guard errors; an upstream error is not claimed as a later branch execution.
The range probe changes byte and line starts together. S1's malformed probe
changes wrapper schema version; that typed wrapper has no checksum field.
All ten wrapper Err chains contain one element, so no measured multi-cause
chain-order permutation is claimed. The remaining validator text is source-checked.

One selector test bypasses its helper. G02's optional environment-driven replay
test takes its existing early return; it does not establish replay coverage.
No D1/D2/M3 export variables or forwarding were assumed. Historical G03 embedded
base/config labels remain historical inputs, separate from M06 provenance.

[coverage-report.json](coverage-report.json) records exact test summaries,
three named existing model-download ignores, warnings and every final command.
After removal of observers, the required normal library command, package
all-target check and local formatting check run once on the restored final inputs.
Their raw results are in `runs/final-*`; no expectation or original assertion
changed. Known bugs bd-yptp, bd-3dkq, bd-1izf, bd-3umt and bd-16eq remain separate.
No workspace-wide success, semantic bugfix or performance claim is made.

| Required final command | Exit | Result |
| --- | ---: | --- |
| Remote `test -p spur-graph --lib` | 0 | 589 passed, 0 failed, 3 existing ignores, 0 filters |
| Remote `check -p spur-graph --all-targets` | 0 | Complete; unused-code/import warnings retained |
| Local `fmt -- --check` | 0 | Formatting check passed |

[post-candidate.json](post-candidate.json) derives actual measurements from these
records, source controls and final commands. It records acceptance_blocked=true;
it is not a solver receipt or a copied prospective policy result.

## ROOT replay and return

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/migration/verify.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/migration/controls.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/migration/scope_controls.py
```

The verifier authenticates the packet while reporting conservation=false; it does
not turn a strict failure into acceptance. The latter two commands regenerate
only B's synthetic-control reports. `entry_verify.py`, `prepare_capture.py`,
`acquire.py`, `restore_final.py` and `final_checks.py` are phase-specific acquisition
tools, not read-only replay commands. A's restore script is never used in B.

`manifest.json` authenticates every packet member except itself. `review.diff.gz`
is the scoped staged unified text diff, with binary notices instead of duplicate
payloads; it excludes itself and the manifest. The final Git commit is the
authoritative complete diff. Durable INTENT 4985 and acquisition ACTION 4994,
runtime risk `0cffd7dd...` and blocker `45cc6321...` target this canonical issue.
Final RECORD includes the committed checkpoint/manifest hashes and is replayed
in Beads, avoiding a self-referential commit/audit hash.

The task remains for ROOT review with a blocking strict result. No self-approval,
issue closure, main merge, ROOT actual POST, M07 or retention change occurred.
