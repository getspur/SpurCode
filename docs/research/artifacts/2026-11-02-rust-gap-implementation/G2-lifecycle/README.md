# N0 lifecycle investigation (in progress, not accepted)

The vendored `ort` source is the unmodified crates.io `ort` 2.0.0-rc.10
archive, SHA-256 `1fa7e49bd669d32d7bc2a15ec540a527e7764aec722a45467814005725bcd721`,
matching the checksum in the dispatched base `Cargo.lock`. The archive has 76
files; `ort-upstream-files.sha256` records their hashes. Both upstream licenses
are included in `vendor/ort`. Each extracted file and the complete file set were
compared against the archive before any local modifications. The root Cargo
patch selects this source for `ort`; the vendored code is still unmodified and
contains no lifecycle repair. `scripts/spur-cargo tree --offline -p spur-graph
-i ort` confirms the workspace and FastEmbed resolve to the same local package.
Complete offline metadata did not run because the local cache lacks the
unrelated Windows package `clipboard-win`; no full build is claimed here.

The baseline and new subprocess logs are retained here as failures, not
acceptance evidence. The original relocated regression copies a private runtime
module, so the new `ort_lifecycle` subprocess tests exercise the production
initializer instead. Successful inference and query assertions precede the
shutdown abort; they do not establish safe native ownership. The catalog
projections and their complete reloaded receipts are recorded in issue
`bd-3b6f` audits 2387, 2388, and 2390; those finite caller-declared models do
not prove Rust or native lifetime safety. Audit 2393 records outstanding
quiescence concerns. No GREEN or POST is claimed.

With `SPUR_REMOTE=0`, `CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target`,
and `TMPDIR=/private/tmp`, and the update/bless, ORT, DYLD, archive and
`RUSTFLAGS` overrides unset, the following were observed:

- `scripts/spur-cargo test -p spur-graph --test ort_runtime_cache -- --nocapture`
  exited 101: 2 passed, 1 failed; its direct child exited 134. The tested base
  was `752ae520753f379bd0df5854e0f9a9127f2a270f`; the test executable SHA-256
  was `bb6b3c59def51bb3896d997e8493c9acace1ceef5b782258abf58b8017431fde`.
- `scripts/spur-cargo test -p spur-graph --test ort_lifecycle -- --nocapture`
  initially exited 101 with missing production initializer API (compile RED).
  After exposing that initializer, it exited 101 with 2 passed and 6 failed
  at source `3a77f513efd01f7bbcaf2e20950b27b61aa10735`: ORT-only, combined,
  relocated, repeat initialization, failed session, and overlapping session
  owners aborted at shutdown. PGQ-only and the inert child entry passed.
- `scripts/spur-cargo test -p spur-graph --test ort_lifecycle output_value_survives_session_drop_before_normal_exit -- --nocapture`
  exited 101, 0 passed, 1 failed, 8 filtered, at test commit `852fff800`;
  the value remained readable after the session was dropped, then the process
  aborted. Its executable SHA-256 was
  `af96528a8dc29dfc02f100d1c149befde3a8ff8ec9a10e44d9e6894b45f3c59a`.
- `scripts/spur-cargo test -p spur-graph --test ort_shutdown -- --nocapture`
  exited 101 at test commit `9c41c9b25`: this is a compile RED for the proposed
  `try_shutdown` API, not proof of a runtime shutdown failure or success.

The full command environment, raw output, and complete finite PRE requests,
responses and reloads are recorded in issue audits 2389, 2391 and 2392 and in
the neighboring logs. The production-state tests establish observed paths;
they do not trace native release counts or prove thread quiescence.

After selecting the unchanged local wrapper, the full `ort_lifecycle` command
with `--offline` exited 101 at source `1e77b05eeeedc08af93133e38c1bd1dbc9a5b589`:
2 passed and 7 failed. The test executable SHA-256 was
`af96528a8dc29dfc02f100d1c149befde3a8ff8ec9a10e44d9e6894b45f3c59a`.
The current `ort_shutdown` test also covers a value retained after session drop;
its compile RED contains four references to the still-missing `try_shutdown`
API. These results do not indicate an implemented repair.
