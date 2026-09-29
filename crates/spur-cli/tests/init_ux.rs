//! Init UX tests: contract guard + behavioral tests for `spur init`.
//!
//! Kept as one file because the contract test is trivial and colocating
//! it with the behavioral tests means future contributors see the
//! install-hint requirement when they open this file.

#![cfg(unix)]

#[test]
fn install_hints_cover_all_seed_agents() {
    // Can't access the private const directly from an integration test.
    // Re-encode it here via a parallel list. If the two drift, the test
    // fails and forces the contributor to update both sides.
    //
    // Alternative: expose INSTALL_HINTS as pub from a lib target. That
    // would be cleaner, but spur-cli is a binary-only crate and adding
    // a lib target just for this is overkill. Keep the parallel list.
    let expected_names: &[&str] = &[
        "kiro",
        "claude-code",
        "codex-bin",
        "codex",
        "gemini",
        "opencode",
        "goose",
        "kimi",
        "grok",
        "pi",
    ];
    let seeds = spur_acp::config::load_seed_template();
    for agent in &seeds.entries {
        assert!(
            expected_names.contains(&agent.name.as_str()),
            "seed agent `{}` has no INSTALL_HINTS entry — add one to \
             crates/spur-cli/src/main.rs AND to expected_names in this test",
            agent.name
        );
    }
    // Also check the reverse direction: no orphan expected_names that
    // aren't in seeds (would indicate a stale hint for a deleted agent).
    let seed_names: Vec<_> = seeds.entries.iter().map(|a| a.name.as_str()).collect();
    for expected in expected_names {
        assert!(
            seed_names.contains(expected),
            "expected_names has `{expected}` but it's not in seed template"
        );
    }
}

#[test]
fn codex_onboarding_pins_current_acp_adapter() {
    const ADAPTER: &str = "@agentclientprotocol/codex-acp@1.9.0";
    let init_source = include_str!("../src/commands/init.rs");
    let cookbook = include_str!("../../../docs/spur/agent-onboarding-cookbook.md");

    assert_eq!(
        init_source.matches(ADAPTER).count(),
        2,
        "both codex-bin and codex install hints must pin {ADAPTER}"
    );
    assert!(
        cookbook.contains(ADAPTER),
        "the onboarding cookbook must name the pinned Codex ACP adapter"
    );
    assert!(!init_source.contains("@zed-industries/codex-acp"));
    assert!(!cookbook.contains("@zed-industries/codex-acp"));
}

#[test]
fn seed_agents_do_not_include_legacy_claude_stream_json() {
    let seeds = spur_acp::config::load_seed_template();
    let names: Vec<_> = seeds.entries.iter().map(|a| a.name.as_str()).collect();

    assert!(
        !names.contains(&"claude-code-sj"),
        "spur init should no longer offer claude-code-sj; only claude-code is supported"
    );
    assert!(
        !names.contains(&"claude-code-js"),
        "spur init should not offer the legacy claude-code-js alias"
    );
}

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

/// Serialize tests that spawn subprocesses to avoid tempdir/PATH
/// collision if run in parallel.
static LOCK: Mutex<()> = Mutex::new(());

fn stub_binary(dir: &std::path::Path, name: &str) {
    let path = dir.join(name);
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

fn stub_which(dir: &std::path::Path) {
    let path = dir.join("which");
    fs::write(
        &path,
        "#!/bin/sh\nIFS=:\nfor dir in $PATH; do\n  if [ -x \"$dir/$1\" ]; then\n    echo \"$dir/$1\"\n    exit 0\n  fi\ndone\nexit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

fn controlled_path(dir: &std::path::Path) -> String {
    dir.display().to_string()
}

fn load_layered_with_home(repo: &Path, home: &Path) -> spur_acp::config::SpurConfig {
    let original_home = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    let effective = spur_acp::config::load_layered(repo);
    match original_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    effective.unwrap()
}

fn seed_agent_commands() -> Vec<String> {
    let mut commands = Vec::new();
    for agent in spur_acp::config::load_seed_template().entries {
        if !commands.contains(&agent.command) {
            commands.push(agent.command);
        }
    }
    commands
}

fn stub_seed_agent_commands(dir: &std::path::Path) {
    for command in seed_agent_commands() {
        stub_binary(dir, &command);
    }
}

fn spur() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_spur"));
    // Strip both debug-only env vars that can perturb the resolved
    // feature snapshot in the spawned child:
    //
    // - `SPUR_LICENSE_DEV_PLAN`: post-policy-gap fix, unknown values
    //   fall through to community, but an explicit `pro` would change
    //   the resolved feature set beyond what these tests assert.
    // - `SPUR_LICENSE_TEST_STRIP_KEYS`: a dev shell that has set this
    //   for an unrelated test run would otherwise strip features the
    //   `init` path needs, tripping `require_cli_gate(...)`.
    c.env_remove("SPUR_LICENSE_DEV_PLAN");
    c.env_remove("SPUR_LICENSE_TEST_STRIP_KEYS");
    c
}

#[test]
fn init_with_zero_agents_writes_no_config() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let tmp = TempDir::new().unwrap();
    stub_which(tmp.path());

    let status = spur()
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(tmp.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(
        status.success(),
        "spur init should exit 0 even with no agents"
    );
    assert!(
        !tmp.path().join(".spur/config.toml").exists(),
        "spur init with zero agents must NOT write .spur/config.toml"
    );
}

#[test]
fn init_with_existing_config_merges_agents() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".spur")).unwrap();

    // Make Claude Code a user-layer baseline entry. Project init may then
    // legitimately omit it from the sparse project overlay.
    let mut user_config = spur_acp::config::SpurConfig::default();
    let claude_code = spur_acp::config::load_seed_template()
        .entries
        .into_iter()
        .find(|agent| agent.name == "claude-code")
        .expect("seed template should include claude-code");
    user_config.agents.entries.push(claude_code);
    let user_path = home.path().join(".spur/config.toml");
    fs::create_dir_all(user_path.parent().unwrap()).unwrap();
    fs::write(&user_path, toml::to_string_pretty(&user_config).unwrap()).unwrap();

    // Pre-existing config with custom brain and bot settings.
    let existing = r#"
[brain]
default = "kiro"

[bot.telegram]
enabled = true
operator_user_id = 12345
    "#;
    fs::write(tmp.path().join(".spur/config.toml"), existing).unwrap();
    stub_which(tmp.path());
    stub_binary(tmp.path(), "npx");

    let status = spur()
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(tmp.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(status.success(), "merge init should exit 0");
    let effective = load_layered_with_home(tmp.path(), home.path());
    let agent_names: Vec<_> = effective
        .agents
        .entries
        .iter()
        .map(|agent| agent.name.as_str())
        .collect();

    assert!(
        agent_names.contains(&"claude-code"),
        "effective config should retain the baseline Claude Code agent; got {agent_names:?}"
    );
    assert!(
        agent_names.contains(&"codex"),
        "effective config should merge the other agent discovered through npx; got {agent_names:?}"
    );
    assert_eq!(effective.brain.default, "claude-code");
    assert_eq!(effective.brain.fallback, ["codex"]);
    assert!(effective.bot.telegram.enabled);
    assert_eq!(effective.bot.telegram.operator_user_id, Some(12345));
}

#[test]
fn init_keeps_user_cooldown_when_project_omits_it() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let user_path = home.path().join(".spur/config.toml");
    fs::create_dir_all(user_path.parent().unwrap()).unwrap();

    let mut user_config = spur_acp::config::SpurConfig::default();
    user_config.failover.cooldown_minutes = 10;
    user_config.agents.entries.push(
        spur_acp::config::load_seed_template()
            .entries
            .into_iter()
            .find(|agent| agent.name == "claude-code")
            .unwrap(),
    );
    fs::write(&user_path, toml::to_string_pretty(&user_config).unwrap()).unwrap();
    stub_which(repo.path());
    stub_binary(repo.path(), "npx");

    let status = spur()
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(repo.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(status.success());
    let project: toml::Value =
        toml::from_str(&fs::read_to_string(repo.path().join(".spur/config.toml")).unwrap())
            .unwrap();
    assert!(
        project.get("failover").is_none(),
        "project wrote inherited failover: {:?}",
        project.get("failover")
    );
    assert_eq!(
        load_layered_with_home(repo.path(), home.path())
            .failover
            .cooldown_minutes,
        10
    );
}

#[test]
fn init_accepts_sparse_project_agent_override() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let user_path = home.path().join(".spur/config.toml");
    fs::create_dir_all(user_path.parent().unwrap()).unwrap();
    fs::create_dir_all(repo.path().join(".spur")).unwrap();

    let mut user_config = spur_acp::config::SpurConfig::default();
    user_config.agents.entries.push(
        spur_acp::config::load_seed_template()
            .entries
            .into_iter()
            .find(|agent| agent.name == "claude-code")
            .unwrap(),
    );
    let inherited_command = user_config.agents.entries[0].command.clone();
    fs::write(&user_path, toml::to_string_pretty(&user_config).unwrap()).unwrap();
    fs::write(
        repo.path().join(".spur/config.toml"),
        "[[agents.entries]]\nname='claude-code'\ncapabilities=['project']\n",
    )
    .unwrap();
    stub_which(repo.path());
    stub_binary(repo.path(), "npx");

    let status = spur()
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(repo.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(status.success());
    let effective = load_layered_with_home(repo.path(), home.path());
    let agent = effective
        .agents
        .entries
        .iter()
        .find(|agent| agent.name == "claude-code")
        .unwrap();
    assert_eq!(agent.command, inherited_command);
    assert_eq!(agent.capabilities, vec!["project".to_string()]);
    let project: toml::Value =
        toml::from_str(&fs::read_to_string(repo.path().join(".spur/config.toml")).unwrap())
            .unwrap();
    let agent_overlay = project["agents"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"].as_str() == Some("claude-code"))
        .unwrap();
    assert!(agent_overlay.get("command").is_none());
}

#[test]
fn init_does_not_copy_unchanged_sparse_user_agent() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let user_path = home.path().join(".spur/config.toml");
    fs::create_dir_all(user_path.parent().unwrap()).unwrap();
    fs::create_dir_all(repo.path().join(".spur")).unwrap();
    let agent =
        "[[agents.entries]]\nname='claude-code'\ncommand='claude'\ntransport='acp'\nrole='both'\n";
    fs::write(&user_path, agent).unwrap();
    fs::write(repo.path().join(".spur/config.toml"), agent).unwrap();
    stub_which(repo.path());
    stub_binary(repo.path(), "npx");

    let status = spur()
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(repo.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(status.success());
    let project: toml::Value =
        toml::from_str(&fs::read_to_string(repo.path().join(".spur/config.toml")).unwrap())
            .unwrap();
    let entries = project["agents"]["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| entry["name"].as_str() != Some("claude-code")),
        "unchanged user agent should stay inherited"
    );
}

#[test]
fn init_accepts_agent_completed_by_project_layer() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let user_path = home.path().join(".spur/config.toml");
    fs::create_dir_all(user_path.parent().unwrap()).unwrap();
    fs::create_dir_all(repo.path().join(".spur")).unwrap();
    fs::write(
        &user_path,
        "[[agents.entries]]\nname='codex'\ncapabilities=['user']\n",
    )
    .unwrap();
    fs::write(
        repo.path().join(".spur/config.toml"),
        "[[agents.entries]]\nname='codex'\ncommand='codex'\ntransport='acp'\n",
    )
    .unwrap();
    stub_which(repo.path());
    stub_binary(repo.path(), "npx");

    let status = spur()
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(repo.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(status.success());
    let effective = load_layered_with_home(repo.path(), home.path());
    let codex = effective
        .agents
        .entries
        .iter()
        .find(|agent| agent.name == "codex")
        .unwrap();
    assert_eq!(codex.command, "codex");
    assert_eq!(codex.capabilities, vec!["user".to_string()]);
    let project: toml::Value =
        toml::from_str(&fs::read_to_string(repo.path().join(".spur/config.toml")).unwrap())
            .unwrap();
    let overlay = project["agents"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"].as_str() == Some("codex"))
        .unwrap();
    assert!(overlay.get("capabilities").is_none());
    assert!(overlay.get("role").is_none());
    assert!(overlay.get("permissions").is_none());
}

#[test]
fn init_with_force_resets_agents_preserves_non_agent_config() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".spur")).unwrap();

    // Config with a manually-added agent and bot settings.
    let existing = r#"
[[agents.entries]]
name = "custom-agent"
command = "custom"
transport = "acp"
role = "both"

[brain]
default = "custom-agent"

[bot.telegram]
enabled = true
operator_user_id = 99999
"#;
    fs::write(tmp.path().join(".spur/config.toml"), existing).unwrap();

    // Only kiro-cli is on PATH.
    stub_which(tmp.path());
    stub_binary(tmp.path(), "kiro-cli");

    let status = spur()
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(tmp.path()))
        .args(["init", "--force"])
        .status()
        .expect("spawn spur init --force");

    assert!(status.success(), "force init should exit 0");
    let after = fs::read_to_string(tmp.path().join(".spur/config.toml")).unwrap();

    // --force should drop the custom agent.
    assert!(
        !after.contains("custom-agent"),
        "--force should reset to discovered-only; got:\n{after}"
    );

    // Should preserve bot config.
    assert!(
        after.contains("enabled = true"),
        "should preserve bot config; got:\n{after}"
    );
    assert!(
        after.contains("operator_user_id = 99999"),
        "should preserve bot operator; got:\n{after}"
    );

    // Brain should adapt to the discovered agent.
    assert!(
        after.contains(r#"default = "kiro""#),
        "brain should adapt to kiro; got:\n{after}"
    );
}

#[test]
fn init_prefers_claude_code_as_default_brain() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let tmp = TempDir::new().unwrap();

    stub_which(tmp.path());
    stub_binary(tmp.path(), "kiro-cli");
    stub_binary(tmp.path(), "npx");

    let status = spur()
        .current_dir(tmp.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(tmp.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(status.success(), "init should exit 0");
    let after = fs::read_to_string(tmp.path().join(".spur/config.toml")).unwrap();

    let effective: spur_acp::config::SpurConfig = toml::from_str(&after).unwrap();
    assert_eq!(
        effective.brain.default, "claude-code",
        "claude-code should be the default brain when available"
    );
    assert!(
        after.contains(r#"    "kiro","#),
        "kiro should become a fallback brain; got:\n{after}"
    );
}

#[test]
fn project_init_with_user_layer_writes_sparse_overlay() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let path_dir = TempDir::new().unwrap();
    stub_which(path_dir.path());
    stub_seed_agent_commands(path_dir.path());
    fs::create_dir_all(repo.path().join(".beads")).unwrap();

    let mut user_config = spur_acp::config::SpurConfig::default();
    user_config.agents = spur_acp::config::load_seed_template();
    user_config.brain.default = "codex".to_string();
    user_config.brain.fallback.clear();
    let user_path = home.path().join(".spur/config.toml");
    fs::create_dir_all(user_path.parent().unwrap()).unwrap();
    fs::write(&user_path, toml::to_string_pretty(&user_config).unwrap()).unwrap();

    let status = spur()
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(path_dir.path()))
        .arg("init")
        .status()
        .expect("spawn spur init");

    assert!(status.success(), "project init should exit 0");

    let project_path = repo.path().join(".spur/config.toml");
    let raw = fs::read_to_string(&project_path).unwrap();
    let table = toml::from_str::<toml::Value>(&raw)
        .unwrap()
        .as_table()
        .cloned()
        .unwrap();

    assert!(
        !table.contains_key("worktree"),
        "project layer must not re-expand default worktree settings:\n{raw}"
    );
    assert!(
        !table.contains_key("cost"),
        "project layer must not re-expand default cost settings:\n{raw}"
    );
    assert!(
        !table.contains_key("agents"),
        "project layer must not duplicate user-layer agents:\n{raw}"
    );
    assert!(
        !table.contains_key("brain"),
        "project layer must not duplicate user-layer brain preference:\n{raw}"
    );

    let effective = load_layered_with_home(repo.path(), home.path());
    assert_eq!(effective.brain.default, "codex");
    assert!(effective.brain.fallback.is_empty());
    assert_eq!(
        effective.agents.entries.len(),
        spur_acp::config::load_seed_template().entries.len(),
        "layered load should recover user-layer agents"
    );
}

#[test]
fn init_global_writes_home_config_not_project_config() {
    let _g = LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let path_dir = TempDir::new().unwrap();
    stub_which(path_dir.path());
    stub_binary(path_dir.path(), "npx");
    fs::create_dir_all(repo.path().join(".beads")).unwrap();

    let status = spur()
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("PATH", controlled_path(path_dir.path()))
        .args(["init", "--global"])
        .status()
        .expect("spawn spur init --global");

    assert!(status.success(), "global init should exit 0");

    let global_path = home.path().join(".spur/config.toml");
    assert!(
        global_path.exists(),
        "global init should write {}",
        global_path.display()
    );
    assert!(
        !repo.path().join(".spur/config.toml").exists(),
        "global init must not write the project config"
    );
    let raw = fs::read_to_string(global_path).unwrap();
    assert!(
        raw.contains(r#"name = "claude-code""#),
        "global config should include discovered agents:\n{raw}"
    );
    assert!(
        raw.contains("[context_service]") && raw.contains(r#"url = "https://context.getspur.dev""#),
        "global config should expose the context service endpoint:\n{raw}"
    );
}
