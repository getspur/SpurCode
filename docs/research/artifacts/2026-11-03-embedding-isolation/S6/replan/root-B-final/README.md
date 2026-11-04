# S6B independent final review and S6C PRE

B's host integration is supported for approval on macOS ARM64. The final stdin fix returns immediately for an empty `ReadBuf`; its committed assertion RED precedes the fix. This packet adds no production changes. C's real-model analyst acceptance remains pending.

Reviewed authored tip: `8b61df7120700c357fb8e111aa765cf48debcecb`. Engine wrapper: `0f7a11f7c2286322bc427f3fe8a393d17539a870`, with the same tree. Test commit `898e47fe2` precedes fix `b8897ede1`. The worker's frozen `stdin-final` packet and earlier failed signal observations remain intact.

## Independent runtime checks

`run_checks.py` executed fourteen commands using `scripts/spur-cargo`, `SPUR_REMOTE=0`, `TMPDIR=/private/tmp`, and the isolated target `/private/tmp/rust-gap-g1-acceptance-target`. All final command exits are zero. The ten test commands report 132 passing tests; the signal integration test contains eight separately recorded process scenarios. The two no-default-feature library checks, default normal/build dependency inspection, and workspace formatting also passed. Exact commands and scoped exclusions are in `checks.json` and compressed logs.

The initial dependency parser incorrectly matched `ort v` within `gix-transport v`. The original dependency output was preserved, and an exact package-token parser with three injection controls found no `fastembed`, `ort`, or `ort-sys`. This was an evidence-script failure, not a production dependency failure. `project_models.py` initially checked an inaccurate diagnostic string; the final assertions accept the two implemented error paths, signal termination and visible shutdown failure. No production code or runtime observations were changed to satisfy that check.

`observe_hosts.py` independently repeated twelve host/mode/cancellation scenarios and two compiled CLI controls. Logs bind executed binaries, source, fixture bytes, requests, PID observations and process exits. Local callback/worker/CLI/socket cases reuse one helper for warm plus query. Off and Sidecar-with-missing-service start no local helper. Auto uses a reachable external v1 peer or the local helper as appropriate. v1 inputs are translated once. Cancellation and missing-helper cases leave no recorded child alive; completed compatibility results retain visible cleanup-error diagnostics. The deliberately invalid analyst DB in B's process fixtures validates error-path lifetime handling, not a successful analyst pack response.

All eight compiled CLI signal scenarios returned exit 1 with a visible interruption or shutdown error and no recorded fixture PID alive **before** observer cleanup. The observer's later process-group kill does not establish a passing result. Normal stdio EOF and the public embedding service's ordinary shutdown control return zero. These are protocol fixtures, not neural inference.

One real-model-dependent pack unit, `knowledge_context_pack_reports_confidence_from_real_hybrid_fusion`, was explicitly deferred to C. It is neither a B pass nor an established member of the eight separately preserved direct-native abort baselines. Windows compilation did not reach a compiler verdict because prerequisites were unavailable; Windows interrupt/lifetime behavior is unverified. Arbitrary custom blocking callbacks must eventually return for joining to finish.

## Fresh Jev and solver evidence

The current catalog supplied `data_integrity.mutually_consistent`, `data_integrity.unique`, and `data_integrity.cardinality`. All seven Jev interpretations opened their gates. Their tuple rules, facts, verify mode, subjects and lack of unknowns were checked without alteration; catalog uniqueness and exact row-count rules were then added explicitly. Every solve was persisted, pinned and reloaded through MCP. Exact requests, Jev provenance, responses, lowered persisted requests and models are in `proof-*.json.gz`.

| Stage | Positive snapshots | Rejected corruptions |
|---|---:|---:|
| Whole-B POST | 5 | 15 |
| C PRE requirements | 2 | 12 |

B positives: `sol_52c1af6c1b184a91`, `sol_788907713a2146c1`, `sol_54dea70f94cb4051`, `sol_1bf95b98844f4683`, `sol_2866e33f73e24af6`. C positives: `sol_cb7e6d39a34045d4`, `sol_88df85bf7ada49a0`. All receipt IDs and outcomes are also in `receipts.json`.

`verify.py` reconstructs source-bound projections, independently checks each finite tuple/uniqueness/cardinality predicate, checks all fixed bindings in the reloaded lowerings, and evaluates all 102 lowered hard predicates. Positive models equal their complete assigned facts; negative snapshots disagree with the declared contract. These are finite evidence checks. They do not prove all concurrent schedules, native memory safety, or neural equivalence.

## C acceptance handoff

`projection.json.gz` contains the three exact reference hashes, model artifact fingerprints, inputs and byte budgets from approved S4C. C must obtain **fresh analyst-path** vectors; archived reference vectors remain an oracle only. The requirements reject swapped profiles/references, fixture or archived inference, missing/duplicate models, unjoined owners, descriptor-only warm-up, replaced generations, nonzero normal shutdown and late publication.

Use Nomic, CodeRank and Jina with their existing mean pooling/unit normalization/legacy-preformatted input contracts and 768 dimensions. Preserve existing model-specific tokenizer and artifact fingerprints, maximum length 8192, and the existing S4C comparison tolerance of `1e-4`; this packet introduces no shipping defaults. Run two independent owned helper lifecycles per model, observe reuse within each, and close/join them. Bind all new vector observations to source/build/runtime/model identities.

C must also produce a successful analyst/pack response against a valid indexed fixture, run or narrowly migrate the deferred real hybrid-confidence fixture, and retain failure/mode/socket regressions. Packaging and the final application/notebook experiment remain later stages. The canonical C execution issue is `bd-1ktd`; `bd-4ha3` is tracking only. Actual C execution and root POST are required before claiming the analyst migration complete.

Replay the offline packet:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-03-embedding-isolation/S6/replan/root-B-final/verify.py
```
