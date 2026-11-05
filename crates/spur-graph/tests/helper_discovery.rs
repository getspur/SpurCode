#![cfg(all(feature = "embed", unix))]
use spur_graph::{
    store::lance_sections::{load_embedding_model, load_embedding_model_cancellable_with_cleanup},
    EmbeddingModelSelection,
};
use std::{
    ffi::OsString, fs, os::unix::fs::PermissionsExt, path::Path, process::Command, sync::Arc,
    time::Instant,
};

const CHILD: &str = "SPUR_M3_DISCOVERY_CHILD";

#[test]
fn discovery_child() {
    let Ok(case) = std::env::var(CHILD) else {
        return;
    };
    let before = std::env::var_os("SPUR_ORT_CACHE");
    let root = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let log = root.join("calls.jsonl");
    let result = if case == "cancel" || case == "timeout" {
        let policy = spur_embedding::LocalPolicy::default()
            .client_policy()
            .unwrap();
        let cleanup =
            Arc::new(spur_embedding::CleanupOwner::new(policy.shutdown_poll_interval).unwrap());
        let cancel = spur_embedding::Cancellation::new();
        let trigger = cancel.clone();
        let watch = log.clone();
        let job = (case == "cancel").then(|| {
            std::thread::spawn(move || {
                let deadline = Instant::now() + policy.startup_timeout;
                while !watch.is_file() {
                    assert!(Instant::now() < deadline, "probe never started");
                    std::thread::yield_now();
                }
                trigger.cancel();
            })
        });
        let budget = if case == "timeout" {
            policy.shutdown_poll_interval
        } else {
            policy.startup_timeout
        };
        let result = load_embedding_model_cancellable_with_cleanup(
            EmbeddingModelSelection::NomicEmbedTextV15,
            false,
            cancel,
            Instant::now() + budget,
            Arc::clone(&cleanup),
        );
        if let Some(job) = job {
            job.join().unwrap();
        }
        cleanup
            .wait_for_cleanup(Instant::now() + policy.shutdown_timeout)
            .unwrap();
        result
    } else {
        load_embedding_model(EmbeddingModelSelection::NomicEmbedTextV15, false)
    };
    let expected = match case.as_str() {
        "missing" | "missing_sibling" => Some("helper executable missing"),
        "relative" | "empty_override" => Some("invalid helper path"),
        "version" => Some("helper version mismatch"),
        "protocol" => Some("helper protocol mismatch"),
        "os" | "arch" => Some("helper platform mismatch"),
        "malformed" | "unknown" | "duplicate" => Some("invalid helper metadata"),
        "oversized" => Some("helper metadata exceeds"),
        "exit" | "abort" => Some("helper compatibility exit"),
        "timeout" => Some("deadline exceeded"),
        "cancel" => Some("cancelled"),
        "empty_cache" | "no_cache_base" | "relative_home" => Some("invalid helper cache"),
        "invalid_json" => Some("invalid embedding helper configuration"),
        _ => None,
    };
    if let Some(expected) = expected {
        let error = result.err().expect("invalid helper accepted");
        use spur_embedding::{ClientError, DiscoveryError};
        let typed = error
            .downcast_ref::<DiscoveryError>()
            .expect("discovery lost typed failure");
        assert!(
            match case.as_str() {
                "missing" | "missing_sibling" => matches!(typed, DiscoveryError::Missing(_)),
                "relative" | "empty_override" => matches!(typed, DiscoveryError::Path { .. }),
                "version" => matches!(typed, DiscoveryError::Version { .. }),
                "protocol" => matches!(typed, DiscoveryError::Protocol { .. }),
                "os" | "arch" => matches!(typed, DiscoveryError::Platform { .. }),
                "malformed" | "unknown" | "duplicate" =>
                    matches!(typed, DiscoveryError::Metadata(_)),
                "oversized" => matches!(typed, DiscoveryError::MetadataLimit(_)),
                "exit" | "abort" => matches!(typed, DiscoveryError::Exit(_)),
                "timeout" => matches!(typed, DiscoveryError::Client(ClientError::Timeout)),
                "cancel" => matches!(typed, DiscoveryError::Client(ClientError::Cancelled)),
                "empty_cache" | "no_cache_base" | "relative_home" =>
                    matches!(typed, DiscoveryError::Cache(_)),
                "invalid_json" => matches!(typed, DiscoveryError::Policy(_)),
                _ => false,
            },
            "unexpected typed error: {typed:?}"
        );
        assert!(
            format!("{error:#}").contains(expected),
            "{case}: expected {expected:?}, got {error:#}"
        );
        if let Ok(contents) = fs::read_to_string(&log) {
            assert!(
                !contents.contains("--describe-model"),
                "descriptor launched after rejected probe: {contents}"
            );
            for row in contents.lines() {
                let event: serde_json::Value = serde_json::from_str(row).unwrap();
                let pid = event["pid"].as_u64().unwrap().to_string();
                let status = Command::new("/bin/kill")
                    .args(["-0", &pid])
                    .output()
                    .unwrap();
                assert!(!status.status.success(), "probe {pid} remains alive");
                assert!(
                    String::from_utf8_lossy(&status.stderr).contains("No such process"),
                    "invalid absence probe: {status:?}"
                );
            }
        }
        fs::write(root.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({"case":case,"error":format!("{error:#}"),"typed_error":format!("{typed:?}")})).unwrap()).unwrap();
    } else {
        let mut model =
            result.unwrap_or_else(|e| panic!("{case}: compatible helper not selected: {e:#}"));
        let rows = model
            .embed(&["search_document: discovery regression"], None)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 768);
        assert!(rows[0].iter().all(|x| x.is_finite()));
        model.close().unwrap();
        let contents = fs::read_to_string(&log).unwrap();
        let events: Vec<serde_json::Value> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.len(), if case == "legacy" { 2 } else { 3 });
        let expected = before
            .clone()
            .map(std::path::PathBuf::from)
            .map(|p| {
                if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir().unwrap().join(p)
                }
            })
            .unwrap_or_else(|| {
                if case == "xdg" {
                    root.join("xdg/spur")
                } else {
                    root.join("home/.spur/cache")
                }
            });
        for event in events {
            assert_eq!(event["cache"].as_str().unwrap(), expected.to_str().unwrap());
        }
        fs::write(root.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({"case":case,"rows":rows.len(),"dimensions":rows[0].len(),"finite":true,"checked_close":true})).unwrap()).unwrap();
    }
    assert_eq!(
        std::env::var_os("SPUR_ORT_CACHE"),
        before,
        "parent cache mutated"
    );
    assert!(
        !root.join("home/.spur/cache").exists(),
        "configuration created native cache"
    );
}

fn run_case(case: &str) {
    let root = tempfile::Builder::new()
        .prefix("M3 helper path ü ")
        .tempdir()
        .unwrap();
    let host = root.path().join("host with spaces");
    fs::copy(std::env::current_exe().unwrap(), &host).unwrap();
    let sibling = root.path().join("spur-embedding-worker");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/discovery_worker.py");
    fs::copy(&fixture, &sibling).unwrap();
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = Command::new(&host);
    command
        .args(["--exact", "discovery_child", "--nocapture"])
        .env(CHILD, case)
        .env_remove("SPUR_EMBEDDING_HELPER_CONFIG")
        .env_remove("SPUR_EMBEDDING_HELPER")
        .env_remove("SPUR_ORT_CACHE")
        .env_remove("XDG_CACHE_HOME")
        .env("HOME", root.path().join("home"))
        .env("SPUR_M3_LOG", root.path().join("calls.jsonl"))
        .env("SPUR_M3_CASE", case)
        .env("SPUR_M3_VERSION", env!("CARGO_PKG_VERSION"))
        .env("SPUR_M3_OS", std::env::consts::OS)
        .env("SPUR_M3_ARCH", std::env::consts::ARCH);
    match case {
        "override" | "nonunicode" => {
            use std::os::unix::ffi::OsStringExt;
            let name = if case == "nonunicode" {
                OsString::from_vec(b"override-\xff helper".to_vec())
            } else {
                "explicit helper ü".into()
            };
            let path = root.path().join(name);
            fs::copy(&sibling, &path).unwrap();
            fs::remove_file(&sibling).unwrap();
            command.env("SPUR_EMBEDDING_HELPER", path);
        }
        "missing" => {
            command.env("SPUR_EMBEDDING_HELPER", root.path().join("missing"));
        }
        "missing_sibling" => {
            fs::remove_file(sibling).unwrap();
        }
        "relative" => {
            command.env("SPUR_EMBEDDING_HELPER", "relative/helper");
        }
        "empty_override" => {
            command.env("SPUR_EMBEDDING_HELPER", "");
        }
        "explicit_cache" => {
            command.env("SPUR_ORT_CACHE", root.path().join("runtime cache ü"));
        }
        "relative_cache" => {
            command.env("SPUR_ORT_CACHE", "relative cache");
        }
        "xdg" => {
            command
                .env_remove("HOME")
                .env("XDG_CACHE_HOME", root.path().join("xdg"));
        }
        "empty_cache" => {
            command.env("SPUR_ORT_CACHE", "");
        }
        "relative_home" => {
            command.env("HOME", "relative/home");
        }
        "no_cache_base" => {
            command.env_remove("HOME");
        }
        "invalid_json" => {
            command.env("SPUR_EMBEDDING_HELPER_CONFIG", "{}");
        }
        "legacy" => {
            let policy = spur_embedding::LocalPolicy::default();
            let config = serde_json::json!({"executable":"/usr/bin/python3", "args":[fixture],
                "artifact_budget":policy.artifact_budget,"limits":policy.limits,
                "queue_capacity":policy.queue_capacity,"max_restarts":policy.max_restarts,
                "startup_timeout_ms":policy.startup_timeout_ms,"request_timeout_ms":policy.request_timeout_ms,
                "shutdown_timeout_ms":policy.shutdown_timeout_ms,"shutdown_poll_interval_ms":policy.shutdown_poll_interval_ms});
            command
                .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string())
                .env("SPUR_EMBEDDING_HELPER", "invalid override");
        }
        _ => {}
    }
    let output = command.output().unwrap();
    if let Some(export) = std::env::var_os("SPUR_M3_EXPORT") {
        let destination = std::path::PathBuf::from(export).join(case);
        fs::create_dir_all(&destination).unwrap();
        for file in ["calls.jsonl", "result.json"] {
            let path = root.path().join(file);
            if path.is_file() {
                fs::copy(path, destination.join(file)).unwrap();
            }
        }
        fs::write(destination.join("command.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "program":command.get_program(),"args":command.get_args().collect::<Vec<_>>(),
            "environment":command.get_envs().collect::<Vec<_>>(),"exit":output.status.to_string(),
            "stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)
        })).unwrap()).unwrap();
    }
    assert!(
        output.status.success(),
        "{case}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_cases(cases: &[&str]) {
    let failures: Vec<_> = cases
        .iter()
        .filter_map(|case| {
            std::panic::catch_unwind(|| run_case(case))
                .err()
                .map(|_| *case)
        })
        .collect();
    assert!(failures.is_empty(), "failing cases: {failures:?}");
}

#[test]
fn discovery_selection_and_cache() {
    run_cases(&[
        "sibling",
        "override",
        "explicit_cache",
        "relative_cache",
        "xdg",
        "legacy",
    ]);
}

// APFS on the exercised macOS host rejects invalid UTF-8 names with EILSEQ.
#[cfg(target_os = "linux")]
#[test]
fn discovery_executes_nonunicode_path() {
    run_case("nonunicode");
}

#[test]
fn discovery_rejects_invalid_paths_and_cache() {
    run_cases(&[
        "missing",
        "missing_sibling",
        "relative",
        "empty_override",
        "empty_cache",
        "no_cache_base",
        "relative_home",
        "invalid_json",
    ]);
}

#[test]
fn discovery_rejects_metadata_before_descriptor() {
    run_cases(&[
        "version",
        "protocol",
        "os",
        "arch",
        "malformed",
        "unknown",
        "duplicate",
        "oversized",
        "exit",
        "abort",
    ]);
}

#[test]
fn discovery_probe_deadline_and_cleanup() {
    run_case("timeout");
    run_case("cancel");
}
