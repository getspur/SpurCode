# ROOT D4 review: repair required before approval

Reviewed normalized worker tip `6189871f88470fd0dd10f49f7471bfd52ca6bdf5`.
The authored history through `d6c64319e` is protected at
`spur/review-evidence/g3-d4-attempt2-authored`.
Canonical `bd-1qn9` remains in progress under plan
`527c672b-ee30-4641-94e9-9c9166f36336`.

ROOT verified the current task state, dispatch/start/completion audits and risk signal.
Selected durable comments 4012–4017 are retained in `beads-audits.json`.
The scoped diff contains exactly five authorized source/test files.
The original assertion RED precedes the implementation in the protected history.
ROOT reran the independent native observer successfully: all 1,682 source bindings matched;
full/no-op/edit/delete/readd/empty payloads and six mutation controls matched the supplied exports.
This checks supplied evidence; it is not fresh native ROOT POST or implementation approval.

The final committed AWS broader suite is red: extractor 53 passed, one failed, three ignored;
Git incremental 26 passed, two failed, one ignored. The baseline restoration packet proves these
three failures already occur at the submitted base. ROOT inspected their causes:

1. A `.rs`-suffixed gitlink is a commit pointer, not physical Rust source bytes.
   The graph manifest must retain its exact `gitlink:<commit>` identity and participate in graph
   content hashes/original inventories. Both full and incremental Rust capture must exclude it
   from syntax coverage and must not attempt a filesystem source read.
2. The old sentinel-anchor test truncates a Rust file after facts extraction and then expects
   authenticated publication of an out-of-bounds owner. The current Rust source validator
   correctly rejects it. Preserve the generic sentinel feature with a non-Rust control where
   supported, and separately assert rejection of this truncated Rust input. Do not weaken
   source/OID/owner authentication to satisfy the old test.

Approval is withheld. The narrow repair may extend `rust_evidence.rs` beyond origin union/pruning
only to make its physical-domain policy agree with the builder's explicit gitlink marker.
Existing mixed symlink/sparse/gitlink and pointer-change assertions must remain intact.
Add native evidence for full/no-op/pointer-change with a real Rust file and `.rs` gitlink,
showing exact manifest preservation, excluded syntax domain, origin reuse and source validation.
Keep true physical missing/IO/OID errors as errors; do not skip paths merely because reads fail.
No storage, schema, query, cache, native inference or Cargo changes are authorized.

## Supplemental PRE

`rework-pre/verify.py` passed at this exact source.
Fingerprint: `af813bd7958ee77eca5b6702ad2c7d2cbf77f2bef432361fe978caf0ceb710d2`;
1,694 bindings. Jev's open candidate preserves exact facts, mode and the exact-boundary binding.
ROOT explicitly added catalog unique/cardinality checks.

| Receipt | Status | Interpretation |
|---|---|---|
| sol_db8b59ff16db461c | SAT | Physical source, manifest-only gitlink and truncated-source rejection contract |
| sol_c367b84283bc464a | UNSAT | Gitlink-as-source and stale-fact publication each fail separate exact-boundary bindings |

The negative receipt contains two separately attributed failing rule bindings; both are
independently checked. All nine finite/lowered predicates and complete persisted models/cores
were checked. Both receipts are pinned and reloaded.
The three-receipt draft hit the existing count cap before obtaining a solve ID.
The final two-receipt encoding preserves both negative boundaries with separate rule bindings.
Only two exact archived/live controls from CLOSED C issue `bd-j62n` were independently checked,
archived and unpinned; six prior predicates were verified.
All eleven D3 POST pins and all seven original D4 PRE pins remain protected.
The real Git commit fixture in `gitlink-input.json` is independently hashed as a Git commit,
never treated as Rust file bytes.

Run before any retry test/source edit:

```sh
python3 -B docs/research/artifacts/2026-11-06-rust-evidence-durability/D4/ROOT-review/rework-pre/verify.py
```

ROOT's attempted fresh replay initially resolved no cloud helper in the temporary checkout and
started a local compile. ROOT stopped its specific Cargo process (exit 130) before any test.
An explicit remote-only retry inside the sandbox could not reach AWS endpoints (exit 200).
The network-authorized remote attempt reached AWS but its upload guard rejected concurrent
documentation changes (exit 1); no tests executed. These are infrastructure outcomes, not a
replacement for the preserved actual native RED.
The checkout has been moved under the repository's `.spur/worktrees` directory so future remote
runs use an isolated remote worktree. Further replay must start from a stable committed state.

D4 remains unmerged. Required broader GREEN, refreshed native exports, ROOT actual-output
POST/Jev and independent review remain gates before D5.
