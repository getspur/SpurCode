# M4 independent root review and complete local stage-A POST

Reviewed `bd-2hgz`, delegation `baa51dff3b96439b`. The resolver confirms
**codex / gpt-6.1-sol / xhigh**. Authorized INTENT/replay, completion audit,
in-progress status and absence of unresolved signals were verified first.
Normalized tip `411c23d6956654baba9dcb0656e1dcb9c43e9d7b` exactly matches authored
evidence tree `a507d1921`; both and all test/fix milestones are protected under
`spur/review-evidence/embedding-sol-m4-*`. Entry was root-approved `0b8fbcd84`.

The eight-file source/test diff adds a pure neutral planner, one admitted batched
client call and the graph model-handle adapter. It uses actual serde string and
envelope sizing, maximum u64 request-ID overhead, UTF-8 limits, checked sums and
ordered ranges. Entire input admission happens before helper work. A later
chunk failure returns an error rather than successful prefix rows. One effective
deadline and the existing completion/close gates cover the logical call.
Existing supervisor methods are byte-identical after masking only the new batch
method and error variant. No native/model/analyst owner source or numeric default
changed. Existing graph outer per-row progress semantics remain intact.

The historical verifier's absolute reference paths pointed into the removed
worker worktree. `historical-replay.json` records the original failure and a
read-only mapping to byte-identical reference files in committed `0b8fbcd84`.
Root replayed the source/output projection before shared-target rebuilding,
without changing historical captures, candidates or source. Historical compile
RED, process-access failure and the corrected exact-float test remain preserved.
The stale graph index lacked the new planner; review used current committed bytes.

Fresh commands used `scripts/spur-cargo`, the approved local environment and one
Cargo writer. `run_checks.py`, stage JSON and hashed compressed streams retain
exact commands, environments, timings, outputs and exits.

| Fresh check | Result |
|---|---|
| Graph store/model/progress/publication/disabled tests, including seven new batch groups | 66 passed; 3 existing download tests ignored |
| Planner reference replay through real Rust writer | 19 cases /113 chunks; also included in neutral suite |
| Neutral protocol/policy/supervisor/planner suite | 48 passed, none ignored |
| Graph discovery/compatibility/cancellation | 12 passed, none ignored |
| Analyst modes/socket/owned-runtime integration | 29 passed, none ignored |
| Analyst embedding units | 9 passed plus one nested subprocess assertion; one existing real-sidecar test ignored |
| Client/graph/analyst all-target check and scoped formatting | exit 0 |
| Ordinary helper and explicit real tests | no test features; two tests passed, three models |

The new real runs use private copies of all 15 pinned model artifacts and refs.
Nomic, CodeRank and Jina produced ordered finite 3×768 vectors; maximum reference
differences were **1.0058284e-7 /0 /0**, within approved tolerance1e-4. Different
batch sizes and the neutral path also agree within that tolerance. All checked
helper closes succeeded; neutral traces contain normal raw wait status0.
`normal-actual.json`, `normal-batch-actual.json`, `normal-observation.json` retain
actual vectors, identities, model revisions, process events and artifact hashes.
Ordinary helper SHA256:
`9463cc1a394ac4dc2fd49911933344344cb22f5d4c564215abc7f2d95f635cfa`.
Private native bytes match pinned SHA
`2b885992d3d6fa4130d39ec84a80d7504ff52750027c547bb22c86165f19406a`.

The shell used by the Cargo wrapper stripped the initial DYLD trace setting;
that empty trace is not evidence. Root then executed the copied native graph
host **directly**, with the setting applied inside Python. Actual inference and
checked exits passed; its **853-line** loader trace contains no ONNX Runtime.
Normal/build dependency trees for graph, analyst and CLI exclude native inference;
the ordinary helper tree excludes DuckDB. Exact helper/host linked-library and
undefined-symbol inspections are also captured. These are actual macOS ARM64
test-host observations. Full installed/default CLI validation remains M5.

`ground_stage_a.py` binds all **seven original requirements** to fresh runtime,
dependency, source-preservation and native-boundary evidence. The 32 actual
batch captures contain 141 emitted chunks and 423 row/raw/serialized-body
comparisons. Root Jev POST gates opened at .97/.97, with exact facts, first rule,
verify mode and no unknowns inspected. Uniqueness and cardinality were added from
the inspected catalog, preserving the complete original model and all nine
negative controls. `post-proof.json.gz` contains **12 persisted/reloaded receipts:
2 SAT and10 expected UNSAT**, including the one-byte-short capacity control.
`verify.py` independently checks 32 compound predicates, every numeric pair,
generic domain/fixed assignment/model and per-rule result. Finite observations
and declared contracts do not prove general native safety, every subprocess
schedule, latency bounds or other-platform execution.

Fifteen completed M3 receipts were exactly compared with immutable committed
archives before unpinning; full comparisons and retention responses are archived
as compressed evidence. Current M4 PRE, this POST and M5 PRE remain retained.
No solver quota or artifact files were changed directly.

Run `python3 -B docs/research/artifacts/2026-11-05-embedding-sol/root-M4-review/verify.py`
before downstream source changes. [M5 PRE](../M5-pre/README.md) supplies the next
entry gate. Main, its five tracked user edits, notebooks, graph/CURRENT, external
notebook source and installed/global caches were preserved. Packaging, actual
default CLI distribution and final performance/containment/Rust notebook gates
remain pending; this review does not claim them complete.
