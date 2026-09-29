use anyhow::Result;
use spur_acp::config::{AgentConfig, ContextServiceConfig, SpurConfig};
use spur_acp::types::AgentRole;
use spur_core::Orchestrator;
use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

/// User-facing install commands per seed agent. Surfaced by `spur init`
/// when a seed agent's binary is not on $PATH. Kept here (not in the
/// schema) because hints are onboarding copy — they don't belong in
/// every user's round-tripped `.spur/config.toml`.
///
/// Contract: every agent in `spur_acp::config::load_seed_template()`
/// must have an entry here. Enforced by `tests/init_ux.rs`.
pub const INSTALL_HINTS: &[(&str, &str)] = &[
    ("kiro", "brew install kiro-cli"),
    (
        "claude-code",
        "npm install -g npx   # then re-run `spur init`",
    ),
    (
        "codex-bin",
        "npm install -g @agentclientprotocol/codex-acp@1.9.0",
    ),
    ("codex", "npx @agentclientprotocol/codex-acp@1.9.0"),
    ("gemini", "npm install -g @google/gemini-cli"),
    ("opencode", "npm install -g opencode"),
    (
        "goose",
        "curl -fsSL https://github.com/block/goose/raw/main/install.sh | bash   # then `goose configure`",
    ),
    ("kimi", "see docs/spur/agent-onboarding-cookbook.md"),
    (
        "grok",
        "curl -fsSL https://x.ai/cli/install.sh | bash   # then `grok login`",
    ),
    (
        "pi",
        "npm i -g --ignore-scripts @earendil-works/pi-coding-agent@0.85.1 && npm i -g pi-acp@0.0.33 && pi install npm:pi-mcp-adapter",
    ),
];

pub fn install_hint(name: &str) -> &'static str {
    INSTALL_HINTS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, h)| *h)
        .unwrap_or("see docs/spur/agent-onboarding-cookbook.md")
}

/// Initialize SPUR: converge local config with the environment.
///
/// Philosophy: `spur init` is a config convergence tool, not a one-shot
/// writer. It is safe to run multiple times.
///
/// - Discovers agents on `$PATH` and merges them into existing config.
/// - Preserves user customizations (capabilities, review policy, delegation).
/// - Preserves all non-agent config (bot, pm, project, cost, worktree, …).
/// - Recomputes brain / fallback based on the merged agent list.
/// - Walks an interactive human through three yes/no install steps —
///   config, PM tracker, skills — defaulting each to Yes.
/// - Validates before persisting.
///
/// Non-interactive runs (no TTY, e.g. CI or `git clone → spur init`) and
/// `--yes` assume Yes for all three steps, preserving the auto-everything
/// behavior automation depends on.
///
/// `--force` resets the agent list to discovered-only (drops manually-added
/// agents) while still preserving non-agent sections.
#[allow(clippy::fn_params_excessive_bools)]
pub async fn run(
    repo_root: PathBuf,
    global: bool,
    force: bool,
    with_skills: bool,
    assume_yes: bool,
) -> Result<()> {
    let config_path = if global {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".spur").join("config.toml"))
            .ok_or_else(|| anyhow::anyhow!("could not resolve home directory for --global"))?
    } else {
        repo_root.join(".spur").join("config.toml")
    };
    let config_label = if global {
        "~/.spur/config.toml"
    } else {
        ".spur/config.toml"
    };

    // ── Phase 1: Environment discovery ─────────────────────────────────
    println!("[spur] Scanning agents on $PATH...");
    let mut orch = Orchestrator::new(repo_root.clone(), SpurConfig::default(), None)?;
    let found_names = orch.init_agents().await?;
    let seed = spur_acp::config::load_seed_template();

    let seed_order: Vec<&str> = seed.entries.iter().map(|e| e.name.as_str()).collect();
    let mut discovered: Vec<AgentConfig> = orch.registry.list().into_iter().cloned().collect();
    discovered.sort_by_key(|a| {
        seed_order
            .iter()
            .position(|&s| s == a.name.as_str())
            .unwrap_or(usize::MAX)
    });
    let discovered_names: std::collections::HashSet<_> = found_names.iter().cloned().collect();

    // ── Phase 2: Load the target layer over its effective baseline ─────
    let baseline = if global {
        toml::Value::try_from(SpurConfig::default())?
    } else {
        spur_acp::config::layered::default_user_baseline(&repo_root)?
    };
    let (mut config, existed_before) = load_or_default_config(&config_path, &baseline)?;
    let mut config_sources = vec![config_path.clone()];
    if !global {
        if let Some(base) = directories::BaseDirs::new() {
            config_sources.push(base.home_dir().join(".spur/config.toml"));
        }
    }
    let (mut default_is_explicit, mut fallback_is_explicit) = (false, false);
    for source in config_sources.into_iter().filter(|path| path.exists()) {
        let raw = std::fs::read_to_string(&source)
            .map_err(|err| anyhow::anyhow!("failed to read {}: {err}", source.display()))?;
        let value: toml::Value = toml::from_str(&raw)
            .map_err(|err| anyhow::anyhow!("failed to parse {}: {err}", source.display()))?;
        let brain = value.get("brain");
        default_is_explicit |= brain.and_then(|section| section.get("default")).is_some();
        fallback_is_explicit |= brain.and_then(|section| section.get("fallback")).is_some();
    }

    // ── Phase 3: Agent convergence ─────────────────────────────────────
    config.agents.entries = merge_agents(&config.agents.entries, &discovered, force);

    // ── Phase 4: Recompute brain & fallback ────────────────────────────
    recompute_brain_and_fallback(&mut config, default_is_explicit, fallback_is_explicit);

    // ── Phase 5: Display discovery results ─────────────────────────────
    println!();
    for agent in &seed.entries {
        if discovered_names.contains(&agent.name) {
            println!("  ✓ {}", agent.name);
        } else {
            println!(
                "  ✗ {:<18}install: {}",
                agent.name,
                install_hint(&agent.name)
            );
        }
    }

    // ── Phase 6: No-agent early exit ───────────────────────────────────
    // Nothing to configure and no prior config to converge — point the user
    // at the install hints and stop before any prompts.
    if config.agents.entries.is_empty() && !existed_before {
        println!();
        println!("No agents found. Install one of the above and re-run `spur init`.");
        return Ok(());
    }

    // ── Step 1 of 3: Config ────────────────────────────────────────────
    // Each step asks an interactive human; a non-TTY run or `--yes` assumes
    // Yes (see `confirm`). The brain / bot / permission sub-prompts only run
    // when the user opts to write the config.
    let agent_count = config.agents.entries.len();
    let config_written = if confirm(
        &format!("Write {config_label} with {agent_count} detected agent(s)?"),
        assume_yes,
    ) {
        // Brain selection (interactive only in TTY).
        if std::io::stdin().is_terminal() {
            if let Err(e) = prompt_default_brain_selection(&mut config) {
                eprintln!("[spur] default brain prompt failed: {e}; continuing");
            }
        }

        // Telegram bot setup (interactive only in TTY). Compiled out unless
        // the `telegram-bot` feature is enabled.
        #[cfg(feature = "telegram-bot")]
        if std::io::stdin().is_terminal() {
            if let Err(e) = maybe_prompt_bot_setup(&mut config) {
                eprintln!("[spur] bot setup prompt failed: {e}; continuing");
            }
        }

        // Validate before write.
        if let Err(e) = validate_all_agents(&config) {
            eprintln!("[spur] config validation failed: {e}");
            return Err(e);
        }

        // Permission-bypass safety prompt (interactive only in TTY).
        if std::io::stdin().is_terminal() {
            if let Err(e) = prompt_permission_bypass(&mut config) {
                eprintln!("[spur] permission bypass prompt failed: {e}; continuing");
            }
        }

        // Atomic persist.
        let full = toml::Value::try_from(&config)?;
        let mut sparse = spur_acp::config::layered::sparse_diff(&full, &baseline);
        expose_default_context_service_section(&mut sparse, &baseline)?;
        std::fs::create_dir_all(config_path.parent().unwrap())?;
        std::fs::write(&config_path, toml::to_string_pretty(&sparse)?)?;
        true
    } else {
        println!("[spur] skipped writing {config_label}.");
        false
    };

    // ── Phase 4b: Pi MCP wiring ──────────────────────────────────────
    // pi-acp drops ACP mcpServers, so pi reaches SPUR's standalone stdio
    // MCP servers through the pi-mcp-adapter extension + .pi/mcp.json.
    // Idempotent; never overwrites user entries.
    match materialize_pi_mcp_config(&repo_root, &config) {
        Ok(true) => println!("  ✓ .pi/mcp.json (SPUR MCP servers for pi)"),
        Ok(false) => {}
        Err(e) => eprintln!("  [spur] warning: pi MCP wiring failed: {e}"),
    }

    // ── Step 2 of 3: PM tracker ────────────────────────────────────────
    // Idempotent: an existing `.beads/` needs no work, so we don't prompt.
    // Bootstrapping here makes the golden-path "git clone → spur init"
    // produce a working tracker without a second command.
    if !repo_root.join(".beads").exists()
        && confirm("Bootstrap the beads issue tracker (.beads/)?", assume_yes)
    {
        println!();
        println!("[spur] bootstrapping tracker...");
        if let Err(e) = run_pm_init(repo_root.clone()).await {
            eprintln!("[spur] warning: pm init failed: {e}");
            // Do not return Err — config (init's primary contract) is handled
            // separately above. The user can re-run `spur pm init` directly.
        }
    }

    // ── Step 3 of 3: Skills install ────────────────────────────────────
    // `--with-skills` forces the full fanout (all adapters) and implies Yes.
    // The default path is filtered: only adapters whose agent was discovered
    // on `$PATH` (plus `SpurHermetic` for brain prompt injection).
    if with_skills {
        if let Err(e) = run_skills_init(&repo_root) {
            eprintln!("[spur] warning: skills install failed: {e}");
        }
    } else if !config.agents.entries.is_empty() && confirm("Install SpurPower skills?", assume_yes)
    {
        let allowed = adapters_for_discovered_agents(&discovered_names);
        if let Err(e) = run_skills_init_filtered(&repo_root, &allowed) {
            eprintln!("[spur] warning: skills install failed: {e}");
        }
    }

    // ── Summary ────────────────────────────────────────────────────────
    print_summary(&config, config_written, config_label);

    Ok(())
}

// ── Pi MCP wiring ───────────────────────────────────────────────

/// SPUR standalone stdio MCP servers exposed to pi via the
/// `pi-mcp-adapter` extension. `command` stays on $PATH (not an absolute
/// path) so a committed `.pi/mcp.json` stays portable across machines.
const PI_MCP_SERVERS: &[(&str, &str, &[&str])] = &[
    ("spur-graph", "spur", &["graph", "mcp"]),
    ("spur-analyst", "spur", &["analyst", "mcp"]),
    ("spur-solver", "spur", &["solver", "mcp"]),
    ("spur", "spur", &["mcp"]),
];

/// Idempotently ensure `.pi/mcp.json` wires SPUR's standalone stdio MCP
/// servers for any pi agent in the merged config.
///
/// pi has no built-in MCP and pi-acp drops ACP `mcpServers` (accepted but
/// never forwarded), so the `pi-mcp-adapter` extension + this project
/// config file are the supported bridge. Servers are lazy — they spawn on
/// first tool use, so idle context cost is one proxy tool.
///
/// Merge semantics: existing user/server entries are preserved verbatim;
/// SPUR entries are added when missing and left untouched when present
/// (user overrides like `disabled: true` or extra args win). Returns
/// `true` when the file was created or modified.
fn materialize_pi_mcp_config(repo_root: &std::path::Path, config: &SpurConfig) -> Result<bool> {
    let has_pi = config
        .agents
        .entries
        .iter()
        .any(|a| a.kind == spur_acp::types::AgentKind::Pi);
    if !has_pi {
        return Ok(false);
    }

    let path = repo_root.join(".pi").join("mcp.json");
    let existing: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse {} — fix or remove it and re-run `spur init`: {e}",
                path.display()
            )
        })?,
        Err(_) => serde_json::json!({}),
    };

    let mut root = existing;
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let servers = root
        .as_object_mut()
        .expect("root is an object")
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        anyhow::bail!(
            "{} has a non-object `mcpServers` — fix or remove it and re-run `spur init`",
            path.display()
        );
    }
    let server_map = servers.as_object_mut().expect("checked above");

    let mut added: Vec<&str> = Vec::new();
    for (name, command, args) in PI_MCP_SERVERS {
        // Only insert when absent — user overrides (e.g. `disabled: true`
        // or extra args) are preserved verbatim.
        if !server_map.contains_key(*name) {
            server_map.insert(
                (*name).to_string(),
                serde_json::json!({ "command": command, "args": args }),
            );
            added.push(name);
        }
    }

    if !added.is_empty() {
        std::fs::create_dir_all(path.parent().expect(".pi parent"))
            .map_err(|e| anyhow::anyhow!("failed to create .pi/: {e}"))?;
        let mut json = serde_json::to_string_pretty(&root)?;
        json.push('\n');
        std::fs::write(&path, json)
            .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", path.display()))?;
    }

    Ok(!added.is_empty())
}

/// Ask a yes/no question, defaulting to Yes. Returns Yes without prompting
/// when `assume_yes` (`--yes`) is set or stdin is not a TTY (CI, piped
/// input, `git clone → spur init`), so non-interactive runs keep performing
/// all of init's setup automatically.
fn confirm(question: &str, assume_yes: bool) -> bool {
    if assume_yes || !std::io::stdin().is_terminal() {
        return true;
    }
    print!("{question} [Y/n] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => true, // EOF / read error → default Yes
        Ok(_) => interpret_yes_no(&line),
    }
}

/// Interpret a yes/no answer with Yes as the default: empty input is Yes, and
/// only an explicit "n" / "no" (case-insensitive) is No.
fn interpret_yes_no(input: &str) -> bool {
    !matches!(input.trim().to_ascii_lowercase().as_str(), "n" | "no")
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn load_or_default_config(
    path: &std::path::Path,
    baseline: &toml::Value,
) -> Result<(SpurConfig, bool)> {
    let mut merged = baseline
        .as_table()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("config baseline is not a TOML table"))?;
    let existed_before = path.exists();
    if existed_before {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
        let layer: toml::Value = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))?;
        let table = layer
            .as_table()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{} is not a TOML table", path.display()))?;
        spur_acp::config::merge_tables(&mut merged, table);
    }
    let config = toml::Value::Table(merged)
        .try_into()
        .map_err(|e| anyhow::anyhow!("failed to build config for {}: {e}", path.display()))?;
    Ok((config, existed_before))
}

fn expose_default_context_service_section(
    sparse: &mut toml::Value,
    baseline: &toml::Value,
) -> Result<()> {
    if sparse.get("context_service").is_some() {
        return Ok(());
    }

    let default_context_service = toml::Value::try_from(ContextServiceConfig::default())?;
    if baseline.get("context_service") != Some(&default_context_service) {
        return Ok(());
    }

    if let Some(table) = sparse.as_table_mut() {
        table.insert("context_service".to_owned(), default_context_service);
    }
    Ok(())
}

fn merge_agents(
    existing: &[AgentConfig],
    discovered: &[AgentConfig],
    force: bool,
) -> Vec<AgentConfig> {
    if force {
        // Reset to discovered-only, but preserve seed ordering.
        return discovered.to_vec();
    }

    // Start with existing agents (preserves user customizations).
    let mut merged: HashMap<String, AgentConfig> = existing
        .iter()
        .map(|a| (a.name.clone(), a.clone()))
        .collect();

    // Overlay newly discovered agents that aren't already present.
    for disc in discovered {
        if !merged.contains_key(&disc.name) {
            merged.insert(disc.name.clone(), disc.clone());
        }
    }

    // Sort by seed order for deterministic output.
    let seed = spur_acp::config::load_seed_template();
    let seed_order: Vec<&str> = seed.entries.iter().map(|e| e.name.as_str()).collect();
    let mut result: Vec<AgentConfig> = merged.into_values().collect();
    result.sort_by_key(|a| {
        seed_order
            .iter()
            .position(|&s| s == a.name.as_str())
            .unwrap_or(usize::MAX)
    });
    result
}

fn recompute_brain_and_fallback(
    config: &mut SpurConfig,
    default_is_explicit: bool,
    fallback_is_explicit: bool,
) {
    let entries = &config.agents.entries;

    let selected_brain_is_valid = default_is_explicit
        && entries.iter().any(|agent| {
            agent.name == config.brain.default
                && matches!(agent.role, AgentRole::Brain | AgentRole::Both)
        });
    let brain_name = if selected_brain_is_valid {
        config.brain.default.clone()
    } else {
        entries
            .iter()
            .find(|a| {
                a.name == "claude-code" && matches!(a.role, AgentRole::Brain | AgentRole::Both)
            })
            .or_else(|| {
                entries
                    .iter()
                    .find(|a| matches!(a.role, AgentRole::Brain | AgentRole::Both))
            })
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                if !entries.is_empty() {
                    println!();
                    println!(
                        "  (note: no brain-capable agents registered; using {} as brain)",
                        entries[0].name
                    );
                    entries[0].name.clone()
                } else {
                    String::new()
                }
            })
    };

    let configured_fallback_is_valid = fallback_is_explicit
        && config.brain.fallback.iter().all(|name| {
            name != &brain_name
                && entries.iter().any(|agent| {
                    agent.name == *name && matches!(agent.role, AgentRole::Brain | AgentRole::Both)
                })
        });
    let fallbacks: Vec<String> = if configured_fallback_is_valid {
        config.brain.fallback.clone()
    } else {
        entries
            .iter()
            .filter(|a| {
                a.name != brain_name && matches!(a.role, AgentRole::Brain | AgentRole::Both)
            })
            .map(|a| a.name.clone())
            .collect()
    };

    config.brain.default = brain_name;
    config.brain.fallback = fallbacks;
}

fn prompt_default_brain_selection(config: &mut SpurConfig) -> Result<()> {
    let brain_agents: Vec<String> = config
        .agents
        .entries
        .iter()
        .filter(|a| matches!(a.role, AgentRole::Brain | AgentRole::Both))
        .map(|a| a.name.clone())
        .collect();

    if brain_agents.is_empty() {
        return Ok(());
    }

    let default_index = brain_agents
        .iter()
        .position(|name| name == &config.brain.default)
        .unwrap_or(0);

    println!();
    println!(
        "Select default brain [default: {}]:",
        brain_agents[default_index]
    );
    for (idx, name) in brain_agents.iter().enumerate() {
        println!("  {}) {}", idx + 1, name);
    }
    eprint!("> ");
    std::io::stderr().flush()?;

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let selected = match line.trim() {
        "" => brain_agents[default_index].clone(),
        input => match input.parse::<usize>() {
            Ok(choice) if (1..=brain_agents.len()).contains(&choice) => {
                brain_agents[choice - 1].clone()
            }
            _ => {
                println!(
                    "Invalid selection; keeping {}.",
                    brain_agents[default_index]
                );
                brain_agents[default_index].clone()
            }
        },
    };

    config.brain.default = selected.clone();
    config.brain.fallback = brain_agents
        .into_iter()
        .filter(|name| name != &selected)
        .collect();

    Ok(())
}

#[cfg(feature = "telegram-bot")]
fn maybe_prompt_bot_setup(config: &mut SpurConfig) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        return Ok(());
    }

    let already_enabled = config.bot.telegram.enabled;

    println!();
    if already_enabled {
        println!("Telegram bot is already configured (enabled = true).");
        println!("Reconfigure? [y/N]");
    } else {
        println!("SPUR can run a Telegram bot for remote interaction.");
        println!("Configure it now? [y/N]");
    }
    eprint!("> ");
    std::io::stderr().flush()?;

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    if !line.trim().eq_ignore_ascii_case("y") {
        return Ok(());
    }

    // Prompt for operator_user_id
    println!();
    println!("How to get your Telegram numeric user ID:");
    println!();
    println!("  1. SIMPLEST: Message @userinfobot and copy the 'id' number.");
    println!();
    println!("  2. VIA API: If @userinfobot is unavailable, send a DM to your");
    println!("     bot, then run:");
    println!("       curl \"https://api.telegram.org/bot<TOKEN>/getUpdates\"");
    println!("     and read result[0].message.from.id.");
    println!();
    println!("     NOTE: if spur bot telegram is already running, it consumes");
    println!("     updates via long polling — getUpdates may return empty.");
    println!("     Stop the bot first, and if you ever used webhooks, clear");
    println!("     them with:");
    println!("       curl \"https://api.telegram.org/bot<TOKEN>/deleteWebhook?drop_pending_updates=false\"");
    println!();
    println!("Enter your operator_user_id:");
    eprint!("> ");
    std::io::stderr().flush()?;

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let operator_id = line.trim().parse::<i64>().ok();

    // Token guidance — never prompt for the secret itself.
    println!();
    println!("Bot token: set SPUR_TELEGRAM_BOT_TOKEN as an environment variable");
    println!("(get your token from @BotFather). Do NOT paste it here — it would");
    println!("be stored in .spur/config.toml which is not gitignored by default.");
    println!();
    println!("Once the env var is set, start the bot with:");
    println!("  spur bot telegram");

    config.bot.telegram.enabled = true;
    if let Some(id) = operator_id {
        config.bot.telegram.operator_user_id = Some(id);
    }

    Ok(())
}

fn validate_all_agents(config: &SpurConfig) -> Result<()> {
    use spur_acp::validate_agent_config;

    let mut fatal = 0_usize;
    let mut _warn = 0_usize;

    for entry in &config.agents.entries {
        match validate_agent_config(entry) {
            Ok(()) => {}
            Err(errors) => {
                for e in errors {
                    if e.is_fatal() {
                        eprintln!("  \u{2717} {}: {}", entry.name, e);
                        fatal += 1;
                    } else {
                        eprintln!("  \u{26a0} {}: {}", entry.name, e);
                        _warn += 1;
                    }
                }
            }
        }
    }

    if fatal > 0 {
        Err(anyhow::anyhow!("{fatal} fatal agent validation error(s)"))
    } else {
        Ok(())
    }
}

fn print_summary(config: &SpurConfig, config_written: bool, config_label: &str) {
    let any_bypass = config
        .agents
        .entries
        .iter()
        .any(|a| a.effective_permissions().skip);
    let bypass_str = if any_bypass {
        "enabled for some agents — review .spur/config.toml"
    } else {
        "disabled (safety-default)"
    };
    let fallback_str = if config.brain.fallback.is_empty() {
        "none".to_string()
    } else {
        config.brain.fallback.join(", ")
    };

    println!();
    if config_written {
        println!("Config written to {config_label}.");
        println!(
            "Brain: {} (fallback: {}). Bypass: {}.",
            config.brain.default, fallback_str, bypass_str
        );
    } else {
        println!("Config: left unchanged (you declined to write {config_label}).");
    }

    #[cfg(feature = "telegram-bot")]
    if config.bot.telegram.enabled {
        println!(
            "Telegram bot: enabled (operator_user_id = {:?})",
            config.bot.telegram.operator_user_id
        );
        if config.bot.telegram.bot_token.is_none() {
            println!("  Reminder: set SPUR_TELEGRAM_BOT_TOKEN env var before running");
        }
    }

    if config.agents.entries.len() >= 2 {
        println!();
        println!("Tip: set `capabilities = [\"security\", ...]` on each agent");
        println!("to enable capability-based delegation from the brain.");
    }

    println!();
    println!("Next step:");
    println!("  spur run \"describe the repo in 3 bullets\"    # one-shot");
    println!("  spur tui                                     # interactive TUI");
    println!("  spur config check                            # validate your setup");
    #[cfg(feature = "telegram-bot")]
    if config.bot.telegram.enabled {
        println!("  spur bot telegram                            # start Telegram bot");
    }
}

/// `spur pm init` — bootstrap the beads tracker in a fresh repo.
///
/// Idempotent: safe to run multiple times. On a fresh repo, this:
///   1. creates `.beads/` and initializes `beads.db` + `issues.jsonl` via
///      `BeadsCrateAdapter::open` (which runs `init_writer_with_flush`);
///   2. appends beads-derived-file entries to `.gitignore` if missing
///      (so `issues.jsonl` stays committed and `beads.db` / locks / temps
///      do not);
///   3. ensures `[pm.beads] enabled = true` in `.spur/config.toml` if the
///      file already exists. If the config doesn't exist, prints a hint
///      pointing at `spur init` instead of fabricating one.
pub async fn run_pm_init(repo_root: PathBuf) -> Result<()> {
    use spur_pm::beads_crate::adapter::{AdapterConfig, BeadsCrateAdapter};

    let beads_dir = repo_root.join(".beads");
    let already_existed = beads_dir.exists();

    std::fs::create_dir_all(&beads_dir)
        .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", beads_dir.display()))?;

    // Bootstrap SQLite + JSONL by opening once and dropping. The adapter's
    // `open` runs `init_writer_with_flush` under `.write.lock`, which creates
    // `beads.db` (schema) and leaves an empty `issues.jsonl`.
    let _adapter = BeadsCrateAdapter::open(&beads_dir, AdapterConfig::default())
        .await
        .map_err(|e| anyhow::anyhow!("beads adapter init failed: {e}"))?;
    drop(_adapter);

    // `init_writer_with_flush` only emits `issues.jsonl` when the DB is dirty;
    // a fresh DB skips the write. Materialize an empty file ourselves so the
    // source-of-truth artifact exists at well-known path from the first run
    // and `git add .beads/issues.jsonl` works without a follow-up command.
    let jsonl = beads_dir.join("issues.jsonl");
    if !jsonl.exists() {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&jsonl)
            .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", jsonl.display()))?;
    }

    // Patch .gitignore — selective entries so `issues.jsonl` stays committed.
    let gitignore_added =
        ensure_gitignore_lines(&repo_root.join(".gitignore"), BEADS_GITIGNORE_LINES)?;

    // Patch .spur/config.toml only if it exists.
    let config_path = repo_root.join(".spur").join("config.toml");
    let config_touched = if config_path.exists() {
        ensure_pm_beads_enabled(&config_path)?
    } else {
        false
    };

    println!();
    if already_existed {
        println!("[pm init] .beads/ already present — verified schema is up to date.");
    } else {
        println!("[pm init] initialized .beads/ (beads.db + issues.jsonl).");
    }
    if gitignore_added > 0 {
        println!(
            "[pm init] added {gitignore_added} entr{} to .gitignore.",
            if gitignore_added == 1 { "y" } else { "ies" }
        );
    } else {
        println!("[pm init] .gitignore already up to date.");
    }
    if config_touched {
        println!("[pm init] set [pm.beads] enabled = true in .spur/config.toml.");
    } else if !config_path.exists() {
        println!("[pm init] tip: run `spur init` to create .spur/config.toml.");
    }
    println!();
    println!("Next: commit `.beads/issues.jsonl` and `.gitignore`, then create issues.");
    Ok(())
}

/// Lines added to `.gitignore` by `spur pm init`. Selective on purpose:
/// `issues.jsonl` is the source of truth and MUST stay committed.
const BEADS_GITIGNORE_LINES: &[&str] = &[
    ".beads/beads.db",
    ".beads/beads.db-*",
    ".beads/.write.lock",
    ".beads/issues.jsonl.*.tmp",
];

/// Append any of `lines` not already present in `path`. Returns count added.
/// Creates the file (with a leading section comment) if it doesn't exist.
fn ensure_gitignore_lines(path: &std::path::Path, lines: &[&str]) -> Result<usize> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let present: std::collections::HashSet<&str> = existing
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    let mut to_add: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !present.contains(*l))
        .collect();
    if to_add.is_empty() {
        return Ok(0);
    }

    let mut out = String::with_capacity(existing.len() + 256);
    out.push_str(&existing);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !existing.contains("# spur pm (beads)") {
        out.push_str("\n# spur pm (beads) — derived files; commit issues.jsonl\n");
    }
    for line in &to_add {
        out.push_str(line);
        out.push('\n');
    }

    std::fs::write(path, out)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", path.display()))?;
    Ok(to_add.drain(..).count())
}

/// Ensure `[pm.beads] enabled = true` without expanding a sparse config.
/// Returns `true` if the file was changed.
fn ensure_pm_beads_enabled(config_path: &std::path::Path) -> Result<bool> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", config_path.display()))?;
    let mut config: toml::Value = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", config_path.display()))?;
    if config
        .get("pm")
        .and_then(|pm| pm.get("beads"))
        .and_then(|beads| beads.get("enabled"))
        .and_then(toml::Value::as_bool)
        == Some(true)
    {
        return Ok(false);
    }
    let root = config
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("{} is not a TOML table", config_path.display()))?;
    let pm = root
        .entry("pm")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[pm] is not a TOML table in {}", config_path.display()))?;
    let beads = pm
        .entry("beads")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "[pm.beads] is not a TOML table in {}",
                config_path.display()
            )
        })?;
    beads.insert("enabled".to_owned(), toml::Value::Boolean(true));
    std::fs::write(config_path, toml::to_string_pretty(&config)?)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", config_path.display()))?;
    Ok(true)
}

/// Run the SpurPower skills installer independently of config init.
pub fn run_skills_init(repo_root: &std::path::Path) -> Result<()> {
    match spur_core::skills::installer::run(repo_root) {
        Ok(summary) => {
            println!();
            print!("{summary}");
            print_gitattributes_advisory_if_needed(repo_root);
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("skills install failed: {e}")),
    }
}

/// Filtered skills install — used by `spur init`'s default-on path so we
/// don't materialize dotfile dirs for agents the user doesn't have.
pub fn run_skills_init_filtered(
    repo_root: &std::path::Path,
    adapters: &[spur_core::skills::adapters::Adapter],
) -> Result<()> {
    match spur_core::skills::installer::run_filtered(repo_root, adapters) {
        Ok(summary) => {
            println!();
            print!("{summary}");
            print_gitattributes_advisory_if_needed(repo_root);
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("skills install failed: {e}")),
    }
}

/// Map discovered agent names to skill adapters. `SpurHermetic` is always
/// included for brain prompt injection. Unknown agent names are ignored.
fn adapters_for_discovered_agents(
    discovered: &std::collections::HashSet<String>,
) -> Vec<spur_core::skills::adapters::Adapter> {
    use spur_core::skills::adapters::Adapter;
    let mut set: std::collections::HashSet<Adapter> = std::collections::HashSet::new();
    set.insert(Adapter::SpurHermetic);
    for name in discovered {
        match name.as_str() {
            "claude-code" => {
                set.insert(Adapter::ClaudeCode);
            }
            "codex" | "codex-bin" => {
                set.insert(Adapter::Codex);
            }
            "gemini" => {
                set.insert(Adapter::Gemini);
            }
            "kiro" => {
                set.insert(Adapter::Kiro);
            }
            "opencode" => {
                set.insert(Adapter::OpenCode);
            }
            "kimi" => {
                set.insert(Adapter::Kimi);
            }
            _ => {}
        }
    }
    // Preserve `Adapter::all()` order so output is deterministic.
    Adapter::all()
        .iter()
        .copied()
        .filter(|a| set.contains(a))
        .collect()
}

fn prompt_permission_bypass(config: &mut SpurConfig) -> Result<()> {
    let bypass_agents: Vec<&str> = config
        .agents
        .entries
        .iter()
        .filter(|a| a.effective_permissions().skip)
        .map(|a| a.name.as_str())
        .collect();

    if bypass_agents.is_empty() {
        return Ok(());
    }

    println!();
    println!("WARNING: the following agents have permission bypass enabled:");
    for name in &bypass_agents {
        println!("  - {name}");
    }
    println!();
    println!("Permission bypass allows agents to execute tools without prompting.");
    println!("Keep bypass enabled? [y/N]");
    eprint!("> ");
    std::io::stderr().flush()?;

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    if line.trim().eq_ignore_ascii_case("y") {
        return Ok(());
    }

    // User declined — disable bypass for all agents.
    for agent in &mut config.agents.entries {
        agent.permissions.skip = false;
        agent.skip_permissions = false;
    }
    println!("Permission bypass disabled for all agents.");
    Ok(())
}

fn print_gitattributes_advisory_if_needed(repo_root: &std::path::Path) {
    let path = repo_root.join(".gitattributes");
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    if !(contents.contains("*.md") && contents.contains("eol=lf")) {
        println!();
        println!("Tip: add `*.md text eol=lf` to .gitattributes for cross-platform");
        println!("     teammates. SpurPower marker files may thrash across CRLF/LF");
        println!("     systems otherwise.");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_pm_beads_enabled, interpret_yes_no, materialize_pi_mcp_config,
        recompute_brain_and_fallback, AgentRole, SpurConfig,
    };

    fn config_with_pi_kind(enabled: bool) -> SpurConfig {
        let mut config = SpurConfig::default();
        let mut agent = spur_acp::config::AgentConfig::with_defaults("pi");
        agent.kind = if enabled {
            spur_acp::types::AgentKind::Pi
        } else {
            spur_acp::types::AgentKind::Generic
        };
        config.agents.entries.push(agent);
        config
    }

    #[test]
    fn recompute_preserves_valid_brain_preference() {
        let mut config = SpurConfig::default();
        for name in ["claude-code", "codex"] {
            let mut agent = spur_acp::config::AgentConfig::with_defaults(name);
            agent.role = AgentRole::Both;
            config.agents.entries.push(agent);
        }
        config.brain.default = "codex".to_string();
        config.brain.fallback = vec!["claude-code".to_string()];

        recompute_brain_and_fallback(&mut config, true, true);

        assert_eq!(config.brain.default, "codex");
        assert_eq!(config.brain.fallback, vec!["claude-code".to_string()]);
    }

    #[test]
    fn recompute_preserves_explicit_empty_fallback() {
        let mut config = SpurConfig::default();
        for name in ["claude-code", "codex"] {
            let mut agent = spur_acp::config::AgentConfig::with_defaults(name);
            agent.role = AgentRole::Both;
            config.agents.entries.push(agent);
        }
        config.brain.default = "codex".to_string();
        config.brain.fallback.clear();

        recompute_brain_and_fallback(&mut config, true, true);

        assert_eq!(config.brain.default, "codex");
        assert!(config.brain.fallback.is_empty());
    }

    #[test]
    fn recompute_replaces_fallback_whose_agent_is_missing() {
        let mut config = SpurConfig::default();
        for name in ["claude-code", "codex"] {
            let mut agent = spur_acp::config::AgentConfig::with_defaults(name);
            agent.role = AgentRole::Both;
            config.agents.entries.push(agent);
        }
        config.brain.default = "claude-code".to_string();
        config.brain.fallback = vec!["kiro".to_string()];

        recompute_brain_and_fallback(&mut config, true, true);

        assert_eq!(config.brain.fallback, ["codex"]);
    }

    #[test]
    fn pm_init_preserves_sparse_unrelated_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[failover]\ncooldown_minutes=10\n").unwrap();

        assert!(ensure_pm_beads_enabled(&path).unwrap());

        let value: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["failover"]["cooldown_minutes"].as_integer(), Some(10));
        assert_eq!(value["pm"]["beads"]["enabled"].as_bool(), Some(true));
        assert!(value.get("worktree").is_none());
    }

    #[test]
    fn pm_init_accepts_sparse_agent_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[[agents.entries]]\nname='codex'\ncapabilities=['project']\n",
        )
        .unwrap();

        assert!(ensure_pm_beads_enabled(&path).unwrap());

        let value: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value["agents"]["entries"][0]["name"].as_str(),
            Some("codex")
        );
        assert!(value["agents"]["entries"][0].get("command").is_none());
        assert_eq!(value["pm"]["beads"]["enabled"].as_bool(), Some(true));
    }

    #[test]
    fn empty_input_defaults_to_yes() {
        assert!(interpret_yes_no(""));
        assert!(interpret_yes_no("\n"));
        assert!(interpret_yes_no("   \n"));
    }

    #[test]
    fn explicit_no_is_no() {
        for input in ["n", "N", "no", "No", "NO", " no \n"] {
            assert!(!interpret_yes_no(input), "{input:?} should be No");
        }
    }

    #[test]
    fn yes_and_anything_else_is_yes() {
        for input in ["y", "Y", "yes", "Yes", "yep", "sure", "1"] {
            assert!(interpret_yes_no(input), "{input:?} should be Yes");
        }
    }

    #[test]
    fn pi_mcp_writes_config_only_for_pi_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!materialize_pi_mcp_config(dir.path(), &config_with_pi_kind(false)).unwrap());
        assert!(!dir.path().join(".pi/mcp.json").exists());

        assert!(materialize_pi_mcp_config(dir.path(), &config_with_pi_kind(true)).unwrap());
        let written: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".pi/mcp.json")).unwrap(),
        )
        .unwrap();
        let servers = written["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 4);
        assert_eq!(servers["spur-graph"]["command"], "spur");
        assert_eq!(
            servers["spur-graph"]["args"],
            serde_json::json!(["graph", "mcp"])
        );
        assert_eq!(
            servers["spur-solver"]["args"],
            serde_json::json!(["solver", "mcp"])
        );
        assert_eq!(servers["spur"]["args"], serde_json::json!(["mcp"]));
    }

    #[test]
    fn pi_mcp_is_idempotent_and_preserves_user_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Pre-existing user config with one custom server and one SPUR
        // server carrying a user override (`disabled`).
        std::fs::create_dir_all(dir.path().join(".pi")).unwrap();
        std::fs::write(
            dir.path().join(".pi/mcp.json"),
            r#"{"mcpServers":{"chrome":{"command":"npx","args":["-y","x"]},"spur-graph":{"command":"spur","args":["graph","mcp"],"disabled":true}}}"#,
        )
        .unwrap();

        // First run adds the two missing servers; second is a no-op.
        assert!(materialize_pi_mcp_config(dir.path(), &config_with_pi_kind(true)).unwrap());
        assert!(!materialize_pi_mcp_config(dir.path(), &config_with_pi_kind(true)).unwrap());

        let after: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".pi/mcp.json")).unwrap(),
        )
        .unwrap();
        let servers = after["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 5, "chrome + 4 SPUR servers");
        assert_eq!(servers["chrome"]["command"], "npx");
        assert_eq!(
            servers["spur-graph"]["disabled"], true,
            "user override preserved"
        );
        assert_eq!(servers["spur-analyst"]["command"], "spur");
    }

    #[test]
    fn pi_mcp_rejects_corrupt_mcp_servers_array() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".pi")).unwrap();
        std::fs::write(dir.path().join(".pi/mcp.json"), r#"{"mcpServers":[]}"#).unwrap();
        assert!(materialize_pi_mcp_config(dir.path(), &config_with_pi_kind(true)).is_err());
    }
}
