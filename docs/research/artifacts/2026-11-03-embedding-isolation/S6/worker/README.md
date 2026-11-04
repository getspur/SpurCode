# S6 preflight and dependency RED

Canonical task `bd-3cu4`, delegation `84aee00079894d0f`. This is an incomplete handoff, not a GREEN or approval.

HEAD at dispatch `38f6838c07770dd5f1631a3a363896dff94ef7e0`: graph, embedding client, embedding worker, workspace Cargo manifest and lockfile match candidate `c06d12b2489c8f49af1c0be1f3d28a9e1428b64a` (`git diff --name-status` empty for those paths). Candidate is not an ancestor of dispatch HEAD. Root artifacts were imported without source changes in `7ba67e828`. The S5 verifier exited 0, reported S5 POST 3 SAT / 18 UNSAT, S6 PRE 4 SAT / 12 UNSAT, eight baseline failures retained and source hashes verified. All sixteen specified persisted receipts were reloaded via `get_solve_result`; each saved lowered request, status and model compared equal to the root PRE archive.

Test commit `060ff9393` adds the default product dependency closure assertion. The full command

```
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target CARGO_NET_OFFLINE=true SPUR_ORT_ARCHIVE=/private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz scripts/spur-cargo test -p spur-analyst --test embedding dependency_boundary -- --nocapture
```

was interrupted during a cold dependency rebuild (exit 130); no test ran. The target directory and pinned archive specified in the handoff were absent at the start of this attempt. A copy of the committed test body in `/private/tmp/spur-s6-dependency-red/tests/boundary.rs`, changing only `env!("CARGO_MANIFEST_DIR")` to the analyst crate's absolute path for the standalone harness, was run with:

```
SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target CARGO_NET_OFFLINE=true scripts/spur-cargo test --manifest-path /private/tmp/spur-s6-dependency-red/Cargo.toml --test boundary -- --nocapture
```

It exited 101 with an assertion failure: `spur-analyst default product closure contains fastembed v5.8.0`. This is an actual dependency-tree assertion RED, not a runtime embedding result.

The accompanying raw trees were generated with `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target CARGO_NET_OFFLINE=true scripts/spur-cargo tree -p PACKAGE --edges normal,build --prefix none`. All four commands exited 0. `spur-graph` contains none of `fastembed`, `ort`, or `ort-sys`; `spur-analyst`, `spur-core`, and `spur-cli` each contain all three. No implementation, real helper process, socket translation, cancellation test, or POST observation was produced. The named runtime dylib exists in an S4C review cache, but the pinned archive was not found or substituted. Missing archive risk is recorded on the issue.
