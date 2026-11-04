# Final-source real analyst acceptance — worker evidence for bd-eha0

This packet is for canonical bd-eha0, plan d155a8d0-540b-4a8c-873a-a31e30a4b663, delegation db813b81d91c4f4f. It does not approve the issue. Root must independently validate the actual outputs, execute fresh measured POST, attach its fresh packaging PRE and exact integrated source handoff, and review this node before packaging bd-3pcp is released.

No production or Cargo files changed relative to a5c278c2485c0eade76b572c6ae82c71dc85890f; `production.diff` is empty. The 29 source files match fingerprint `4550b8534529cf6b745e1d231b0b75a71a34d7a6aa78519f77fcd2b650a37d9d`, the SHA-256 of sorted compact JSON of root's source hash map. The exact original approved graph fix is retained. The immutable ordinary review import is e7ca1b66df9b7c15b480b7cdc616c854e045c19c; its verifier exited 0 before experiments (197 analyst and 40 client passes, 29 source hashes, 21 receipts and 63 predicates). All 13 task PRE receipts were reloaded through get_solve_result: two SAT and eleven UNSAT. Their exact interpreted requests and results remain in the imported root `proof.json.gz`. PRE encodes requirements and does not claim a measurement.

## Fresh case ledger

All six required named case types were executed on the final source, rather than counting archive equivalence:

| Case | Actual fresh evidence |
|---|---|
| `knowledge_context_pack_reports_confidence_from_real_hybrid_fusion` | `confidence-*`: one passed; strong 0.991935, weak 1.806; real helper returned Closed and was waited |
| `warm_query_and_v1_multi_text_share_owned_runtime` | `v1-*`: one passed; two separate owned lifecycles, four descriptor/service child returncodes zero; `verify_fresh_v1.py` compares all six vectors to the pinned Nomic reference |
| `real_cli_host_serves_indexed_context_pack` | `cli-*`: one passed; actual `knowledge_context_pack_2` RPC answerable, high confidence, hybrid-code; separate fixture and CLI owners closed and four child returncodes zero |
| `real_owned_helper_returns_indexed_context_pack` | `direct-*`: one passed; indexed pack answerable, high confidence, hybrid-code; two distinct owners closed and four child returncodes zero |
| `graph_real_batch_reuses_and_closes` | `*-graph-*`: fresh Nomic, CodeRank and Jina; two lifecycles/model, repeated distinct-input batch and clean shutdown |
| `owned_real_query_reuses_and_closes` | `*-query-*`: fresh Nomic, CodeRank and Jina; two lifecycles/model, warm and repeated query on same owner and clean shutdown |

`verify_models.py` independently checks raw vectors, pinned reference SHA, length 768, finite values, unit norm within 1e-4, per-component reference difference below 1e-4, descriptor and service model/tokenizer identity, generation and sequence, distinct owners, Closed, actual spawned/waited PID sets and zero Python child returncodes. Maximum observed difference is 4.1419983e-9; maximum norm error is 3.700775e-7. The Nomic v1 vectors have norm 0.9999996299225447 and max difference 3.7066078e-9. Each `*-identity.json` retains before and after source, model, helper, runtime and test-binary hashes, commands and subprocess return codes. Python subprocess returncode is not a raw waitpid status; the latter was not observed. A zero waited returncode represents a reaped child, not an invented raw wait status.

The v1 handler calls the internal helper sequentially with single texts and deliberately repeats the query; no native multi-text frame is claimed. The distinct two-text graph batch uses a different owner from the analyst query. The helper proxy logs actual PID, proxy PID, generation and monotonic request IDs. The client constructs a protocol Session with the worker generation and validates each request/reply against it; consistent PID/generation/request sequence is the observable session reuse evidence, not a separately captured memory address. Socket transport was covered separately by the unchanged ordinary/host evidence.

## Commands and identity

Normal helper and CLI builds ran separately, then lib and integration test executables were built, always one Cargo writer, with `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_ORT_ARCHIVE=/private/tmp/spur-embedding-isolation-assets/onnxruntime-osx-arm64-1.22.0.tgz` and `scripts/spur-cargo`:

- `build -p spur-embedding-worker --bin spur-embedding-worker`, exit 0.
- `build -p spur-cli --bin spur`, exit 0.
- `test -p spur-analyst --lib --no-run --message-format=json`, exit 0.
- `test -p spur-analyst --test real_owned_acceptance --no-run --message-format=json`, exit 0.

The runners execute the resulting exact test binary directly, with explicit supervised helper configuration and fresh output paths. `run_fresh.py` handles `confidence`, `v1`, and `cli`; `run_direct.py direct` handles the direct pack; `run_models.py <model> <query|graph>` handles the six model/case pairs. Each recorded exit is 0. All runtime tests used authorized Unix socket/process access and `/private/tmp/s6c-duckdb`; the models were read from the specified existing caches. Before/after snapshots were equal for every run. Archive SHA-256 is `cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07`; dylib `2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`; normal helper `2c44ca9141058a218105769af7b5c11a1e1aca2cd18b1ac741466d4ba17620df`; CLI `46fdbca2009b1db78f11ad69515baa0e73ad0a4564f0c4271d03e712bc1aadc3`; lib test executable `7b425f89304baf7bc0a91e5b5a4af5151b25a1e6b972b9e0bc5928d048d48618`; integration executable `082864609f393c8163142c518742be7088c26505356b6f4499fd36b1432abf6b`. Model file, tokenizer, reference and source hashes for all three are in their individual identity and verification records.

## Independent checks and controls

From repository root:

```
python3 -B docs/research/artifacts/2026-11-04-analyst-recovery/real-acceptance/verify_all.py
python3 -B docs/research/artifacts/2026-11-04-analyst-recovery/real-acceptance/check_model_controls.py
python3 -B docs/research/artifacts/2026-11-04-analyst-recovery/real-acceptance/generate_post_candidates.py
```

All exited 0 after the final changes. `post-candidates.json` is a derived input packet for root, not a Jev or solver receipt. Controls reject a corrupted vector, nonzero child returncode, descriptor or service generation mismatch, changed before/after identity, missing child wait and missing case. The initial negative-control failure is preserved in `control-red.log`/`.exit`: it exposed that the independent verifier had not checked descriptor generation. The verifier was corrected; it now checks each descriptor Ready generation against its launch argument and verifies its model/tokenizer fingerprint. No production fix was made.

Old frozen checkpoint verification was run against the source-equivalent preserved worktree, not by modifying its verifier. Historical all-model query, graph and direct pack evidence remains available with its original identities; it is not relabeled as current. Historical query test-binary identity was incomplete, and risk signal d5b57ec9-56d0-4d4d-81ad-b0c601597e7b was recorded. All affected named cases have since been freshly rerun; root must assess and clear that signal. Existing ordinary evidence covers the named native-free and lifecycle boundaries on unchanged source; this packet does not claim a new complete core suite. Eight legacy direct-native graph abort baselines, Windows, packaging and notebook work remain outside this result.
