import { callTool as wbExploreCallTool, display as wbExploreDisplay } from "@getspur/spurlab-sdk";

{
  const target = {
    symbol: "notebook_app_doctor",
    file: "src/mcp/tools/notebook_app_doctor.rs",
  };
  const depth = 2;
  const edgeKinds = ["calls", "references_other", "references_hof", "calls_dyn"];

  function toolErrorText(result: unknown): string | null {
    if (!result || typeof result !== "object") return null;
    const text = (result as { text?: unknown }).text;
    if (typeof text !== "string") return null;
    return text.startsWith("Error executing tool") ? text : null;
  }

  async function safeEvidenceCall(name: string, args: Record<string, unknown>): Promise<Record<string, unknown>> {
    try {
      const result = await wbExploreCallTool(name, args);
      const textError = toolErrorText(result);
      if (textError) return { name, ok: false, error: textError };
      return { name, ok: true, result };
    } catch (error) {
      return {
        name,
        ok: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }

  const calls: Array<Promise<Record<string, unknown>>> = [];
  if (target.symbol.trim()) {
    calls.push(safeEvidenceCall("wb_scorecard", { symbol: target.symbol }));
    calls.push(safeEvidenceCall("wb_subgraph", { symbol: target.symbol, depth, edge_kinds: edgeKinds }));
    calls.push(safeEvidenceCall("wb_blast_radius", { symbol: target.symbol }));
  }
  if (target.file.trim()) {
    calls.push(safeEvidenceCall("wb_cochange", { file: target.file }));
  }

  const results = await Promise.all(calls);
  const failed = results.filter((item) => !item.ok);
  const status = failed.length ? "warn" : "ok";

  const escapeWorkbenchValue = (value: unknown) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch] ?? ch));

  const html = `<div data-wb-source-stub="explore" style="display:none" data-status="${escapeWorkbenchValue(status)}">${escapeWorkbenchValue(JSON.stringify(results))}</div>`;
  wbExploreDisplay.html(html)
}
