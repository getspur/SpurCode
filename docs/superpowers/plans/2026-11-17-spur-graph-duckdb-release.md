# spur_graph DuckDB 2.0 Release Contract Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan`.
> Each task is a beads issue with explicit dependencies and isolated scope.

**Source spec:** `/Users/kevintruong/project/spur-notebook/docs/superpowers/specs/2026-11-17-spur-graph-app-design.ipynb`, release-gate cells `d48b1c1c-137c-4029-9202-4564fbfec07e` and `2b0c4f25-91b3-4374-ab13-896e360f8a53`.
**Formal @spec cells (if notebook):** none.
**Design epic:** `bd-1ul3` and `bd-q0lu` (closed design/readiness records).
**Implementation epic:** `bd-1al5`.
**Authorization:** User explicitly approved implementation with TDD; proceed through dispatch/review/integration.

**Goal:** Deliver a versioned, diagnosable, reproducibly packaged native DuckDB 2.0 extraction/change contract validated by a headless code exploration/review consumer.

**Architecture:** Preserve existing primitive APIs and single-file resolution boundaries. Add typed runtime metadata and per-row diagnostic extraction, consume these in the cache/snapshot and package contracts, then validate a thin headless app consumer. No production UI, compiler-wide resolver, WASM or public publishing is included.

**Tech Stack:** Rust 2021; duckdb-neo 0.20000.2; DuckDB 2.0.0; spur-graph native extraction; Python unittest; typed SQL/Parquet; GitHub Actions.

**Routing:** Initial claude-code launches failed authentication before work. The atomic recovery rerouted all pre-scoped tasks to codex; its soft multi-file avoidance is overridden because interfaces/scopes are fixed and root retains architecture/review. Plan engine still owns dependencies and isolation.

**TDD:** Catalog-first SOLVE PRE; actual behavior test RED before production code; minimal GREEN; relevant regression suite; SOLVE POST when PRE solved. Preserve exact commands, raw exits, source/artifact identities and separate test/fix commits where required. No guessed limits, skips, stale binaries or mock substitutes for real native acceptance.

**Build policy:** Always scripts/spur-cargo; remote default and remote RED is real. An explicitly justified host-native packaging target may use local wrapper execution, never to evade a remote failure. Use task-specific target/output paths.

**Historical blocker correction:** bd-1i5s already completed/verified native ingestion and immutable cache publication. Only notebook readback remained blocked by transport. Root will resume the prepared read-only inspector in a Rust cell. Do not rerun refresh.py or overwrite the baseline/published cache.

## Interface contract

- Existing symbols, file-facts and symbol-changes functions keep signatures/types.
- R1 adds typed spur_graph_info(): extension_version, facts_schema_version, duckdb_api_version, extractor_id, supported_languages, resolution_scope. Source identity is distinct from packaged binary SHA.
- R2 adds typed spur_graph_file_facts_result(path,source): status, error_code, error_message, facts (exact existing type). Parent statuses: ok, empty_source, empty_facts, null_input, extraction_error. Error codes: invalid_path, unsupported_language, malformed_input, extraction_failed. Infrastructure faults remain real errors.
- Cache key remains (blob_oid,logical_path,extractor_sha). Historical missing error details stay unknown until explicit invalidation.
- Publication advances a current manifest only after a complete immutable consistent snapshot and checksums exist.
- Packaging binds freshly loaded artifact metadata to exact ABI/platform and binary SHA; unsigned/local trust stays explicit.
- Headless consumer results retain exact snapshot/source/evidence keys, completeness and unresolved status.

## DAG

```text
R1 -> R4 ------+
 |            +-> R5 --+
 +-> R3 ------+        +-> R7
R2 --/        +-> R6 --+
```

R3 depends on R1/R2; R5 and R6 depend on R1/R2/R3/R4. Root notebook readback is independent.


### R1: Expose versioned typed extension metadata and deterministic extractor identity

**Task ID:** `R1`
**Beads issue:** `bd-2bfl`
**Depends on:** none
**Suggested Worker:** `codex` (authenticated recovery route)

**Files:**
- `crates/spur-graph-duckdb/src/lib.rs`
- `crates/spur-graph-duckdb/src/metadata.rs`
- `crates/spur-graph-duckdb/build.rs`
- `crates/spur-graph-duckdb/Cargo.toml`
- `crates/spur-graph-duckdb/Cargo.lock`
- `crates/spur-graph-duckdb/tests/test_contract.py`

**Acceptance Criteria:**
Add nonbreaking spur_graph_info() typed STRUCT with extension_version, facts_schema_version, duckdb_api_version, extractor_id, supported_languages and resolution_scope. Exact ABI remains2.0.0, scope single_file. Deterministic extractor identity must cover adapter/native extraction source, queries and locked build inputs, independent of absolute worktree paths and unrelated repo edits; do not claim binary SHA self-identity. Artifact SHA remains packaging receipt authority. Discover native language registry instead of duplicating a guessed list. Preserve existing function signatures and types. Test metadata schema, repeat/fresh connection equality, supported languages, and source identity semantics. Rust build provenance must not require a clean git checkout or mislabel dirty source as HEAD. No changes to file_facts.rs, cache workflow or package script. This cohesive source/provenance task needs the six named paths; emit drift before adding more.

**Scope Boundary:** Only named files/APIs. No unrelated crates, source/user edits, semantic core resolver, historical artifacts, UI or public publication. Signal before expanding.

**Implementation:**
1. Navigate solve catalog and execute matching PRE before RED.
2. Write a real regression using the existing test harness and real pinned native connection. Contract assertions (con is the harness's loaded DuckDB connection):

```python
info = con.execute('SELECT spur_graph_info()').fetchone()[0]
self.assertEqual(info['duckdb_api_version'], '2.0.0')
self.assertEqual(info['resolution_scope'], 'single_file')
self.assertTrue(info['extractor_id'])
```

3. Run focused test; record the intended feature-missing assertion failure before production changes. The extension env variable must name this task's actual build/package receipt:

```sh
: "\${SPUR_GRAPH_EXTENSION:?set from the task package receipt}"
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -p 'test_contract.py' -v
```

R7 is integrated verification/documentation work and does not manufacture a failing feature. R4-R6 define their CLI/harness seams in tests before adding implementation; conceptual assertion snippets do not authorize nonexistent helper calls in shipped tests.

4. Implement minimal behavior, rebuild/package actual native changes through wrapper, run focused and relevant regression checks, and POST-solve the landed policy when PRE solved.
5. Commit owned tests then implementation with intent-focused repository messages. Return committed diff, summary paragraph, exact test-plan/results bullets, RED/GREEN evidence and source/build/runtime/binary identities.

**Signal checkpoint:** Scope drift before extra files/interfaces; blocked for unavailable requirements; risk for uncertain provenance. Do not close/merge your task or weaken acceptance.

### R2: Add typed per-input diagnostic extraction without breaking the facts API

**Task ID:** `R2`
**Beads issue:** `bd-32u0`
**Depends on:** none
**Suggested Worker:** `codex` (authenticated recovery route)

**Files:**
- `crates/spur-graph-duckdb/src/file_facts.rs`
- `crates/spur-graph-duckdb/tests/test_diagnostics.py`

**Acceptance Criteria:**
Register additive spur_graph_file_facts_result(logical_path,source) in existing file_facts::register, without editing lib.rs. Return STRUCT(status VARCHAR,error_code VARCHAR,error_message VARCHAR,facts <exact existing facts type>). Status contract: ok, empty_source, empty_facts, null_input, extraction_error. Recoverable row failures use stable codes invalid_path, unsupported_language, malformed_input or extraction_failed mapped from actual native error variants; include truthful diagnostic text. NULL source/path returns nonNULL diagnostic parent with status null_input/factsNULL and no spurious extraction error. Good/empty rows retain exact native facts and list order. Mixed good/NULL/error chunks, constants/selections/large child lists, repeated connections and unchanged old spur_graph_file_facts exception semantics are acceptance. Do not hide internal allocator/vector/kernel faults as row errors or catch native crashes. Add a runnable SQL example to the new function documentation. No core resolver edits or arbitrary truncation/size bounds.

**Scope Boundary:** Only named files/APIs. No unrelated crates, source/user edits, semantic core resolver, historical artifacts, UI or public publication. Signal before expanding.

**Implementation:**
1. Navigate solve catalog and execute matching PRE before RED.
2. Write a real regression using the existing test harness and real pinned native connection. Contract assertions (con is the harness's loaded DuckDB connection):

```python
r = con.execute("SELECT spur_graph_file_facts_result('bad.unknown','text')").fetchone()[0]
self.assertEqual(r['status'],'extraction_error')
self.assertEqual(r['error_code'],'unsupported_language')
self.assertTrue(r['error_message'])
```

3. Run focused test; record the intended feature-missing assertion failure before production changes. The extension env variable must name this task's actual build/package receipt:

```sh
: "\${SPUR_GRAPH_EXTENSION:?set from the task package receipt}"
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -p 'test_diagnostics.py' -v
```

R7 is integrated verification/documentation work and does not manufacture a failing feature. R4-R6 define their CLI/harness seams in tests before adding implementation; conceptual assertion snippets do not authorize nonexistent helper calls in shipped tests.

4. Implement minimal behavior, rebuild/package actual native changes through wrapper, run focused and relevant regression checks, and POST-solve the landed policy when PRE solved.
5. Commit owned tests then implementation with intent-focused repository messages. Return committed diff, summary paragraph, exact test-plan/results bullets, RED/GREEN evidence and source/build/runtime/binary identities.

**Signal checkpoint:** Scope drift before extra files/interfaces; blocked for unavailable requirements; risk for uncertain provenance. Do not close/merge your task or weaken acceptance.

### R3: Preserve cache diagnostics and publish coherent typed snapshots

**Task ID:** `R3`
**Beads issue:** `bd-43c1`
**Depends on:** `R1`, `R2`
**Suggested Worker:** `codex` (authenticated recovery route)

**Files:**
- `crates/spur-graph-duckdb/examples/file_facts.sql`
- `crates/spur-graph-duckdb/tests/test_file_facts_pipeline.py`
- `crates/spur-graph-duckdb/examples/publish_snapshot.py`
- `crates/spur-graph-duckdb/tests/test_publish_snapshot.py`

**Acceptance Criteria:**
Consume diagnostic API for missing cache keys only and persist error_code/error_message with parent status. Add explicit additive migration for prior cache schema; never fabricate old error details or silently reparse immutable keys. Preserve full key(blob_oid,logical_path,extractor_sha), all child/status/occurrence semantics, custom schemas, checkpoints and no-op replay. Existing pipeline tests must pass. Add snapshot publisher that exports all seven named tables from one consistent read-only DB snapshot into a new staging/version directory, records schema/count/checksum/source-label provenance and only atomically advances a current manifest after the complete set is finalized. Failed publication leaves prior manifest/data readable; no partial success or silently overwritten snapshots. Caller-supplied source labels must be identified as supplied, not independently certified. Keep legacy SQL direct exports documented as legacy non-atomic; the supported publisher is the coherent app consumption path. No new numerical default limits without evidence/solve.

**Scope Boundary:** Only named files/APIs. No unrelated crates, source/user edits, semantic core resolver, historical artifacts, UI or public publication. Signal before expanding.

**Implementation:**
1. Navigate solve catalog and execute matching PRE before RED.
2. Write a real regression using the existing test harness and real pinned native connection. Contract assertions (con is the harness's loaded DuckDB connection):

```python
self.assertEqual(self.scalar("SELECT error_code FROM file_facts_cache WHERE logical_path='bad.unknown'"),'unsupported_language')
self.assertEqual(self.scalar('SELECT count(*) FROM file_facts_missing'),0)  # after explicit replay
# Publication fault fixture must leave the previous current manifest and all checksummed files unchanged.
```

3. Run focused test; record the intended feature-missing assertion failure before production changes. The extension env variable must name this task's actual build/package receipt:

```sh
: "\${SPUR_GRAPH_EXTENSION:?set from the task package receipt}"
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -p 'test_file_facts_pipeline.py' -v
```

R7 is integrated verification/documentation work and does not manufacture a failing feature. R4-R6 define their CLI/harness seams in tests before adding implementation; conceptual assertion snippets do not authorize nonexistent helper calls in shipped tests.

4. Implement minimal behavior, rebuild/package actual native changes through wrapper, run focused and relevant regression checks, and POST-solve the landed policy when PRE solved.
5. Commit owned tests then implementation with intent-focused repository messages. Return committed diff, summary paragraph, exact test-plan/results bullets, RED/GREEN evidence and source/build/runtime/binary identities.

**Signal checkpoint:** Scope drift before extra files/interfaces; blocked for unavailable requirements; risk for uncertain provenance. Do not close/merge your task or weaken acceptance.

### R4: Make native extension packaging reproducible and independently verifiable

**Task ID:** `R4`
**Beads issue:** `bd-kcyb`
**Depends on:** `R1`
**Suggested Worker:** `codex` (authenticated recovery route)

**Files:**
- `crates/spur-graph-duckdb/scripts/package_extension.py`
- `crates/spur-graph-duckdb/scripts/verify_package.py`
- `crates/spur-graph-duckdb/tests/test_package.py`

**Acceptance Criteria:**
Keep existing --library/--out invocation compatible. Package into a staging output, load/probe the packaged artifact on a fresh exact2.0.0 native connection, verify info/ABI/platform/version and emit a versioned sidecar with exact binary SHA256, size, source/extractor/schema identity and explicit unsigned/local trust mode. Verifier must reject tampered/missing/inconsistent package files and incompatible runtime/ABI/platform clearly, and distinguish artifact data from live readiness. Bind metadata to loaded artifact, not an unrelated source tree. Failure must not advertise a valid new package or destroy the last completed release. Test real native package load plus negative cases; mocks only for unavailable-platform mechanics, never as a substitute for real load. No fake signatures, auto public publishing, network installation or version upgrades.

**Scope Boundary:** Only named files/APIs. No unrelated crates, source/user edits, semantic core resolver, historical artifacts, UI or public publication. Signal before expanding.

**Implementation:**
1. Navigate solve catalog and execute matching PRE before RED.
2. Write a real regression using the existing test harness and real pinned native connection. Contract assertions (con is the harness's loaded DuckDB connection):

```python
# Real packaged artifact required for positive path.
manifest = verify_package(output_path)
self.assertEqual(manifest['duckdb_api_version'],'2.0.0')
# Flip a byte in a fixture copy; verification must reject checksum mismatch before loading.
```

3. Run focused test; record the intended feature-missing assertion failure before production changes. The extension env variable must name this task's actual build/package receipt:

```sh
: "\${SPUR_GRAPH_EXTENSION:?set from the task package receipt}"
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -p 'test_package.py' -v
```

R7 is integrated verification/documentation work and does not manufacture a failing feature. R4-R6 define their CLI/harness seams in tests before adding implementation; conceptual assertion snippets do not authorize nonexistent helper calls in shipped tests.

4. Implement minimal behavior, rebuild/package actual native changes through wrapper, run focused and relevant regression checks, and POST-solve the landed policy when PRE solved.
5. Commit owned tests then implementation with intent-focused repository messages. Return committed diff, summary paragraph, exact test-plan/results bullets, RED/GREEN evidence and source/build/runtime/binary identities.

**Signal checkpoint:** Scope drift before extra files/interfaces; blocked for unavailable requirements; risk for uncertain provenance. Do not close/merge your task or weaken acceptance.

### R5: Add explicit native release-check automation and CI for the standalone crate

**Task ID:** `R5`
**Beads issue:** `bd-6h5s`
**Depends on:** `R1`, `R2`, `R3`, `R4`
**Suggested Worker:** `codex` (authenticated recovery route)

**Files:**
- `crates/spur-graph-duckdb/scripts/check_release.py`
- `crates/spur-graph-duckdb/tests/test_release_driver.py`
- `.github/workflows/spur-graph-duckdb.yml`
- `crates/spur-graph-duckdb/tests/test_project_quality.py`
- `crates/spur-graph-duckdb/tests/test_retrieval_parity.py`
- `crates/spur-graph-duckdb/examples/retrieval_parity.py`

**Acceptance Criteria:**
Create deterministic checked release entry point covering exact-version environment preflight, wrapper-based native build/example setup, package verification and complete existing/new unittest suite with all required native-oracle env values, clear raw exits and output receipts. Do not silently skip tests or adopt stale /tmp artifacts. Add dedicated workflow with relevant crate/core/query/wrapper path triggers; use project wrapper and declare actual configured targets. If hosted CI requires local native packaging, document that native-target reason. Check current official CI docs before assuming runner platform/architecture. Local checks need no external publication credentials and must not claim a configured CI platform was actually tested here. Driver tests must fail under missing/wrong runtime and propagate child failures. No unrelated release workflows or broad toolchain upgrades.

**Scope Boundary:** Only named files/APIs. No unrelated crates, source/user edits, semantic core resolver, historical artifacts, UI or public publication. Signal before expanding.

**Implementation:**
1. Navigate solve catalog and execute matching PRE before RED.
2. Write a real regression using the existing test harness and real pinned native connection. Contract assertions (con is the harness's loaded DuckDB connection):

```python
# Pass a deliberately failing child phase via a controlled test seam.
self.assertNotEqual(run_release_check(failing_phase).returncode,0)
# Missing native oracle / wrong DuckDB runtime must fail preflight, never report skipped success.
```

3. Run focused test; record the intended feature-missing assertion failure before production changes. The extension env variable must name this task's actual build/package receipt:

```sh
: "\${SPUR_GRAPH_EXTENSION:?set from the task package receipt}"
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -p 'test_release_driver.py' -v
```

R7 is integrated verification/documentation work and does not manufacture a failing feature. R4-R6 define their CLI/harness seams in tests before adding implementation; conceptual assertion snippets do not authorize nonexistent helper calls in shipped tests.

4. Implement minimal behavior, rebuild/package actual native changes through wrapper, run focused and relevant regression checks, and POST-solve the landed policy when PRE solved.
5. Commit owned tests then implementation with intent-focused repository messages. Return committed diff, summary paragraph, exact test-plan/results bullets, RED/GREEN evidence and source/build/runtime/binary identities.

**Signal checkpoint:** Scope drift before extra files/interfaces; blocked for unavailable requirements; risk for uncertain provenance. Do not close/merge your task or weaken acceptance.

### R6: Implement headless app-consumer acceptance with snapshot and process recovery

**Task ID:** `R6`
**Beads issue:** `bd-1u5s`
**Depends on:** `R1`, `R2`, `R3`, `R4`
**Suggested Worker:** `codex` (authenticated recovery route)

**Files:**
- `crates/spur-graph-duckdb/examples/app_consumer.py`
- `crates/spur-graph-duckdb/tests/test_app_consumer.py`
- `crates/spur-graph-duckdb/tests/fixtures/app_consumer.json`

**Acceptance Criteria:**
Provide a small reusable headless Explore/Review consumer of the release contract. Fixture supplies frozen base/head multi-file sources and caller lineage, including same-byte different paths, invalid/empty/NULL rows, valid local helper and unresolved external/macro constructor cases, and ambiguous change. Validate source/extractor identity, full-key joins, conservative diagnostics and pair changes; save/reopen a coherent snapshot and repeat without duplicates. Result manifest must expose exact snapshot/schema/extractor/artifact identity, typed result files, coverage/completeness and no false compiler/repository-wide resolution claims. Run native work in an isolated subprocess so injected early exit/interruption cannot falsely publish a completed result; check process cleanup and later healthy run. Budgets, when needed, are explicit caller inputs or existing runtime-derived values, not invented production constants. No app UI, full repository crawler, unrestricted SQL service, git/network fetch or production supervisor architecture.

**Scope Boundary:** Only named files/APIs. No unrelated crates, source/user edits, semantic core resolver, historical artifacts, UI or public publication. Signal before expanding.

**Implementation:**
1. Navigate solve catalog and execute matching PRE before RED.
2. Write a real regression using the existing test harness and real pinned native connection. Contract assertions (con is the harness's loaded DuckDB connection):

```python
# A reply/result for base snapshot A cannot be labeled as snapshot B.
self.assertEqual(result['snapshot_id'], requested_snapshot_id)
# Faulted child must leave no new completed result pointer; next healthy process succeeds.
```

3. Run focused test; record the intended feature-missing assertion failure before production changes. The extension env variable must name this task's actual build/package receipt:

```sh
: "\${SPUR_GRAPH_EXTENSION:?set from the task package receipt}"
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -p 'test_app_consumer.py' -v
```

R7 is integrated verification/documentation work and does not manufacture a failing feature. R4-R6 define their CLI/harness seams in tests before adding implementation; conceptual assertion snippets do not authorize nonexistent helper calls in shipped tests.

4. Implement minimal behavior, rebuild/package actual native changes through wrapper, run focused and relevant regression checks, and POST-solve the landed policy when PRE solved.
5. Commit owned tests then implementation with intent-focused repository messages. Return committed diff, summary paragraph, exact test-plan/results bullets, RED/GREEN evidence and source/build/runtime/binary identities.

**Signal checkpoint:** Scope drift before extra files/interfaces; blocked for unavailable requirements; risk for uncertain provenance. Do not close/merge your task or weaken acceptance.

### R7: Verify the exact release candidate and finish the supported SQL contract documentation

**Task ID:** `R7`
**Beads issue:** `bd-3cc0`
**Depends on:** `R5`, `R6`
**Suggested Worker:** `codex` (authenticated recovery route)

**Files:**
- `crates/spur-graph-duckdb/README.md`
- `crates/spur-graph-duckdb/docs/release-contract.md`

**Acceptance Criteria:**
On integrated predecessor source, build/package a fresh native release candidate and run complete release checks against that exact binary. Run current macro-binding positive/negative controls, native parity, all suites and headless Explore/Review fixture; bind source/build/runtime/artifact identities and named failures, no old binary passed as current. Existing corrected corpus in docs/research/artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s is immutable, already verified; inspect exact published manifest and use read-only reference for current release compatibility/parity checks, never rerun refresh.py or overwrite baseline. Root separately completes notebook readback. Document stable v1 facts/schema/identity contract, all five public function examples, NULL/error/diagnostic semantics, native2.0-only ABI, supported package/trust route, cache migration/retry/coherent snapshot recipe, operational limitations and app/headless quickstart with no hardcoded machine-local /tmp dependencies. Record observed vs configured platform support, current correctness evidence and any unresolved gates honestly. No public release tag/upload or claiming compiler semantics. Verification-only/documentation changes do not need contrived RED; all added behavior in predecessors must have authentic TDD evidence.

**Scope Boundary:** Only named files/APIs. No unrelated crates, source/user edits, semantic core resolver, historical artifacts, UI or public publication. Signal before expanding.

**Implementation:**
1. Navigate solve catalog and execute matching PRE before RED.
2. Write a real regression using the existing test harness and real pinned native connection. Contract assertions (con is the harness's loaded DuckDB connection):

```python
# Release acceptance invokes scripts/check_release.py on the integrated source and generated artifact.
# A successful receipt must bind the same binary checksum used by every runtime acceptance phase.
```

3. Run focused test; record the intended feature-missing assertion failure before production changes. The extension env variable must name this task's actual build/package receipt:

```sh
: "\${SPUR_GRAPH_EXTENSION:?set from the task package receipt}"
/private/tmp/spur-graph-duckdb-v2-20261114/bin/python -m unittest discover -s crates/spur-graph-duckdb/tests -p 'test_*.py' -v
```

R7 is integrated verification/documentation work and does not manufacture a failing feature. R4-R6 define their CLI/harness seams in tests before adding implementation; conceptual assertion snippets do not authorize nonexistent helper calls in shipped tests.

4. Implement minimal behavior, rebuild/package actual native changes through wrapper, run focused and relevant regression checks, and POST-solve the landed policy when PRE solved.
5. Commit owned tests then implementation with intent-focused repository messages. Return committed diff, summary paragraph, exact test-plan/results bullets, RED/GREEN evidence and source/build/runtime/binary identities.

**Signal checkpoint:** Scope drift before extra files/interfaces; blocked for unavailable requirements; risk for uncertain provenance. Do not close/merge your task or weaken acceptance.

## Completion gate

Root verifies task Beads audits, diffs, artifacts and downstream overlay predictions; approves via review_task; integrates the fully approved plan; verifies the integrated release candidate; and closes the epic only then. Preserve unrelated dirty files. No public release/tag/upload/push in this authorization.

## Root preflight addenda

The engine canonical epic is `bd-2a0m`, plan `ec61197f-12d0-40d6-b0bc-ae67d63bd643`. Source task records were closed as superseded; canonical IDs are R1 `bd-4q5c`, R2 `bd-7ofn`, R3 `bd-3b3i`, R4 `bd-1u5k`, R5 `bd-1gyo`, R6 `bd-3ucn`, R7 `bd-3jt4`.

R5 may update the two historical hash-pinned test files and their retrieval helper to accept explicitly verified current manifests while preserving historical defaults and tamper rejection. This is not permission to self-approve arbitrary binaries.

The engine brain-snapshot base captures unrelated pre-existing user edits. Final working-repository integration must apply only reviewed task commits after that snapshot, excluding the snapshot commit itself, so those user edits remain uncommitted and untouched.

### Reviewed-source continuation

The original controller plan is superseded, not a completed release. R1 passed independent review and root provenance verification. R2's canonical candidate (`df291105f`, `01acc87f6`) had genuine RED/GREEN evidence, but review found one important defect: `ExtractError::NoTree` was converted into a recoverable row. It must remain a SQL error. The correction requires an assertion RED before the minimal production fix.

Authentication and queued-dispatch failures exhausted the original retry budget. `request_changes` consequently rejected R2 and cascaded pending tasks; `RetryTask` refused the exhausted budget, and the attempted split did not become a schedulable plan member. Those operations and outcomes remain in Beads. Undispatched old tasks were stopped before continuing.

The continuation plan is `f02f5ee1-f011-4b13-bb0b-e8d6451ee34a`, engine epic `bd-3qno`. Its explicit base is `f2ea872c19fc00aeab6d891458d3bb1408ca5dcb`: the clean recorded source plus only the independently approved R1 test and implementation commits. It excludes the earlier dirty-file snapshot. The continuation preserves all release gates and runs R2 through R7 serially to match observed worker capacity.

| Task | Canonical continuation issue |
| --- | --- |
| R2 | `bd-ujgg` |
| R3 | `bd-94uh` |
| R4 | `bd-13rf` |
| R5 | `bd-36op` |
| R6 | `bd-27tt` |
| R7 | `bd-r7f4` |

R2 adopts the existing scoped implementation and original TDD receipts, corrects the review finding test-first, and verifies the combined metadata/diagnostic candidate. Final integration can use the clean continuation branch while preserving unrelated working files. The authorization still excludes public publishing.
