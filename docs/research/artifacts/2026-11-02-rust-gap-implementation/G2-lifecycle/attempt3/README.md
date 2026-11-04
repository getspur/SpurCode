# N0 attempt 3 — work in progress, not accepted

Binding audit 2434 and PRE audit 2436 are on bd-3b6f. PRE receipt
`sol_18edbcb0d2394660` was persisted, pinned and reloaded before edits. It
checks a three-transition finite trace, not native or Rust memory safety.
The issue remains in progress. Earlier attempt refs and the 76-file upstream
hash ledger are retained.

Two native subprocess regressions were recorded before the associated changes:
`session_allocator_outlives_its_handle` and
`session_internal_allocator_outlives_session` each failed with SIGSEGV (exit
101 from the test command), then passed. Managed async's conversion sentinel
also failed before its scoped rejection was added. The `*-red.log` and
`*-green.log` files contain raw output. The weak session backing in the second
case is private; upgrading it requires a live allocator reference. The
separately constructed allocator is retained by the tensor and retains its
session until native allocator release. This does not constitute a complete
wrapper ownership audit.

`final-focused.log`: local `scripts/spur-cargo test -p spur-graph --test
ort_shutdown -- --nocapture`, exit 0, 14 entries: seven subprocess drivers
and seven child entries. Inference output 1,4,9,16,25,36 and PGQ result b were
printed. The CUDA allocator setup failure returned `No requested allocator
available` and checked shutdown succeeded; no native release count was
observed. Test executable SHA-256
`3416aea7c809b6c18a90f6aa1f95aec566f6e6c416f0641d8bb3fb36c026f0bc`.
The tracked source diff against the three test commits at that run SHA-256
was `40997c63bdadcbbc7b848d5bbaa5fad8628cb6af1889878efc2e83857a837f6e`.
All runs used `SPUR_REMOTE=0`, `TMPDIR=/private/tmp`, the acceptance target,
and removed the relevant ORT/DYLD/RUSTFLAGS/update/bless variables.

`final-all-targets.log` and `final-nodefault.log` record successful graph
all-target checks, with warnings. The no-default prior failure was in unchanged
baseline test references; feature guards now scope those assertions. The
isolated wrapper feature-matrix logs show managed+training exit 101 with the
specific compile error and training without managed exit 0. The isolated copy
uses the workspace lock and a separate target. Normal automatic exit is N1.

Outstanding: complete supported-resource/source audit and release ordering,
real native release counts for post-create finalization failure, full matrix
and final checks bound to a committed implementation, complete source-bound
POST plus negative controls, and a durable completion audit. No complete owner
API or G2 safety claim is made. `git diff --check HEAD` also reports existing
trailing whitespace in the preserved upstream README and inherited raw baseline
log; neither was edited to change historical evidence.
