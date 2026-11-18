# Spur Graph Review: interactive notebook app specification

Status: design draft grounded in executed experiments. Native app implementation is a subsequent step. The accompanying interactive artifact demonstrates navigation over captured evidence; it does not collect GitHub data, run an AI review or publish a review.

## Product decision

Spur Graph is a view inside Spur Notebook. Its primary review workflow starts with a PR's changed files and symbols, then explains their relationships to the entire tracked repository. Unchanged code remains visible whenever it supplies a client, contract, callback, test, configuration or component relationship.

The reviewer should be able to answer:

1. What behavior changed, and where should I inspect its effects?
2. Which paths connect this change to surrounding code, and how well established is each connection?
3. Which components and contracts does it cross? Does an explicit architecture policy permit those changes?
4. Who is participating, who has declared responsibility, and who has relevant historical context?
5. What review questions or findings follow from the available evidence?

The default is a focused review workspace. Repository exploration is an explicit scope that can grow from a selection. A whole-repository graph canvas is available as a later exploration mode, without replacing the changed-file review path.

## Grounding and scope

The design uses DuckDB PR #28731, “Preserve aggregate domains for proven singleton groups.” Its captured comparison is base `629e395a854d72dcb1f414e1a0451c68ac0e9f1f` to head `d29b26bb92ac940a0807be0f43cc385b31623839`.

The completed [repository context experiment](../../research/2026-11-18-pr28731-change-context.md) inventories 18,342 tracked head files and establishes five changed files, five semantic function changes, unchanged API clients, unchanged wrapper bodies, callback registration/dispatch candidates, shared aggregate metadata, CMake membership and inherited test exclusions. The context map is selected, typed evidence over the tracked tree. It is not a complete semantic or runtime impact graph.

The [C++ resolution investigation](2026-11-18-cpp-repository-resolution.md) shows that the installed SQL extraction contract remains single-file. A shared Rust graph recovers additional cross-file relationships, but has known qualifier, receiver, shadowing and overload errors. The UX must preserve those limits. A node count or numeric confidence cannot turn syntax matching into compiler proof.

DuckTails, Spur Graph and DuckPGQ loaded together in the [extension POC](../../research/2026-11-17-spur-review-extensions-poc.md). The optional graph engine traverses established tables; it cannot infer relationships missing from extraction. GitHub data is a captured resource snapshot, with incomplete current reviewer/check information. Historical people evidence and AI readiness are supported; an AI review has not been executed.

## People and jobs

| Person | Main job | Success signal |
|---|---|---|
| Contributor | Check consequences before requesting review | Explains affected contracts, callers and test modes with citations |
| Maintainer | Find the risky assumptions efficiently | Opens an exact witness, distinguishes a defect from a question, records a decision |
| New contributor | Learn the architecture around a PR | Moves from a changed function to its build component, clients and shared interfaces |
| Reviewing agent | Retrieve bounded, revision-correct evidence | Gets typed records and source references identical to those shown to the human |

## Information architecture

```text
Spur Notebook
  Notebook | App
  Spur Graph Review
    Repository setup
    GitHub inbox: Pull requests | Issues | Saved reviews
    Review workspace
      Changes + context
      Architecture
      People
      AI review
    Evidence and jobs
```

The notebook owns the app and its evidence pipeline. App mode hides code cells from the review flow. “Open in notebook” selects the producing cell and returns to the same app selection afterward. The Notebook/App switch is host navigation; the review app never draws a second notebook tab bar.

## Journey A: connect and collect

Repository setup is a resumable page, not a modal with many mandatory fields.

1. Choose a local Git directory. Show canonical root, remote candidates, dirty status, shallow state and object availability.
2. Confirm upstream identity and optional fork independently. A remote named `origin` is not assumed to be upstream.
3. Select the saved GitHub connection (`@github-pat` in this workspace). Keep only its reference in the browser. Public/offline/cached use remains possible.
4. Validate the actual DuckDB runtime and the three provider probes. Show each provider's status and its consequence, such as “History unavailable” or “SQL path queries available.” Advanced details contain versions and receipts.
5. Choose index scope. The tracked-file inventory is the context baseline; language extraction, binary metadata, generated/vendor exclusions and missing submodules are separate coverage categories. Show excluded paths before collection.
6. Save configuration, then explicitly start collection or open the captured example. Saving does not switch branches, stash local edits or fetch all history.

The PR inbox supports search, state/author/label filters, Issues/PRs tabs, a pasted URL and saved review runs. Each row shows title, number, author, collection time and completeness. Missing review/check fields read “Not collected”; a successful empty collection reads “None.” Cached rows remain readable during refresh.

Opening a PR resolves target base, merge base and head, then freezes source blobs and GitHub resources into a review run. Default comparison is merge-base to head. Working-tree review, target-branch drift and an experimental merge-result comparison are separately named contexts.

## Journey B: understand a change in context

### Workspace layout

At a wide desktop viewport, use three panes below a persistent PR header and lens tabs:

```text
duckdb / #28731   Preserve aggregate domains...      [Snapshot] [Review packet]
629e395 -> d29b26b  Captured source   Partial linking  New revision if available
[Changes + context] [Architecture] [People] [AI review]

Changed files + symbols | Selected change and question              | Evidence
                       | [Context map] [Paths] [Source / diff]      | Relation, status
5 file entries         | [Incoming clients] [Dependencies]         | Exact source
expand to 5 functions  | [Shared contracts] [Tests + config]        | Revision and scope
                       |                                           |
unchanged context in   | Small typed neighborhood around selection | Why related?
separate result list   | Ordered paths below map                    | Limits / next step
```

The header is compact. Counts establish PR scope, not a dashboard of graph statistics. No risk score, centrality leaderboard or percentage-complete blast radius appears without an agreed meaning and measurement.

The changed list groups code and tests/configuration, with path suffixes and change badges. Files expand to normalized symbols. A modified file can contain an unchanged function: the function badge must say “Unchanged body” even when its file says “Modified.” Rename, deletion, duplicate/ambiguous identity and unsupported source remain explicit states.

### Selection contract

The shared selection contains repository, review run, comparison side, file identity, optional scoped symbol identity, lens and optional evidence ID. Selection is independent of lens filters and presentation mode.

- Clicking a file selects its change summary and context. Clicking a symbol narrows context without discarding the file.
- Clicking a path, graph edge or node opens its evidence in the right pane. Nodes expose all matching records; a default witness is shown with an “Other witnesses” count.
- Changing a lens retains the selection. People and architecture initially filter to that scope; “Whole PR” is an explicit escape.
- Back/forward restores selection, lens and expansion state within the same run. A refresh never jumps the reviewer to another file.
- Copy reference copies a review-scoped citation. Agent exports use the same identity; raw node IDs never join graphs from different snapshots.
- A node outside the changed files has an “Unchanged context” badge. Opening it keeps the changed seed pinned. “Explore from here” makes a new exploration root and leaves a breadcrumb back to the PR selection.

### Context questions and expansion

Use named questions rather than only a depth slider:

| Control | Meaning |
|---|---|
| Incoming clients | Who references or calls the selected API? Arrows point client to callee |
| Dependencies | What does the selection call/read/use? Arrows point selection to dependency |
| Shared contracts | Who writes/reads/registers the same contract? Participation is not automatically runtime propagation |
| Tests + config | What tests, inherited configuration and build declarations relate to the change? |

Default neighborhood is bounded to two relationships, with a smaller readable subset rendered and the returned total listed. Production node/row budgets are configuration backed by a usability/performance probe, not fixed completeness claims. “Show more” paginates; “Expand from this node” displays the added scope and frontier. Cancellation preserves previously loaded results.

The map uses stable layered positions: upstream clients at left, selected change at center, dependencies/contracts at right. It avoids continuous force motion. A sibling path list is always available and encodes the same relations, direction, uncertainty and selected witness. Filtering candidates removes their paths from both views and reports how many were hidden. It does not upgrade remaining paths to verified runtime behavior.

If no path is found, display “No path found within this query's scope” plus the missing/excluded frontier and available next actions. Never display “No impact” from an empty result.

### Evidence grammar

Change state and relation quality are independent channels. Use text badges, line patterns and icons/shapes in addition to color.

| Evidence | Label in app | Map treatment | Permitted conclusion |
|---|---|---|---|
| Source span changed | Modified / Added / Deleted | Selected node has strong outline; added node has plus badge | Bytes or normalized symbol changed |
| Identical symbol body | Unchanged body | Neutral node, explicit body badge | Body is identical although line/file may change |
| Syntax binder result | Syntax-bound call | Solid arrow, method in inspector | Extractor selected this endpoint; C++ semantics may remain unverified |
| Compiler-backed binding, if later supplied | Compiler-resolved call | Solid arrow plus provider label | Binding under recorded build configuration |
| Textual API reference | Reference candidate | Dashed arrow and “Candidate” text | Source references a name; overload/definition may be unresolved |
| Registration + dispatcher | Possible callback flow | Dashed arrow, both witness groups | A possible flow, not an observed dispatch |
| Shared metadata | Shared contract | Dotted links labeled reads/writes | A review neighborhood around a common contract |
| Parsed config inheritance | Inherits / Excludes test | Solid typed configuration arrow | Effective declared test selection, not an executed test result |
| CMake membership | Build-declared component | Containment or membership edge | Declared target membership, not policy compliance |
| Historical co-change | Historical association | Separate opt-in layer | Past co-editing, not causal dependency |
| Missing linkage | Unresolved frontier | Open-ended stub and readable message | Analysis is incomplete at that frontier |

The inspector shows relation direction, before/head status, method, exact witness file/span and source bytes. Advanced details expose blob hashes, graph identity, query bounds and originating artifacts. A function-start location is labeled as a function anchor, never presented as an exact call site. Syntax-bound paths carry the weakest relevant limitation even when one leg is well established.

## Required PR #28731 interactions

### 1. Unchanged clients through wrappers

Select `ExtractAggregationStatsInternal` in `relation_statistics_helper.cpp`. Incoming context shows unchanged `relation_manager.cpp` and `relation_statistics_extractor.cpp`, plus existing optimizer test references. Their reference-candidate edges reach the public API. The two overloaded wrapper bodies at head lines 688 and 696 are unchanged, although their positions moved. Their syntax-bound links reach the changed internal helper, which calls the new helpers.

The inspector must show the seam: source-reference candidate outside the retained graph, syntax binding inside it. Selecting an unchanged wrapper shows its identical body hashes and before/head positions. The reviewer can compare wrapper behavior without seeing a false modification caused by movement.

### 2. Shared identity contract

Select `RewriteOrderedFirst`, then Shared contracts. Show its setter at line 57, the unchanged `AggregateFunction` contract declaration/read/set methods, the statistics reader at line 664, unchanged `minmax.cpp` producers and the unchanged recursive-CTE consumer at line 42.

The heading reads “Shared contract neighborhood.” Selecting the recursive-CTE consumer explains why it is present and says runtime impact has not been established. The four callback registrations can be opened alongside the unchanged dispatcher at `aggregate_rewrite.cpp:58`; the connection is labeled possible callback flow.

### 3. Non-code propagation

Select the added `distinct_on_stats_propagation.test` or `stats_dependent_tests.json`, then Tests + config. Show:

- The changed shared skip list excludes the new regression.
- Two unchanged configurations, `disable_optimizer.json` and `verify_serializer.json`, inherit the list.
- The changed `verify_aggregate_state_export.json` also excludes it directly.
- The unchanged Makefile lists those three execution/verification configurations; the unchanged loader implements the inheritance.

There are four configuration records: one shared list and three execution/verification configurations. Only two inherit the shared list. Status reads “Declared exclusion; suite not run.” A test result can only replace that statement when a recorded execution exists.

## Journey C: architecture alignment

Architecture presents a component membership table and a small boundary map derived from the same selection. Initial DuckDB components use actual CMake targets: `duckdb_aggr_distr`, `duckdb_optimizer`, `duckdb_optimizer_join_order`, `duckdb_optimizer_relation_statistics`, `duckdb_operator_set` and `test_optimizer`.

Each component row expands to changed and unchanged members plus the build declaration witness. Crossing a contract connects review context; it is not automatically a new dependency or violation.

A separate policy panel reads “Alignment not evaluated: no approved policy.” Later policy setup proposes path/symbol membership, permitted dependency direction, public interfaces and exceptions. Human confirmation versions the policy. A PR cannot silently amend the trusted baseline used to approve itself.

Rule evaluations require rule ID/version, a before/head edge comparison and evidence. Distinguish Aligned, Potential violation, Needs mapping, Exception and Not evaluated. Existing base dependencies are not reported as newly introduced violations. Candidate links produce investigation items rather than definitive violations.

## Journey D: people

People has three independent sections: Current participants, Declared responsibility and Historical expertise. Missing GitHub fields show Unknown with refresh/permission details. A successful CODEOWNERS search with no match says No declared match; missing collection says Not collected.

Captured examples are `lnkuiper` for statistics and `kryonix` for aggregate rewriting. Their evidence comes from base-region blame and sampled file history, not the author's new PR lines. The detail view shows region, denominator, revision, time/sample bounds, commit witnesses and identity mapping. It does not sum incompatible samples into a synthetic ownership score.

Selecting a person highlights their relevant files and suggests review questions. “Draft request” creates an editable local note in a future implementation. Sending or assigning is a separate explicit action. The prototype has no contact action.

## Journey E: AI review and human decisions

The initial AI page says “Not run.” A readiness checklist shows diff, typed paths, architecture mapping/policy, people context, checks and known gaps. Partial evidence is allowed with visible scope and limits.

1. Select Whole PR, Current selection or Component; choose correctness, compatibility, architecture and test focus.
2. Prepare an immutable packet. Show included evidence and omissions before execution.
3. Select a saved provider and run the review explicitly. Record packet hash, model/provider, prompt version, scope and job state.
4. Present defects separately from questions and coverage gaps. Each defect includes trigger, consequence, severity, supporting evidence, counter-evidence, confidence rationale and validation steps. An unsupported citation is rejected or shown as ungrounded.
5. Keep, dismiss with reason or request verification. Dispositions bind to a finding version. Export an editable draft packet; posting to GitHub requires an explicit action and a preview of the actual destination/text.

People evidence helps route a question. It never establishes correctness. A reachable node is not itself a defect. Architecture alignment cannot be approved without a policy. PR prose and source comments are data rather than instructions to the reviewing agent.

For this prototype, the action is “Preview evidence packet.” It opens actual captured source evidence and proposed review questions labeled as design examples. It cannot imply an AI job executed.

## Shared state, jobs and failure recovery

| State/event | Visible behavior | Recovery |
|---|---|---|
| Initial inventory/index | Per-stage progress with usable captured data | Cancel job or open an earlier run |
| Partial extraction/linking | Scope message near context controls; inspectable frontier | Retry failed partition or inspect source |
| New PR head/context | “New revision available”; old run remains fixed | Create new run, compare changes, explicitly carry notes |
| GitHub rate/permission failure | Resource-specific unavailable status; cached records remain | Retry or change saved connection |
| Missing Git object | Name missing revision/object and affected comparison | Offer targeted fetch with declared refs |
| Provider mismatch | Explain disabled capability before load/query | Select correct runtime or retain SQL fallback |
| Canceled/failed AI job | Saved evidence and prior findings remain visible | Retry against same packet or prepare a new one |
| Stale asynchronous response | Result saved under originating run | Never overwrite active selection/run |
| Binary/generated/deleted file | File stays in changed list; source/extraction status explicit | View metadata, before-side source or adjust exclusions |
| Invalid or stale citation | Source unavailable/stale state, no silent relocation | Resolve exact blob or open the referenced run |

Notes and decisions persist separately from analysis results. Refresh does not erase them. Carrying a note to another run requires a stable identity match or human confirmation; a line offset alone is insufficient.

## Human and agent interaction parity

The agent interface offers the same proposed read contracts as the app: list changes, retrieve context by selection/question, retrieve exact evidence, inspect coverage, prepare packet and record a draft decision. These names describe required capabilities, not existing endpoints.

Responses include `review_run_id`, selection identity, typed records, source references, coverage, query bounds, truncation cursor and unresolved frontier. Expansion is bounded and cancelable. Every narrative conclusion can cite an evidence ID that the human opens in the inspector. A human can replay an agent's selection/path without rerunning collection.

Writes use explicit intents with expected run/version. A backend allowlist resolves paths, credentials and provider calls. Browser controls never execute arbitrary SQL/shell or read a local repository directly. Agent publication follows the same review/authorization boundary as human publication.

## Proposed data contracts and provider boundaries

| Contract | Required fields / responsibility |
|---|---|
| ReviewRun | repo/upstream/fork, comparison semantics, target/merge base/head, resource snapshot IDs, source manifests, provider versions, policy version |
| Change | before/head file/blob identity, change status, normalized symbol identity, match method and ambiguity |
| ContextNode | run-scoped identity, symbol/file/component kind, change state, source span, component memberships |
| ContextRelation | source/target, direction, type, evidence class, method, source-side status, witness IDs, limitations |
| ContextQueryResult | root, question, bounds, returned/hidden/truncated counts, cursor, unresolved frontier, coverage |
| Evidence | exact revision/blob, source span/anchor kind, content hash, origin, observed time, artifact/query reference |
| PersonEvidence | role, identity mapping, file/region, base revision, sample/date policy, source witnesses |
| ReviewPacket | run and policy IDs, ordered evidence IDs, scope, omissions, packet hash |
| Finding / Decision | packet/finding version, type, citations/counter-evidence, validation state, human disposition/reason |

Git/GitHub supply identities and resource snapshots. DuckTails supplies diff/history/blame. Spur Graph supplies extraction, matching, diagnostics and whatever binding scope its contract actually reports. A repository linker is still needed to automate reliable cross-file relationships. SQL/DuckPGQ traverse typed edges. An app-layer architecture evaluator supplies versioned membership/policy evaluations. A configured AI provider consumes the review packet. No extension is assigned a capability merely because its name sounds relevant.

## Visual design and interaction details

Design assumption: reuse the existing notebook brief's **Tech Utility** direction. This draft has not been recorded as a user-approved production visual system.

| Token | Proposed value / behavior |
|---|---|
| Canvas / panel / subtle fill | `#f6f7f6` / `#ffffff` / `#eff2ef` |
| Main / secondary text | `#1b2723` / `#53615b` |
| Border / strong border | `#d8dfda` / `#87998f` |
| Action / selected | `#176b48` / pale green `#e6f2e9` |
| Candidate / warning | Dark brown `#805416`, text and dash pattern always accompany it |
| Typography | System sans at 14px body; system mono at 12px source metadata; 22px PR title |
| Spacing | 4px rhythm; 8px compact gap, 12-16px panel padding, 24px major separation |
| Shape | 4-6px control radius, 1px dividers; avoid decorative card grids |
| Motion | Brief selection/expansion transition, no graph drift; honor reduced motion |

The changed seed and primary action use the green accent. Unchanged context is neutral. Semantic added/removed source backgrounds are subdued and carry plus/minus prefixes. Candidate state uses text and line patterns. Main evidence labels are readable at normal zoom; long symbols wrap and full paths are available on focus as well as hover.

### Accessibility and responsive behavior

- All controls have names and visible focus rings. Tabs use roving focus/arrow keys; Enter/Space selects. File/symbol rows follow tree/list keyboard conventions in implementation.
- Path list rows are keyboard reachable; every graph edge and node has an equivalent list action. SVG does not become the only way to retrieve evidence.
- Selection changes announce a concise status through a polite live region. Do not announce every progress tick. Source lines have accessible line numbers and text; evidence order is logical.
- Escape closes a sheet/dialog and restores the invoking control. Opening exact source focuses its heading. Keyboard shortcuts are discoverable and avoid overriding typing fields.
- At wide desktop width show all three panes. At intermediate width collapse the evidence inspector to a named drawer. At narrow width switch between Changes, Context and Evidence as full-width panels; preserve selection and back behavior.
- Production transitions between layouts must use content-fit testing at 200% zoom and long C++ symbols. Graph can scroll within a labeled region; the whole document must not require horizontal scrolling.
- Dark theme follows host tokens in implementation. This prototype covers the light direction only.

## Acceptance journeys

| ID | Given / action | Required result |
|---|---|---|
| UX-01 | Open captured PR | Five files, pinned comparison and partial linking visible; no full-coverage claim |
| UX-02 | Select internal stats function, inspect incoming path | Unchanged caller, unchanged wrapper and changed internal visible; candidate seam labeled |
| UX-03 | Open unchanged wrapper | Identical-body evidence and moved before/head positions; no false modified badge |
| UX-04 | Select rewrite and shared contract | Unchanged declaration and recursive-CTE reader appear with read/write witnesses and explicit runtime limit |
| UX-05 | Inspect callback | Four registrations and dispatcher shown; possible flow label retained |
| UX-06 | Select test/config | Two inherited and one direct execution/verification exclusions, shared list separate, suite not run |
| UX-07 | Open architecture | Actual CMake membership; alignment not evaluated without a policy |
| UX-08 | Open people | Historical candidates separate from unknown assignments/declared ownership |
| UX-09 | Preview AI packet | Same run/evidence; no generated defect or completed AI job implied |
| UX-10 | Change lens then return | File/symbol selection persists; source witness can be reopened |
| UX-11 | Filter candidates / no known path | Hidden candidates explained; absence never becomes no-impact claim |
| UX-12 | Move head, fail collector, receive old response | New run offered; old evidence/notes remain and no mixed snapshot shown |
| UX-13 | Keyboard / narrow / scripts disabled | Readable default design; path list equivalent; no graph-only task |
| UX-14 | Agent sends a citation | Human opens exact run/blob/span; invalid/stale IDs visibly rejected |

## Delivery boundary and next implementation slice

This delivery is the specification, captured-data interactive design artifact, notebook source and verification record. It does not report a native app doctor result or live integration readiness.

The first implementation slice should load a typed captured review run in a native notebook frontend, with shared selection, unchanged-wrapper context, path list and source inspector. Validate this one journey through the actual notebook DAG and doctor before wiring live collectors. Then add config/contract context, architecture and people, followed by explicitly triggered AI review. Reliable C++ repository linking remains a parallel capability requirement; it must not be concealed by a polished graph.
