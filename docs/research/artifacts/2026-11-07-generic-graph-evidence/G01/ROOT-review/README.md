# G01 ROOT checkpoint recovery

Attempt 2 dispatch lease expired. Its committed source/evidence is protected at
`65cec9a1e2cc314b34d4c1a85d4411a74d2342a4` on
`spur/review-evidence/generic-G01-attempt2-checkpoint`. This is a recovery
checkpoint, not approval or a forged worker completion.

The original implementation passed 17 tests remotely. Its actual command,
compressed log and native packet are in `root-generic-command.json`,
`root-generic.log.gz`, `root-original-native.json.gz` and
`root-original-verification.json`. Production bytes match original
`b2bd69cb6`; ROOT evidence was untracked during capture.

The worker's frozen sequential command finished after the lease notice with
18 passed, zero failed/ignored. `attempt2-final-focused-command.json`,
`attempt2-final-focused.log.gz`, `attempt2-native-exports.json.gz` and
`attempt2-final-focused-verification.json` preserve its real outputs. All ten
compiled-source SHA256 values equal the checkpoint's Git blobs. The replay
entry exercises 53 finite synthetic contract requests. No collector, artifact,
disk, cache or runtime observations are claimed.

The earlier partial-log snapshot remains in `attempt2-partial-evidence.tar.gz`
with `attempt2-recovery-snapshot.json`. Its running markers are historical;
they have not been relabeled as completion. The source transfer diagnosis
includes the worker's narrowly targeted cleanup of its own retired receivers;
no lock deletion or other-job cleanup was reported.

`recover_builder.py`, `builder-recovery-pre.json` and
`builder-recovery-result.json` document the prior root-owned recovery: the
exact token's successful CloudTrail launch, unique terminated one-time Spot
request, no live pool/token instances, then the existing CAS token retirement
under the existing provider lifecycle lock. No shared config, pool limit or
launch guard changed, and no instance was terminated by ROOT.

Remaining gates: genuine missing-API compiler RED at the protected test-first
commit, query regressions, all-target check, supplied-request replay, formatting
and full diffcheck, a durable handoff, then independent ROOT actual-output POST.
No downstream G02, main integration or approval is authorized by these results
alone. Resume from the checkpoint and preserve both verification epochs.
