# U2: typed notebook symbols and source changes

Task bd-1khk / U2, isolated worker base `26ce5a162ba844f5b13433c069265fb56f7f55ba`.
Rationale: compare unique symbols inside paired notebook cells while retaining
original-source provenance and independent structural/configuration events.

Only spur-graph extraction/matching and task-owned tests/evidence changed.
No SQL crate, ordinary schema, legacy notebook graph identity, retained native
binary, prior evidence, external notebook, cloud resource, issue closure or
merge is part of this delivery. The root reviews this gate before U3.

## Public interface for U3

Exported module: `spur_graph::extract::notebook_symbols`.

```rust
pub fn extract_notebook_symbols(
    path: &str,
    bytes: &[u8],
) -> anyhow::Result<NotebookSymbolDocument>;

pub fn compare_notebook_sources(
    path: &str,
    old_path: Option<&str>,
    change_kind: &str,
    left_source: Option<&[u8]>,
    right_source: Option<&[u8]>,
) -> anyhow::Result<NotebookComparison>;
```

`public-interface.rs.txt` contains the exact public structures/enums, plus
shared U1 `NormalizedCell`, preparation/diagnostic types, `Language` and native
`ExtractedSymbol` fields. It is a schema transcription, not another module.

| Public result | Fields and interpretation |
| --- | --- |
| `NotebookSymbolDocument` | `contract_version=1`, exact input `path`, SHA256 of exact input bytes, opaque root metadata, all cells, aggregate status, typed diagnostics |
| `NotebookCellSymbols` | full shared `NormalizedCell` in `cell`, plus `symbols`; retains ID/identity quality, ordinal/type, resolved language, metadata, original/analysis source and hashes, preparation policy, directives/coordinate map, status and normalization diagnostics |
| `NotebookSymbol` | canonical `id`, full native `ExtractedSymbol`, original `source_ref` |
| `SourceRef` | notebook path/hash, cell ID/ordinal/identity quality/source hash/language, `DecodedCodeCellUtf8`, byte and line ranges |
| `NotebookComparison` | `contract_version=1`, status/typed diagnostics, complete optional before/after documents, independent symbol/cell/document change lists |
| `NotebookSymbolChange` | Added/Modified/Deleted/Renamed; optional full before/after symbols, each with its own original reference |
| `NotebookCellChange` | Added/Deleted/Moved/TypeChanged/LanguageChanged/SourceChanged/MetadataChanged/DependencyDirectivesChanged, optional before/after `CellRef` |
| `NotebookDocumentChange` | Added/Deleted/Renamed/MetadataChanged, optional before/after paths and metadata |
| `SymbolDiagnostic` | typed code, optional Before/After side, cell ordinal, original byte range, message; shared normalization codes remain typed via `Normalization(...)` |

Byte ranges are half-open in original decoded cell UTF-8. Line ranges retain
the native one-based start/end row convention. The external coordinate-space
spelling is `decoded_code_cell_utf8`. Hash strings are computed lowercase hex
SHA256, without a caller label. No snapshot/repository assertion is included;
the consumer must validate source bytes before adding its snapshot label.
Extra symbol ambiguity diagnostics are document diagnostics with cell ordinals;
the cell's status is Partial. Preserve both levels of diagnostics in U3.

The new ID consists of a percent-escaped notebook path, `#cell:`, the escaped
cell ID, then `#symbol:` and a JSON tuple containing language label, name, kind,
optional scope, byte start and an optional ordinal discriminator for ambiguous
cell IDs. Percent, delimiters and non-ASCII bytes are escaped; the allowlist is
ASCII alphanumeric plus `-_.~/`. The JSON tuple keeps scope/name boundaries
unambiguous. Original path and cell ID remain unchanged in typed fields.
This is an occurrence identity, not a promise of stable identity across source
edits or runtime equivalence. Offset movement can update IDs and current refs
without being reported as a symbol edit. Legacy graph stable keys are unchanged.

M requires both sides. A requires absent left/present right; D present
left/absent right. R and R100 require both sides and a distinct valid old_path.
Other kinds, including copies and scored renames other than R100, return
Partial with `UnsupportedChangeKind` and no inferred changes. Other kinds may
not supply a differing old_path. Missing/unexpected sides and malformed source
documents return typed Invalid results with diagnostics and available inventory.
Paths must be nonblank and contain no NUL. Extractor/setup/internal failures
propagate through `Err`; they are not downgraded to row coverage diagnostics.

Non-code, unknown-language, empty and imports-only cells stay in inventory.
Unknown language and unsupported notebook preprocessing remain Partial.
Duplicate effective cell IDs remain retained and explicitly Partial on this
symbol route (other malformed shapes stay Invalid). Their IDs use an ordinal
discriminator; matching is skipped with an ambiguity diagnostic. Repeated
anonymous sources also skip pairing. Same-cell duplicate symbol matching keys
retain all symbols, mark Partial, and skip symbol matching while keeping cell
events. Source-derived anonymous edits conservatively add/delete cells/symbols.

## Matching and compatibility

The ordinary wrapper retains its existing language inference and offset-sensitive
behavior. Its implementation delegates to a crate-private explicit-language
inner matcher; only notebook callers prefilter paired identical ORIGINAL symbol
slices. Pairing uses same cell ID, same normalized language and unique
(name, kind, scope); it never flattens cells. Type/language changes remove/add
symbols conservatively. An original byte difference continues through the
existing syntax/token matcher; none of these results claims semantic equivalence.

Full before/after symbols are retained in maps keyed by exact pre-match
`SnapshotKey` values. Direct before-pair keys are captured before matching;
rename results resolve the matcher's exact previous SnapshotKey. No before ref
is reconstructed by guessing a name or position after matching. Snapshot-hash
collisions are diagnosed as Partial before entering the matcher. Public IDs
use the structured preimage above, not that finite internal hash.

Rust preparation is the shared U1 `evcxr-leading-dep-mask-v1` policy. This code
neither executes directives nor reads targets. Cell source, metadata and
dependency events are independent of symbol changes. Output/execution-count
changes alone create no tested code/structure changes. Saved profiles are opaque
metadata, not verified runtime compatibility.

## TDD and exact receipts

`6eb1c4cf6` commits the test-only contract scaffold and regressions before the
production implementation. The RED scaffold is compiled only under cfg(test):
raw notebook extraction calls the existing intentionally-empty notebook symbol
API, and Rust comparisons exercise the existing ordinary matcher. It avoids an
unresolved-import/compiler-error RED. The implementation removes that adapter,
exports the same contract and leaves the test predicates byte-identical.

Every `.json` command receipt includes argv, environment overrides, UTC times,
source commit, exact source/fixture SHA256s, exit and raw-log SHA256.
`capture.py` never overwrites a receipt. `verify_receipts.py` checks source and
test identity, original U1 bytes, solver outcomes and reverse-application of
`source.patch`. The patch is the unified Rust source/test diff against the
dispatched base; it excludes generated receipts.

Remote command, with local fallback explicitly disabled:

```bash
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --no-default-features --lib extract::notebook_symbols
```

`remote-red.json/.log`: exit 200 before compilation. In this sandbox the AWS
endpoint connection failed for both wrapper routes. The root's prior independent
read-only check reported a healthy running instance with SSM ConnectionLost;
neither this receipt nor the prior failures constitute a remote compiler/test
result. The authorized host recovery was deliberate, not a retry of a remote
compiler/assertion failure. No cloud-state change command was issued directly.

All host Rust receipts use this exact environment and the same wrapper:

```bash
SPUR_REMOTE=0 SPUR_NO_LOCAL_FALLBACK=1 \
CARGO_TARGET_DIR=/private/tmp/spur-notebook-u1-bd-bi1d-target \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 SPUR_SCCACHE_S3=0 \
TMPDIR=/private/tmp/spur-notebook-u1-bd-bi1d-tmp \
scripts/spur-cargo test -p spur-graph --no-default-features --lib FILTER
```

The focused run reuses the existing U1 host test cache; the retained native
release target/package is untouched. These are HOST results, not remote success.

| Receipt stem | Filter / extra arguments | Exit | Observed result |
| --- | --- | --- | --- |
| `host-red` | `extract::notebook_symbols` | 101 | 15 failed; frozen base 0 instead of 46; dependency version/header/comment prefix movement explicitly yields false Modified |
| `host-green-initial` | identical to RED | 0 | 15 passed, 0 ignored; 46/24 symbols, 17/11 functions and 28 anchors |
| `host-extract-green` | `extract::` | 0 | 136 passed, 0 ignored |
| `host-matcher-green` | `git_walk::tests::` | 0 | 48 passed, 0 ignored; ordinary offset-shift remains Modified |
| `host-real-evidence` | `extract::notebook_symbols::tests::frozen_app_symbols_and_all_28_independent_function_anchors -- --nocapture` | 0 | frozen refs/anchors pass; 50 real changes and 10 cell events |
| `host-canonical-mutation-red` | `extract::notebook_symbols::tests::canonical_ids_separate_delimiter_and_percent_witnesses` | 101 | replacing only the encoding with raw concatenation yields the exact equal-ID witness, after extraction succeeds |
| `host-canonical-green` | identical mutation-test command | 0 | restored encoding separates delimiter/percent/Unicode witnesses |

The new production code adds no warnings. Host lib tests report two existing
dead-code warnings in `src/mcp/mod.rs`; the RED scaffold/mutation additionally
has expected unused test-only declarations. Local `scripts/spur-cargo fmt -p
spur-graph -- --check` and Rust source `git diff --check` both exit 0.

The real input file SHA256 is
`bc0e5f23ba34c20149d5c8168f4a3a1fa8b3af3c8759adaef418f2343686b608`.
The independent 28-anchor input SHA256 is
`93623d013f4767ac621853e7855aa84a700b73d1814f84c6b3c02a555928fd83`.
All 70 symbol refs and every emitted before/after change ref resolve against
their correct frozen decoded source, cell and original byte/line coordinates.

`offset-only-real-changes.json` explains the intentional 55-to-50 difference:
PERSPECTIVE_VERSION, EvidencePort, esc, normalizeValue and tableRows have exactly
equal original slices at different offsets. It retains both ranges, original
slice/hash and source evidence hashes. Ten separate cell events remain.

## Solver scope and remaining limits

Catalog-first PRE and POST use implemented data_integrity.unique and
data_integrity.foreign_key rules, with persist:false because persistence quota
is full. Exact requests and raw results are retained in solver-pre.json and
solver-post.json. Flat helper keys/raw delimiter IDs fail; cell-qualified helper
keys and the escaped public-ID witness pass. An exact before SourceRef tuple
passes; mixing an old document hash with a new cell hash fails. POST checks the
landed encoded public-ID witness. Cached solver results are labelled by the raw
solver envelope; no persisted solve ID is claimed.

These are finite two-helper/two-document/witness snapshots, not proofs of
arbitrary notebook parsing, unbounded ID behavior, execution, or runtime
dependency resolution. Rust preparation retains U1's bounded syntax support.
Rename inference retains the ordinary matcher's bounded token heuristics;
ambiguity is Partial and never a claim of semantic identity. Anonymous source
edits and repeated sources have explicit identity limits.

This U2 gate does not implement/validate SQL nested vectors, consumer cache or
publication migration, DuckDB2 native loading, or frontend navigation. Those
belong to U3-U5. No complete workspace test-suite or remote test success is
claimed. Source-hash validation and caller snapshot attachment remain required
in the consumer.
