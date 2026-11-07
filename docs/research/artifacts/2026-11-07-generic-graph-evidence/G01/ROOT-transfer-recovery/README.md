# G01 ROOT acceptance after transfer recovery

ROOT independently completed the checks missing from worker handoff 4552.
The worker correctly stopped before lease expiry and did not claim compiler
RED or completion. Its 26-file evidence-only result is preserved at
98ad7f45f9b5faf78dfbc629c78b078b4a235c01. No production, test or Cargo bytes
were changed during this verification.

## Actual verification

| Check | Actual result |
| --- | --- |
| Original first-test b543a7fa12ef8a36b8154787fd47c864495b3512 | Compiler RED, exit 101, E0432 unresolved spur_graph::evidence; no tests ran |
| Final generic contract, supplied positive fixture | 18 passed, 0 failed, 0 ignored, 0 filtered |
| query_client::tests | 26 passed, 0 failed, 0 ignored, 566 filtered |
| Graph all-target check | Exit 0 |
| Supplied required-null replay | 1 test passed; native integrity, capability and registry reject cell_type |
| Independent ROOT replay | 1 test passed; 16 cases, 7 integrity rejections, plus registry/filter/atomic-replacement controls |
| Local workspace formatting | Exit 0 |

Each command JSON records the exact source epoch, arguments, environment,
exit and log SHA256. Logs are archived unchanged as gzip; log-archives.json
binds their uncompressed bytes. The failed inline-TOML setup attempt is
retained separately as supplied-positive; it is not compiler or behavior RED.
Using a checked-in Cargo config file delivered the fixture without quote loss.

The ten embedded compiled-source hashes and lengths match the final code at
65cec9a1e2cc314b34d4c1a85d4411a74d2342a4 and the current checkout. Four
production files remain byte-identical to original implementation b2bd69cb6.
Original first-test history precedes that implementation. The later ROOT
and worker commits contain evidence only. Native executable SHA256 is
cc0b73535b5557190516c060d1b45774c59d205a6ff88ee06d9a79107f5b4e2b.

## Independent POST

post-inputs.json binds 77 source paths and actual native request/result
tuples for 19 cases. Fingerprint:
4392d55bd5597143449b029bea64cabc1939d4c67ac9968e0a10c9055ef0da98.

Jev 1.14.0 returned an open gate, weakest confidence 0.78. Two earlier calls
omitted top-level subjects and produced empty bindings; those candidates
were preserved and never executed. The corrected top-level subject contract
selects mutually_consistent. Execution transparently adds the catalog's
provider uniqueness, case uniqueness and exact coverage bindings. Facts
and unknowns are unchanged. All four bindings have nonempty subjects.

| Request | Persisted receipt | Result |
| --- | --- | --- |
| Actual observations | sol_1797a791aad242ee | SAT / pass |
| Altered observations and duplicate provider | sol_79cd295bfba74567 | UNSAT / fail |

Complete persisted envelopes were pinned and reloaded. The independent
finite evaluator checks all eight compiled predicates and rejects each of
14 false controls separately. The receipt store initially reached its
existing 512-artifact limit. Two consumed D8 PRE receipts were completely
reloaded, compared with their archives and independently checked before
their retention pins were released. No solver limit or threshold changed.

Replay from the repository root:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G01/ROOT-transfer-recovery/verify-native.py .
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G01/ROOT-transfer-recovery/verify-post.py
```

This accepts the generic in-memory contract, static registry, inventory
identity and optional unavailable query default. It is finite native
acceptance, not a universal cryptographic proof. Provider codecs, artifact
carrier migration, storage, backend forwarding and cache/runtime identity
remain subsequent stages. No Python implementation substitutes for native
BLAKE3 or graph execution.
