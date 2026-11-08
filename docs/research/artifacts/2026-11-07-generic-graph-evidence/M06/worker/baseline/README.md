# M06A authenticated native baseline — ROOT review

Evidence-only baseline on clean base `ca95a46f4bdbf18d4490467fbd282c4e36e2aae8`.
All 153 bound inputs, including all product/support sources, are restored to that
base. `schema.rs` was never edited. No four-operation migration, M06 feature
acceptance, ROOT POST, M07, main merge, issue closure, or self-approval occurred.
B remains behind ROOT's serial review.

Canonical `bd-235m`; source tracker `bd-3ex4`; delegation
`282f75d9213441d5`; assignee `spur-worker-282f75d9213441d5`; plan
`b9e69ee5-ca31-4374-8f8d-404c58599647` / `M06A-native-baseline`.
Entry and later checks found in_progress, matching assignment/base/plan/task,
and no supersession. The orchestrator renewed the lease; this worker did not
mutate assignment or retention.

## Observed results

| Acquisition | Exit | Actual native result |
| --- | ---: | --- |
| Initial sandbox wrapper | 200 | AWS endpoint transport failed before compilation; zero native records |
| Network-enabled probe v1 | 101 | Observer json! recursion failure before tests; zero native records |
| Corrected integration command | 0 | provider 16 + macro 9 + Rust contract 19 passed; zero ignored/filtered |
| Required full library command | 0 | 589 passed; 3 ignored; zero failed/filtered |

The corrected observer constructs the same ordered 153-element binding Vec
outside json!, avoiding the recursive token array. No recursion limit, Cargo
file, compiler option, original fixture, original assertion, or production
validator changed. Failed variant sources/patches and raw logs remain under
`attempts/probe-v1/` and `runs/compile-probe-01/`. A local controls-script
development error on helper `validation:null` is also archived under
`attempts/harness-v1/`; it caused no native rerun.

There are **42 actual observations**, **4 native compiled-binding records**,
and **1 separate G03 supplemental record**. Counts by site are S1=6, S2=9,
S3=7, S4=20. All 7 search and 21 selector tests executed; selector
`candidate_selector_uses_uri_when_qualified_name_is_empty` does not call its
artifact helper. Helper execution is counted from captures, not test names.

The 3 existing ignored library tests are the CodeRankEmbed, NomicEmbedTextV15,
and JinaEmbeddingsV2BaseCode model-download smoke tests; their full names and
reasons are in [coverage-report.json](coverage-report.json).
These results cover the selected suites. Separate bugs bd-yptp, bd-3dkq,
bd-1izf, bd-3umt, and bd-16eq were not repaired or accepted here.

## Wrapper case coverage

Each public schema wrapper received the captured graph and genuine source
arguments. New negative cases are explicitly synthetic **input** mutations on
clones of real existing physical fixtures; their Results are actual Rust output.

| Site / case | Actual Result or first error |
| --- | --- |
| S1 absent | Rust evidence unavailable; rebuild with Rust evidence capture enabled |
| S1 present_empty | Ok |
| S1 nonempty_existing_fixtures | Ok |
| S1 stale_source_bytes | source bytes do not match graph manifest blob OID |
| S1 malformed_version_or_checksum | incompatible Rust evidence versions |
| S1 wrong_graph_owner | Rust evidence graph symbol absent, ambiguous or cross-file |
| S2 absent | Ok |
| S2 present_empty_from_real_empty_build | Ok |
| S2 valid_nonempty | Ok |
| S2 stale_source_bytes | macro source OID mismatch |
| S2 scope_manifest_mismatch | macro scope does not match artifact source manifest |
| S2 caller_missing_from_graph | macro caller is not an actual graph function symbol |
| S2 caller_wrong_kind | same exact caller guard error |
| S2 caller_wrong_path | same exact caller guard error |
| S2 caller_wrong_byte_or_line_range | same exact caller guard error |

All declared cases emitted Results. The unique schema scope/caller error sites,
unchanged schema bytes, valid nonempty prerequisite, and captured concrete
mutations establish those guard reaches. Earlier source/version errors are not
claimed as execution of later guards. The range case changes both byte and line
starts; it does not isolate the two conjuncts. S1's malformed case changes the
wrapper schema version; the typed Rust wrapper has no wrapper checksum member.
The existing G03 native checksum/metadata negatives also ran unchanged.

All ten wrapper Err chains have one element. No actual multi-cause chain-order
permutation control is claimed. Complete chains, Debug and Display stay in the
raw records and strict comparison. Full inputs, graphs, native serialized bytes,
Debug, canonical/capability Results and skipped node-ID vectors are retained.

The optional D1/D2/M3 export variables were not used or assumed forwarded.
`native_export_replay_from_supplied_request` is counted by Rust as a passing
test but takes its existing early return without G02_EVIDENCE_REQUEST; it is not
claimed as native replay coverage. The G03 native exporter emitted its real
compiled-in request/config and supplied cases. Its embedded historical base and
config labels are historical inputs, not M06 provenance.

## Provenance and raw data

- [native/index.json](native/index.json) separately indexes 46 native references
  and 1 supplemental reference, with original payload SHA-256, byte spans,
  test/site/case and occurrence correspondence. Raw stdout/stderr remain
  separately compressed under `runs/`. The exact complement spans identify
  all diagnostics; no joining, stripping of values, or output rendering occurred.
- [native/records.jsonl.gz](native/records.jsonl.gz) contains exact framed payload
  slices; [native/supplemental.jsonl.gz](native/supplemental.jsonl.gz) contains the
  exact G03 JSON slice. Records and supplemental references are never interleaved.
- [native/provenance.json](native/provenance.json) binds each observation to its
  preceding compiler record and the actual command. Four native records each
  authenticate all **153 actual include_bytes! inputs**, independently checked
  against saved bytes. Three distinct executables appear in those records; the G03 supplement
  authenticates a fourth test executable.
  All report actual `rustc 1.94.1 (e408947bf 2026-03-25)` with complete -vV output.
- `source-inputs.tar.gz` freezes all 153 base byte streams (7,805,832 source bytes).
  `original/` additionally freezes the four temporary-edit targets.
  `variant/`, `variant-sources.json` and `observer.patch.gz` freeze the exact
  successful compiled variant before execution. Native hashes are computed by
  Rust over compiler-included bytes; no expected hashes fill observation fields.
- Each `runs/*/command.json` records exact argv/env, start/end, exits, HEAD before
  and after, actual stream hashes, and source stability. The raw remote invocation
  records the provider flags and -j8. Both successful commands held HEAD ca95a46f4
  and identical source bytes throughout upload/build/run.

The graph-first precise read exceeded its 15-second deadline; orientation did
not return before its orchestration cell was terminated. No current graph
freshness/OID was established. Bounded current/frozen reads were the authorized
fallback; no graph resync occurred. Entry lifecycle archives retain this gap,
the advertised report_signal schema's missing signal_id error and successful
documented retry, and the restricted process diagnostic after sandbox ps failed.
The process diagnostic's host environment is explicitly a separate during-run
shell snapshot; actual forced command environment is in command.json.

## Strict copied-output controls

[compare.py](compare.py) first authenticates source byte archives, complete raw
stream/reference/span/multiplicity coverage and native compiled bindings. It then
compares full values, all native wire byte arrays, Debug, Results/errors, ordered
lists and run outcome counts. It does not recalculate native canonical hashes,
capabilities, Debug or serialization in Python.

[controls.py](controls.py) executes independent mutations of copied actual fields,
rather than letting raw hash mismatch stand in for every semantic comparison.
Strict identity passes and **46 synthetic controls reject**; exact targets and
original native references are in [controls.json](controls.json). Controls cover
missing/extra records and references, input/site/case/test mismatch,
None/Some(empty)/omission, scalar/owner/ID/path changes, file/symbol order,
JSON-equivalent serialized-byte changes, version/hash/canonical/capability/Debug,
Result/error/chain changes, metadata-error ordering, omitted/null counts and
metadata, compiler/executable omissions, and forged/missing/reordered source
bindings. These controls are synthetic output/reference mutations, not native
Rust observations or the earlier PRE policy falsecontrols.

Only named process/executable transport fields are excluded after presence and
provenance checks. Compiler -vV, package version and all product values stay
strict. Two actual G03 temporary-root substrings in
`cases[i].rust_negative_error` are separately indexed with exact original spans
in [native/observed-roots.json](native/observed-roots.json). Strict mode rejects a
changed root; the explicit `--allow-observed-g03-roots` comparison view can map
only those authenticated case error-path spans. Raw records are never rewritten.

**G03 metadata-map/error ordering remains strict.** Its raw diagnostic contains
formatted metadata maps and may expose comparison drift in B. No M05 ordering
exception applies. Any such failure must stay intact and return to ROOT;
A's copied identity does not demonstrate cross-process diagnostic stability.

## Restoration, PRE and ROOT review

[observer-conservation.json](observer-conservation.json) proves that removing only
observer scaffolding gives the exact original helpers, that both support files
keep the originals as exact prefixes during capture, and that the other 149
inputs (including schema) never change. [restoration.json](restoration.json)
authenticates every current restored byte against the 153-input base ledger;
the product/support Git diff is empty.

Current-source PRE verification passed before capture and again after restoration.
Fingerprint `a99d2c132816f945b50cee8c45476c3cc743a2669f69253db8931d9e187e0b4d`.
Both complete live envelopes, `sol_e556e670e1904bad` SAT/pass and
`sol_aa9e9b76e042467c` UNSAT/fail, equal proof.json.gz's reloaded envelopes field
for field; complete tool returns are archived under `entry/`.
Jev remains BLOCKED at 0.48; the explicit direct-catalog policy route remains
unchanged. No solver creation, pin/unpin, retired-ID reload, quota/limit change
or ROOT actual POST occurred. Protected M04 receipts are untouched.

From the repository root, ROOT can review without native reruns:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/baseline/verify_observers.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/baseline/compare.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/baseline/controls.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/worker/baseline/verify.py
```

`review.diff.gz` is the scoped staged unified text diff, with binary payload
notices rather than duplicate native blobs. Its only omitted generated files
are itself and manifest.json. `manifest.json` hashes every committed packet file
except itself. The final Git commit is the complete authoritative diff.
INTENT/ACTION audit requests and persistence replays are in `entry/`; the final
RECORD, including commit and manifest hashes, is persisted and replayed in the
canonical issue, avoiding a self-referential commit/audit hash.

ROOT must independently inspect source/probe conservation, raw/native bindings,
all case errors and controls, then approve A before B. In_progress is retained
for review. This packet asserts baseline acquisition completion only.

## Matching B acquisition, after ROOT approval

Reuse the successful v2 capture code, exact case names/arguments and command
configuration from [commands-for-B.json](commands-for-B.json). No rerun of the
failed v1 variant is proposed. Copy tools/evidence into B's own authorized
directory; do not overwrite this packet. The install generator preserves the
current artifact literal and can reanchor its separate fenced observer around
B's two removed None fields. Review that probe semantics are byte-identical.

B must independently freeze its actual 153 compiler inputs, changed schema
snapshot, all four probe snapshots and stable HEAD before its commands.
The comparator's source bundle requires a saved variant snapshot for **every**
input differing from its base archive, including B's schema migration.
It verifies those bytes, not authorization for arbitrary source changes; ROOT
still reviews the exact four-operation product diff.

Run exactly the same integration and library commands below, once on the
authorized stable variant, using scripts/spur-cargo and no local fallback:

```sh
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_SYNC_FULL=1 scripts/spur-cargo test -p spur-graph --test rust_evidence_contract --test macro_invocation_artifacts --test generic_evidence_providers --no-fail-fast -- --nocapture --test-threads=1
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_SYNC_FULL=1 scripts/spur-cargo test -p spur-graph --lib -- --nocapture --test-threads=1
```

B removes its observers and restores both support tests while retaining only
its separately authorized four operations. A's `restore.py` intentionally restores
ca95 bytes and must not undo B's migration. Compare authenticated A/B bundles
strictly first; retain any failure and use only the explicit actual-root mapping
where applicable. No error-order or artifact-order normalization is authorized.
ROOT owns M06 actual POST/acceptance and any M07 successor.
