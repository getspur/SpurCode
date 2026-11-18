# Tool Design — server tools and frontend cells

Load this at **Step 3**, when writing the Python MCP server tools and the TypeScript
frontend cells. For the reactive/stateful patterns of `interactive-decoupled` apps, also
load `port-state-patterns.md`.

## Designing the tool surface

- **One job per tool.** A tool that mixes an unrelated read and write is two tools.
- **Descriptions start with a behavior cue:** "Use this when …". The model picks tools
  from descriptions — write them for the model, not for humans.
- **Make inputs explicit and machine-friendly:** required fields, enums, bounds. Avoid
  free-form strings where an enum fits.
- **Partition the return value intentionally:**
  - model-visible result — concise; the agent reasons over it and may chain on it.
  - widget-only / large payloads — keep them out of the small model-visible result; hand
    bulk data to the frontend through a **port**, not the tool return (see
    `port-state-patterns.md`).
- **Idempotency:** reads and pure transforms should be safe to retry. If a tool mutates
  or is non-idempotent, say so in its description.

## Python MCP server — `App` + capability accessors

Install `spur_app` (canonical source `sdk/python/`; published as `spurlab-sdk`).

```python
from spur_app import App

app = App("my-app")          # name must match spur-app.json "name"

@app.tool()
def process(port_names: list[str], output_path: str) -> dict:
    """Use this when the user wants to render the captured clip to a file."""
    frame = app.ports.read(port_names[0])     # requires capabilities.ports
    # frame.bytes / .mime / .version / .kind ("arrow"|"media") / .duration_sec / .path
    out = app.artifacts.path(output_path)     # requires capabilities.artifacts_dir
    out.write_bytes(frame.bytes)
    return {"output": str(out)}               # concise, model-visible

if __name__ == "__main__":
    app.run()                                 # stdio transport
```

Undeclared capability → `MissingCapabilityError` on first `app.ports` / `app.artifacts`
access. Custom env declared in `mcp_server.env` is reachable via `app.env.path(...)`.
Full API: `sdk/docs/python-sdk.md`.

## TypeScript frontend cell — call a tool, render the result

Each example is a separate notebook cell. `display.*` must be the **last expression** (or
`return`) — Deno-Jupyter renders only the cell's return value.

```ts
import { callTool, display } from "@getspur/spurlab-sdk";

const result = await callTool("process", {
  port_names: ["my-port"],
  output_path: "renders/out.mp4",
});
return display.html(`<video controls src="..."></video>`);
```

`callTool` connects to `SPUR_NOTEBOOK_MCP_SOCKET` (host-injected), runs the MCP
initialize handshake, and calls `tools/call`. Full API: `sdk/docs/typescript-sdk.md`.

## Test your tools without a live host

Use `spur_app.testing.FakePortStore` so Level-1 checks (see `validation-ladder.md`) can
exercise a tool before any host is involved.

```python
from spur_app.testing import FakePortStore
from pathlib import Path

# N = directories between this test file and the repo root:
#   my-app/tests/test_foo.py          → parents[2]
#   app_gallery/spur-growth/tests/...  → parents[3]
FIXTURES = Path(__file__).resolve().parents[2] / "sdk" / "fixtures" / "port-store"

def test_process():
    with FakePortStore.from_fixtures(FIXTURES):   # patches SPUR_PORTS_ROOT
        result = process(["spur-ad-capture"], "out.mp4")
        assert result["output"].endswith("out.mp4")
```

Or build a store programmatically:

```python
with FakePortStore() as store:
    store.add_media("clip", b"fake-bytes", mime="video/mp4", duration_sec=5.0)
    store.add_arrow("data", b"ipc-bytes")
    assert store.port_store.read("clip").duration_sec == 5.0
```
