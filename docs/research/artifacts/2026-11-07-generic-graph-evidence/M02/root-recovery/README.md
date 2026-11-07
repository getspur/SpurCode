# M02 audit transport recovery

ROOT did not approve attempt 1. Its twelve evidence files authenticate a blocked
entry, not an implemented migration. The evidence-only checkpoint is
`1cac86f1214118820854ff5f1ff6b0964306d76d`; all three scoped source files still
match original PRE `4a411dbd24f306a27b48b7e11b2ab381b7e39797`.

ROOT independently reran the archived entry check and current-source PRE verifier,
checked the complete diff, and confirmed the empty source diff. Exact commands
and zero exit codes are in `verification.json`. No M02 native execution has run.

The original canonical task assignment was correct. Audit request and identical
replay both returned `user cancelled MCP tool call`, without an authorization
denial or other cancellation cause. Neither produced a durable worker receipt.
The decoded request is 946,647 bytes, including full solver envelopes. This size
is an observation, not a demonstrated cause of cancellation. Full envelopes and
exact cancellation records remain in the immutable attempt-1 archive.

ROOT has now persisted and read back canonical recovery INTENT on `bd-1zi2`;
`root-intent-readback.json` contains the exact Beads comment and comment ID.
That record and the updated plan task establish INTENT before ACTION under
spur-way. The original task's additional requirement for a successful worker
INTENT call is replaced by this durable ROOT intent; no source or validation
requirement is relaxed. Worker audit payloads should reference real committed
artifacts, fingerprints and solve IDs rather than duplicate the full envelopes.

A redundant worker audit transport cancellation should be archived and reported
while the authorized work continues under ROOT's persisted intent. An explicit
authorization denial, assignment mismatch, supersession or stop still requires
ROOT resolution. Never claim an unreturned worker audit persisted. Completion
must be recorded in Beads and independently reviewed; ROOT POST and every actual
native conservation requirement remain pending. There is no new solver model,
source scope, resource bound, performance fix, user permission flow or successor.

The next attempt must recover this checkpoint with a fast-forward from its
original plan base before editing source. Preserve attempt-1 files verbatim and
write new evidence under `M02/worker/attempt2/`. The historical
`worker/verify_entry.py` intentionally asserts unchanged source; execute it at
the recovered checkpoint before migration. Do not edit it to claim native work.
