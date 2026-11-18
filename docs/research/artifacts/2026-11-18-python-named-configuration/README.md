# Python named configuration exploration

**Named Python execution works through MCP in this build.** Six notebook code cells completed successfully; sixteen checks over their observed outputs, profile identities and configuration state passed.

[Live notebook](/Users/kevintruong/project/spur-notebook/2026-11-18-python-named-configuration-exploration.ipynb) · [Captured notebook](experimental-notebook.ipynb) · [Live schema and operation receipts](results.json) · [Verification](verification.json)

## Verified behavior

Two task-owned profiles, `py-probe-a` and `py-probe-b`, use separate local project roots and the same existing Python 3.12.10 interpreter with ipykernel 7.4.0. Both were saved using create-only UUID/CAS guards, explicitly qualified, selected on cells and executed through `notebook_run_cell`.

| Probe | Result |
|---|---|
| Passive existing-interpreter Preview | No profile identity allocated or profile saved |
| Save | Stable profile UUIDs; no dependency installation or implicit startup |
| Explicit environment qualification | Reviewed plan with no downloads/build hooks and only a private ready receipt as write scope |
| Project A | Module A imported; configured cwd; process 93662 |
| Project B | Module B imported; configured cwd; process 94915; A's globals absent |
| Factory control | Separate process 91094; named globals absent |
| Notebook default A | An unbound cell reused A's process and state |
| Explicit B selector with default A | B overrode the default and retained B's own state |
| Final default | Restored to factory Python |
| Source preservation | Six original fixture files retain their hashes |

PIDs describe this run, not a future runtime identity. The shared interpreter does not establish separate dependency-environment isolation. Profile UUIDs and distinct kernel slots are preserved in the receipts. Both named runtimes were ready and applied in the final live registry.

Project A generated one normal Python bytecode cache after import. Original source/config files were unchanged. Project B's diagnostic cell disabled bytecode generation before importing its fixture. No package installation or existing environment edits were performed by the experiment; qualification recorded the existing interpreter and package inventory.

## Environment modes

| Mode | Contract | Tested scope |
|---|---|---|
| Existing interpreter | Project-relative interpreter or local binding | Project-relative existing interpreter qualified and executed twice; local bindings untested |
| Requirements | Files, constraints, pip/uv installer and hash policy | Passive Preview only; environment not prepared |
| pyproject | Manifest, extras and wheel/editable mode | Passive Preview only; environment not prepared |
| uv project | Manifest, optional package/extras/groups, locked environment policy | Passive Preview reported missing uv.lock as expected; no lock or environment was created |

The schema requires explicit preparation for managed dependencies. A Save or Preview result is not environment readiness. This exploration qualified only existing interpreters; it did not test dependency solving, managed environment creation, upgrades or installer failures.

## Current application limit

Named Python advertises plain execution with **Arrow/data ports unavailable** (`project.version=0`, `available=false`). Both prepared environment receipts and final runtime profiles report that limitation. This notebook does not claim dataframe/Arrow publication through the named Python host bridge. Human UI interaction, restart/recovery and ports publication were not tested.

## Replay and scope

The saved profiles point to `/private/tmp/spur-python-named-_47wrdz6/a` and `/private/tmp/spur-python-named-_47wrdz6/b`. The original three source files for each project are captured in [fixtures](fixtures/a/module_from_project.py). The interpreter reference is local to this machine. A real project would normally point at its prepared interpreter, such as `.venv/bin/python`, or a configured local binding.

The factory setup cell creates new roots when replayed. Keep the original roots, or update the task-owned profile configs to the newly printed roots and qualify them explicitly. Run A and B's selected setup cells before checking their state. To repeat the historical inherited-default check, temporarily choose A as notebook default; the delivered notebook restores factory default. The retained output records execution under that earlier selected context.

The notebook was created and operated entirely through notebook MCP. The immutable capture was copied only after a complete MCP document read. The existing implementation notebook, existing environments and unrelated profiles were preserved. No production code changes or release qualification are claimed.
