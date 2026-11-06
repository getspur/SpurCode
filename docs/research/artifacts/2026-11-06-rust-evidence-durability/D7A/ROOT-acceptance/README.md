# D7A ROOT checkpoint acceptance

ROOT reviewed canonical bd-1sy1 in progress with no signal labels, completion audit 4178 persisted/replayed, a matching 52-file diff and unchanged original test bytes. Original authored history is protected by spur/review-evidence/g3-d7a-attempt1-authored at ceb3e4c91d9be784d95d26fda0cef5a167a51d54. Normalized worker source: deb30750ed80d430691a1d001e6b3b9c4c83a358.

The worker packet verifier and complete source diff check passed. ROOT then independently executed the recorded native ELF again, checking all 803 source/build-input hashes and executable SHA-256 before and after. The seed reached the availability assertion and exited 101 (0 passed, 1 failed, 7 filtered); the custom control exited 0 (1 passed, 0 failed, 7 filtered). The control returned Unavailable, source_domain=None and empty files/origins. No compiler errors occurred in either replay. Native executable SHA-256: 5b7d5e950d51064884120903c26c90c0044ed9ad34bf0cb1691e6be93e1b1248. Exact commands, source digest, timestamps, toolchain, native logs and parsed output are retained here.

Fresh ROOT POST fingerprint: `54b329db1992a15f3a45437c06d061f5abc9d0b78160fa94263c4a155bd642ff`; 2,459 bindings and 18 independently evaluated finite/lowered predicates. Jev gate open, data_integrity / verify; exact native-run binding inspected and unique/cardinality constraints included. Both complete receipts are persisted, pinned and reloaded:

- sol_d0250cedf29b4db8 — SAT/pass for actual native RED/control GREEN and exact scaffold.
- sol_3710c8675d9a412a — UNSAT/fail for independently forged seed pass, known-empty custom domain and full-feature completion claims.

`python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7A/ROOT-acceptance/verify_post.py` passed. The consumed two PRE envelopes were archived, exactly compared with full live reloads and independently checked (nine predicates) before unpinning. No quota/store files were changed.

This accepts only the TDD/scaffold checkpoint. All eight original tests are preserved, but only the seed and custom control ran. The seed stops before file-count, parity and stale-OID assertions. The scaffold has no explicit &T forwarding or functional memory/Parquet contexts; those are required in D7B. No full-suite GREEN, complete panic detection or full D7 approval is established. Keep the checkpoint on a staging branch; local main must wait for completed D7 implementation and its independent POST.
