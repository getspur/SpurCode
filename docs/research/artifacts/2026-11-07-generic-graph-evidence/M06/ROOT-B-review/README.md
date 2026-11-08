# ROOT M06B review: bounded comparison follow-up

Decision: request an evidence-only follow-up before approval. Product migration
`2ee50a5fef63ff2b98cf6e965072ce228197fecc` is preserved. No main merge, actual POST,
whole-M06 acceptance or M07 dispatch has occurred.

ROOT verified canonical `bd-3qkw` is in progress with completion audit 4999 and
RECORD 4997. The blocked signal `45cc6321-a6e7-4350-93b1-53d731647fdd` correctly
reports two G03 diagnostic differences; it remains unresolved pending the new
comparison tests and ROOT actual POST. The earlier lease risk ended with the
completed acquisition and clean restored checkpoint. `beads-readback.json`
contains the complete canonical comments at review time.

The source diff has exactly four operations in three files: use the two owned
evidence getters and replace the two helper fixtures' typed None fields with
empty draft tails. The remaining validation bodies, error ordering, explicit
fixture fields and eight compatibility accesses remain exact. Both support test
files were restored. `source.diff` is ROOT's independent Git diff.

ROOT replayed B's packet verifier, all 46 copied-native controls and eight source
controls. They pass authenticity/control checks while correctly reporting
conservation=false. Both native bundles authenticate 153 actual source inputs,
four compiler binding records and their executable correspondence. All 42 actual
site observations match exactly. No subset is accepted in place of the whole
supplemental-inclusive dataset.

The genuine commands retain 44 integration passes, 589 library passes and three
existing download-model ignores. Final restored-source library/check/fmt passed.
No new native executions were performed by ROOT. Existing limits remain explicit:
one selector test bypasses its helper, an optional G02 replay early-returns, the
ten Err chains have one element, and the range probe changes both byte/line starts.
Known unrelated failures and the archived transport failure remain unchanged.

`diagnose.py` independently authenticates both bundles and locates every remaining
difference. Only two fields differ: G03 cases zero and one `rust_negative_error`.
Each contains expected and actual maps with thirteen duplicate-free string pairs.
All decoded pairs, complete raw entry slices, map delimiters, positional separator
bytes and outer text match after the existing actual-root view. Only whole-entry
order differs. The unchanged native formatter uses HashMap `{:?}` and promises no
stable entry order. Its complete source and the native G03 producer bytes match
A/B compiler readbacks and Git. `diagnosis.json` retains every raw entry slice.
This diagnosis does not activate a qualified comparator or override strict=false.

ROOT authored and verified `../comparison-pre/`: an explicit prospective policy
for exactly these four maps in two fields, with strict protection of every other
value/byte and full dataset coverage. Jev opened at 0.86; the two persisted catalog
receipts returned SAT/pass and UNSAT/fail, with eight independent compiled
predicate checks and fourteen individual negative policy controls. This is a
policy proof, not parser correctness or native acceptance. The original source
PRE was fully reloaded and losslessly archived before its authorized consumption
into the actual POST review pipeline; M04 pins remain protected.

Next: Codex/gpt-6.1-sol/xhigh will implement and test only the new comparison
under `worker/comparison/`, using frozen A/B captures. Require a real failing
assertion first, both real case permutations and adversarial controls. Preserve
all previous files and false comparisons. No source edits, capture, builds or
native reruns are authorized. ROOT will independently review this result and run
the required actual POST before any approval or local-main merge.

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/ROOT-B-review/diagnose.py
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M06/comparison-pre/verify.py
```
