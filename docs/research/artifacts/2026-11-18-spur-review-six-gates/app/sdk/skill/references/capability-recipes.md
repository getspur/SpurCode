# Capability Recipes — the four composable host powers

Load this at **Step 1**. Capabilities are an axis orthogonal to the archetype (see
`app-archetypes.md`): declare the ones the app needs, and the host provisions them. The
manifest struct uses `deny_unknown_fields` — unknown keys are **rejected** at
deserialization, so keep the keys exactly as below.

> Never read an env var the manifest did not declare — the host will not inject it.

## The four capabilities

| Key | What the host provisions when declared |
|---|---|
| `ports` | Injects `SPUR_PORTS_ROOT` at plugin spawn. Doctor verifies declared `read` ports exist as DAG sources; user-driven source pushes enter the same DAG/port-store bridge. |
| `canvas_capture` | The full recorder loop: `data-capture` canvas → `jute-video-capture` postMessage → `push_capture_port` → port store (includes `duration_sec`). **Requires `active_output_scripts`.** |
| `active_output_scripts` | One-time per-app trust prompt; on grant, output iframes run with `allow-scripts allow-same-origin`. |
| `artifacts_dir` | Injects `SPUR_ARTIFACTS_DIR` and creates the directory. |

## Minimal manifest (all four declared)

```json
{
  "schema": "spur.app/v1",
  "name": "my-app",
  "entry_notebook": "app.ipynb",
  "open_mode": "app",
  "runtime": { "jute_min": "0.1.0", "features": ["frontend-cells"] },
  "capabilities": {
    "ports": { "read": ["my-port"], "write": [] },
    "canvas_capture": true,
    "active_output_scripts": true,
    "artifacts_dir": true
  },
  "skill": "skill/SKILL.md"
}
```

Declare only what you use. `open_mode: "app"` spawns the plugin and grants caps;
`"notebook"` opens a plain notebook. Full field reference: `sdk/docs/manifest.md`; JSON
Schema: `sdk/schema/spur-app.schema.json`.

## Overlay recipe — capture/recorder (`canvas_capture` + `active_output_scripts`)

Emit a capture canvas from a frontend cell. The recorder loop pushes frames to a media
port that a server tool can then read. Worked at scale in gallery apps such as
`app_gallery/spur-growth`; on a
no-server app in `app_gallery/open_design`.

```ts
import { capture, display } from "@getspur/spurlab-sdk";

const html = capture.canvas({
  port: "my-cell-id",   // must match the DAG source cell id
  fps: 30,
  durationSec: 60,
  width: 1280,
  height: 720,
});
return display.html(html);   // last expression — Deno-Jupyter renders the return value
```

## Overlay recipe — artifact producer (`artifacts_dir`)

A server tool reads a port and writes a file the host can surface. Requires `ports` to
read and `artifacts_dir` to write.

```python
out = app.artifacts.path("renders/out.mp4")   # Path; parent dirs created
out.write_bytes(app.ports.read("clip").bytes)
return {"output": str(out)}
```

If a capability is not declared (or the host did not inject it), the matching accessor
raises `MissingCapabilityError` on first access (`app.ports`, `app.artifacts`).

## Deprecated: `runtime.features`

`features` (e.g. `"frontend-cells"`, `"anywidget-afm"`, `"ports-arrow"`) are honored but
never enforced; doctor warns if present. Express needs through `capabilities`, not
`features`.
