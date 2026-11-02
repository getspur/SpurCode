# Plan inspector low-friction follow-up

Approved by the user after the 2026-11-02 review. Tracking epic: `bd-kep`.
Implementation is inline; the old inspector plan and detached I3 retries are unrelated.

The inspector must make the immediate result of an action visible, show requested
content without requiring a search through other content, and advertise how to
switch panes when only one pane fits.

1. **U1 / bd-3od — action errors.** Keep a command error visible in the existing
   next-action header row at compact sizes, with a clear error label and the full
   text in help. Preserve it when unrelated loop events arrive. Clear it when a
   subsequent plan command is confirmed. This avoids spending another body row.
2. **U2 / bd-g9b — linked issue.** Opening an issue gives it its own scrollable
   content, beginning with loading/error or issue identity and description.
   Closing it restores the previous task position, including a queued bottom
   request. Task selection changes still reset the task position.
3. **U3 / bd-3mr — compact controls.** Reserve compact footer cells for Esc, help,
   the current j/k meaning, and Tab between board/detail. Issue mode advertises
   its return key. Richer actions are included only when they fit.
4. **U4 / bd-2qa — task rows.** Fit stacked titles to the cells remaining after
   cursor/status and priority metadata. Use a visible ellipsis. Blocker/setup
   conflict and retry information outrank historical worker/dependency chips.
5. **U5 / bd-1aj — legibility.** Give essential controls and semantic labels a
   stronger foreground than decorative borders. Verify built-in palette pairs
   at 4.5:1, while preserving user overrides and documenting that terminal
   background overrides can change effective contrast.
6. **U6 / bd-34h — navigation.** `/` opens a task finder matching task name, task
   ID and issue ID. Up/Down choose a match; Enter selects it; Esc preserves the
   original browsing state. `]`/`[` cycle forward/backward through tasks that
   need attention (review, escalation, failure/rejection/error, setup conflict,
   or dependency blockers). Search owns its keys and never dispatches actions.

Selection must remain stable across plan snapshots while the selected task still
exists. Search preserves a selected matching result across snapshot updates when
possible. Empty results and no attention tasks give explicit feedback. Help and
user documentation expose all new controls.

Each task requires catalog-first PRE solve, observed behavioral RED, a test
commit, minimal implementation, remote GREEN, POST solve and a fix/feature
commit. Geometry comes from declared terminal-cell budgets or observed
TestBackend rectangles; workflow models are finite witnesses, not proofs of
human usability. No local test fallback is permitted.
