# U1 notebook normalization — bd-bi1d

Scope: the shared normalized model and graph adapter in `spur-graph` only.
Specification: `docs/superpowers/specs/2026-11-17-notebook-cell-symbol-uplift.md`.
PoC: `docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/README.md`.
Base: `52e14c9cde58f0650bacb72daa04b705fd3c17da`; worktree initially clean.

The following intended implementation and remote receipts describe historical
attempt 1 (`ef32eb5b4`), retained unchanged. Attempt 2 implementation and host
verification are recorded below.

## Intended implementation

1. Commit compileable public contract scaffolding and embedded regression tests.
2. Observe meaningful remote RED, then implement lossless source decoding,
   existing token/identity rules, typed diagnostics, metadata and hashes.
3. Prepare notebook Rust by lexically recognizing leading single-line `:dep`
   headers, validating the dependency assignment as TOML, and masking bytes.
   Retain original source as the authority; parse only analysis bytes.
4. Adapt graph extraction without modifying `GraphFacts` or ordinary-file APIs.
5. Run focused remote GREEN, retain exact receipts, commit implementation,
   and report durable evidence without merging or closing the issue.

The normalization scaffold in the test commit intentionally returns no cells;
it allows regression tests to fail through assertions rather than missing APIs.

## Validation receipts

Command: `SPUR_NO_LOCAL_FALLBACK=1 scripts/spur-cargo test -p spur-graph --no-default-features --lib extract::notebook`.

- `red.json` / `red.log`: sandbox attempt, exit 200 before compilation because
  AWS endpoints were unreachable. This is not a RED assertion witness.
- `red-network.json` / `red-network.log`: network-enabled attempt, exit 200
  before compilation. Both cloud aliases selected the configured Malaysia
  builder; SSM key authorization was Undeliverable. This is not RED.
- `solver.json`: exact catalog rule specification and PRE requests/results,
  `persist:false`; distinct complete effective keys SAT/pass, duplicates
  UNSAT/fail. Finite supplied witnesses only, not parser correctness.

No local test fallback was used. Formatting uses local `scripts/spur-cargo fmt`.
Neither prior PoC evidence nor its retained binary is modified.

## Interface contract

`pub fn normalize_notebook(bytes: &[u8]) -> NormalizedNotebook`

`pub fn NormalizedCell::identity_path(&self, notebook_path: &str) -> String`

Public types: `NormalizedNotebook`, `NormalizedCell`, `PreparedSource`,
`DependencyDirective`, `NotebookStatus`, `CellIdentityQuality`,
`NotebookDiagnostic`, `NotebookDiagnosticCode`, `SourceCoordinateMap`.

The typed result preserves invalid cell inventory, original/analysis source
and SHA256 hashes, dependency text/ranges, root/cell metadata, ordinal/type,
language, identity quality, status and diagnostics. Missing/invalid source is
distinct from a valid empty source. Analysis offsets refer to the original
decoded cell under an explicit identity coordinate map.

## Bounds

U1 does not implement U2 symbols/comparison, SQL schemas, packaging, native
release replay, runtime dependency resolution or frontend integration. It must
never execute commands or open `:load` targets. Legacy graph facts cannot
represent notebook coverage; downstream callers must inspect normalization
diagnostics. Anonymous identities remain source-derived and order-sensitive
for repeated identical sources. Saved kernel profiles are opaque metadata.

## Attempt 2: implemented and verified on the host

Rationale: one normalized inventory and preparation policy gives graph and U2
the same original-source identities and coordinates, while typed diagnostics
expose coverage that legacy GraphFacts cannot represent.

The pre-applied attempt-1 diff was recommitted as `5b5f8d1cd`; latest task audit
fixtures were committed as `9253e9940` before any production implementation.
`0363da1dd` retains meaningful host RED; `0e61c92d8` implements normalization
and graph adaptation after that RED. A subsequent review regression in
`b30f19f2e` covers absolute Rust paths and inline command boundaries, followed
by the separately verified fix `bd53e6f93`. No issue was closed and no branch was merged.

`source.patch` is the unified source/test diff against the declared base.
`verification.json` pins its SHA256 and confirms that the tested source hashes
exactly match committed blobs in `bd53e6f93`.

Root explicitly authorized host fallback after both remote routes exited 200
before compilation; both aliases resolve to the same unhealthy SSM connection.
All tests below are **host execution, not remote success**. No cloud resource
or builder configuration was changed. Before building, 15 GiB was available;
the fresh task-owned target finished at 1.3 GiB with 12 GiB still available.
The old `/private/tmp/spur-r7-65bb3605-release-1/native-target` (1.6 GiB), release
package, old PoC evidence and retained native binary were not written.

### Exact focused RED/GREEN command

```bash
SPUR_REMOTE=0 \
CARGO_TARGET_DIR=/private/tmp/spur-notebook-u1-bd-bi1d-target \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 SPUR_SCCACHE_S3=0 \
TMPDIR=/private/tmp/spur-notebook-u1-bd-bi1d-tmp \
scripts/spur-cargo test -p spur-graph --no-default-features --lib extract::notebook
```

`capture_host.py` runs this wrapper command and retains exact command/environment,
working directory, input commit, source SHA256s, test-section SHA256s, UTC times,
exit code and raw-log SHA256. GREEN runs before committing implementation;
their source hashes identify the tested bytes independently of the parent HEAD.

| Receipt stem (each has `.json` and raw `.log`) | Exit | Result |
| --- | --- | --- |
| `host-red` | 101 | Initial host run: 20 passed / 15 failed; 14 feature assertions plus one pre-existing macOS temp-path mismatch |
| `host-red-canonical-tmp` | 101 | 21 passed / 14 failed; meaningful normalization/validation assertions only |
| `host-green-final` | 0 | 35 passed / 0 failed; same command/environment and exact test predicates as canonical RED |
| `host-lexical-red` | 101 | 35 passed / 1 failed; `::std` incorrectly classified as a command |
| `host-lexical-green` | 0 | 36 passed / 0 failed; same command/environment and predicates as lexical RED |
| `host-extract-green` | 0 | 118 passed / 0 failed; same environment with filter `extract::` instead of `extract::notebook` |

No tests were ignored. The broader run covers ordinary extraction, all language
query contracts, notebook stable keys, declared/actual metadata facts and direct
versus batch extraction. Existing unrelated dead-code warnings remain in logs.
Local `SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -- --check` and
`git diff --check` both exited 0.

The initial host RED exposed an existing `/var` versus `/private/var` alias
mismatch in the direct/batch test. Setting a canonical task-owned TMPDIR removed
that environment mismatch without changing source/test bytes or ordinary APIs.
Both RED runs and hashes are retained. The intermediate `host-green` receipt is
also retained; its simplistic first-`#[cfg(test)]` predicate hash temporarily
included production bytes. Use `host-green-final` for the exact predicate match.

The graph function-recovery fixture already passed with the old graph parser on
this host; it is compatibility evidence, not a falsely claimed graph RED. The
new model/preparation and invalid/duplicate graph inventory did fail before
implementation. The preparation assertion checks original/analysis hashes,
byte-length identity, multibyte directives, CRLF, original function ranges and
policy. Root's lifetime/:load, raw-byte-string, nested-comment, cross-category
effective-ID collision and Python magic fixtures all pass.

`solver-host-pre.json` and `solver-host-post.json` retain exact catalog requests
and raw results using `persist:false`: finite distinct keys SAT/pass; duplicate
complete keys UNSAT/fail. Cached solver results are identified as such. These
finite uniqueness witnesses do not establish parser or runtime correctness.
`verification.json` cross-checks receipt hashes, predicate equality and retained
attempt-1 artifacts. The graph retrieval tool lacked a worktree artifact, so
raw working-tree reads were used after attempting the required graph tools.

### Public interface for U2

```rust
pub fn normalize_notebook(bytes: &[u8]) -> NormalizedNotebook;
pub fn NormalizedCell::identity_path(&self, notebook_path: &str) -> String;
pub const RUST_PREPARATION_POLICY: &str = "evcxr-leading-dep-mask-v1";
```

All are exported in `spur_graph::extract::notebook_model`. Public structs/enums
are `NormalizedNotebook`, `NormalizedCell`, `PreparedSource`, `DependencyDirective`,
`NotebookStatus`, `CellIdentityQuality`, `NotebookDiagnostic`,
`NotebookDiagnosticCode`, `SourceCoordinateMap`; their fields/variants are public.

Notebook status aggregates cell status with Invalid > Partial > Complete.
Notebook-level diagnostics describe wrapper/root failures; cell diagnostics
carry zero-based ordinals and optional half-open decoded UTF-8 byte ranges.
Original and analysis SHA256s belong to decoded cells; notebook SHA256 covers
exact wrapper bytes. `SourceCoordinateMap::Identity` preserves decoded-cell
bytes/lines. Missing metadata is Null; provided metadata is preserved verbatim,
with non-object shapes explicitly Invalid. Invalid source shapes yield None,
never silently filtered text. Invalid inventory is rejected by graph extraction.

SourceDerivedIdentity is informational; repeated anonymous sources are Partial.
Duplicate effective IDs make every colliding cell Ambiguous/Invalid, including
explicit IDs colliding with anonymous hashes or occurrence suffixes. Existing
valid trimming, BLAKE3 IDs/suffixes, language registry/precedence and notebook
identity paths remain compatible. Original source owns graph labels and spans;
only the parser uses analysis bytes. GraphFacts and ordinary-file APIs/schemas
are unchanged; no dependencies or notebook runtime requirements were added.

Only syntactically supported leading single-line dependency assignments are
masked. `directives` records their original text/ranges; unsupported forms stay
in original/analysis source with Partial diagnostics. Raw/ordinary strings,
chars, lifetimes, nested block/line comments and absolute Rust paths are handled
by the lexical boundary. Commands following comments/code are never promoted
to supported leading headers. No commands execute and no :load targets open.

Complete denotes supported normalization/preparation, not proof of Rust/Python
syntax, dependency resolution or runtime equivalence. This is bounded Evcxr
preparation, with conservative inline :dep/:load detection. Python first-nonblank
%/%%/! prefixes are Partial and stay unchanged; Python magics are not implemented.
Legacy graph success alone still cannot represent complete notebook coverage.
U2 symbols/comparison, SQL schemas, native packaging/replay and frontend work
remain outside U1; no remote compilation success is claimed.
