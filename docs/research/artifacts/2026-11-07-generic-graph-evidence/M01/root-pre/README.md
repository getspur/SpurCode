# M01 ROOT prospective PRE

Accepted G03/local main: `c1e11019ab6b0eb32d062c30f17ac8d970fcd89d`.
G03 source `bd-36nv` and ROOT review `bd-1cz3` are closed. This fresh PRE is
ROOT-owned `bd-2ulk` under generic architecture parent `bd-2rg4`.

This batch prepares callers for the later generic carrier cutover. It has no
intended semantic change. The PRE encodes prospective conservation obligations;
the worker still must execute existing native fixtures before and after migration
and preserve their values/bytes. A fresh actual ROOT POST/review follows.

## Exact caller ownership

- `crates/spur-analyst/src/overlay/session_tests.rs`: complete_manifest_json
  manifest/count constructors and artifact_with_file artifact constructor.
- `crates/spur-cli/tests/analyst_temporal_views.rs`: structural_artifact.
- `crates/spur-core/src/server/handlers/rebuild_singleflight.rs`: tests::artifact.

There are five construction sites: three artifacts, one manifest and one nested
row-count literal. Their typed evidence and optional macro counts are currently
None. Remove named typed fields in favor of G03 draft/metadata/accessor APIs;
preserve every explicit original field, including test-specific graph/schema
versions, hash strings, complete=true, IDs, node IDs and row counts. The G03
empty constructor's current-version defaults must never overwrite explicit old
fixture values. Use owned typed getters for reads and decode/mutate/setter for
mutations when present. Keep GraphFacts fields and legacy wire negative inputs.

Only those three source/test paths and new M01/worker evidence may be written.
No G04/M02 work, generic artifact/manifest carrier, schema/wire/collector/cache
change, new dependency/feature/capacity, or unrelated bug repair. Unchanged
existing assertions are the refactor's test-first foundation. Do not invent a
runtime RED or add implementation-mirror tests. Temporary or retained test-only
native observation may expose existing fixture values without weakening their
assertions; distinguish its compiled identity from the actual migration.

## Required execution and current baseline

Use scripts/spur-cargo for every Cargo action; remote default and no local
fallback after a remote failure. Local fmt. Execute the existing affected suites
before/after: analyst overlay, CLI analyst_temporal_views, and the core
rebuild_singleflight unit filter as needed for the real core fixture export.
Final all-target check covers spur-graph plus the three touched packages. Preserve
all existing negative/None/Some(empty) controls; G03 provider tests cover the
stable API presence/owned-clone contract. Record all passed/failed/ignored/filtered
counts and exact commands/source/executable/compiler/config identities.

Historical `bd-16eq` tracks
worktree_overlay_keeps_base_edges_to_reemitted_delta_targets. Its tracker snapshot
is retained, and the current test source is bound. An old report does not prove
the current status. Run the unmodified test before/after; if still RED, retain the
exact failures and unchanged-test/production identities for ROOT risk review.
Do not skip it, claim it fixed, or repair it outside this migration scope.

Actual native observations must include the existing analyst manifest/count and
artifact helpers, CLI structural artifact and core fixture, their serialized
bytes/value and native canonical/capability identities before/after. Test-specific
labels and header versions are part of conservation. Python only joins/hashes
archives; it must not reimplement graph hashes or supply expected passing output.

## Constraint request and replay

Fingerprint `27b31490e5d9fa2a2ec7d497db9b8fa59635ca374e9d5e1a46e58ab417ed5653`:
93 frozen source/archive bindings, five named sites, eight prospective cases and
six distinct ownership/count lanes (42 compiler variables). Jev selected verify /
mutually_consistent on contract, gate open at weakest0.86. ROOT inspected the
candidate and added catalog case/lane uniqueness and exact coverage explicitly.

Persisted and pinned: `sol_34ed94966ab34923` SAT/pass and
`sol_08ffe9df48744921` UNSAT/fail. Complete reloads and independent finite oracle
evaluation check eight compiler predicates. Each of nine false controls is also
rejected separately. This is a finite policy check, not universal code proof.

ROOT freed quota by reloading consumed G02 POST receipts, comparing full archived
envelopes, and independently validating eight predicates before releasing only
those two pins. Full reloads are archived here; the superseding accepted G03 POST
pair remains protected. Quotas/thresholds were unchanged.

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M01/root-pre/verify.py --current-source
```

After source edits omit --current-source to authenticate the frozen epoch;
POST must bind the actual new compiled source. Read-only graph orientation was
stale (792c0e6a versus exact50a57726), so ROOT used the exact named working-tree
source sites and did not mutate external graph/CURRENT.
