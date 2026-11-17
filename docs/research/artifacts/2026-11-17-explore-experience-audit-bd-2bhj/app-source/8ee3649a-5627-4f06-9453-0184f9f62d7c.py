# --- Query (the input) ---------------------------------------------------------
# Set TARGET_SYMBOL (and optionally TARGET_FILE) below, then RUN this cell to load
# that symbol into the workbench, then run the "Code Graph Workbench" cell below.
# Suggestions (top symbols by blast radius) are printed underneath to type from.
import os, sys


def _find_up(rel: str) -> str | None:
    d = os.getcwd()
    while True:
        cand = os.path.join(d, rel)
        if os.path.exists(cand):
            return cand
        parent = os.path.dirname(d)
        if parent == d:
            return None
        d = parent


for _p in (os.path.join(os.getcwd(), "server"), _find_up("sdk/python/src")):
    if _p and _p not in sys.path:
        sys.path.insert(0, _p)

import analyst
from tools.subgraph import _subgraph_rows

# ── INPUT ──────────────────────────────────────────────────────────────────────
TARGET_SYMBOL = "PortStore"
TARGET_FILE = "src/dag/ports.rs"   # drives the co-change panel
# ────────────────────────────────────────────────────────────────────────────────

EDGE_KINDS = ["calls", "references_other", "references_hof", "calls_dyn"]
spur.put("subgraph", _subgraph_rows(TARGET_SYMBOL, depth=2, edge_kinds=EDGE_KINDS))
spur.put("scorecard", analyst.scorecard_rows(TARGET_SYMBOL))
spur.put("analyst_rows", analyst.blast_radius_rows(TARGET_SYMBOL))
spur.put("cochange", analyst.cochange_rows(TARGET_FILE))
print(f"Loaded {TARGET_SYMBOL!r} / {TARGET_FILE!r} — run the workbench cell below to view.\n")

# Suggestions — type one of these into TARGET_SYMBOL above and re-run.
con = analyst.open_analyst()
if con is not None:
    rows = con.execute(
        """
        SELECT entity_name, symbol_kind, file_path
        FROM v_symbol_scorecard
        WHERE entity_name IS NOT NULL AND entity_name <> ''
        ORDER BY blast_radius_score DESC NULLS LAST, pagerank DESC NULLS LAST
        LIMIT 20
        """
    ).fetchall()
    con.close()
    print("Suggestions (top by blast radius):")
    for name, kind, path in rows:
        print(f"  • {str(name):<30} {str(kind):<14} {path}")
