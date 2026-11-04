# G1b evidence

Assigned issue `bd-leg`, delegation `a1d19e5860c7411e`; binding audit
2127. Test-first commit `3fbc9ffc6`, additional test coverage `82527b074`,
implementation `97ad06620`. The latter includes the two extra tuple entries
in the G1a fixture's expected parity list. The six corpus golden edits each
change exactly one `macro_invocations.extractor_version` value; no blessing
was used.

Fresh PRE `sol_21a290c32481482b`: catalog `data_integrity.unique` and
`cardinality` pass for the six expected owner/ordinal pairs, persisted and
reloaded in `pre.json`. This is a finite input contract, not corpus proof.
`red.log.gz` records exit 101 from
`SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_tuple_fields`:
the test compiled and asserted an empty field list against six expected rows.
The first implementation build separately failed with E0308 before a cast was
fixed; that compile failure was not the feature RED.

The current compiled extractor produced `actual-exports.json` through
`SPUR_G1B_EXPORT_SMALL=<absolute-path> SPUR_REMOTE=0 scripts/spur-cargo test
-p spur-graph --test rust_tuple_fields
physical_entries_include_owned_metadata_and_match_byte_extraction` (exit 0).
It contains the exact source, graph symbols/edges and byte-extractor rows.
The original relative export path failed (exit 101, missing directory) because
the test runs from the crate directory; the absolute-path rerun succeeded.

POST uses exact exported start/owner/ordinal tuples and independent source
positions. The 64-variable solver limit required disjoint struct and variant
projections; attempted combined requests and their validation errors are
retained in `post.json`. `sol_a421032eade54477` and
`sol_1c6902555bb24151` are SAT/pass for those projections.
`sol_05f1d25b5da04d4c` rejects a wrong existing struct owner;
`sol_ec1fe138ddec4c6f` rejects moving a field to a different existing
variant while uniqueness and cardinality still pass; and
`sol_8d4e567a608c400d` rejects a missing row. All were persisted, reloaded
and pinned; raw requests, results and reloads are in `post*.json`.
These finite checks certify only the encoded fixture, not all Rust programs.

The following commands exited 0, with complete outputs in corresponding
compressed logs:

- `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_tuple_fields`: 3 passed, 1 explicitly ignored export harness.
- `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_declaration_ownership --test rust_macro_token_tree_query --test rust_macro_invocations --test bytes_extractor`: all passed.
- `SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test extractor`: 54 passed, 3 ignored. Its initial unblessed run failed only on the six version fields.
- `SPUR_REMOTE=0 scripts/spur-cargo check -p spur-graph --all-targets`: passed with pre-existing warnings.
- `SPUR_REMOTE=0 scripts/spur-cargo fmt --all -- --check` and `git diff --check`: passed.

SHA-256 bindings:

| Artifact | SHA-256 |
| --- | --- |
| Frozen inputs gzip | `637303918ca2b90b3fe94e6fbc1bfbed53fbec5b6a7e8567a73c8fa9a98478d5` |
| Structural candidates | `a1fbd521529c3129701c0f9c8730e22ab16de1fb62b87a04208c52c975a6886d` |
| Independent syn rows | `676609411cc105896274c38b1d5fa687a4475d37bd21fd35697dbdf14817e66c` |
| Current languages.rs | `612cf8ba3d2b160d654b8683bbb97500e9c3ffbd0cd398b926a13b0edc5d0c19` |
| Current rust edge query | `ba27698caedefaea96eaa43ab61023d7a6f4108bf0468f6bfcc2f7bd00c9f031` |
| Focused actual export | `8daeae17dd941c7630405324886a8b79cdcd274cd734d3676b63bf51558ee319` |
| Frozen graph export gzip | `dad5d4e8a3ce81bdf1d4c9674235708712ccc86645b42964dfa962fbf42b3f0c` |
| Matched rows | `39c18b6606ddcb9edcbdcf8f24c8b413d772201bd20bce0d0a2f03ccb29ed862` |

The syn inventory has 453 tuple rows: seven belong to a separate held-out
fixture and 446 belong to the 136 frozen corpus files. All 446 candidate keys
match independent syn rows exactly on path, source OID, kind and full span;
owner and ordinal agree. Their type ranges reproduce exact source bytes; 61
full starts precede type starts (49 with attributes, 12 with visibility).
Full entry policy includes owned outer attributes (including outer doc comments)
and visibility, ends at the type end, and excludes commas and neighboring
ordinary comments. Type ranges are
independent syn evidence, not newly persisted graph schema. Physical ordinals
do not assert configured compiler field numbers. IDs retain path/FQN/kind/start
semantics: the test records that a new type at the same ordinal/start retains
the ID, while a changed offset or ordinal changes it.

## Frozen rebuild and join

`prepare_frozen.py` creates 136 isolated roots outside Git, checks every
source against its saved Git blob OID, and keeps the original relative path.
With that root in `SPUR_G1B_ROOT` and an absolute output path in
`SPUR_G1B_EXPORT`, the following current-code build exited 0 after 563.25s:

```text
SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_tuple_fields export_frozen_graph -- --ignored
python3 docs/research/artifacts/2026-11-02-rust-gap-implementation/G1b/check_frozen.py docs/research/artifacts/2026-11-02-rust-gap-implementation/G1b/frozen-graph.json.gz /private/tmp/g1b-rechecked.json
```

The second command exited 0: 446/446 exact matches, including 340 variant
and 106 struct fields, with no missing or duplicate full spans. The compressed
graph export is the actual output of those builds; `frozen-matches.json`
records the matched source OIDs, spans, graph field and owner IDs, ordinals,
names, and syn type ranges. The first join (exit 1, preserved in
`frozen-join-initial.log.gz`) required syn and graph owner starts to agree;
229 failed because syn includes preceding owner doc attributes while the
existing graph owner starts at the declaration. Every one of those 229 had the
same exact frozen candidate owner ID and end byte. The final join uses that
exact source-bound owner ID, owner kind, name and end; it does not join by
overlap or fuzzy name. Field ranges themselves remain exact syn full ranges.

The first combined-root build was interrupted (exit 130) because it was slow;
the successful run uses one frozen file per root. A process sample of that
successful run showed CPU time in existing macro-evidence validation and
repeated source hashing. A later attempt to optimize the hash dependency was
interrupted (exit 130) once the original run completed; it supplied no evidence.

## Retry 3: outer doc comments

Actual issue `bd-leg`, live delegation `6cf52a5d4cef4757`, worker-started
audit 2142. Overlay baseline commit `236a4f038` preserves reviewed attempt 2;
test commits `69f635e2c` and `faf2dc689` precede fix `9e2a20131`. This
worker leaves the issue in progress for review. The branch components identify
the brain session and worker session, not the delegation. The plan includes
the authorized G1a/G1b/G1c Beads mapping.

Fresh catalog PRE `sol_eb8f5618dc9045bc` is SAT/pass and reloaded/pinned;
`retry-pre.json` contains the exact request, result and reload. It verifies
the original finite six owner/ordinal contract, not comment parsing or all
possible spans.

`SPUR_REMOTE=0 scripts/spur-cargo test -p spur-graph --test rust_tuple_fields
outer_doc_comments -- --nocapture` exited 101 twice, after successful
compilation. The first assertion RED shows the omitted line doc in both graph
and byte extraction (`retry-red-line.log.gz`); the expanded RED shows omitted
line, block and variant docs in both paths while ordinary comment controls
match (`retry-red.log.gz`). These are assertion failures, not compile failures.

The minimal fix checks the tree-sitter comment node's `outer` field, in
addition to explicit attributes and visibility; ordinary comment nodes do
not have that marker. This adjusts the unapproved, already bumped G1b
extractor version rather than introducing another version: schema remains 11.
`SPUR_G1B_EXPORT_DOCS=/private/tmp/g1b-retry-actual.json SPUR_REMOTE=0
scripts/spur-cargo test -p spur-graph --test rust_tuple_fields` exited 0:
5 passed, 1 ignored export harness. `retry-actual-exports.json` contains the
source and actual graph/byte symbols and graph edges from that compiled run.
`retry-rows.py` checks exact owner edges and graph/byte parity and constructs
independent expected full spans and source type spans; type spans are fixture
source evidence, not a new graph export or persisted schema. `retry-rows.json`
records all seven actual/expected rows, IDs, owner, ordinal and type ranges.
An additional global uniqueness/exact-range Python assertion passed for all
seven rows.

POST `retry-post.json` includes exact requests, responses and reloaded pinned
receipts. Split projections fit the solver's finite-variable limit:
`sol_440df8edd7554748`, `sol_ea4a551903ea410e`,
`sol_2181af48f9fe49c7`, and `sol_9631e97b494a4e70` are SAT/pass.
`sol_5a5cb01c48e541f2` (omitted outer doc), `sol_e3435fb585384aaa`
(included ordinary comment), and `sol_da5359eeec8e4b41` (wrong existing
variant owner) are UNSAT/fail on tuple consistency while uniqueness and
cardinality pass. These are finite fixture checks, not universal parser proofs.
The first projection for the last repeated `u8` failed because the evidence
script used the first textual occurrence, not the occurrence inside `E::B`;
`retry-post-initial.json` retains it. After correcting the expected offset,
a second failed receipt (`retry-post-index-correction.json`) retained the
old type-offset cell; the final valid projection updates both values.

`python3 .../G1b/check_frozen.py .../G1b/frozen-graph.json.gz
/private/tmp/g1b-retry-frozen-matches.json` exited 0 with 446/446 exact
source-bound joins (340 variant and 106 struct). This is the previously
compiled frozen export, **not** a rebuild after the fix. To establish whether
the new branch affects it, `retry-scan-docs.c` was compiled against the locked
tree-sitter 0.26.13 and tree-sitter-rust 0.24.2 static libraries from the
local build, then `python3 .../G1b/retry-scan-docs.py
/private/tmp/g1b-scan-docs` checked all 136 frozen source Git blob IDs and
reported 446 direct type nodes and zero grammar-marked outer docs anywhere in
their tuple declaration lists (exit 0). Thus no frozen row exercises the new
branch; the prior exact join remains applicable, without claiming a rebuild.
The scan source SHA-256 is
`bf8edf2db02ad34de2cfde5ddd08bd973d3e7d865dcb537b57792f9cac87aed6`.

Other fresh commands exited 0: `SPUR_REMOTE=0 scripts/spur-cargo test -p
spur-graph --test rust_declaration_ownership --test rust_macro_token_tree_query
--test rust_macro_invocations --test bytes_extractor --test extractor`
(unblessed extractor 54 passed/3 ignored), `SPUR_REMOTE=0 scripts/spur-cargo
check -p spur-graph --all-targets` (existing warnings), `scripts/spur-cargo fmt
--all -- --check`, and `git diff --check de23e80e1`. Logs are in
`retry-affected.log.gz` and `retry-check.log.gz`.

Retry source bindings (SHA-256): `languages.rs`
`3eb9f2f6f68919706582fc5b9e6532a741abb37467152f520f16b90997f7f131`;
query `ba27698caedefaea96eaa43ab61023d7a6f4108bf0468f6bfcc2f7bd00c9f031`;
regression test `db399c08314fb2440a3e4415f331effc4dd85a847cd3d5eb6eeb14edb33ce33d`;
compiled fixture export `a4e6c8e835e3a88da1d0cd2b05fadefe3ecdbb838e4ec4c26707097f30446ec2`;
source-bound rows `72a781fd8103c42700929e7689695c22decc69383798ebbecc248154e1814d84`.
