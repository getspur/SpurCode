# S1 final root review

Candidate: `2fd3ec7b842dbf0e3138eb0fdfd3328e2b65feb6`; authored worker head `7902eb1a3`.
Task `bd-339u`, plan `6f98d9fa-715e-4aed-9cef-73ad302d937f`.
Decision: accept S1 after the checks below; S2 remains a separate reviewed implementation unit.

The strict wire deserializers use private empty struct variants while preserving public unit variants and serialized JSON. Root reviewed the57-line production change and the variant matrix. The previous two genuine RED assertions are preserved, and the worker's test/fix sequence precedes the code change.

Root independently ran the complete crate suite through scripts/spur-cargo with SPUR_REMOTE=0, TMPDIR=/private/tmp and the shared target:14passed, zero failed; see root-test.log.gz (verbatim compressed output) and root-test.json. The worker also retained successful scoped fmt/all-target check/Clippy logs. No source behavior was changed during root verification.

Root verified all8source hashes, the saved log/dependency-tree hashes and all3worker binary hashes before the independent rebuild. The independent build has its own captured3binary hashes. Raw logs contain43distinct case observations exactly matching independent expected policy and worker exports. The7case-ID partitions are disjoint/exhaustive. No native inference or DB package is in the captured normal/build closure.

## Fresh actual-output Jev and SOLVE POST

All7Jev partition compiles opened. Root inspected family, verify mode, rule/subjects, exact facts and empty unknowns. Jev compiled the tuple-policy binding; root explicitly added catalog uniqueness/cardinality bindings, then separately executed global metadata/coverage and adversarial controls.

The64-variable cap is honored by disjoint case-ID domains. Per-partition completeness, uniqueness and exact counts ensure every expected key appears once; exact global group counts and zero unmatched observations guard omission/extras. Raw-log parsing independently confirms that projection; it is not inferred from test count alone.

| Check | Verdict | Receipt |
|---|---|---|
| partition_0 (7 cases) | sat / pass | sol_ff74c582e39a40f2 |
| partition_1 (7 cases) | sat / pass | sol_4abf59b8b7624635 |
| partition_2 (7 cases) | sat / pass | sol_5898245f678949b9 |
| partition_3 (7 cases) | sat / pass | sol_6b12155efaed4a86 |
| partition_4 (7 cases) | sat / pass | sol_a48bd273797a42b4 |
| partition_5 (7 cases) | sat / pass | sol_3c4e2fb559394d3b |
| partition_6 (1 cases) | sat / pass | sol_f52510192cac403b |
| global_disjoint_complete_43 | sat / pass | sol_19e06e5754a9449e |
| source_binding | sat / pass | sol_56aea6db7c284931 |
| binary_and_native_binding | sat / pass | sol_452eb78bc7e14318 |
| omit_case | unsat / fail | sol_b154aa4cd0e8406c |
| duplicate_case | unsat / fail | sol_9cf6d12c0dfa4c85 |
| unexpected_case | unsat / fail | sol_10bf5bce8fc94773 |
| stale_source | unsat / fail | sol_487f1d2638764eff |
| stale_binary | unsat / fail | sol_e0e94eb092ee4c58 |
| injected_ort | unsat / fail | sol_305ecd3dab0d46ec |
| invert_valid_hello | unsat / fail | sol_3e7f938ed54b4b85 |
| invert_wrong_version | unsat / fail | sol_af30aebe0f9b4561 |
| invert_wrong_fingerprint | unsat / fail | sol_83c06b16a00b4113 |
| invert_nonfinite | unsat / fail | sol_568554bb87fc4dbc |
| invert_shutdown_unknown_field | unsat / fail | sol_650482918fc74f43 |
| invert_closed_unknown_field | unsat / fail | sol_a006f2becb424525 |

All22results were persisted/reloaded and inspected:10positive passes and12expected rejections. The previous failed Shutdown/Closed behavior is now observed as rejection and the inverted outcomes fail. Exact Jev provenance, original family requests, returned lowered queries, actual raw observations and source/build bindings are in post.json.gz; SHA256SUMS.json covers this record and logs.

This is finite protocol evidence. It does not establish universal parser correctness, native lifetime safety, subprocess containment, performance, or completion of Rust coverage gaps. Real child process behavior starts in S2; native inference remains S3/S4.

The engine scheduling epic bd-8tee must remain open for automatic dispatch. Prepare and deliver source-bound S2 PRE before approving the S1 handoff; keep root-mediated actual-output Jev POST before every task approval.
