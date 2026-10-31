use std::collections::BTreeSet;

use serde_json::{json, Value};
use spur_graph::panic_evidence::*;
use spur_graph::{artifact_from_facts, build_facts, GraphIndexArtifact};

const MANIFEST: &[u8] = b"[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
const LOCK: &[u8] = b"version = 4\n";
const CONFIG: &[u8] = b"[build]\nincremental = false\n";

struct Fixture {
    source: String,
    raw: Vec<u8>,
    graph: GraphIndexArtifact,
    sites: MethodCallSiteEvidenceSet,
    identity: CaptureBuildIdentity,
}

impl Fixture {
    fn new() -> anyhow::Result<Self> {
        let value: Value =
            serde_json::from_str(include_str!("fixtures/panic_evidence/cga-format.json"))?;
        assert!(value["fixture_status"]
            .as_str()
            .unwrap()
            .starts_with("synthetic"));
        let source = value["source"].as_str().unwrap().to_owned();
        let raw = serde_json::to_vec(&value["export"])?;
        let root = tempfile::tempdir()?;
        std::fs::create_dir(root.path().join("src"))?;
        std::fs::write(root.path().join("src/lib.rs"), &source)?;
        let (facts, _) = build_facts(root.path(), None)?;
        let graph = artifact_from_facts(&facts, root.path())?;
        let sites = collect_method_call_sites(&graph, &sources(&source))?;
        let identity = CaptureBuildIdentity {
            schema_version: CAPTURE_BUILD_SCHEMA_VERSION.into(),
            scope: CaptureScope::BoundedFixture,
            provenance: ExportProvenance::SyntheticFixture,
            graph_content_hash: graph.graph_content_hash.clone(),
            capture_root: "/fixture".into(),
            inputs: vec![
                CaptureInput { path: "src/lib.rs".into(), kind: CaptureInputKind::Source, sha256: sha256_bytes(source.as_bytes()) },
                CaptureInput { path: "Cargo.toml".into(), kind: CaptureInputKind::Manifest, sha256: sha256_bytes(MANIFEST) },
                CaptureInput { path: "Cargo.lock".into(), kind: CaptureInputKind::Lockfile, sha256: sha256_bytes(LOCK) },
                CaptureInput { path: ".cargo/config.toml".into(), kind: CaptureInputKind::Config, sha256: sha256_bytes(CONFIG) },
            ],
            // Synthetic identity too: P3 must record actual rustc -vV bytes.
            rustc_verbose: "rustc synthetic\nbinary: rustc\ncommit-hash: synthetic\ncommit-date: 2026-10-18\nhost: x86_64-unknown-linux-gnu\nrelease: synthetic-nightly\nLLVM version: synthetic\n".into(),
            toolchain: CGA_TOOLCHAIN.into(),
            host: "x86_64-unknown-linux-gnu".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            exporter_revision: CGA_REVISION.into(),
            exporter_binary_sha256: sha256_bytes(b"synthetic exporter binary"),
            packages: vec!["fixture".into(), "dep".into()],
            targets: vec!["lib:fixture".into(), "bin:fixture".into()],
            features: vec!["checks".into(), "extra".into()],
            default_features: false,
            all_features: false,
            profile: "dev".into(),
            cfg_flags: vec!["--cfg=one".into(), "--cfg=two".into()],
            compiler_flags: vec!["-Coverflow-checks=off".into(), "-Coverflow-checks=on".into()],
            cargo_args: vec!["rustc".into(), "--locked".into(), "--lib".into()],
            environment: CAPTURE_ENV_ALLOWLIST.iter().map(|key| ((*key).into(), None)).collect(),
            capture_command: vec!["/repo/scripts/spur-cargo".into(), "rustc".into(), "--locked".into()],
            raw_export_sha256: sha256_bytes(&raw),
        };
        Ok(Self {
            source,
            raw,
            graph,
            sites,
            identity,
        })
    }

    fn inputs(&self) -> Vec<CaptureInputBytes<'_>> {
        vec![
            CaptureInputBytes {
                path: "src/lib.rs",
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
            CaptureInputBytes {
                path: ".cargo/config.toml",
                bytes: CONFIG,
            },
        ]
    }

    fn import(
        &self,
        identity: &CaptureBuildIdentity,
        expected: &str,
        raw: &[u8],
    ) -> anyhow::Result<CompilerEvidenceSet> {
        import_cga(CgaImportRequest {
            identity,
            expected_build_id: expected,
            raw_export: raw,
            inputs: &self.inputs(),
            graph: &self.graph,
            sources: &sources(&self.source),
            method_sites: &self.sites,
        })
    }

    fn altered_raw(&self, edit: impl FnOnce(&mut Value)) -> anyhow::Result<CompilerEvidenceSet> {
        let mut raw: Value = serde_json::from_slice(&self.raw)?;
        edit(&mut raw);
        let bytes = serde_json::to_vec(&raw)?;
        let mut identity = self.identity.clone();
        identity.raw_export_sha256 = sha256_bytes(&bytes);
        self.import(&identity, &identity.build_id()?, &bytes)
    }

    fn valid(&self) -> anyhow::Result<CompilerEvidenceSet> {
        self.import(&self.identity, &self.identity.build_id()?, &self.raw)
    }
}

fn sources(source: &str) -> [PanicEvidenceSource<'_>; 1] {
    [PanicEvidenceSource {
        file_path: "src/lib.rs",
        bytes: source.as_bytes(),
    }]
}

#[test]
fn every_semantic_identity_field_is_bound_before_association() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let id = f.identity.build_id()?;
    for (field, value) in [
        ("schema_version", json!("other")),
        ("provenance", json!("compiler_capture")),
        ("graph_content_hash", json!("changed")),
        ("capture_root", json!("/different")),
        ("rustc_verbose", json!("different compiler")),
        ("toolchain", json!("stable")),
        ("host", json!("aarch64-unknown-linux-gnu")),
        ("target", json!("aarch64-unknown-linux-gnu")),
        ("exporter_revision", json!("unsupported")),
        (
            "exporter_binary_sha256",
            json!(sha256_bytes(b"different exporter")),
        ),
        ("packages", json!(["different"])),
        ("targets", json!(["bin:other"])),
        ("features", json!(["different"])),
        ("default_features", json!(true)),
        ("all_features", json!(true)),
        ("profile", json!("release")),
        ("cfg_flags", json!(["--cfg=changed"])),
        ("compiler_flags", json!(["-Copt-level=1"])),
        ("cargo_args", json!(["check"])),
        ("capture_command", json!(["/different/command"])),
        (
            "raw_export_sha256",
            json!(sha256_bytes(b"different export")),
        ),
    ] {
        let mut changed = serde_json::to_value(&f.identity)?;
        changed[field] = value;
        let changed: CaptureBuildIdentity = serde_json::from_value(changed)?;
        if let Ok(changed_id) = changed.build_id() {
            assert_ne!(id, changed_id, "identity omitted {field}");
        }
        assert!(
            f.import(&changed, &id, &f.raw).is_err(),
            "associated changed {field}"
        );
    }
    for index in 0..f.identity.inputs.len() {
        let mut changed = f.identity.clone();
        changed.inputs[index].sha256 = sha256_bytes(b"changed input");
        assert_ne!(id, changed.build_id()?, "input inventory omitted {index}");
        assert!(f.import(&changed, &id, &f.raw).is_err());
    }
    let mut changed = f.identity.clone();
    changed
        .environment
        .insert("RUSTFLAGS".into(), Some("-Copt-level=1".into()));
    assert_ne!(id, changed.build_id()?);
    assert!(f.import(&changed, &id, &f.raw).is_err());
    Ok(())
}

#[test]
fn sets_canonicalize_but_argument_and_flag_order_is_meaningful() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let mut changed = f.identity.clone();
    changed.inputs.reverse();
    changed.features.reverse();
    changed.targets.reverse();
    changed.packages.reverse();
    assert_eq!(
        f.identity.build_id()?,
        changed.build_id()?,
        "set order must canonicalize"
    );
    let value = serde_json::to_value(&f.identity)?;
    for field in [
        "cfg_flags",
        "compiler_flags",
        "cargo_args",
        "capture_command",
    ] {
        let mut changed = value.clone();
        changed[field].as_array_mut().unwrap().reverse();
        let changed: CaptureBuildIdentity = serde_json::from_value(changed)?;
        assert_ne!(
            f.identity.build_id()?,
            changed.build_id()?,
            "lost {field} order"
        );
    }
    assert_eq!(
        sha256_bytes(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    Ok(())
}

#[test]
fn incomplete_unsupported_and_duplicate_envelopes_are_rejected() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    for field in [
        "graph_content_hash",
        "capture_root",
        "rustc_verbose",
        "toolchain",
        "host",
        "target",
        "exporter_revision",
        "exporter_binary_sha256",
        "profile",
        "raw_export_sha256",
    ] {
        let mut value = serde_json::to_value(&f.identity)?;
        value[field] = json!("");
        assert!(
            serde_json::from_value::<CaptureBuildIdentity>(value)?
                .build_id()
                .is_err(),
            "accepted empty {field}"
        );
    }
    for field in [
        "inputs",
        "packages",
        "targets",
        "cargo_args",
        "capture_command",
    ] {
        let mut value = serde_json::to_value(&f.identity)?;
        value[field] = json!([]);
        assert!(
            serde_json::from_value::<CaptureBuildIdentity>(value)?
                .build_id()
                .is_err(),
            "accepted empty {field}"
        );
    }
    for field in ["inputs", "packages", "targets", "features"] {
        let mut value = serde_json::to_value(&f.identity)?;
        let first = value[field][0].clone();
        value[field].as_array_mut().unwrap().push(first);
        assert!(
            serde_json::from_value::<CaptureBuildIdentity>(value)?
                .build_id()
                .is_err(),
            "duplicate {field}"
        );
    }
    for kind in [
        CaptureInputKind::Manifest,
        CaptureInputKind::Lockfile,
        CaptureInputKind::Source,
    ] {
        let mut changed = f.identity.clone();
        changed.inputs.retain(|i| i.kind != kind);
        assert!(changed.build_id().is_err(), "missing {kind:?}");
    }
    let mut changed = f.identity.clone();
    changed.environment.clear();
    assert!(
        changed.build_id().is_err(),
        "unset environment must be explicit"
    );
    let mut changed = f.identity.clone();
    changed
        .environment
        .insert("UNRECORDED_CARGO_SETTING".into(), Some("1".into()));
    assert!(changed.build_id().is_err(), "unsupported ambient settings");
    let mut changed = f.identity.clone();
    changed.inputs[0].path = "../src/lib.rs".into();
    assert!(changed.build_id().is_err());
    let mut changed = f.identity.clone();
    changed.exporter_binary_sha256 = "invalid".into();
    assert!(changed.build_id().is_err());
    Ok(())
}

#[test]
fn exact_raw_and_all_input_bytes_are_required_before_parsing() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let id = f.identity.build_id()?;
    let rejected = f.import(&f.identity, &id, b"invalid JSON");
    assert!(
        rejected.is_err(),
        "changed raw bytes must fail checksum before parsing"
    );
    let error = rejected.unwrap_err();
    assert!(
        error.to_string().contains("checksum"),
        "raw checksum must precede parsing: {error}"
    );
    for index in 0..f.inputs().len() {
        let mut inputs = f.inputs();
        inputs[index].bytes = b"stale";
        assert!(
            import_cga(CgaImportRequest {
                identity: &f.identity,
                expected_build_id: &id,
                raw_export: &f.raw,
                inputs: &inputs,
                graph: &f.graph,
                sources: &sources(&f.source),
                method_sites: &f.sites
            })
            .is_err(),
            "accepted stale input {index}"
        );
    }
    for mode in ["missing", "duplicate", "extra"] {
        let mut inputs = f.inputs();
        match mode {
            "missing" => {
                inputs.pop();
            }
            "duplicate" => inputs.push(inputs[0]),
            _ => inputs.push(CaptureInputBytes {
                path: "extra",
                bytes: b"extra",
            }),
        }
        assert!(
            f.identity.validate_inputs(&inputs).is_err(),
            "accepted {mode} input"
        );
    }
    Ok(())
}

#[test]
fn pinned_graph_preserves_kinds_instances_raw_locations_and_capability_gaps() -> anyhow::Result<()>
{
    let f = Fixture::new()?;
    let result = f.valid()?;
    assert_eq!(
        result.observations.len(),
        9,
        "one row per exported edge, including instantiated uses"
    );
    assert_eq!(
        serde_json::to_value(&result.raw_graph)?,
        serde_json::from_slice::<Value>(&f.raw)?
    );
    let kinds: BTreeSet<_> = result
        .observations
        .iter()
        .map(|o| match o.kind {
            CgaEdgeKind::Called(_) => "Called",
            CgaEdgeKind::VTableCalled(_) => "VTableCalled",
            CgaEdgeKind::FnPtrCast(_) => "FnPtrCast",
            CgaEdgeKind::BuiltinAssert => "BuiltinAssert",
            CgaEdgeKind::AsyncPanic => "AsyncPanic",
        })
        .collect();
    assert_eq!(kinds.len(), 5);
    assert_eq!(
        result.observations[0].canonical_callee.as_deref(),
        Some("core::option::Option::<u8>::unwrap")
    );
    assert_eq!(
        result.observations[2].canonical_callee.as_deref(),
        Some("fixture::Safe::unwrap")
    );
    assert_eq!(
        result.observations[8].canonical_callee.as_deref(),
        Some("core::option::Option::<u16>::unwrap")
    );
    assert_ne!(
        result.observations[0].canonical_caller,
        result.observations[8].canonical_caller
    );
    assert_eq!(
        result.observations[0].association,
        result.observations[8].association
    );
    assert_eq!(
        result.capabilities.receiver_type,
        CapabilityStatus::Unavailable
    );
    assert_eq!(
        result.capabilities.assert_reason,
        CapabilityStatus::Unavailable
    );
    assert_eq!(
        result.capabilities.macro_expansion,
        CapabilityStatus::Unavailable
    );
    assert_eq!(
        serde_json::to_value(result.coordinate_convention)?,
        json!("line1_scalar_column1_end_exclusive")
    );
    println!("P2_OBSERVED={}", serde_json::to_string(&result)?);
    Ok(())
}

#[test]
fn every_root_edge_source_and_edge_target_reference_is_checked() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    for (label, invalid) in [
        ("root", json!({"roots":[5]})),
        ("duplicate root", json!({"roots":[0,0]})),
        ("source", json!({"edges":{"5":[]}})),
        ("Called", json!({"kind":{"Called":5}})),
        ("VTableCalled", json!({"kind":{"VTableCalled":5}})),
        ("FnPtrCast", json!({"kind":{"FnPtrCast":5}})),
    ] {
        assert!(
            f.altered_raw(|v| {
                if let Some(roots) = invalid.get("roots") {
                    v["roots"] = roots.clone();
                } else if let Some(edges) = invalid.get("edges") {
                    v["edges"] = edges.clone();
                } else {
                    v["edges"]["0"][0]["kind"] = invalid["kind"].clone();
                }
            })
            .is_err(),
            "accepted dangling {label}"
        );
    }
    assert!(f
        .altered_raw(|v| v["nodes"][0]["name"] = json!(""))
        .is_err());
    assert!(f
        .altered_raw(|v| v["receiver_type"] = json!("invented"))
        .is_err());
    let raw = br#"{"roots":[],"nodes":[],"edges":{"0":[],"0":[]}}"#;
    let mut identity = f.identity.clone();
    identity.raw_export_sha256 = sha256_bytes(raw);
    assert!(
        f.import(&identity, &identity.build_id()?, raw).is_err(),
        "duplicate map keys must not disappear"
    );
    Ok(())
}

#[test]
fn exact_unique_containing_and_ambiguous_joins_are_distinct() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let result = f.altered_raw(|v| {
        // The method identifier is contained in exactly the Safe invocation.
        let start = v["edges"]["0"][3]["location"]["span"]["start"]["column"]
            .as_u64()
            .unwrap()
            + 5;
        v["edges"]["0"][3]["location"]["span"]["start"]["column"] = json!(start);
        v["edges"]["0"][3]["location"]["span"]["end"]["column"] = json!(start + 6);
    })?;
    assert_eq!(
        result.observations.len(),
        9,
        "association requires every compiler observation"
    );
    assert!(
        matches!(
            result.observations[0].association,
            LocationAssociation::Exact { .. }
        ),
        "exact call span"
    );
    assert!(
        matches!(
            result.observations[3].association,
            LocationAssociation::UniqueContaining { .. }
        ),
        "unique containing AST site"
    );
    match &result.observations[4].association {
        LocationAssociation::Ambiguous { site_ids } => assert_eq!(
            site_ids.len(),
            2,
            "inner method token belongs to two chained calls"
        ),
        other => panic!("chain span must remain ambiguous: {other:?}"),
    }
    Ok(())
}

#[test]
fn unknown_external_unmapped_and_unrelated_callers_never_join_by_name() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let result = f.valid()?;
    assert_eq!(
        result.observations.len(),
        9,
        "unknown locations must retain observations"
    );
    assert!(matches!(
        result.observations[5].association,
        LocationAssociation::Unmapped { .. }
    ));
    assert_eq!(
        result.observations[6].association,
        LocationAssociation::External
    );
    assert_eq!(
        result.observations[7].association,
        LocationAssociation::Unmapped {
            reason: UnmappedReason::UnknownLocation
        }
    );
    let wrong = f.altered_raw(|v| {
        v["nodes"][0]["location"]["span"] =
            json!({"start":{"line":7,"column":4},"end":{"line":7,"column":9}})
    })?;
    assert!(
        matches!(
            wrong.observations[0].association,
            LocationAssociation::Unmapped {
                reason: UnmappedReason::CallerNotBound
            }
        ),
        "unrelated real function is not the call's owner"
    );
    let named = f.altered_raw(|v| {
        v["edges"]["0"][0]["location"]["span"] =
            json!({"start":{"line":3,"column":4},"end":{"line":3,"column":10}})
    })?;
    assert!(
        matches!(
            named.observations[0].association,
            LocationAssociation::Unmapped { .. }
        ),
        "callee spelling cannot create a site"
    );
    Ok(())
}

#[test]
fn local_paths_bind_exactly_and_unknown_files_stay_unmapped() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let absolute =
        f.altered_raw(|v| v["edges"]["0"][0]["location"]["path"] = json!("/fixture/src/lib.rs"))?;
    assert_eq!(
        absolute.observations.len(),
        9,
        "absolute local paths retain observations"
    );
    assert_eq!(
        absolute.observations[0].association,
        f.valid()?.observations[0].association
    );
    let external =
        f.altered_raw(|v| v["edges"]["0"][0]["location"]["path"] = json!("/other/src/lib.rs"))?;
    assert_eq!(
        external.observations[0].association,
        LocationAssociation::External
    );
    let absent =
        f.altered_raw(|v| v["edges"]["0"][0]["location"]["path"] = json!("src/missing.rs"))?;
    assert!(matches!(
        absent.observations[0].association,
        LocationAssociation::Unmapped { .. }
    ));
    assert!(f
        .altered_raw(|v| v["edges"]["0"][0]["location"]["path"] = json!("src/../src/lib.rs"))
        .is_err());
    Ok(())
}

#[test]
fn scalar_utf8_columns_and_exclusive_endpoints_convert_without_guessing() -> anyhow::Result<()> {
    let source = "é🦀a.unwrap();\nnext\n";
    let span = CgaSpan {
        start: CgaFilePosition { line: 1, column: 3 },
        end: CgaFilePosition {
            line: 1,
            column: 13,
        },
    };
    assert_eq!(
        span.to_byte_range(source.as_bytes())?,
        [6, 16],
        "columns count Unicode scalars; endpoints exclude the next token"
    );
    assert_eq!(&source[6..16], "a.unwrap()");
    let multiline = CgaSpan {
        start: CgaFilePosition { line: 1, column: 3 },
        end: CgaFilePosition { line: 2, column: 5 },
    };
    assert_eq!(
        &source[multiline.to_byte_range(source.as_bytes())?[0]
            ..multiline.to_byte_range(source.as_bytes())?[1]],
        "a.unwrap();\nnext"
    );
    for bad in [
        CgaSpan {
            start: CgaFilePosition { line: 0, column: 0 },
            end: CgaFilePosition { line: 1, column: 1 },
        },
        CgaSpan {
            start: CgaFilePosition {
                line: 1,
                column: 14,
            },
            end: CgaFilePosition {
                line: 1,
                column: 15,
            },
        },
        CgaSpan {
            start: CgaFilePosition { line: 2, column: 1 },
            end: CgaFilePosition { line: 1, column: 3 },
        },
    ] {
        assert!(
            bad.to_byte_range(source.as_bytes()).is_err(),
            "invalid coordinate {bad:?}"
        );
    }
    assert!(span.to_byte_range(b"\xff").is_err());
    assert!(
        span.to_byte_range(b"a\r\nb").is_err(),
        "CRLF normalization is outside the bounded convention"
    );
    Ok(())
}

#[test]
fn malformed_local_node_and_edge_spans_are_rejected() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    for object in ["node", "edge"] {
        for span in [
            json!({"start":{"line":0,"column":0},"end":{"line":1,"column":1}}),
            json!({"start":{"line":4,"column":999},"end":{"line":4,"column":1000}}),
            json!({"start":{"line":5,"column":1},"end":{"line":4,"column":1}}),
        ] {
            assert!(
                f.altered_raw(|v| {
                    if object == "node" {
                        v["nodes"][0]["location"]["span"] = span;
                    } else {
                        v["edges"]["0"][0]["location"]["span"] = span;
                    }
                })
                .is_err(),
                "accepted malformed local {object}"
            );
        }
    }
    Ok(())
}

#[test]
fn graph_sources_and_site_ownership_revalidate_and_roundtrip() -> anyhow::Result<()> {
    let mut f = Fixture::new()?;
    let result = f.valid()?;
    let decoded: CompilerEvidenceSet = serde_json::from_slice(&serde_json::to_vec(&result)?)?;
    assert_eq!(result, decoded);
    let id = f.identity.build_id()?;
    decoded.validate(CgaImportRequest {
        identity: &f.identity,
        expected_build_id: &id,
        raw_export: &f.raw,
        inputs: &f.inputs(),
        graph: &f.graph,
        sources: &sources(&f.source),
        method_sites: &f.sites,
    })?;
    let mut changed = decoded.clone();
    changed.observations.clear();
    assert!(
        changed
            .validate(CgaImportRequest {
                identity: &f.identity,
                expected_build_id: &id,
                raw_export: &f.raw,
                inputs: &f.inputs(),
                graph: &f.graph,
                sources: &sources(&f.source),
                method_sites: &f.sites
            })
            .is_err(),
        "serialized observations must revalidate"
    );
    let unrelated = f
        .graph
        .symbols
        .iter()
        .find(|s| s.entity_name == "other")
        .unwrap()
        .stable_symbol_id
        .clone();
    f.sites.sites[0].caller_stable_symbol_id = Some(unrelated);
    assert!(
        f.valid().is_err(),
        "P1 ownership must revalidate before joining"
    );
    let mut f = Fixture::new()?;
    f.graph.graph_content_hash = "changed".into();
    assert!(f.valid().is_err());
    let mut f = Fixture::new()?;
    f.source.push(' ');
    assert!(f.valid().is_err());
    Ok(())
}

fn controls() -> Value {
    serde_json::from_str(include_str!("fixtures/panic_evidence/cga-format.json")).unwrap()
}

fn fixture_with_source(source: &str) -> anyhow::Result<Fixture> {
    let mut f = Fixture::new()?;
    f.source = source.into();
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("src"))?;
    std::fs::write(root.path().join("src/lib.rs"), &f.source)?;
    let (facts, _) = build_facts(root.path(), None)?;
    f.graph = artifact_from_facts(&facts, root.path())?;
    f.sites = collect_method_call_sites(&f.graph, &sources(&f.source))?;
    f.identity.graph_content_hash = f.graph.graph_content_hash.clone();
    f.identity.inputs.iter_mut().find(|i| i.path == "src/lib.rs").unwrap().sha256 =
        sha256_bytes(f.source.as_bytes());
    Ok(f)
}

// Independent fixture encoder: byte offsets become one-based scalar columns.
fn location_at(source: &str, range: [usize; 2]) -> Value {
    let position = |offset| {
        let prefix = &source[..offset];
        json!({
            "line": 1 + prefix.bytes().filter(|b| *b == b'\n').count(),
            "column": 1 + prefix.rsplit('\n').next().unwrap().chars().count()
        })
    };
    json!({"path":"src/lib.rs","span":{"start":position(range[0]),"end":position(range[1])}})
}

fn header(source: &str, marker: &str) -> [usize; 2] {
    let start = source.find(marker).unwrap();
    [start, start + source[start..].find('{').unwrap()]
}

fn synthetic_observation(
    f: &mut Fixture,
    definition: [usize; 2],
    edge: [usize; 2],
) -> anyhow::Result<CompilerEvidenceSet> {
    // Deliberately unrelated names: lexical identity must come from bytes.
    f.raw = serde_json::to_vec(&json!({
        "roots":[0],
        "nodes":[
            {"name":"synthetic::opaque::<u16>","location":location_at(&f.source,definition)},
            {"name":"synthetic::callee","location":{"path":"{unknown}","span":{"start":{"line":0,"column":0},"end":{"line":0,"column":0}}}}
        ],
        "edges":{"0":[{"kind":{"Called":1},"location":location_at(&f.source,edge)}]}
    }))?;
    f.identity.raw_export_sha256 = sha256_bytes(&f.raw);
    f.valid()
}

#[test]
fn regression_real_pinned_export_uses_one_based_scalar_columns() -> anyhow::Result<()> {
    let witness = &controls()["real_coordinate_witness"];
    let source = witness["source"].as_str().unwrap();
    assert_eq!(sha256_bytes(source.as_bytes()), witness["provenance"]["source_sha256"]);
    assert_eq!(witness["provenance"]["upstream_revision"], CGA_REVISION);
    let span: CgaSpan = serde_json::from_value(witness["edge"]["location"]["span"].clone())?;
    assert_eq!((span.start.line, span.start.column, span.end.column), (4, 63, 77));
    let range = span.to_byte_range(source.as_bytes())?;
    assert_eq!(range, [100, 114], "authentic pinned compiler span after 雪🙂");
    assert_eq!(&source[range[0]..range[1]], "value.unwrap()");
    // Exercise association too, with a clearly synthetic two-node projection.
    let mut f = fixture_with_source(source)?;
    let caller: CgaSpan = serde_json::from_value(witness["node"]["location"]["span"].clone())?;
    let result = synthetic_observation(&mut f, caller.to_byte_range(source.as_bytes())?, range)?;
    assert!(matches!(result.observations[0].association, LocationAssociation::Exact { .. }));
    Ok(())
}

#[test]
fn regression_zero_local_columns_are_rejected() -> anyhow::Result<()> {
    for (start, end) in [(0, 1), (1, 0), (0, 0)] {
        let span = CgaSpan {
            start: CgaFilePosition { line: 1, column: start },
            end: CgaFilePosition { line: 1, column: end },
        };
        assert!(span.to_byte_range(b"a").is_err(), "pinned compiler columns are one-based: {span:?}");
    }
    let f = Fixture::new()?;
    for object in ["node", "edge", "unsupplied"] {
        assert!(f.altered_raw(|v| {
            let loc = if object == "node" { &mut v["nodes"][0]["location"] }
                else { &mut v["edges"]["0"][0]["location"] };
            loc["span"]["start"]["column"] = json!(0);
            if object == "unsupplied" { loc["path"] = json!("src/missing.rs"); }
        }).is_err(), "zero column in local {object}");
    }
    Ok(())
}

#[test]
fn regression_nested_definition_cannot_authorize_outer_site() -> anyhow::Result<()> {
    let source = controls()["ownership_witness"]["source"].as_str().unwrap().to_owned();
    let mut f = fixture_with_source(&source)?;
    assert!(f.graph.symbols.iter().any(|s| s.entity_name == "inner"));
    let site = f.sites.sites.iter().find(|s| s.receiver_text == "x").unwrap();
    let outer = f.graph.symbols.iter().find(|s| s.entity_name == "outer").unwrap();
    assert_eq!(site.caller_stable_symbol_id.as_deref(), Some(outer.stable_symbol_id.as_str()));
    let method = site.method_byte_range[0];
    let result = synthetic_observation(&mut f, header(&source, "fn inner"), [method, method + 3])?;
    assert_eq!(result.observations[0].association, LocationAssociation::Unmapped {
        reason: UnmappedReason::CallerNotBound
    }, "nested function header must not authorize an ancestor's call site");
    Ok(())
}

#[test]
fn regression_closure_definition_cannot_authorize_outer_or_sibling_site() -> anyhow::Result<()> {
    let source = controls()["ownership_witness"]["source"].as_str().unwrap().to_owned();
    let mut f = fixture_with_source(&source)?;
    let closure = header(&source, "|z:");
    for receiver in ["x", "w"] {
        let method = f.sites.sites.iter().find(|s| s.receiver_text == receiver).unwrap().method_byte_range[0];
        let result = synthetic_observation(&mut f, closure, [method, method + 3])?;
        assert_eq!(result.observations[0].association, LocationAssociation::Unmapped {
            reason: UnmappedReason::CallerNotBound
        }, "closure definition cannot authorize receiver {receiver}");
    }
    Ok(())
}

#[test]
fn regression_nondefinition_token_cannot_bind_caller() -> anyhow::Result<()> {
    let source = controls()["ownership_witness"]["source"].as_str().unwrap().to_owned();
    let mut f = fixture_with_source(&source)?;
    let method = f.sites.sites.iter().find(|s| s.receiver_text == "x").unwrap().method_byte_range[0];
    let result = synthetic_observation(&mut f, [method, method + 2], [method, method + 3])?;
    assert_eq!(result.observations[0].association, LocationAssociation::Unmapped {
        reason: UnmappedReason::CallerNotBound
    }, "an arbitrary body token is not a callable definition");
    Ok(())
}

#[test]
fn ordinary_generic_nested_and_closure_definitions_bind_without_name_joins() -> anyhow::Result<()> {
    let source = "fn outer(x: Option<u8>) { fn inner(y: Option<u8>) { y.unwrap(); } x.unwrap(); let c = |z: Option<u8>| { z.unwrap() }; }\nfn generic<T>(t: Option<T>) { t.unwrap(); }\nfn ordinary(a: Option<u8>) { a.unwrap(); }\n";
    let mut f = fixture_with_source(source)?;
    for (marker, receiver) in [("fn outer","x"),("fn inner","y"),("|z:","z"),("fn generic","t"),("fn ordinary","a")] {
        let site = f.sites.sites.iter().find(|s| s.receiver_text == receiver).unwrap();
        let range = site.byte_range;
        let method = site.method_byte_range;
        let owner = site.lexical_owners.iter().find(|o| matches!(o.syntax_kind.as_str(), "function_item" | "closure_expression")).unwrap().byte_range;
        let definition = header(source, marker);
        for span in [definition, owner] {
            let result = synthetic_observation(&mut f, span, range)?;
            assert!(matches!(result.observations[0].association, LocationAssociation::Exact { .. }), "{marker} full span/header: {:?}", result.observations[0].association);
        }
        let result = synthetic_observation(&mut f, definition, method)?;
        assert!(matches!(result.observations[0].association, LocationAssociation::UniqueContaining { .. }), "{marker} method token");
    }
    Ok(())
}

fn patched_producer() -> Value {
    let p = &controls()["real_coordinate_witness"]["provenance"];
    json!({
        "build":p["producer_build"],
        "cargo_cga_sha256":p["cargo_cga_sha256"],
        "cga_driver_sha256":p["cga_driver_sha256"]
    })
}

fn identity_with_producer(f: &Fixture) -> Value {
    let mut value = serde_json::to_value(&f.identity).unwrap();
    value.as_object_mut().unwrap().remove("exporter_binary_sha256");
    value["producer"] = patched_producer();
    value
}

#[test]
fn regression_patched_and_unmodified_producers_are_representable_and_hash_bound() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let value = identity_with_producer(&f);
    let identity = serde_json::from_value::<CaptureBuildIdentity>(value.clone());
    assert!(identity.is_ok(), "both producer binaries and patched lineage must be representable: {identity:?}");
    let identity = identity?;
    let expected = identity.build_id()?;
    assert_eq!(serde_json::to_value(&identity)?["producer"], patched_producer());
    for (field, replacement) in [
        ("cargo_cga_sha256", json!(sha256_bytes(b"other launcher"))),
        ("cga_driver_sha256", json!(sha256_bytes(b"other driver"))),
        ("build", json!({"state":"patched","patch_sha256":sha256_bytes(b"other patch")})),
        ("build", json!({"state":"unmodified"})),
    ] {
        let mut changed = value.clone();
        changed["producer"][field] = replacement;
        let changed: CaptureBuildIdentity = serde_json::from_value(changed)?;
        assert_ne!(expected, changed.build_id()?, "producer hash omitted {field}");
        // Malformed raw bytes expose any accidental parse-before-identity path.
        let error = f.import(&changed, &expected, b"not JSON").unwrap_err();
        assert!(error.to_string().contains("expected build identity mismatch"), "{error}");
    }
    Ok(())
}

#[test]
fn regression_producer_metadata_is_mandatory() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let mut value = serde_json::to_value(&f.identity)?;
    value.as_object_mut().unwrap().remove("producer");
    assert!(serde_json::from_value::<CaptureBuildIdentity>(value).is_err(),
        "a lone exporter fingerprint cannot establish producer lineage");
    Ok(())
}

#[test]
fn producer_missing_invalid_and_contradictory_fields_are_rejected() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let value = identity_with_producer(&f);
    for field in ["build", "cargo_cga_sha256", "cga_driver_sha256"] {
        let mut missing = value.clone();
        missing["producer"].as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<CaptureBuildIdentity>(missing).is_err(), "missing producer {field}");
    }
    for build in [
        json!({}),
        json!({"state":"patched"}),
        json!({"state":"patched","patch_sha256":null}),
        json!({"state":"patched","patch_sha256":""}),
        json!({"state":"patched","patch_sha256":"bad"}),
        json!({"state":"unmodified","patch_sha256":sha256_bytes(b"patch")}),
        json!({"state":"unknown"}),
    ] {
        let mut changed = value.clone();
        changed["producer"]["build"] = build;
        let invalid = serde_json::from_value::<CaptureBuildIdentity>(changed);
        assert!(invalid.is_err() || invalid?.build_id().is_err(), "invalid producer build state");
    }
    for field in ["cargo_cga_sha256", "cga_driver_sha256"] {
        for digest in [json!(null), json!(""), json!("bad")] {
            let mut changed = value.clone();
            changed["producer"][field] = digest;
            let invalid = serde_json::from_value::<CaptureBuildIdentity>(changed);
            assert!(invalid.is_err() || invalid?.build_id().is_err(), "invalid {field}");
        }
    }
    Ok(())
}

#[test]
fn regression_compiler_provenance_cannot_use_synthetic_rustc_identity() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let mut changed = f.identity.clone();
    changed.provenance = ExportProvenance::CompilerCapture;
    assert!(changed.build_id().is_err(), "synthetic verbose identity cannot claim a pinned compiler capture");
    Ok(())
}

#[test]
fn regression_real_rustc_identity_must_match_the_pinned_nightly() -> anyhow::Result<()> {
    let f = Fixture::new()?;
    let mut identity = f.identity.clone();
    identity.rustc_verbose = controls()["real_coordinate_witness"]["provenance"]["rustc_verbose"].as_str().unwrap().into();
    identity.host = "aarch64-apple-darwin".into();
    identity.target = identity.host.clone();
    identity.provenance = ExportProvenance::CompilerCapture;
    identity.build_id()?;
    for (from, to) in [
        ("8006091eb71ae7d9e9bccb66c6122c9d68af4466", "0000000000000000000000000000000000000000"),
        ("2026-10-17", "2026-10-18"),
        ("1.101.0-nightly", "1.100.0-nightly"),
    ] {
        let mut changed = identity.clone();
        changed.rustc_verbose = changed.rustc_verbose.replace(from, to);
        assert!(changed.build_id().is_err(), "wrong compiler component {from}");
    }
    Ok(())
}
