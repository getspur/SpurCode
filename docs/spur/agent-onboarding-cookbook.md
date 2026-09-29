# Agent Onboarding Cookbook

This guide walks through adding a new agent to spur in under 10 minutes.
No Rust required when the agent uses an existing transport + dispatch combination.

## Quick start

Open `.spur/config.toml`, add a `[[agents.entries]]` block, then run `spur config check` to validate.

If your agent is one spur knows about (kiro, claude-code, claude-code-acp, codex, gemini, opencode, kimi, grok, goose, pi), `spur init` writes a matching block automatically when the binary is on `$PATH`. **`spur init` overwrites `.spur/config.toml`** — if you have customizations, edit by hand instead.

## Decision tree

### Step 1 — Choose a transport

| Your agent… | `transport =` | Example |
|---|---|---|
| Speaks ACP natively (has `--acp` or an ACP wrapper) | `"acp"` | kiro (`kiro-cli acp`), codex |
| Emits Claude-style stream-json on stdout | `"stream-json"` | `claude -p --output-format stream-json …` |
| Accepts a prompt on stdin, emits text on stdout | `"stdio"` | (rare — custom integrations) |
| Is a plain CLI that takes a prompt as trailing arg | `"cli-wrap"` | `gemini "your prompt here"` |

**How do I know which my agent speaks?** Run it with `--help`. If you see an ACP flag, pick `"acp"`. If the docs mention stream-json, pick `"stream-json"`. If it accepts a prompt as a positional argument, `"cli-wrap"`. When in doubt start with `"cli-wrap"` — it's the simplest and works for most CLIs.

### Step 1b — Choose an AgentKind

`kind` tells the TUI's adapter layer which per-agent rendering rules to apply (tool-family classification, observe-payload extraction, mode-badge vocabulary, signature tint). It is **orthogonal to `transport`** — multiple kinds can share the same transport.

```toml
kind = "claude-code-acp"   # or one of the values below
```

| If your agent is… | `kind =` |
|---|---|
| Claude Code via `claude -p --output-format stream-json` | `"claude-stream-json"` |
| Claude Code via `@agentclientprotocol/claude-agent-acp` | `"claude-code-acp"` |
| Codex via `@agentclientprotocol/codex-acp@1.9.0` (global install or npx) | `"codex-acp"` |
| Kiro CLI (`kiro-cli acp`) | `"kiro"` |
| Gemini CLI (`gemini --acp`) | `"gemini"` |
| xAI Grok Build CLI (`grok agent stdio`) | `"grok"` |
| Pi coding agent via `pi-acp` | `"pi"` |
| Anything else | `"generic"` (this is also the default when the field is omitted) |

`"generic"` applies heuristic fallbacks (case-insensitive title matching, ACP `ToolKind` passthrough). Your agent will work fine — you'll just get generic glyphs and no mode-badge translation. File an issue if your agent's tool vocabulary is widely used and deserves a dedicated `AgentKind` variant.

**Don't infer — declare.** spur does not try to guess `kind` from `command`/`args` substrings. An explicit value in the TOML is cheap, reviewable, and immune to upstream rename.

### Step 2 — Choose a dispatch

| Your agent… | `[commands]` |
|---|---|
| Accepts `/slash` commands as ordinary prompt text | `dispatch = "prompt_text"` |
| Has a vendor-ext RPC that receives structured commands | `dispatch = "vendor_exec"` + `exec_method = "…"` |

Most agents use `prompt_text`. Use `vendor_exec` only if your agent exposes a custom JSON-RPC method for commands and you've verified it works in your agent's current release.

### Step 3 — Add static commands (optional)

Static commands are slash commands that appear in the `/` popup before your agent connects. Useful when your agent doesn't expose a discovery endpoint.

```toml
[[agents.entries.commands.static]]
name = "compact"
description = "Compact conversation history"

[[agents.entries.commands.static]]
name = "model"
description = "Switch model"
hint = "[model-name]"
```

If your agent later advertises commands at runtime via an `ingest` binding, the dynamic entries override statics on `(handle, name)` match.

### Step 4 — Add permissions (optional)

If your agent has a bypass-permissions flag or session mode:

```toml
[agents.entries.permissions]
args = ["--trust-all-tools"]         # CLI flag appended at spawn
session_mode = "bypassPermissions"   # ACP session mode set after new_session
# skip = true                        # uncomment to enable bypass
```

Both fields are applied when `skip = true` is explicit. Declared-but-not-enabled (the default) is safety-by-default — spur never auto-bypasses permissions unless you opt in.

### Step 5 — Validate config shape

```
$ spur config check
✓ my-agent
```

This checks your agent block parses cleanly and the `vendor_exec` / `prompt_text` choices are consistent (e.g. `vendor_exec` requires `exec_method`). It does **not** boot your agent.

### Step 6 — Test at runtime

```
$ spur chat my-agent "say hello"
```

This launches the agent, opens a session, and sends one prompt. If it hangs or errors, the most likely causes are (a) wrong `command` / `args` — the binary can't be found or doesn't accept those flags; (b) wrong `transport` — the binary speaks a different dialect. Check the terminal for error output; spur logs to `.spur/logs/` for post-mortem.

### Step 7 — Probe native ACP capabilities

Before adding an ACP agent to the seed template, capture the handshake that
SPUR will see. Pass the agent's complete argv as one shell-quoted `--args`
string; the default does not send a billed prompt:

```bash
python3 scripts/probe_acp_capabilities.py \
  --command my-agent --args "acp --stdio" --label my-agent
```

The command writes full NDJSON frames plus a structured report under
`.spur/logs/probe-my-agent-<timestamp>.*`. In `matrix`, check
`config_options_advertised`, `modes_advertised`, command discovery, and observed
update/extension variants. Matrix `model_select_advertised` and
`effort_select_advertised` report configOptions selects exactly as advertised on
the wire. `spur_slash_model` and `spur_slash_effort` instead predict SPUR's
product surface, including proven proprietary DirectSetModel catalogs for Grok
and Kiro (and Grok's proprietary effort metadata) without fabricating
configOptions.

Vendor extension notifications (e.g. Kiro `_kiro.dev/commands/available`, Grok
`_x.ai/*`) are always harvested into `vendor_notifications` with **full
payloads**, a de-duplicated `commands_catalog`, and any
`meta.optionsMethod` targets. To actively probe those methods plus
`session/set_mode` / `session/set_model` when proprietary planes advertise
values:

```bash
python3 scripts/probe_acp_capabilities.py \
  --command kiro-cli --args acp --label kiro \
  --probe-vendor-rpc --always-approve
```

RPC failures are soft (recorded under `vendor_rpc_results` / matrix
`vendor_rpc_errors`); the probe still exits 0 when the handshake succeeded.
Add extra methods with repeatable `--vendor-method '_x.ai/sessionConfig/update'`.
Add `--prompt "Reply with exactly: pong"` only when you need billed
`session/update` evidence; use `--try-set-model` to check the legacy model RPC
as a separate opt-in.

## Worked example: codex

```toml
[[agents.entries]]
name = "codex"
command = "codex"
args = ["--acp"]
transport = "acp"
role = "both"
cost_tier = "low"

[agents.entries.display]
handle = "codex"

[agents.entries.commands]
dispatch = "prompt_text"

[[agents.entries.commands.static]]
name = "compact"
description = "Compact conversation history"
```

Zero Rust. `/compact` appears in the popup immediately — submitting it sends `"/compact"` as a plain text prompt to codex.

### Codex worker profile activation

Codex ACP 1.9.0 advertises model, reasoning-effort, and collaboration options,
but it does not advertise a profile option for the already-open primary
session. A TUI `@worker` mention is advisory context for the brain, not an ACP
request and not an automatic dispatch. For example:

```text
worker://codex?agent=reviewer&model=gpt-5.6-sol&effort=xhigh
```

maps to SPUR delegation arguments as follows:

| Mention component | Delegation argument |
|---|---|
| Worker URI name `codex` | `agent = "codex"` |
| URI query slot `agent=reviewer` | `profile = "reviewer"` |
| `model=gpt-5.6-sol` | `model = "gpt-5.6-sol"` |
| `effort=xhigh` | `effort = "xhigh"` |

SPUR materializes the named profile for a fresh SPUR-managed worker and starts
that worker with the requested model and effort. This does not mutate or switch
the current brain session. To change the primary brain's startup profile, open
a new connection/session with the desired startup configuration.

Codex 0.153.2 can load role files for newly spawned Codex-native children, but
the TUI mention does not invoke that native spawn path, and SPUR does not enable
`multi_agent_v2` by default. Keeping mention activation on the SPUR-managed
worker route preserves SPUR isolation and review semantics as well as the
explicit model/effort request. The mention remains a preference, not an
override: normal delegation suitability checks still apply.

## Worked example: Grok Build CLI

```toml
[[agents.entries]]
name = "grok"
command = "grok"
args = ["--no-auto-update", "agent", "stdio"]
# Optional spawn-time model pin (insert before `stdio`):
# args = ["--no-auto-update", "agent", "--model", "grok-build", "stdio"]
transport = "acp"
kind = "grok"
role = "both"
cost_tier = "medium"

[agents.entries.display]
handle = "grok"

[agents.entries.commands]
dispatch = "prompt_text"
```

**Important — verify the ACP planes.** SPUR synthesizes mid-session `/model`
from a matching configOptions select or Grok's non-empty proprietary model
catalog, and `/effort` from configOptions or Grok's proven effort metadata.
Codex also gets `/fast` only when it advertises a non-empty select with
`id = "fast-mode"`; selections use `configId = "fast-mode"` with
`session/set_config_option`.
Grok's own TUI also has local commands that are not ACP evidence. Before
assuming model switching works under SPUR, run:

```bash
python3 scripts/probe_acp_capabilities.py \
  --command grok --args "--no-auto-update agent --always-approve stdio" \
  --label grok --always-approve
```

See `docs/superpowers/specs/2026-07-13-grok-acp-capability-probe-results.md`.
That report retains Grok-specific `_meta` interpretation; the generic harness
records vendor-neutral meta planes and the same compact SPUR capability matrix.
Do not add a static `[[commands.static]] name = "model"` unless you have
verified Grok honors `/model …` as prompt text — a static entry does not call
`session/set_config_option`.

## Worked example: Pi coding agent (`pi-acp`)

```toml
[[agents.entries]]
name = "pi"
command = "pi-acp"
args = []
transport = "acp"
kind = "pi"
role = "both"
cost_tier = "medium"

[agents.entries.display]
handle = "pi"

[agents.entries.commands]
dispatch = "prompt_text"
```

`pi-acp` spawns `pi --mode rpc`. Install both (`@earendil-works/pi-coding-agent@0.85.1`
and `pi-acp@0.0.33`). Live probe 2026-09-17 advertised `configOptions` `model` and
`thought_level`, so SPUR synthesizes `/model` and `/effort` — do not add a static
`model` command. For Z.AI Coding Plan, reuse OpenCode's `zai-coding-plan` key into
`~/.pi/agent/auth.json` under `zai`; see `docs/onboarding/pi-zai.md`.

For SPUR MCP tools (`code_*`, analyst) inside pi: `pi install npm:pi-mcp-adapter`
plus a project `.pi/mcp.json` pointing at the standalone stdio servers
(`spur graph mcp`, `spur analyst mcp`, `spur mcp`). The adapter is lazy — one
proxy tool, servers spawn on first use. pi-acp itself still does not forward
ACP `mcpServers` (accepted but dropped), so ACP-delegated MCP remains
unavailable.

## Worked example: a hypothetical `my-agent`

Suppose `my-agent` is a Python CLI that accepts a prompt on stdin and emits text on stdout. It has no slash-command machinery, no ACP support.

```toml
[[agents.entries]]
name = "my-agent"
command = "my-agent"
args = ["--stream"]
transport = "stdio"
role = "worker"
cost_tier = "medium"

[agents.entries.display]
handle = "my"

[agents.entries.commands]
dispatch = "prompt_text"

[[agents.entries.commands.static]]
name = "help"
description = "Print agent help"
```

`spur chat my-agent "refactor foo.py"` now works. Users can type `/help` to send the literal text `/help` to the agent.

## Adding your agent to the seed template

Once your config works, contribute it back so other spur users get it via `spur init`:

1. Add an `[[agents.entries]]` block to `crates/spur-acp/src/seed_agents.toml`.
2. Add an entry to `INSTALL_HINTS` in `crates/spur-cli/src/main.rs` — a terse one-liner telling new users how to install your agent. Example: `("my-agent", "brew install my-agent")`. A contract test in `crates/spur-cli/tests/init_ux.rs` enforces parity between the seed template and `INSTALL_HINTS`.
3. Run `cargo test -p spur-acp config::tests::seed_template` and `cargo test -p spur-cli --test init_ux` to confirm parse + contract checks still pass.
4. Open a PR. No other Rust changes are required.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `spur init` didn't register my agent | Binary not on `$PATH` (seed template only) — add entry manually. |
| Config check fails "vendor_exec requires exec_method" | Add `exec_method = "..."` under `[commands]`. |
| `/command` does nothing | `dispatch` mismatch — check your agent expects prompt_text or vendor_exec. |
| Agent errors "unknown flag: `--acp`" | Wrong `args` for your installed version — check `--help` and update. |
| `--trust-all-tools` (or similar bypass flag) seems to be ignored | You probably forgot to set `skip = true` under `[permissions]`. Declaring bypass args isn't enough — spur requires explicit opt-in. |
| No `/model` or `/effort` on an ACP session | Agent did not advertise matching non-empty select `configOptions` on `session/new`. Probe with `scripts/probe_acp_capabilities.py`; pin the model at spawn until the agent advertises options. |

## Further reading

- `docs/spur/agent-config.md` — full schema reference.
- `docs/superpowers/specs/2026-04-14-agent-onboarding-roadmap.md` — design context.
- `docs/superpowers/specs/2026-04-14-spec2-agent-command-surface.md` — command dispatch generalization.
- `docs/superpowers/specs/2026-04-14-spec3-seed-template-codex-onboarding.md` — seed template & codex onboarding.
