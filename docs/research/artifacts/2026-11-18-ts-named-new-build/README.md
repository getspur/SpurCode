# Named TS configuration: new-build verification

**The new build exposes named TypeScript/Deno configurations.** Discovery, saving a named profile and selecting it on a TS cell were verified. Agent/MCP execution then failed before evaluating the source.

[Verification notebook](/Users/kevintruong/project/spur-notebook/2026-11-18-ts-named-configuration-new-build.ipynb) · [Captured notebook](experimental-notebook.ipynb) · [Exact receipts](results.json) · [Verification](verification.json)

This supersedes the earlier runtime-interface observation in the [named-profile recheck](../2026-11-18-deno-named-profile-recheck/README.md). The earlier standalone results remain a separate historical comparison.

## What was verified

| Step | Observed result |
|---|---|
| Live Deno schema | Includes named configuration, project_root and profile identity |
| Initial preview inside the repository | Blocked by an ancestor package.json; the file was preserved |
| Isolated local-only project | Removes the ancestor dependency-mode diagnostic |
| Create-only save | Profile ts-config-probe saved with stable UUID 64e074b9-8f19-4782-b7ad-ef895c3e131f |
| TS cell selection | Persisted metadata.spur.kernel_profile=ts-config-probe |
| Notebook default | Preserved as null / standalone factory |
| Named cell execution | Kernel ensure rejects the exact discovered native slot before source execution |
| Configuration restart | Transport closed before a result; recovered profile remains saved/unapplied |
| Standalone control | Bundled Deno 2.8.1 imports the same local typed module successfully |
| Fixture files | Before/after hashes match |

The old diagnostic notebook was read-only because another app instance owned its writer lease. This verification used a separate new notebook. No lease was bypassed and no existing profile/default was edited.

## Reproduction

The task-owned named profile points to the six-file local fixture at `/private/tmp/spur-ts-named-probe-3f23vd3j`, using the notebook-relative root `../../../../private/tmp/spur-ts-named-probe-3f23vd3j`. [Original fixture source](../2026-11-18-deno-project-configuration/fixture/deno.json) includes a local import alias, JSON and custom JSX settings, with no third-party dependencies.

Cell `8b0f359a-c7ee-4784-96d9-5776a5f7c0f8` explicitly selects this profile. Its source imports the alias, JSON and TSX entrypoint. `notebook_run_cell` returned:

```text
kernel ensure failed: unknown, foreign or superseded native slot;
inspect notebook_kernel_profiles
```

The profile registry returns the same saved UUID and exact native slot. A second attempt through the recovered direct app-owned endpoint returns the same error. The cell has no execution count and no outputs, so no named alias/JSON/JSX success is claimed. The standalone control is separately labeled and saved with its passing output.

The Save and restart call lost its MCP transport before completion. Do not infer startup success, crash cause or full rollback from that event. After reconnect, the durable profile was present, but its runtime was saved/unapplied and no named process was listed. The request was not retried blindly.

The remaining issue is tracked as **bd-2dsf**: diagnose valid named Deno startup/ownership routing while preserving foreign/superseded slot rejection. This is a reproduction, not an implementation fix. Human Run-button execution, prepared third-party dependencies and ports API v2 are untested in this check. The temporary fixture and saved task profile are retained for follow-up; the project path must exist when replaying.
