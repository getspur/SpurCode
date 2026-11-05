# M3 independent root review

Reviewed `bd-2fkd`, delegation `1d775afd778c44e8`, against entry
`8e655269b99cebaa4b1e192bfd10d8e32ffcd9de`. Normalized worker tip
`4fdeb18b0490d1b3159078dd8437effa54ac9808` has exactly the authored evidence
tree `1a2354d8748df4161db14c8521bc458c8775de0c`. Test commits `9980f669d`
and `6c99d0a9a`, fix `5ae93bf9c`, authored evidence and normalized history are
protected by `spur/review-evidence/embedding-sol-m3-*` refs. The actual resolved
worker was codex / gpt-6.1-sol / xhigh. Its INTENT replay, completion audit,
in-progress issue and absence of unresolved signals were checked before review.

The seven-file source/test diff adds immutable helper command/cache discovery,
strict bounded compatibility metadata validation, and shared graph policy.
Complete legacy JSON retains program/prefix and precedence. Newly selected
helpers execute directly by absolute OS path, with no shell/PATH search or
runtime compilation/download. Descriptor and compatibility share the effective
startup deadline. Child cache configuration does not mutate the parent.
Native/model/analyst ownership sources remain unchanged. The stale graph index
did not contain the new discovery symbols; review used exact committed bytes.

The worker's `M3/verify.py` passed **before shared-target rebuilding**, checking
51 source hashes, preserved private binary/model/runtime hashes, 26 exported
cases and recorded results. Historical raw logs were retained. The independent
fresh commands are in `run_checks.py`; exact arguments, selected environment,
exit status, timestamps and hashed compressed streams are retained per stage.
All builds/tests used `scripts/spur-cargo`, local authorized execution, one
Cargo writer and the approved native archive. No remote failure was retried.

| Fresh check | Result |
|---|---|
| Graph discovery, compatibility and cancellation | 12 passed, none ignored |
| Neutral client/protocol/policy suite | 47 passed, none ignored |
| Analyst Auto/Sidecar mode regression | 1 passed, 28 unrelated cases filtered |
| Graph/client all-target compilation and scoped formatting | exit 0 |
| Separately built ordinary worker | no test features, non-test build |
| Real private-layout graph and neutral Nomic calls | 1 test passed; two helper lifecycles |

The fresh real helper SHA256 is
`8cf2858c2434b47dadcf98ef53d3c197e791f821f25993b8b48e07bbe6bd7e6c`.
Both paths returned identical finite 1×768 vectors, with checked shutdown and
raw wait status 0. Native cache bytes match
`2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`.
Source model files were hash-checked before/after use of private copies.
`normal-observation.json` and `normal-actual.json` retain the precise identities,
commands, vectors and process observations. This is macOS ARM64 evidence;
Linux non-Unicode execution remains explicitly untested. These measurements
are observations, not latency guarantees or broad native safety claims.

Root Jev POST gates opened at weakest confidence .98 for integrity and resource
bindings. The inspected requests exactly retain facts, mode and selected rules;
uniqueness and cardinality were added from the inspected catalog. Eight fresh
persisted/reloaded receipts contain **5 SAT and 3 expected UNSAT** results:
`sol_458c3d36af9c4900`, `sol_f1af74bb775d415d`, `sol_67fdd85599704ec3`,
`sol_68cd085408004372`, `sol_bf73845758bc4929`,
`sol_fad897ed0b7b4107`, `sol_13b9b743f3b24206`, `sol_a84895125d304943`.
The negative controls forge protocol acceptance, omit coverage and break the
normal run's source/binary binding. `verify.py` independently evaluates all
22 compound catalog predicates and verifies the generic variable domains,
fixed assignments, models, rule decisions and numeric comparison pairs.
The solver checks finite supplied observations, not arbitrary subprocesses.

Thirteen completed M2 PRE/POST receipts were exactly compared with immutable
committed archives before retention was released, to use the existing quota.
Full comparisons and retention responses are compressed in
`retirement-precheck.json.gz` and `retirement-operations.json.gz`. Current M3
PRE and new M3 POST/M4 PRE receipts remain pinned. No quota or solver files were
modified directly. Preparation diagnostics record a projection cwd correction;
the underlying Rust regression passed throughout.

Run `python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M3-review/verify.py`
before downstream source changes. [M4 PRE](../M4-pre/README.md) is the successor
entry gate. Batching implementation, shipped packaging, three-model final
acceptance and the fresh Rust notebook remain pending. Local main, its five
tracked user edits, old/live notebooks and external notebook source were preserved.
