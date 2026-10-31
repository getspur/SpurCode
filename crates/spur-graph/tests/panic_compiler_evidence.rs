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
        result.coordinate_convention,
        CgaCoordinateConvention::Line1ScalarColumn0EndExclusiveNeedsRealExportCheck
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
            json!({"start":{"line":7,"column":3},"end":{"line":7,"column":8}})
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
            json!({"start":{"line":3,"column":3},"end":{"line":3,"column":9}})
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
        start: CgaFilePosition { line: 1, column: 2 },
        end: CgaFilePosition {
            line: 1,
            column: 12,
        },
    };
    assert_eq!(
        span.to_byte_range(source.as_bytes())?,
        [6, 16],
        "columns count Unicode scalars; endpoints exclude the next token"
    );
    assert_eq!(&source[6..16], "a.unwrap()");
    let multiline = CgaSpan {
        start: CgaFilePosition { line: 1, column: 2 },
        end: CgaFilePosition { line: 2, column: 4 },
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
                column: 13,
            },
            end: CgaFilePosition {
                line: 1,
                column: 14,
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
