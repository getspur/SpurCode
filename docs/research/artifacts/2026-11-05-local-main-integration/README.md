# Local main integration — bd-cmr7

User explicitly authorized merging the reviewed recovery into local main, then continuing packaging.

The merge combines local main3c79173e45a6cf40c56e9f2f2cc2ff9754006f14 with approved recoverydb9592231277d7fecf0f6ecad62641119f721c68. Both sides changed disjoint paths since their merge base; Git produced no conflicts or manual source resolutions. Main's51 independent commits, including TUI work and xtask, are retained. The root review packet fromc1596d8c0d8cd4712ce8c5d3e613607579703017 is included without changing its frozen identities.

Verification in an isolated checkout:
- CLI and TUI all-target check passed; existing warnings remain.
- Client feature-enabled lifecycle/protocol suite:40 passed.
- Unchanged CLI analyst_build_emits_external_dependency_surface_views:1 passed, retaining its original five-row assertion.
- Workspace formatting and production-source diff checks passed.
- Full diff check reports only preexisting vendor/ort/README.md trailing whitespace at line36; upstream bytes are preserved.

Of the40 packaging source/grounding files, only xtask/src/main.rs differs from the frozen recovery packet because main has independently reviewed TUI tooling changes. source-binding.json records the new hash map and fingerprint. Old packet verifiers remain frozen to their original source and binary identities; they must not be relabeled as current-main executions.

Before publication, five dirty tracked files and986 matching incoming/untracked files were backed up byte-for-byte (991 files,94,248,844 bytes) under /private/tmp/20261105-local-main-recovery. The backup also preserves original status, patches and untracked list. Original dirty ACP and notebook work is excluded from this merge commit. Matching incoming artifacts may become tracked as part of the approved history while retaining their bytes.

The unfinished packaging checkpoint3406dc24a975f3bcc528da4c3ee34dbb682e07c7 is protected at spur/review-evidence/embedding-packaging-attempt1-partial. Its xtask work is not part of this approved-source merge. Packaging's old plan is blocked replaying already-integrated overlays; remaining work will restart directly from committed merged main, with TDD, fresh source-bound PRE and root POST retained. No remote publication is authorized by this local merge.
