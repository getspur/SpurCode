# Native release contract v1

This contract covers `spur_graph` 0.1.0, `facts_schema_version = 1`, native
DuckDB **exactly 2.0.0**, and `C_STRUCT_UNSTABLE`. The adapter preserves the
shared native extraction/matching behavior. It adds typed vectors, metadata,
row diagnostics and local packaging; it does not certify compiler semantics.
The [approved implementation plan](../../../docs/superpowers/plans/2026-11-17-spur-graph-duckdb-release.md)
implements the release gates from bd-q0lu and the app design bd-1ul3. Root
retains final review and integration authority.

## Identity and compatibility

| Identity | Authority and meaning |
|---|---|
| Git source HEAD/status | Release receipt; identifies the checkout and its recorded tracked/untracked changes, not the binary |
| `extractor_id` | Loaded `spur_graph_info()` plus an independent fresh compiled source probe; `source-sha256-v1:<64 lowercase hex>` |
| `facts_schema_version` | Loaded metadata, currently `1`; typed layout compatibility, not a promise of identical extraction across extractor versions |
| Package SHA-256 | Sidecar `binary.sha256` verified against exact packaged bytes; distinct from source ID and raw library SHA |
| Runtime | Python package/native engine versions, platform, executable/engine hashes and compiler version in release receipt |
| Source bytes | Caller scheme and digest plus logical path; repository/commit/parent labels are separate claims |
| Snapshot/run | Immutable version/run manifest and checksummed tables; requested snapshot identity must match |

`build.rs` hashes a domain-separated, sorted, length-framed inventory of exact
adapter/local locked dependency source, query, binding and build inputs,
manifests, standalone lockfile, toolchain/config and relevant inherited workspace
settings. It excludes absolute worktree paths, Git HEAD, docs/tests/outputs and
unrelated workspace members. Registry/Git dependencies are bound through the
lockfile. The compiled probe re-enumerates inputs before and after acceptance,
including additions/removals and inherited settings. Source ID is not a binary
fingerprint; ambient compiler flags and produced machine code are outside it.
Keep the build/runtime receipt and package checksum too.

All scalar source text is UTF-8 `VARCHAR`. Full-facts paths are nonempty relative
logical filenames with no absolute root, parent traversal or drive prefix; no
file needs to exist. The functions do not discover files, access Git, fetch
sources or perform repository-wide resolution. Language selection comes from
SPUR's native registry; discover `supported_languages` instead of duplicating
an extension-to-language table.

For full facts, retain `(blob_oid, logical_path, extractor_sha)` on every parent,
child and join. Same bytes at different paths can produce different facts.
`extractor_sha` is the caller's verified version key: the packaged SHA is useful
for exact-binary caching; the source ID can identify source-version caching when
schema/runtime/artifact provenance is retained separately. Never mix meanings
silently. Preserve source identity schemes explicitly: a Git blob OID is not a
plain content SHA. The headless fixture supports exactly `sha256:utf8-v1`, with
`sha256:<digest of UTF-8 bytes>` and `null-source` for NULL. Unsupported scheme
labels are rejected both on input and stored-result readback.

Native numeric node/edge/span IDs are local to one extraction. Always qualify
numeric and textual fact IDs by the full input key. `stable_symbol_id`,
`stable_key`, `source_oid`, anchors and macro candidate labels retain native
meaning; none substitutes for a complete repository/snapshot identity.

## Five public functions and examples

Load a verified local package in a **fresh** connection configured with
`allow_unsigned_extensions = true` (CLI: `duckdb -unsigned`). Use the environment
based load example in the [README](../README.md#all-five-functions). Do not
hot-load a different binary over an already loaded extension.

```sql
-- 1. Runtime metadata: no arguments.
SELECT spur_graph_info();

-- 2. Symbols: LIST<Symbol>.
SELECT spur_graph_symbols('src/a.rs', 'fn a() {}');

-- 3. Full single-file facts: Facts STRUCT with five typed lists.
SELECT spur_graph_file_facts('src/a.rs', 'fn a() {}');

-- 4. Diagnostic parent: Result STRUCT, including recoverable bad input.
SELECT spur_graph_file_facts_result('bad.unknown', 'text');

-- 5. Shared native pair matcher: changes plus ambiguity diagnostics.
SELECT spur_graph_symbol_changes('src/a.rs', 'src/a.rs', 'M',
    spur_graph_symbols('src/a.rs', 'fn a() { let x = 1; }'),
    spur_graph_symbols('src/a.rs', 'fn a() { let x = 2; }'));
```

`spur_graph_symbols(path VARCHAR, source VARCHAR)` returns `Symbol[]`.
`spur_graph_file_facts(path VARCHAR, source VARCHAR)` returns `Facts`.
`spur_graph_file_facts_result(path VARCHAR, source VARCHAR)` returns
`STRUCT(status VARCHAR, error_code VARCHAR, error_message VARCHAR, facts Facts)`.
`spur_graph_symbol_changes(path VARCHAR, old_path VARCHAR, kind VARCHAR,
old_symbols Symbol[], new_symbols Symbol[])` returns
`STRUCT(changes Delta[], diagnostics VARCHAR[])`. The aliases below describe
nested types, not installed SQL types.

```text
Info = STRUCT(
  extension_version VARCHAR, facts_schema_version UINTEGER,
  duckdb_api_version VARCHAR, extractor_id VARCHAR,
  supported_languages VARCHAR[], resolution_scope VARCHAR)

Symbol = STRUCT(
  entity_name VARCHAR, symbol_kind VARCHAR, enclosing_scope VARCHAR,
  byte_start UBIGINT, byte_end UBIGINT, line_start UBIGINT, line_end UBIGINT,
  anchor_hash VARCHAR, tokens VARCHAR[])
Delta = STRUCT(
  symbol Symbol, stable_symbol_id VARCHAR, file_path VARCHAR,
  change_kind VARCHAR, previous_stable_symbol_id VARCHAR)

Facts = STRUCT(nodes Node[], edges Edge[], spans Span[],
               macro_invocations Invocation[], macro_scopes MacroScope[])
Node = STRUCT(
  node_id UBIGINT, stable_key VARCHAR, label VARCHAR, kind VARCHAR,
  file_id UBIGINT, source_span_id UBIGINT, first_seen_run_id UBIGINT)
Edge = STRUCT(
  edge_id UBIGINT, source_node_id UBIGINT, target_node_id UBIGINT,
  relation VARCHAR, target_label VARCHAR, import_path VARCHAR,
  receiver_text VARCHAR, scope_text VARCHAR, confidence VARCHAR,
  confidence_score FLOAT, edge_kind VARCHAR, bind_method VARCHAR,
  evidence_id UBIGINT, directed BOOLEAN, change_kind ChangeKind)
Span = STRUCT(
  span_id UBIGINT, file_id UBIGINT, start_byte UINTEGER, end_byte UINTEGER,
  start_line UINTEGER, end_line UINTEGER)
Scope = STRUCT(stable_symbol_id VARCHAR, kind VARCHAR)
Location = STRUCT(file_path VARCHAR, source_oid VARCHAR,
                  byte_range UBIGINT[], line_range UBIGINT[])
Attribute = STRUCT(raw_text VARCHAR, owner Scope,
                   byte_range UBIGINT[], line_range UBIGINT[])
Invocation = STRUCT(
  site_id VARCHAR, file_path VARCHAR, source_oid VARCHAR,
  caller_stable_symbol_id VARCHAR, enclosing Scope,
  raw_macro_path VARCHAR, normalized_macro_path VARCHAR,
  byte_range UBIGINT[], line_range UBIGINT[], attributes Attribute[],
  test_context VARCHAR, debug_assertions_candidate BOOLEAN,
  execution_phase VARCHAR, resolution Resolution, sink_family_candidate VARCHAR)
Resolution = STRUCT(kind VARCHAR, definition Location)
MacroScope = STRUCT(scope Scope, file_path VARCHAR, source_oid VARCHAR,
                    byte_range UBIGINT[], line_range UBIGINT[])
ChangeKind = STRUCT(kind VARCHAR, renamed_from RenamePrev)
RenamePrev = STRUCT(kind VARCHAR, file GitPath, symbol SnapshotKey)
GitPath = STRUCT(path VARCHAR, encoding VARCHAR, bytes VARCHAR)
SnapshotKey = STRUCT(stable_symbol_id VARCHAR, commit VARCHAR)
```

Inspect the actual native SQL type with `DESCRIBE SELECT
spur_graph_file_facts('src/a.rs', '')`; use `duckdb_functions()` for signatures
and examples. [test_file_facts.py](../tests/test_file_facts.py) checks the exact
schema and every native field against an independently executed direct-Rust
oracle. Lists preserve native order and multiplicity; IDs/ranges retain integer
precision. Byte coordinates address UTF-8 bytes, not character positions.
`confidence_score` is native IEEE binary32 (`FLOAT`), not a calibrated precision
estimate. Native enums retain serialization labels; inactive payloads are NULL.
`GitPath` preserves native UTF-8 or encoded byte payloads. Parquet retains these
typed structs/lists; JSON is only metadata/oracle transport.

## NULL, errors, diagnostics and conservative evidence

| Function/input | Result |
|---|---|
| `symbols` or `file_facts`, either argument NULL | SQL NULL |
| `symbols`, valid empty parse | Non-NULL empty list; keep completion parent |
| `file_facts`, valid empty source | Native non-NULL facts, potentially file node/span |
| Throwing extraction API, unsupported/invalid input | SQL error; `TRY(...)` produces NULL and loses diagnostics |
| `file_facts_result`, either argument NULL | Non-NULL `null_input` parent, NULL facts/code/message; NULL takes precedence over path/language checks |
| Diagnostic success | Native facts unchanged, NULL code/message |
| Known recoverable diagnostic failure | `extraction_error`, NULL facts, stable code and truthful native message |
| Setup, no-tree, unknown extraction/internal/vector faults | SQL error; never silently reclassified as bad rows |
| `symbol_changes`, any top-level argument NULL | SQL NULL |
| `symbol_changes`, NULL required nested field/list element/token | SQL error |

Diagnostic success statuses are `ok`, `empty_source`, and `empty_facts`.
`empty_source` takes precedence over all-five-lists-empty classification after
successful native extraction. An empty malformed notebook is an input error,
not a successful empty source. Tree-sitter partial syntax trees can be native
successes; there is no invented general syntax-validation diagnostic.

Current recoverable codes are `invalid_path`, `unsupported_language` and
`malformed_input`. The earlier design named `extraction_failed`, but this
candidate does not map `NoTree`, setup or arbitrary erased native errors to
that code. Those remain SQL errors. Existing migrated errors have unknown
(NULL) code/message until an explicit retry; never infer historical text.

The pair matcher accepts `A`, `M`, `D`, `R`; callers normalize Git `T` to `M`
and `C` to `R` before calling. For non-renames, `old_path` equals `path`, except
it may be empty for `A`. Supply typed empty symbol lists for an absent side.
Delta kinds are `added`, `modified`, `deleted`, `renamed_from_symbol`.
`previous_stable_symbol_id` is populated only for a native symbol predecessor.
Keep `diagnostics`, including `ambiguous_rename:` evidence, and actual
commit/parent/pair keys. Cross-process list ordering of matcher results is not
promised; do not rewrite stored evidence to manufacture identity.

A resolved local helper, a macro-body singleton heuristic and a qualified
external constructor have different evidence. Preserve `scope_text`,
`receiver_text`, `import_path`, `bind_method`, `confidence`, macro resolution
kind and NULL targets. Local-target percentages are not precision. Explicit
path/local macro candidates are not proof of compiler binding or expansion.

## Package and trust route

Use `scripts/check_release.py` for release acceptance; see the README for the
complete invocation. Manual packaging accepts `--library RAW_NATIVE_LIBRARY
--out PACKAGE.duckdb_extension`; verification takes that package path. Both use
the pinned native runtime. The packager probes the staged library, writes the
DuckDB footer and sidecar, verifies a fresh final load, and atomically advances
a shared package-generation pointer for the binary and sidecar aliases.
Unmanaged outputs are refused; complete generations remain immutable.

The v1 sidecar contains `format_version`, all six `Info` fields, `abi`,
`platform`, `source_identity` (equal to `extractor_id`), `binary` (name, SHA-256,
byte size), and `trust = {mode: unsigned_local, signature: absent}`. Verification
checks the exact schema, checksum/size, footer ABI/runtime/platform, and freshly
loaded metadata. A saved sidecar is artifact data, not a saved readiness pass.

For portable copies, resolve one immutable generation and copy its binary and
adjacent sidecar together, then verify. A concurrent pointer swap can cause a
safe verification failure; retry using one resolved generation. A checksum
cannot authenticate an untrusted publisher. Current-release trust for guarded
retrieval checks requires the verified sidecar path/checksum **and** an
independent fresh build source ID; hashing an arbitrary binary is insufficient.
Historical hash allowances remain historical controls, not approval for a new
candidate. No public tag, upload, push or extension-community registration is
part of this contract.

## Cache migration, retry and coherent publication

The following recipe uses the README environment and a successful release run.
It creates a caller-owned demo database, runs the transactional cache prefix
twice, closes the writer and publishes a fresh immutable seven-table version.
It does not touch a historical corpus or use legacy direct exports.

```sh
export FACTS_DEMO="$RELEASE_RUN/cache-demo"
mkdir "$FACTS_DEMO"
"$GRAPH_PYTHON" - <<'PYTHON'
import hashlib, json, os
from pathlib import Path
import duckdb

root = Path.cwd()
crate = root / 'crates/spur-graph-duckdb'
package = Path(os.environ['SPUR_GRAPH_EXTENSION']).resolve(strict=True)
manifest = json.loads(Path(str(package) + '.json').read_bytes())
assert hashlib.sha256(package.read_bytes()).hexdigest() == manifest['binary']['sha256']
source = 'fn helper() {} fn entry() { helper(); remote_call(); }'
blob = 'sha256:' + hashlib.sha256(source.encode()).hexdigest()
extractor = manifest['binary']['sha256']
workflow = (crate / 'examples/file_facts.sql').read_text().split('-- LEGACY NON-ATOMIC', 1)[0]
database = Path(os.environ['FACTS_DEMO']) / 'facts.duckdb'
with duckdb.connect(str(database), config={'allow_unsigned_extensions': True}) as con:
    con.execute("LOAD '" + str(package).replace("'", "''") + "'")
    con.execute('CREATE TEMP TABLE staged_sources AS SELECT ?::VARCHAR blob_oid, '
                '?::VARCHAR logical_path, ?::VARCHAR source, ?::VARCHAR extractor_sha',
                [blob, 'src/demo.rs', source, extractor])
    con.execute('CREATE TEMP TABLE source_lineage AS SELECT * EXCLUDE(source), '
                "'demo'::VARCHAR snapshot_id FROM staged_sources")
    con.execute(workflow)
    con.execute(workflow)
    assert con.execute('SELECT count(*) FROM file_facts_missing').fetchone()[0] == 0
    print(con.execute('SELECT logical_path,status,error_code FROM file_facts_cache').fetchall())
PYTHON
"$GRAPH_PYTHON" crates/spur-graph-duckdb/examples/publish_snapshot.py \
  --database "$FACTS_DEMO/facts.duckdb" --output "$FACTS_DEMO/snapshots" \
  --version demo-v1 --source-label source_scheme=sha256:utf8-v1
```

Use one cache writer, with no surrounding transaction. Stage non-NULL full
keys; source alone may be NULL. Contradictory bytes (including NULL versus
non-NULL) for the same key are rejected before extraction. Keep arbitrary
caller lineage separately; reserve `extraction_status`. The pipeline stores
one diagnostic result per missing key, including failures/NULL/empty parents,
and maintains five child tables plus all caller occurrences in the same
transaction. No-op replay expands zero facts, but still compares cache keys
and refreshes caller occurrences. `fact_ordinal` preserves list position.

The additive migration adds `error_code` and `error_message` without changing
old facts/status or re-extracting cache hits. Missing child/checkpoint layout
bootstraps all five children from cached facts. Dropping `file_facts_child_state`
explicitly requests a rebuild. Direct edits to payloads/children/checkpoints
are unsupported. An incompatible output type requires an explicit migration
or a fresh database; the additive diagnostic migration cannot fix it.

To retry, delete the **exact** key from `file_facts_cache`, restage its source
for the loaded extractor, and rerun. For example, execute parameterized
`DELETE FROM file_facts_cache WHERE blob_oid=? AND logical_path=? AND
extractor_sha=?`, then the workflow. A removed key absent from staging loses
its children/checkpoint; unmatched caller occurrences report `not_cached`.
A new extractor version creates new keys. `TRY` may cause row-by-row retries;
there is no exactly-once parser-call promise on errors.

The publisher uses one read-only connection/transaction for all seven tables:
cache, nodes, edges, spans, macro invocations, macro scopes and occurrences.
It records each schema, count, SHA-256 and byte size. Source labels explicitly
say `caller_supplied` and `independently_verified = false`. Finalization
precedes atomic `current.json` replacement. Failures preserve the prior pointer
and all referenced version files; a pointer failure may leave an unreferenced
complete version. Read the manifest once, validate files, and use only its
versioned paths. No extension is required to query the stored typed Parquet.
Never combine trailing legacy `COPY` files into a current app dataset.

Publication requires local POSIX advisory locks and same-filesystem atomic
rename (packaging also requires symlinks). Keep persistent lock inodes; deleting
them breaks publisher serialization. There is no garbage collection; retain
old versions while readers may use them. Interrupted staging cleanup is an
explicit operator action. Atomic visibility does not guarantee survival of
sudden power loss after the pointer swap.

## App boundary and operating limits

The [headless quickstart](../README.md#headless-explorereview-quickstart) creates
immutable Explore/Review results. `read_result` binds a requested base/head
snapshot to one completed run and validates the immutable manifest, source
scheme and checksummed files. Run manifests retain artifact/source/schema and
runtime identities, fixture hash, process PID/raw exit, snapshot lineage,
coverage/status counts, unresolved counts, replay result and typed tables.
Unsuccessful extraction is visible as incomplete coverage; unavailable review
inputs retain their pair row with NULL result. Caller labels are not certified
Git identities.

Native work runs in a child process. The parent reaps it on success, error,
interruption or an explicit caller timeout; a candidate file alone cannot
publish completion. Faults preserve the last complete result; a later healthy
run can publish. Timeout is optional caller policy, not a production memory
or time cap. This harness is not a production supervisor, repository crawler,
network fetcher, unrestricted SQL service or UI implementation.

Whole-blob parsing remains native AST work per missing key, not SIMD parsing.
Ordinary parser/query state is reused per worker/language; notebook cell parser
state stays within its notebook input. Input batch byte budgets do not bound
AST/output/nested-vector/global-fold memory. No new performance, concurrency,
resource-limit or compiler-completeness guarantee is made here.

Managed notebook SQL remains DuckDB 1.5.5 and cannot load this native2 ABI.
Root's separate native Rust-cell transport/readback does not upgrade that SQL
runtime. Browser/WASM, Windows, wider native runtime versions, public/signed
distribution, production admission/backpressure and app UI remain outside the
observed support claim.

## Observed release acceptance: 2026-11-17, R7

This is a fresh integrated candidate, not a relabeled historical binary.
The release driver ran on clean source
`66663c00a4b4b7c898b1d1f5b66aecf99fc348f5`, with approved predecessor changes
on original base `adaa1f86e2c95201a331046a8c0ea785b8781399`.
R7 changes only this document and the README; those documentation bytes are
outside the extractor source-input inventory. Root still owns final gate
review/integration; this evidence does not close an issue or authorize public
release.

| Bound identity | Observed value |
|---|---|
| Extractor source ID | `source-sha256-v1:ea849cf9a54b442671ac3e2b4972291c24a64c03457bcffcee4e9a50ef22de35` |
| Packaged binary SHA-256 | `28ddce83f7e1a6eb0164cc5c09300de9029739c45f6973f485fa7b005797f122` |
| Raw native library SHA-256 | `7e4ef5a37961b9373723882d1cd35195dce7b4f62698c72c4198baa34069fdb2` |
| Sidecar SHA-256 | `db50a83c8c652d468b560deb25769a8826250d34659198e90213d6be415cf6f8` |
| Build source probe SHA-256 | `8a2d63c9bb21ca0cbb68be89ade079f0b9aeef62f5efd55c0b1c7f9656c2f404` |
| Runtime | Python 3.12.10; DuckDB package `2.0.0`, native `v2.0.0`; `osx_arm64` |
| Native engine SHA-256 | `1e0c41d7d52834b43bf66c1e8b25d6aeee7911609515c9563383b3ffb28919b3` |
| Python executable SHA-256 | `08b9cf48ba3b352010826e1b28dac25cdf45d1028efed29691c360123c09aada` |
| Compiler/host | Rust/Cargo 1.94.1; `aarch64-apple-darwin`; macOS 26.7.1 ARM64 |
| Contract | Extension 0.1.0; facts schema 1; single_file; unsigned_local |

The raw library and packaged binary differ because packaging appends the
DuckDB extension footer. The source ID is shared with R5's approved candidate,
but this freshly built package has a different binary SHA. Neither R5's
`2968d20ef8d4f07df100e50f6aaccb0b2bece7df048c1a92884fc12c90e2ae89`
nor the frozen corpus's corrected binary is claimed as this run's artifact.

Raw evidence is retained in the task-owned directories
`/private/tmp/spur-r7-65bb3605-release-1` and
`/private/tmp/spur-r7-65bb3605-evidence`. These are receipt locations for this
observation, not dependencies of the quickstarts. The former contains
`receipt.json`, raw phase stdout/stderr, both source-probe inventories,
`unittest-summary.json`, all native build artifacts, package and sidecar. The
latter contains the exact top-level invocation, `current-acceptance.py` and its
command/output/JSON receipt, executable copies of documented commands, solver
requests/results and final verification. Preserve them together when reviewing
or moving the candidate.

The exact release invocation was the pinned native Python running
`crates/spur-graph-duckdb/scripts/check_release.py --out
/private/tmp/spur-r7-65bb3605-release-1 --host-native --expect-platform osx_arm64`,
with `PYTHONDONTWRITEBYTECODE=1`, `SPUR_SCCACHE_S3=0` and
`SPUR_NO_LOCAL_FALLBACK=1`. Rust tests used the default remote mode, forced
remote by the driver; native packaging used a separate explicit host target.
Every driver child phase exited `0`: source-head, source-status, rustc-version,
cargo-version, rust-tests, native-build, source-probe, package, verify-package,
unittests and source-probe-final. The top-level driver exited `0`.

| Acceptance coverage | Current evidence and limits |
|---|---|
| API/schema/identity | Exact typed contract, registry discovery and independent source-inventory checks passed; both source probes agree |
| Diagnostics and errors | Remote Rust 2/2; SQL mixed/NULL/error/nested-vector tests passed; NoTree/setup/unknown faults stay SQL errors |
| Native correctness | Direct-Rust field/order parity and persistent native-versus-SQL retrieval checks passed in the full suite |
| Macro binding controls | Current binary leaves `Vec::new` unresolved, resolves valid local helper/LocalFactory controls and distinguishes both constructors in a mixed macro; frozen control and direct-Rust facts agree |
| Cache and publication | Diagnostic migration, full-key replay, child repair, seven-table snapshot consistency and failed-publication preservation passed; documented cache recipe executed, exit 0 |
| Packaging and release driver | Fresh package load/integrity/ABI tests and driver negative controls passed; source and native artifact identities rechecked after suite |
| Headless Explore/Review | Base/head fixture saved and reopened; exact package SHA, source/snapshot keys, typed exports, replay, ambiguity, unavailable inputs, interruption/timeout cleanup and healthy recovery verified |
| Frozen corrected corpus/readback | Published manifest, schema/counts/provenance and current sample/control parity verified read-only; cache checksum unchanged; root separately supplied native notebook readback |

The complete suite ran **206 tests, 0 failures, 0 errors, 0 skips, 0 expected
failures** (exit 0). It includes the current R6 source-scheme corrections.
Release-driver tests deliberately run failing/skip child fixtures; their raw
logs contain those named negative outcomes while the outer tests assert that
they prevent a release pass. There is no unresolved failed phase in this R7
release receipt.

Additional acceptance (exit 0) used this same package SHA for all current
extraction calls, direct-Rust parity and the persistent headless run. The
headless fixture retains eight source rows per snapshot: three `ok`, one
`empty_source`, one `null_input`, three `extraction_error`; unresolved edges
remain visible (base 8, head 7). All four review pairs remain, with one
unavailable pair and one pair with diagnostics. Both snapshots honestly report
incomplete extraction coverage. Replay found zero missing keys and identical
stored tables. These are fixture observations, not repository-wide metrics.
All five contract SQL examples, README function example, headless CLI and
cache/publisher recipe executed successfully. The build quickstart was checked
with `sh -n`; the actual build/check was the recorded equivalent invocation
above, using the already pinned Python environment.

### Frozen reference boundary

The authority is the main repository's
`docs/research/artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s/published.json`.
It binds cache SHA-256
`ad6f7d3874b4829e9754df2ac7ab7ed0bafc7004d241561a2ad8e52fdd81172b`,
corrected historical extension SHA-256
`be0548ce3a1217cf0b50110ad02419223c895daa8d2a932d7a81747da4c38892`,
and frozen commit `61ba3d200439a541dce4c1617ca9277be7d438af`.
R7 inspected that exact manifest, README and `inspect_published.py`. The inspector
pins its old corrected extension and was not used as a current release pass.

The fresh candidate attached the published database `READ_ONLY`. Counts remain
4,404 inventory paths, 4,098 admitted sources/current facts and 8,196 versioned
cache keys. All five collections match for the inspector's real-source sample,
`crates/spur-acp/src/capability_evidence.rs`, and `control.rs`; current results
also match the new direct-Rust oracle. The real `Vec::new` negative control was
recomputed under the current binary. Both the cache SHA and inspected reference
file hashes were checked before/after. No refresh/ingestion script ran and no
historical file was written. This is current sample/control compatibility,
not a claim of freshly re-extracting all 4,098 sources.

Root's `notebook-readback-20261117-release.json` records successful native
2.0 readback of that historical corrected cache/binary. The frozen README's
older transport-blocked note is historical; the later receipt is separate
readback evidence. The task's notebook read tool denied this worker access to
the root acceptance notebook; no raw notebook access or mutation bypass was
used. Root owns its acceptance review. Managed notebook SQL remains 1.5.5.

### TDD lineage and solver scope

R7 adds no behavior, so its RED phase is **not applicable**; no failing test was
manufactured. Approved predecessor receipts retain authentic assertions before
implementation, separate test/implementation commits and raw failures:

| Predecessor | Authentic RED | GREEN evidence and original commits |
|---|---|---|
| R1 metadata/provenance | 4 missing-function assertions; exit 1 | 7 contract + 50 regressions; exit 0; `db50e0cad` test, `709b5c268` implementation |
| R2 NoTree correction | `internal failure must propagate`; remote 1 failed/1 passed, exit 101 | Same remote 2/2, SQL diagnostics 8/8 + regressions 39/39 + R1 7/7, exit 0; `5f061fa66` test, `11d0de8cf` fix |
| R3 cache/snapshots | 10 intended assertions; exit 1 | 29 focused + 64 regressions, exit 0; `95ad5e1d4` test, `205d7837d` implementation |
| R4 packaging | 12 intended assertions plus 2 hardening failures; exit 1 | 15 focused + 64 native regressions, exit 0; `cd039f0b6`, `842c3e81b` tests, `7f8e0117d` implementation |
| R5 final source guard | 4 intended source-stability assertions; exit 1 | 13 focused, then 192 complete release tests, exit 0; `c116e7160` test, `e1134db58` fix |
| R6 source-scheme correction | 2 assertions (`ValueError` not raised); exit 1 | 14 focused + 84 regressions, exit 0; `027e420bc` test, `e0be2783e` fix |

These original worker commits/receipts describe predecessor TDD, not this
run's source HEAD. Integrated source commits may have different IDs. Earlier
intermediate failures remain in predecessor receipts; only final approved
results are represented as GREEN. R7's fresh 206-test run supplies integrated
regression evidence rather than substituting old binaries or test counts.

Catalog rule `data_integrity.mutually_consistent` checked the finite identity
policy: current acceptance phases must use the fresh candidate and native
2.0.0, never a historical reference or managed 1.5.5 runtime. PRE passed
(`sat/pass`); a stale-reference negative control failed (`unsat/fail`). POST
passed (`sat/pass`) for the observed binary/source/runtime/platform tuples; a
stale headless-artifact substitution failed (`unsat/fail`). This solver
checks supplied finite facts, not extraction semantics, OS execution or
compiler correctness. Native commands and checksums provide that execution
evidence. Persistence was rejected at `ArtifactCount` limit 512, attempted 513;
exact persisted-attempt errors and nonpersisted requests/results are retained
without deleting earlier receipts.

### Remaining release boundaries

Observed native acceptance is local macOS 26.7.1 ARM64 (`osx_arm64`) only.
Remote Linux ran Rust unit tests, which is not a Linux load/package acceptance
claim. The dedicated workflow configures `macos-15` / `aarch64-apple-darwin` /
`osx_arm64`; it was not executed as hosted CI in this task. Other platforms,
WASM and DuckDB versions outside exactly 2.0.0 have no readiness claim here.
Root final review/integration and any hosted-run acceptance remain separate
gates; public publishing/signing is not authorized. The single-file scope,
resource/supervision limits and managed-runtime split above remain explicit
limitations, not silently satisfied compiler or production-service gates.
