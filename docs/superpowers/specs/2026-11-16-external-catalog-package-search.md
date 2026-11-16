# External catalog package-name search

User approved implementation with TDD and deployment on 2026-11-16; tracked by
bd-lah8 / bd-2v9g.

`external_catalog` keeps its existing argument shape. When `package` is omitted,
`name_filter` discovers indexed packages within the selected source. It matches
case-insensitive substrings and ANDs alphanumeric keywords split on punctuation
or whitespace, regardless of keyword order. A keyword may also match the full
name or a package-name component within one insertion, deletion, substitution,
or adjacent transposition. Fuzzy matching requires at least three Unicode
characters in the keyword. Empty or whitespace-only filters list packages;
nonempty punctuation-only filters match none. Exact substring matching remains
available for punctuation-bearing package names.

The catalog contains names and coordinates, not description or tag metadata.
This is package-name discovery rather than semantic topic search. Supplying
`package` continues exact coordinate browsing; file-symbol filtering is unchanged.
Results retain lexical source/name ordering, keyset pagination, source isolation,
and catalog generation. `total_matches` counts the complete filtered set before
the cursor and page limit.

Both the SQL and Parquet backends use one shared Rust matcher. The SQL path
groups package metadata within the selected source, applies that matcher, then
applies cursor/limit. No package graph or source content is searched for keywords.

The solver catalog has no matching string-search policy rule. A validated generic
Z3 request returned `sat` with `max_edits=1` and `min_fuzzy_chars=3`: accept one
edit, reject two edits, disable fuzzy matching at lengths one/two, and accept a
three-character typo (`axm` → `axum`). Artifact persistence was full, so the exact
request/result is recorded on bd-lah8 and execution used `persist=false`.

Tests must observe RED before production edits, then establish matching parity,
Unicode safety, pagination/source isolation, and exact package descent. Deploy
the reviewed Code Lambda artifact and verify live requests separately.
