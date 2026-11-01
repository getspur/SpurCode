use serde_json::{json, Value};
use spur_graph::panic_evidence::*;
use spur_graph::{artifact_from_facts, build_facts, GraphIndexArtifact};

const HELD_SOURCE: &str = include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/fixtures/compiler/lib.rs");
const HELD_RAW: &[u8] = include_bytes!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/capture/compiler.callgraph.json");
const MANIFEST: &[u8] =
    b"[package]\nname=\"grounding_compiler\"\nversion=\"0.1.0\"\nedition=\"2021\"\n";
const LOCK: &[u8] = b"version = 4\n";

struct Fixture {
    source: String,
    path: String,
    raw: Vec<u8>,
    graph: GraphIndexArtifact,
    methods: MethodCallSiteEvidenceSet,
    calls: RustCallSiteEvidenceSet,
    facts: PanicSemanticFacts,
    identity: CaptureBuildIdentity,
    binding: PanicCompilerConfigBinding,
}

impl Fixture {
    fn held() -> anyhow::Result<Self> {
        assert_eq!(
            sha256_bytes(HELD_SOURCE.as_bytes()),
            "d5b043f5f88a8a7373b0b9a87fa0e901dee74142532bd677131db429bcff24a0"
        );
        Self::new(HELD_SOURCE, "lib.rs", HELD_RAW.to_vec(), true)
    }

    fn synthetic() -> anyhow::Result<Self> {
        let v: Value =
            serde_json::from_str(include_str!("fixtures/panic_evidence/cga-format.json"))?;
        Self::new(
            v["source"].as_str().unwrap(),
            "src/lib.rs",
            serde_json::to_vec(&v["export"])?,
            false,
        )
    }

    fn new(source: &str, path: &str, raw: Vec<u8>, held: bool) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(file, source)?;
        let (f, _) = build_facts(dir.path(), None)?;
        let graph = artifact_from_facts(&f, dir.path())?;
        let sources = [PanicEvidenceSource {
            file_path: path,
            bytes: source.as_bytes(),
        }];
        let methods = collect_method_call_sites(&graph, &sources)?;
        let calls = collect_rust_call_sites(&graph, &sources)?;
        let verbose = include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/capture/rustc-version.stdout");
        let provenance: Value = serde_json::from_str(include_str!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/capture/provenance.json"))?;
        let flags: Vec<String> = serde_json::from_value(provenance["compiler_flags"].clone())?;
        let patch = include_bytes!("../../../docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/capture/cga-diff.stdout");
        let build: PanicBuildConfig = serde_json::from_value(json!({
            "schema_version":PANIC_BUILD_CONFIG_SCHEMA_VERSION,"graph_content_hash":graph.graph_content_hash,
            "target":"aarch64-apple-darwin","edition":"2021","cfg":[],"overflow_checks":true,
            "debug_assertions":true,"panic_strategy":"unwind","rustc_verbose":verbose,"codegen_flags":flags,"features":[]
        }))?;
        let facts = collect_panic_semantics(&graph, &sources, &build)?;
        // This is a replay envelope for frozen raw bytes, not a new capture.
        let identity: CaptureBuildIdentity = serde_json::from_value(json!({
            "schema_version":CAPTURE_BUILD_SCHEMA_VERSION,"scope":"bounded_fixture",
            "provenance":if held {"compiler_capture"} else {"synthetic_fixture"},
            "graph_content_hash":graph.graph_content_hash,
            "capture_root":if held {"/Users/kevintruong/project/SpurCode/.spur/worktrees/28041184-f799-4627-922f-b1c33170f239/docs/research/artifacts/2026-11-01-panic-implementation-grounding/grounding/fixtures/compiler"} else {"/fixture"},
            "inputs":[{"path":path,"kind":"source","sha256":sha256_bytes(source.as_bytes())},
                {"path":"Cargo.toml","kind":"manifest","sha256":sha256_bytes(MANIFEST)},
                {"path":"Cargo.lock","kind":"lockfile","sha256":sha256_bytes(LOCK)}],
            "rustc_verbose":verbose,"toolchain":CGA_TOOLCHAIN,"host":"aarch64-apple-darwin","target":"aarch64-apple-darwin",
            "exporter_revision":CGA_REVISION,"producer":{"build":{"state":"patched","patch_sha256":sha256_bytes(patch)},
                "cargo_cga_sha256":"7f84bc715138c09744fd351859ca02b240ea768d9756069229e9250fa9447b7a",
                "cga_driver_sha256":"5647b7f2c7f857dda0dd0204e91d870287988181ca5ec4d7792810a561886172"},
            "packages":["grounding_compiler"],"targets":["lib:grounding_compiler"],"features":[],"default_features":true,"all_features":false,
            "profile":"dev","cfg_flags":[],"compiler_flags":flags,"cargo_args":["rustc","--lib"],
            "environment":CAPTURE_ENV_ALLOWLIST.iter().map(|k|((*k).to_owned(),Value::Null)).collect::<serde_json::Map<_,_>>(),
            "capture_command":["frozen-raw-replay"],"raw_export_sha256":sha256_bytes(&raw)
        }))?;
        let binding = PanicCompilerConfigBinding {
            capture_build_id: identity.build_id()?,
            semantic_build_hash: build.content_hash()?,
        };
        Ok(Self {
            source: source.into(),
            path: path.into(),
            raw,
            graph,
            methods,
            calls,
            facts,
            identity,
            binding,
        })
    }

    fn sources(&self) -> [PanicEvidenceSource<'_>; 1] {
        [PanicEvidenceSource {
            file_path: &self.path,
            bytes: self.source.as_bytes(),
        }]
    }

    fn inputs(&self) -> [CaptureInputBytes<'_>; 3] {
        [
            CaptureInputBytes {
                path: &self.path,
                bytes: self.source.as_bytes(),
            },
            CaptureInputBytes {
                path: "Cargo.toml",
                bytes: MANIFEST,
            },
            CaptureInputBytes {
                path: "Cargo.lock",
                bytes: LOCK,
            },
        ]
    }

    fn report(&self) -> anyhow::Result<PanicCompilerValidation> {
        let sources = self.sources();
        let inputs = self.inputs();
        let import = || CgaImportRequest {
            identity: &self.identity,
            expected_build_id: &self.binding.capture_build_id,
            raw_export: &self.raw,
            inputs: &inputs,
            graph: &self.graph,
            sources: &sources,
            method_sites: &self.methods,
        };
        let compiler = import_cga(import())?;
        self.correlate(&compiler, &self.calls, &self.facts, &self.binding)
    }

    fn correlate(
        &self,
        compiler: &CompilerEvidenceSet,
        calls: &RustCallSiteEvidenceSet,
        facts: &PanicSemanticFacts,
        binding: &PanicCompilerConfigBinding,
    ) -> anyhow::Result<PanicCompilerValidation> {
        let sources = self.sources();
        let inputs = self.inputs();
        correlate_panic_compiler(PanicCompilerValidationRequest {
            import: CgaImportRequest {
                identity: &self.identity,
                expected_build_id: &self.binding.capture_build_id,
                raw_export: &self.raw,
                inputs: &inputs,
                graph: &self.graph,
                sources: &sources,
                method_sites: &self.methods,
            },
            compiler,
            calls,
            facts,
            expected_build: &self.facts.build,
            config_binding: binding,
        })
    }
}

#[test]
fn held_out_capture_retains_exact_instances_categories_and_uncertainty() -> anyhow::Result<()> {
    let f = Fixture::held()?;
    let r = f.report()?;
    assert_eq!(r.compiler.raw_graph.nodes.len(), 6);
    assert_eq!(r.compiler.observations.len(), 7);
    assert_eq!(r.called_observations, vec![0, 1, 5]);
    assert_eq!(r.virtual_observations, vec![6]);
    assert_eq!(r.address_observations, vec![4]);
    assert_eq!(r.implicit_assert_observations, vec![2, 3]);
    assert!(r.async_panic_observations.is_empty());
    assert_eq!(
        r.compiler.raw_graph.nodes[5].name,
        "<() as grounding_compiler::Concrete>::read"
    );
    assert!(r.instances[5].body_id.is_some());
    assert!(
        r.instances[1].body_id.is_none(),
        "trait declaration is not an executable body"
    );
    assert_eq!(r.retained_unknown_obligations, f.facts.unknown_obligations);
    assert!(r.cleared_obligations.is_empty());
    assert_eq!(
        r.semantic_completeness,
        PanicCompilerCompleteness::Unestablished
    );
    assert_eq!(r.correlations[0].call_site_ids.len(), 1);
    assert!(
        r.correlations[4].call_site_ids.is_empty(),
        "address cast never becomes a call"
    );
    assert!(
        r.correlations[6].call_site_ids.is_empty(),
        "enumeration never proves dispatch"
    );
    assert!(r
        .limitations
        .contains(&PanicCompilerLimitation::GenericVirtualProducerTrust));
    println!("R7_REPORT={}", serde_json::to_string(&r)?);
    println!("R7_CALLS={}", serde_json::to_string(&f.calls)?);
    println!("R7_FACTS={}", serde_json::to_string(&f.facts)?);
    Ok(())
}

#[test]
fn macro_definition_is_retained_without_an_invocation_or_payload() -> anyhow::Result<()> {
    let f = Fixture::held()?;
    let r = f.report()?;
    let c = &r.correlations[1];
    assert_eq!(c.location_kind, PanicCompilerLocationKind::MacroDefinition);
    assert!(c.call_site_ids.is_empty() && c.semantic_site_ids.is_empty() && c.body_id.is_none());
    assert_eq!(r.compiler.observations[1].raw_location.span.start.line, 6);
    assert_eq!(
        r.compiler.observations[1].canonical_callee.as_deref(),
        Some("grounding_compiler::expanded_target")
    );
    for i in [2, 3] {
        assert_eq!(r.compiler.observations[i].kind, CgaEdgeKind::BuiltinAssert);
        assert!(r.compiler.observations[i].canonical_callee.is_none());
    }
    assert!(r
        .limitations
        .contains(&PanicCompilerLimitation::AssertReasonUnavailable));
    assert!(r
        .limitations
        .contains(&PanicCompilerLimitation::ExpansionInvocationUnavailable));
    Ok(())
}

#[test]
fn existing_synthetic_capture_remains_compatible_and_explicit() -> anyhow::Result<()> {
    let f = Fixture::synthetic()?;
    let r = f.report()?;
    assert!(r
        .limitations
        .contains(&PanicCompilerLimitation::SyntheticCapture));
    assert_eq!(
        r.compiler.identity.provenance,
        ExportProvenance::SyntheticFixture
    );
    assert!(r.called_observations.len() > 1);
    assert!(r.cleared_obligations.is_empty());
    println!("R7_SYNTHETIC={}", serde_json::to_string(&r)?);
    Ok(())
}

#[test]
fn forged_report_references_owners_sources_and_obligations_are_rejected() -> anyhow::Result<()> {
    let f = Fixture::held()?;
    let r = f.report()?;
    let original = serde_json::to_value(&r)?;
    for (pointer, value) in [
        ("/compiler/observations/0/from_node", json!(5)),
        ("/compiler/observations/0/edge_index", json!(999)),
        ("/compiler/observations/0/kind", json!({"FnPtrCast":1})),
        ("/compiler/observations/0/canonical_callee", json!("forged")),
        ("/instances/0/body_id", json!(r.instances[5].body_id)),
        ("/correlations/0/body_id", json!(r.instances[5].body_id)),
        ("/correlations/0/call_site_ids", json!(["forged"])),
        ("/semantic_build_hash", json!("forged")),
        ("/cleared_obligations", json!(["forged"])),
        ("/retained_unknown_obligations", json!([])),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        let changed: PanicCompilerValidation = serde_json::from_value(changed)?;
        let sources = f.sources();
        let inputs = f.inputs();
        let result = changed.validate(PanicCompilerValidationRequest {
            import: CgaImportRequest {
                identity: &f.identity,
                expected_build_id: &f.binding.capture_build_id,
                raw_export: &f.raw,
                inputs: &inputs,
                graph: &f.graph,
                sources: &sources,
                method_sites: &f.methods,
            },
            compiler: &r.compiler,
            calls: &f.calls,
            facts: &f.facts,
            expected_build: &f.facts.build,
            config_binding: &f.binding,
        });
        assert!(result.is_err(), "accepted forged {pointer}");
        println!(
            "R7_REJECTION={}",
            json!({"case":pointer,"rejected":result.is_err()})
        );
    }
    Ok(())
}

#[test]
fn independently_bound_build_and_canonical_source_facts_are_required() -> anyhow::Result<()> {
    let f = Fixture::held()?;
    let r = f.report()?;
    let mut calls = f.calls.clone();
    calls.sites[0].body_id = f
        .calls
        .bodies
        .iter()
        .find(|b| b.body_id != calls.sites[0].body_id)
        .unwrap()
        .body_id
        .clone();
    assert!(f
        .correlate(&r.compiler, &calls, &f.facts, &f.binding)
        .is_err());
    let mut facts = f.facts.clone();
    facts.sources[0].source_oid = "forged".into();
    assert!(f
        .correlate(&r.compiler, &f.calls, &facts, &f.binding)
        .is_err());
    for field in ["capture_build_id", "semantic_build_hash"] {
        let mut b = serde_json::to_value(&f.binding)?;
        b[field] = json!("forged");
        let b = serde_json::from_value(b)?;
        assert!(f.correlate(&r.compiler, &f.calls, &f.facts, &b).is_err());
    }
    for (field, value) in [
        ("target", json!("wrong")),
        ("overflow_checks", json!(false)),
        ("debug_assertions", json!(false)),
        ("edition", json!("2024")),
        ("panic_strategy", json!("abort")),
        ("cfg", json!([{"key":"wrong","value":null}])),
        ("features", json!(["wrong"])),
        ("codegen_flags", json!([])),
        ("rustc_verbose", json!("wrong")),
    ] {
        let mut facts = serde_json::to_value(&f.facts)?;
        facts["build"][field] = value;
        let mut facts: PanicSemanticFacts = serde_json::from_value(facts)?;
        facts.build_hash = facts.build.content_hash()?;
        let result = f.correlate(&r.compiler, &f.calls, &facts, &f.binding);
        assert!(result.is_err(), "accepted config {field}");
        println!(
            "R7_REJECTION={}",
            json!({"case":field,"rejected":result.is_err()})
        );
    }
    Ok(())
}

#[test]
fn stale_raw_export_and_invalid_edge_references_are_rejected() -> anyhow::Result<()> {
    let mut f = Fixture::held()?;
    f.raw.push(b' ');
    assert!(f.report().is_err());
    let mut f = Fixture::held()?;
    let mut raw: Value = serde_json::from_slice(&f.raw)?;
    raw["edges"]["0"][0]["kind"] = json!({"Called":999});
    f.raw = serde_json::to_vec(&raw)?;
    f.identity.raw_export_sha256 = sha256_bytes(&f.raw);
    f.binding.capture_build_id = f.identity.build_id()?;
    assert!(f.report().is_err());
    Ok(())
}

#[test]
fn free_calls_implicit_operations_and_async_categories_keep_raw_references() -> anyhow::Result<()> {
    let source = "fn helper(x:u8)->u8{x}\nfn entry(x:u8)->u8{let y=helper(x); [1u8,2u8][x] + y}\n";
    let loc = |line, start, end| json!({"path":"src/lib.rs","span":{"start":{"line":line,"column":start},"end":{"line":line,"column":end}}});
    let raw = serde_json::to_vec(
        &json!({"roots":[0],"nodes":[{"name":"f::entry","location":loc(2,1,18)},
        {"name":"f::helper","location":loc(1,1,19)}],"edges":{"0":[
        {"kind":{"Called":1},"location":loc(2,26,35)},
        {"kind":"BuiltinAssert","location":loc(2,37,49)},
        {"kind":"AsyncPanic","location":loc(2,37,53)}]}}),
    )?;
    let f = Fixture::new(source, "src/lib.rs", raw, false)?;
    let r = f.report()?;
    assert_eq!(r.correlations[0].call_site_ids.len(), 1);
    assert_eq!(r.correlations[1].implicit_operation_ids.len(), 1);
    assert_eq!(r.async_panic_observations, vec![2]);
    assert_eq!(r.compiler.observations[2].kind, CgaEdgeKind::AsyncPanic);
    assert!(r.correlations[2].implicit_operation_ids.is_empty());
    println!("R7_FREE={}", serde_json::to_string(&r)?);
    Ok(())
}

#[test]
fn conflicting_long_form_check_flags_cannot_be_rebound_as_agreement() -> anyhow::Result<()> {
    for flags in [
        vec!["--codegen=overflow-checks=off"],
        vec!["--codegen", "overflow-checks=off"],
    ] {
        let mut f = Fixture::held()?;
        let mut build = f.facts.build.clone();
        build.codegen_flags = flags.into_iter().map(String::from).collect();
        f.identity.compiler_flags = build.codegen_flags.clone();
        f.facts = collect_panic_semantics(&f.graph, &f.sources(), &build)?;
        f.binding.capture_build_id = f.identity.build_id()?;
        f.binding.semantic_build_hash = build.content_hash()?;
        let result = f.report();
        assert!(
            result.is_err(),
            "accepted explicit overflow=off with semantic checks=true"
        );
        println!(
            "R7_REJECTION={}",
            json!({"case":build.codegen_flags,"rejected":result.is_err()})
        );
    }
    Ok(())
}

// Hand-authored compiler syntax controls, never generated from correlation output.
const CHECK_CASES: &str = include_str!(
    "../../../docs/research/artifacts/2026-11-01-panic-implementation/R7/rework2/check-cases.json"
);

fn check_flag_case(case: &Value) -> anyhow::Result<()> {
    let mut f = Fixture::held()?;
    let mut build = f.facts.build.clone();
    build.codegen_flags = serde_json::from_value(case["flags"].clone())?;
    match case["key"].as_str().unwrap() {
        "overflow_checks" => build.overflow_checks = case["semantic"].as_bool().unwrap(),
        "debug_assertions" => build.debug_assertions = case["semantic"].as_bool().unwrap(),
        _ => unreachable!(),
    }
    // Rebind both independent identities so only explicit compiler agreement is tested.
    f.identity.compiler_flags = build.codegen_flags.clone();
    f.facts = collect_panic_semantics(&f.graph, &f.sources(), &build)?;
    f.binding.capture_build_id = f.identity.build_id()?;
    f.binding.semantic_build_hash = build.content_hash()?;
    let result = f.report();
    println!(
        "R7_CHECK={}",
        json!({"case":case,"build":build,"binding":f.binding,
            "accepted":result.is_ok(),"report":result.as_ref().ok(),
            "error":result.as_ref().err().map(|e|format!("{e:#}"))})
    );
    assert_eq!(
        result.is_ok(),
        case["accepted"].as_bool().unwrap(),
        "explicit check agreement for {} with flags {}: {:?}",
        case["id"],
        case["flags"],
        result.as_ref().err()
    );
    if let Ok(report) = result {
        assert_eq!(report.compiler.identity.compiler_flags, build.codegen_flags);
        assert_eq!(report.semantic_build_hash, build.content_hash()?);
        assert_eq!(
            report.retained_unknown_obligations,
            f.facts.unknown_obligations
        );
        assert!(report.cleared_obligations.is_empty());
        assert_eq!(report.called_observations, vec![0, 1, 5]);
        assert_eq!(report.address_observations, vec![4]);
        assert_eq!(report.virtual_observations, vec![6]);
        assert_eq!(report.implicit_assert_observations, vec![2, 3]);
    }
    Ok(())
}

fn review_flag_case(id: &str) -> anyhow::Result<()> {
    let cases: Vec<Value> = serde_json::from_str(CHECK_CASES)?;
    check_flag_case(cases.iter().find(|c| c["id"] == id).unwrap())
}

#[test]
fn review_flag_underscore_overflow() -> anyhow::Result<()> {
    review_flag_case("underscore_overflow")
}

#[test]
fn review_flag_underscore_debug() -> anyhow::Result<()> {
    review_flag_case("underscore_debug")
}

#[test]
fn review_flag_bare_overflow() -> anyhow::Result<()> {
    review_flag_case("bare_overflow")
}

#[test]
fn review_flag_bare_debug() -> anyhow::Result<()> {
    review_flag_case("bare_debug")
}

#[test]
fn review_flag_alias_last_wins() -> anyhow::Result<()> {
    review_flag_case("alias_last_wins")
}

#[test]
fn check_flag_forms_matching_values_and_mixed_order_controls() -> anyhow::Result<()> {
    let cases: Vec<Value> = serde_json::from_str(CHECK_CASES)?;
    for case in cases.iter().filter(|c| c["review"] == false) {
        check_flag_case(case)?;
    }
    Ok(())
}
