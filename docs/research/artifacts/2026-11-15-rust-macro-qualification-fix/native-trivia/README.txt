Macro qualifier trivia repair bd-3p18
End recovered macro scopes at the last significant qualifier token so legal trailing comment trivia cannot block grounded local constructor resolution.

Full unified diff: repair.patch
Verification commands, command SHA-256 and full log SHA-256: verification.json
Per-callee native assertion and parent evidence: per-callee.json
Parent source/compiler/extension provenance: provenance.json

Test commit: 645330c244f7c13077522fde38fea9bc5a4ab11e
Fix commit: 5fa07540ebb8c63b822f2c16639259702555935f
68 native tests pass. Candidate native scope controls 4/6; repaired 6/6.
Two preexisting warning families retained. Worker git tree clean.
No merge/closure; parent extension rebuild and frozen comparisons remain.

Beads completion evidence: bd-3p18 comment 6769; identical idempotent readback confirmed.
