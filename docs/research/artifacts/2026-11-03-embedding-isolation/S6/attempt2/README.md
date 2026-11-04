# S6 retry checkpoint — incomplete

The protected three-commit chain was replayed as `44dc8e066`, `441d34449`, `83339e4dc`; `69e9aa80c` imports only the recovery evidence. The S5 verifier exited 0 before production edits and verified source hashes, four SAT and twelve UNSAT PRE results. The new ORT archive hashes to `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`.

Test `3fc4b7ee3` requires warm-up to open an actual supervised helper session; its result was not yet observed at this checkpoint. `8b17de920` is an **intermediate, unapproved** type/dependency adaptation, not a completed fix: it still retains global handles and detached jobs, and warm-up only describes a model without opening a session. Cancellation, joining, teardown, error paths and real helper inference remain outstanding. Do not merge or claim S6 GREEN on this basis.

The four captured `scripts/spur-cargo tree -p PACKAGE --edges normal,build --prefix none` commands used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target CARGO_NET_OFFLINE=true`. They exited 0 and their normal/build trees contain no package named `fastembed`, `ort`, `ort-sys`, `owner-test-hooks`, or `test-support`; `dependency-results.json` records hashes and exact commands. This is dependency evidence only, not a runtime or feature-tree acceptance result.

A cold full test build was launched with the authorized archive and single target; its outcome must be recorded separately. The first attempt's interrupted exit 130 and assertion RED exit 101 remain in `../worker/README.md`.
