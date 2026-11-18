# Changed files in the tracked repository context

[Findings](../../2026-11-18-pr28731-change-context.md) ·
[Live notebook](/Users/kevintruong/project/spur-notebook/2026-11-17-cpp-extraction-gap-lab.ipynb)

The current notebook section maps the PR's five changed files and five changed
C++ function seeds into unchanged code, shared metadata contracts, build
components, tests and configuration. The complete tracked-file catalog is the
context universe; this is a PR-specific evidence map, not a claim of complete
semantic relationships for every file.

- `tracked-files.json.gz`: all 18,342 tracked head entries, body hashes, change
  flags and base/head unchanged-blob verification. Binary blobs retain metadata.
- `relationships.json`: typed relationships and pinned source/blob witnesses.
- `seed-node-mapping.json`, `native-incoming-paths.json`: changed symbol seeds
  and retained native two-call-hop paths into them. Numeric node IDs retain an
  explicit graph identity; do not join them to IDs from another graph.
- `unchanged-forwarder-identities.json`: before/head body hashes for the two
  unchanged wrapper bodies inside the changed statistics source file.
- `effective-test-selection.json`: parsed skip-list inheritance, including the
  two unchanged inheriting configurations. These are selection declarations,
  not executed test coverage.
- `source-references.json`, `callback-registrations.json`: exact reference and
  callback source locations. Candidates are not promoted to direct bound calls.
- `declared-components.json`: CMake-declared object targets for changed files
  and relevant unchanged code. Conditional build configuration is not executed.
- `code-context.svg`, `configuration-context.svg`: review diagrams. Amber marks
  changed functions/files; dashed code edges are context/candidate evidence.
- `pinned-context/`: source witnesses read from immutable head Git objects.
- `verification.json`, `context-cell-replay.json`, `capture-manifest.json`:
  checks, execution receipts and final capture identities.

The active cache is
`.spur/artifacts/cpp-gap-repository-index/change-context.duckdb`, with
`tracked_files`, `relationships` and `changed_symbols` tables. The SQL cell
accepts a changed file path and lists its incoming and outgoing context. The
script scans the entire tracked head tree, parses configuration inheritance and
build declarations, and reuses the already captured native Rust binding data.
Its source-backed edge checks reject dangling file paths, incorrect witnesses
and out-of-range source lines.

`native_bound_syntax` means a reported binding from the earlier Rust probe,
not compiler certainty. Its original artifact and line are retained. A repository
line on those records is a source-function start when uniquely known, not an
invented call-site coordinate. Other relationship classes distinguish parsed
config/build facts, source-reference candidates, shared-contract participation,
configuration consumers and possible callback dispatch.

An optional new full Rust export hit notebook startup timeouts; the unused cell
was removed after retaining its source and attempt receipt in
`optional-rust-export-attempt.json`. No output from that attempt is used. The
completed baseline's immutable native evidence remains the source for native
paths and reported cross-file target files.

The six added context cells were executed in order. The earlier 20-cell C++
baseline has its own immutable capture under
`../2026-11-17-cpp-extraction-gap-lab/`; it was not replayed for this correction.
Production extractor and extension code remain unchanged.
