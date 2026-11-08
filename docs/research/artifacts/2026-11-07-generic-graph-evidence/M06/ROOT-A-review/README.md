# M06A ROOT baseline checkpoint review

ROOT approves A as authenticated pre-migration acquisition. This is not M06
feature acceptance, an actual before/after result, or authorization to start M07.
The dependent B worker may now perform its four already-authorized operations.

Original worker commit `2407d39cab150b418a41a7db3d0c1c92a4d81797` changes exactly
72 files under the baseline evidence directory. Canonical RECORD 4969 and
completion 4972 match that commit, the fixed manifest digest, native index,
controls and restoration hashes. The worktree is clean. All 153 source inputs
match both the PRE inventory and original Git bytes; schema.rs never changed.
Removing only the capture scaffolding reproduces both exact original helpers,
and the two support tests retain their original bytes as exact prefixes during
acquisition. Final source restoration is exact.

ROOT replayed observer conservation, the raw/source/provenance checks, strict
identity, all 46 copied-native rejection controls and the full packet verifier.
It independently verified all 71 manifest members, then parsed every native
frame directly from both successful raw streams and matched references/spans:
17 integration frames plus one G03 frame, and 29 library frames. There are 42
observations (S1=6, S2=9, S3=7, S4=20) plus four native compilation records,
each authenticating all 153 compiler-included source byte streams. Three native
test executables plus the separate G03 executable are represented.

Actual integration outcomes are 44 passed, zero failed/ignored/filtered; library
outcomes are 589 passed, zero failed, three named existing model-download ignores
and zero filtered. One selector test bypasses its helper. The env-driven G02
replay test takes its existing early return and is not claimed as replay coverage.
The 15 declared wrapper cases emitted actual Results. Scope/caller guard reaches
are supported by unchanged source and the actual prerequisite graph/input values;
earlier source/version failures do not establish later branch execution.
The range mutation changes byte and line starts together. All ten native Err
chains have one element, so no measured multi-cause chain-order test is claimed.

Eight additional ROOT probes passed: seven mutations reject, while the explicit
actual-root view accepts only a copied change to the two recorded G03 temporary
root spans. The same change rejects in strict mode. Altered observation order,
run exit/pass counts, compiler identity, non-root error bytes and metadata-map
entry order all reject, including with the explicit root view enabled.

The quota/PRE issue was resolved earlier under user approval; A did not change
retention. Current M06 PRE remains the direct catalog proof with Jev blocked at
0.48. No solver or native rerun was needed for this checkpoint review. The earlier
transport exit 200, probe-v1 json! compile exit 101 and local harness development
error are preserved. The observer fix builds the binding Vec outside json!;
it does not increase recursion limits or alter original tests/validators.

Risk 404e3dca (acquisition lease) ended with successful restoration and completion.
Risk 9f03ad72 remains an explicit B boundary: G03 metadata/error ordering must
stay strict. No M05 order exception is inherited. Only the two genuine captured
temporary-root spans may use the explicit path view, after a strict result is
retained. Any other B drift requires ROOT review and remains a failing comparison.

B must preserve this packet. Copy observation tools into its own directory,
reanchor only the capture wrappers, and snapshot **every** changed compiler
input, including schema.rs. A's install generator only saves its four temporary
targets; B must additionally save the changed schema snapshot. A's restore.py
restores ca95 bytes and must not undo B's migration. Preserve separate clean
migration snapshots and capture-only variants/diffs. The appended integration
probe code and native comparison semantics remain identical.

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/ROOT-A-review/replay.py
```

This recreates the deterministic ROOT replay report from immutable captures,
without native execution or baseline edits. ROOT owns final M06 actual POST,
independent adversarial review, and any later local-main merge after B.
