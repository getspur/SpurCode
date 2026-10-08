<h1 align="center">SPUR</h1>

<p align="center">
  <strong>The control tower for your CLI coding agents.</strong>
  <br />
  Plan with one agent. Delegate to many. Review every change in one terminal.
</p>

<p align="center">
  <a href="https://github.com/getspur/SpurCode/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/getspur/SpurCode/actions/workflows/ci.yml/badge.svg" /></a>
  <a href="https://www.npmjs.com/package/@getspur/spur-cli"><img alt="npm" src="https://img.shields.io/npm/v/@getspur/spur-cli?logo=npm" /></a>
  <a href="./Cargo.toml"><img alt="Rust 1.88+" src="https://img.shields.io/badge/rust-1.88%2B-dea584?logo=rust" /></a>
  <a href="./LICENSE"><img alt="GPL-3.0-only" src="https://img.shields.io/badge/license-GPL--3.0--only-2ea44f" /></a>
</p>

<p align="center">
  <a href="https://getspur.dev">Website</a>
  ·
  <a href="./docs/user-docs">Documentation</a>
  ·
  <a href="#capability-demos">Demos</a>
  ·
  <a href="#quick-start">Quick start</a>
  ·
  <a href="./CONTRIBUTING.md">Contributing</a>
</p>

> [!WARNING]
> SPUR is early-stage software. APIs, configuration, and workflows may change while the project stabilizes.

## Featured demo

[spur-dag-aware-multi-agent.webm](https://github.com/user-attachments/assets/16d2fe40-d1cb-4024-b415-1aadebae63ac)

<p align="center">
  <sub>
    <a href="./docs/demos/spur-dag-aware-multi-agent.webm">Watch the 85-second WebM demo</a>
    — a brain plans the work, parallel agents execute it, and SPUR brings every result back to one review lane.
  </sub>
</p>

## Why SPUR

Running one coding agent is simple. Running several across vendors, repositories, and worktrees quickly becomes an operations problem.

SPUR is a Rust-native terminal application that sits above the agents you already use. It turns a task or issue into a durable plan, delegates work into isolated git worktrees, streams progress into one TUI, and keeps a human in control of every change that reaches the staging branch.

| You need | SPUR gives you |
|---|---|
| One place to operate a mixed agent fleet | Claude Code, Codex, Gemini, Kimi, OpenCode, Kiro (partial), and generic ACP agents behind one interface |
| Safe parallel execution | One isolated git worktree per worker, with liveness checks and orphan cleanup |
| Review before integration | Approve, reject, modify, or retry every worker result from a shared review lane |
| Predictable merge order | DAG-aware plans and topological cherry-pick onto a staging branch |
| Durable sessions | Event replay, persisted plans, and content-addressed outcomes that survive restarts |
| Cross-vendor visibility | A unified cost ledger and analytics across supported agent CLIs |
| Code-aware delegation | A tree-sitter code graph with stable symbol identities and incremental rebuilds |
| Review away from the terminal | An optional Telegram frontend backed by the same orchestration state machine |

## Capability demos

Eight focused capability demos — plus the featured walkthrough above — cover the full SPUR workflow in under nine minutes. Follow them in order for a guided tour, or jump directly to the capability you need.

### 1. Get productive

#### Install, detect, initialize

[spur-install-and-init.webm](https://github.com/user-attachments/assets/4ec5b465-7312-492f-926f-2883cf1e90b4)

<p align="center">
  <sub>
    <a href="./docs/demos/spur-install-and-init.webm">Watch the 1:10 WebM demo</a>
    — install the npm package, let <code>spur init</code> discover agent and PM tools, choose a brain, review permission settings, and launch the TUI.
  </sub>
</p>

#### Learn the keyboard model

[spur-help-and-shortcuts.webm](https://github.com/user-attachments/assets/3ca1c72f-976d-4ea8-a039-43b6c0f71747)

<p align="center">
  <sub>
    <a href="./docs/demos/spur-help-and-shortcuts.webm">Watch the 8-second WebM demo</a>
    — open the built-in help, understand navigation and compose modes, and learn the shortcuts for panels, lineage, review, and session control.
  </sub>
</p>

### 2. Control the workspace

#### Navigate sessions and commands

[spur-session-navigation.webm](https://github.com/user-attachments/assets/5f75d862-010a-4eda-bc2c-b162bd5c9452)

<p align="center">
  <sub>
    <a href="./docs/demos/spur-session-navigation.webm">Watch the 13-second WebM demo</a>
    — open the session picker, return to prior work, inspect lineage, and use the universal command palette without leaving the keyboard.
  </sub>
</p>

#### Tour the operating surfaces

[spur-tui-tour.webm](https://github.com/user-attachments/assets/9d10be9c-8a48-4511-95cb-82653704d740)

<p align="center">
  <sub>
    <a href="./docs/demos/spur-tui-tour.webm">Watch the 36-second WebM demo</a>
    — move through the command palette, lineage and worker views, sprint browser, plan inspector, and the task detail surfaces used during real work.
  </sub>
</p>

### 3. Orchestrate real work

#### Follow live delegation

[spur-live-delegation.webm](https://github.com/user-attachments/assets/28c0b009-d1e3-4d6c-9856-5b42c666c752)

<p align="center">
  <sub>
    <a href="./docs/demos/spur-live-delegation.webm">Watch the 1:26 WebM demo</a>
    — watch a brain dispatch complementary reviews to multiple vendors, follow each worker in real time, and keep the active fleet visible in one terminal.
  </sub>
</p>

#### Use Kiro as a cross-vendor brain

[spur-kiro-brain-multi-agent.webm](https://github.com/user-attachments/assets/49d70fca-45f9-4dec-a9de-c5c4f5ac1dfe)

<p align="center">
  <sub>
    <a href="./docs/demos/spur-kiro-brain-multi-agent.webm">Watch the 1:59 WebM demo</a>
    — run Kiro as the orchestrator while Gemini, Kimi, and Codex inspect the same code from distinct angles, then bring their outcomes back for consolidation.
  </sub>
</p>

### 4. Explore code with `spur-graph`

[spur-graph-explore-code.webm](https://github.com/user-attachments/assets/95ed7a64-d8ce-444f-8cf5-b9db6890fae9)

<p align="center">
  <sub>
    <a href="./crates/spur-graph/assets/spur-graph-explore-code.webm">Watch the 33-second WebM demo</a>
    — resolve current source, follow consumers, and assemble a concrete impact model from graph-backed evidence. The <a href="./crates/spur-graph/README.md"><code>spur-graph</code> guide</a> covers the underlying pipeline and query surface.
  </sub>
</p>

### 5. Prove constraints with `spur-solver`

[spur-tui-solver.webm](https://github.com/user-attachments/assets/60e745cf-bd86-4345-809c-80d9f1da28f3)

<p align="center">
  <sub>
    <a href="./crates/spur-solver/assets/spur-tui-solver.webm">Watch the 67-second WebM demo</a>
    — translate a real engineering question into constraints, return concrete <code>sat</code> models, and separate correctness gaps from explicit design limits. The <a href="./crates/spur-solver/README.md"><code>spur-solver</code> guide</a> explains the model-finding loop.
  </sub>
</p>

> **DAG-aware planning and integration · 1:25.** The [featured demo above](./docs/demos/spur-dag-aware-multi-agent.webm) shows staged dependencies, parallel branches, plan inspection, and convergence into one review flow.

## Quick start

You need Git and at least one supported coding-agent CLI installed and authenticated.

```sh
npm install -g @getspur/spur-cli

cd your-project
spur init
spur
```

`spur init` discovers available agents and creates the project configuration under `.spur/`. Once the TUI opens, enter a task directly or select one from your project-management integration.

Validate the generated configuration at any time:

```sh
spur config check
```

Prefer not to install globally? Run SPUR through npm:

```sh
npx @getspur/spur-cli tui
```

## How it works

```mermaid
flowchart LR
    task["Task or issue"] --> brain["Brain agent"]
    brain --> plan["DAG plan"]
    plan --> worktrees["Isolated worktrees"]
    worktrees --> worker1["Worker A"]
    worktrees --> worker2["Worker B"]
    worktrees --> worker3["Worker C"]
    worker1 --> review["Review lane"]
    worker2 --> review
    worker3 --> review
    review --> staging["DAG-ordered staging branch"]
```

- The **brain** reasons about the task, chooses workers, and submits a plan.
- **Workers** execute bounded subtasks concurrently in isolated worktrees.
- SPUR records lineage, events, outcomes, and cost while work is running.
- The **review lane** keeps integration human-controlled; rejected work can retry with your feedback.
- Approved commits land on a staging branch in dependency order.

SPUR speaks [Agent Client Protocol](https://github.com/agentclientprotocol/agent-client-protocol) to agents and exposes delegation tools over MCP. It coordinates agents; it does not replace their in-session experience.

## Core capabilities

- **Cross-vendor brain switching.** Move orchestration between vendors when a model rate-limits or a different agent is a better fit.
- **Parallel and DAG-aware delegation.** Dispatch independent work concurrently while preserving dependency order.
- **Structured review and retry.** Every completion becomes a reviewable result with bounded Reflexion-style retries.
- **Local-first durability.** Plans, events, and outcomes remain inspectable on disk and recover after interruptions.
- **Unified analytics.** Read supported vendors' local session data into one DuckDB-backed cost and usage view.
- **Plan mutation.** Split, replace, or amend work while a plan is in flight.
- **Multi-brain safety.** Ownership and session checks prevent two orchestrators from mutating the same plan accidentally.
- **Code-graph context.** Stable symbol IDs, call edges, documentation sections, and incremental indexing support code-aware retrieval.

## Scope

SPUR is an orchestration and review layer for a fleet of coding agents.

It is not an IDE, chat client, CI/CD system, or fully autonomous “set and forget” engineer. The review gate is intentional. SPUR works alongside your editor, project-management tool, and agent subscriptions rather than replacing them.

Native project-management support currently covers [beads](https://github.com/steveyegge/beads) and GitHub Issues through `gh`.

## Configuration

Configuration lives in `.spur/config.toml`. Agent entries describe the executable, transport, role, permissions, cost tier, and delegation hints used by the brain when routing work.

- Start with [`spur init`](./docs/user-docs/00-getting-started.md).
- Browse the [example configuration](./.spur/config.toml.example).
- Read the [configuration guide](./docs/user-docs/05-configuration.md).

## Documentation

| Guide | What it covers |
|---|---|
| [Getting started](./docs/user-docs/00-getting-started.md) | Installation, initialization, and the first task |
| [Configuration](./docs/user-docs/05-configuration.md) | Per-repository agent and runtime settings |
| [Privacy](./docs/PRIVACY.md) | Telemetry tiers, retention, and opt-out controls |
| [OSS boundary](./docs/OSS_BOUNDARY.md) | What belongs in the public product repository |
| [Changelog](./CHANGELOG.md) | Unreleased and shipped changes |

## Development

Clone the repository, then use the workspace wrapper for Rust commands:

```sh
git clone https://github.com/getspur/SpurCode.git spur
cd spur

scripts/spur-cargo build --workspace
scripts/spur-cargo test --workspace
scripts/spur-cargo clippy --workspace -- -D warnings
scripts/spur-cargo fmt --all
```

See [CONTRIBUTING.md](./CONTRIBUTING.md) for test tiers and contribution conventions.

<details>
<summary><strong>Workspace map</strong></summary>

| Crate | Responsibility |
|---|---|
| `spur-cli` | Binary entry point and CLI commands |
| `spur-core` | Orchestration, review, lineage, and the event pipeline |
| `spur-acp` | ACP clients, transports, capabilities, and event types |
| `spur-tui` | The `ratatui` terminal interface |
| `spur-mcp` | Delegation tools exposed to brain agents |
| `spur-context` | DuckDB analytics and agent-log extractors |
| `spur-cost` | Pricing registry and session ledger |
| `spur-graph` | Tree-sitter code graph and stable symbol identities |
| `spur-analyst` | DuckDB-backed graph and documentation analysis |
| `spur-worktree` | Worktree isolation, liveness, and cleanup |
| `spur-pm` | Project-management adapters |
| `spur-interactive` | Frontend bridge for non-TUI clients |
| `spur-bot` | Telegram frontend |
| `spur-blob-store` | Content-addressed delegation outcomes |
| `spur-license` / `spur-license-admin` | License and feature-key registry |

</details>

## Telemetry and privacy

Tier 1 crash diagnostics and performance telemetry are on by default; Tier 2 usage telemetry is opt-in.

Disable telemetry for one run:

```sh
SPUR_TELEMETRY=0 spur
```

Or disable it persistently:

```sh
spur telemetry disable all
```

See the [privacy documentation](./docs/PRIVACY.md) for collected fields, retention, and deletion steps.

## Community and license

Issues, feature requests, and pull requests are welcome. Start with [CONTRIBUTING.md](./CONTRIBUTING.md), open a [GitHub issue](https://github.com/getspur/SpurCode/issues), or send product feedback through [getspur.dev/feedback](https://getspur.dev/feedback).

SPUR is licensed under the [GNU General Public License v3.0 only](./LICENSE) (`GPL-3.0-only`). The Community tier requires no license key; paid feature entitlements are separate from the source license.
