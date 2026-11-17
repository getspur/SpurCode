# U5 final root review

Approved after the documentation correction and actual notebook acceptance.
The release contract now states that native publication records schemas/counts,
while `read_result` checks contract metadata, manifest context and file integrity.

Root ran the attempt-2 receipt verifier at original commit `599f1e0a2`: exit 0.
It rechecked 325 native inputs, 157 prior-attempt files, 281 historical files,
the unchanged candidate and the original TDD ancestry. Normalized plan output
`e023ada61` has identical production and evidence bytes. The prior full release
results remain REMOTE 61 core + 4 adapter tests and HOST 223 Python tests,
zero skips; the prose correction did not require rebuilding or rerunning them.

Root created and ran a separate notebook through Notebook MCP:
`/Users/kevintruong/project/spur-notebook/2026-11-17-spur-graph-notebook-symbol-uplift-acceptance.ipynb`.
Its visible runtime selector was changed to DuckDB 2.0.0 after confirming the
selected tab. Seven SQL cells succeeded and saved durable outputs. They load
the verified c5f36a0f candidate, show 46/24 symbols and 17/11 functions, match
28 independent function anchors, check 70 original cell references and hashes,
return 50 symbol changes/10 cell events, verify no-ops, and demonstrate Rust
`:dep` extraction at original line 2 with explicit unsupported coverage.

[review.json](review.json) preserves exact MCP-read cell sources, outputs,
versions, execution provenance and limitations. The earlier artifact remains
unchanged. The stable local candidate copy is under
`.spur/artifacts/notebook-symbol-uplift-native/`.

This validates extension/consumer support for Explore and Review. Completing
their notebook app UI remains separate work. The artifact is local and unsigned;
the reader does not revalidate advertised schemas/counts.

## Final integration

All five approved tasks were integrated into the local working branch at
`108e395bafdcfed63f236c2de6b8aa14b49d676b`. Its production crates exactly match
the approved normalized U5 output. A clean checkout of that integrated commit
passes the source guard for all 325 recorded inputs and the same extractor ID.
The guard correctly rejects the main working tree's two pre-existing ACP edits;
those edits are preserved rather than included in the release candidate.
All five main and eleven external tracked user files remain byte-identical.

[integration.json](integration.json) records the source checks, preserved hashes,
closed execution/planning issues and fixed bugs. Original RED/implementation
histories remain reachable through `spur/notebook-uplift-original-tdd-u1` through
`u5`, alongside the Plan Engine's normalized integration history. No PR, push,
deployment or full app UI change was made.
