# G1c: source-bound Rust declaration evidence

Task `bd-16m`, plan `75f37d22-0849-413b-bbc1-79c70f33949e`, delegation `8c25ad4d7826435d`; dispatched base `727293379bd862993fdb23b06beec82c6d2e5503`. The collector is a separate, non-persistent API. Its coverage is restricted to supplied physical Rust files and tuple fields; cfg is raw syntax. A source with recovered syntax is unknown even if healthy subtrees yield individual rows.

## PRE and POST

The fresh catalog `data_integrity.unique` and `data_integrity.mutually_consistent` PRE in `pre.json` was persisted and reloaded. `sol_a36d1ef5b5dd4dde` is SAT/pass for the two-row source/owner/ordinal fixture; `sol_b046be0e338f4a37` is UNSAT/fail after moving a row to another existing owner. These finite fixtures are not a proof about all Rust inputs.

After the recovery fix, `actual-export.json` was produced by the current compiled test; it contains exact source strings, graph, serialized set, and nine rejected modified sets. `post-final.json` contains exact requests, responses, and reloaded receipts: `sol_29025bab069343ce`, `sol_cfc66d2c75d7445c`, and `sol_e525833f760b4f11` are SAT/pass for all five fixture tuples in three disjoint projections; `sol_a3303e8ce0314b51` is UNSAT/fail for the wrong-existing-owner control. One initial combined five-row request exceeded the solver's 64-variable limit and did not yield a verdict; the recorded projections cover the same five rows.

The 136-root rebuild's actual serialized outputs are `frozen-export.json`. `check_frozen.py` checks the 446 frozen rows against source OID, graph field and owner IDs, physical ordinal, full and type spans, independent captured attribute/visibility text, and raw attribute/visibility byte slices; its exact final output is `frozen-join.json` and `frozen-join-final.log`. All 446 match. `tests/extension_credential_bridge.rs` remains unknown because its parse recovered; its independent healthy field is present. This is not a complete-source claim for that file. The initial implementation produced 445 matches, documented in `frozen-join-initial.json` and `frozen-join.log`.

`project_frozen.py` independently compares exact tuples with G1b's frozen source-bound inventory and emits the SHA-256 projection in `frozen-projection.json`. All 446 are unique and equal by the explicit exact join; the digest encoding assumes SHA-256 collision resistance. The 23 complete, disjoint catalog verification requests, responses, and reloaded receipts are in `post-corpus.json.gz`; all are SAT/pass for their encoded batches. Cross-batch uniqueness is checked by `project_frozen.py`, not by the solver. Receipt IDs in batch order:

```text
sol_d2e4eae4338545be sol_1c527bc9a0534c2a sol_a766803f8e584794
sol_f2fa161ef11e47c1 sol_1d90b1e9f6054da0 sol_8f322082793b4d9e
sol_8edaa44ac4454ed3 sol_72899fdac9c34538 sol_85c7d24a56104b38
sol_def99dd63dfc450e sol_5c1a06a6f0db486d sol_271f500734634375
sol_0e7185058f0c4f9c sol_d1c1a7dfd966438f sol_bc7b447a822b411e
sol_9ae9ce3491d541ba sol_0f3be1a8e54240c4 sol_f8de80777c57479c
sol_d9744feb77c84e02 sol_c3eb621a13764192 sol_3b23ba77a11e4bb0
sol_2bf52327f6594954 sol_128b051cbd364e82
```

## RED and GREEN

All Cargo commands use `SPUR_REMOTE=0 scripts/spur-cargo`; logs retain their outputs and exit codes. The initial `test -p spur-graph --test rust_declaration_evidence` exited 101 with missing public imports: **compile RED**, no test assertions ran (`red.log`). After the first implementation, the corpus join found an unknown file whose healthy field was suppressed. The added `parse_recovery_retains_independently_healthy_tuple_rows_as_unknown` regression exited 101 **after compiling**, with assertion `left: 0, right: 1` (`recovery-red.log`), then passed with the fix.

Final commands and results:

```text
SPUR_G1C_EXPORT_SMALL=<absolute G1c/actual-export.json> SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_declaration_evidence
  exit 0; 6 passed, 1 ignored (recovery-green.log)
TMPDIR=/private/tmp SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_declaration_ownership --test rust_tuple_fields --test macro_invocation_contract --test rust_macro_token_tree_query --test rust_macro_invocations
  exit 0; 31 passed, 1 ignored (affected-final.log)
TMPDIR=/private/tmp SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --lib extract::notebook
  exit 0; 18 passed (notebook-final.log)
SPUR_REMOTE=0 scripts/spur-cargo check -p spur-graph --all-targets
  exit 0 (check-final.log)
scripts/spur-cargo fmt --package spur-graph -- --check
  exit 0
git diff 727293379bd862993fdb23b06beec82c6d2e5503 --check
  exit 0
```

The first notebook run without canonical `TMPDIR` exited 101: an existing test compared `/var/...` with `/private/var/...` (`notebook.log`). Its canonical-temp rerun passed; both are retained. The build emitted existing dead-code warnings documented in the logs.

Frozen fixtures were generated outside Git using the existing G1b `prepare_frozen.py` (exit 0; it validates 136 Git blob OIDs), at `/var/folders/mv/vtzfmys92yl9rwcsgf9n1x780000gn/T/spur-g1b-batch-es8hb7l6`. The fresh rebuild and checks were:

```text
SPUR_G1C_ROOT=/var/folders/mv/vtzfmys92yl9rwcsgf9n1x780000gn/T/spur-g1b-batch-es8hb7l6 SPUR_G1C_EXPORT=<absolute G1c/frozen-export.json> SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_declaration_evidence export_frozen_declarations -- --ignored
  exit 0; 1 passed; 337.92 seconds (frozen-final.log)
python3 docs/research/artifacts/2026-11-02-rust-gap-implementation/G1c/check_frozen.py
  exit 0; 446/446 (frozen-join-final.log)
python3 docs/research/artifacts/2026-11-02-rust-gap-implementation/G1c/project_frozen.py
  exit 0; 446 unique projections (projection.log)
```

`source-hashes.txt` binds the final Rust sources, frozen research inputs and exports. Test commits: `86c64f44b`, `958794dc8`, `77de1a050`, and `c874941c1`; feature/fix commits: `91a68d31f` and `5647500b6`. The supplemental export and recovery tests were committed after the initial feature; the recovery test had its own observed assertion RED before the recovery fix. No graph schema, carrier, persistence, external project, or registry change was made.
