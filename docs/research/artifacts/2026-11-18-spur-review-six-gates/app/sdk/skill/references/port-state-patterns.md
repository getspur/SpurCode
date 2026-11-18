# Port & State Patterns — reactive and decoupled apps

Load this at **Step 3** when the archetype is `interactive-decoupled` (or any app with
long-lived widget state, repeated interactions, or a frontend that reacts to data
changing over time). Skip it for simple `headless` / `vanilla-widget` render apps.

SpurLab already has the primitives the ChatGPT-Apps "interactive state sync" guidance
reaches for — the **reactive DAG** (`src/dag/`) and **versioned ports**. These patterns
say how to use them deliberately.

## 0. Three state planes (choose deliberately)

| Plane | Mechanism | Durable? | Drives reactive DAG? | Allowed intents |
|-------|-----------|----------|----------------------|-----------------|
| **Port version (source of truth)** | Versioned port store + `source.push` / `ports.pushSource` / `NotebookClient.push_source` | Yes (disk) | Yes | Filters, selection, query forms — anything backend cells must recompute |
| **Widget model state** | AFM `model-state.update` / `model-state.custom` → kernel `comm` | No | No | Cosmetic UI only (panel open, theme, draft labels) |
| **Browser-only** | React/local state | No | No | Ephemeral presentation; never critical actions |

**Decision rules:**

- Filter / selection / query that backend must recompute → **port version only**.
- Imperative action (export, render, send) → **tool call** (`callTool` / `@app.tool`), not a port.
- Cosmetic widget chrome → model-state or browser-only is fine.

**Anti-pattern:** using `model-state.update` for filters or query state that should
rerun polyglot backend cells. That state dies on remount and never cascades the DAG.

### Write-path by context

| Context | API |
|---------|-----|
| Deno kernel cell | `ports.pushSource(port, ipcBytes)` from `@getspur/spurlab-sdk` |
| AFM React widget (iframe) | `experimental.invoke("source.push", { port, payload: number[] })` |
| Python plugin | `NotebookClient().push_source(port, ipc_bytes)` |
| Media capture | `capture.canvas` + host `push_capture_port` (not `pushSource`) |

## 1. The port `version` is your monotonic event token

Every port frame carries a `version` counter (`frame.version` in Python,
`data.version` in TS). Use it the way ChatGPT apps use `stateVersion` / `resetCount`:
to detect "same shape, new event" updates reliably, including refresh/retry/reset where
other fields may not change. Compare the version you last rendered against the current
one instead of diffing payloads.

```ts
import { ports } from "@getspur/spurlab-sdk";

const data = await ports.read("my-port");
// data.bytes / data.mime / data.version / data.kind / data.durationSec
if (data.version !== lastRenderedVersion) {
  lastRenderedVersion = data.version;
  // re-render from the new frame
}
```

## 2. Decouple data cells from the render cell

Keep computation and presentation in separate cells:

- **data cells** fetch/compute/mutate and **write ports** — these can be **polyglot**:
  Python (`python3`), Rust (`evcxr`), Go (`gonb`), or Deno. A Python data cell can write
  a port a Deno render cell reads (cross-language is supported and tested).
- **the render cell** reads ports and focuses on presentation. The reactive DAG re-runs
  it when an upstream port changes — so the widget updates without you re-issuing a tool
  call, and without a full remount.

This is the SpurLab form of "separate data tools from render tools": data lives in the
DAG, render reacts to it.

## 3. User input drives source ports

A Deno/Perspective frontend can be passive or active:

- Passive: the Deno render cell reads upstream ports and builds the Perspective view.
  The reactive DAG re-runs it when those ports change.
- Active: browser-side events push intent into a declared source port, then the DAG
  refreshes the backend and render cells.

For active apps, use anywidget/React as the app bridge. Button, filter, and selection
handlers should call:

```ts
// AFM widget (browser) — needs commId authorization on the host.
// Payloads are dataframe/Arrow IPC bytes encoded as number[].
await experimental.invoke("source.push", {
  port: "query",
  payload: Array.from(arrowIpcBytes),
});

// Deno kernel cell — SDK helper (agent origin via notebook_push_source):
import { ports } from "@getspur/spurlab-sdk";
await ports.pushSource("query", arrowIpcBytes);
```

The target port must be a declared source port, and the byte payload must match that
source's schema. For UI state, encode a small one-row Arrow table rather than keeping
the state only in browser memory.

Use `callTool(...)` instead when the interaction should run an imperative MCP server
tool. A Perspective filter, selected row, or query form should usually be state in a
source port; a "render/export/send" action is usually a tool.

Do not treat a plain `display.html(...)` iframe as a backend control surface. It can
render Perspective and local browser interactions, but it cannot call Deno-side
`ports.read` or rerun backend cells unless it goes through an app bridge.

React is optional UI technology. The reactive part of this pattern is the Spur
reactive DAG engine: source-port updates schedule backend and render cells.

For implementation details on Deno Jupyter, anywidget AFM lifecycle, React bundling,
Perspective rendering, and the industry grounding behind this pattern, load
`anywidget-deno-polyglot.md`.

## 4. Cross-language bridge checklist

Use this checklist when wiring a Deno/React/Perspective frontend to a polyglot
backend:

- Declare every user-driven source in notebook metadata
  (`metadata.spur.dag.source.port`). The runtime rejects pushes to undeclared
  source ports.
- Encode dataframe state as Arrow IPC bytes. Browser `source.push` wraps those
  bytes in a JSON `number[]`; `NotebookClient.push_source(...)` sends the same
  bytes through `notebook_push_source`.
- Match the declared source schema exactly when one is present. Field names and
  Arrow types are validated before the port version is written.
- Treat media as a separate path. Canvas/video capture writes a media port
  (`kind: "media"`, `.media`, `mime`, `size`, optional `duration_sec`); it is
  not a dataframe source push.
- Let the runtime schedule languages. Current DAG kernelspecs are Python/SQL
  through `python3`, Deno through `deno`, Rust through `evcxr`, and Go through
  `gonb`.
- Refresh UI from port `version`, not from hidden browser state. A changed
  version means the backend has published a new frame for the render cell or
  frontend to read.

Perspective remains a renderer in this pattern. User input emits source-port
state, backend cells recompute across their language kernels, and Perspective
refreshes from the new Arrow/media port versions.

## 5. Idempotent, retry-safe handlers

Repeated calls with the same input must not corrupt state. Reads and pure transforms
should be idempotent; a mutating tool should make its side effect explicit in its
description (see `tool-design.md`). The reactive DAG may re-run a downstream cell more
than once — design for it.

## 6. Host-less fallback for fast iteration

Where feasible, let a frontend cell degrade gracefully when host APIs are absent so you
can iterate it in a plain browser/Deno-Jupyter context: if `callTool` / `ports` are
unavailable, render from local placeholder state instead of throwing. This removes the
connector round-trip from the inner loop for pure-UI tweaks.

## Anti-patterns

- Stuffing large blobs into a tool's model-visible return instead of a port.
- Pushing ad hoc JSON rows as source state instead of Arrow IPC bytes that match
  the declared source schema.
- Treating hidden client-side state as the source of truth for a critical action — the
  DAG/port store is the source of truth.
- Using AFM `model-state.update` for filters/query that must recompute backend cells.
- Re-running compute on every render instead of letting the DAG react to a port version
  change.
