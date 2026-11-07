use spur_graph::query_client::{GraphQueryClient, InMemoryClient, OverlayClient};
use spur_graph::{artifact_from_facts, build_facts, RustEvidenceAvailability, RustEvidenceFilter};
use std::{collections::HashSet, fs, sync::Arc};
#[test]
fn zero_replacement_removes_old_sites_and_keeps_neighbor_origin() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.rs"),
        "fn leaf() {} fn a() { leaf(); }\n",
    )
    .unwrap();
    fs::write(root.path().join("b.rs"), "pub struct Keep(u8);\n").unwrap();
    let base =
        artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    let before_b = base
        .rust_evidence
        .as_ref()
        .unwrap()
        .files
        .iter()
        .find(|f| f.file_path == "b.rs")
        .unwrap()
        .clone();
    fs::write(root.path().join("a.rs"), "pub struct Empty;\n").unwrap();
    let (delta, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &[root.path().join("a.rs")])
            .unwrap();
    assert_eq!(shadowed, HashSet::from(["a.rs".into()]));
    let overlay =
        OverlayClient::from_artifacts(InMemoryClient::new(Arc::new(base)), delta, shadowed)
            .unwrap();
    let result = overlay
        .rust_evidence(&RustEvidenceFilter::default())
        .unwrap();
    assert_eq!(result.availability, RustEvidenceAvailability::Complete);
    assert_eq!(result.files.len(), 2);
    assert_eq!(
        *result.files.iter().find(|f| f.file_path == "b.rs").unwrap(),
        before_b
    );
    let a = result.files.iter().find(|f| f.file_path == "a.rs").unwrap();
    match &a.calls {
        spur_graph::RustCallEvidence::Complete { evidence } => {
            assert!(evidence.sites.is_empty() && evidence.bodies.is_empty())
        }
        other => panic!("unexpected {other:?}"),
    }
}

use serde_json::{json, Value};
use spur_graph::query_client::ParquetClient;
use spur_graph::{
    artifact_from_facts_for_paths, build_facts_for_paths, read_artifact_parquet,
    write_artifact_parquet, GraphIndexArtifact, RustEvidenceAvailability as Availability,
    RustEvidenceFilter as Filter, RustEvidenceQueryResult, WriteOptions,
};
use std::{collections::BTreeMap, path::Path};

type Sources = BTreeMap<String, Vec<u8>>;
const BASE_A: &str = "struct Pair(pub u8, pub u16);\nfn a() { leaf(); leaf(); }\n";
const BASE_B: &str = "fn b() { other(); }\n";
const ZERO: &str = "pub struct Empty;\n";

fn build(root: &Path) -> GraphIndexArtifact {
    artifact_from_facts(&build_facts(root, None).unwrap().0, root).unwrap()
}
fn file(path: &str) -> Filter {
    Filter {
        file_path: Some(path.into()),
        source_oid: None,
    }
}
fn sources(root: &Path, graph: &GraphIndexArtifact) -> Sources {
    graph
        .file_manifests
        .iter()
        .map(|m| (m.path.clone(), fs::read(root.join(&m.path)).unwrap()))
        .collect()
}
fn authenticate(graph: &GraphIndexArtifact, raw: &Sources) {
    let inputs = raw
        .iter()
        .map(|(path, bytes)| spur_graph::RustDeclarationSource {
            file_path: path,
            bytes,
        })
        .collect::<Vec<_>>();
    if graph.rust_evidence.is_some() {
        graph.validate_rust_evidence_sources(&inputs).unwrap();
    } else {
        assert!(graph.validate_rust_evidence_sources(&inputs).is_err());
        for m in &graph.file_manifests {
            assert_eq!(m.content_oid, spur_graph::git_blob_oid(&raw[&m.path]));
        }
    }
}
fn fixture() -> (tempfile::TempDir, GraphIndexArtifact, Sources) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), BASE_A).unwrap();
    fs::write(root.path().join("b.rs"), BASE_B).unwrap();
    let graph = build(root.path());
    let raw = sources(root.path(), &graph);
    authenticate(&graph, &raw);
    (root, graph, raw)
}
fn selected(root: &Path, paths: &[&str]) -> Arc<GraphIndexArtifact> {
    let paths = paths.iter().map(|p| root.join(p)).collect::<Vec<_>>();
    let facts = build_facts_for_paths(root, &paths).unwrap();
    Arc::new(artifact_from_facts_for_paths(&facts, root, &paths).unwrap())
}
fn capture(client: &dyn GraphQueryClient, filter: &Filter) -> Value {
    match client.rust_evidence(filter) {
        Ok(r) => json!({"ok":r}),
        Err(e) => json!({"error":format!("{e:#}")}),
    }
}
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &to.join(e.file_name()));
        } else {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}
fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn export_case(
    name: &str,
    base: &GraphIndexArtifact,
    base_raw: &Sources,
    delta: &GraphIndexArtifact,
    delta_raw: &Sources,
    shadowed: &HashSet<String>,
    outputs: &Value,
    base_disk: &Path,
    delta_disk: &Path,
) {
    println!("D8_NATIVE {}", json!({"name":name,"outputs":outputs}));
    if let Some(packet) = std::env::var_os("SPUR_D8_PACKET") {
        let out = Path::new(&packet).join(name);
        fs::create_dir_all(&out).unwrap();
        write_json(&out.join("base.json"), base);
        write_json(&out.join("delta.json"), delta);
        let mut shadowed = shadowed.iter().collect::<Vec<_>>();
        shadowed.sort();
        write_json(&out.join("shadowed.json"), &shadowed);
        write_json(&out.join("actual.json"), outputs);
        for (kind, raw) in [("base-source", base_raw), ("delta-source", delta_raw)] {
            for (path, bytes) in raw {
                let p = out.join(kind).join(path);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, bytes).unwrap();
            }
        }
        copy_tree(base_disk, &out.join("base-disk"));
        copy_tree(delta_disk, &out.join("delta-disk"));
    }
}

/// Query real memory, persisted base and D5-reloaded delta clients. The oracle
/// selects original WHOLE file objects; it never forges a stored carrier/hash.
fn exercise(
    name: &str,
    base: &GraphIndexArtifact,
    base_raw: &Sources,
    delta: Arc<GraphIndexArtifact>,
    delta_raw: &Sources,
    shadowed: HashSet<String>,
    expected: Availability,
) -> RustEvidenceQueryResult {
    authenticate(base, base_raw);
    authenticate(&delta, delta_raw);
    let base_out = tempfile::tempdir().unwrap();
    let delta_out = tempfile::tempdir().unwrap();
    write_artifact_parquet(base, base_out.path(), WriteOptions::default(), vec![]).unwrap();
    write_artifact_parquet(&delta, delta_out.path(), WriteOptions::default(), vec![]).unwrap();
    let reloaded = Arc::new(read_artifact_parquet(delta_out.path()).unwrap());
    assert_eq!(reloaded.rust_evidence, delta.rust_evidence);
    assert_eq!(reloaded.file_manifests, delta.file_manifests);
    authenticate(&reloaded, delta_raw);
    let memory = OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(base.clone())),
        Arc::clone(&delta),
        shadowed.clone(),
    )
    .unwrap();
    let disk = OverlayClient::from_artifacts(
        ParquetClient::open(base_out.path()).unwrap(),
        Arc::clone(&delta),
        shadowed.clone(),
    )
    .unwrap();
    let reloaded = OverlayClient::from_artifacts(
        ParquetClient::open(base_out.path()).unwrap(),
        reloaded,
        shadowed.clone(),
    )
    .unwrap();
    let result = memory.rust_evidence(&Filter::default()).unwrap();
    assert_eq!(result.availability, expected, "{name}");
    let mut files = base
        .rust_evidence
        .as_ref()
        .map(|s| {
            s.files
                .iter()
                .filter(|f| !shadowed.contains(&f.file_path))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.extend(
        delta
            .rust_evidence
            .as_ref()
            .map(|s| {
                s.files
                    .iter()
                    .filter(|f| shadowed.contains(&f.file_path))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    files.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    assert_eq!(result.files, files, "exact whole-file replacement: {name}");
    let mut origins = BTreeMap::new();
    for set in [base.rust_evidence.as_ref(), delta.rust_evidence.as_ref()]
        .into_iter()
        .flatten()
    {
        for o in &set.origins {
            if files
                .iter()
                .any(|f| f.origin_graph_content_hash == o.graph_content_hash)
            {
                origins.insert(
                    (o.manifest_version.clone(), o.graph_content_hash.clone()),
                    o.clone(),
                );
            }
        }
    }
    assert_eq!(
        result.origins,
        origins.into_values().collect::<Vec<_>>(),
        "whole original inventories: {name}"
    );
    let mut active = base
        .file_manifests
        .iter()
        .filter(|m| !shadowed.contains(&m.path))
        .collect::<Vec<_>>();
    active.extend(
        delta
            .file_manifests
            .iter()
            .filter(|m| shadowed.contains(&m.path)),
    );
    active.sort_by(|a, b| a.path.cmp(&b.path));
    let domain = result.source_domain.as_ref().unwrap();
    assert_eq!(
        domain
            .iter()
            .map(|s| (&s.file_path, &s.stable_file_id, &s.source_oid))
            .collect::<Vec<_>>(),
        active
            .iter()
            .map(|m| (&m.path, &m.stable_file_id, &m.content_oid))
            .collect::<Vec<_>>()
    );
    let mut filters = vec![
        Filter::default(),
        file("a.rs"),
        file("b.rs"),
        file("missing.rs"),
        file("readme.md"),
        file("external://pkg/a.rs"),
        Filter {
            file_path: None,
            source_oid: Some("invalid-shape".into()),
        },
    ];
    for g in [base, delta.as_ref()] {
        for m in &g.file_manifests {
            filters.push(Filter {
                source_oid: Some(m.content_oid.clone()),
                ..file(&m.path)
            });
        }
    }
    let outputs = filters
        .iter()
        .map(|f| {
            let m = capture(&memory, f);
            let d = capture(&disk, f);
            let r = capture(&reloaded, f);
            assert_eq!(m, d, "memory/disk {name}: {f:?}");
            assert_eq!(m, r, "D5 delta {name}: {f:?}");
            json!({"filter":f,"memory":m,"disk":d,"delta_reloaded":r})
        })
        .collect::<Vec<_>>();
    export_case(
        name,
        base,
        base_raw,
        &delta,
        delta_raw,
        &shadowed,
        &json!(outputs),
        base_out.path(),
        delta_out.path(),
    );
    result
}

#[test]
fn native_whole_file_edits_zero_cfg_and_recovery_keep_historical_neighbors() {
    for (name, text, availability) in [
        (
            "edit",
            "struct Replaced(pub u32);\nfn changed() { new_call(); }\n",
            Availability::Complete,
        ),
        ("zero", ZERO, Availability::Complete),
        (
            "cfg",
            "struct Visible(pub u8);\n#[cfg(feature = \"x\")] fn hidden() { leaf(); }\n",
            Availability::Partial,
        ),
        (
            "recovery",
            "struct Kept(pub u8);\nfn broken( {\n",
            Availability::Partial,
        ),
    ] {
        let (root, base, base_raw) = fixture();
        fs::write(root.path().join("a.rs"), text).unwrap();
        let (delta, shadowed) = OverlayClient::<InMemoryClient>::extract_delta(
            root.path(),
            &[root.path().join("a.rs")],
        )
        .unwrap();
        let delta_raw = sources(root.path(), &delta);
        assert_ne!(base.graph_content_hash, delta.graph_content_hash);
        let result = exercise(
            name,
            &base,
            &base_raw,
            Arc::clone(&delta),
            &delta_raw,
            shadowed,
            availability,
        );
        assert_eq!(
            result.files[1],
            base.rust_evidence.as_ref().unwrap().files[1]
        );
        if availability == Availability::Partial {
            assert!(matches!(
                result.files[0].calls,
                spur_graph::RustCallEvidence::Unknown { .. }
            ));
            assert!(!result.files[0].declarations.rows.is_empty());
        }
        if name == "zero" {
            assert!(result.files[0].declarations.rows.is_empty());
            match &result.files[0].calls {
                spur_graph::RustCallEvidence::Complete { evidence } => {
                    assert!(evidence.bodies.is_empty() && evidence.sites.is_empty())
                }
                _ => panic!("zero not captured"),
            }
        }
    }
}

// Capture the original seed's tuple neighbor through the same native export
// path. The original seed and its assertions above remain unchanged.
#[test]
fn native_packet_includes_original_tuple_neighbor() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.rs"),
        "fn leaf() {} fn a() { leaf(); }\n",
    )
    .unwrap();
    fs::write(root.path().join("b.rs"), "pub struct Keep(u8);\n").unwrap();
    let base = build(root.path());
    let raw = sources(root.path(), &base);
    fs::write(root.path().join("a.rs"), ZERO).unwrap();
    let (delta, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &[root.path().join("a.rs")])
            .unwrap();
    let delta_raw = sources(root.path(), &delta);
    exercise(
        "original-tuple-neighbor",
        &base,
        &raw,
        delta,
        &delta_raw,
        shadowed,
        Availability::Complete,
    );
}

#[test]
fn deletion_is_explicit_then_physical_and_readd_uses_current_source() {
    let (root, base, base_raw) = fixture();
    let empty = selected(root.path(), &[]);
    let shadowed = HashSet::from(["a.rs".into()]);
    let result = exercise(
        "explicit-delete",
        &base,
        &base_raw,
        empty,
        &Sources::new(),
        shadowed.clone(),
        Availability::Complete,
    );
    assert_eq!(result.files.len(), 1);
    fs::remove_file(root.path().join("a.rs")).unwrap();
    let (delta, actual_shadow) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &[root.path().join("a.rs")])
            .unwrap();
    assert_eq!(actual_shadow, shadowed);
    assert!(delta.file_manifests.is_empty());
    exercise(
        "physical-delete",
        &base,
        &base_raw,
        delta,
        &Sources::new(),
        shadowed.clone(),
        Availability::Complete,
    );
    let deleted = OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(base.clone())),
        selected(root.path(), &[]),
        shadowed.clone(),
    )
    .unwrap();
    let r = deleted.rust_evidence(&file("a.rs")).unwrap();
    assert_eq!(r.availability, Availability::Unavailable);
    assert_eq!(r.source_domain, Some(vec![]));
    assert!(r.files.is_empty() && r.origins.is_empty());
    let old = base
        .file_manifests
        .iter()
        .find(|m| m.path == "a.rs")
        .unwrap()
        .content_oid
        .clone();
    assert!(deleted
        .rust_evidence(&Filter {
            source_oid: Some(old.clone()),
            ..file("a.rs")
        })
        .is_err());
    fs::write(root.path().join("a.rs"), "pub struct Added(u8);\n").unwrap();
    let delta = selected(root.path(), &["a.rs"]);
    let raw = sources(root.path(), &delta);
    let overlay =
        OverlayClient::from_artifacts(deleted, Arc::clone(&delta), shadowed.clone()).unwrap();
    assert_eq!(
        overlay.rust_evidence(&file("a.rs")).unwrap().files,
        delta.rust_evidence.as_ref().unwrap().files
    );
    assert!(overlay
        .rust_evidence(&Filter {
            source_oid: Some(old),
            ..file("a.rs")
        })
        .is_err());
    exercise(
        "readd",
        &base,
        &base_raw,
        delta,
        &raw,
        shadowed,
        Availability::Complete,
    );
    exercise(
        "delete-all",
        &base,
        &base_raw,
        selected(root.path(), &[]),
        &Sources::new(),
        HashSet::from(["a.rs".into(), "b.rs".into()]),
        Availability::Complete,
    );
}

#[test]
fn legacy_live_delta_suppresses_base_but_is_not_a_tombstone() {
    let (root, base, base_raw) = fixture();
    fs::write(root.path().join("a.rs"), ZERO).unwrap();
    let mut delta = (*selected(root.path(), &["a.rs"])).clone();
    delta.rust_evidence = None;
    let raw = sources(root.path(), &delta);
    let r = exercise(
        "live-legacy-delta",
        &base,
        &base_raw,
        Arc::new(delta),
        &raw,
        HashSet::from(["a.rs".into()]),
        Availability::Partial,
    );
    assert_eq!(r.files.len(), 1);
    assert_eq!(
        r.source_domain.unwrap()[0].availability,
        Availability::Unavailable
    );
}

#[test]
fn known_legacy_remainder_is_partial_until_fully_replaced_or_deleted() {
    let (root, mut base, base_raw) = fixture();
    base.rust_evidence = None;
    fs::write(root.path().join("a.rs"), ZERO).unwrap();
    let delta = selected(root.path(), &["a.rs"]);
    let raw = sources(root.path(), &delta);
    let overlay = OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(base.clone())),
        Arc::clone(&delta),
        HashSet::from(["a.rs".into()]),
    )
    .unwrap();
    assert_eq!(
        overlay.rust_evidence(&file("a.rs")).unwrap().availability,
        Availability::Complete
    );
    assert_eq!(
        overlay.rust_evidence(&file("b.rs")).unwrap().availability,
        Availability::Unavailable
    );
    exercise(
        "legacy-base",
        &base,
        &base_raw,
        Arc::clone(&delta),
        &raw,
        HashSet::from(["a.rs".into()]),
        Availability::Partial,
    );
    exercise(
        "legacy-all-shadowed",
        &base,
        &base_raw,
        delta,
        &raw,
        HashSet::from(["a.rs".into(), "b.rs".into()]),
        Availability::Complete,
    );
}

#[test]
fn oid_and_original_shape_errors_are_checked_against_current_sources() {
    let (root, base, _) = fixture();
    fs::write(root.path().join("a.rs"), ZERO).unwrap();
    let delta = selected(root.path(), &["a.rs"]);
    let old = base
        .file_manifests
        .iter()
        .find(|m| m.path == "a.rs")
        .unwrap()
        .content_oid
        .clone();
    let now = delta.file_manifests[0].content_oid.clone();
    let overlay = OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(base)),
        delta,
        HashSet::from(["a.rs".into()]),
    )
    .unwrap();
    assert!(overlay
        .rust_evidence(&Filter {
            source_oid: Some(old),
            ..file("a.rs")
        })
        .is_err());
    assert_eq!(
        overlay
            .rust_evidence(&Filter {
                source_oid: Some(now),
                ..file("a.rs")
            })
            .unwrap()
            .availability,
        Availability::Complete
    );
    for filter in [
        Filter {
            source_oid: Some("bad".into()),
            file_path: None,
        },
        Filter {
            source_oid: Some("bad".into()),
            ..file("readme.md")
        },
    ] {
        assert!(overlay.rust_evidence(&filter).is_err());
    }
}

/// A backend with ordinary graph routing but independent evidence capability.
struct Custom<B: GraphQueryClient = InMemoryClient> {
    inner: B,
    result: Option<RustEvidenceQueryResult>,
    error: bool,
}
impl<B: GraphQueryClient> GraphQueryClient for Custom<B> {
    fn rust_evidence(&self, f: &Filter) -> anyhow::Result<RustEvidenceQueryResult> {
        f.validate_shape()?;
        if self.error {
            anyhow::bail!("custom recorded failure")
        }
        Ok(self.result.clone().unwrap_or(RustEvidenceQueryResult {
            availability: Availability::Unavailable,
            source_domain: None,
            files: vec![],
            origins: vec![],
        }))
    }
    fn search_symbols(
        &self,
        o: &spur_graph::SearchOptions,
    ) -> anyhow::Result<spur_graph::SearchResult> {
        self.inner.search_symbols(o)
    }
    fn find_caller_edges(&self, s: &str) -> Vec<spur_graph::OwnedCallerRecord> {
        self.inner.find_caller_edges(s)
    }
    fn find_callee_edges(&self, s: &str) -> Vec<spur_graph::OwnedCalleeRecord> {
        self.inner.find_callee_edges(s)
    }
    fn resolve_selector(&self, s: &str) -> anyhow::Result<spur_graph::SelectorResolution> {
        self.inner.resolve_selector(s)
    }
    fn symbol_by_id(&self, s: &str) -> anyhow::Result<Option<spur_graph::GraphSymbolArtifact>> {
        self.inner.symbol_by_id(s)
    }
    fn symbols_by_file(&self, s: &str) -> anyhow::Result<Vec<spur_graph::GraphSymbolArtifact>> {
        self.inner.symbols_by_file(s)
    }
    fn symbols_by_path_name(
        &self,
        p: &str,
        n: &str,
    ) -> anyhow::Result<Vec<spur_graph::GraphSymbolArtifact>> {
        self.inner.symbols_by_path_name(p, n)
    }
    fn file_manifest_by_path(
        &self,
        p: &str,
    ) -> anyhow::Result<Option<spur_graph::GraphFileManifestEntry>> {
        self.inner.file_manifest_by_path(p)
    }
    fn file_exists(&self, p: &str) -> anyhow::Result<bool> {
        self.inner.file_exists(p)
    }
    fn temporal_index(&self) -> Arc<spur_graph::temporal::TemporalIndex> {
        self.inner.temporal_index()
    }
}

/// Export custom capability/projection inputs separately from stored artifacts.
/// This permits independent projection mutations without forging a stored set.
fn export_custom(
    name: &str,
    base: &GraphIndexArtifact,
    base_raw: &Sources,
    delta: Arc<GraphIndexArtifact>,
    delta_raw: &Sources,
    shadowed: HashSet<String>,
    projection: Option<RustEvidenceQueryResult>,
    error: bool,
) {
    authenticate(base, base_raw);
    authenticate(&delta, delta_raw);
    let base_out = tempfile::tempdir().unwrap();
    let delta_out = tempfile::tempdir().unwrap();
    write_artifact_parquet(base, base_out.path(), WriteOptions::default(), vec![]).unwrap();
    write_artifact_parquet(&delta, delta_out.path(), WriteOptions::default(), vec![]).unwrap();
    let memory = OverlayClient::from_artifacts(
        Custom {
            inner: InMemoryClient::new(Arc::new(base.clone())),
            result: projection.clone(),
            error,
        },
        Arc::clone(&delta),
        shadowed.clone(),
    )
    .unwrap();
    let disk = OverlayClient::from_artifacts(
        Custom {
            inner: ParquetClient::open(base_out.path()).unwrap(),
            result: projection.clone(),
            error,
        },
        Arc::clone(&delta),
        shadowed.clone(),
    )
    .unwrap();
    let reloaded = Arc::new(read_artifact_parquet(delta_out.path()).unwrap());
    authenticate(&reloaded, delta_raw);
    let reloaded = OverlayClient::from_artifacts(
        Custom {
            inner: ParquetClient::open(base_out.path()).unwrap(),
            result: projection.clone(),
            error,
        },
        reloaded,
        shadowed.clone(),
    )
    .unwrap();
    let outputs = [
        Filter::default(),
        file("a.rs"),
        file("b.rs"),
        Filter {
            file_path: None,
            source_oid: Some("invalid-shape".into()),
        },
    ]
    .into_iter()
    .map(|filter| {
        let m = capture(&memory, &filter);
        let d = capture(&disk, &filter);
        let r = capture(&reloaded, &filter);
        assert_eq!(m, d);
        assert_eq!(m, r);
        json!({"filter":filter,"memory":m,"disk":d,"delta_reloaded":r})
    })
    .collect::<Vec<_>>();
    export_case(
        name,
        base,
        base_raw,
        &delta,
        delta_raw,
        &shadowed,
        &json!({"custom_base":true,"base_query_result":projection,"base_query_error":error,"outputs":outputs}),
        base_out.path(),
        delta_out.path(),
    );
}

#[test]
fn unknown_custom_remainder_keeps_delta_without_claiming_known_complete_domain() {
    let (root, base, base_raw) = fixture();
    fs::write(root.path().join("a.rs"), ZERO).unwrap();
    let delta = selected(root.path(), &["a.rs"]);
    let custom = Custom {
        inner: InMemoryClient::new(Arc::new(base.clone())),
        result: None,
        error: false,
    };
    let overlay =
        OverlayClient::from_artifacts(custom, Arc::clone(&delta), HashSet::from(["a.rs".into()]))
            .unwrap();
    let r = overlay.rust_evidence(&Filter::default()).unwrap();
    assert_eq!(r.availability, Availability::Partial);
    assert_eq!(r.source_domain, None);
    assert_eq!(r.files, delta.rust_evidence.as_ref().unwrap().files);
    assert_eq!(
        overlay.rust_evidence(&file("a.rs")).unwrap().availability,
        Availability::Complete
    );
    println!("D8_NATIVE {}", json!({"name":"unknown-custom","result":r}));
    export_custom(
        "unknown-custom",
        &base,
        &base_raw,
        Arc::clone(&delta),
        &sources(root.path(), &delta),
        HashSet::from(["a.rs".into()]),
        None,
        false,
    );
}
#[test]
fn no_op_delegates_values_errors_and_original_request_exactly() {
    let (root, base, raw) = fixture();
    let saved_base = base.clone();
    exercise(
        "identity",
        &base,
        &raw,
        selected(root.path(), &[]),
        &Sources::new(),
        HashSet::new(),
        Availability::Complete,
    );
    let memory = InMemoryClient::new(Arc::new(base));
    let overlay = OverlayClient::new(&memory, root.path(), &[]).unwrap();
    for f in [
        Filter::default(),
        file("a.rs"),
        file("readme.md"),
        Filter {
            source_oid: Some("stale".into()),
            ..file("a.rs")
        },
    ] {
        assert_eq!(capture(&overlay, &f), capture(&memory, &f));
    }
    let custom = Custom {
        inner: memory,
        result: None,
        error: true,
    };
    let overlay = OverlayClient::new(custom, root.path(), &[]).unwrap();
    assert_eq!(
        capture(&overlay, &Filter::default()),
        json!({"error":"custom recorded failure"})
    );
    assert_eq!(
        capture(
            &overlay,
            &Filter {
                file_path: None,
                source_oid: Some("shape".into())
            }
        ),
        json!({"error":"source_oid requires file_path"})
    );
    export_custom(
        "identity-custom-error",
        &saved_base,
        &raw,
        selected(root.path(), &[]),
        &Sources::new(),
        HashSet::new(),
        None,
        true,
    );
}
#[test]
fn conflicting_same_key_original_inventories_are_rejected() {
    let (root, base, base_raw) = fixture();
    let delta = selected(root.path(), &["a.rs"]);
    let memory = InMemoryClient::new(Arc::new(base.clone()));
    let mut result = memory.rust_evidence(&Filter::default()).unwrap();
    let mut conflict = delta.rust_evidence.as_ref().unwrap().origins[0].clone();
    conflict.sources[0].content_oid = "conflicting-original".into();
    result.origins.push(conflict);
    let custom = Custom {
        inner: memory,
        result: Some(result.clone()),
        error: false,
    };
    let overlay =
        OverlayClient::from_artifacts(custom, Arc::clone(&delta), HashSet::from(["a.rs".into()]))
            .unwrap();
    let error = overlay.rust_evidence(&Filter::default()).unwrap_err();
    assert!(format!("{error:#}").contains("conflicting Rust origin inventories"));
    println!(
        "D8_NATIVE {}",
        json!({"name":"origin-conflict","error":format!("{error:#}")})
    );
    export_custom(
        "origin-conflict",
        &base,
        &base_raw,
        Arc::clone(&delta),
        &sources(root.path(), &delta),
        HashSet::from(["a.rs".into()]),
        Some(result),
        false,
    );
}

#[test]
fn declaration_unknown_retains_the_delta_complete_call_lane() {
    let (root, base, base_raw) = fixture();
    fs::write(
        root.path().join("a.rs"),
        "struct New(pub u32);\nfn changed() { new_call(); }\n",
    )
    .unwrap();
    let mut delta = (*selected(root.path(), &["a.rs"])).clone();
    let original_delta = delta.clone();
    let raw = sources(root.path(), &delta);
    authenticate(&original_delta, &raw);
    let f = &mut delta.rust_evidence.as_mut().unwrap().files[0];
    f.declarations.sources[0].availability = spur_graph::RustDeclarationAvailability::Unknown {
        reason: "recorded declaration uncertainty".into(),
    };
    let expected = f.clone();
    let inputs = raw
        .iter()
        .map(|(file_path, bytes)| spur_graph::RustDeclarationSource { file_path, bytes })
        .collect::<Vec<_>>();
    let authentication_error = delta.validate_rust_evidence_sources(&inputs).unwrap_err();
    let delta = Arc::new(delta);
    let overlay = OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(base.clone())),
        Arc::clone(&delta),
        HashSet::from(["a.rs".into()]),
    )
    .unwrap();
    let r = overlay.rust_evidence(&file("a.rs")).unwrap();
    assert_eq!(r.availability, Availability::Partial);
    assert_eq!(r.files, vec![expected]);
    match &r.files[0].calls {
        spur_graph::RustCallEvidence::Complete { evidence } => assert!(!evidence.sites.is_empty()),
        _ => panic!("available call lane lost"),
    }
    println!(
        "D8_NATIVE {}",
        json!({"name":"declaration-unknown-call-lane", "result":r})
    );
    let base_out = tempfile::tempdir().unwrap();
    let delta_out = tempfile::tempdir().unwrap();
    write_artifact_parquet(&base, base_out.path(), WriteOptions::default(), vec![]).unwrap();
    write_artifact_parquet(&delta, delta_out.path(), WriteOptions::default(), vec![]).unwrap();
    let shadowed = HashSet::from(["a.rs".into()]);
    let disk = OverlayClient::from_artifacts(
        ParquetClient::open(base_out.path()).unwrap(),
        Arc::clone(&delta),
        shadowed.clone(),
    )
    .unwrap();
    let reloaded = OverlayClient::from_artifacts(
        ParquetClient::open(base_out.path()).unwrap(),
        Arc::new(read_artifact_parquet(delta_out.path()).unwrap()),
        shadowed.clone(),
    )
    .unwrap();
    let outputs = [Filter::default(), file("a.rs"), file("b.rs")]
        .into_iter()
        .map(|filter| {
            let m = capture(&overlay, &filter);
            let d = capture(&disk, &filter);
            let rr = capture(&reloaded, &filter);
            assert_eq!(m, d);
            assert_eq!(m, rr);
            json!({"filter":filter,"memory":m,"disk":d,"delta_reloaded":rr})
        })
        .collect::<Vec<_>>();
    export_case(
        "declaration-unknown-call-lane",
        &base,
        &base_raw,
        &delta,
        &raw,
        &shadowed,
        &json!({"controlled_stored_metadata_mutation":true,"original_delta":original_delta,
            "full_source_authentication_error":format!("{authentication_error:#}"),"outputs":outputs}),
        base_out.path(),
        delta_out.path(),
    );
}

/// Arbitrary-input native replay. Paths refer to caller-supplied files and disk
/// directories. JSON-pointer mutations operate on private decoded copies.
#[test]
fn d8_native_replay() {
    let Some(request) = std::env::var_os("SPUR_D8_REPLAY_REQUEST") else {
        return;
    };
    let request: Value = serde_json::from_slice(&fs::read(request).unwrap()).unwrap();
    fn load(request: &Value, name: &str) -> GraphIndexArtifact {
        let mut v: Value =
            serde_json::from_slice(&fs::read(request[name].as_str().unwrap()).unwrap()).unwrap();
        if let Some(mutations) = request
            .get(format!("{name}_mutations"))
            .and_then(Value::as_array)
        {
            for mutation in mutations {
                *v.pointer_mut(mutation["pointer"].as_str().unwrap())
                    .expect("existing JSON pointer") = mutation["value"].clone();
            }
        }
        serde_json::from_value(v).unwrap()
    }
    let base = load(&request, "base");
    let delta = Arc::new(load(&request, "delta"));
    let shadowed: HashSet<String> = serde_json::from_value(request["shadowed"].clone()).unwrap();
    let filters: Vec<Filter> = serde_json::from_value(request["filters"].clone()).unwrap();
    let mut observed = json!({"base_graph":base.graph_content_hash,"delta_graph":delta.graph_content_hash,"shadowed":request["shadowed"],"outputs":[]});
    fn replay<B: GraphQueryClient>(
        inner: B,
        delta: Arc<GraphIndexArtifact>,
        shadowed: &HashSet<String>,
        filters: &[Filter],
        request: &Value,
    ) -> Vec<Value> {
        let mut projection = request
            .get("base_query_result")
            .cloned()
            .filter(|v| !v.is_null());
        if let Some(mutations) = request
            .get("base_result_mutations")
            .and_then(Value::as_array)
        {
            for m in mutations {
                *projection
                    .as_mut()
                    .expect("base_query_result required")
                    .pointer_mut(m["pointer"].as_str().unwrap())
                    .expect("existing projection pointer") = m["value"].clone();
            }
        }
        let error = request["base_query_error"].as_bool().unwrap_or(false);
        let custom_base =
            request["custom_base"].as_bool().unwrap_or(false) || projection.is_some() || error;
        let custom = Custom {
            inner: &inner,
            result: projection.map(serde_json::from_value).transpose().unwrap(),
            error,
        };
        let backend: &dyn GraphQueryClient = if custom_base { &custom } else { &inner };
        let overlay = OverlayClient::from_artifacts(backend, delta, shadowed.clone());
        filters
            .iter()
            .map(|f| match &overlay {
                Ok(c) => capture(c, f),
                Err(e) => json!({"error":format!("{e:#}")}),
            })
            .collect()
    }
    let values = replay(
        InMemoryClient::new(Arc::new(base.clone())),
        Arc::clone(&delta),
        &shadowed,
        &filters,
        &request,
    );
    let mut outputs = filters
        .iter()
        .zip(values)
        .map(|(f, v)| json!({"filter":f,"memory":v}))
        .collect::<Vec<_>>();
    if let Some(dir) = request.get("base_disk").and_then(Value::as_str) {
        let base_disk = ParquetClient::open(Path::new(dir));
        match base_disk {
            Ok(disk) => {
                let values = replay(disk, Arc::clone(&delta), &shadowed, &filters, &request);
                for (row, value) in outputs.iter_mut().zip(values) {
                    row["disk"] = value;
                }
            }
            Err(e) => {
                observed["disk_open_error"] = json!(format!("{e:#}"));
            }
        }
        if let Some(delta_dir) = request.get("delta_disk").and_then(Value::as_str) {
            match read_artifact_parquet(Path::new(delta_dir)) {
                Ok(reloaded) => match ParquetClient::open(Path::new(dir)) {
                    Ok(disk) => {
                        let values =
                            replay(disk, Arc::new(reloaded), &shadowed, &filters, &request);
                        for (row, value) in outputs.iter_mut().zip(values) {
                            row["delta_reloaded"] = value;
                        }
                    }
                    Err(e) => {
                        observed["delta_reloaded_error"] = json!(format!("{e:#}"));
                    }
                },
                Err(e) => {
                    observed["delta_load_error"] = json!(format!("{e:#}"));
                }
            }
        }
    }
    // Authenticate each original artifact separately against independent raw
    // source directories; never authenticate an overlay projection as an artifact.
    for (name, graph) in [("base", &base), ("delta", delta.as_ref())] {
        if let Some(dir) = request
            .get(format!("{name}_source"))
            .and_then(Value::as_str)
        {
            let bytes = graph
                .file_manifests
                .iter()
                .map(|m| {
                    (
                        m.path.clone(),
                        fs::read(Path::new(dir).join(&m.path)).unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let inputs = bytes
                .iter()
                .map(|(path, bytes)| spur_graph::RustDeclarationSource {
                    file_path: path,
                    bytes,
                })
                .collect::<Vec<_>>();
            observed[format!("{name}_authentication")] =
                match graph.validate_rust_evidence_sources(&inputs) {
                    Ok(()) => json!({"ok":true}),
                    Err(e) => json!({"error":format!("{e:#}")}),
                };
        }
    }
    observed["outputs"] = json!(outputs);
    println!("D8_REPLAY {observed}");
    if let Some(output) = std::env::var_os("SPUR_D8_REPLAY_OUTPUT") {
        write_json(Path::new(&output), &observed);
    }
}
