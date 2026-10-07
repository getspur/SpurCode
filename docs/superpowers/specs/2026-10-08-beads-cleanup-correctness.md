# Beads cleanup correctness

Review: `bd-1kc17`. Implementation: `bd-33a3b`. The user requested the fixes after the review.

SPUR must recognize the pinned exporter's `issues.jsonl.tmp` alongside legacy
`issues.jsonl.<pid>.tmp` names. The same matcher controls startup's pending-work
probe and stale-temp sweep. Existing age, write-lock, and unrelated-file
protections remain in force. A failed real export supplies the regression fixture.

In beads_rust, every enabled backup attempt with an existing target must apply
the configured per-stem history retention even when content is unchanged. A
fresh identical backup can be reused. If the identical backup has expired,
create a fresh copy before rotation, preserving recovery when the configured
count and age limits permit retention. Existing zero-limit behavior is unchanged.

Fix upstream at the exact pinned base revision, then pin SPUR to the tested
commit. Do not change retention defaults, add gzip handling, purge issue/audit
records, or run cleanup against live data.

Validation: RED/GREEN for the actual temp-file path, startup fast path, expired
history on deduplication, reduced count, and renewal of an expired last backup;
SPUR library tests; upstream checks and sync safety tests; catalog compatibility
verification before and after implementation.
