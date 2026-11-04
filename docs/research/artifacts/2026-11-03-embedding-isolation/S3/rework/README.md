# S3 startup rework — GREEN, root Jev POST review pending

The worker reads and validates Hello before native initialization so a failed initialization can return a correlated, byte-bounded Unavailable frame. Existing checked native cleanup and product Unavailable behavior remain in place.

Commits: carry-forward `e387d33a5`, regression tests `9fb799554` (supplied test commit `e1ed254750484376ba450886f16d6c834c600d72`), fix `7b1a825a4`. `implementation.diff` is the worker/workspace diff from S2 candidate `251a1fb0627ed10849077a7424648cee0c8ae32c`.

All commands below used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target` and `SPUR_ORT_ARCHIVE=/private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz`. Cargo commands used `scripts/spur-cargo`; client tests had process-inspection escalation. Logs preserve warnings and output bytes, gzip-compressed.

| Command | Result | Log |
|---|---|---|
| `scripts/spur-cargo test -p spur-embedding-worker --features test-support --test native_lifecycle root_initialization_failure -- --nocapture` | RED exit 101, 0 passed, 2 assertion failures, both EOF, raw status 256 | red.log.gz |
| `scripts/spur-cargo test -p spur-embedding-worker --features test-support -- --nocapture` | GREEN exit 0, 6 integration tests passed | green.log.gz |
| `scripts/spur-cargo build -p spur-embedding-worker` | exit 0, normal helper | build.log.gz |
| `scripts/spur-cargo test -p spur-embedding --features test-support` | exit 0, 32 tests passed | client-full.log.gz |
| `scripts/spur-cargo tree -p spur-embedding-worker -e normal,build` and `tree -p spur-embedding -e normal,build` | worker tree excludes graph, analyst, DuckDB/PGQ, test-support and owner hooks; client tree excludes ORT | worker-tree.log.gz, client-tree.log.gz |
| `python3 rework/observe.py` and `python3 rework/startup_wire.py` (full paths from repository root) | exit 0; native observations and normal-helper wire frames recorded | observe.log.gz, wire.log.gz |

`startup-wire.json` preserves request and reply bytes and actual `waitpid` statuses. For limits 1 and 256 the response is version 2, generation `d32a3002-a3aa-4b1c-9ae7-ff0824970841`, request 1, `Unavailable`, with messages `n` and `native initialization failed`, and raw status 256. `native-observations.json` records repeated real fixture outputs, checked Busy followed by successful release, active and idle/repeated close, signal control, and source, executable and runtime hashes. A signal-killed child is not a clean exit. Four owner counters were not independently measured; these observations do not prove all wrapper paths or unbounded liveness.

Root's immutable PRE archive and root-review archive have the supplied SHA256 values. All four source bindings in the original PRE were independently hashed from candidate `251a1fb...` and matched. All 13 supplied solver IDs were reloaded via `get_solve_result` and their complete lowered requests and statuses compared equal to the archives: original eight (2 SAT, 6 UNSAT), startup PRE (1 SAT, 1 UNSAT), and historical failed POST (3 UNSAT). These last three are failed acceptance checks, not successful controls. No local Jev compilation was claimed. `post-candidates.json` supplies independent policy, exact observation-bound input candidates and negative controls; fresh root Jev POST remains required before approval. The issue remains in progress.
