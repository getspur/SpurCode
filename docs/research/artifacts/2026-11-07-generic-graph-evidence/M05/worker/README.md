M05 is **partial**: the fifteen-site migration is committed and final source validation is complete, but full strict native equality has two unresolved ordering gaps. ROOT must resolve these facts before actual POST, independent review, or acceptance. No owner/error normalization, product fix for the gaps, approval, merge, issue closure, or M06 work was performed.

The production diff is only the three authorized files: eight field reads, three writes, and four `None` fields in two constructors. Each query context constructor reuses one owned getter result across its reads. Getter/filter/validation ordering, setter order, unchecked capability tuple, explicit literal values, skipped IDs, vectors, and assertions remain intact. The other 142 bound inputs, 31 excluded spellings, and five legacy path controls remain unchanged. All observers are removed. See [source-verification.json](source-verification.json).

Actual final validation matches the genuine uninstrumented baseline:

| Command / suite | Baseline | Final |
|---|---|---|
| `test -p spur-graph --lib` | 589 passed, 0 failed, 3 ignored, 0 filtered | Same |
| `generic_evidence_providers` | 16 passed | 16 passed |
| `macro_invocation_queries` | 13 passed, 1 failed | Same |
| `rust_evidence_contract` | 18 passed | 18 passed |
| `rust_evidence_queries` | 8 passed | 8 passed |
| Two exact owning library observation cases | 2 passed, 590 filtered | Same |
| `check -p spur-graph --all-targets` | Not required | Exit 0 |
| Local `fmt -- --check` | Not required | Exit 0 |

The three ignored library cases download CodeRankEmbed, NomicEmbedTextV15, and JinaEmbeddingsV2BaseCode models; exact names/reasons are in [final-summary.json](final-summary.json). The unchanged macro failure is `parquet_macro_queries_reject_missing_source_manifest_rows`, at fixture line 41: `invalid Rust evidence` caused by `stale graph content hash`. Its intended later missing-manifest assertion is not reached. The required exact integration command exits 101 at that suite; a separately recorded `--no-fail-fast` continuation runs all four suites. No expectation was changed or skipped.

Both phases contain 1,043 authentic native records, all nine hook kinds, 22 binding packets, and all 21 grounded test lanes. Every binding packet compares all 145 compiled `include_bytes!` SHA256 values to the command snapshot and checks runtime readback. Commands retain stable HEAD/source snapshots, actual environment, wrapper exit, timestamps, raw log hashes, and actual native compiler/executable identities. Both private constructors export full wire, Debug, skipped IDs, eleven vector lengths/capacities, and native canonical/capability values. [coverage.json](coverage.json) records conservative nonzero counts for all fifteen sites and distinguishes guard-only/setup activity from later-site execution.

Strict comparison remains **false**:

- **1,025 / 1,043 records match exactly**, after only documented thread/executable/log-line diagnostics and actual temporary-root normalization. Eighteen unmatched records are nine hydrated macro owner projections and their context inputs. Full symbol rows match as multisets, but their vector order differs. Existing caller `HashSet` iteration is the candidate cause; the complete raw vectors, Debug, and serialized bytes remain in the evidence. No sorting or acceptance was applied. See [comparison.json](comparison.json), the preserved earlier [strict-integration-comparison.json](strict-integration-comparison.json), and [owner-order-diagnostic.json](owner-order-diagnostic.json).
- **33 / 36 supplemental native packets match**. Three full metadata-mismatch error strings differ in `HashMap` Debug key order. Decoding the expected/actual metadata maps gives identical maps and other text; the full error strings remain different and retained. See [supplemental-comparison.json.gz](supplemental-comparison.json.gz) and [supplemental-error-order-diagnostic.json](supplemental-error-order-diagnostic.json).
- Fourteen independent mutations are rejected on the **already equal 1,025-record intersection only**, including actual captured-empty substitution, changed source/site/input/result/error/canonical values, actual owner-binding swaps, and changed multiplicity. This does not prove full equality. See [matched-record-controls.json](matched-record-controls.json).

The exact source/commit epochs are:

| Epoch | Commit |
|---|---|
| Approved original source epoch | `82e352916376f52e57818e854804bd33118de6d3` |
| Explicit dispatched base / original native HEAD | `0965b6073f01f2891ecb21333999d508cfc8e073` |
| Baseline evidence checkpoint, before migration | `bb6ffdef2fb6a7230db2f94b306094b181d6f1c6` |
| Baseline observer review ref | `19caee2711c61a35d6c76abd79a1f9b241d06633` |
| Minimal migration / restored final production source | `7c8eeb1433bc613e4bff01f3469133e58719b81f` |
| After observer review ref | `b34e983ff2fa914ccd7bfdb4068ac08ae3ad0d8d` |

Observer commits are isolated review snapshots, not branch merges. They were created from authenticated observed bytes after capture; the commands actually ran at their recorded HEAD with those observed worktree bytes. Exact original/observed/restored bytes and observer patches are under [sources](sources/); historical whitespace is preserved losslessly in gzip archives. The final evidence commit is the branch commit containing this report, reported to ROOT at handoff.

The original current-source PRE replay passed again after baseline observer restoration. Historical replay also passes after migration. Only `sol_bf98796ac55b41e5` and `sol_ad59c8ed8c2f4b43` were reloaded live, read-only; their entire parsed envelopes equal the committed proof entries. Consumed receipts are verified only from committed archives. No receipt was created, pinned, unpinned, retired, or read from `.spur/solver`. Jev blocked/open outcomes remain those documented by ROOT; no solver/native success is inferred from them. See [pre-receipt-verification.json](pre-receipt-verification.json) and both `pre-verified-*.json` files.

Raw logs are losslessly archived as `runs/*/raw.log.gz`; command snapshots and parsed native packets sit alongside them. Cargo stderr target announcements interrupted long stdout records in transport. The collector removes only exact Cargo target-announcement diagnostic lines, retaining their text/offsets and the full original log hash; it joins all actual JSON bytes without supplying values. Existing G03 frozen request/source epochs remain as recorded. Fresh final G03 compiler/executable metadata is separately attributed in the summary and is not represented as a complete M05 binding packet.

Initial sandbox AWS endpoint exit 200, a subsequent SSM `TargetNotConnected` exit 255, harness startup correction, artifact whitespace preservation, and both strict comparison failures are retained in [diagnostics.json](diagnostics.json). Remote failures were not rerun locally. The runner records the actual wrapper exit even when its orchestration process returns zero. Sccache cumulative compilation-failure counters are host diagnostics, not M05 compiler failures. Existing issues `bd-3dkq`, `bd-1izf`, `bd-3umt`, and `bd-16eq` remain outside scope. No whole-workspace GREEN or performance claim is made.

Replay authentic evidence with `collect.py` for each observed run, `coverage.py` for both run pairs, and `matched_controls.py` for the equal intersection. `compare.py` and `supplements.py` intentionally return failure while the strict gaps remain. [post-candidates.json](post-candidates.json) contains actual facts for ROOT; it is not an actual POST or approval. Durable INTENT 4846 and ACTION 4851 were recorded and identically replayed on canonical issue `bd-3uex`; final RECORD receipts are preserved alongside this report.
