# Root review of CGA isolation

The isolation dossier in parent commit `e0e6ebef676fc51c2fffc8c7581dbfdeab54e72d` was reviewed and integrated. Root then ran all seven controls through Notebook MCP with fresh scratch packages/targets and the unchanged pinned producer. Every expected outcome reproduced. The parent report's `in_progress` value is its historical handoff state; the final issue decision is recorded in Beads `bd-3qw` after the root gates finish.

| Control | Actual Cargo exit | Fresh graph | Meaning |
|---|---:|---|---|
| Original four-line input, analysis/export enabled | 101 | No | Exact `drop_glue::<S>` ICE |
| Same input, analysis enabled/export disabled | 101 | No | Disabling serialization does not bypass the fault |
| Same input, CGA input unset | 0 | No | Analysis-disabled control |
| Same input, pinned ordinary rustc | 0 | No | Ordinary compilation succeeds |
| Static dispatch | 0 | Yes | Concrete `Trait<()>` implementation |
| Nongeneric trait | 0 | Yes | Concrete virtual-call traversal |
| Generic argument passed by reference | 0 | Yes | Incorrect unresolved `Trait<S>` remains |

These are controlled compiler observations, not runtime tests or a production fix validation. The original source and flags are preserved. `run.py` uses `SPUR_REMOTE=0 scripts/spur-cargo`; the notebook launched that harness and reaped actual exit 0 after its seven case assertions. The case exits are `101, 101, 0, 0, 0, 0, 0`. Root's source HEAD during this fresh run was the integrated evidence commit `e0e6ebef676fc51c2fffc8c7581dbfdeab54e72d`, while the worker's earlier CLI reproductions used `a9ee997c9`.

The pinned CGA function `dyn_object_call_to_impl_methods` ignores concrete `_args` and reconstructs trait arguments using identity parameters. A nongeneric implementor self type passes its generic-type filter even when the trait argument remains generic. Actual traces show concrete `Trait<()>` becoming an implementation with `S/#1`, then an unresolved drop-glue request. The actual CLI trace follows the same path through Octocrab's `AuthHeaderLayer` implementation of `Layer<S>`. The earlier root-collector hypothesis was not the failing stack frame.

Root independently matched three archived CGA files to the exact upstream Git blobs and three dependency excerpts to cached `.crate` bytes whose hashes match Cargo.lock. The pinned rustc source snapshot here is gzip-compressed; its recomputed Git blob is `1a00293f8ffc6c114d566ebdea2df0e372528c35`. Its `resolve_drop_glue` calls `expect_resolve` under `TypingEnv::fully_monomorphized()`, consistent with the observed failure when `S` remains unresolved.

`root-controls.tar.gz` retains 65 raw evidence members without target directories or binaries. `root-controls.jsonl` and `root-controls.parquet` contain the seven-case ledger. NULL in `unresolved_generic_in_export` means no graph exists for that control; it does not certify concrete substitutions.

The worker audit RPC failed assignment validation. Root persisted the complete exact request through its authorized issue API and reloaded it equal in Beads comment 1012. [audit-repair.json](audit-repair.json) records that repair without claiming the rejected RPC succeeded. The five protected user-file hashes, original producer and published graph/manifest remained unchanged.

## Replay

```bash
python3 docs/research/artifacts/2026-11-01-cga-isolation/review/replay.py
```

This verifies the parent dossier (138 archived members), root's fresh-control evidence (65 members), both payload indices, source/producer identities and the recorded protection checks. It needs no compiler, network, mutable target cache or live Beads database. Notebook cell `8bc45c58-3884-42cb-b6cb-2346811fb9a8` runs the same durable replay; the preceding SQL table displays all seven fresh results.

## Concrete follow-up regression requirements

A later CGA fix must preserve concrete trait/method arguments while resolving each implementor's substitutions. Skipping the implementation or swallowing the drop error would leave a coverage gap. The regression should require the owned-value reproducer to compile with analysis enabled and export its concrete `Trait<()>` implementation. It must also require the reference-value control's export to contain no unresolved type parameters. Static-dispatch and nongeneric virtual controls should remain correct. A successful full CLI capture is a subsequent validation gate. No such fix or CLI success is claimed by this isolation work.
