# U2 attempt 2: unchanged ambiguous cell inventories

Rationale: retain Partial identity coverage and full inventories while avoiding invented cell deletions/additions for unchanged ordered normalized cells.

Parent implementation `033b9edad` and failing-first commit `6eb1c4cf6` remain ancestors. All prior evidence files remain byte-identical. Root review evidence is committed at `838b36605`, under `docs/research/artifacts/2026-11-17-notebook-symbol-uplift-review/U2-attempt1/`.

## RED

`python3 docs/research/artifacts/2026-11-17-notebook-u2-bd-1khk-attempt2/capture.py remote remote-red extract::notebook_symbols::tests::ambiguous_group_` invoked `scripts/spur-cargo test -p spur-graph --no-default-features --lib extract::notebook_symbols::tests::ambiguous_group_` with `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1`. Both remote routes failed AWS endpoint connectivity before compilation; exit **200**, infrastructure only. See `remote-red.json` and the complete raw `remote-red.log`.

The same capture command with `host host-red` deliberately sets `SPUR_REMOTE=0` under the user's infrastructure-recovery authorization. It compiled and ran seven tests: **six failed, one passed; exit 101**. No-op and output-only cases each show actual four cell events versus expected zero, for both repeated anonymous cells and duplicate explicit IDs. Mixed cases show actual seven events versus the three genuine stable-cell events. The conservative actual-edit control passed. All Partial/diagnostic/inventory/ID/SourceRef assertions passed before the event assertions failed. Production source was byte-identical to `033b9edad` (SHA256 `e9fbae6b82a46e8bbfa11f03d377b714ef7039759956ac3b7baacf51933fc83e`).

Exact argv, cwd, environment overrides, source/fixture/capture/diff hashes, times, process exit and log hash are retained in the JSON receipts. `capture.py` refuses to overwrite receipts. `setup.json` distinguishes earlier disk/sandbox setup failures and scoped host cache cleans from assertion RED. Initial free disk space was 3.3GiB, fell to 116MiB, then recovered externally to 9.1GiB; the cause of that external change was not investigated.

## Scope and finite checks

The six prior finite uniqueness/SourceRef catalog requests were replayed before implementation with `persist:false`; `solver-pre.json` retains exact requests and raw results. Expected outcomes: flat helpers fail, cell-qualified helpers pass, delimiter witness fails, structured witness passes, exact before ref passes, wrong before ref fails. These models cover only their declared finite snapshots, not parsing, arbitrary identities, runtime semantics, or ambiguous-group comparison correctness. Runtime regressions establish the latter concrete cases.

Public signatures and schema remain those in [the immutable U2 interface](../2026-11-17-notebook-u2-bd-1khk/public-interface.rs.txt). No SQL crate, ordinary-file matcher, normalized model, legacy graph identity, retained native package or earlier evidence is changed by this rework. Beads closure and merge remain the brain's responsibility.

## Predicate refinement and final RED

The first candidate run (`host-green.json`/`.log`, exit **101**) passed all six review regressions and exposed an overstrict extra control assertion: appending a third anonymous duplicate preserves the first two occurrence inventories and should produce only the new occurrence's Added event. The original control demanded a Deleted event too. That assertion was corrected to require exactly one Added event at ordinal 2; duplicate explicit groups with a changed count remain conservative. This retains genuine additions without requiring the false churn being fixed.

With production restored byte-for-byte to `033b9edad`, `python3 docs/research/artifacts/2026-11-17-notebook-u2-bd-1khk-attempt2/capture.py host host-red-final extract::notebook_symbols::tests::ambiguous_group_` compiled and meaningfully failed **all seven tests**, exit **101**. The refined addition assertion observed five events versus one. `host-red-final.json`/`.log` retain the exact final predicate, source hashes, environment and failure output. The initial RED and failed candidate receipts remain immutable. This final RED is the comparison point for final GREEN.

The shared task target appeared recreated between initial RED and the first candidate run (directory timestamp and dependency recompilation); the candidate rebuilt dependencies from scratch. No worker cache clean occurred after initial RED, and the cause was not investigated. The two pre-existing dead-code warnings in `crates/spur-graph/src/mcp/mod.rs` remain outside scope.

## Final GREEN and review handoff

Final RED was committed in `6fa59d9f2`, after initial RED commit `bce9f671a`. The restored implementation adds eleven lines inside the ambiguous-group branch, after Partial status and its diagnostic are recorded. It compares the complete ordered `NormalizedCell` inventory for that effective-ID group. Equal inventories skip invented cell D/A; different source, metadata, language, type, preparation, ordinal, identity, diagnostic, or count facts retain the existing conservative branch. It neither pairs ambiguous symbols nor skips comparisons for independent stable groups.

All final verification is **host execution**, deliberately authorized infrastructure recovery, not remote success:

| Capture invocation suffix | Exact wrapper argv after `scripts/spur-cargo` | Exit | Result |
| --- | --- | --- | --- |
| `host host-red-final extract::notebook_symbols::tests::ambiguous_group_` | `test -p spur-graph --no-default-features --lib extract::notebook_symbols::tests::ambiguous_group_` | 101 | 7 meaningful failures against unchanged production |
| `host host-green-final extract::notebook_symbols::tests::ambiguous_group_` | same | 0 | 7 passed |
| `host host-extract-green extract:: -- --nocapture` | `test -p spur-graph --no-default-features --lib extract:: -- --nocapture` | 0 | 143 passed, 0 failed, 0 ignored |
| `host host-matcher-green git_walk::tests::` | `test -p spur-graph --no-default-features --lib git_walk::tests::` | 0 | 48 passed, 0 failed, 0 ignored |

Each capture invocation begins `python3 docs/research/artifacts/2026-11-17-notebook-u2-bd-1khk-attempt2/capture.py`. Exact host overrides in every receipt: `SPUR_REMOTE=0`, `SPUR_NO_LOCAL_FALLBACK=1`, `CARGO_TARGET_DIR=/private/tmp/spur-notebook-u1-bd-bi1d-target`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `SPUR_SCCACHE_S3=0`, `RUSTC_WRAPPER=` and `TMPDIR=/private/tmp/spur-notebook-u1-bd-bi1d-tmp`. Final RED/GREEN commands, environments and test bytes match.

The extraction suite reasserts frozen **46/24 symbols, 17/11 functions and all 28 anchors**, original refs/offset behavior, collision-safe IDs and ordinary extraction compatibility. The real comparison remains **50 symbol changes and 10 cell events**. The ordinary matcher suite includes its existing offset-sensitive behavior. `solver-post-final.json` replays all six finite requests after focused GREEN with the same expected outcomes. `solver-post.json` is the earlier candidate-stage replay, preserved separately.

[rework-final.patch](rework-final.patch) is the complete final source/test unified diff against `033b9edad`; [rework.patch](rework.patch) preserves the earlier candidate with the initial control assertion. `verify_receipts.py` verifies all log/source/predicate hashes, final RED/GREEN identity, unchanged prior evidence and public interface, original ancestry, finite solver outcomes and the final patch. `verification.json` records its actual output and final formatting/scope checks.

Exact unchanged public entry points for U3, in `spur_graph::extract::notebook_symbols`:

```rust
pub fn extract_notebook_symbols(
    path: &str,
    bytes: &[u8],
) -> anyhow::Result<NotebookSymbolDocument>;

pub fn compare_notebook_sources(
    path: &str,
    old_path: Option<&str>,
    change_kind: &str,
    left_source: Option<&[u8]>,
    right_source: Option<&[u8]>,
) -> anyhow::Result<NotebookComparison>;
```

The exact version-1 document/cell/symbol/SourceRef/status/diagnostic/change schemas remain in [public-interface.rs.txt](../2026-11-17-notebook-u2-bd-1khk/public-interface.rs.txt), including shared normalized/prepared-source types. No downstream schema change is required for this correction.

Limits: suppression requires exact ordered normalized inventory equality, not matching through ambiguous edits; changed ambiguous groups remain Partial and conservative. No execution/semantic equivalence is claimed. Finite solver snapshots do not prove arbitrary parsing/identity/reference behavior. Native SQL, consumer, publication and release validation belong to downstream tasks and were not rerun here. No beads closure or merge was performed.
