# ACP upstream contract fixes

Approved scope: `bd-a6kw6` and `bd-1d09o`, discovered in the 2026-09-29 upstream audit. Keep production changes in `spur-acp`, with agent-client-protocol 1.0.1 and schema 1.1.0 unchanged.

Model switching must preserve the identity of the configuration option selected by `SpurAgentCaps::model_option()`. The category identifies its purpose for the UI; the option's opaque ID identifies it on the wire. Preserve vendor direct dispatch and the full response snapshot.

Native connections use schema v1. Initialization must reject every negotiated version other than `ProtocolVersion::V1`, report the unsupported version, and finish the existing bounded shutdown before returning the error. Do this before caching session capabilities or setting Ready. V1 initialization must continue to work.

Regression coverage must exercise the real native connection and SDK against a small stdio protocol peer: standard and vendor model IDs, category preference when a legacy option coexists, response snapshot freshness, V1 acceptance, V0/V2/u16::MAX rejection, subprocess cleanup, and prevention of new sessions after rejection.

Use JEV before and after implementation. Inspect its gate, selected catalog rule, bindings, and mode; pass the candidate through `solve_rules`. Configuration ID compatibility uses `configuration.attribute_allowed_pair`. Initialization uses `workflow.safety_invariant`, augmented with catalog initial-state and transition checks. These are finite source-normalized models, complemented by runtime regressions.

No dependency upgrade, new agent quirks, new shutdown timeout, or changes to permission, notification, or prompt behavior are required.

Cleanup scope: rejection awaits the existing bounded shutdown. The regressions confirm that the cooperative test peer is reaped before rejection returns. They do not establish that guarantee for peers that ignore EOF/SIGTERM; the existing timeout behavior is unchanged.

PRE evidence: Jev 1.13.0 opened both gates (weakest confidence 0.94 for ID compatibility and 0.93 for workflow safety). Current model-ID verification failed in `sol_e4d33d4e295e4d1e`; intended ID preservation passed in `sol_b361a91a05b841d2`. Current initialization safety failed in `sol_2cab9f49734f430f`; intended disconnect trace passed in `sol_4e67538af25d407b`. The workflow execution explicitly augments Jev's single safety binding with catalog initial-state and transition rules. Full inputs, provenance, execution changes, and results are retained at `.spur/scratch/acp-upstream-fixes-2026-09-30/jev-pre.json`.

POST evidence: after all seven native integration regressions passed, Jev 1.13.0 opened both gates (weakest confidence 0.97 and 0.94). The implemented ID preservation passed in `sol_b2bfcb2b57c24b6c`; the implemented rejection/cleanup trace passed in `sol_4d140ade591f4f30`. Original invalid fixtures still failed in `sol_6a9e4b6f28b04709` and `sol_e493a2ad18df4bad`. Results were returned from the validated solver cache for identical encoded constraints. Exact Jev inputs, provenance, execution changes, domain requests, and outcomes are saved at `.spur/scratch/acp-upstream-fixes-2026-09-30/jev-post.json`. These finite checks establish ID-pair compatibility and safety of the two-step initialization model; they are not an unbounded proof of the crate.
