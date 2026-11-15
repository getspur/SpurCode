use std::{collections::BTreeSet, fs, path::Path, sync::Arc};

use spur_graph::query_client::{GraphQueryClient, InMemoryClient, OverlayClient, ParquetClient};
use spur_graph::{
    artifact_from_facts, build_facts, write_artifact_parquet, GraphIndexArtifact,
    MacroEvidenceAvailability as Availability, MacroEvidenceSourceScope,
    MacroInvocationFilter as Filter, MacroInvocationQueryResult as ResultSet, MacroScopeKind,
    WriteOptions,
};

const SOURCE: &str = "#![allow(dead_code)]\nconst V: () = item!();\ntype T = kind!();\n#[cfg(test)]\nfn a() { assert!(true); assert!(true); }\nfn z() { alias!(); }\n";

fn build(root: &Path) -> GraphIndexArtifact {
    artifact_from_facts(&build_facts(root, None).unwrap().0, root).unwrap()
}

fn fixture() -> (tempfile::TempDir, GraphIndexArtifact) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), SOURCE).unwrap();
    fs::write(root.path().join("b.rs"), "fn keep() { keep!(); }\n").unwrap();
    fs::write(root.path().join("empty.rs"), "fn empty() {}\n").unwrap();
    fs::write(
        root.path().join("physical.ipynb#cell:rust-cell.rs"),
        "fn physical() { yes!(); }\n",
    )
    .unwrap();
    fs::write(root.path().join("note.ipynb"), r#"{"cells":[{"cell_type":"code","id":"rust-cell.rs","metadata":{"language":"rust"},"source":["fn cell() { panic!(); }"],"outputs":[],"execution_count":null}],"metadata":{"kernelspec":{"language":"rust","name":"rust","display_name":"Rust"}},"nbformat":4,"nbformat_minor":5}"#).unwrap();
    let artifact = build(root.path());
    (root, artifact)
}

fn file(path: &str) -> Filter {
    Filter {
        file_path: Some(path.into()),
        ..Filter::default()
    }
}

fn disk(artifact: &GraphIndexArtifact) -> (tempfile::TempDir, ParquetClient) {
    let out = tempfile::tempdir().unwrap();
    write_artifact_parquet(artifact, out.path(), WriteOptions::default(), vec![]).unwrap();
    let client = ParquetClient::open(out.path()).unwrap();
    (out, client)
}

fn assert_scopes(result: &ResultSet) {
    let wanted: BTreeSet<_> = result
        .rows
        .iter()
        .flat_map(|row| {
            std::iter::once(row.enclosing.stable_symbol_id.as_str()).chain(
                row.attributes
                    .iter()
                    .map(|a| a.owner.stable_symbol_id.as_str()),
            )
        })
        .collect();
    assert_eq!(
        wanted,
        result
            .scopes
            .iter()
            .map(|s| s.scope.stable_symbol_id.as_str())
            .collect()
    );
    for row in &result.rows {
        for scope in result.scopes.iter().filter(|s| {
            wanted.contains(s.scope.stable_symbol_id.as_str()) && s.file_path == row.file_path
        }) {
            assert_eq!(scope.source_oid, row.source_oid);
        }
        assert!(result
            .snapshots
            .iter()
            .any(|s| s.source_oids.get(&row.file_path) == Some(&row.source_oid)));
    }
}

#[test]
fn filters_forwarding_and_scope_locations_match_parquet() {
    let (_root, artifact) = fixture();
    let (_out, parquet) = disk(&artifact);
    let memory = InMemoryClient::new(Arc::new(artifact.clone()));
    let caller = artifact
        .symbols
        .iter()
        .find(|s| s.entity_name == "a")
        .unwrap();
    let filters = [
        Filter::default(),
        file("a.rs"),
        Filter {
            caller_stable_symbol_id: Some(caller.stable_symbol_id.clone()),
            ..Filter::default()
        },
        Filter {
            caller_stable_symbol_id: Some(caller.stable_symbol_id.clone()),
            ..file("b.rs")
        },
    ];
    for filter in filters {
        let expected = memory.macro_invocations(&filter).unwrap();
        assert_eq!(expected.availability, Availability::Complete);
        assert_eq!(
            expected.source_scope,
            Some(MacroEvidenceSourceScope::PhysicalRustFiles)
        );
        assert_eq!(parquet.macro_invocations(&filter).unwrap(), expected);
        let reference: &dyn GraphQueryClient = &memory;
        assert_eq!((&reference).macro_invocations(&filter).unwrap(), expected);
        assert_scopes(&expected);
    }
    let selected = memory
        .macro_invocations(&Filter {
            caller_stable_symbol_id: Some(caller.stable_symbol_id.clone()),
            ..file("a.rs")
        })
        .unwrap();
    assert_eq!(selected.rows.len(), 2, "repeated sites must survive");
    assert_ne!(selected.rows[0].site_id, selected.rows[1].site_id);
    assert!(selected.rows[0].byte_range < selected.rows[1].byte_range);
    assert_eq!(
        memory
            .symbol_by_id(selected.rows[0].caller_stable_symbol_id.as_deref().unwrap())
            .unwrap()
            .unwrap(),
        *caller
    );
    let all = memory.macro_invocations(&file("a.rs")).unwrap();
    for kind in [MacroScopeKind::Const, MacroScopeKind::Type] {
        let row = all.rows.iter().find(|r| r.enclosing.kind == kind).unwrap();
        assert!(row.caller_stable_symbol_id.is_none());
        let scope = all
            .scopes
            .iter()
            .find(|s| s.scope == row.enclosing)
            .unwrap();
        assert_eq!(scope.file_path, "a.rs");
        assert!(
            scope.byte_range[0] <= row.byte_range[0] && scope.byte_range[1] >= row.byte_range[1]
        );
    }
    assert_eq!(
        memory.artifact().edges,
        artifact.edges,
        "queries cannot insert Calls edges"
    );
    println!(
        "M5_FILTER_FACTS {}",
        serde_json::json!({"result":selected,"expected":artifact.macro_invocations.as_ref().unwrap().rows.iter().filter(|r| r.caller_stable_symbol_id.as_deref() == Some(&caller.stable_symbol_id)).collect::<Vec<_>>() })
    );
}

#[test]
fn complete_empty_legacy_unavailable_and_unsupported_source_parity() {
    let (_root, mut artifact) = fixture();
    let (_out, parquet) = disk(&artifact);
    let memory = InMemoryClient::new(Arc::new(artifact.clone()));
    for path in [
        "empty.rs",
        "physical.ipynb#cell:rust-cell.rs",
        "note.ipynb#cell:rust-cell.rs",
        "note.ipynb",
    ] {
        let expected = memory.macro_invocations(&file(path)).unwrap();
        assert_eq!(parquet.macro_invocations(&file(path)).unwrap(), expected);
        assert_eq!(
            expected.availability,
            if path == "empty.rs" || path.starts_with("physical") {
                Availability::Complete
            } else {
                Availability::UnsupportedSource
            }
        );
        if path == "empty.rs" {
            assert!(expected.rows.is_empty());
            assert!(expected.scopes.is_empty());
        }
        if path.starts_with("physical") {
            assert_eq!(expected.rows.len(), 1);
        }
    }
    artifact.macro_invocations = None;
    let (_legacy, parquet) = disk(&artifact);
    let memory = InMemoryClient::new(Arc::new(artifact));
    assert_eq!(
        memory
            .macro_invocations(&Filter::default())
            .unwrap()
            .availability,
        Availability::Unavailable
    );
    assert_eq!(
        parquet.macro_invocations(&Filter::default()).unwrap(),
        memory.macro_invocations(&Filter::default()).unwrap()
    );
}

#[test]
fn stale_source_and_missing_scope_fail_even_when_filtered_out() {
    let (_root, artifact) = fixture();
    let (_out, parquet) = disk(&artifact);
    let memory = InMemoryClient::new(Arc::new(artifact.clone()));
    let stale = Filter {
        source_oid: Some("0".repeat(40)),
        ..file("a.rs")
    };
    assert!(memory.macro_invocations(&stale).is_err());
    assert!(parquet.macro_invocations(&stale).is_err());
    let valid = Filter {
        source_oid: Some(
            artifact
                .file_manifests
                .iter()
                .find(|f| f.path == "a.rs")
                .unwrap()
                .content_oid
                .clone(),
        ),
        ..file("a.rs")
    };
    assert_eq!(
        memory.macro_invocations(&valid).unwrap(),
        parquet.macro_invocations(&valid).unwrap()
    );
    for corrupt_source in [false, true] {
        let mut broken = artifact.clone();
        let set = broken.macro_invocations.as_mut().unwrap();
        if corrupt_source {
            set.rows[0].source_oid = "0".repeat(40);
        } else {
            set.scopes.clear();
        }
        assert!(
            InMemoryClient::new(Arc::new(broken))
                .macro_invocations(&file("empty.rs"))
                .is_err(),
            "full provenance must be checked before filtering"
        );
    }
}

#[test]
fn corrupt_core_tables_fail_before_any_filter_and_freeze_after_validation() {
    let (_root, artifact) = fixture();
    for table in ["macro_invocations.parquet", "macro_scopes.parquet"] {
        let (out, parquet) = disk(&artifact);
        fs::write(out.path().join(table), b"corrupt").unwrap();
        assert!(parquet.macro_invocations(&file("empty.rs")).is_err());
    }
    let (out, parquet) = disk(&artifact);
    let expected = parquet.macro_invocations(&file("a.rs")).unwrap();
    fs::write(out.path().join("macro_scopes.parquet"), b"corrupt").unwrap();
    assert_eq!(
        parquet.macro_invocations(&file("a.rs")).unwrap(),
        expected,
        "validated context must be immutable"
    );
    assert!(ParquetClient::open(out.path())
        .unwrap()
        .macro_invocations(&file("a.rs"))
        .is_err());
    fs::remove_file(out.path().join("edges.parquet")).unwrap();
    assert_eq!(parquet.macro_invocations(&file("a.rs")).unwrap(), expected);
}

#[test]
fn overlay_replaces_deletes_moves_and_retains_scopes_and_snapshot_provenance() {
    let (root, artifact) = fixture();
    let (_out, parquet) = disk(&artifact);
    let before = artifact.macro_invocations.as_ref().unwrap().clone();
    fs::write(
        root.path().join("a.rs"),
        "#![allow(dead_code)]\nconst NEW: () = new!();\n",
    )
    .unwrap();
    fs::rename(
        root.path().join("physical.ipynb#cell:rust-cell.rs"),
        root.path().join("moved.rs"),
    )
    .unwrap();
    fs::remove_file(root.path().join("empty.rs")).unwrap();
    fs::write(root.path().join("added.rs"), "fn added() { added!(); }\n").unwrap();
    let canonical_root = root.path().canonicalize().unwrap();
    let changed = [
        "a.rs",
        "physical.ipynb#cell:rust-cell.rs",
        "moved.rs",
        "empty.rs",
        "added.rs",
    ]
    .map(|p| canonical_root.join(p));
    let (delta, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &changed).unwrap();
    let memory = OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(artifact)),
        delta.clone(),
        shadowed.clone(),
    )
    .unwrap();
    let parquet = OverlayClient::from_artifacts(parquet, delta.clone(), shadowed).unwrap();
    let oracle = InMemoryClient::new(Arc::new(build(root.path())));
    for filter in [
        Filter::default(),
        file("a.rs"),
        file("b.rs"),
        file("moved.rs"),
        file("empty.rs"),
        file("physical.ipynb#cell:rust-cell.rs"),
    ] {
        let actual = memory.macro_invocations(&filter).unwrap();
        assert_eq!(actual, parquet.macro_invocations(&filter).unwrap());
        let expected = oracle.macro_invocations(&filter).unwrap();
        assert_eq!(actual.rows, expected.rows);
        assert_eq!(actual.scopes, expected.scopes);
        assert_eq!(actual.availability, Availability::Complete);
        assert_scopes(&actual);
    }
    let result = memory.macro_invocations(&Filter::default()).unwrap();
    assert!(!result
        .scopes
        .iter()
        .any(|s| s.file_path == "physical.ipynb#cell:rust-cell.rs"));
    let keep = before.rows.iter().find(|r| r.file_path == "b.rs").unwrap();
    assert!(result.rows.contains(keep));
    println!(
        "M5_OVERLAY_FACTS {}",
        serde_json::json!({"result":result,"expected":oracle.macro_invocations(&Filter::default()).unwrap(),"base":before,"delta":delta.macro_invocations})
    );
}

#[test]
fn overlay_unknown_base_stays_partial_except_replaced_file_and_identity_forwards() {
    let (root, mut artifact) = fixture();
    artifact.macro_invocations = None;
    let base = InMemoryClient::new(Arc::new(artifact));
    let identity = OverlayClient::new(&base, root.path(), &[]).unwrap();
    assert_eq!(
        identity
            .macro_invocations(&Filter::default())
            .unwrap()
            .availability,
        Availability::Unavailable
    );
    fs::write(root.path().join("a.rs"), "fn fresh() { fresh!(); }\n").unwrap();
    let overlay = OverlayClient::new(
        base,
        root.path(),
        &[root.path().canonicalize().unwrap().join("a.rs")],
    )
    .unwrap();
    assert_eq!(
        overlay
            .macro_invocations(&Filter::default())
            .unwrap()
            .availability,
        Availability::Partial
    );
    assert_eq!(
        overlay
            .macro_invocations(&file("b.rs"))
            .unwrap()
            .availability,
        Availability::Unavailable
    );
    assert_eq!(
        overlay
            .macro_invocations(&file("a.rs"))
            .unwrap()
            .availability,
        Availability::Complete
    );
}

struct ExternalBackend;
impl GraphQueryClient for ExternalBackend {
    fn search_symbols(
        &self,
        _: &spur_graph::SearchOptions,
    ) -> anyhow::Result<spur_graph::SearchResult> {
        anyhow::bail!("unsupported")
    }
    fn find_caller_edges(&self, _: &str) -> Vec<spur_graph::OwnedCallerRecord> {
        vec![]
    }
    fn find_callee_edges(&self, _: &str) -> Vec<spur_graph::OwnedCalleeRecord> {
        vec![]
    }
    fn resolve_selector(&self, _: &str) -> anyhow::Result<spur_graph::SelectorResolution> {
        anyhow::bail!("unsupported")
    }
    fn symbol_by_id(&self, _: &str) -> anyhow::Result<Option<spur_graph::GraphSymbolArtifact>> {
        Ok(None)
    }
    fn symbols_by_file(&self, _: &str) -> anyhow::Result<Vec<spur_graph::GraphSymbolArtifact>> {
        Ok(vec![])
    }
    fn symbols_by_path_name(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Vec<spur_graph::GraphSymbolArtifact>> {
        Ok(vec![])
    }
    fn file_manifest_by_path(
        &self,
        _: &str,
    ) -> anyhow::Result<Option<spur_graph::GraphFileManifestEntry>> {
        Ok(None)
    }
    fn file_exists(&self, _: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn temporal_index(&self) -> Arc<spur_graph::temporal::TemporalIndex> {
        unimplemented!("unused")
    }
}

#[test]
fn external_backend_must_opt_in_to_macro_coverage() {
    let result = ExternalBackend
        .macro_invocations(&Filter::default())
        .unwrap();
    assert_eq!(result.availability, Availability::Unavailable);
    assert!(result.rows.is_empty());
    assert!(result.snapshots.is_empty());
    assert_eq!(result.source_scope, None);
}

#[test]
fn unsupported_notebook_caller_never_claims_complete_empty_coverage() {
    let (_root, artifact) = fixture();
    let caller = artifact
        .symbols
        .iter()
        .find(|s| s.entity_name == "cell")
        .expect("Rust notebook caller");
    let filter = Filter {
        caller_stable_symbol_id: Some(caller.stable_symbol_id.clone()),
        ..Filter::default()
    };
    let (_out, parquet) = disk(&artifact);
    let memory = InMemoryClient::new(Arc::new(artifact));
    let result = memory.macro_invocations(&filter).unwrap();
    assert_eq!(result.availability, Availability::UnsupportedSource);
    assert_eq!(parquet.macro_invocations(&filter).unwrap(), result);
}

#[test]
fn evidence_only_const_and_type_scopes_are_retrieved_without_graph_lookup() {
    let (_root, mut artifact) = fixture();
    let set = artifact.macro_invocations.as_mut().unwrap();
    for scope in &mut set.scopes {
        if matches!(
            scope.scope.kind,
            MacroScopeKind::Const | MacroScopeKind::Type
        ) {
            let old = scope.scope.stable_symbol_id.clone();
            let id = format!("macro-scope-v1:test-{old}");
            scope.scope.stable_symbol_id = id.clone();
            for row in &mut set.rows {
                if row.enclosing.stable_symbol_id == old {
                    row.enclosing.stable_symbol_id = id.clone();
                }
                for attribute in &mut row.attributes {
                    if attribute.owner.stable_symbol_id == old {
                        attribute.owner.stable_symbol_id = id.clone();
                    }
                }
            }
        }
    }
    let (_out, parquet) = disk(&artifact);
    let memory = InMemoryClient::new(Arc::new(artifact));
    let result = memory.macro_invocations(&file("a.rs")).unwrap();
    assert_eq!(parquet.macro_invocations(&file("a.rs")).unwrap(), result);
    assert_scopes(&result);
    for scope in result
        .scopes
        .iter()
        .filter(|s| matches!(s.scope.kind, MacroScopeKind::Const | MacroScopeKind::Type))
    {
        assert!(memory
            .symbol_by_id(&scope.scope.stable_symbol_id)
            .unwrap()
            .is_none());
    }
}

#[test]
fn overlay_caller_filters_match_current_sources_and_unsupported_coverage() {
    let (root, artifact) = fixture();
    let notebook_caller = artifact
        .symbols
        .iter()
        .find(|s| s.entity_name == "cell")
        .unwrap()
        .stable_symbol_id
        .clone();
    let old_caller = artifact
        .symbols
        .iter()
        .find(|s| s.entity_name == "a")
        .unwrap()
        .stable_symbol_id
        .clone();
    let old_oid = artifact
        .file_manifests
        .iter()
        .find(|f| f.path == "a.rs")
        .unwrap()
        .content_oid
        .clone();
    let (_out, parquet) = disk(&artifact);
    fs::write(
        root.path().join("a.rs"),
        "fn fresh() { fresh!(); fresh!(); }\n",
    )
    .unwrap();
    let paths = [root.path().canonicalize().unwrap().join("a.rs")];
    let memory =
        OverlayClient::new(InMemoryClient::new(Arc::new(artifact)), root.path(), &paths).unwrap();
    let parquet = OverlayClient::new(parquet, root.path(), &paths).unwrap();
    let oracle = InMemoryClient::new(Arc::new(build(root.path())));
    let new_caller = oracle
        .artifact()
        .symbols
        .iter()
        .find(|s| s.entity_name == "fresh")
        .unwrap()
        .stable_symbol_id
        .clone();
    for caller in [new_caller, old_caller, notebook_caller] {
        let filter = Filter {
            caller_stable_symbol_id: Some(caller),
            ..Filter::default()
        };
        let actual = memory.macro_invocations(&filter).unwrap();
        let expected = oracle.macro_invocations(&filter).unwrap();
        assert_eq!(actual, parquet.macro_invocations(&filter).unwrap());
        assert_eq!(actual.rows, expected.rows);
        assert_eq!(actual.scopes, expected.scopes);
        assert_eq!(actual.availability, expected.availability);
    }
    let stale = Filter {
        source_oid: Some(old_oid),
        ..file("a.rs")
    };
    assert!(memory.macro_invocations(&stale).is_err());
    assert!(parquet.macro_invocations(&stale).is_err());
}

// Preserve all footer metadata and cardinality so a pruned query must rely on
// the one-time full-body checksum rather than silently trusting skipped rows.
fn tamper_table_string(path: &Path, name: &str) {
    use arrow_array::{ArrayRef, RecordBatch, StringArray};
    use parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};
    let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).unwrap()).unwrap();
    let schema = reader.schema().clone();
    let batches = reader
        .build()
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let mut writer =
        ArrowWriter::try_new(fs::File::create(path).unwrap(), schema.clone(), None).unwrap();
    let column_index = schema.index_of(name).unwrap();
    for batch in batches {
        let strings = batch
            .column(column_index)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let mut values = strings
            .iter()
            .map(|s| s.map(str::to_owned))
            .collect::<Vec<_>>();
        values[0] = Some("same-count-tamper".to_owned());
        let mut columns: Vec<ArrayRef> = batch.columns().to_vec();
        columns[column_index] = Arc::new(StringArray::from(values));
        writer
            .write(&RecordBatch::try_new(schema.clone(), columns).unwrap())
            .unwrap();
    }
    writer.close().unwrap();
}

#[test]
fn filtered_parquet_rejects_same_count_body_tamper_and_needs_no_edges() {
    let (_root, artifact) = fixture();
    for (table, column) in [
        ("macro_invocations.parquet", "raw_macro_path"),
        ("macro_scopes.parquet", "scope_id"),
    ] {
        let (out, parquet) = disk(&artifact);
        tamper_table_string(&out.path().join(table), column);
        let error = parquet.macro_invocations(&file("empty.rs")).unwrap_err();
        assert!(format!("{error:#}").contains("checksum"), "{error:#}");
    }
    let (out, parquet) = disk(&artifact);
    fs::remove_file(out.path().join("edges.parquet")).unwrap();
    let result = parquet.macro_invocations(&file("a.rs")).unwrap();
    assert_eq!(result.rows.len(), 5);
    assert_scopes(&result);
}

#[test]
fn file_scope_and_whole_artifact_complete_empty_round_trip() {
    let root = tempfile::tempdir().unwrap();
    for source in ["register!();\n", "fn empty() {}\n"] {
        fs::write(root.path().join("top.rs"), source).unwrap();
        let artifact = build(root.path());
        let (_out, parquet) = disk(&artifact);
        let memory = InMemoryClient::new(Arc::new(artifact));
        let result = memory.macro_invocations(&file("top.rs")).unwrap();
        assert_eq!(result.availability, Availability::Complete);
        assert_eq!(parquet.macro_invocations(&file("top.rs")).unwrap(), result);
        assert_scopes(&result);
        if source.starts_with("register") {
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0].enclosing.kind, MacroScopeKind::File);
            assert!(result.rows[0].caller_stable_symbol_id.is_none());
        } else {
            assert!(result.rows.is_empty());
            assert!(result.scopes.is_empty());
            assert_eq!(result.snapshots[0].source_oids.len(), 1);
        }
    }
}

#[test]
fn source_oid_without_file_is_rejected_by_overlay_as_by_base() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), "fn a() { assert!(true); }\n").unwrap();
    fs::write(root.path().join("b.rs"), "fn b() {}\n").unwrap();
    let artifact = build(root.path());
    let caller = artifact
        .symbols
        .iter()
        .find(|s| s.entity_name == "a")
        .unwrap();
    let caller_only = Filter {
        caller_stable_symbol_id: Some(caller.stable_symbol_id.clone()),
        ..Filter::default()
    };
    let invalid = Filter {
        source_oid: Some(
            artifact
                .file_manifests
                .iter()
                .find(|f| f.path == "a.rs")
                .unwrap()
                .content_oid
                .clone(),
        ),
        ..caller_only.clone()
    };
    let explicit_file = Filter {
        file_path: Some("a.rs".into()),
        ..invalid.clone()
    };
    let (_out, parquet) = disk(&artifact);
    let memory = InMemoryClient::new(Arc::new(artifact));
    let expected = memory.macro_invocations(&caller_only).unwrap();
    assert_eq!(expected.rows.len(), 1);
    let identity_memory = OverlayClient::new(&memory, root.path(), &[]).unwrap();
    let identity_parquet = OverlayClient::new(&parquet, root.path(), &[]).unwrap();

    // The selected caller is untouched. Only dirty-overlay routing can turn
    // the invalid original filter into one with an inferred file_path.
    fs::write(root.path().join("b.rs"), "fn b() { other!(); }\n").unwrap();
    let changed = root.path().canonicalize().unwrap().join("b.rs");
    let (delta, shadowed) =
        OverlayClient::<InMemoryClient>::extract_delta(root.path(), &[changed]).unwrap();
    let overlay_memory =
        OverlayClient::from_artifacts(&memory, delta.clone(), shadowed.clone()).unwrap();
    let overlay_parquet = OverlayClient::from_artifacts(&parquet, delta, shadowed).unwrap();
    let clients: [(&str, &dyn GraphQueryClient); 6] = [
        ("memory", &memory),
        ("parquet", &parquet),
        ("identity_memory", &identity_memory),
        ("identity_parquet", &identity_parquet),
        ("overlay_memory", &overlay_memory),
        ("overlay_parquet", &overlay_parquet),
    ];
    let mut facts = Vec::new();
    for (backend, client) in clients {
        for filter in [&caller_only, &explicit_file] {
            let actual = client.macro_invocations(filter).unwrap();
            assert_eq!(actual.availability, Availability::Complete);
            assert_eq!(actual.rows, expected.rows, "{backend}");
            assert_eq!(actual.scopes, expected.scopes, "{backend}");
            facts.push(serde_json::json!({"backend":backend,"filter":filter,"result":actual}));
        }
        let actual = client.macro_invocations(&invalid);
        facts.push(match actual {
            Ok(result) => serde_json::json!({"backend":backend,"filter":invalid,"result":result}),
            Err(error) => {
                serde_json::json!({"backend":backend,"filter":invalid,"error":format!("{error:#}")})
            }
        });
    }
    println!(
        "M5_REWORK_FILTER_FACTS {}",
        serde_json::json!({"cases":facts})
    );
    for fact in facts
        .iter()
        .filter(|f| f["filter"] == serde_json::to_value(&invalid).unwrap())
    {
        assert!(
            fact.get("error").is_some(),
            "overlay accepted source_oid without explicit file_path: {fact}"
        );
        assert!(fact["error"]
            .as_str()
            .unwrap()
            .contains("source_oid requires file_path"));
    }
}

#[test]
fn parquet_macro_queries_reject_missing_source_manifest_rows() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), "fn a() { assert!(true); }\n").unwrap();
    fs::write(root.path().join("empty.rs"), "fn empty() {}\n").unwrap();
    let artifact = build(root.path());
    assert_eq!(artifact.file_manifests.len(), 2);
    let (out, parquet) = disk(&artifact);
    assert_eq!(parquet.manifest().row_counts.file_manifests, 2);
    let valid = parquet.macro_invocations(&Filter::default()).unwrap();
    assert_eq!(valid.availability, Availability::Complete);
    assert_eq!(valid.snapshots[0].source_oids.len(), 2);
    assert!(valid.snapshots[0].source_oids.contains_key("empty.rs"));
    assert!(!valid.rows.iter().any(|row| row.file_path == "empty.rs"));

    let mut incomplete = artifact.clone();
    incomplete.file_manifests.retain(|f| f.path != "empty.rs");
    // The normal artifact writer rejects inconsistent evidence. Corrupt the
    // persisted table directly to exercise the reader's cardinality check.
    {
        use arrow_array::{BooleanArray, StringArray};
        use arrow_select::filter::filter_record_batch;
        use parquet::{
            arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter},
            file::properties::WriterProperties,
        };
        let path = out.path().join("file_manifests.parquet");
        let reader =
            ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&path).unwrap()).unwrap();
        let schema = reader.schema().clone();
        let metadata = reader
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .cloned();
        let batches = reader
            .build()
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let properties = WriterProperties::builder()
            .set_key_value_metadata(metadata)
            .build();
        let mut writer = ArrowWriter::try_new(
            fs::File::create(&path).unwrap(),
            schema.clone(),
            Some(properties),
        )
        .unwrap();
        let path_column = schema.index_of("path").unwrap();
        for batch in batches {
            let paths = batch
                .column(path_column)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let keep = BooleanArray::from_iter(paths.iter().map(|p| p != Some("empty.rs")));
            writer
                .write(&filter_record_batch(&batch, &keep).unwrap())
                .unwrap();
        }
        writer.close().unwrap();
    }
    // Keep the original manifest, both checksummed macro tables, and a valid
    // supporting Parquet table; only a zero-site source row has been omitted.
    let mut facts = Vec::new();
    for filter in [Filter::default(), file("a.rs"), file("empty.rs")] {
        // Use a fresh client so every filter must validate before certifying
        // coverage. The already validated client retains its frozen context.
        let client = ParquetClient::open(out.path()).unwrap();
        let actual = client.macro_invocations(&filter);
        facts.push(match actual {
            Ok(result) => serde_json::json!({"filter":filter,"result":result}),
            Err(error) => serde_json::json!({"filter":filter,"error":format!("{error:#}")}),
        });
    }
    println!(
        "M5_REWORK_SOURCE_FACTS {}",
        serde_json::json!({
            "expected_sources":artifact.file_manifests,
            "decoded_sources":incomplete.file_manifests,
            "declared_count":parquet.manifest().row_counts.file_manifests,
            "valid_result":valid,
            "cases":facts,
        })
    );
    for fact in facts {
        assert!(
            fact.get("error").is_some(),
            "macro query accepted declared2/actual1 source manifest rows: {fact}"
        );
        let error = fact["error"].as_str().unwrap();
        assert!(
            error.contains("file_manifests") && error.contains("2") && error.contains("1"),
            "{error}"
        );
    }
    assert_eq!(
        parquet.macro_invocations(&Filter::default()).unwrap(),
        valid
    );
}

#[test]
fn stored_macro_callers_require_the_complete_function_identity() {
    let (_root, artifact) = fixture();
    let caller = artifact
        .symbols
        .iter()
        .position(|symbol| symbol.entity_name == "a")
        .unwrap();

    for mismatch in [
        "id",
        "path",
        "kind",
        "byte_start",
        "byte_end",
        "line_start",
        "line_end",
        "missing",
    ] {
        let mut changed = artifact.clone();
        let symbol = &mut changed.symbols[caller];
        match mismatch {
            "id" => symbol.stable_symbol_id.push_str("-different"),
            "path" => symbol.file_path = "b.rs".into(),
            "kind" => symbol.symbol_kind = "module".into(),
            "byte_start" => symbol.byte_range[0] += 1,
            "byte_end" => symbol.byte_range[1] += 1,
            "line_start" => symbol.line_range[0] += 1,
            "line_end" => symbol.line_range[1] += 1,
            "missing" => {
                changed.symbols.remove(caller);
            }
            _ => unreachable!(),
        }
        // Even an unrelated filter must validate every persisted caller.
        let error = InMemoryClient::new(Arc::new(changed))
            .macro_invocations(&file("empty.rs"))
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("macro caller is not an actual graph function symbol"),
            "{mismatch}: {error:#}"
        );
    }
}

#[test]
fn stored_macro_callers_accept_functions_methods_and_full_key_collisions() {
    let (_root, artifact) = fixture();
    let caller = artifact
        .symbols
        .iter()
        .position(|symbol| symbol.entity_name == "a")
        .unwrap();
    let expected = InMemoryClient::new(Arc::new(artifact.clone()))
        .macro_invocations(&file("a.rs"))
        .unwrap();

    for kind in ["function", "method"] {
        let mut changed = artifact.clone();
        changed.symbols[caller].symbol_kind = kind.into();
        let actual = InMemoryClient::new(Arc::new(changed))
            .macro_invocations(&file("a.rs"))
            .unwrap();
        assert_eq!(actual, expected, "{kind}");
    }

    // A map that keeps only the first or last symbol for an ID would lose
    // the genuine match. Membership must include path and both ranges.
    for first in [false, true] {
        let mut changed = artifact.clone();
        let mut collision = changed.symbols[caller].clone();
        collision.file_path = "b.rs".into();
        collision.byte_range[1] += 1;
        collision.line_range[1] += 1;
        if first {
            changed.symbols.insert(0, collision);
        } else {
            changed.symbols.push(collision);
        }
        let actual = InMemoryClient::new(Arc::new(changed))
            .macro_invocations(&file("a.rs"))
            .unwrap();
        assert_eq!(actual, expected, "collision first={first}");
    }

    let mut duplicated = artifact.clone();
    duplicated.symbols.push(duplicated.symbols[caller].clone());
    assert_eq!(
        InMemoryClient::new(Arc::new(duplicated))
            .macro_invocations(&file("a.rs"))
            .unwrap(),
        expected,
        "identical graph keys must preserve existential caller matching"
    );
}
