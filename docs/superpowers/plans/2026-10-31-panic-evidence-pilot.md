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
