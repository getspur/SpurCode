# G2 local-main integration

All three tasks in plan `c16a0cff-63ff-4abe-83da-07a83b19df64` passed ROOT review. The accepted cumulative tree is `9ab5ed3a23b63d9a0ead1a7ae73ddaabb14c88f4`; it includes the independent Rust notebook, actual source joins and current-source POST in `../ROOT-C-review/`.

The generated integration tip `4c3e90eeea4e29e7bcea755b1c7a6f450da421da` replayed three appended ROOT commits but omitted their inherited implementation/evidence histories. The complete-tree comparison failed with 331 differing paths. ROOT mistakenly proceeded with the local merge before consuming this failure, creating `ec4d883e98c17b2026c9711fe3a5073b10a581c5`.

ROOT repaired this in an isolated checkout by merging the exact approved cumulative history. The repair commit is `31d4b9346ee42706590acaeaa4f6a674e6abef25`. Its complete committed tree is identical to the approved tree. The independent POST verifier passed in the isolated checkout and again after local main fast-forwarded to the repair. No history was reset or rewritten; both the incomplete generated branch and its correction remain reachable.

The POST still binds 348 source/evidence files, eleven receipts (four SAT/seven UNSAT) and 33 independently checked predicates. Review evidence covers 1,334 normal graph cases, 24 named ignores and 27 unchanged CLI cases. Three fresh Rust notebook executions reproduced the 446 tuple, nine foreign-function and three extern-block records; canonical and SQL mutations were rejected/detected.

All five protected dirty user files remain hash-identical and unstaged. No push or publication occurred. The repair changes integration history, not tested production bytes. G2 source tasks and phase are closed; G3 durable artifact/table transport remains separate and continues under the existing authorization.

Run from the integrated checkout:

```bash
python3 -B docs/research/artifacts/2026-11-05-rust-foreign-resume/ROOT-C-review/verify_post.py
```

Physical source correspondence does not establish Cargo cfg selection, macro expansion, compiler dispatch, resolved linkage or complete panic detection.
