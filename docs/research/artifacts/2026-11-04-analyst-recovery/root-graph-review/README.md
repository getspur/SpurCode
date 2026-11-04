# Graph FTS root review and ordinary validation PRE

The scoped graph-query fix is supported for approval. Authored tip `36d466ac88e872c6d42f12c657c97dd4a4deab57` and engine wrapper `8d3fb7c9462529150701b459efe163c9e23473c8` have identical tree `9ba0f4f106cbc3cfd84168790d14feb844d4844a`. The only production change is explicit per-connection FTS loading, with contextual errors, in `graph_candidates.rs`. Existing query/API/limits and `with_conn` behavior remain intact.

The committed existing regression was executed RED before the fix: `b5c3fb24b` precedes `b5d1b5978`, with missing FTS `stem`. The worker then recorded one focused pass and the complete 24-case search suite. Root independently reran **24 passed, zero failed/ignored/filtered** in 20.23 seconds, plus formatting, scoped source diff checks and the worker evidence verifier; all four command exits are zero. Exact commands and compressed logs are in this packet. No model or Windows measurement was performed here.

Fresh Jev POST accepted the measured context/graph FTS rows: `sol_4e1ece5c1ed14191`. Four corrupted snapshots rejected. The finite contract applies to the tested isolated extension directory, whose helper disables autoload/autoinstall; it is not a general program-safety proof.

Fresh ordinary validation PRE accepted the declared final-source contract: `sol_60ee99a3908c4b01`. Six negatives reject suite failure, graph skipping, invalid PID evidence, native dependency leakage, an unbound source and missing graph coverage. This PRE is a requirement for the next worker, not an already executed ordinary suite. It explicitly separates the deferred real cases.

`proof.json.gz` preserves both Jev interactions and catalog cards, all twelve exact requests/results and pinned/reloaded lowerings. `verify_review.py` reconstructs measured FTS facts from source/test evidence, independently checks 36 lowered hard predicates and binds sixteen source files. `receipts.json` lists every ID. Thirteen cache pins belonging to superseded, fully archived C2 snapshots/C3 controls were released to allow persistence; active graph PRE and these new receipts remain pinned. Old archives are preserved at `2d34149ed0cfcdd647dd4e9cd00dd2b16050233d`.

The ordinary node is canonical `bd-2jqb`, followed by real-helper final node `bd-eha0`. Their production source must match fingerprint `f1cbef28ee20baca309a7617d01975fcae565a8e39169f4345688c7ea1e1815d`; compute it as SHA-256 of sorted compact JSON of the path-to-SHA-256 map in `source-binding.json`. The worker may import this exact docs-only packet if absent from its dependency overlay. Do not replay graph/B source commits or alter the frozen packet.

Ordinary validation must include the graph regression and all ordinary analyst targets, feature-enabled client tests with meaningful PID probes, and all four native-free product closures. Every ignored/excluded real case must be assigned explicitly to the final gate. Root runs fresh POST before approving ordinary validation. Packaging remains blocked on `bd-eha0`, followed by packaging's own fresh PRE and exact approved source import. Neither old C nor whole S6 is approved by this graph fix alone.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-04-analyst-recovery/root-graph-review/verify_review.py
```
