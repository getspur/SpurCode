# M02 independent ROOT review

The nine-site refactor is eligible for scoped approval. It replaces eighteen
explicit typed `None` fields with nine `GraphIndexArtifact::empty()` update tails
in the three approved files. ROOT independently reversed the transformation to
the exact original bytes, preserving every other expression and all 413 assertion
macros. No observation code, workload adjustment or assertion change remains.

Original source: `4a411dbd24f306a27b48b7e11b2ab381b7e39797`.
Migration: `076aff8d53dcb50075b59083adb1025c5387c783`.
Original final: `ac499068601f4899bb9b0529564bb2971f760538`.
Normalized worker output: `ef8bc22c99d53e8195b52a7069b69db28862ebc1`.
The final trees are identical; ROOT preserved the original worker lineage.

ROOT parsed all 38 actual native records directly from the compressed command
logs. The 19 before/after observations cover all nine constructors, with complete
matching values and inputs. Small wire/Debug values, skipped node IDs and native
Git identities were authenticated. Large 50,000-snapshot benchmark values remain
bound by their native digest/length, canonical/capability hash, capacity and
companion commit-index observations. ROOT applied both saved observer patches to
frozen Git source in temporary directories and checked all three resulting
compiled source hashes against command and native platform readbacks. The 125
final source bindings differ from PRE only in the approved three files. Command
snapshots bind those three compiled files and fixed HEAD; this is not an
independent rebuild or attestation of every dependency. Six additional ROOT
mutations of actual native values/arguments were rejected; ten worker controls
were also authenticated. No Python native canonical/BLAKE3 implementation is used.

Final unfiltered native results retain six failures:

| Target | Passed | Failed | Ignored | Filtered |
|---|---:|---:|---:|---:|
| Core graph integration | 44 | 5 | 0 | 0 |
| Worker dispatch integration | 21 | 1 | 0 | 0 |
| Graph library | 589 | 0 | 3 | 0 |

The core and worker passed/failed names match their actual baselines, including
the same diagnostic categories. ROOT accepts these risks for this strictly
mechanical migration, with no whole-suite GREEN claim. The three real-tool
ownership failures remain open in `bd-3dkq`, two freshness failures in `bd-1izf`,
and the curated worker count mismatch (34 versus 31) in `bd-3umt`. Existing analyst
overlay issue `bd-16eq` remains separate. The three graph ignores are named
pre-existing model-download smoke tests. Both package all-target checks, scoped
formatting, source/evidence diff checks and real before/after release benchmark
routes passed. Final assertions were not filtered or weakened.

Worker INTENT/ACTION evidence comments 4716, 4717, 4722 and 4726 match exact saved
requests and responses. Completion UUID `e3770ec9-fd67-4e32-931f-7d4f29528b4a`
did not appear as worker evidence in the canonical readback. Its unreturned
calls remain immutable and unconfirmed. Orchestrator completion 4730 and ROOT
record 4732 durably reconcile the returned result; neither impersonates the
missing worker receipt. Both startup/baseline risk signals have been reviewed:
startup completed, and baseline failures remain open as scoped follow-ups.

The first POST projection duplicated full passed-name lists and source context
inside each finite tuple. Jev rejected that 260,041-byte facts payload with a
transport `max_tokens_exceeded` error. The exact oversized input/error are kept.
The final projection retains full raw data and authenticates passed-name lists
by digest, binds common source/archive context once, and keeps failing names and
counts explicit. Its facts payload is 23,021 bytes. No service quota, threshold,
solver bound, evidence requirement or native result changed. Jev 1.14 then opened
at weakest confidence 0.76.

Actual POST fingerprint:
`de3fb9b2a28123a7e18aea369d3005081bb802588fb074269b6f50bfde96b5b8`.
It binds 125 source files, 128 archives, 18 actual cases, two ownership lanes and
60 compiler variables. `sol_57a43e4e09d44e5d` is SAT/pass;
`sol_13955c8458594eba` is UNSAT/fail. Both complete receipts are pinned/reloaded;
the independent finite oracle checks eight predicates and rejects nineteen
individual false claims. This verifies the finite evidence projection, not all
possible program behaviors. Consumed M02 PRE pins were released only after exact
full live-envelope comparison and eight independent predicates; accepted M01
POST pins remain protected.

Replay ROOT verification (frozen source and actual output):

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M02/ROOT-review/verify-post.py
```

Add `--current-source` only while the bound M02 source is current. The worker's
own verifier was separately replayed at its original result tree; its scope
assertion is historical and excludes subsequent ROOT documentation. ROOT's
frozen-epoch replay remains usable after later caller slices. No production
repair of the open follow-ups or successor implementation is claimed here.
