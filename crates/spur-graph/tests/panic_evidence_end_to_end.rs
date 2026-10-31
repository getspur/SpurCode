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
    let mut inventory: Vec<_> = corpus["files"].as_object().unwrap().iter().map(|(path,text)| {
        let kind = if path.ends_with(".rs") {"source"} else if path.ends_with("Cargo.lock") {"lockfile"} else if path.ends_with("Cargo.toml") {"manifest"} else {"config"};
        json!({"path":path,"kind":kind,"sha256":sha256_bytes(text.as_str().unwrap().as_bytes())})
    }).collect();
    inventory.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
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
            "source" => {
                assert_eq!(request["identity"]["inputs"][0]["kind"], "config");
                let input = request["identity"]["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["kind"] == "source" && r["path"].as_str().unwrap().ends_with(".rs"))
                    .expect("source control needs a physical Rust identity input");
                let path = input["path"].as_str().unwrap();
                let row = changed["inputs"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["path"] == path)
                    .expect("identity path must exist in request inputs");
                let text = row["text"].as_str().unwrap();
                assert_eq!(sha256_bytes(text.as_bytes()), input["sha256"]);
                row["text"] = json!(format!("{text} "));
                assert_eq!(changed["identity"], request["identity"]);
                let mut restored = changed.clone();
                restored["inputs"] = request["inputs"].clone();
                assert_eq!(restored, request);
                assert_eq!(
                    changed["inputs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(request["inputs"].as_array().unwrap())
                        .filter(|(a, b)| a != b)
                        .count(),
                    1
                );
            }
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
                let symbols = request["graph"]["symbols"].as_array().unwrap();
                let unique_function = |id: &Value, path: &Value| {
                    let matches: Vec<_> = symbols
                        .iter()
                        .filter(|s| s["stable_symbol_id"] == *id)
                        .collect();
                    matches.len() == 1
                        && matches[0]["file_path"] == *path
                        && matches!(
                            matches[0]["symbol_kind"].as_str(),
                            Some("function" | "method")
                        )
                };
                let (index, replacement) = request["method_sites"]["sites"].as_array().unwrap().iter().enumerate()
                    .find_map(|(index, site)| {
                        let original = &site["caller_stable_symbol_id"];
                        let path = &site["file_path"];
                        if !unique_function(original, path) { return None; }
                        symbols.iter().find(|s| s["stable_symbol_id"] != *original && unique_function(&s["stable_symbol_id"], path))
                            .map(|s| (index, s["stable_symbol_id"].clone()))
                    }).expect("ownership control needs two unique existing function/method callers in the same file");
                let site = &request["method_sites"]["sites"][index];
                assert!(unique_function(
                    &site["caller_stable_symbol_id"],
                    &site["file_path"]
                ));
                assert!(unique_function(&replacement, &site["file_path"]));
                assert_ne!(site["caller_stable_symbol_id"], replacement);
                changed["method_sites"]["sites"][index]["caller_stable_symbol_id"] = replacement;
                let mut restored = changed.clone();
                restored["method_sites"]["sites"][index]["caller_stable_symbol_id"] =
                    site["caller_stable_symbol_id"].clone();
                assert_eq!(restored, request, "only the caller identity changes");
            }
            _ => unreachable!(),
        }
        let error = example::execute(changed)
            .expect_err(&format!("reject {field} corruption before association"));
        if field == "source" {
            assert!(
                error.to_string().contains("input checksum"),
                "source byte binding must cause rejection: {error:#}"
            );
        }
    }
    Ok(())
}

// Test capture()'s actual control construction without requiring a compiler in
// the portable suite. Only process/producer I/O is replaced. This is not the
// mandatory real-export acceptance, which runs separately with the pinned CGA.
fn capture_control_contract(control: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/experiments/panic-evidence-capture.py");
    let mut child = Command::new("python3")
        .args(["-B", "-c", CAPTURE_CONTROL_CONTRACT])
        .arg(script)
        .arg(control)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    serde_json::to_writer(child.stdin.as_mut().unwrap(), &import_request()?)?;
    child.stdin.take().unwrap().flush()?;
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "capture {control} mutation contract: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn capture_source_control_changes_only_physical_source_with_config_first() -> anyhow::Result<()> {
    capture_control_contract("source")
}

#[test]
fn capture_ownership_control_uses_existing_different_same_file_caller() -> anyhow::Result<()> {
    capture_control_contract("ownership")
}

const CAPTURE_CONTROL_CONTRACT: &str = r#"
import copy, importlib.util, json, subprocess, sys, tempfile
from pathlib import Path
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location('capture', sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
good = json.load(sys.stdin)
control = sys.argv[2]

class Checked(Exception): pass

def check_request(bad, request):
    if control == 'source':
        assert request['identity']['inputs'][0]['kind'] == 'config'
        before = {r['path']: r for r in request['inputs']}
        changed = [r for r in bad['inputs'] if r != before[r['path']]]
        assert len(changed) == 1, 'exactly one input must change'
        path = changed[0]['path']
        identity = next(r for r in request['identity']['inputs'] if r['path'] == path)
        assert identity['kind'] == 'source', 'source control mutated config instead of source'
        assert path.endswith('.rs') and (fixture / path).is_file()
        assert m.digest((fixture / path).read_bytes()) == identity['sha256']
        assert changed[0]['text'] == before[path]['text'] + ' '
        expected = copy.deepcopy(request)
        next(r for r in expected['inputs'] if r['path'] == path)['text'] += ' '
    else:
        original = {s['site_id']: s for s in request['method_sites']['sites']}
        changed = [s for s in bad['method_sites']['sites'] if s != original[s['site_id']]]
        assert len(changed) == 1, 'exactly one site must change'
        site = changed[0]
        old = original[site['site_id']]['caller_stable_symbol_id']
        new = site['caller_stable_symbol_id']
        for caller in (old, new):
            symbols = [s for s in request['graph']['symbols'] if s['stable_symbol_id'] == caller]
            assert len(symbols) == 1, 'both callers must be unique existing graph symbols'
            assert symbols[0]['file_path'] == site['file_path']
            assert symbols[0]['symbol_kind'] in ('function', 'method')
        assert old != new
        expected = copy.deepcopy(request)
        next(s for s in expected['method_sites']['sites'] if s['site_id'] == site['site_id'])['caller_stable_symbol_id'] = new
    assert bad == expected, 'control changed unrelated identity, graph, source, span or site fields'

def example(binary, request, work, env, directory, name, success=True):
    if name == 'snapshot': return {'graph': good['graph'], 'method_sites': good['method_sites']}
    if name.endswith('identity'): return {'build_id': 'portable-control-only'}
    if name == 'import':
        requests.append(copy.deepcopy(request))
        return {'compiler_evidence': {}}
    if name == 'validate': return {'valid': True}
    if name == 'negative-' + control:
        check_request(request, requests[0])
        raise Checked()
    return {'rejected': True}

def run(argv, cwd, env, directory, name, *args, **kwargs):
    if name == 'rustc-identity':
        return subprocess.CompletedProcess(argv, 0, ('commit-hash: ' + m.RUSTC_COMMIT + '\nhost: portable\n').encode(), b'')
    assert name == 'compiler'
    path = directory / 'target' / 'portable.json'
    path.parent.mkdir(parents=True)
    path.write_text(json.dumps({'roots': [], 'nodes': [{'name': 'panic_pilot::main'}], 'edges': {}}))
    return subprocess.CompletedProcess(argv, 0, b'', ('cga-emit:' + str(path) + '\n').encode())

with tempfile.TemporaryDirectory() as tmp:
    work = Path(tmp).resolve() / 'capture'
    fixture, plan = m.prepare(work)
    requests = []
    binary = Path(sys.executable).resolve()
    pins = {'cga_driver_sha256': m.digest(binary.read_bytes()), 'cargo_cga_sha256': m.digest(binary.read_bytes())}
    m.producer = lambda args, work: (binary, pins, binary)
    m.closed_environment = lambda work: {}
    m.run, m.example = run, example
    args = SimpleNamespace(**{k: str(binary) for k in ('cga_driver', 'cargo_cga', 'cga_source', 'expected_driver_sha256', 'expected_launcher_sha256', 'example')})
    try:
        m.capture(args, work, fixture, plan)
    except Checked:
        pass
    else:
        raise AssertionError('capture did not construct the requested negative control')
"#;

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
