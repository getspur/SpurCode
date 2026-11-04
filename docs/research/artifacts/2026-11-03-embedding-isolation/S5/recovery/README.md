# S5 checkpoint recovery

Checkpoint `2814a3f5935b34a119efbe0ec760b51f7a8b49ab` is protected at `spur/review-evidence/embedding-s5-expired-authored`. It includes the original test-first source history and frozen evidence. Root's branch is `spur/review-evidence/embedding-s5-recovery`; these recovery files change documentation only.

Plan `b3135858-c6f8-4c51-a360-3f31a0766440`, canonical task `bd-39y0` / `embedding-graph-routing`, escalated after delegation `d75e247efed54eb6` lost its lease. The plan projection reported zero running workers at recovery. The substantial implementation is preserved but not approved. The next dispatch is narrowed to checkpoint replay, missing verification and evidence finalization, with the existing DAG and codex/Terra/xhigh routing retained.

Fresh Jev PRE produced two open gates. Both intended policies passed and ten adversarial mutations were rejected; every receipt was persisted, reloaded and pinned. `pre-proof.json.gz` contains inputs, interpretations, catalog specifications, family requests, lowered requests and responses. Run the read-only finite-policy/source verifier with:

```sh
python3 docs/research/artifacts/2026-11-03-embedding-isolation/S5/recovery/verify.py
```

| Case | Expected | Receipt |
|---|---|---|
| dependency_boundary | sat | sol_35c03c7a4a134926 |
| publication_policy | sat | sol_bfd0bb7bd34848a4 |
| graph_keeps_native | unsat | sol_acfe34183f1e4895 |
| graph_native_fallback | unsat | sol_b4bb0f88dba14ba5 |
| worker_links_database | unsat | sol_8404c975b8104545 |
| disabled_spawns | unsat | sol_01e2cac80e174ff0 |
| error_exit0_publishes | unsat | sol_d3fea97e5db64f24 |
| pending_abort_publishes | unsat | sol_8269558dacbe4055 |
| later_abort_erases_prior | unsat | sol_3d57cab228a84175 |
| wrong_identity_publishes | unsat | sol_ee9000760c90459c |
| nonfinite_publishes | unsat | sol_e12d5aaa14164a7c |
| truncated_exit0_publishes | unsat | sol_517f8f6e71854cde |

The dependency model distinguishes Arrow/Parquet storage from linked DuckDB/LanceDB runtime packages. The saved graph tree uses the former and excludes native inference and these runtime packages; the worker owns model files and native inference. The old proposal remains in `../root-pre/`, and its correction remains in `../worker/post-interpretation-note.md`. PRE models are prospective requirements, not actual-output POST approval.

Eight failures remain in the saved standalone native wrapper suite (43 passed, eight failed). Seven relocated test sources and three fixtures are recorded as byte-identical to the approved base. A matching historical shutdown signature does not prove the same cause or waive the failures. Keep the assertions, failures and logs visible. S5 as a whole is not GREEN.

Graph publication/fault/disabled checks and real-model exports already exist under `../worker/` and `../worker-partial/`. Inspect them before repeating costly inference. Rerun neutral-client tests that failed because process inspection was denied with the required permission, plus graph publication and all-target gates against recovered source. A clean child exit after an error cannot authorize a pending batch; preserve earlier validated output after a later failure.

Analyst still consumes the old graph loader API. S6 owns its migration and native dependency removal. Packaging, whole-CLI and notebook acceptance remain later tasks. This recovery does not establish a production-ready CLI.

In the newly assigned isolated worker checkout, verify an equivalent approved base then replay the full `d2956a1fb604a1467829ace828d26238ab29e4d9..2814a3f5935b34a119efbe0ec760b51f7a8b49ab` commit chain, preserving test-before-fix history. Import only this recovery directory from the root recovery commit. Compare production bytes with the checkpoint, verify missing gates and commit evidence early. Do not modify dirty main, expired checkouts, global runtimes/caches, live notebooks, the external notebook project or graph/CURRENT.

Use `scripts/spur-cargo` with authorized local environment and one target writer. Root must execute fresh Jev POST from actual dependency, sidecar and process observations before approval. Leave `bd-39y0` in progress for review.
