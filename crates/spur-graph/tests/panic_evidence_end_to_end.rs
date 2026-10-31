use serde_json::{json, Value};
use spur_graph::panic_evidence::*;

#[allow(dead_code)]
#[path = "../examples/panic_evidence.rs"]
mod example;

fn corpus() -> Value {
    serde_json::from_str(include_str!("fixtures/panic_evidence/corpus.json")).unwrap()
}

fn collected() -> anyhow::Result<(tempfile::TempDir, Value, Value)> {
    let dir = tempfile::tempdir()?;
    let corpus = corpus();
    for (path, text) in corpus["files"].as_object().unwrap() {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, text.as_str().unwrap())?;
    }
    let sources: Vec<_> = corpus["files"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(path, _)| path.ends_with(".rs"))
        .map(|(path, text)| json!({"file_path":path,"text":text}))
        .collect();
    let (facts, _) = spur_graph::build_facts(dir.path(), None)?;
    let graph = spur_graph::artifact_from_facts(&facts, dir.path())?;
    let request = json!({"command":"collect","graph":graph,"sources":sources});
    let output = example::execute(request.clone())?;
    Ok((dir, request, output))
}

#[test]
fn json_collect_preserves_held_out_multiplicity_and_utf8() -> anyhow::Result<()> {
    let (_dir, _, output) = collected()?;
    assert!(
        output["method_sites"]["sites"].is_array(),
        "collect returns source-bound rows"
    );
    let set: MethodCallSiteEvidenceSet = serde_json::from_value(output["method_sites"].clone())?;
    let source = corpus()["files"]["src/lib.rs"].as_str().unwrap().to_owned();
    let options: Vec<_> = set
        .sites
        .iter()
        .filter(|s| s.file_path == "src/lib.rs" && s.call_text == "a.unwrap()")
        .collect();
    assert_eq!(
        options.len(),
        3,
        "Option and Result repeated invocations survive independently"
    );
    let unicode = set
        .sites
        .iter()
        .find(|s| s.call_text == "x.expect(\"unicode\")")
        .unwrap();
    assert_eq!(
        &source[unicode.byte_range[0]..unicode.byte_range[1]],
        unicode.call_text
    );
    assert!(set.sites.iter().any(|s| s.file_path == "dep/src/lib.rs"));
    assert!(
        !set.sites.iter().any(|s| s.call_text.contains("\"macro\"")),
        "macro tokens are not physical expanded method sites"
    );
    assert!(set
        .sites
        .iter()
        .all(|s| s.compiler_target == CompilerTargetState::Unknown));
    Ok(())
}

#[test]
fn json_collect_rejects_stale_sources() -> anyhow::Result<()> {
    let (_dir, mut request, _) = collected()?;
    request["sources"][0]["text"] = json!("fn forged() {}\n");
    assert!(
        example::execute(request).is_err(),
        "JSON wiring must enforce P1 graph binding"
    );
    Ok(())
}

// Explicitly synthetic protocol control; fresh compiler acceptance is performed
// by the capture script and is mandatory, separate from this default test.
fn import_request() -> anyhow::Result<Value> {
    let (dir, collect, output) = collected()?;
    let raw = "{\"roots\":[],\"nodes\":[],\"edges\":{}}";
    let corpus = corpus();
    let inputs: Vec<_> = corpus["files"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, text)| json!({"path":path,"text":text}))
        .collect();
    let inventory: Vec<_> = corpus["files"].as_object().unwrap().iter().map(|(path,text)| {
        let kind = if path.ends_with(".rs") {"source"} else if path.ends_with("Cargo.lock") {"lockfile"} else if path.ends_with("Cargo.toml") {"manifest"} else {"config"};
        json!({"path":path,"kind":kind,"sha256":sha256_bytes(text.as_str().unwrap().as_bytes())})
    }).collect();
    let identity: CaptureBuildIdentity = serde_json::from_value(json!({
        "schema_version":CAPTURE_BUILD_SCHEMA_VERSION,"scope":"bounded_fixture","provenance":"synthetic_fixture",
        "graph_content_hash":collect["graph"]["graph_content_hash"],"capture_root":dir.path().to_str().unwrap(),
        "inputs":inventory,"rustc_verbose":"rustc synthetic\nbinary: rustc\ncommit-hash: synthetic\ncommit-date: synthetic\nhost: synthetic\nrelease: synthetic-nightly\nLLVM version: synthetic\n",
        "toolchain":CGA_TOOLCHAIN,"host":"synthetic","target":"synthetic","exporter_revision":CGA_REVISION,
        "producer":{"build":{"state":"unmodified"},"cargo_cga_sha256":sha256_bytes(b"synthetic launcher"),"cga_driver_sha256":sha256_bytes(b"synthetic driver")},
        "packages":["panic-pilot","pilot-dep"],"targets":["lib:panic_pilot","bin:panic-pilot","lib:pilot_dep"],
        "features":[],"default_features":false,"all_features":false,"profile":"dev","cfg_flags":[],
        "compiler_flags":["-Copt-level=0","-Cpanic=unwind"],"cargo_args":["check","--frozen"],
        "environment":CAPTURE_ENV_ALLOWLIST.iter().map(|k| ((*k).to_owned(),Value::Null)).collect::<serde_json::Map<_,_>>(),
        "capture_command":["synthetic"],"raw_export_sha256":sha256_bytes(raw.as_bytes())
    }))?;
    Ok(
        json!({"command":"import","graph":collect["graph"],"sources":collect["sources"],"method_sites":output["method_sites"],
        "inputs":inputs,"identity":identity,"expected_build_id":identity.build_id()?,"raw_export":raw}),
    )
}

#[test]
fn json_import_validate_roundtrip_and_explicit_capabilities() -> anyhow::Result<()> {
    let mut request = import_request()?;
    let out = example::execute(request.clone())?;
    assert!(
        out["compiler_evidence"].is_object(),
        "import returns validated compiler evidence"
    );
    assert_eq!(
        out["compiler_evidence"]["capabilities"]["macro_expansion"],
        "unavailable"
    );
    request["command"] = json!("validate");
    request["compiler_evidence"] = out["compiler_evidence"].clone();
    assert_eq!(example::execute(request.clone())?["valid"], true);
    request["compiler_evidence"]["build_id"] = json!("forged");
    assert!(
        example::execute(request).is_err(),
        "serialized evidence is recollected before trust"
    );
    Ok(())
}

#[test]
fn json_import_rejects_source_config_checksum_reference_and_ownership_corruption(
) -> anyhow::Result<()> {
    let request = import_request()?;
    for field in ["source", "config", "checksum", "reference", "ownership"] {
        let mut changed = request.clone();
        match field {
            "source" => changed["inputs"][0]["text"] = json!("changed"),
            "config" => changed["identity"]["compiler_flags"] = json!(["-Coverflow-checks=on"]),
            "checksum" => changed["raw_export"] = json!("{}"),
            "reference" => {
                let raw = "{\"roots\":[99],\"nodes\":[],\"edges\":{}}";
                changed["raw_export"] = json!(raw);
                changed["identity"]["raw_export_sha256"] = json!(sha256_bytes(raw.as_bytes()));
                let id: CaptureBuildIdentity = serde_json::from_value(changed["identity"].clone())?;
                changed["expected_build_id"] = json!(id.build_id()?);
            }
            "ownership" => {
                changed["method_sites"]["sites"][0]["caller_stable_symbol_id"] = json!("forged")
            }
            _ => unreachable!(),
        }
        assert!(
            example::execute(changed).is_err(),
            "reject {field} corruption before association"
        );
    }
    Ok(())
}

#[test]
fn capture_script_exposes_the_bounded_wrapper_only_command_contract() -> anyhow::Result<()> {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/experiments/panic-evidence-capture.py");
    let output = std::process::Command::new("python3")
        .arg(script)
        .arg("--help")
        .output()?;
    assert!(
        output.status.success(),
        "capture script must be executable: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout)?;
    for flag in [
        "--work-dir",
        "--cga-driver",
        "--cargo-cga",
        "--producer-patch",
        "--expected-driver-sha256",
        "--expected-launcher-sha256",
        "--example",
    ] {
        assert!(help.contains(flag), "capture contract exposes {flag}");
    }
    Ok(())
}

#[test]
fn capture_prepare_recreates_exact_inputs_and_independent_configuration_flags() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let work = root.path().join("capture");
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/experiments/panic-evidence-capture.py");
    let output = std::process::Command::new("python3")
        .arg(&script)
        .arg("--prepare-only")
        .arg("--work-dir")
        .arg(&work)
        .output()?;
    assert!(
        output.status.success(),
        "bounded preparation succeeds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(plan["configurations"].as_array().unwrap().len(), 5);
    assert_eq!(plan["inputs"].as_array().unwrap().len(), 7);
    for (path, text) in corpus()["files"].as_object().unwrap() {
        assert_eq!(
            std::fs::read_to_string(work.join("fixture").join(path))?,
            text.as_str().unwrap()
        );
    }
    let configs = plan["configurations"].as_array().unwrap();
    for (i, c) in configs.iter().enumerate() {
        let flags = c["compiler_flags"].as_array().unwrap();
        assert!(flags.contains(&json!("-Copt-level=0")));
        assert!(flags.contains(&json!("-Cpanic=unwind")));
        let expected = corpus()["configurations"][i].clone();
        assert!(flags.contains(&json!(format!(
            "-Cdebug-assertions={}",
            if expected["debug_assertions"] == true {
                "on"
            } else {
                "off"
            }
        ))));
        assert!(flags.contains(&json!(format!(
            "-Coverflow-checks={}",
            if expected["overflow_checks"] == true {
                "on"
            } else {
                "off"
            }
        ))));
    }
    let again = std::process::Command::new("python3")
        .arg(script)
        .arg("--prepare-only")
        .arg("--work-dir")
        .arg(work)
        .output()?;
    assert!(
        !again.status.success(),
        "capture refuses to overwrite an existing directory"
    );
    Ok(())
}

#[test]
fn frozen_held_out_compiler_witness_verifies_scalar_columns_and_exclusive_end() -> anyhow::Result<()>
{
    let corpus = corpus();
    let witness = &corpus["compiler_witness"];
    assert_eq!(
        witness["provenance"], "compiler_capture_partial_projection",
        "fresh real compiler witness must be retained; no synthetic substitute"
    );
    assert_eq!(witness["exporter_revision"], CGA_REVISION);
    for (path, checksum) in witness["source_sha256"].as_object().unwrap() {
        assert_eq!(
            sha256_bytes(corpus["files"][path].as_str().unwrap().as_bytes()),
            checksum.as_str().unwrap()
        );
    }
    let source = corpus["files"]["src/lib.rs"].as_str().unwrap();
    let location: CgaLocation =
        serde_json::from_value(witness["unicode_observation"]["edge"]["location"].clone())?;
    let range = location.span.to_byte_range(source.as_bytes())?;
    assert_eq!(
        &source[range[0]..range[1]],
        "x.expect(\"unicode\")",
        "actual exporter endpoints select the full call and exclude trailing space/brace"
    );
    assert_eq!(witness["configurations"].as_array().unwrap().len(), 5);
    let (_dir, _, output) = collected()?;
    let sites: MethodCallSiteEvidenceSet = serde_json::from_value(output["method_sites"].clone())?;
    for config in witness["configurations"].as_array().unwrap() {
        assert_eq!(config["runtime_controls_passed"], 9);
        assert!(config["raw_export_sha256"].as_str().unwrap().len() == 64);
        let cases = &config["cases"];
        for (case, count) in [
            ("options", 4),
            ("results", 4),
            ("custom", 2),
            ("chain", 2),
            ("generic", 2),
            ("unicode", 1),
            ("expanded", 1),
            ("dependency", 1),
            ("indexing", 1),
        ] {
            assert_eq!(
                cases[case].as_array().unwrap().len(),
                count,
                "retained actual {case} observations"
            );
        }
        let name = config["name"].as_str().unwrap();
        assert_eq!(
            cases["addition"].as_array().unwrap().len(),
            usize::from(name.contains("o1"))
        );
        assert_eq!(
            cases["debug_control"].as_array().unwrap().len(),
            usize::from(name.starts_with("d1"))
        );
        assert_eq!(
            cases["feature_control"].as_array().unwrap().len(),
            usize::from(name.ends_with("extra"))
        );
        let expanded = &cases["expanded"][0];
        assert_eq!(expanded["association"]["state"], "unmapped");
        assert_eq!(expanded["association"]["reason"], "no_site_at_span");
        assert_eq!(
            cases["generic"][0]["association"]["site_id"],
            cases["generic"][1]["association"]["site_id"]
        );
        assert_ne!(
            cases["generic"][0]["raw_caller_node"]["name"],
            cases["generic"][1]["raw_caller_node"]["name"]
        );
        for row in cases["custom"].as_array().unwrap() {
            assert!(row["raw_target_node"]["name"]
                .as_str()
                .unwrap()
                .starts_with("panic_pilot::Safe::"));
        }
        for rows in cases.as_object().unwrap().values() {
            for row in rows.as_array().unwrap() {
                if let Some(site_id) = row["association"]["site_id"].as_str() {
                    let site = sites.sites.iter().find(|s| s.site_id == site_id).unwrap();
                    let loc: CgaLocation =
                        serde_json::from_value(row["raw_edge"]["location"].clone())?;
                    let text = corpus["files"][&site.file_path].as_str().unwrap();
                    let range = loc.span.to_byte_range(text.as_bytes())?;
                    assert!(site.byte_range[0] <= range[0] && range[1] <= site.byte_range[1]);
                    if row["association"]["state"] == "exact" {
                        assert_eq!(range, site.byte_range);
                    }
                }
            }
        }
    }
    Ok(())
}
