# Focused D8 repair: first entry attempt

The repair remains unimplemented. Attempt `ee105e24117a4771` verified PRE and
its assignment, then called `report_audit` with
`task_id: "D8-empty-capability-repair"`. That tool expects the canonical Beads
issue ID in this field and returned `issue D8-empty-capability-repair not found`.
The request's own evidence correctly identified canonical `bd-3eyg`.

This was a request-argument error. The entry binding was verified; the error
does not establish an assignment-service failure. The completed worker branch
remains exactly `b264c7add5f556c0e2abc2ae1c0c5f7c7cf734e4`, without a diff.
No implementation, build or new runtime result occurred.

The next bounded retry must verify its fresh assignment and submit a fresh
canonical INTENT with `report_audit.task_id` set literally to `bd-3eyg`.
The plan slug belongs in `evidence.plan_task_id`. ROOT must not replay a worker
audit or restore the completed attempt's delegation label.

Correcting an accidentally supplied slug to the already assigned canonical
issue is a routine parameter correction within the authorized task. A real
authorization rejection for canonical `bd-3eyg` still requires ROOT repair
before source edits or builds. The full original request, error, PRE result
and live receipt responses are preserved in the compressed record.
