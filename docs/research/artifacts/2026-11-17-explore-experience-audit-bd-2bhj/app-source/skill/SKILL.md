---
name: code-graph-workbench
description: "Use when answering codebase questions inside the Code Graph Workbench app - call wb_* evidence tools before answering and cite pushed stable_symbol_ids."
---

# Code Graph Workbench - Evidence-Grounded Answers

<HARD-GATE>
Answer ONLY from evidence pushed this turn by MCP tools:
(`wb_ping`, `notebook_push_source`, `notebook_run_cell`).
Call the relevant wb_* evidence tool(s) BEFORE answering. Every claim in the
answer must cite a `stable_symbol_id` returned by a tool this turn. If no tool
was called, say "no evidence pushed this turn" instead of answering.
</HARD-GATE>

## The loop

1. Resolve what the user is asking about (symbol, file, or change).
2. Call the matching evidence tool - it runs the real analyst/graph query,
   pushes Arrow to the bound panel port, and returns the pushed
   `stable_symbol_id`s for your citations.
3. Answer compactly; map citation markers `[n1]`, `[n2]` to the returned ids.
4. Never fabricate counts, scores, or edges: if the tool returned an empty or
   guided-empty result, report that honestly.

(Evidence tools `wb_blast_radius`, `wb_subgraph`, `wb_scorecard`, `wb_cochange`
arrive in the follow-up epic; until then `wb_ping` verifies the surface.)
