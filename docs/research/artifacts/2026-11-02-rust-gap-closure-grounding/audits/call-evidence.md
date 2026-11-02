# Current call/body evidence and persistence seams

Root audit for bd-3cs, completed after the optional worker task bd-36l lost two leases. Research only. Production baseline bf5d146e132cfbb8eb40eb0b1833802c02529923. Scoped raw reads were used after the active knowledge context lookup failed against the analyst DB.

## Reusable APIs, directly exercised

- `panic_evidence/call_sites.rs:20,37,53,65,76,89,107`: RustBodyKind, RustBodyEvidence, RustCallSiteEvidence, RustCallSourceBinding, RustCallSiteEvidenceSet, validate and collect_rust_call_sites. Snapshot-bound body/site IDs are not cross-edit semantic identities. Macro token trees are opaque; parse/cfg uncertainty prevents complete collection. Ownership is not execution.
- `panic_evidence/semantic_facts.rs:54,68,501,528,552`: PanicBuildConfig is an explicit build declaration; typed facts carry sources, bodies, body coverage, values, expressions, control flow, sites and obligations. Canonical validation authenticates automatic collection; a hash does not attest compiler execution.
- `panic_evidence/semantics.rs:34–65`: collect_panic_semantics calls R3 and retains a typed cfg/parse collection obligation when no exact body inventory can be established. Supplying cfg in a build declaration does not evaluate cfg syntax.
- `panic_evidence/analysis.rs:27,41,79,121`: explicit entry/domain/exploration limits, separate bounded outcomes and witnesses whose runtime_reproduced starts false. Raw solver status alone is not the reporting outcome.
- `examples/panic_evidence.rs:51,174,189,208,283,350`: snapshot returns graph plus method_sites; analyze returns semantic facts; solve authenticates canonical facts before narrowing domains. It does not directly expose the general R3 evidence set, hence the tiny public-API adapter in this experiment.
- `panic_evidence/compiler_validation.rs:218` keeps FnPtrCast in address observations. `cga.rs:91,335` preserves the observation category even when references share validation machinery.

The experiment adapter linked the existing rlib using its recorded toolchain and native linker runtime, then called the public collector and canonical validator. Actual results: six sites, five bodies and two sources; one source has zero sites. Four leaf invocations span function, closure and async owners while the coarse graph has one leaf edge. Parquet round-trip validates; five distinct forged/omitted variants reject. See call-transport-results.json and collector-lineage.json.

## Missing durable graph integration

The measured v11 artifact manifest has macro invocation/scope tables and has no R3 method/free-call body/site/source table. R3 APIs existing in memory do not imply their persistence.

`store/build.rs:32–33,252–253,377–405,906–976,2598–2692` show versioning, source buckets, legacy/extractor invalidation, macro validation and composition seams. A call evidence feature must cover each analogous path, including unchanged-file reuse and deletion; a new writer alone would lose data in deltas.

`store/canonical_hash.rs:11–45` hashes a body that includes optional macro evidence and preserves legacy hashes when absent. New call evidence and its availability/version must enter the artifact/checksum contract intentionally. Source-only graph_content_hash does not identify a semantic build or certify sidecar bytes.

`store/cache.rs:950` checks stored macro evidence against expected evidence. Extend equality/cache inputs for call evidence and its extractor. `query_client.rs:157,298,777–814,1939` provide the memory/overlay/Parquet macro query pattern; an equivalent call query needs validated filters, complete file replacement and original source/body provenance.

## Required contract boundaries

1. Persist source manifests even for complete zero-site files. Legacy missing evidence is unavailable, not a complete empty table.
2. Current R3 rejects cfg-bearing sources. Represent per-file unavailable/unknown explicitly; do not suppress the error or synthesize a zero count. A raw syntax collector and a configured executable body inventory are separate contracts.
3. Keep exact site/body/source tuples; an FK only proves the referenced body exists. The solver and production validator both demonstrate the difference.
4. Replace all evidence for an overlaid file and tombstone deleted files, retaining unmodified base sources. Do not merge new sites with old body/source identities.
5. Bind schema/extractor, graph/source, evidence checksum and availability to transport/cache. Bind semantic facts and receipts additionally to the independently expected build and input domain.
6. Retain type/guard/loop/call-depth/opaque-call uncertainty. Expansions, traits and implicit checks need pinned compiler evidence; historical CGA partial captures and address flow cannot establish complete execution reachability.
7. Preserve lexical namespace separately from new extern structural containers. Foreign declarations have no local executable body; zero outgoing edges are not a safety proof.

## Independent runtime check

The source-derived u8 addition detector produced x=255 and its own persisted replay command reproduced the expected native panic. Checks-off and domain 0..254 give complete bounded UNSAT. A macro-generated bounds panic is present in MIR/runtime while the source detector reports incomplete semantics with raw UNSAT. A cfg-selected assertion differs natively by feature, while both source collections stay cfg unknown. These results identify where persistence helps and where compiler/configuration work is still required.

This audit adds no production API, table, resolver rule or full-project panic claim.
