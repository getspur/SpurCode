use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use spur_acp::config::ContextServiceConfig;
use spur_core::mcp::delegation::DelegationMcpDeps;
use spur_core::mcp::plan::PlanMcpDeps;
use spur_core::mcp::signals::SignalMcpDeps;
use spur_license::policy::PolicyResolver;
use spur_license::{FeatureGate, FeatureKey, LicenseState, Plan};

const SOLVER_TOOLS: &[&str] = &[
    "solve_rule_spec",
    "solve_rules",
    "solve_constraint_spec",
    "solve_constraint_check",
    "solve_constraints",
    "solve_smt",
    "get_solve_result",
];

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct JevFlagGuard {
    previous: Option<String>,
    _lock: MutexGuard<'static, ()>,
}

impl JevFlagGuard {
    fn acquire() -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self {
            previous: std::env::var("SPUR_JEV_ENABLED").ok(),
            _lock: lock,
        }
    }

    fn set(&self, value: Option<&str>) {
        let _held_lock = &self._lock;
        match value {
            Some(value) => std::env::set_var("SPUR_JEV_ENABLED", value),
            None => std::env::remove_var("SPUR_JEV_ENABLED"),
        }
    }
}

impl Drop for JevFlagGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("SPUR_JEV_ENABLED", value),
            None => std::env::remove_var("SPUR_JEV_ENABLED"),
        }
    }
}

fn feature_gate() -> Arc<FeatureGate> {
    let gate = Arc::new(FeatureGate::new(PolicyResolver::embedded()));
    let features = BTreeSet::from([FeatureKey::PM_PRO_BEADS_ADVANCED.as_str().to_owned()]);
    gate.update_state(&LicenseState::active_validated(Plan::Pro, features));
    gate
}

fn brain_tool_names() -> Vec<String> {
    let registry = spur_core::mcp::brain_tool_registry(
        DelegationMcpDeps::catalog_only(),
        PlanMcpDeps::catalog_only(),
        SignalMcpDeps {
            pm_service: None,
            event_sink: None,
            feature_gate: feature_gate(),
        },
        &ContextServiceConfig {
            url: String::new(),
            ..ContextServiceConfig::default()
        },
    )
    .expect("brain registry");
    registry
        .list_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

fn brain_tool_names_for_repo_root(repo_root: &Path) -> Vec<String> {
    let registry = spur_core::mcp::brain_tool_registry_for_repo_root(
        DelegationMcpDeps::catalog_only(),
        PlanMcpDeps::catalog_only(),
        SignalMcpDeps {
            pm_service: None,
            event_sink: None,
            feature_gate: feature_gate(),
        },
        &ContextServiceConfig {
            url: String::new(),
            ..ContextServiceConfig::default()
        },
        repo_root,
    )
    .expect("brain registry");
    registry
        .list_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

fn solver_tool_names(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|name| SOLVER_TOOLS.contains(&name.as_str()) || name.as_str() == "jev_compile")
        .cloned()
        .collect()
}

#[test]
fn jev_compile_is_flagged_brain_only_solver_tool() {
    let flag = JevFlagGuard::acquire();
    let repo = tempfile::tempdir().expect("temp repo");
    let spur_dir = repo.path().join(".spur");
    fs::create_dir_all(&spur_dir).expect("create .spur directory");
    let config_path = spur_dir.join("config.toml");

    flag.set(None);
    let disabled = solver_tool_names(&brain_tool_names());
    assert_eq!(disabled, SOLVER_TOOLS, "flag-off solver catalog drifted");

    fs::write(&config_path, "[jev]\nenabled = false\n").expect("write disabled config");
    let config_disabled = solver_tool_names(&brain_tool_names_for_repo_root(repo.path()));
    assert_eq!(
        config_disabled, SOLVER_TOOLS,
        "config-disabled solver catalog drifted"
    );

    flag.set(Some("1"));
    let env_enabled = solver_tool_names(&brain_tool_names_for_repo_root(repo.path()));
    assert_eq!(env_enabled.len(), SOLVER_TOOLS.len() + 1);
    assert_eq!(env_enabled.last().map(String::as_str), Some("jev_compile"));

    flag.set(None);
    fs::write(&config_path, "[jev]\nenabled = true\n").expect("write enabled config");
    let config_enabled = solver_tool_names(&brain_tool_names_for_repo_root(repo.path()));
    assert_eq!(config_enabled.len(), SOLVER_TOOLS.len() + 1);
    assert_eq!(
        config_enabled.last().map(String::as_str),
        Some("jev_compile")
    );

    let worker_names: Vec<String> = spur_core::mcp::worker_tools_list()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        worker_names.iter().all(|name| name != "jev_compile"),
        "jev_compile must never be exposed to workers"
    );

    fs::write(&config_path, "[jev\n").expect("write malformed config");
    let malformed_config = solver_tool_names(&brain_tool_names_for_repo_root(repo.path()));
    assert_eq!(
        malformed_config, SOLVER_TOOLS,
        "config load failure should leave Jev disabled"
    );
}
