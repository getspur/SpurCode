# S3 root PRE: scoped native worker lifecycle

The source candidate is `251a1fb0627ed10849077a7424648cee0c8ae32c`.
This prospective PRE accompanies the independently approved S2 supervisor;
it is not native inference or native teardown acceptance.

Two fresh Jev gates opened and their exact bindings/facts were inspected:
`workflow.transition_allowed` for the six-step close sequence, and
`resource.request_within_limit` for zero outstanding jobs, sessions, inputs
and outputs at checked environment release. Root explicitly added the same
workflow binding for the separately declared idle-close trace. All inputs
are complete, with no synthesis unknowns.

The desired lifecycle and zero-owner gate passed; four retained-owner
controls, release before dropping values/sessions, and skipping admission
closure were rejected. All eight results were persisted and reloaded.
`root-pre.json` records IDs/checksums; `root-pre.full.json.gz` preserves exact
Jev provenance, original requests, results and complete lowered reloads.

The pinned official ORT 1.22.0 bundle/checksums and source-bound real
`mul_1.onnx` fixture are recorded from the existing candidate. Preserve
materialization/checksum behavior while moving ownership into the worker.
The normal helper must be free of graph/analyst/DuckDB dependencies, and
the neutral client remains free of ORT. Use only the native inference API
surface actually exercised; do not restart the general ownership audit.

Test real fixture inference and ordinary native shutdown in subprocesses,
including retained output blocking premature release, initialization
failure, idle/repeated/active close and real signal termination. Record
actual native API outcomes, outputs, source/runtime/model/executable hashes
and raw wait statuses for POST. A Rust boolean or a printed result alone
does not establish native release. A signal-killed process is containment,
not a clean native exit.

Product model loading belongs to S4. Do not label tiny-fixture vectors as
production model embeddings. A test-support-only native probe may exercise
the same runtime owner; normal production Embed must return typed
Unavailable until the model implementation exists. Keep test support out
of normal helper builds. Main returns normally after real cleanup; do not
swallow aborts or use forced successful exit as cleanup.

These finite models cover declared order and counters only. They do not
prove C API lifetime correctness, all wrapper paths, unbounded liveness or
real-time OS scheduling. Numeric product defaults remain unselected.
Worker-local Jev absence is covered by the authorized root bridge: reload
the eight receipts, proceed through TDD, and provide exact actual-output
POST candidates to root after GREEN.
