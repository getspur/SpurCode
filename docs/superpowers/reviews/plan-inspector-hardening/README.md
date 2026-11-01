# Plan Inspector hardening evidence

The accepted [design](../../specs/2026-11-01-plan-inspector-hardening-design.md) and [implementation plan](../../plans/2026-11-01-plan-inspector-hardening.md) cover ten findings from the original inspector screenshot. All ten are implemented and reviewed. Each task has persisted PRE/POST solver receipts, observed behavioral RED before its fix, and remote GREEN evidence.

| Task / issue | Resulting behavior | Test commit | Fix commit | Evidence |
| --- | --- | --- | --- | --- |
| I1 / bd-1o3 | Readable selected-stage window with overflow indication | 16289aab7 | 4b3e1a82e | [I1](I1.md) |
| I2 / bd-2e6 | Selected title and metadata remain visible in lanes and stacked lists | f17c60e66 | dff2abc34 | [I2](I2.md) |
| I3 / bd-30b | Independent detail scrolling with wrapped-row clamping and immediate bottom-to-up movement | a5e8c875a | 667526c20 | [I3](I3.md) |
| I4 / bd-2vm | Escalations have a glyph, attention count, and complete progress denominator | b5c8552bd | b06e95f91 | [I4](I4.md) |
| I5 / bd-3ee | Setup conflicts show explicit blocked reasons even without blocker IDs | 1a9ea828d | afb8fdb57 | [I5](I5.md) |
| I6 / bd-36e | Worker history and disjoint lifecycle counts are labeled accurately | 6f2e580ee | 9f08e7fa5 | [I6](I6.md) |
| I7 / bd-2ns | Next action remains visible beside deduplicated current risks | 9a90b4c11 | 92bc869da | [I7](I7.md) |
| I8 / bd-mig | State, progress and attention survive narrow/short layouts; full identity remains reachable | 9574caf4b | 07c28a185 | [I8](I8.md) |
| I9 / bd-1aa | Action/error-first detail, multiline output, expandable metadata and parent/child navigation | 5bfc71923 | 9003755e8 | [I9](I9.md) |
| I10 / bd-ptq | Mode-correct controls, scrollable help/legend and grapheme-aware cell budgets | 61f346ff5, 1f063a380 | 45c0a3b97 | [I10](I10.md) |

Independent review produced additional regressions and fixes before approval:

| Contract | Test commit | Fix commit | Evidence |
| --- | --- | --- | --- |
| I3 loaded output beyond 65,535 wrapped rows and queued bottom/issue-open reset | 1604ba2ca | ca63589e3 | [I3 review](I3-review.md) |
| I3 multiline Unicode byte mapping and wrapped carry capacity | e00636014 | cc04f54a5 | [I3 mapping](I3-mapping.md) |
| I8 mixed attention on a 90x8 terminal and complete counts in help | 1e1f8e2b8 | 9ed05e9c1 | [I8 review](I8-review.md) |

Solver verification is deliberately bounded to the encoded facts and traces. Geometry POST facts come from actual TestBackend observations. Generic scroll models include explicit finite domains and separately check feasibility and absence of unsafe witnesses. Cached receipts are identified in each report and are not presented as independent reruns. Persisted receipts are pinned and retrievable with get_solve_result; complete replay requests are recorded in the task reports.

The early detached worker attempts stopped before behavioral RED and were not approved or integrated. Direct remote execution subsequently established the required failures and completed the fixes. Original I3 issue bd-3kb was superseded by bd-30b; stale recovery dispatches are not part of the delivered implementation.

Final verification is recorded in [I10](I10.md):

- Full remote spur-tui suite: 2,136 passed, 0 failed, 4 ignored; exit 0.
- Focused inspector/library targets: 1,400 passed, 0 failed, 4 ignored; exit 0.
- Remote production-library clippy with -D warnings: exit 0.
- Local scoped formatting and final staged whitespace checks: exit 0.
- Independent read-only final review: no remaining Critical/Important findings.

Rendering and interactions are verified with ratatui TestBackend, including narrow/short layouts, loaded output beyond 65,535 wrapped rows, Unicode text, resize clamping, and modal ownership. Product controls and glyphs are documented in [Issues and Planning](../../../user-docs/04-issues-and-planning.md#plan-inspector). The work is committed locally; unrelated workspace changes are preserved.
