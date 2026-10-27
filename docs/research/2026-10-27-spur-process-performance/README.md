# Live spur process sampling, 2026-10-27

The [executed notebook](../2026-10-27-spur-process-performance.ipynb) analyzes two running SpurCode `spur tui --brain codex` processes using DuckDB 1.5.5 and **quack_flamegraph 253fab9**. Tracking issue: `bd-3nq`.

| Measurement | PID 94421 | PID 91228 |
|---|---:|---:|
| CPU during own sample window (% of one core) | 2.892% | 2.464% |
| Physical footprint during own sample | 3.016 GiB | 1.048 GiB |
| All-thread observations in known wait calls | 99.943% | 99.914% |
| Writer observations outside known wait calls | 2 / 56,262 | 32 / 59,122 |
| TUI share outside known wait calls | 70.4% | 68.4% |

No maintenance bottleneck was visible in these quiet windows. The visible writer work includes lock setup, a dirty-count query, and teardown. The `WriteLockGuard::new → Command::output` chain contributes 4 and 29 observations; current source ties this to the `hostname` fallback in `crates/spur-pm/src/beads_crate/write_lock.rs:277`. It is a small possible optimization target, not a demonstrated user-visible regression. Terminal rendering dominates the observations outside known waits.

These are live observations, **not** a controlled before/after comparison. No large dirty export or 15-minute sweep was induced, and no foreground latency benchmark ran. Optimized maintenance functions are inlined/unnamed; absence of their literal names is not evidence of zero cost. Memory footprint is sizable, but CPU samples cannot attribute retained allocations or establish a leak. Processes were never restarted or signalled, and no live `.beads` files were modified by the profiling scripts.

## Capture and validation

The capture records a 15-second counter-only window, a separate 35-second `/usr/bin/sample <pid> 35 1` run per process, then another 15-second counter-only window. Both processes' CPU/I/O/memory counters are read at each phase boundary. Capture phase durations also include symbol-processing time. The “before” and “after” windows refer to the profiler, not to deployment of maintenance; their CPU differences cannot isolate profiler overhead because the workload is uncontrolled.

The installed executable reports `spur 1.24.0`, has SHA-256 `6586dd8eea64768a6512a2c7fbcf054be504860867549e6351b938c80979021e`, and has Mach-O UUID `4C4C44C7-5555-3144-A16E-1DBED7E31E56`, matching both processes' rusage UUIDs. It contains maintenance markers and was installed after commit `ee46348fd`; this is provenance evidence, not a reproducible-build attestation.

CPU values use `proc_pid_rusage` user/system **Mach ticks**, converted with the host's `mach_timebase_info` ratio **125/3**. The unit conversion was cross-checked against `ps` and an independent 0.2-second `time.process_time()` run (relative error 0.0068%). Raw tick deltas remain in `counters.json`. CPU percentages mean percent of one core, not of all 10 logical CPUs. Apple's [rusage implementation](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c) supplies these task counters.

The unmodified [upstream sample-collapse script](https://github.com/brendangregg/FlameGraph/blob/master/stackcollapse-sample.awk) is accompanied by `include-waits.awk`, which clears its default wait exclusion list. `prepare_profiles.py` verifies nonnegative counts and exact conservation of thread-root observations before omitting zero-count parent rows. Totals: 1,406,550 observations across 50 thread roots for PID 94421; 1,064,198 across 37 roots for PID 91228.

The notebook uses [quack_flamegraph](https://duckdb.org/community_extensions/extensions/quack_flamegraph) `read_folded` and `flamegraph_coverage`. Coverage avoids recursive-frame double counting. The wait classification is an explicit heuristic on leaf symbols, visible in notebook cell `...ae08`. “Other” observations are not guaranteed on-CPU samples, and all-thread observations cannot be divided by nominal sample frequency to recover process CPU time.

## Files and reproduction

- `capture.py`: read-only live capture; recheck current PIDs/workspaces before reuse.
- `spur-*.sample.txt`: complete raw macOS profiles; `.sample.log` records profiler completion.
- `stackcollapse-sample.awk`, `include-waits.awk`, `prepare_profiles.py`: reproducible collapse with all waits retained.
- `spur-*.folded`: complete positive folded stacks.
- `counters.json`, `cpu-unit-validation.json`, `collapse-validation.json`, `analysis-summary.json`: measurement and validation evidence.
- `spur-*.other.folded`, `spur-*.svg`: filtered visualization inputs and interactive SVGs. SVG percentages apply only to observations outside the listed wait calls, not all process CPU.
- `flamegraph.pl`: unmodified [upstream renderer](https://github.com/brendangregg/FlameGraph/blob/master/flamegraph.pl), with its license header retained.

Capture and rebuild folded profiles:

```sh
python3 docs/research/2026-10-27-spur-process-performance/capture.py \
  --pids <current-spur-pid> --seconds 35 \
  --output /tmp/spur-new-profile
```

For the saved capture, run `python3 docs/research/2026-10-27-spur-process-performance/prepare_profiles.py`, then run the notebook cells through Notebook MCP. The saved notebook has ten successful executable cells, including SQL rankings, validation assertions, and two SVG outputs. SVG structure validation found 481 and 383 rectangles respectively. No Rust production code changed in this task.
