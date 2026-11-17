# C++ repository resolution: measured baseline and proposed uplift

The main experiment now indexes every tracked C++/header file at both PR
snapshots through the actual DuckDB extension. Indexing succeeds, but each SQL
input retains single-file resolution. The existing Rust shared builder already
links across files and produces additional bindings. Exposing that capability
is necessary, alongside fixing demonstrated precision and extraction gaps.

This is a research proposal. No production extractor or extension behavior was
changed.

[Live experimental notebook](/Users/kevintruong/project/spur-notebook/2026-11-17-cpp-extraction-gap-lab.ipynb)
· [Evidence](../../research/artifacts/2026-11-17-cpp-extraction-gap-lab/)
· [Repository results](../../research/artifacts/2026-11-17-cpp-extraction-gap-lab/repository-summary.json)
· [Rust comparison](../../research/artifacts/2026-11-17-cpp-extraction-gap-lab/rust-full-repository-summary.json)

## 1. What was actually indexed

DuckDB PR #28731 inputs are immutable Git objects:

- Base: `629e395a854d72dcb1f414e1a0451c68ac0e9f1f`.
- Head: `d29b26bb92ac940a0807be0f43cc385b31623839`.
- DuckDB runtime: native `2.0.0`; extension `0.1.0`.
- Extension artifact SHA256:
  `41f5e65ad778a4e90d513ef5dc2f5065d0f45629968a20a231ece962e9bf627f`.
- Extractor identity:
  `source-sha256-v1:6ff57778ccc62a9cbd78f39549836fd9a7c91f55c3f94830443b239e8a17f2c2`.

The manifest inventories all 18,341 base and 18,342 head tree entries. This
investigation extracts the complete C++/header inventory: **4,924 files per
snapshot**, including tests, third-party code, extensions, tools and benchmarks.
Other languages remain outside this C++ extraction scope. Untracked files,
generated build outputs and system headers are outside the pinned Git trees.

The notebook launches a consumer that executes
`spur_graph_file_facts_result(logical_path, source)` over SQL row batches.
It preserves the native typed result, input hashes and full source context.
There are 4,926 unique `(path, blob, extractor)` inputs across the two snapshots;
unchanged inputs are reused. Each snapshot has 4,923 `ok` results and one valid
`empty_source` result (`third_party/thrift/thrift/stdcxx.h`). No consumer errors
or SQL errors remain. `ok` describes extraction completion, not compiler validity.

File-local node IDs are joined with snapshot, path, blob and extractor context.
There are no dangling bound endpoints under that composite key. The local
database and frozen sources live under `.spur/artifacts/cpp-gap-repository-index/`;
the captured notebook, summaries, identities and scripts accompany this proposal.

## 2. The before measurements

| Head experiment | Call facts | Bound calls | Unresolved calls | Cross-file bound calls |
|---|---:|---:|---:|---:|
| Extension, whole C++ inventory | 230,744 | 20,551 | 210,193 | 0, by single-file contract |
| Extension, two changed files | 366 | 17 | 349 | 0, by single-file contract |
| Existing shared Rust builder, whole inventory | 229,688 | 43,289 | 186,399 | 27,314 |
| Existing shared Rust builder, two changed files | 353 | 69 | 284 | 53 |

The Rust experiment calls `build_facts_for_paths` directly in a notebook Rust
cell against all 4,924 verified head inputs. Both approaches produce 156,494
head nodes. The shared builder reclassifies **13 changed-file call facts as
`constructs`**, so 353 calls plus 13 constructions account for the original 366
facts. Raw call counts cannot be compared as a semantic recall percentage.

The extra bindings are existing syntax heuristics, not compiler-verified targets.
Examples include `SetSingleValueIdentity` in `aggregate_function.hpp` and
`GetExpressionClass` in `base_expression.hpp`. Some targets are declarations:
`EstimateDirectFilterDomain` calls a header declaration rather than the distinct
implementation node. Declaration/definition identity therefore matters for
impact traversal from changed function bodies.

The original 349 unresolved facts break down as follows:

| Evidence shape | Facts |
|---|---:|
| Receiver without a call chain | 211 |
| Receiver containing another call | 22 |
| Bare name | 87 |
| Explicit `std` scope | 8 |
| Other explicit scope | 21 |

Against the complete head callable-name inventory, **41** have zero same-name
nodes, **56** have one, and **252** have several. These are raw name candidates;
they include declaration/template representations and do not establish visibility,
type compatibility or correct binding. For example, `size` has 63 raw candidates
across 41 files. No candidate lookup was converted into a confirmed edge.

The two files contain **531 tree-sitter call-expression nodes** but emit 366
call facts. Casts, constructions, macros, omitted syntax and representation
choices affect that comparison. Without call-site identity it is invalid to
label the difference as exactly 165 missing runtime calls. The probe does prove
specific omitted template and recursive calls below.

## 3. Compiler-grounded failure cases

All 14 controlled fixtures compile with Apple Clang 17 in C++17 syntax-only mode.
Clang AST references supply expected targets. The actual Rust core and actual
native extension produce identical nodes, edges and spans for all 14 inputs.
This parity confirms current behavior; it does not mean the behavior is correct.

| Case | Compiler evidence | Current observed behavior |
|---|---|---|
| Qualified miss | `foreign::leaf` declaration | Binds `actual::leaf` with `bare_qualified_singleton` |
| Header receiver | Receiver type is `external::B` | Single-file extraction binds unrelated `A::run` |
| Function-pointer shadow | Call refers to parameter `caller::target` | Binds the global `target` function |
| Overload | `pick(1.0)` selects `double pick(double)` | Chooses `int pick(int)` with `same_file_duplicate` |
| Typed receivers | `a.run()` and `b.run()` bind to different classes | Both remain unresolved |
| Header free function | Included inline `util` exists | Remains unresolved even in the selected-file shared builder |
| Bare template | One `identity<T>` definition | Wrapper/inner representations prevent binding |
| Qualified/member templates | Two calls with compiler-selected targets | Neither call is emitted |
| Recursion | `rec` calls itself | No call edge is emitted |
| Callback | Direct `invoke`, indirect parameter `f`, callback value | Direct/indirect call evidence differs; custom callback value has no reference edge |
| Macro | Clang-expanded call is `callback` | Syntax graph retains unresolved `RUN`; expansion is unavailable |
| Construction | Clang identifies construction of `A` | Unresolved `calls` fact in this fixture |

Positive controls cover a local bare function and namespace-qualified functions.
The receiver fixture also shows that adding a header can remove a false
singleton binding without producing the correct binding. A count increase alone
is not a sufficient acceptance criterion.

## 4. Where the current implementation loses evidence

All references below are grounded in the current worktree; hashes are captured
in `source-identities.json`.

- `tree_sitter.rs::BytesExtractor::extract_file_facts` creates a fresh builder
  for one source. `build_facts_for_paths` shares a builder across selected paths.
  The extension currently exposes the former scope.
- `queries/cpp/spur-edges.scm` captures ordinary bare, qualified and member
  calls and bare template calls. The measured qualified/member template shapes
  are absent. Custom callback reference capture is limited to a closed STL list.
- `languages.rs::collect_typed_bindings` accepts `parameter` and
  `let_declaration`, rather than the principal C++ declaration forms. C++
  receiver and shadowing evidence is consequently missing.
- `tree_sitter.rs::resolve_call_edge_after_qualified_miss` protects explicit
  Rust qualifier misses; C++ can fall through to bare resolution.
- `same_file_duplicate_function_candidate` treats same-file functions with
  the same qualified name as duplicates and selects the first. That is not
  sufficient to distinguish C++ overloads from duplicate syntax captures.
- `function_singleton_safe` uses generic path-scope rules. The included-header
  free-function fixture demonstrates that a shared builder alone is insufficient.
- `callable_symbol_candidates` excludes the source node, contributing to the
  measured missing recursive call.
- `PendingEdge` lacks call-site coordinates, argument/template evidence and a
  structured unresolved reason. Graph edge evidence IDs do not substitute for
  immutable call-site source references.
- `queries/cpp/tags.scm` can capture both a template wrapper and its inner
  definition. Canonical callable identity must also unify declarations with
  definitions while retaining distinct overload signatures.

## 5. Proposed implementation order

### A. Make contradictory bindings impossible

Start with failing tests for the four compiler-grounded false bindings. Preserve
unresolved evidence when a qualifier conflicts, a lexical value shadows a
function, a known receiver type conflicts, or an overload has no justified
selection. Collapse duplicate captures only when they represent the same
underlying declaration/definition identity, not merely the same qualified name.
Keep C++ policy isolated from existing language behavior where their rules differ.

### B. Retain complete C++ call evidence and canonical entities

Add qualified/member template coverage, recursive calls and explicit construction
classification. Capture C++ parameters/local declarations, pointer/reference and
alias forms with lexical scope. Retain call byte spans, callee AST kind, raw
receiver/qualifier, argument arity, template syntax and parser coverage.

Separate source call sites, canonical callables, declarations, definitions and
candidate bindings. Callback values, indirect calls and macro expansion require
different relations/evidence. Do not infer that passing a callback always executes
it. Preserve an explicit reason for each unsupported or unresolved case.

### C. Expose repository resolution through the extension

Reuse the existing shared Rust builder as the measured starting point, after
the soundness and identity guards. The consumer should stage an immutable full
source manifest, parse/cache sources, build a repository symbol index, resolve
in the appropriate context and publish coherent typed results.

The new repository API must carry repository/revision and build/translation-unit
context, globally meaningful symbol identity, target source references,
resolution method, candidates and unresolved reasons. A bare file-local integer
must never identify a target in another file or revision.

Choose the concrete DuckDB table/function ABI during implementation against the
pinned native API. Avoid presenting a giant unbounded source LIST as a production
ingestion design. Keep the current seven functions compatible and explicitly
single-file; add a versioned repository result contract. SQL row batching
amortizes parser setup but does not itself change resolution scope.

### D. Add a compiler-backed semantic tier

Use compilation-database inputs when available to ground includes, defines,
language flags, overloads, aliases, templates and macro expansions. Store those
configuration identities with results. Syntax-only mode remains useful and must
report its limits. Virtual dispatch and function pointers can require possible
target sets; compiler binding does not prove all runtime behavior.

Do not promise that every unresolved C++ reference becomes a single exact edge.

### E. Make the app honest and useful for humans and agents

Index the whole relevant repository snapshot first. Changed symbols are review
entry points; callers and impact paths must traverse that snapshot's graph.
Canonical declaration/definition identity lets an incoming call to a header
declaration reach the changed implementation.

Show graph scope, build context, extraction coverage and unresolved/candidate
counts next to impact results. Each path needs source witnesses and binding
evidence. Allow explicit exploration of candidates, while keeping them separate
from accepted impact paths. Agents receive the same identifiers, reasons and
source references that humans can inspect. An incomplete graph cannot justify
an assertion that a change has no wider impact.

Architecture membership and stakeholder attribution remain separate evidence
layers, joined to these canonical symbols; the call graph alone cannot establish
ownership or architectural policy.

## 6. Acceptance for the uplift

1. The four false-binding fixtures select the compiler target or remain explicitly
   unresolved; no contradictory target is bound. Keep overloads distinct and
   declarations/definitions navigable as one callable identity.
2. Account for every fixture call expression as a captured site or an explicit
   unsupported form. Test templates, recursion, construction, indirect calls,
   callback references, macros and header-only definitions independently.
3. Add translation-unit negatives: internal linkage, anonymous namespaces,
   aliases, overloads across headers, conditional compilation and differing flags.
   Candidate inventory size must not change an existing binding without evidence.
4. Re-run all 4,924 C++ inputs per pinned snapshot through the versioned SQL
   repository API. Preserve full source keys, empty/error diagnostics, stable
   site identity and replay/cache correctness. Reject cross-snapshot targets.
5. Verify impact paths through a declaration to a changed definition. Report
   observed syntax bindings separately from compiler-backed bindings and possible
   runtime targets; validate both humans' drill-down and agents' query results.
6. Run appropriate core regressions and native extension release checks after
   production changes. This research run verifies the probes, not a future uplift.

The Solve finite policy rejects the observed contradictory bindings (`unsat`)
and accepts their proposed conservative unresolved decisions (`sat`). It checks
the encoded fixture policy, not general C++ semantics. The server artifact quota
was full; successful non-persisted solver responses and requests are retained
locally with the initial quota errors.

The consumer microbenchmark compared identical native calls: 14 separate SELECTs
took 5.28 seconds; one 14-row SQL query took 0.35 seconds. Transaction batching
alone barely changed the cost. This justifies batching this probe's SQL work;
it is not a release performance guarantee. A cold memory/throughput benchmark
must precede production budgets and supervision choices.

Recommended first implementation slice: TDD for C++ binding soundness and
canonical callable/site identity, followed by the repository SQL contract using
the existing shared builder. Preserve this notebook as the before evidence.
