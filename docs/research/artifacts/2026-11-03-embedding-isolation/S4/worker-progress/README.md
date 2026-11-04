# S4 worker progress — incomplete

Issue `bd-2dhg`, delegation `10a0cc37d8a64bc5`, dispatched base `65189c90c146bc6bc3d972f5b88b076b729ef19c`. No production implementation or GREEN is claimed. Root POST remains required after actual implementation and measurements.

The imported `root-pre/verify.py` passed with three open Jev interpretations, three positive and ten negative receipts, five candidate-source bindings and five local Nomic asset bindings. All thirteen listed solver IDs were also reloaded through `get_solve_result` and their SAT/UNSAT statuses matched. This is prospective evidence only. The cache-entry witness does not establish an artifact-byte budget or a native RSS ceiling.

Additional root PRE requested via `report_progress`: `resource.request_within_limit` with measured Nomic artifact sum 548026095 bytes, caller-declared limit 548026095 for the positive and 548026094 for the negative; checked sum overflow must reject. The expected predicate is actual artifact bytes <= caller limit before materialization. It does not assert a hard bound on native RSS. Await root's Jev interpretation, inspection, catalog SOLVE and persisted/reloaded receipts before implementing that new bound.

## Observed RED

Both commands used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target SPUR_ORT_ARCHIVE=/private/tmp/rust-gap-g1-acceptance-target/debug/build/spur-graph-6801f523f6a15259/out/onnxruntime-osx-arm64-1.22.0.tgz scripts/spur-cargo`.

| Commit | Command suffix | Result |
|---|---|---|
| `5190d72799f76b727773b9f03939e6645a072583` | `test -p spur-embedding-worker --test embedding_models -- --nocapture` | assertion RED, exit 101: worker treated `--describe-model` as an integer and returned ParseIntError, with no identity frame. |
| `eb3b98d76d0398a68d01ecc60018e41d322fed24` | `test -p spur-embedding-worker --test embedding_models hello_rejects_unverified_artifact_identity -- --nocapture` | assertion RED, exit 101: worker replied Ready to an all-zero model and tokenizer fingerprint. |

The second test uses the old seven-argument worker interface to expose the inherited defect; it must be adapted if the authorized implementation adds a required caller budget. Inherited vendor warnings were emitted during both test builds.

## Asset investigation

The five Nomic assets in `root-pre/grounding.json` were verified by the root PRE verifier. CodeRank and Jina assets were absent from that cache root. HEAD requests with Python `urllib.request` returned HTTP 200 for `jamie8johnson/CodeRankEmbed-onnx` revision `151669b173750250e611e1be00c812f112ea6020` `onnx/model.onnx` (content length 547715715) and `jinaai/jina-embeddings-v2-base-code` revision `main` `onnx/model.onnx` (content length 641517466); no files were downloaded. A sandboxed attempt failed DNS; the authorized network retry succeeded. Neither is a real inference result.

The locally cached FastEmbed 5.8.0 source specifies Mean pooling for Nomic and Jina, normalized default output, tokenizer max length limited by its config, and the four tokenizer/config files. Its registered Nomic and Jina metadata both list `onnx/model.onnx` and no additional model files. Any implementation must still identify actual files used and account for external weights if present.

## Attempt 2 continuation

Current delegation is `dcd5d3d661e84bed`; issue `bd-2dhg` is `in_progress`. The supplemental root artifact-budget archive was imported unchanged and its verifier passed. `get_solve_result` reloaded all 15 PRE receipts: 4 SAT and 11 expected UNSAT; the two supplemental lowered requests compare 548026095 with 548026095 and 548026094 respectively. This remains prospective policy, not a native RSS bound or implementation proof.

The prior two assertion REDs were reproduced with the command above (exit 101). A new macOS acceptance test invokes the existing registered FastEmbed Nomic reference on two already formatted inputs, checks two finite 768-element rows, drops the model and successfully calls the checked environment release before requesting worker descriptor identity. It then expects two client batches to agree with that reference and a successful worker shutdown. On the unmodified worker it fails at the missing descriptor (exit 101, ParseIntError); it does **not** yet exercise worker inference, prove reuse, or demonstrate a successful worker exit. Run it with the same environment and command suffix `test -p spur-embedding-worker --test embedding_models product_nomic_matches_registered_reference_and_closes -- --nocapture`. Repeated worker lifecycle, observed cache generation, budget rejection and POST remain outstanding.
