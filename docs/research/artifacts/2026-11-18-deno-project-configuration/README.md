# Deno project configuration notebook

[Live notebook](/Users/kevintruong/project/spur-notebook/2026-11-18-deno-project-configuration.ipynb) · [Captured notebook](experimental-notebook.ipynb) · [Diagnostic results](results.json) · [Local fixture](fixture/deno.json)

The new notebook checks Spur Notebook's Deno runtime and project configuration. All seven code cells executed successfully. Ten observations were confirmed, including expected negative alias-import cases. No existing project configuration or dependency manifests were changed.

## Observed environment

- Deno 2.8.1, TypeScript 6.0.3, Apple arm64.
- Executable: `/Users/kevintruong/Applications/SpurLab.app/Contents/MacOS/deno`.
- Working directory: `/Users/kevintruong/project/spur-notebook`.
- No `deno.json` or `deno.jsonc` found at the working directory or its ancestors.
- The only inspected ancestor package file is `/Users/kevintruong/package.json`. Presence does not prove it was loaded by the notebook kernel.

## Configuration experiment

The isolated local fixture has an import alias, a typed module, JSON data, a custom JSX runtime and TS/TSX entrypoints. It uses no third-party packages.

| Probe | Result |
|---|---|
| CLI explicit config: alias + JSON | Passed |
| CLI automatic discovery: alias + JSON | Passed |
| CLI configured JSX runtime | Passed |
| CLI local-only type check | Passed |
| CLI no-config control | Rejected alias as expected |
| Notebook direct file / JSON import | Both loaded |
| Existing notebook importing configured fixture entrypoint | Alias unresolved |
| Existing notebook after changing cwd to fixture | Alias still unresolved; cwd restored |

This establishes that the already-running kernel does not adopt this fixture's import alias through a file import or `Deno.chdir()`. It does not establish that fresh-kernel startup from a configured project is unsupported. The installed `deno jupyter --help` advertises neither `--config` nor `--import-map`; startup implementation and other possible configuration paths were not audited.

The initial `deno check --cached-only` trial was rejected because that subcommand does not accept the flag in this binary. The corrected command used `--no-remote --no-npm --no-lock --config ...` and exited 0. Both trials are retained. These are configuration diagnostics, not a production test suite or an app fix.

## Reproduce

Open the live or captured notebook and run the seven Deno cells in document order. The fixture-creation cell creates a fresh directory under `/private/tmp`; the diagnostics cell records that path and exact command arguments. The saved fixture contains matching source bytes for independent inspection. No package script is run or token/environment value dumped.

Official references: [Deno project configuration](https://docs.deno.com/runtime/fundamentals/configuration/) and [Deno Jupyter](https://docs.deno.com/runtime/reference/cli/jupyter/). The next meaningful probe is fresh configured-kernel startup, followed by the host's project settings and launch behavior. Existing saved outputs make that boundary explicit.
