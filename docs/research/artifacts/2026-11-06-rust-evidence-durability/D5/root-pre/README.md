# D5 ROOT PRE and dispatch gate

D4 is approved, closed and integrated on local main at f51d56931a6110332adf5808b3b18a3733b2956a. This packet authorizes the next scoped implementation stage; it is not D5 acceptance.

The source task is bd-28iq under Rust durability phase bd-3l2. Plan Engine creates the canonical dispatched issue. That issue, not bd-28iq or a branch UUID, is the worker audit target. The worker route is the user's codex / codex profile / gpt-6.1-sol / xhigh preference. The full bounded contract is [worker-spec.md](worker-spec.md) and the approved D5 plan. The direct schema transport test fallout is explicitly scoped there.

Run from the checkout root:

```bash
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D5/root-pre/verify.py
```

Fingerprint: 689f9675aabc38b08859aa59efb430c761f7dc53af344fc42874c95e90f505dc.
The verifier rederives the exact 75 current source/document/catalog/script bindings and two new-file absence checks, then compares all complete requests, finite decisions, lowered variables, fixed facts, hard predicates, model values and persisted statuses. The independent finite evaluator extends the inherited oracle for FK and conditional-required semantics; the initial unsupported-oracle preflight stopped before any solve, was corrected, and is not a production test RED.

| Receipt | Expected | Independent predicates |
|---|---|---:|
| sol_1af8d56bc37d4375 — five tables / zero / absence / capability | SAT / pass | 8 |
| sol_f056cb782b08455e — declaration and call lane nullability | SAT / pass | 3 |
| sol_8ac31c08bbe6467a — independent adversarial controls | UNSAT / fail | 11 |

All three receipts were persisted, pinned and fully reloaded. Jev returned an open data_integrity / verify request for tables_exact with identical supplied facts; the complete assembled request adds the other seven explicitly discovered bindings. Jev interpretation is separate from the 22 solver predicates. The catalog responses are archived under catalog/.

These are prospective transport projections grounded in D4's accepted raw fixture and native envelope, not observations of schema-v13 Parquet. Explicit NULL labels and zero-profile tuples in the finite model do not prove physical Parquet nullability or file presence. D5 POST must inspect and replay the actual tables, schemas, metadata, rows and corruptions. D4's old IDs, checksum, manifest version and graph hash are grounding only: v13 requires fresh native recollection. No production/test source changed for this PRE.

Independent controls:
- Remove a zero-row source table while retaining a complete v13 declaration: both the exact tuple and conditional-required predicate reject it.
- Relabel an Unknown call lane as complete zero: its exact tuple rejects it while declarations remain independently required.
- Substitute a different existing body in the same file: the FK passes, while exact call-range/innermost-body ownership rejects it. Expected ownership is independently checked against D1's byte geometry and raw call bytes.

Retention: the receipt store was full. Before releasing pins, ROOT preserved the complete original and live envelopes of the two now-closed D4 rework PRE receipts and one closed D3 source-partition POST receipt, compared them exactly and independently replayed all 12 old predicates. Files retired-archives.json.gz, retired-live.json.gz and retirement-operations.json preserve that record. Their old IDs are sol_db8b59ff16db461c, sol_c367b84283bc464a and sol_62240493caca47b2. D4's seven current POST receipts supersede that predecessor partition and remain pinned; all other current D3 POST partitions remain untouched. Only explicit pin=false retention calls were used; no quota/store files were edited.

Implementation gates: commit/run the existing-API assertion RED before source changes; actual GREEN and native disk exports; leave the task in progress for fresh ROOT POST/Jev and independent review. D6 remains gated. Five user-edited main files are outside this isolated checkout and unchanged.

The initial PRE packet had trailing blank lines in five authored text files. They were normalized before dispatch; draft-before-eof-normalization.json.gz preserves the prior fingerprint. Exact facts, rule bindings and all three persisted requests are unchanged; the final fingerprint binds the normalized scripts and spec.
