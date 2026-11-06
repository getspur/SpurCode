# D7B ROOT review — repair required

Canonical issue `bd-2bur`, plan `22f47352-bb77-4b70-b764-3188bd8562aa`, task `D7B-rust-evidence-query`. This review does not approve D7B or begin D8.

ROOT verified the worker packet and preserved its authored history at `spur/review-evidence/g3-d7b-attempt1-authored`. Normalized implementation `12b63a1e4bdd30780433846764c3339c13710062` is unchanged. The original eight query tests and the worker's six tests remain byte-identical.

## Confirmed omission

Test commit `f59ab57836e2e2ee49b67216e24adccadb12dd43` reached the intended native runtime assertion: **exit 101; zero passed, one failed, zero ignored, zero filtered**. Source, executable and toolchain were checked on the replacement AWS builder. The executable SHA-256 is `8676ed12450667eab3ed2c310e81a34a52a090dfa8cce8d71c97fa6915316b63`; all 804 captured source/config/test hashes matched.

The fixture is the exact 18 bytes `pub struct Empty;\n` in `empty.rs`. Its independently calculated Git blob OID is `7b9d80fd6565cd8c82444ed7f2ec564cec16dbe6`. Six same-width substitutions change `empty.rs` to `zzzzz.rs` only inside the sources Parquet footer. Data pages and the artifact manifest are unchanged. The full decoded evidence still equals the original envelope, including its shared semantic checksum. The untouched disk control matches the in-memory result.

| Filtered result | Availability | Source-domain entries | Evidence files | Origins |
| --- | --- | ---: | ---: | ---: |
| Original memory/disk | Complete | 1 | 1 | 1 |
| Footer-mutated cold disk | Complete | 1 | 0 | 0 |

`store/rust_evidence.rs::read_table_input` uses unvalidated footer min/max values to exclude row groups. Those statistics are not bound by the decoded evidence checksum. `RustEvidenceQueryTables::query` assigns validated availability before decoding the pruned projection and does not detect the omission. This is a stored-query correctness defect; it is not evidence that the source file contains a panic.

`rework-pre/regression-first/` preserves the complete command/result, compressed raw log, exact structured observation and remote identity receipt. `pruning-statistics/` preserves the independently supplied D5 footer mutation and the earlier `TargetNotConnected` transport failure. That failed transport is not the runtime RED.

The later test-only commit `f51831679` allows a repair to reject invalid metadata at the full reader as well as the query. It keeps the final assertion against silent omission. Its follow-up wrapper run exited 1 before compilation/test execution because ROOT committed review evidence during upload. The source revision/state guard correctly rejected that changing checkout, despite unchanged graph/test source bytes. `final-regression-upload/` preserves this operational failure; it is not a runtime RED. The first native RED remains valid. The retry worker must reproduce the final committed regression from the stable recovery checkpoint before production edits.

## Repair PRE

Run `python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7B/ROOT-review/rework-pre/verify.py` before editing source. Fingerprint: `fe093c69e5b62d397360453a9060c48dfc549e1f029f8c91c240020fd9360cf6`; 2,489 bindings and nine independently evaluated predicates.

- `sol_0758615e27eb4b72`: SAT/pass, prospective lossless projection.
- `sol_5c4156dbf86442e2`: SAT/pass, prospective explicit rejection.
- `sol_edd2d7acc44f4c86`: UNSAT/fail, the actual native silent omission.

Jev returned an open gate for the intended finite tuple-membership verification. ROOT inspected the facts, rule, mode and empty unknowns, then added the catalog uniqueness and cardinality bindings. Complete persisted requests/results and pinned reloads are in `proof.json.gz`. The independent evaluator checks both fixed inputs and lowered predicates. These SAT cases specify permissible future repairs; they are not a completed repair or actual-output POST.

The original D7B PRE's three complete live receipts were compared exactly against their archive and all 11 predicates were independently evaluated before releasing their pins. `consumed-pre-*` records that retention transition; do not require the consumed IDs to remain live.

## Review disposition

Keep file-path predicate filtering over frozen bytes, full-envelope and owner validation, whole file groups, referenced whole origins and zero additional evidence reads after freezing. Do not trust footer exclusion bounds without establishing their correspondence to actual validated data. Fix all affected path-pruned tables, including nonzero body/site/declaration cases. Returning the original complete projection or explicitly rejecting corrupt metadata is acceptable. Retaining an incomplete projection labeled Complete is not.

The worker macro failure `parquet_macro_queries_reject_missing_source_manifest_rows` is a separately demonstrated accepted-predecessor fixture failure; its source-bound baseline remains explicit. The worker-specific completion receipt was unconfirmed, while reconciler completion audit 4232 exists. A retry must persist and replay a fresh proper completion audit.

ROOT will run independent native replay and fresh actual-output POST after repair before approval/integration. D8 overlay replacement follows D7B acceptance; D9 cache identity and D10 fresh Rust notebook follow their declared dependencies. Stored integrity does not establish source syntax authentication, call-target correctness or complete panic detection.
