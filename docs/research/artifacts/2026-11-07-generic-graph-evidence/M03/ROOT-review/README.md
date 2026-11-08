# M03 independent ROOT review

M03 is eligible for approval. The three-file change replaces twelve typed `None`
fields with six `GraphIndexArtifact::empty()` tails. ROOT reversed it to the exact
original bytes, preserving every other expression and all 502 assertion macros,
including the explicit production empty-artifact header/manifest/hash calls.

Original PRE: `3b3def64ab3fb81458aafe3145a90ce93a8069f5`.
Migration: `4179448927de9d8f03c312cafa5819f2acf0abb5`.
Original final: `190723c6aa520f87ee3b395bf3a8c1ca65e23a0c`.
Normalized output: `5de5f250cb4cb60497e5c1aeced6694c44e204d0`.
The final trees are identical; original commit lineage is preserved.

ROOT parsed actual command logs independently. Nine records before and nine after
cover all six constructors. Complete native values, caller-resolved function and
named thread/case, inputs, eleven vector lengths/capacities, skipped node IDs and
native canonical/capability identities match. Numeric thread IDs, sequence and
shifted caller line numbers remain in raw output and are excluded from value
identity only. Small raw values were independently length/SHA256 checked; large
payloads retain their native BLAKE3/SHA256/length. No Python implementation of
native canonical hashing is used. ROOT rejected six additional mutations of real
records; thirteen worker mutations were replayed independently.

Four saved observer variants were reconstructed against frozen Git source in
temporary directories. The accepted native `include_bytes!` source exports match
the appropriate reconstructed overlays and command snapshots. All 131 source
inputs in each required command remained stable; final source differs only in the
approved three files. Earlier observer-only compiler failure and rejected
interleaved export runs remain archived and are not offered as acceptance.
ROOT's status parser removes complete native JSON event records before matching
libtest lines split by those exports; source/native event authentication remains
separate. Receipt IDs returned as decimal strings are compared to Beads IDs with
their full audit payload unchanged.

The benchmark uses the documented supported override. Native preparation built
the pinned sample corpus through existing graph APIs. All fifteen read-only
artifact file bytes and three corpus file bytes are authenticated and equal
before/after; both processes performed six identical loads from the same path.
The original 131,072-symbol pruning workload, 512 noise items and capacities are
unchanged. The default external fixture/repository-fallback route was not run.
This establishes constructor conservation and makes no performance claim.

| Native validation | Passed | Failed | Ignored | Filtered |
|---|---:|---:|---:|---:|
| Full graph library, before and final | 589 | 0 | 3 | 0 |
| Exact owning cases, each phase | 2 | 0 | 0 | 590 |
| Existing provider/accessor negatives, each phase | 16 | 0 | 0 | 0 |

Both release Criterion hit/miss cases succeeded in each phase. All-target check,
local formatting, complete diff and archive replay passed. Named before/after
test outcomes match. The three existing model-download ignores remain explicit.
Open inherited defects `bd-3dkq`, `bd-1izf`, `bd-3umt` and analyst `bd-16eq`
were not rerun or repaired, and no whole-workspace GREEN is claimed.

INTENT 4750, ACTION 4752 and RECORD 4757 match saved requests, live Beads payloads
and successful identical replays. RECORD artifact hashes also match its original
validation commit `09546e21945102a13e88d769a4b7dc4e076558db`. Orchestrator completion
4759 records this attempt. No unreturned worker receipt is claimed here.

Actual POST fingerprint:
`565977759be81ec2f2e3145068a1fb628e7ebd9f4f38d7ff133c0eb395afe1ee`.
The finite model binds 131 source inputs, 72 archives, sixteen actual cases, two
ownership lanes and 54 compiler variables. Jev 1.14 gate is open at weakest 0.82.
`sol_8f83ae0c197f4e2e` is SAT/pass and `sol_9e5493d4cc3a4371` is UNSAT/fail.
Both full receipts are pinned/reloaded. The independent finite oracle verifies
eight predicates and rejects seventeen individual false claims. This is evidence
consistency for the bound cases, not a proof about every execution.

Consumed M03 PRE receipts were fully reloaded, compared and independently checked
before releasing only their pins. Accepted M02 POST pins remain protected. No
quota, solver bound, confidence threshold or product resource limit changed.

Replay frozen ROOT verification:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M03/ROOT-review/verify-post.py
```

`--current-source` additionally checks working bytes while M03 source is current.
The worker's own verifier was replayed at its result tree before adding ROOT
documents; its scope assertion is historical. ROOT replay remains valid after
later caller slices. No successor implementation is part of this acceptance.
