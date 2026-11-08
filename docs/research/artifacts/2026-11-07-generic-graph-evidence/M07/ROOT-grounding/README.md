# ROOT review of M07 grounding

ROOT accepts grounding commit `e731cf826dbd92b5cc8ee502348310048b392895`.
This is documentation approval only. No M07 product migration, native execution,
test result or solver proof is supplied by the grounding.

Canonical `bd-1xja` was in progress with no unresolved signals. INTENT 5049,
RECORD 5050 and completion 5053 are confirmed in the independent readback embedded
in `verification.json`. Exactly three worker documents differ from clean base
`c67c0775738b2bb79d5ce225090f01a1edff99ee`; their complete hashes match RECORD.

ROOT independently checked all 1,179 source/reference files (21,471,019 bytes)
against base Git blobs and the clean isolated source tree, both inventory/PRE
fingerprints, the complete source-map binding, all source snippets and line/byte
ranges, ownership, policy/operation/coverage references and null native fields.
An independent Rust token masker also found exactly the mapped 86 identifiers:
61 migrate and 25 preserve. The 49 operations, eight call adaptations, nineteen
lanes, 32 selected test symbols, 124 source-declared available tests, 84 prospective
labels, 441 operation/dimension cells, thirteen policy rows, twenty proposed
controls and 229 literal include edges check out. These are static counts.

The worker's 562 range count is the text-bearing subset. ROOT additionally
verified 49 text-free PRE operation ranges and three whole-file ranges: 614
structural range occurrences in total. No source mismatch was found.

The 1,179 files are a conservative local-package/reference superset, as the worker
states. They are not actual rustc inputs or proof of executed code. Actual native
compiler, generated/registry/dependency, executable and remote identities still
require acquisition. Main's existing user edits are excluded by the dispatched
clean commit. In particular this broad inventory includes ACP files with user
edits on main; verify it in a clean checkout of the bound source epoch, rather
than treating those preserved user edits as a grounding defect or tested input.

## ROOT choices for the next PRE

`decision.json` accepts the mechanical proposals inside the three owned files.
Keep the private version predicate pure over `Option<&RustEvidenceSet>`; its two
existing Result callers decode at the original evaluation positions. Preserve
all short circuits and the predicate's comparisons. Do not turn getter errors
into false/default. Make the bucket helper fallible, bind its owned Rust/macro
Options once, preserve the complete bucket/origin construction, and return
`Ok(buckets)`. Four production callers propagate and two existing tests unwrap.
All eight callers are explicitly mapped; independent source counts found six
bucket calls and two predicate calls plus each declaration.

The bucket helper's private return type changes. Its actual before map and after
Result must both be retained. The next PRE must explicitly relate the whole
before map to the after Ok value; any Err rejects that relation. Caller-level
Results, including complete errors and order, remain strict. This is a declared
source-signature adaptation, not permission for general error normalization.

Preserve `GraphArtifactBodyForHash`, borrowed field shapes/order, None omission,
native Serde/error context and the old golden. Owned getter bindings must remain
alive through serialization. Preserve FileBucket's private per-file fields,
original origin Arc sharing, GraphFacts lanes, cache key members, raw wire negative
controls, parsed-manifest identity, hot/cold behavior and publication order.

Before a full baseline or the larger migration, use a bounded native capture
pilot to establish real compiler/source/executable receipts, raw frame/reference
coverage and attribution in all three library owners and a genuine integration
route. Integration builds do not use library `cfg(test)`; an observer that reaches
only unit tests is insufficient. `sha2` is already a normal dependency, so no
feature/dependency change is needed. FileBucket already derives Debug but has no
declared Serde contract: distinguish observer field encoding from an original
native Serde value. Keep exact native Debug and all fields.

The next fresh Jev/catalog PRE owns temporary capture writes, exact pilot routes,
source/compiler receipts, logical signature mapping, restoration, observed-case
cardinality and test gates. Native measurements stay null until executed. Full
acquisition and migration remain gated on ROOT's pilot review. Strict comparison
is the default; no M05/M06 ordering/error qualification is inherited.

All reported coverage limits remain: helper/hash bypasses, fallback short
circuits, opt-in early returns, incomplete old exports, missing native macro
publication mismatch, multi-cause reorder and serialization-error fixtures.
Existing fixtures/assertions are not repaired to invent coverage. No actual test
count or site multiplicity is inferred from the 84 planning labels.

## Replay

In a clean checkout of the bound M07 grounding epoch:

```sh
python3 -B docs/research/artifacts/2026-11-07-generic-graph-evidence/M07/ROOT-grounding/verify.py
```

This reads source/Git/Beads and verifies the saved review. It executes no Cargo,
native acquisition or solver. ROOT owns approval, ordinary local integration and
the fresh PRE before any implementation. Existing main user paths, old evidence
and protected M06/M04 solver receipts are preserved.
