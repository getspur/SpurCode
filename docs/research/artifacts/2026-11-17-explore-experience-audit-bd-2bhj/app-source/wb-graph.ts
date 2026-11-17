import { display, ports, callTool } from "@getspur/spurlab-sdk";
import { tableFromIPC } from "npm:apache-arrow@21.1.0";

const PERSPECTIVE_VERSION = "4.5.1";
const DEFAULT_SYMBOL = "PortStore";
const DEFAULT_FILE = "src/dag/ports.rs";
const DEFAULT_DEPTH = 2;
const EMPTY_TEXT = "No graph evidence matched the current query.";

type EvidencePort = {
  name: string;
  rows: Record<string, any>[];
  status: string;
  version: number | null;
  bytes: number;
  message: string;
};

type WorkbenchState = {
  query: {
    symbol: string;
    file: string;
    depth: number;
    status: string;
    error: string;
  };
  ports: EvidencePort[];
};

function esc(value: unknown): string {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch] ?? ch));
}

function jsonScript(value: unknown): string {
  return JSON.stringify(value)
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/&/g, "\\u0026");
}

function normalizeValue(value: any): any {
  if (value === null || value === undefined) return null;
  if (typeof value === "bigint") {
    const asNumber = Number(value);
    return Number.isSafeInteger(asNumber) ? asNumber : value.toString();
  }
  if (typeof value?.toJSON === "function") return normalizeValue(value.toJSON());
  if (Array.isArray(value)) return value.map(normalizeValue);
  if (typeof value === "object") {
    const output: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) output[key] = normalizeValue(item);
    return output;
  }
  return value;
}

function tableRows(table: any): Record<string, any>[] {
  if (!table || typeof table.toArray !== "function") return [];
  return table.toArray().map((row: any) => normalizeValue(row));
}

async function readEvidencePort(name: string): Promise<EvidencePort> {
  try {
    const data = await ports.read(name);
    const looksArrow = data.kind === "arrow" || (data.kind == null && (data.bytes?.byteLength ?? 0) > 0);
    if (!looksArrow) {
      return {
        name,
        rows: [],
        status: "not_arrow",
        version: data.version,
        bytes: data.bytes.byteLength,
        message: `${name} is ${data.kind}, not Arrow IPC`,
      };
    }
    const table = tableFromIPC(data.bytes);
    const rows = tableRows(table);
    return {
      name,
      rows,
      status: rows.length ? "ok" : "empty",
      version: data.version,
      bytes: data.bytes.byteLength,
      message: rows.length ? "" : EMPTY_TEXT,
    };
  } catch (error) {
    return {
      name,
      rows: [],
      status: "missing",
      version: null,
      bytes: 0,
      message: error instanceof Error ? error.message : String(error),
    };
  }
}

function evidencePort(name: string, rows: unknown): EvidencePort {
  const normalized = Array.isArray(rows) ? rows.map(normalizeValue) : [];
  return {
    name,
    rows: normalized,
    status: normalized.length ? "ok" : "empty",
    version: null,
    bytes: 0,
    message: normalized.length ? "" : EMPTY_TEXT,
  };
}

async function fetchInitialState(symbol: string, file: string, depth: number): Promise<WorkbenchState> {
  try {
    const result: any = await callTool("wb_query", { symbol, file, depth });
    if (result?.ok) {
      return {
        query: {
          symbol: String(result.symbol || symbol),
          file: String(result.file || file || symbol),
          depth: Number(result.depth ?? depth),
          status: "live",
          error: "",
        },
        ports: [
          evidencePort("subgraph", result.subgraph),
          evidencePort("scorecard", result.scorecard),
          evidencePort("analyst_rows", result.analyst_rows),
          evidencePort("cochange", result.cochange),
        ],
      };
    }
  } catch (_error) {
    // The app may be loaded before the plugin is restarted. Fall back to the
    // last DAG evidence ports so the workbench still renders.
  }

  return {
    query: { symbol, file: file || symbol, depth, status: "ports", error: "" },
    ports: [
      await readEvidencePort("subgraph"),
      await readEvidencePort("scorecard"),
      await readEvidencePort("analyst_rows"),
      await readEvidencePort("cochange"),
    ],
  };
}

const initialState = await fetchInitialState(DEFAULT_SYMBOL, DEFAULT_FILE, DEFAULT_DEPTH);

const html = `
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@perspective-dev/viewer@${PERSPECTIVE_VERSION}/dist/css/pro-dark.css">
<style>
:root{color-scheme:dark}html,body{margin:0;height:100%;min-height:100dvh;background:#090a0d;overflow:hidden}.cg-app{position:fixed;inset:0;width:100vw;height:100dvh;min-height:640px;box-sizing:border-box;display:grid;grid-template-columns:320px minmax(520px,1fr) 360px;grid-template-rows:64px minmax(340px,1fr) 252px;background:#090a0d;color:#e7eef0;font:12px/1.45 Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;overflow:hidden}.cg-app *{box-sizing:border-box}.cg-querybar{grid-column:1/4;display:grid;grid-template-columns:300px minmax(0,1fr) auto;gap:12px;align-items:center;border-bottom:1px solid #263036;background:#0d1114;padding:0 12px;min-width:0}.cg-brand{display:flex;align-items:center;gap:10px;min-width:0}.cg-mark{width:13px;height:13px;border-radius:50%;background:#2bd1a3;box-shadow:0 0 18px rgba(43,209,163,.65)}.cg-title{min-width:0}.cg-title h1{margin:0;color:#f6f8fa;font-size:14px;font-weight:800;letter-spacing:0}.cg-title div{color:#829099;font-size:11px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.cg-form{display:grid;grid-template-columns:minmax(180px,1.1fr) minmax(160px,.9fr) 96px 82px;gap:8px;align-items:end;min-width:0}.cg-field{display:grid;gap:3px;min-width:0}.cg-field span{color:#829099;font-size:10px;font-weight:800;text-transform:uppercase}.cg-field input,.cg-field select{width:100%;height:32px;border:1px solid #35424a;border-radius:6px;background:#090e11;color:#edf4f6;padding:0 9px;font:12px/1.2 inherit;outline:none}.cg-field input:focus,.cg-field select:focus{border-color:#2bd1a3;box-shadow:0 0 0 2px rgba(43,209,163,.16)}.cg-button{height:32px;border:1px solid #238b70;border-radius:6px;background:#1fb98f;color:#06100d;font-weight:850;cursor:pointer}.cg-button:disabled{border-color:#38454c;background:#20282d;color:#7f8d95;cursor:wait}.cg-status{display:flex;align-items:center;justify-content:flex-end;gap:6px;min-width:0}.cg-chip{display:inline-flex;align-items:center;gap:6px;border:1px solid #374148;background:#151a1e;color:#b8c6cc;border-radius:999px;padding:4px 8px;white-space:nowrap}.cg-chip strong{color:#74828a;font-weight:700}.cg-chip.ok{border-color:#1f745e;color:#8ef0c9;background:#0f211c}.cg-chip.warn{border-color:#8a6424;color:#ffd485;background:#21190d}.cg-chip.idle{border-color:#3a454d;color:#9ba9b0;background:#11171b}.cg-chip.busy{border-color:#2a6d8f;color:#94d8ff;background:#0d1b23}.cg-pane{min-width:0;min-height:0;border-right:1px solid #263036;border-bottom:1px solid #263036;background:#0b0f12;overflow:hidden}.cg-pane-head{height:42px;display:flex;align-items:center;justify-content:space-between;gap:10px;padding:0 12px;border-bottom:1px solid #222b31;background:#11161a}.cg-pane-head h2{margin:0;color:#f6f8fa;font-size:11px;font-weight:800;text-transform:uppercase;letter-spacing:0}.cg-pane-head span{color:#829099;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.cg-left{grid-column:1;grid-row:2/4}.cg-center{grid-column:2;grid-row:2}.cg-right{grid-column:3;grid-row:2/4;border-right:0}.cg-bottom{grid-column:2;grid-row:3;display:grid;grid-template-columns:1fr 1fr;border-right:1px solid #263036}.cg-kpis{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;padding:10px}.cg-kpi{border:1px solid #263036;background:#10161a;border-radius:8px;padding:8px;min-width:0}.cg-kpi span{display:block;color:#7f8d95;font-size:10px;text-transform:uppercase;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.cg-kpi strong{display:block;color:#f6f8fa;font-size:18px;margin-top:2px;font-variant-numeric:tabular-nums;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.cg-kpi.green strong{color:#6ce6bd}.cg-kpi.amber strong{color:#f1b45a}.cg-kpi.blue strong{color:#66c7ff}.cg-kpi.coral strong{color:#ff8f8f}.cg-perspective-wrap{height:100%;min-height:0;padding:8px}.cg-left .cg-perspective-wrap{height:calc(100% - 158px)}.cg-right .cg-perspective-wrap{height:calc(100% - 270px);min-height:180px}perspective-viewer{width:100%;height:100%;border:1px solid #263036;border-radius:8px;background:#0b0f12;overflow:hidden}.cg-topology-wrap{height:100%;padding:10px;display:grid;grid-template-rows:minmax(0,1fr) 34px;gap:8px}.cg-cy{width:100%;height:100%;min-height:0;border:1px solid #263036;border-radius:8px;background:#080a0d;overflow:hidden}.cg-legend{display:flex;align-items:center;gap:8px;min-width:0;overflow:hidden}.cg-legend span{border:1px solid #2c3940;background:#11181c;color:#b7c3c9;border-radius:999px;padding:3px 8px;white-space:nowrap}.cg-empty{height:100%;display:flex;flex-direction:column;align-items:center;justify-content:center;border:1px dashed #3a474f;border-radius:8px;color:#a8b4ba;background:#0d1215;text-align:center;padding:16px}.cg-empty strong{color:#f1b45a;margin-bottom:4px}.cg-detail{padding:12px;display:grid;gap:10px}.cg-symbol{border-bottom:1px solid #263036;padding-bottom:10px;min-width:0}.cg-symbol h3{margin:0 0 5px;color:#f6f8fa;font-size:14px;line-height:1.3;overflow-wrap:anywhere}.cg-symbol div{color:#8998a0;overflow-wrap:anywhere}.cg-score-grid{display:grid;grid-template-columns:1fr 1fr;gap:8px}.cg-toast{position:absolute;right:14px;top:74px;z-index:20;max-width:min(520px,calc(100vw - 28px));border:1px solid #8a6424;background:#21190d;color:#ffd485;border-radius:8px;padding:8px 10px;box-shadow:0 12px 30px rgba(0,0,0,.35);display:none}.cg-toast[data-show="true"]{display:block}@media (max-width:1180px){.cg-app{grid-template-columns:280px minmax(420px,1fr);grid-template-rows:104px minmax(340px,1fr) 300px 280px}.cg-querybar{grid-column:1/3;grid-template-columns:1fr;gap:6px;padding:8px 12px}.cg-status{display:none}.cg-form{grid-template-columns:minmax(160px,1fr) minmax(140px,1fr) 86px 78px}.cg-left{grid-column:1;grid-row:2/4}.cg-center{grid-column:2;grid-row:2}.cg-bottom{grid-column:2;grid-row:3}.cg-right{grid-column:1/3;grid-row:4;border-right:0}.cg-right .cg-perspective-wrap{height:calc(100% - 130px)}}@media (max-width:780px){html,body{overflow:auto}.cg-app{position:relative;grid-template-columns:1fr;grid-template-rows:auto 360px 360px 320px 360px;min-height:100dvh;height:auto;overflow:visible}.cg-querybar{position:sticky;top:0;z-index:5;grid-column:1;grid-template-columns:1fr}.cg-form{grid-template-columns:1fr 1fr;align-items:end}.cg-left,.cg-center,.cg-right,.cg-bottom{grid-column:1;grid-row:auto}.cg-bottom{grid-template-columns:1fr;grid-template-rows:1fr 1fr}.cg-pane{border-right:0}.cg-left .cg-perspective-wrap,.cg-right .cg-perspective-wrap{height:calc(100% - 128px)}}
</style>
<section class="cg-app" data-wb="workbench">
  <header class="cg-querybar">
    <div class="cg-brand"><div class="cg-mark"></div><div class="cg-title"><h1>Code Graph Workbench</h1><div id="cg-subtitle">Perspective ${esc(PERSPECTIVE_VERSION)}</div></div></div>
    <form id="cg-query-form" class="cg-form" data-wb-query="form">
      <label class="cg-field"><span>Symbol</span><input data-wb-query="symbol" name="symbol" value="${esc(initialState.query.symbol)}" autocomplete="off" spellcheck="false"></label>
      <label class="cg-field"><span>File</span><input data-wb-query="file" name="file" value="${esc(initialState.query.file)}" autocomplete="off" spellcheck="false"></label>
      <label class="cg-field"><span>Depth</span><select data-wb-query="depth" name="depth"><option value="0">0</option><option value="1">1</option><option value="2" selected>2</option><option value="3">3</option></select></label>
      <button id="cg-run" class="cg-button" type="submit">Run</button>
    </form>
    <div id="cg-status" class="cg-status"></div>
  </header>
  <aside class="cg-pane cg-left"><div class="cg-pane-head"><h2>Blast Radius</h2><span id="cg-blast-line"></span></div><div id="cg-kpis" class="cg-kpis"></div><div class="cg-perspective-wrap"><perspective-viewer id="wb-perspective-blast" plugin="Datagrid"></perspective-viewer></div></aside>
  <main class="cg-pane cg-center"><div class="cg-pane-head"><h2>Topology</h2><span id="cg-topology-line"></span></div><div class="cg-topology-wrap"><div id="cg-cy" class="cg-cy"></div><div id="cg-legend" class="cg-legend"></div></div></main>
  <aside class="cg-pane cg-right"><div class="cg-pane-head"><h2>Inspector</h2><span id="cg-scorecard-line"></span></div><div class="cg-detail"><div class="cg-symbol"><h3 id="cg-insp-name"></h3><div id="cg-insp-file"></div><div id="cg-insp-kind"></div></div><div id="cg-score-grid" class="cg-score-grid"></div></div><div class="cg-perspective-wrap"><perspective-viewer id="wb-perspective-scorecard" plugin="Datagrid"></perspective-viewer></div></aside>
  <section class="cg-pane cg-bottom"><div class="cg-pane"><div class="cg-pane-head"><h2>Nodes</h2><span id="cg-node-line"></span></div><div class="cg-perspective-wrap"><perspective-viewer id="wb-perspective-nodes" plugin="Datagrid"></perspective-viewer></div></div><div class="cg-pane"><div class="cg-pane-head"><h2>Edges / Co-change</h2><span id="cg-edge-line"></span></div><div class="cg-perspective-wrap"><perspective-viewer id="wb-perspective-edges" plugin="Datagrid"></perspective-viewer></div></div></section>
  <div id="cg-toast" class="cg-toast" role="status"></div>
  <script type="application/json" id="cg-initial-state">${jsonScript(initialState)}</script>
  <script type="module">
    import perspective from "https://cdn.jsdelivr.net/npm/@perspective-dev/client@${PERSPECTIVE_VERSION}/dist/cdn/perspective.js";
    import "https://cdn.jsdelivr.net/npm/@perspective-dev/viewer@${PERSPECTIVE_VERSION}/dist/cdn/perspective-viewer.js";
    import "https://cdn.jsdelivr.net/npm/@perspective-dev/viewer-datagrid@${PERSPECTIVE_VERSION}/dist/cdn/perspective-viewer-datagrid.js";
    import cytoscape from "https://esm.sh/cytoscape@3.30.2";

    const EMPTY_TEXT = ${jsonScript(EMPTY_TEXT)};
    const worker = await perspective.worker();
    const viewerTables = new Map();
    let cy = null;
    let currentState = JSON.parse(document.getElementById("cg-initial-state").textContent || "{}");

    const schemas = {
      blast: {entity_name: "string", symbol_kind: "string", caller_count: "integer", hot_caller_count: "integer", caller_churn_90d: "integer", blast_radius_score: "float", file_path: "string", stable_symbol_id: "string"},
      scorecard: {entity_name: "string", qualified_name: "string", symbol_kind: "string", pagerank: "float", in_degree: "integer", out_degree: "integer", callers: "integer", churn_90d: "integer", posture: "string", file_path: "string"},
      nodes: {depth: "integer", entity_name: "string", symbol_kind: "string", qualified_name: "string", file_path: "string", stable_symbol_id: "string"},
      edges: {depth: "integer", edge_kind: "string", relation: "string", source_stable_id: "string", target_stable_id: "string", file_a: "string", file_b: "string", cochange_count: "integer", has_static_edge: "boolean"},
    };

    function text(id, value) {
      const el = document.getElementById(id);
      if (el) el.textContent = String(value ?? "");
    }

    function html(id, value) {
      const el = document.getElementById(id);
      if (el) el.innerHTML = value;
    }

    function escapeHtml(value) {
      return String(value ?? "").replace(/[&<>"']/g, function(ch) {
        if (ch === "&") return "&amp;";
        if (ch === "<") return "&lt;";
        if (ch === ">") return "&gt;";
        if (ch === '"') return "&quot;";
        if (ch === "'") return "&#39;";
        return ch;
      });
    }

    function fmtInt(value) {
      const n = Number(value ?? 0);
      return Number.isFinite(n) ? Math.round(n).toLocaleString("en-US") : "0";
    }

    function fmtNumber(value, digits) {
      const n = Number(value);
      return Number.isFinite(n) ? n.toFixed(digits ?? 3) : "-";
    }

    function shortId(value) {
      const s = String(value ?? "");
      if (!s) return "-";
      return s.length > 20 ? s.slice(0, 10) + "..." + s.slice(-6) : s;
    }

    function labelFor(row) {
      return String(row?.entity_name || row?.qualified_name || shortId(row?.stable_symbol_id));
    }

    function rowsFor(state, name) {
      const port = (state.ports || []).find(function(item) { return item.name === name; });
      return Array.isArray(port?.rows) ? port.rows : [];
    }

    function portFor(state, name) {
      return (state.ports || []).find(function(item) { return item.name === name; }) || {name, rows: [], status: "missing", version: null, message: EMPTY_TEXT};
    }

    function portLine(port) {
      if (port.status !== "ok") return port.status || "missing";
      const tag = port.version == null ? "live" : "v" + port.version;
      return tag + " / " + fmtInt((port.rows || []).length) + " rows";
    }

    function statusClass(port) {
      if (port.status === "ok") return "ok";
      if (port.status === "empty" || port.status === "missing") return "idle";
      return "warn";
    }

    function chip(label, value, klass) {
      return '<span class="cg-chip ' + (klass || "") + '"><strong>' + escapeHtml(label) + '</strong>' + escapeHtml(value) + '</span>';
    }

    function kpi(label, value, accent) {
      return '<div class="cg-kpi ' + (accent || "") + '"><span>' + escapeHtml(label) + '</span><strong>' + escapeHtml(value) + '</strong></div>';
    }

    function nodeFill(kind) {
      const value = String(kind || "").toLowerCase();
      if (value.includes("function") || value.includes("method")) return "#2bd1a3";
      if (value.includes("struct") || value.includes("class") || value.includes("enum") || value.includes("trait")) return "#66c7ff";
      if (value.includes("module") || value.includes("file")) return "#b48cff";
      if (value.includes("constant")) return "#f1b45a";
      return "#9aa5ad";
    }

    function normalizeRows(rows) {
      return Array.isArray(rows) ? rows.map(function(row) { return row || {}; }) : [];
    }

    function evidencePort(name, rows) {
      const normalized = normalizeRows(rows);
      return {name, rows: normalized, status: normalized.length ? "ok" : "empty", version: null, bytes: 0, message: normalized.length ? "" : EMPTY_TEXT};
    }

    function stateFromQueryResult(result, requested) {
      if (!result || !result.ok) {
        return {query: {symbol: requested.symbol, file: requested.file || requested.symbol, depth: requested.depth, status: "error", error: String(result?.error || "query failed")}, ports: currentState.ports || []};
      }
      return {
        query: {symbol: String(result.symbol || requested.symbol), file: String(result.file || requested.file || requested.symbol), depth: Number(result.depth ?? requested.depth), status: "live", error: ""},
        ports: [
          evidencePort("subgraph", result.subgraph),
          evidencePort("scorecard", result.scorecard),
          evidencePort("analyst_rows", result.analyst_rows),
          evidencePort("cochange", result.cochange),
        ],
      };
    }

    function buildModel(state) {
      const subgraph = rowsFor(state, "subgraph");
      const edgeRows = subgraph.filter(function(row) { return row.row_kind === "edge" || row.source_stable_id || row.target_stable_id; });
      const nodeRows = subgraph.filter(function(row) { return row.row_kind === "node" || (row.stable_symbol_id && !row.source_stable_id && !row.target_stable_id); });
      const nodesById = new Map();
      nodeRows.forEach(function(row) {
        const id = String(row.stable_symbol_id || "");
        if (id) nodesById.set(id, row);
      });
      edgeRows.forEach(function(edge) {
        [edge.source_stable_id, edge.target_stable_id].forEach(function(endpoint) {
          const id = String(endpoint || "");
          if (id && !nodesById.has(id)) nodesById.set(id, {stable_symbol_id: id, entity_name: shortId(id), symbol_kind: "external", depth: edge.depth ?? 1});
        });
      });
      const nodes = Array.from(nodesById.values()).slice(0, 120);
      const nodeIds = new Set(nodes.map(function(node) { return String(node.stable_symbol_id || ""); }));
      const cyNodes = nodes.map(function(node) {
        return {data: {id: String(node.stable_symbol_id || ""), label: labelFor(node).slice(0, 28), kind: String(node.symbol_kind || ""), qname: String(node.qualified_name || labelFor(node)), file: String(node.file_path || ""), depth: Number(node.depth ?? 0), root: Number(node.depth ?? 0) === 0 ? 1 : 0}};
      });
      const cyEdges = [];
      edgeRows.slice(0, 260).forEach(function(edge, index) {
        const source = String(edge.source_stable_id || "");
        const target = String(edge.target_stable_id || "");
        if (source && target && nodeIds.has(source) && nodeIds.has(target)) {
          cyEdges.push({data: {id: "cge" + index, source, target, kind: String(edge.edge_kind || ""), hot: edge.is_shortest_path ? 1 : 0, dyn: String(edge.edge_kind || "") === "calls_dyn" ? 1 : 0}});
        }
      });
      const scorecard = rowsFor(state, "scorecard");
      const blast = rowsFor(state, "analyst_rows");
      const cochange = rowsFor(state, "cochange");
      const tables = {
        blast: blast.map(function(row) { return {entity_name: String(row.entity_name || ""), symbol_kind: String(row.symbol_kind || ""), caller_count: Number(row.caller_count ?? 0), hot_caller_count: Number(row.hot_caller_count ?? 0), caller_churn_90d: Number(row.caller_churn_90d ?? 0), blast_radius_score: Number(row.blast_radius_score ?? 0), file_path: String(row.file_path || ""), stable_symbol_id: String(row.stable_symbol_id || "")}; }),
        scorecard: scorecard.map(function(row) { return {entity_name: String(row.entity_name || ""), qualified_name: String(row.qualified_name || ""), symbol_kind: String(row.symbol_kind || ""), pagerank: Number(row.pagerank ?? 0), in_degree: Number(row.in_degree ?? 0), out_degree: Number(row.out_degree ?? 0), callers: Number(row.callers ?? 0), churn_90d: Number(row.churn_90d ?? 0), posture: String(row.posture || ""), file_path: String(row.file_path || "")}; }),
        nodes: nodes.map(function(node) { return {depth: Number(node.depth ?? 0), entity_name: labelFor(node), symbol_kind: String(node.symbol_kind || ""), qualified_name: String(node.qualified_name || ""), file_path: String(node.file_path || ""), stable_symbol_id: String(node.stable_symbol_id || "")}; }),
        edges: edgeRows.map(function(edge) { return {depth: Number(edge.depth ?? 0), edge_kind: String(edge.edge_kind || ""), relation: String(edge.relation || ""), source_stable_id: String(edge.source_stable_id || ""), target_stable_id: String(edge.target_stable_id || ""), confidence: String(edge.confidence || ""), bind_method: String(edge.bind_method || ""), is_shortest_path: Boolean(edge.is_shortest_path)}; }),
        cochange: cochange.map(function(row) { return {file_a: String(row.file_a || ""), file_b: String(row.file_b || ""), cochange_count: Number(row.cochange_count ?? 0), has_static_edge: Boolean(row.has_static_edge)}; }),
      };
      return {nodes, edgeRows, cyElements: {nodes: cyNodes, edges: cyEdges}, tables};
    }

    async function loadViewer(id, rows, schema, config) {
      const viewer = document.getElementById(id);
      if (!viewer) return;
      const previous = viewerTables.get(id);
      const table = await worker.table(rows.length ? rows : schema);
      await viewer.load(table);
      if (config && typeof viewer.restore === "function") await viewer.restore(config);
      viewerTables.set(id, table);
      if (previous && typeof previous.delete === "function") await previous.delete();
    }

    function renderGraph(model) {
      const container = document.getElementById("cg-cy");
      if (!container) return;
      if (!model.cyElements.nodes.length) {
        container.innerHTML = '<div class="cg-empty"><strong>No graph evidence</strong><span>' + escapeHtml(EMPTY_TEXT) + '</span></div>';
        if (cy) { cy.destroy(); cy = null; }
        return;
      }
      container.innerHTML = "";
      if (!cy) {
        cy = cytoscape({
          container,
          elements: model.cyElements,
          style: [
            {selector: "node", style: {"background-color": function(ele) { return nodeFill(ele.data("kind")); }, "label": "data(label)", "color": "#cdd8dd", "font-size": 7, "text-valign": "bottom", "text-halign": "center", "text-margin-y": 2, "text-background-color": "#080a0d", "text-background-opacity": 0.65, "text-background-padding": 1.5, "width": 11, "height": 11, "border-width": 1, "border-color": "#091014"}},
            {selector: "node[root = 1]", style: {"width": 22, "height": 22, "border-width": 2, "border-color": "#f7fbff", "font-size": 10, "font-weight": "bold", "color": "#f6f8fa"}},
            {selector: "edge", style: {"width": 1, "line-color": "#355767", "curve-style": "haystack", "opacity": 0.55}},
            {selector: "edge[dyn = 1]", style: {"line-color": "#ff6b6b"}},
            {selector: "edge[hot = 1]", style: {"line-color": "#f1b45a", "width": 2.2, "opacity": 0.95}},
            {selector: ".cg-faded", style: {"opacity": 0.1, "text-opacity": 0.08}},
            {selector: ".cg-pick", style: {"border-color": "#2bd1a3", "border-width": 3}},
          ],
          wheelSensitivity: 0.2,
          minZoom: 0.2,
          maxZoom: 3,
        });
        cy.on("tap", "node", function(evt) {
          const node = evt.target;
          cy.elements().addClass("cg-faded");
          node.closedNeighborhood().removeClass("cg-faded");
          cy.elements().removeClass("cg-pick");
          node.addClass("cg-pick");
          text("cg-insp-name", node.data("qname") || node.data("label"));
          text("cg-insp-file", node.data("file") || "-");
          text("cg-insp-kind", (node.data("kind") || "-") + " / selected");
        });
        cy.on("tap", function(evt) { if (evt.target === cy) cy.elements().removeClass("cg-faded cg-pick"); });
      } else {
        cy.elements().remove();
        cy.add(model.cyElements);
      }
      cy.layout({name: "concentric", concentric: function(n) { return 100 - (Number(n.data("depth")) || 0) * 25; }, levelWidth: function() { return 1; }, minNodeSpacing: 14, spacingFactor: 1.05, animate: false}).run();
      cy.ready(function() { cy.fit(undefined, 24); });
      setTimeout(function() { if (cy) cy.fit(undefined, 24); }, 30);
    }

    function setBusy(active) {
      const button = document.getElementById("cg-run");
      if (button) {
        button.disabled = active;
        button.textContent = active ? "Run..." : "Run";
      }
      const status = document.getElementById("cg-status");
      if (status && active) status.innerHTML = chip("query", "running", "busy");
    }

    function showToast(message) {
      const toast = document.getElementById("cg-toast");
      if (!toast) return;
      toast.textContent = message || "";
      toast.dataset.show = message ? "true" : "false";
    }

    async function renderWorkbench(state) {
      currentState = state;
      const model = buildModel(state);
      const subgraph = portFor(state, "subgraph");
      const scorecard = portFor(state, "scorecard");
      const analystRows = portFor(state, "analyst_rows");
      const cochange = portFor(state, "cochange");
      const primaryScore = model.tables.scorecard[0] || {};
      const selectedNode = model.nodes.find(function(node) { return Number(node.depth ?? 0) === 0; }) || model.nodes[0] || {};
      const selectedLabel = primaryScore.qualified_name || primaryScore.entity_name || labelFor(selectedNode) || state.query.symbol || "No symbol selected";
      const selectedFile = primaryScore.file_path || selectedNode.file_path || state.query.file || "-";
      const edgeKinds = Array.from(new Set(model.edgeRows.map(function(row) { return String(row.edge_kind || row.relation || "edge"); }).filter(Boolean))).sort();
      const totalHotCallers = model.tables.blast.reduce(function(sum, row) { return sum + Number(row.hot_caller_count || 0); }, 0);
      const maxBlast = Math.max(0, ...model.tables.blast.map(function(row) { return Number(row.blast_radius_score || 0); }));

      text("cg-subtitle", "Perspective ${esc(PERSPECTIVE_VERSION)} / " + state.query.symbol + " / depth " + state.query.depth);
      html("cg-status", [subgraph, scorecard, analystRows, cochange].map(function(port) { return chip(port.name, portLine(port), statusClass(port)); }).join(""));
      text("cg-blast-line", portLine(analystRows));
      text("cg-scorecard-line", portLine(scorecard));
      text("cg-topology-line", fmtInt(model.nodes.length) + " nodes / " + fmtInt(model.edgeRows.length) + " edges");
      text("cg-node-line", fmtInt(model.tables.nodes.length));
      text("cg-edge-line", fmtInt(model.tables.edges.length) + " / " + fmtInt(model.tables.cochange.length));
      text("cg-insp-name", selectedLabel);
      text("cg-insp-file", selectedFile);
      text("cg-insp-kind", String(primaryScore.symbol_kind || selectedNode.symbol_kind || "-") + " / posture " + String(primaryScore.posture || "-"));
      html("cg-kpis", kpi("Rows", fmtInt(model.tables.blast.length), "green") + kpi("Hot callers", fmtInt(totalHotCallers), "amber") + kpi("Max blast", fmtNumber(maxBlast, 2), "blue") + kpi("Co-change", fmtInt(model.tables.cochange.length), "coral"));
      html("cg-score-grid", kpi("Pagerank", fmtNumber(primaryScore.pagerank, 5), "green") + kpi("In / Out", fmtInt(primaryScore.in_degree) + " / " + fmtInt(primaryScore.out_degree), "blue") + kpi("Callers", fmtInt(primaryScore.callers), "amber") + kpi("Churn 90d", fmtInt(primaryScore.churn_90d), "coral"));
      html("cg-legend", edgeKinds.length ? edgeKinds.map(function(kind) { return "<span>" + escapeHtml(kind) + "</span>"; }).join("") : "<span>" + escapeHtml(EMPTY_TEXT) + "</span>");
      showToast(state.query.error || "");

      renderGraph(model);
      await loadViewer("wb-perspective-blast", model.tables.blast, schemas.blast, {columns: ["entity_name", "caller_count", "hot_caller_count", "caller_churn_90d", "blast_radius_score", "file_path"], sort: [["blast_radius_score", "desc"]]});
      await loadViewer("wb-perspective-scorecard", model.tables.scorecard, schemas.scorecard, {columns: ["entity_name", "pagerank", "in_degree", "out_degree", "callers", "churn_90d", "posture", "file_path"]});
      await loadViewer("wb-perspective-nodes", model.tables.nodes, schemas.nodes, {columns: ["depth", "entity_name", "symbol_kind", "qualified_name", "file_path", "stable_symbol_id"], sort: [["depth", "asc"]]});
      await loadViewer("wb-perspective-edges", model.tables.edges.concat(model.tables.cochange), schemas.edges, {columns: ["depth", "edge_kind", "relation", "source_stable_id", "target_stable_id", "file_a", "file_b", "cochange_count", "has_static_edge"]});
    }

    function callAppTool(tool, args) {
      const id = "cg-tool-" + Date.now() + "-" + Math.random().toString(36).slice(2);
      return new Promise(function(resolve, reject) {
        const timer = window.setTimeout(function() {
          window.removeEventListener("message", onMessage);
          reject(new Error("app tool call timed out"));
        }, 30000);
        function onMessage(event) {
          const data = event.data;
          if (!data || data.type !== "jute-app-tool-result" || data.id !== id) return;
          window.clearTimeout(timer);
          window.removeEventListener("message", onMessage);
          if (data.ok) resolve(data.result);
          else reject(new Error(String(data.error || "app tool call failed")));
        }
        window.addEventListener("message", onMessage);
        window.parent.postMessage({type: "jute-app-tool-call", id, tool, args}, "*");
      });
    }

    const form = document.getElementById("cg-query-form");
    if (form) {
      const depth = form.elements.namedItem("depth");
      if (depth) depth.value = String(currentState.query?.depth ?? ${DEFAULT_DEPTH});
      form.addEventListener("submit", async function(event) {
        event.preventDefault();
        const symbolInput = form.elements.namedItem("symbol");
        const fileInput = form.elements.namedItem("file");
        const depthInput = form.elements.namedItem("depth");
        const symbol = String(symbolInput?.value || "").trim();
        const file = String(fileInput?.value || "").trim();
        const parsedDepth = Math.max(0, Math.min(3, Number(depthInput?.value ?? ${DEFAULT_DEPTH}) || ${DEFAULT_DEPTH}));
        if (!symbol) {
          showToast("symbol must not be empty");
          return;
        }
        setBusy(true);
        showToast("");
        try {
          const result = await callAppTool("wb_query", {symbol, file: file || null, depth: parsedDepth});
          await renderWorkbench(stateFromQueryResult(result, {symbol, file, depth: parsedDepth}));
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          await renderWorkbench({query: {symbol, file: file || symbol, depth: parsedDepth, status: "error", error: message}, ports: currentState.ports || []});
        } finally {
          setBusy(false);
        }
      });
    }

    await renderWorkbench(currentState);
  </script>
</section>`;

display.html(html)
