# Anywidget + Deno Polyglot Frontend Widgets

Load this at **Step 3** when the app uses a Deno frontend cell with
anywidget/React/Perspective and a reactive DAG or polyglot backend. It complements
`port-state-patterns.md`: this file covers widget implementation practice; that file
covers the Spur source-port and versioned-port contract.

## External grounding

- anywidget AFM is browser-standard ESM with `initialize` and `render` lifecycle hooks.
  `render` receives `model`, `el`, `signal`, and `host`; use `signal` for cleanup.
  `experimental.invoke` is host-specific, so use it only for Spur-provided actions such
  as `source.push`. Source: https://anywidget.dev/en/afm/
- React belongs behind the AFM bridge. Use `@anywidget/react` (`createRender`,
  `useModelState`) when React is worth the bundle cost; keep the exported module
  standard AFM. Source: https://github.com/manzt/anywidget/tree/main/packages/react
- anywidget recommends plain ESM first. Use esbuild or Vite when TypeScript, JSX,
  React, or local dependencies need bundling; ship static ESM/CSS for production.
  Source: https://anywidget.dev/en/bundling/
- Deno Jupyter is a TypeScript/JavaScript notebook kernel with Web/Deno APIs, npm
  imports, rich MIME display, and HTML output helpers. Current Deno docs also state
  that `deno jupyter` runs notebook code with `--allow-all`, so do not treat it as a
  sandbox for untrusted apps. Source:
  https://docs.deno.com/runtime/reference/cli/jupyter/
- For React/JSX in Deno, configure `deno.json` explicitly with `jsx: "react-jsx"`,
  `jsxImportSource`, and pinned npm imports. React is best when the React ecosystem is
  needed; Preact is smaller when compatibility is enough. Source:
  https://docs.deno.com/runtime/reference/jsx/
- Arrow is the polyglot data plane: it is a standardized, language-agnostic columnar
  format with libraries across JavaScript, Python, Rust, Go, and other languages.
  Source: https://arrow.apache.org/overview/
- Perspective packages moved to the `@perspective-dev/*` npm family. For new
  frontend widgets use `@perspective-dev/client`, `@perspective-dev/viewer`,
  `@perspective-dev/viewer-datagrid`, and `@perspective-dev/viewer-charts`;
  do not add legacy `@finos/*` Perspective packages to new scaffolds. Sources:
  https://www.npmjs.com/package/@perspective-dev/client and
  https://github.com/perspective-dev/perspective

## Recommended shape

```text
frontend/src/*.tsx
  -> build to browser-standard ESM/CSS
  -> thin Deno frontend cell calls spur.anywidget()
  -> anywidget AFM renders React controls + Perspective
  -> browser event
  -> experimental.invoke("source.push", Arrow IPC intent)
  -> Spur reactive DAG reruns Python / SQL / Deno / Rust / Go cells
  -> cells publish versioned Arrow/media ports
  -> frontend reads changed port version and refreshes Perspective
```

React and Perspective are view technology. The source of truth is the reactive DAG plus
versioned ports.

## Clean project structure

Do not put the React app body in the notebook cell. Keep notebook cells as integration
points and keep application code in files:

```text
my-app/
  app.ipynb
  deno.json
  frontend/
    package.json
    vite.config.ts
    src/
      widget.tsx          # AFM entry: exports render/createRender(App)
      App.tsx             # React UI
      spurHost.ts         # source.push / callTool / ports adapter
      ports.ts            # Arrow/media port readers
      perspective.ts      # Perspective table/view lifecycle
      styles.css
    dist/
      widget.js           # built browser ESM
      styles.css          # built CSS, if emitted separately
  backend/
    python/metrics.py
    rust/metrics.rs
    go/metrics.go
    schemas/query.schema.json
  skill/
    SKILL.md
```

The source boundary is:

```text
author TSX in frontend/src
  -> build/test/typecheck outside the notebook
  -> include frontend/dist assets in the app bundle
  -> Deno cell only wires state + host bridge + widget display
```

`frontend/src` is for maintainable app code. `app.ipynb` is for declaring the app surface,
initial model state, port bindings, source-port metadata, and the Deno loader. The
manifest is embedded in notebook `metadata.spur_app` for current scaffolds.
`frontend/dist` is what the runtime should depend on once packed widget assets are wired.

## Scaffold owner

The default generated app skeleton is owned by `src/spur_app/scaffold.rs`
(`graph://file/c8301c36db3afe72` in the code graph when available). Future template work
should update the Rust scaffold, not only this reference:

- `templates()` decides which scaffold templates exist.
- `scaffold_app(...)` writes the app directory and refuses to overwrite existing files.
- `template_text_files(...)` controls app-root text files such as server files, skill
  files, vendored SDK files, `frontend/src`, build config, and polyglot `backend/` stubs.
- `notebook_for_template(...)` owns the generated `app.ipynb` cells, including the thin
  Deno `spur.anywidget()` loader cell.

When standardizing the TSX widget layout, update the scaffold tests and generated files
together. The `react-widget` template should emit the TSX/Vite frontend skeleton; the
`interactive-decoupled` template should also emit polyglot backend stubs plus a
frontend-emitted `query` source port.

## Thin Deno cell

Today Spur's Deno bootstrap exposes `const { widget } = await spur.anywidget();`. The
helper creates an anywidget AFM from a render function and an optional `imports` prelude,
so the cell should stay small:

```ts
const { widget } = await spur.anywidget();
const widgetModuleUrl = "https://host-or-packaged-asset.example/widget.js";

widget({
  state: {
    inputPort: "query",
    outputPort: "results",
  },
  imports: `
    import { renderApp } from "${widgetModuleUrl}";
  `,
  render(ctx: any) {
    return renderApp(ctx);
  },
});
```

If the app is still prototyping, an inline render function is acceptable. Once the UI is
more than a small demo, move it to `frontend/src`, build it into `frontend/dist`, and keep
the cell as the loader/wiring layer. Do not import raw TSX from the cell for production
unless the host explicitly supports that path; anywidget ultimately needs browser-loadable
ESM. If there is no host-resolvable URL for a local built asset yet, do not pretend a
relative TSX import is production-ready; either keep the render wrapper small or add/use a
proper SDK loader for packed widget assets.

## State boundaries

| State | Put it in | Why |
|---|---|---|
| Button loading/error state | React local state | Ephemeral view concern. |
| Selected tab, collapsed panel | anywidget model or React local state | UI-only, safe to lose on recompute. |
| Filter, selected row, query form | DAG source port | Backend-visible, retryable, refresh-safe. |
| Large dataframe | Versioned Arrow port | Avoids bloating widget model and tool returns. |
| Media capture | Versioned media port | Preserves MIME, size, and duration metadata. |

Do not keep durable interaction state only in browser memory. Browser state disappears on
refresh/remount and cannot drive downstream polyglot cells.

## Event rules

Use `source.push` (AFM) or `ports.pushSource` (Deno cell) when user input should change
**DAG** state. Port version is the only durable source of truth.

```ts
// AFM widget iframe (requires host commId authorization):
await experimental.invoke("source.push", {
  port: "query",
  payload: Array.from(arrowIpcBytes),
});

// Deno kernel cell:
import { ports } from "@getspur/spurlab-sdk";
await ports.pushSource("query", arrowIpcBytes);
```

Rules:

- The target must be a declared `metadata.spur.dag.source.port`.
- Payloads are Arrow IPC bytes encoded as a JSON `number[]`.
- If the source declares a schema, field names and Arrow types must match exactly.
- Encode small UI intent as a one-row Arrow table instead of ad hoc JSON.
- Debounce noisy inputs before pushing. Buttons and committed form submits are better
  source events than every keystroke.

**`model-state.update` / `model-state.custom` are not DAG state.** They only echo into
the kernel widget `comm` channel for cosmetic UI. Never put filters/query that must
recompute backend cells there.

Use `callTool(...)` for imperative work that is not durable DAG state: export, upload,
render, send, create artifact, or call an external service. If the result should feed
later cells, write it to a port instead of returning a large payload to the model.

## Perspective rendering

- Treat Perspective as a renderer over the latest Arrow bytes, not as the owner of
  application state.
- Refresh from port `version`. When the version changes, read the new port payload and
  update/recreate the Perspective table as needed.
- Keep Perspective configuration separate from backend state. UI layout may live in the
  widget model (`model-state.*`, ephemeral); data filters that must rerun backend work
  must live in source ports (durable DAG).
- Dispose old tables/views/listeners during AFM cleanup or React unmount to avoid leaking
  workers and DOM listeners.

## Deno and bundling

- Prefer a small AFM for notebook-first apps. Add React only when component state,
  composition, or ecosystem packages justify it.
- For structured apps, author React in `.tsx` files and build to browser ESM; do not keep
  long React components inline in `app.ipynb`.
- Pin imports in `deno.json` or the bundler config. Avoid unversioned CDN imports in
  app bundles.
- For React widgets, use Vite or esbuild to emit one ESM entry and CSS file. During
  development, anywidget/Vite HMR is fine; production `.spurapp` bundles should carry
  static assets through `widget_assets` / manifest `widgets`.
- For Perspective, use `@perspective-dev/*` package names. Treat `@finos/*`
  Perspective imports as legacy unless maintaining an old app.
- Keep host APIs behind a narrow adapter so the React component can render with fallback
  data outside Spur. The adapter owns `source.push`, `callTool`, and `ports.read`.

## Polyglot model

The polyglot boundary is the port store, not a shared in-memory object:

| Runtime role | Recommended language | Contract |
|---|---|---|
| UI/render | Deno + anywidget/React/Perspective | Reads port versions; emits source events. |
| Analytics/dataframe work | Python or SQL-on-Python | Reads/writes Arrow ports. |
| Fast transforms | Rust (`evcxr`) | Reads/writes Arrow ports. |
| Service-style transforms | Go (`gonb`) | Reads/writes Arrow ports. |

Keep schemas stable and explicit. If a source/output schema changes, update the DAG
metadata, SDK readers, fixtures, and app docs together.

## Validation checklist

- React/TSX app code lives under `frontend/src`; the notebook cell is a small
  `spur.anywidget()` loader.
- Perspective imports use `@perspective-dev/*`, not legacy `@finos/*`.
- `frontend/dist` contains browser-standard ESM/CSS and is included in app packing via
  widget assets.
- The anywidget ESM loads and `render` uses `signal` cleanup for DOM/model listeners.
- React state does not contain large dataframe/media payloads.
- Every user-driven backend refresh goes through `source.push` or `callTool`, not plain
  iframe JavaScript.
- `source.push` payloads are Arrow IPC bytes matching the declared schema.
- A port `version` change refreshes Perspective without a full app remount.
- Python, SQL, Deno, Rust, and Go cells communicate only through Arrow/media ports.
- Untrusted notebooks are not run under Deno Jupyter.

## Anti-patterns

- React calls backend compute on every render.
- Large React components live inline in notebook cells after the prototype phase.
- The Deno cell imports raw TSX directly and assumes the browser-side anywidget runtime
  can load it.
- Perspective owns canonical filter/query state when backend cells need that state.
- Full dataframes are stored in anywidget model state.
- Plain `display.html(...)` buttons are expected to rerun backend cells.
- Each language invents its own JSON shape instead of using Arrow IPC ports.
