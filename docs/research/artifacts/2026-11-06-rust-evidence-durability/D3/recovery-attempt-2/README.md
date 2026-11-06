# D3 attempt 2 lease recovery

The dispatch lease expired. Beads audits 3927–3929 record orphan clearing and escalation. The only new persisted worker audit is INTENT 3926; there is no repair or GREEN claim.

ROOT inspected the still-present staged worktree: its tracked files exactly equal attempt 1 canonical bd97a0e84, and all 41 restored reviewer additions equal committed checkpoint 983c91ae5. The two new worker records, entry.json and live-pre.json.gz, are archived here unchanged. The four captured complete envelopes equal the supplemental PRE proof; ROOT also freshly reloaded those four receipts and compared the complete envelopes exactly. The pre-edit source verifier still passes fingerprint 754ec020484f7a6e7882f69281d369e3873bd3c9711585e5419529f5b32d264a, 15 bindings and 12 independent predicates.

The available recoverable source/test checkpoint remains 983c91ae5f588ffb321b7a10d5dd1a0453ea4023. No partial normalization repair exists to preserve. The failed session is no longer in the delegation registry; status and cancellation APIs return Unknown delegation. Its directory is retained for evidence and will not be reused as an active writer.

Recovery action: atomically rewrite the same task into a shorter checkpoint-based brief and retry through submit_plan_mutation, retaining codex/profile codex/model gpt-6.1-sol/effort xhigh, the five assertion failures, strict TDD, source-bound PRE and required actual-output ROOT POST/review. Main and the five protected user files remain unchanged. No D3 approval/merge or D4 dispatch is authorized before the repair passes review.
