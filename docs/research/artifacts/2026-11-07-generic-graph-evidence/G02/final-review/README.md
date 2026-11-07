# G02 ROOT final acceptance

Reviewed source: 976d138e342d8449e1baa27de396af0d933530e2.
Canonical bd-ode3; plan 73be9add-0d91-4416-a3bf-a3dbdb5e4810.
Both original ROOT regressions pass, with their function bodies unchanged.
The repair removes one overbroad path exclusion and separates query projection
decoding from full stored-capture normalization. Full stored integrity, exact
reconstruction and saved-source validators remain unchanged. Golden expectations
adjust only the natively measured current manifest-version paths; frozen
historical artifacts are unchanged.

## Verification

| Check | Result |
| --- | --- |
| Worker required four suites | 57 passed; 0 failed/ignored/filtered |
| Worker all-target check, fmt, complete diff | Exit 0 |
| Worker native retry | 13 passed, 3 providers, 11 original mutation cases |
| Independent ROOT remote provider suite | 13 passed; 0 failed/ignored/filtered |
| Independent ROOT supplied cases | 9 cases; all expected boundaries verified |
| Native compiled-source bindings | 25 match reviewed Git bytes |
| ROOT POST source/archive bindings | 84 |

The independent cases cover an original positive, consistently rehashed macro
omission, Rust source-group omission, missing stored length/origin, rehashed
scalar/payload mismatch, changed source bytes and opaque provider integrity.
Rehashed macro omission passes stored integrity but fails source recollection;
Rust source-group omission and scalar mismatch fail typed stored validation.
Unavailable query provenance is accepted by the preserved ROOT test while
missing provenance in a complete stored capture is still rejected.

ROOT used package-local copies of its exact archived request/config so the
provider-only remote command could omit unrelated root documents. No omitted
file was a compile/runtime input to that target. Both owned temporary files
were SHA-checked and removed after completion. The raw native record and full
command log are preserved unchanged; parsed native.json.gz is also retained.
No Python BLAKE3, local fallback or production edits were used in this review.

The worker's earlier native exit101 lacks an assertion summary; concurrent
ENOSPC and queue errors were observed, but its cause remains unconfirmed.
That failure stays preserved. The unchanged worker retry and this independent
ROOT run pass; no software repair or infrastructure cause is inferred from it.
The completed worker's lease-timing risk is recorded rather than suppressed.

## Actual-output POST

Fingerprint:
eb34ebec6766676cfdb6fdf0c14bce0ee17f2c6966a0a418b0d6b777627f7f8c.

The model joins 18 actual result groups against independently declared tuples:
9 ROOT cases, 3 provider roundtrips, 3 ROOT regression tests, 11 preserved
mutation boundaries, current-cohort golden tests and archive identity.
63 compiler variables fit the existing 64-variable limit.

| Request | Pinned/reloaded receipt | Result |
| --- | --- | --- |
| Actual outcomes | sol_34ded000eb5d46bb | SAT/pass |
| Altered outcomes and provider alias | sol_6b1c45c1d1e04d61 | UNSAT/fail |

Jev initially blocked on data completeness (0.53). ROOT supplied newly measured
run/source counts and an explicit check that every active enum cell was
present and declared. The expanded evidence request exceeded Jev's token
limit; that transport failure is retained. A compact summary of the same
evidence opened the gate (jev-1.14.0, weakest0.75). Exact facts and thresholds
were unchanged. Execution adds catalog case/provider uniqueness and exact
coverage to the selected mutually_consistent binding; no vacuous subjects.

An independent evaluator checks 8 compiled predicates and rejects all
19 false controls separately. Complete prior G01 POST envelopes were reloaded,
compared and independently checked (8 predicates) before releasing their
consumed-phase retention pins; full archives remain. Current G02 repair PRE
and POST receipts are retained. No solver quota or limit changed.

Replay:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/final-review/verify-post.py --current-source
```

After later implementation, omit --current-source to authenticate this frozen
epoch. This accepts in-memory provider codecs and typed adapters. It does not
claim generic artifact cutover, disk IO, backend forwarding or cache/runtime
integration; those remain later stages.
