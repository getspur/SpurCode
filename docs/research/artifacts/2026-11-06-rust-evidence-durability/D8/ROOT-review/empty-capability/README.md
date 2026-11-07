# D8 ROOT review: absent empty-domain capability

Approval is held for a focused repair. D8's normalized output `91c6a043c`
is byte-identical to authored tip `1cdbd2d4f`; the original history is protected
by `spur/review-evidence/g3-d8-attempt3-authored`.

The worker packet verifies 2,541 source bindings, 17 cases, 423 observations
and 12 recorded replays. ROOT rebuilt the unchanged source with the prescribed
remote wrapper on `i-083233d863b806894`; all 12 overlay tests passed. A separate
fresh capture also passed those 12 tests and recorded executable SHA-256
`7d272505884f0a87ab0c9c665ed57f384b5b0a19e68fa73ea054979e4650c366`.
The prior worker VM was not connected, so its failed ROOT transport attempt
was not treated as a test result.

ROOT supplied 15 independent requests to the fresh executable. They include
duplicated old callsites in shadowed zero and cfg groups, legacy and custom
backends, physical deletion, readdition and declaration uncertainty. Across
the non-counterexample requests, 300 complete query outputs matched the
independently selected original groups or the explicitly supplied custom
backend projection. Custom projections are trusted backend inputs, not stored
artifacts; their raw delegation behavior is recorded separately.

The additional empty-domain cases expose an availability upgrade:

| Input | Observed | Required |
| --- | --- | --- |
| Both carriers absent, identity overlay | Unavailable | Unavailable |
| Both carriers absent, only `README.md` shadowed | Complete | Unavailable |
| Captured empty carriers, only `README.md` shadowed | Complete | Complete |

All three results have empty known Rust domains, file groups and origins.
The second case introduces neither Rust source nor a captured capability.
The empty-domain `all(Complete)` branch currently upgrades missing evidence
through vacuous truth. Preserve the established distinction between absent
legacy evidence and captured empty coverage while retaining D8's deletion,
legacy replacement and Unknown semantics.

The catalog was navigated before the regression. Jev opened the fixed
`data_integrity.mutually_consistent` verification request; ROOT added exact
cardinality and key uniqueness bindings. Required outcomes are SAT/pass
(`sol_b33fce6d9f5b4d6f`); the actual upgrade is UNSAT/fail
(`sol_4e43fd28124942d0`). All six family and lowered predicates were independently
re-evaluated. This is a finite repair PRE and counterexample, not a successful
whole-D8 POST. Its fingerprint is
`240cc8548007f0f46214d457fca854e556a5d1e45db18f4be7cdbb3826c5070b`.

Regression commit `ac713352e75f71936a157ac52f5ccd4921cbf9e5` adds two public-API
tests without production changes. The remote run compiled and exited 101:
one control passed and the absent-capability assertion failed, with source
hashes and Git revision unchanged. Keep both tests and all original D8 tests.

Full input archives, runtime logs, compiler/executable identities, requests,
raw results and complete solver receipts are preserved here. The five closed
D7B receipts were reloaded exactly and independently checked (15 predicates)
before releasing their pins for the fresh repair receipts. Their complete
copies and retention operations are archived; do not require retired IDs live.
The original D8 PRE pins are unchanged.

The worker's final completion-audit request remains unconfirmed after tool
timeouts. Orchestrator completion comment 4324 is independently confirmed.
This distinction is preserved; ROOT has not replayed or forged a worker audit.
The known macro fixture failure `bd-xb5p` remains separate from this repair.

Run `python3 .../empty-capability/verify.py --current-source` before repair.
After source changes, omit `--current-source` to verify the immutable historical
evidence only. A fresh ROOT POST is still required against the repaired source.
