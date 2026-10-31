# Panic Evidence Pilot Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** `2026-10-31-panic-graph-experiment.ipynb`, findings cell `b6607f09-c5a1-46ce-8b8c-f053f54349fc` and pilot spec cell `6c9c3a19-bd0f-4727-b639-9f330b69d1a8` (version 466).
**Formal @spec cells:** `6c9c3a19-bd0f-4727-b639-9f330b69d1a8`.
**Design task:** `bd-3je`; source epic: `bd-1hg`.
**Source task issues:** P1 `bd-ar8`, P2 `bd-hfy`, P3 `bd-lwx`. `graph_plan` verified the serial dependency order.
**Authorization:** User approved the next step after the notebook recommendation committed as `ef4dc5749`. Prior user routing remains Codex, model `gpt-6.1-sol`, effort `xhigh`, task-by-task SOLVE and TDD.

**Goal:** Make a runnable, source-bound and configuration-bound compiler evidence pilot that preserves direct method-call multiplicity and distinguishes compiler observations from unknown targets.

**Architecture:** Add an experimental `spur_graph::panic_evidence` sidecar API, independent of existing deduplicated Calls edges and the v11 writer. Collect direct Rust method sites from graph-bound source bytes; validate and import a pinned CGA export with a capture envelope; exercise both with a standalone fixture and example command. The brain performs final notebook acceptance through MCP.

**Tech stack:** Existing Rust 2021 workspace, tree-sitter-rust, serde, existing Git blob/BLAKE3/SHA256 helpers, CGA revision `2fc63108f0b674fe7a178191ffa15647a18879f1` on `nightly-2026-10-18`, DuckDB notebook MCP.

## Contract and limits

The sidecar is opt-in. Do not change `GraphFacts`, `GraphIndexArtifact`, `GraphEdge`, `SCHEMA_VERSION`, `EXTRACTOR_VERSION`, Parquet tables, ordinary graph builds or CURRENT. This phase does not certify workspace compilation or panic freedom.

The collector accepts an existing `GraphIndexArtifact` and explicit physical source bytes. Proposed public entry point:

```rust
pub struct PanicEvidenceSource<'a> {
    pub file_path: &'a str,
    pub bytes: &'a [u8],
}

pub fn collect_method_call_sites(
    graph: &GraphIndexArtifact,
    sources: &[PanicEvidenceSource<'_>],
) -> anyhow::Result<MethodCallSiteEvidenceSet>;
```

The P1 public set exposes `sites: Vec<MethodCallSiteEvidence>` and a method
`validate(&self, graph: &GraphIndexArtifact, sources: &[PanicEvidenceSource<'_>]) -> anyhow::Result<()>`.
Each row exposes `site_id`, `file_path`, `source_oid`, `byte_range`,
`method_byte_range`, `receiver_byte_range`, `line_range`, `method_name`,
`receiver_text`, `caller_stable_symbol_id`, `lexical_phase` and an explicit
unknown compiler-target state. The remaining bindings and coverage metadata
are serialized too. P2 adds compiler observations beside these source rows.

The set records schema, graph hash, exact supplied source bindings, coverage limited to supplied physical Rust files, and deterministic rows. Each site has a domain-separated ID; Git blob OID; half-open whole-call, method and receiver byte ranges; one-based inclusive line range; exact method/receiver text; lexical phase; and optional real graph caller. Calls inside closures, const/item contexts or ambiguous/malformed ownership must retain that qualification; do not manufacture runtime ownership. Never assign a different existing function merely because its ID exists. The collector handles all direct method names, repeated calls, chains and generic functions, and excludes comments, strings and unparsed macro token trees. Receiver syntax is not a type.

The capture envelope binds the expected graph hash, source/manifest/lock/config inputs, verbose rustc identity, host/target, exporter revision and binary digest, package/target selection, feature/default-feature choices, cfg/compiler flags, Cargo arguments, relevant allowlisted environment, capture command and raw-export checksum. Sets/maps canonicalize deterministically; argument and flag ordering remains meaningful. A changed target, toolchain, feature, flag, source or export must fail an expected-identity check. Bounded fixture capture is the supported initial environment; arbitrary ambient Cargo environments are unsupported.

The pinned CGA schema has `roots`, `nodes` and `edges`. Nodes contain canonical instance names and raw locations. Preserve the exact distinct kinds `Called`, `VTableCalled`, `FnPtrCast`, `BuiltinAssert`, `AsyncPanic`. Check all graph references. External or unknown source locations stay external or unknown. CGA does not export receiver-type fields, assertion reasons or macro expansion maps; represent those capabilities as unavailable. Preserve monomorphized canonical callee strings rather than guessing a type from a method name.

Location conversion must be tested against the actual exporter/compiler, including UTF-8. The serializer uses `RangeInclusive`, while its endpoints come from rustc source-map locations; do not infer byte or end semantics from that Rust container name. Associations use matching source/config identity, path and byte location, never a bare method name. Preserve exact, unique-containing, ambiguous and unmapped outcomes. Multiple instantiated uses remain multiple compiler observations. Absence of an association does not mean safe.

## Delegation and verification

The user explicitly selected Codex. Brain owns the architecture and review; workers receive small serial scopes, overriding Codex's soft architectural/multi-file avoidance with a concrete contract. DAG: `P1 -> P2 -> P3`; shared module changes occur only after predecessor approval. Root remains the only notebook writer. No native collaboration subagents.

Each task must:

1. Navigate `solve_rule_spec`, select implemented catalog rules, run persisted PRE for actual finite identity/reference/cardinality/configuration facts and negative controls. Split projections if needed to fit catalog limits; retain row identity and exact requests.
2. Write tests first. A new API may initially fail compilation; record that separately and add only minimum type/empty-behavior scaffolding needed to obtain a genuine assertion RED before implementing behavior. Missing imports are not the required RED evidence.
3. Commit failing tests as `test(spur-graph): Pn ...`, implement minimally, and commit as `feat`/`fix`. Retain authored commits, not only the normalized final worker tree.
4. Run local appropriate tests with `SPUR_REMOTE=0 scripts/spur-cargo`; never bare cargo or remote fallback. Formatting is local. Do not weaken existing checks or bless snapshots incidentally.
5. Run persisted POST on landed facts, reload receipts with `get_solve_result`, and store commands, exit codes, assertion names, full log locations and digests in the assigned execution Beads audit. Solver checks validate supplied finite metadata, not compiler semantics.
6. Signal scope drift before edits outside the listed scope. Workers must use the actual assigned execution issue and verify `in_progress`, not the source issue template. Workers do not close issues.

All workers preserve pre-existing edits to `crates/spur-acp/src/adapter/codex.rs`, `crates/spur-acp/src/adapter/mod.rs`, `docs/critical-findings.md`, `docs/research/2026-10-27-beads-maintenance-analysis.ipynb`, and `duckdb-extension-journey-interactive-20261027.ipynb`. No global format/clean/rebuild, dependency upgrades, workspace toolchain edits, notebook raw reads/writes, or changes under `.spur/graph`.

## Task P1: Collect and validate individual direct method sites

**Task ID:** `P1`  
**Depends on:** none  
**Suggested worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- Create `crates/spur-graph/src/panic_evidence/mod.rs`: source bindings, site/set types, validator and experimental API.
- Create `crates/spur-graph/src/panic_evidence/method_sites.rs`: tree-sitter collection and lexical provenance.
- Modify `crates/spur-graph/src/lib.rs`: export the new experimental module only.
- Create `crates/spur-graph/tests/panic_method_sites.rs`: contract and direct-syntax tests.

**Acceptance criteria:**

- Every supplied source matches an actual physical `.rs` graph file and its manifest blob, with no duplicates or mismatched snapshot.
- Repeated calls to identical method/receiver expressions remain distinct by byte span; chains and generic calls remain distinct too.
- The set has deterministic canonical ordering, source/file/site identities, exact UTF-8 slices and reproducible validation after JSON round-trip.
- Unknown type and non-function/closure/const provenance remain explicit. Invalid caller membership or ownership is rejected, including another existing function in the same file.
- Reject stale source, missing/duplicate site, changed spans/text/ID and mixed graph hash; source set cardinality is checked against the declared supplied-file scope, without claiming all workspace sites.
- Existing macro contract and syntax extraction regressions pass without changes to their expectations.

**Test witness:**

```rust
const SOURCE: &str = r#"
struct Safe;
impl Safe { fn unwrap(&self) -> u8 { 7 } }
fn repeat(a: Option<u8>, b: Option<u8>, safe: Safe) {
    let _ = a.unwrap(); let _ = b.unwrap(); let _ = a.unwrap();
    let _ = safe.unwrap();
}
"#;

#[test]
fn repeated_calls_remain_individual_source_bound_sites() -> anyhow::Result<()> {
    use spur_graph::{artifact_from_facts, build_facts};
    use spur_graph::panic_evidence::{collect_method_call_sites, PanicEvidenceSource};
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("src"))?;
    std::fs::write(root.path().join("src/lib.rs"), SOURCE)?;
    let (facts, _) = build_facts(root.path(), None)?;
    let graph = artifact_from_facts(&facts, root.path())?;
    let sources = [PanicEvidenceSource {
        file_path: "src/lib.rs", bytes: SOURCE.as_bytes(),
    }];
    let set = collect_method_call_sites(&graph, &sources)?;
    assert_eq!(set.sites.len(), 4, "each direct invocation needs a site");
    assert_eq!(set.sites.iter().map(|s| &s.site_id)
        .collect::<std::collections::BTreeSet<_>>().len(), 4);
    for site in &set.sites {
        assert_eq!(&SOURCE[site.method_byte_range[0]..site.method_byte_range[1]], "unwrap");
        assert_eq!(site.method_name, "unwrap");
    }
    set.validate(&graph, &sources)?;
    let mut missing = set.clone();
    missing.sites.pop();
    assert!(missing.validate(&graph, &sources).is_err());
    Ok(())
}
```

**Implementation sequence:** persist PRE receipts; add fixtures/assertions; obtain assertion RED; implement collector and validation; run the focused suite plus existing macro contract/extraction suite; persist POST; commit and audit. Suggested commands (use actual filter names from the tests):

```bash
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_method_sites
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test macro_invocation_contract
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_macro_invocations
SPUR_REMOTE=0 scripts/spur-cargo fmt --all -- --check
```

**Scope boundary:** No compiler importer, capture process, graph storage changes, CLI installation or notebook edits. If no graph symbol exactly owns a site, report qualified/unknown ownership rather than modifying extraction to invent one.

## Task P2: Validate build-bound CGA evidence and associate locations

**Task ID:** `P2`  
**Depends on:** `P1`  
**Suggested worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- Create `crates/spur-graph/src/panic_evidence/build_identity.rs`: canonical capture/build identity and checksum validation.
- Create `crates/spur-graph/src/panic_evidence/cga.rs`: pinned format, validated compiler graph, raw/converted locations and associations.
- Modify `crates/spur-graph/src/panic_evidence/mod.rs`: expose P2 API without changing P1 semantics.
- Create `crates/spur-graph/tests/panic_compiler_evidence.rs`: identity, graph-reference and association controls.
- Create `crates/spur-graph/tests/fixtures/panic_evidence/cga-format.json`: compact format fixture with explicit synthetic-fixture status; do not claim hand-authored data is compiler output.

**Acceptance criteria:**

- Canonical build identity includes every listed semantic input; flag/argument order is retained. Set ordering alone does not change the ID.
- Expected build/snapshot/source/export checks reject each altered identity field, changed raw bytes and unsupported exporter revision before any trusted association.
- Dangling roots/edge sources/targets and malformed local spans are rejected. All five edge kinds survive serialization and remain semantically distinct.
- Exact and unique-containing path/span joins are distinguished from ambiguous/unmapped/external outcomes; unrelated existing callers and same-name methods do not acquire associations by name.
- Multiple calls/instantiations remain distinct compiler observations. Raw canonical instances and raw locations survive; receiver type, assert reason and macro expansion support remain explicitly unavailable.
- Unit tests cover documented line/column/end conventions and UTF-8; P3 must verify these against a real pinned export before pilot acceptance.

**Test witnesses:**

```rust
// Start from a complete fixed capture envelope and compiler graph fixture.
let expected_id = identity.build_id()?;
let mut changed = identity.clone();
changed.target = "aarch64-unknown-linux-gnu".into();
assert_ne!(expected_id, changed.build_id()?);
// Import with the original expected_id must reject changed, before association.
// A Called edge to core::option::Option::<u8>::unwrap and a Called edge to
// Safe::unwrap at different spans remain different observations; spelling alone
// cannot resolve an unmapped site. BuiltinAssert retains no invented reason.
```

**Implementation sequence:** use catalog data-integrity/configuration rules for fixed-envelope identity/ref controls; retain PRE; write assertion RED tests; implement; run P1/P2 suites and formatting; POST on landed policy; authored test/fix commits and audit.

```bash
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test panic_method_sites --test panic_compiler_evidence
SPUR_REMOTE=0 scripts/spur-cargo check -p spur-graph --lib
SPUR_REMOTE=0 scripts/spur-cargo fmt --all -- --check
```

**Scope boundary:** Read pinned upstream source if needed, without adding a moving git dependency or private-rustc dependency to SpurCode. No broad MIR-text parser, compiler driver rewrite, panic-name heuristic, or whole-program analysis. Signal if a real exporter limitation prevents the specified association; preserve that limitation rather than weaken metadata checks.

## Task P3: Make the pilot executable and verify held-out compiler cases

**Task ID:** `P3`  
**Depends on:** `P2`  
**Suggested worker:** codex / gpt-6.1-sol / xhigh.

**Files:**

- Create `crates/spur-graph/examples/panic_evidence.rs`: JSON-in/JSON-out collect/import/validate commands, no shell interpolation and no graph writes.
- Create `crates/spur-graph/tests/panic_evidence_end_to_end.rs`: end-to-end metadata and held-out acceptance.
- Create `crates/spur-graph/tests/fixtures/panic_evidence/corpus.json`: portable exact file strings for a standalone Rust package plus local dependency; creates no workspace members or nested Git repository.
- Create `scripts/experiments/panic-evidence-capture.py`: standard-library-only bounded capture, exact input inventory, wrapper-only subprocess commands, recorded producer identity, before/after checks and output checksum.
- Modify `crates/spur-graph/README.md`: experimental command contract, supported scope and limitations.
- Modify this plan with verification ledger and actual authored commits; do not edit notebook.

The brain supplies the installed pinned CGA driver path in a Beads comment before this task is dispatched. CGA may be unavailable for a platform: report that result; do not claim compiler GREEN from synthetic fixtures or silently skip a mandatory gate. A default Rust test may use frozen evidence with truthful provenance; manual real-export acceptance remains mandatory.

**Acceptance criteria:**

- Recreate the held-out fixture outside the workspace and produce a real pinned CGA JSON export, retaining exact full raw evidence or an explicitly partial, source-bound projection with its raw checksum.
- Repeat debug-assertions/overflow-checks `(off,off)`, `(off,on)`, `(on,off)`, `(on,on)` at opt-level 0, panic=unwind; add explicit feature-gated contrast. Record actual commands, toolchain, flags and inputs. Each distinct configuration has a distinct identity.
- Verify repeated Option and Result `unwrap`/`expect`, a safe similarly named custom method, chains, generic calls, out-of-line dependency, cfg/feature code, macro-generated calls, Unicode prefix, indexing and addition. Report actual retained compiler observations and any optimizer-elided/unmapped cases honestly.
- Verify source coordinates against the compiler output and bytes; no assumed endpoint convention. Retain unknown macro-expansion associations.
- Runtime good/bad child-process controls use the same declared flags where applicable. No raw MIR-regex prototype is used as a semantic oracle.
- End-to-end negative source/config/checksum/reference/ownership controls reject corrupted inputs; unavailable type/location/coverage remains explicit.
- Focused suites, graph all-target compile, appropriate regressions, formatting and diff checks pass. Run complete graph suite once at final integration if API/build changes justify it; loopback mock tests need local network permission, not source changes.

**Test witness:**

```rust
// The fixture has repeated Option/Result calls and Safe::unwrap. Source row
// count is derived from the actual AST and exact spans; compiler uses retain
// canonical targets, including external core instances. Disabling overflow
// checks changes the build ID and may remove the addition BuiltinAssert.
// Importing d0_o0 evidence as d1_o1 must fail before joining source sites.
```

**Implementation sequence:** PRE from the real fixture/capture contract; fail the end-to-end assertion using prior P2 without executable capture wiring; implement the example and capture script; run all compiler and runtime controls; GREEN and POST; commit/audit exact evidence. The fixture harness must use the repository's absolute `scripts/spur-cargo` with `SPUR_REMOTE=0` even when its working directory is temporary.

**Scope boundary:** No installation over the user's `spur` binary; no default nightly change; no shared graph/cache deletion or rebuild; no third-party service publication; no `.ipynb` disk reads/writes. Root will use the notebook tools to execute the example/capture and save analysis after review.

## Brain acceptance and integration

For each task, verify assigned execution Beads status, durable completion/audit, signals, actual diff, assertion RED and GREEN logs, and reloaded PRE/POST. Request changes for missing real assertions or fabricated semantic certainty. Preserve authored test/fix history using refs or cherry-picks after comparing normalized trees.

After P3 approval, integrate only reviewed deltas into main while preserving all five user edits. Run the final appropriate suite and verify source/fixture inputs. Through Notebook MCP append the executable sidecar experiment, actual coverage/type/configuration results, unsupported cases, costs, raw-export/source hashes, exact commands and solver receipts. Keep historical v11 counts pinned to their original hash. Recheck graph CURRENT/main manifest remained unchanged, commit explicit task paths only, reconcile source and execution issues, and close the epic when all accepted gates pass.

## P3 verification ledger (2026-10-31 UTC)

Execution issue **`bd-1l9`**, delegation `35e65f33e81741a7`, was verified
`in_progress`; P1/P2 were approved in plan
`770b6376-2b19-46bf-9fba-4004ed3e76b3`. The worker leaves the issue open for
brain review. Authored history is preserved:

- Test: `64b3e5981dcf9f8c384ad2ac30a3f196033286f3`
  (`test(spur-graph): P3 assert executable capture and held-out evidence contract`).
- Implementation: `e5451b8f0a23a3fe873d6053b2f2f21034ffa1fe`
  (`feat(spur-graph): P3 run source-bound compiler capture and held-out acceptance`).

The six P3 paths are the example, end-to-end test, corpus JSON, capture script,
crate README and this plan. No P1/P2 implementation, graph schema/storage,
CURRENT, workspace dependencies/toolchain or user notebooks were edited.
Graph-first retrieval was attempted: the knowledge pack failed its analyst DB
query, and exact symbol search reported no graph artifact. Exact native file
reads supplied the fallback; no graph rebuild occurred. Worker `get_issue`
does not expose comments, so producer pins were independently recovered from
approved P2 frozen provenance and checked against the supplied checkout.

**RED:** missing-example compilation was recorded separately in
`/private/tmp/panic-evidence-p3/missing-api.log`; a test-authoring type error is
retained in `scaffold-compile.log`. Minimum empty-response scaffolding then
produced **seven runtime assertion failures**, not compilation errors, in
`red-all.log`: `json_collect_preserves_held_out_multiplicity_and_utf8`,
`json_collect_rejects_stale_sources`,
`json_import_validate_roundtrip_and_explicit_capabilities`,
`json_import_rejects_source_config_checksum_reference_and_ownership_corruption`,
`capture_script_exposes_the_bounded_wrapper_only_command_contract`,
`capture_prepare_recreates_exact_inputs_and_independent_configuration_flags`,
and `frozen_held_out_compiler_witness_verifies_scalar_columns_and_exclusive_end`.

**Fresh compiler acceptance:** full exports and every emitted fragment remain
verbatim under `/private/tmp/panic-evidence-p3/capture-02/`. The script uses the
pinned driver as `RUSTC` through the absolute main-repository `spur-cargo`
wrapper with `--workdir` and `SPUR_REMOTE=0`. It bypasses cargo-cga's implicit
Cargo/build-std launching, while retaining its binary digest as producer lineage.
The supplied producer has the exact retained build.rs rpath patch; no exporter
source modification or installation was performed by P3.

- CGA revision: `2fc63108f0b674fe7a178191ffa15647a18879f1`.
- Rustc: `nightly-2026-10-18`, `1.101.0-nightly`, commit
  `8006091eb71ae7d9e9bccb66c6122c9d68af4466`, host/target
  `aarch64-apple-darwin`; exact `-vV` output is retained.
- Driver SHA256: `5647b7f2c7f857dda0dd0204e91d870287988181ca5ec4d7792810a561886172`.
- Launcher SHA256: `7f84bc715138c09744fd351859ca02b240ea768d9756069229e9250fa9447b7a`.
- Producer patch SHA256: `72f3d4cd64439d988deba0ee1e0f449df8290e6f1d1e0ae83ba82d97b1592aca`.
- Graph content hash: `b6e27b54bc5a4d50833d33ff765c7169032f2688bbb12f1facde04a99e0f0560`.
- Seven-input inventory SHA256 before **and** after every configuration:
  `fde8eab1931a9a77f541c46271d9f8f3e63b35f0eb26d54af3c8c675b8c4242e`.

Every configuration uses opt-level 0 and panic=unwind, with independent explicit
debug-assertions and overflow-checks flags. `d1_o1_extra` enables feature `extra`;
all other captures use no default features and no enabled features.

| Configuration | Build ID | Raw bin-export SHA256 | Observations (exact / containing / unmapped / external) |
| --- | --- | --- | --- |
| d0_o0 | `7259601f616a29794b1e81bd5a2068ef87b43f64a8b8bdc9b6873bfb8be991de` | `27df1702b006375c44c6643ca7fcbf0360ab5ec6a188565fd1630e23dd8ed4cb` | 134 (18 / 1 / 55 / 60) |
| d0_o1 | `33d97393a028ca62026a239d003d374e955aa9e963e38936c8a650fee6c11928` | `6a1920a377d085aaa06ac5f959030b54a50ccdcc40092befc5b9985d5fd9591f` | 135 (18 / 1 / 56 / 60) |
| d1_o0 | `a035376f376336902df2186469826e9704a9a6eac12859b6fe2bfac249c45a52` | `d11a4885b773791cdac643fd8697e76a575d7c1cae14224341138d55ae0f2996` | 135 (18 / 1 / 55 / 61) |
| d1_o1 | `c7f7495f647f0963f2e2c76e0d3a4e459f4441f70f15feb2b6093a31c25874e5` | `ff1ac7f15f02450ad17e2d3a9c4ff9bc0bb5d40b6a658a55737108b7b325c63c` | 136 (18 / 1 / 56 / 61) |
| d1_o1_extra | `d7c6468895cb01b505f5044b4cb12fb7ae94e2b0918ad35752219a66207ec3ef` | `0c7c773aeb17a78b8edec00d49981cb2b3c2e16bff637fa1c69f03df8dc4e0dd` | 137 (19 / 1 / 56 / 61) |

All captures retain four Option calls, four Result calls, two custom Safe calls,
two chained calls, two generic instances at one physical site, one Unicode call,
one macro-produced call, one dependency call and one index BuiltinAssert. The
addition BuiltinAssert occurs only with overflow checks; the debug-control
observation occurs only with debug assertions; the feature call occurs only
with `extra`. The collector retains 18 physical direct method sites. Actual
Unicode raw endpoints select `x.expect("unicode")`: one-based scalar columns,
end exclusive, independently checked against the multibyte-prefix source bytes.

All **45** runtime children used those same compiled executables. Expected
panics exited 101 and reported a panic; good/disabled controls exited zero.
All **25** corruption requests and the cross-configuration substitution exited
nonzero. Brain review subsequently found that the initial source control
mutated a config input and the ownership control used a nonexistent symbol;
those two intended gates were not satisfied by this first capture. The
attempt-2 ledger below supersedes those control claims.
The frozen corpus is explicitly a partial projection: 97 raw node/edge rows
were checked verbatim against full exports; 98 command-log hashes also verify.

**Verification commands:** every Rust command used `SPUR_REMOTE=0` and the
wrapper, with `CARGO_TARGET_DIR=/private/tmp/panic-evidence-p1/target`.

| Artifact under `/private/tmp/panic-evidence-p3/` | Command/result | Log SHA256 |
| --- | --- | --- |
| `red-all.log` | focused P3 test, exit 101; 7 assertion REDs | `966a19a28cc733f13beffc55fc2613cc82f511b3ebfe9adac953666db8c8343a` |
| `focused.log` | P1/P2/P3 + macro contract/extraction, exit 0; 58 passed | `dfcb02ea9ace52893fe7d400e67194444b8da69fd886fbba4274d0e5129ae133` |
| `all-targets.log` | `scripts/spur-cargo check -p spur-graph --all-targets`, exit 0 | `165ce6f7401e96c615e8aad3550e731267c9efdddb01cc2fd12e40741f170672` |
| `full-graph-canonical-tmp.log` | `env -u SPUR_GRAPH_BLESS TMPDIR=/private/tmp scripts/spur-cargo test -p spur-graph`, exit 0; **1,168 passed, 19 existing ignores** | `3466ff165df92064b3deef4b95d5041cc2dce17143890b42944c26328100b358` |
| `format-check.log` | wrapper fmt through a temporary manifest naming only the two P3 Rust files, exit 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `evidence-check.log` | exact raw/projection/inventory/control/log checks, exit 0 | `6da53df542c2efd5fbd26e995526def8752b715af1e658b19bd08d4523b01ef2` |
| `capture-02.log` | fresh five-configuration real compiler/runtime acceptance, exit 0 | `5774b646a5bf84fff335d139aa39a73dbd853d2ed3c57b1bff880652c384da8a` |

An initial full-suite run failed 12 existing tests on `/var` versus
`/private/var` temporary-root identities (559 passed); its command/log is retained
as `full-graph.json`/`.log`, SHA256
`c234ce8d1cc9fbb224405d36839dcdb420251112389596eec569b4b4fac60d43`.
The unchanged complete selection passed after applying the README-documented
canonical TMPDIR. No source workaround, timeout weakening or snapshot blessing
occurred. Risk signal `3627fe65-04af-445c-9ac7-2e15105b4dcd` and resolution audit
935 retain this history for the brain. Existing unrelated compiler/bench
warnings remain in the logs.

**SOLVE:** catalog data-integrity uniqueness, cardinality, references and value
range checks used named finite configuration/input rows; a combined request hit
the 64-variable limit, so identity-preserving projections were used. Exact
requests and complete reloaded receipts are in `solve-pre.json` and
`solve-post.json`, and durable `report_audit` comments **912–919, 925–934** on
`bd-1l9`. PRE IDs:
`sol_4ee36f09ab09443f`, `sol_06e705b026a84c5e`, `sol_7b191c873911451f`,
`sol_ee2ce81e231e4ebd`, `sol_25e01f7576284257`, `sol_6f8b0287e2e24aeb`,
`sol_b9c366a1f07b44d5`, `sol_03a052c6a7c94e8d`.
POST IDs: `sol_85f0dee2216a42f5`, `sol_006e62276e0b4290`,
`sol_7f03dcc4d7904dca`, `sol_370c0329d99a4c07`, `sol_05ed444ea2854640`,
`sol_30fe6ab6c4ae40ba`, `sol_2d88a69bb2fe43a9`, `sol_47abdcf7fa634c8c`,
`sol_f5d9437a91004344`, `sol_f5667fad670e4a90`. Valid projections passed;
duplicate/missing/configuration/reference/inventory negative controls failed.
POST additionally checks actual distinct build/export IDs, unchanged inventories
and nine runtime controls per capture. Solver results prove supplied finite
metadata predicates, not compiler semantics.

**Limits and remaining acceptance:** macro expansion stays unmapped; index/add
assertions have no direct-method site; debug macro locations are external.
Receiver types, assertion reasons and expansion maps remain unavailable.
Optimization/monomorphization can elide or multiply observations; absence is
not safety. Capture is bounded to this standalone package/local dependency and
pinned producer/platform, not arbitrary ambient Cargo or whole-program coverage.
This first attempt required the two control repairs recorded below. Brain
review, root notebook/MCP acceptance and final
integration/CURRENT verification remain with the root, which alone writes the
notebook. The final unified diff, command/exit/hash inventory, changed paths and
authored OIDs are retained in `/private/tmp/panic-evidence-p3/verification.json`
and `/private/tmp/panic-evidence-p3/unified.diff`.

## P3 control repair verification ledger (attempt 2)

Execution issue **`bd-1l9`**, delegation `1bab26aa461d4db3`, was verified
`in_progress` with P1/P2 approved. The issue remains open for brain review.
The staged attempt-1 tree exactly matched retained ref
`spur/panic-evidence/p3-attempt1-authored`; the worker branch now includes its
original test/feature/ledger commits, followed by these authored repair commits:

- Test: `305e4a42bc9bf334691389042990e5a60f7f1a99`
  (`test(spur-graph): P3 assert physical source and existing caller controls`).
- Fix: `5476e18112214570d51c676062f4093744f0d657`
  (`fix(spur-graph): P3 bind negative controls to source and existing callers`).

The repair changes only the capture script, end-to-end test, crate README and
this ledger. The example and frozen positive compiler corpus remain unchanged.
No P1/P2 validators, graph schema/storage, dependencies, toolchains, notebook or
main-tree source were edited. Graph-first retrieval again failed its analyst
query and reported the missing graph artifact; exact native reads supplied the
documented fallback without a rebuild.

**RED:** the two portable capture-construction tests failed against the
unchanged script with runtime assertions, while the other seven P3 tests
passed. `capture_source_control_changes_only_physical_source_with_config_first`
reported `source control mutated config instead of source`;
`capture_ownership_control_uses_existing_different_same_file_caller` reported
`both callers must be unique existing graph symbols`. The tests exercise
`capture()` with only process/producer I/O replaced; this is not compiler
acceptance. Earlier harness-authoring errors were corrected before accepting
this assertion RED. The JSON-boundary corruption test independently constructs
an actual source-byte mismatch and a different existing same-file caller.

**Real compiler controls:** all five configurations were captured afresh under
`/private/tmp/panic-evidence-p3-attempt2/capture/`, with the same pinned producer,
toolchain, explicit flags and seven physical inputs documented above. Full raw
exports are byte-for-byte identical to the retained attempt-1 exports. All
inventories before/after retain SHA256
`fde8eab1931a9a77f541c46271d9f8f3e63b35f0eb26d54af3c8c675b8c4242e`.
New expected build IDs bind the new capture root and exact invocation:

| Configuration | Build ID |
| --- | --- |
| d0_o0 | `5f80d65818d72f30ed1e6c9e5a761334ca3a73ba12bb793e77f5ad2628e8e46f` |
| d0_o1 | `5749cdaf822e38d63cecdd8e0e0121a5a552ee5256510e95628c92fd40f8c055` |
| d1_o0 | `d79e7e2b2ca35ddab9d0c7bae0179f9048a2723aad0c81f767637d79e8313eb5` |
| d1_o1 | `8b2c6e0ad6462dbdd92daaf81a4b19a2fc13ac8c2bf5626b72b42cd43f815354` |
| d1_o1_extra | `d3e81baeaf1392a66353996156b332821d0c5da28ffd58d6d905467169571901` |

In every configuration, the source request changes only `dep/src/lib.rs`
(identity kind `source`), selected by path despite `.cargo/config.toml` sorting
first. Its expected/original SHA256 is
`6936ecc96a26fbc2c7c1ab1bf181a8883bcaa7e7cd8dd65c14deb50ec9e563a6`;
corrupted bytes hash to
`8b7f0a22bfa02565a29aa7d87b96178ab94b7b81008727c9606452891c2d4e64`.
The unchanged expected identity rejects it with
`capture input checksum mismatch: dep/src/lib.rs`.

The ownership request changes only the selected site's caller from existing
function `options` (`49b3dffb0d2e7988`) to existing method `impl Safe::unwrap`
(`a3bea14bc67afc3c`), both unique graph symbols in `src/lib.rs`. Graph, source,
spans and site IDs remain fixed; canonical source-bound recollection rejects
the change. Each negative request has a `.witness.json` containing its exact
changed leaf and selected input or both complete graph symbols. A missing
physical source or suitable existing caller witness fails capture explicitly.
No nonexistent-caller control is counted as this ownership gate.

All **45 runtime children**, **25 corruption requests** and the
cross-configuration substitution passed their expected exit/rejection checks.
The root's unmodified `brain-control-contract.py` (SHA256
`778c7782d5cb0f0707ff86ca9b3fba6bc713f129c451047048e67948d450d4e4`)
passed both assertions for every configuration (**10 assertions**).
`verify-capture.py` independently checked all **98** child command records,
request/stdout/stderr hashes, emitted fragments, inventories, exact mutation
semantics and unchanged positive raw exports; `control-facts.json` records the
observed payload hashes and predicates.

| Artifact under `/private/tmp/panic-evidence-p3-attempt2/` | Result | Log SHA256 |
| --- | --- | --- |
| `red-all.log` | exit 101; 2 assertion REDs, 7 passed | `4085580e103312bae8e0fedfe990bba2fe7bf6a09211b18265f55db757ae7965` |
| `focused-02.log` | exit 0; 60 P1/P2/P3 and macro tests passed | `0e4b86651909abe3b952fac6d7ca6b7cba76efc87760c1a1e6ff9f85e4562d7f` |
| `build-example.log` | local wrapper example build, exit 0 | `e58e9ff2457bb23869591795bbb297b23794ebf5f8d73525238b4fd3bd89d9b7` |
| `all-targets.log` | local wrapper graph all-target check, exit 0 | `47388c8dd2e162eff7bb6b4a488a31530f13d36625b2619db391edb6e03c777b` |
| `capture.log` | five real compiler configurations, exit 0 | `ae285287c6f342e169ab8837e5d3b44a76e713ebdbfd1770f7a7b4e4ff317d8b` |
| `verify-capture.log` | independent artifact/control checks, exit 0 | `fe356ea8368239f44136159f863b247ec4c2d8d367d1fb20598afedd0497ef95` |
| `format-check.log` | wrapper fmt on only the two P3 Rust files, exit 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `full-graph-loopback.log` | identical complete graph suite with loopback permission, exit 0; **1,170 passed, 0 failed, 19 existing ignores**, 90 groups | `d16090723ddaf1066e46c602cb34aecb036015cb65991b9c96b9c0d35b29dc93` |

All Rust commands use `SPUR_REMOTE=0`, the wrapper,
`CARGO_TARGET_DIR=/private/tmp/panic-evidence-p1/target`, canonical
`TMPDIR=/private/tmp` and unset `SPUR_GRAPH_BLESS`. An initial complete graph
run stopped at the existing loopback mock's `TcpListener::bind` with sandbox
EPERM; `full-graph.log` SHA256
`332fb6e163932ee7887ef5c8d307b407e4870a5dccdbfc84a446fe005d2e8edb`
retains the failure. The same complete selection was rerun with loopback
permission, without source changes, skips, blessing or configuration fallback.

**SOLVE:** PRE `sol_a913698a0b2d4ff4` passes the intended finite control
predicates; `sol_60253e9df0b047cc` rejects the prior config/nonexistent-caller
counterexample. POST derives the predicates from each actual landed payload:
`sol_e22f4c7e836744af`, `sol_edb60852587c4014`,
`sol_cdd1d075cfcd4e2e`, `sol_00451f7cbfcb41f8`,
`sol_93099f4c581b4388` all pass. Exact family requests and complete reloaded,
pinned receipts are retained in `solve-pre.json` / `solve-post.json` and durable
audits **953–955, 957–962** on `bd-1l9`. These finite metadata predicates do
not prove compiler semantics. Macro/optimization/unmapped and unavailable-type
limitations remain as documented above. Root notebook/MCP acceptance and final
integration/CURRENT checks remain root-owned.

All P3-owned mandatory gates pass after this repair. The main CURRENT symlink
and active manifest were unchanged during the post-capture verification window
(`main-graph-window-before.json` / `main-graph-window-after.json`); this bounded
check does not replace the root's final integration check. Exact command arrays,
exits, logs, hashes, changed paths, authored OIDs and remaining root gates are in
`/private/tmp/panic-evidence-p3-attempt2/verification.json`, with the repair diff
in `unified.diff` and the complete six-file P3 delta in `p3-complete.diff`.
