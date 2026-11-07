# Local integration source epoch

Local main fast-forwarded to accepted commit `fd594935d2534ab855d715401ad161c6074db621`. The plan integration tree exactly matched normalized worker `7dd41f0c9cb4d9851d2f1093cbc607ecaac2c718`; ROOT acceptance and the complete operational history are included. No push occurred.

All five pre-existing user file edits remained byte-for-byte unchanged, and the main index remained empty. The broad POST source inventory includes two of those files, `crates/spur-acp/src/adapter/{codex,mod}.rs`. Their preserved working-tree bytes differ from the clean native build cohort. Consequently, the strict default POST verifier run on dirty local main correctly fails its source-inventory comparison; the solver cases and Rust graph bytes are unchanged. This is not a native test failure or permission-review rejection.

ROOT independently rederived `prepare_post.derive(source_revision='fd594935d2534ab855d715401ad161c6074db621')`, compared its complete canonical result to the archived POST inputs and independently evaluated all 15 receipt predicates. `committed-epoch-verification.json` records that successful check. The POST establishes the recorded clean commit epoch, not the user's uncommitted ACP changes. Its normal verifier also passes from the clean ROOT review worktree. Do not discard the user's changes or rewrite the accepted cohort to make dirty main match it.

D8 begins from the accepted committed epoch in an isolated worktree. Later phases must explicitly preserve this source-epoch distinction when verifying consumed predecessor receipts. The known macro fixture failure is tracked independently as `bd-xb5p`.
