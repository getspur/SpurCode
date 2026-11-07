# M02 entry checkpoint — blocked before source migration

Canonical assignment: `bd-1zi2`; delegation `acc176a751fe4d78`; assignee
`spur-worker-acc176a751fe4d78`; plan task `M02-caller-preparation`; plan
`81f425dd-f796-4ad0-8fa3-608248a18838`. The assignment was `in_progress`, with
dispatched base `4a411dbd24f306a27b48b7e11b2ab381b7e39797` and no superseding label.
The source-tracking issue `bd-12gb` was not used for audit writes.

Original source / entry HEAD: `4a411dbd24f306a27b48b7e11b2ab381b7e39797`.
There is no migration commit and the source-only unified diff is empty. This
checkpoint changes evidence only. Its commit is discoverable with
`git log -1 --format=%H -- docs/research/artifacts/2026-11-07-generic-graph-evidence/M02/worker/README.md`.

## Verified entry evidence

The unchanged ROOT PRE verifier ran successfully twice with `--current-source`.
`pre-verifier.log` preserves the second output; `entry-verification.json` records
the exact command, exit status and entry source HEAD. Fingerprint:
`a7991096b4283dccc5a146cdcad1ca9111bd74164b9e6cd5d56ef8e64a548df3`.
It reports 125 bindings, nine sites, nine prospective cases, ten independent false
controls, 33 variables and eight predicates. These are prospective obligations.

Both complete live `get_solve_result` responses were received and compared by
parsed JSON equality to `root-pre/proof.json.gz`'s `reloaded` envelopes:

- `sol_741613e11a004a45`: SAT; complete envelope exactly equal.
- `sol_a33aa22358694b4b`: UNSAT; complete envelope exactly equal.

`entry-live.json.gz` preserves both raw MCP response envelopes losslessly.
No solve pins, quota, ROOT PRE, M01 evidence or historical evidence changed.

## Blocking audit result

`intent-request.json.gz` is the exact `report_audit` request against `bd-1zi2`,
including both complete PRE envelopes. Audit ID:
`5b7e19b3-a216-41e7-b7a5-7546c6457e10`.
`intent-response.json.gz` records:

```json
{"content":[{"type":"text","text":"user cancelled MCP tool call"}],"isError":true}
```

One identical request replay returned the same error, preserved in
`intent-replay.json.gz`. Neither response contains a comment ID or a reloaded
audit. Durable INTENT persistence is unconfirmed; no successful replay is claimed.
The tool supplied no more specific authorization/backend reason. This is a tool
failure, not a compiler or assertion RED and not an automatic approval-review
rejection.

The exact error and full canonical assignment snapshot were sent once to ROOT
using `report_progress`, which returned `ok: true`. Both request and response are
archived. Progress acknowledgment is not a durable audit. No alternate issue,
invented task slug, direct PM write, substitute label or additional audit ID was
used to bypass the failed request.

Code graph symbol reads and symbol search also exceeded their 15000 ms deadlines;
no indexed symbol bodies were returned. The initial orientation tool returned
an analyst graph staleness caveat. Limited raw source reads followed the failed
graph attempts; no source was edited.

## Work outstanding

The task explicitly requires durable INTENT before source work. All nine
constructors remain unchanged. No native build, test, check or benchmark command
ran; pass/fail/ignored/filtered counts and all nine before/after observations are
unavailable. No observation patch, migration, native conservation join, native
mutation control, final source validation or ROOT POST candidate is claimed.
The inherited `bd-16eq` risk remains open by task context and was not reproduced
or repaired in this attempt. No ACTION/RECORD audit succeeded or is claimed.

ROOT must resolve audit availability/authorization for the canonical assignment
before this worker can continue the approved baseline → migration → native
conservation → final checks sequence. No self-approval, issue closure, merge,
successor dispatch or ROOT POST was performed.

Replay local entry authentication with:

```sh
python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/M02/worker/verify_entry.py
```

This authenticates archived PRE equality, assignment and audit failure evidence;
it does not perform native acceptance or claim migration completion.
