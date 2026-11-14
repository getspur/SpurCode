# Typed temporal pair contract

`spur_graph_symbol_changes` matches one ordered pair of extracted symbol lists.
It calls `spur_graph::git_walk::symbol_changes_from_extracted`; it does not load
Git objects, parse source, or implement another rename matcher.

```sql
spur_graph_symbol_changes(
    path VARCHAR,
    old_path VARCHAR,
    kind VARCHAR,
    old_symbols SYMBOL[],
    new_symbols SYMBOL[]
) -> STRUCT(
    changes STRUCT(
        symbol SYMBOL,
        stable_symbol_id VARCHAR,
        file_path VARCHAR,
        change_kind VARCHAR,
        previous_stable_symbol_id VARCHAR
    )[],
    diagnostics VARCHAR[]
)
```

`SYMBOL` means exactly the same ordered struct returned by `spur_graph_symbols`:

```sql
STRUCT(
    entity_name VARCHAR,
    symbol_kind VARCHAR,
    enclosing_scope VARCHAR,
    byte_start UBIGINT,
    byte_end UBIGINT,
    line_start UBIGINT,
    line_end UBIGINT,
    anchor_hash VARCHAR,
    tokens VARCHAR[]
)
```

`kind` is `A`, `M`, `D`, or `R`. `T`, `C`, and other values are rejected.
For non-renames, `old_path` must equal `path`; an empty `old_path` is also
accepted for `A`. `R` uses the supplied `old_path` for predecessor identities.
Use empty old symbols for additions and empty new symbols for deletions.
Both lists must have been parsed using the language of the **current** `path`,
including when a rename changes the extension.

Any NULL top-level argument produces NULL. Non-NULL empty lists produce valid
empty results. A symbol list element, required symbol field, token list, or
token element cannot be NULL. `enclosing_scope` may be NULL. Unsupported current
path languages return empty changes and diagnostics, following the shared
matcher. All ambiguity diagnostics from that matcher are returned.

`change_kind` is one of `added`, `modified`, `deleted`, or
`renamed_from_symbol`. Only `renamed_from_symbol` has a non-NULL
`previous_stable_symbol_id`. Deleted changes carry the old symbol payload;
renames carry the new payload and the old path's predecessor identity.
The function preserves input order, last-left duplicate identity lookup, and
the shared matcher's input-order-sensitive rename decisions. Remaining deleted
candidates retain the matcher's existing HashMap iteration behavior.

```sql
WITH parsed AS (
    SELECT
        spur_graph_symbols('old.rs', 'pub fn before(x: u32) -> u32 { x + 1 }') old_symbols,
        spur_graph_symbols('new.rs', 'pub fn after(x: u32) -> u32 { x + 1 }') new_symbols
)
SELECT spur_graph_symbol_changes('new.rs', 'old.rs', 'R', old_symbols, new_symbols)
FROM parsed;
```

## Pair caching and commit projection

The native call uses the internal SHA `__pair__` and no parent. Neither appears
in its result. A pair-cache key must include current path, old path, kind, and
the identities of both **ordered** parsed lists. Parse identity includes the
current path's language and extractor revision as well as the source blob.
Matching decisions can be reused across commits and parents because those
lineage values are projected separately.

When materializing a pair for an actual commit, build the changed snapshot key
from `(stable_symbol_id, current_sha)`. Build a rename predecessor key from
`(previous_stable_symbol_id, current_sha)` as well. Preserve the actual parent
SHA separately on each change. Do not substitute the parent SHA into either
snapshot key and do not persist `__pair__` as commit lineage.

The result contains pair-local candidates. After projecting actual commit and
parent lineage, apply the shared `merge_symbol_changes` fold across the whole
commit, keyed by `(parent_sha, SnapshotKey)`. The first snapshot payload wins;
conflicting change kinds become `Modified`. Keep comparisons against every
merge parent separate.
