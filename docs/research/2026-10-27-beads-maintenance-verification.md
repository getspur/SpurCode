# Beads maintenance implementation verification

Implementation issue: `bd-28k`. Scope: `spur-pm`, its tests/example, and analysis artifacts. All compilation, tests, and lint commands used `scripts/spur-cargo` on the configured AWS builder with local fallback disabled. Formatting ran locally through that wrapper.

## TDD and runtime evidence

- Before production changes, the automatic-export and clean-startup history-budget regressions failed (2 failed, 2 passed). Tests were committed in `77c2adc3f`.
- Subsequent failing tests exposed malformed UTF-8 maintenance stamps suppressing cleanup and flush-only reports resetting measured sizes to zero. Both are fixed and covered.
- The first full suite exposed an existing unchecked short-ID collision in `TestBeadsWorkspace`, before maintenance was involved. A deterministic collision regression failed and was committed in `4ef949509`; the fixture now uses the upstream collision-aware generator.
- `SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-pm`: **284 passed, 0 failed, 2 existing ignored**, including 218 unit tests and 12 maintenance integration tests.
- `RUSTFLAGS='--cfg madsim' SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-pm --features test-helpers --test beads_durability_madsim`: **3 passed**. The cfg was enabled, so these tests actually executed.
- `SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo clippy -p spur-pm --lib --test beads_maintenance --example beads_maintenance_eval -- -D warnings`: **passed**.
- `scripts/spur-cargo fmt -p spur-pm` and `git diff --check`: **passed**.

The all-target strict Clippy command was also run. It remains blocked by existing warnings in unrelated targets: non-Send test helpers in `bv.rs`/`graph_engine_integration.rs`, an unnecessary safety comment in `ingest/github/auth.rs`, and style/documentation warnings in `apply_test.rs`, `cursor_boundary_safe.rs`, `mcp_passthrough_snapshots.rs`, `pm_service.rs`, `watermark_scan_perf.rs`, and `beads_crate_wal_cleanup.rs`. New-code diagnostics were corrected. The focused passing command above does not imply that all targets are lint-clean.

Independent read-only review found no critical or important correctness defects. Its two coverage requests were added and passed: permission-denied history unlink/retry on the non-root builder, and scheduled export recovery after an export failure. Tests also cover a foreground queued job owning the flock, continuous actor traffic, malformed/future shared stamps, cross-adapter throttling, stale plans, symlinks, missing protected copies, dry runs, and pinned WAL readers.

## Solver evidence

Exact requests/results are in [the solver receipt file](2026-10-27-beads-maintenance-solves.json).

| Check | Pre | Post | Result |
|---|---|---|---|
| Approved/shipped policy | `sol_f3b6ef78501d4fdb` | `sol_a578f2fddbd145ff` | sat |
| Two 100 MiB copies within 256 MiB | `sol_1dc164b074404398` | `sol_7152afd04f174228` | pass; post catalog receipt reused its valid cache |
| Four-entry retention violation search | `sol_5043eab2920b428f` | `sol_a31269a86e894a06` | unsat |

Post-policy inputs came from serialized Rust defaults. The arithmetic transition matches the Rust planner's oldest-first removal of unprotected entries while above budget. This is bounded evidence for four entries (each 0–64 GiB), not a proof of arbitrary filesystem or concurrent execution.

## Notebook evaluation of actual implementation

`SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo run -p spur-pm --example beads_maintenance_eval -- docs/research/2026-10-27-beads-maintenance-evaluation.json` passed. The [JSON artifact](2026-10-27-beads-maintenance-evaluation.json) comes from actual Rust APIs on temporary sparse fixtures. Sizes below are logical bytes, not disk-allocation benchmarks or live-repository savings.

| Backup size | Initial copies | Retained copies | Before → after | Reclaimed |
|---|---:|---:|---|---:|
| 10 MiB | 100 | 25 | 1,000 → 250 MiB | 75% |
| 50 MiB | 100 | 5 | 5,000 → 250 MiB | 95% |
| 100 MiB | 100 | 2 | 10,000 → 200 MiB | 98% |
| 500 MiB | 100 | 2 | 50,000 → 1,000 MiB | 98% |

Every case preserved protected/canonical/quarantine fixtures, left all files intact during planning, matched planned/executed reclamation, and was idempotent. The last case reports 744 MiB above budget because two protected copies are retained.

The actor exported an issue in 24 ms with the 20 ms test cadence. A pinned reader kept a 160,712-byte WAL allocated; after releasing it, maintenance truncated the WAL to zero and the committed issue remained readable. Production cadence remains 30 seconds, not the test cadence.

The [notebook](2026-10-27-beads-maintenance-analysis.ipynb) contains three new successful executable cells: schema discovery (`...ec03`), SQL comparisons (`...ec04`), and Python safety/solver assertions (`...ec05`). They ran through Notebook MCP; after the connector closed, the documented stdio proxy reconnected to the existing writer process. An unrelated pre-existing Jev experiment cell was preserved and excluded from this commit.
