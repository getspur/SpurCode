# U1 attempt 3: Rust type annotations versus kernel commands

Task bd-bi1d / U1. Rework requested by root review: `type dep = i32;` followed
by `fn f(x:dep ) {}` must retain Complete normalization and unchanged source.
The review's additional `U1-attempt2` artifact directory was not yet present on
the main branch at session start; tests here retain the exact reported fixture
and the requested annotation variants directly from the task description.

## Preserved history and scope

The orchestrator staged a byte-identical overlay of `e0c32e6b6`. Both
`git diff --cached e0c32e6b6 --stat` and `git diff --stat` returned empty.
Attaching this isolated worker branch to that commit with `git reset --soft`
preserved every working file and original TDD commit, including `5b5f8d1cd`,
`9253e9940`, `0363da1dd`, `0e61c92d8`, `b30f19f2e`, and `bd53e6f93`.
All earlier remote/host receipts remain unchanged in the parent directory.
No merge, issue closure, external notebook/runtime, native release target,
shared-builder operation or cloud change is part of this work.

## Rework plan

1. Commit embedded regressions for dep/load aliases, whitespace, multiline
   parameters/fields/let patterns, and comments around type colons. Retain true
   standalone and inline command fixtures, including after type annotations.
2. Capture an assertion RED using the authorized host wrapper fallback.
3. Narrow lexical command boundaries without implementing a full Evcxr grammar.
   Document `identity_path` as legacy graph compatibility, not a collision-safe
   public symbol preimage.
4. Capture focused notebook GREEN with identical command/environment/predicates,
   then run the extraction suite once. Commit source and exact receipts.

The solver catalog was consulted before RED (`solver-catalog.json`). No
implemented catalog family describes Rust lexical token recognition, and this
fix invents no numeric bound or quota. It uses TDD; the prior identity
uniqueness solver requests/results remain in the parent directory.

## Host execution

Remote routes previously exited 200 before compilation because SSM was
disconnected. Root authorized this host fallback explicitly; no remote compiler
or assertion failure is being retried locally. Before this run, `df -h` showed
19 GiB available and the existing task-owned target occupied 1.3 GiB. Reuse that
cache; never write the retained 1.6 GiB R7 native target or release package.

```bash
SPUR_REMOTE=0 \
CARGO_TARGET_DIR=/private/tmp/spur-notebook-u1-bd-bi1d-target \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 SPUR_SCCACHE_S3=0 \
TMPDIR=/private/tmp/spur-notebook-u1-bd-bi1d-tmp \
scripts/spur-cargo test -p spur-graph --no-default-features --lib extract::notebook
```

`capture_host.py PHASE [FILTER]` captures raw stdout/stderr, exits, exact argv
and overrides, working directory, source commit/SHA256s, test-predicate SHA256s
and log SHA256. Use `extract::` for the broader extraction run. These are host
results, never remote success.

## Verified result

Rationale: a colon owned by a well-formed Rust syntax construct is not a kernel
command boundary, so valid annotations stay Complete while real commands retain
explicit Partial diagnostics and unchanged original-source ranges.

The fix is confined to `notebook_model.rs`: the existing lexical scanner still
excludes strings, lifetimes and comments; a lazy, cell-local parse with the
existing bundled Rust grammar disambiguates remaining colon candidates. Only
the immediate syntax owner must be free of parser errors; unrelated commands
elsewhere in the cell do not invalidate a valid parameter or field. Parsing
uses the current analysis bytes, and masking a supported header invalidates the
cached tree. No dependency, public signature, ID rule, GraphFacts schema or
ordinary-file behavior changed. No directive executes and no target is read.

The regressions cover dep/load aliases, whitespace before/after colons,
multiline parameters/fields/let annotations, const annotations, tuple and struct
let patterns, and line/nested-block comments around colons. They require
Complete status, no diagnostics, byte-identical original/analysis source,
identical hashes and no fabricated dependency directives. Genuine commands
after those functions still have exactly one diagnostic at the real command
range, whether separated by newline, space or a block comment.

| Receipt stem (`.json` and raw `.log`) | Exit | Observation |
| --- | --- | --- |
| `host-red` | 101 | 36 passed / 3 failed; exact root input diagnosed `:dep` at bytes 22..31, plus annotation/comment and real-command-range regressions |
| `host-green` | 0 | 39 passed; intermediate grammar-context implementation, retained without overwriting |
| `host-green-final` | 0 | 39 passed; final implementation parses analysis bytes and invalidates after masking |
| `host-extract-green` | 0 | 121 passed; the previous 118 extraction tests plus all 3 new regressions |

RED and both focused GREEN receipts have identical argv, environment overrides
and test-predicate hashes. The broader run uses the same environment/argv with
only the filter changed to `extract::`. No tests were ignored. Existing stable-ID
compatibility, direct/batch extraction, :dep graph function recovery,
UTF8/multibyte/CRLF spans, raw/ordinary strings, raw-byte strings, lifetimes,
nested comments, unsupported commands/multiline/trailing forms, ID collisions,
metadata/language/source decoding and Python magic fixtures remain passing.

Exact final notebook-model SHA256:
`76f85c09239204dbc2a575654e921353e663a665855daf63c1c6e7fe7db56186`.
Exact model test-predicate SHA256 in RED/GREEN:
`61674c02a16f5829d40027d56578b58c5e16590502c697aabc7a8946128d1121`.
Each JSON receipt retains all source, wrapper, lockfile and raw-log hashes.
GREEN ran before the feature commit; `verification.json` checks those tested
bytes against the committed source and every prior receipt against `e0c32e6b6`.

`SPUR_REMOTE=0 scripts/spur-cargo fmt -p spur-graph -- --check` exited 0.
Source `git diff --check` exited 0. Raw logs and patch evidence retain their
original whitespace. After validation, the task-owned target remained 1.3 GiB,
with 18 GiB available. No old release target/package was overwritten.

`rework.patch` is the unified source/test diff against attempt 2 (`e0c32e6b6`).
`source.patch` is the full U1 source/test diff against dispatched base
`52e14c9cde58f0650bacb72daa04b705fd3c17da`. Their SHA256s and committed-source
cross-checks are in `verification.json`. Original test/feature commits remain
ancestors; the issue stays open for root review and no merge was performed.

Rework commits: `441bd06d3` adds regressions, `079b08558` retains assertion RED,
and `0bac16063` fixes preparation after GREEN. `verify_receipts.py` exited 0:
all 25 prior artifacts are byte-identical, RED/GREEN commands/environments/test
predicates match, final GREEN/extraction hashes match committed source, and both
unified diffs pass reverse-application checks against the final source tree.

## Public interface and limits

The interface remains exported in `spur_graph::extract::notebook_model`:

```rust
pub fn normalize_notebook(bytes: &[u8]) -> NormalizedNotebook;
impl NormalizedCell {
    pub fn identity_path(&self, notebook_path: &str) -> String;
}
pub const RUST_PREPARATION_POLICY: &str = "evcxr-leading-dep-mask-v1";
```

Public models remain `NormalizedNotebook`, `NormalizedCell`, `PreparedSource`,
`DependencyDirective`, `NotebookStatus`, `CellIdentityQuality`,
`NotebookDiagnostic`, `NotebookDiagnosticCode`, and `SourceCoordinateMap`.
The parent receipt documents their fields and diagnostic/coordinate semantics.

`identity_path` is **legacy graph stable-key compatibility**, not a collision-safe
new public symbol preimage. U2 must encode notebook path and cell ID separately
in canonical symbol identity. Its existing output format is preserved.

This is bounded notebook preparation with syntax-context disambiguation, not
an Evcxr grammar or a Rust semantic/runtime check. Rust error recovery can still
produce conservative Partial coverage for malformed constructs. Complete
normalization is not a claim of valid Rust/Python execution or dependency
resolution. Python magics remain diagnosed rather than implemented. No U2
symbols/comparison, SQL schemas, native package/replay or frontend work is part
of this rework. Remote receipts remain infrastructure failures, not compiled
test evidence; every new validation result above is explicit host execution.
