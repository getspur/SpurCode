use std::{collections::BTreeMap, fs, sync::Arc};

use serde_json::{json, Value};
use spur_graph::evidence::{
    providers::{default_registry, legacy_macros::*, rust::*},
    *,
};
use spur_graph::temporal::TemporalIndex;
use spur_graph::*;

const RUST: &str = "pub struct Pair(pub i32, pub bool);\nunsafe extern \"C\" { pub fn foreign(x: i32); }\nfn leaf() {}\nfn run() { leaf(); let _f = || leaf(); panic!(\"hello\"); }\n";

fn inputs() -> Vec<(String, Vec<u8>)> {
    vec![
        ("lib.rs".into(), RUST.as_bytes().to_vec()),
        (
            "cfg.rs".into(),
            b"#[cfg(feature = \"x\")] fn hidden() { leaf(); }\n".to_vec(),
        ),
        ("zero.rs".into(), b"pub struct Zero;\n".to_vec()),
        (
            "independent.py".into(),
            b"def independent():\n    return 7\n".to_vec(),
        ),
    ]
}

fn graph(inputs: &[(String, Vec<u8>)]) -> GraphIndexArtifact {
    let root = tempfile::tempdir().unwrap();
    for (path, bytes) in inputs {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap()
}

fn sources(inputs: &[(String, Vec<u8>)]) -> Vec<EvidenceSource<'_>> {
    inputs
        .iter()
        .filter(|(p, _)| p.ends_with(".rs"))
        .map(|(p, b)| EvidenceSource {
            file_path: p,
            bytes: b,
        })
        .collect()
}

#[test]
fn rust_codec_keeps_actual_captured_empty() {
    let graph = graph(&[]);
    let set = graph.rust_evidence.as_ref().unwrap();
    assert!(set.files.is_empty());
    let ext = encode_rust_evidence(&graph, set).unwrap();
    assert_eq!(ext.descriptor.presence, EvidencePresence::Captured);
    assert_eq!(ext.domain, EvidenceDomain::Known { sources: vec![] });
    assert_eq!(ext.tables.len(), 5);
    assert_eq!(decode_rust_evidence(&graph, &ext).unwrap(), *set);
    RustEvidenceProvider
        .validate_sources(&graph, &ext, &[])
        .unwrap();
}

#[test]
fn actual_nonempty_sets_roundtrip_with_headers_origins_and_unknown_reason() {
    let inputs = inputs();
    let graph = graph(&inputs);
    let rust = graph.rust_evidence.as_ref().unwrap();
    assert!(rust.files.iter().any(|f| !f.declarations.rows.is_empty()));
    assert!(rust
        .files
        .iter()
        .any(|f| !f.declarations.foreign_functions.is_empty()));
    assert!(rust.files.iter().any(|f| matches!(&f.calls, RustCallEvidence::Complete { evidence } if !evidence.sites.is_empty())));
    assert!(rust.files.iter().any(|f| matches!(
        &f.calls,
        RustCallEvidence::Unknown {
            reason: RustEvidenceUnknownReason::UnevaluatedCfg
        }
    )));
    let ext = encode_rust_evidence(&graph, rust).unwrap();
    assert_eq!(decode_rust_evidence(&graph, &ext).unwrap(), *rust);
    RustEvidenceProvider
        .validate_sources(&graph, &ext, &sources(&inputs))
        .unwrap();
    assert_eq!(
        ext.tables
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "rust_declarations",
            "rust_evidence_bodies",
            "rust_evidence_origins",
            "rust_evidence_sites",
            "rust_evidence_sources"
        ]
    );
    for table in &ext.tables {
        assert_eq!(table.storage_path, format!("{}.parquet", table.name));
    }
    let macros = graph.macro_invocations.as_ref().unwrap();
    assert!(!macros.rows.is_empty() && !macros.scopes.is_empty());
    let ext = encode_macro_evidence(&graph, macros).unwrap();
    assert_eq!(decode_macro_evidence(&graph, &ext).unwrap(), *macros);
    LegacyMacroEvidenceProvider
        .validate_sources(&graph, &ext, &sources(&inputs))
        .unwrap();
    assert_eq!(
        ext.descriptor.metadata["original_format"],
        "spur-macro-invocation-v1"
    );
    let EvidenceDomain::Known { sources } = &ext.domain else {
        panic!("known macro domain")
    };
    assert!(sources.iter().all(|s| s.byte_length.is_none()));
    assert!(ext
        .origins
        .iter()
        .all(|o| o.key.graph_content_hash == graph.graph_content_hash));
}

#[test]
fn macro_codec_keeps_actual_captured_empty_and_authenticates_legacy_metadata() {
    let graph = graph(&[]);
    let set = graph.macro_invocations.as_ref().unwrap();
    let mut ext = encode_macro_evidence(&graph, set).unwrap();
    assert_eq!(ext.tables.len(), 2);
    assert_eq!(decode_macro_evidence(&graph, &ext).unwrap(), *set);
    ext.descriptor
        .metadata
        .insert("original_format".into(), "invented".into());
    assert!(decode_macro_evidence(&graph, &ext).is_err());
}

fn table_mut<'a>(ext: &'a mut EvidenceExtension, name: &str) -> &'a mut EvidenceTable {
    ext.tables.iter_mut().find(|t| t.name == name).unwrap()
}

#[test]
fn every_duplicated_declaration_scalar_and_header_is_authenticated() {
    let graph = graph(&inputs());
    let ext = encode_rust_evidence(&graph, graph.rust_evidence.as_ref().unwrap()).unwrap();
    let declarations = ext
        .tables
        .iter()
        .find(|t| t.name == "rust_declarations")
        .unwrap();
    for column in &declarations.columns {
        let mut bad = ext.clone();
        let table = table_mut(&mut bad, "rust_declarations");
        let i = table
            .columns
            .iter()
            .position(|c| c.name == column.name)
            .unwrap();
        table.rows[0][i] = match &table.rows[0][i] {
            EvidenceValue::Int64(n) => EvidenceValue::Int64(n + 1),
            EvidenceValue::Utf8(s) => EvidenceValue::Utf8(format!("{s}forged")),
            _ => unreachable!(),
        };
        assert!(
            decode_rust_evidence(&graph, &bad).is_err(),
            "{}",
            column.name
        );
    }
    let mut bad = ext.clone();
    let table = table_mut(&mut bad, "rust_evidence_sources");
    let i = table
        .columns
        .iter()
        .position(|c| c.name == "declaration_header_json")
        .unwrap();
    table.rows[0][i] = EvidenceValue::Utf8("{}".into());
    assert!(decode_rust_evidence(&graph, &bad).is_err());
}

#[test]
fn rehashed_native_mutations_keep_integrity_stored_and_saved_bytes_distinct() {
    let inputs = inputs();
    let graph = graph(&inputs);
    let ext = encode_rust_evidence(&graph, graph.rust_evidence.as_ref().unwrap()).unwrap();
    let mut body = ext.clone();
    let table = table_mut(&mut body, "rust_evidence_sites");
    let col = table
        .columns
        .iter()
        .position(|c| c.name == "call_text")
        .unwrap();
    table.rows[0][col] = EvidenceValue::Utf8("fake()".into());
    assert!(evidence_extension_checksum(&body).is_ok());
    assert!(RustEvidenceProvider
        .validate_sources(&graph, &body, &sources(&inputs))
        .is_err());
    let mut owner = ext.clone();
    let table = table_mut(&mut owner, "rust_evidence_bodies");
    let col = table
        .columns
        .iter()
        .position(|c| c.name == "definition_stable_symbol_id")
        .unwrap();
    table.rows[0][col] = EvidenceValue::Utf8("forged-owner".into());
    assert!(evidence_extension_checksum(&owner).is_ok());
    assert!(decode_rust_evidence(&graph, &owner).is_err());
    let mut changed = inputs.clone();
    changed
        .iter_mut()
        .find(|(p, _)| p == "lib.rs")
        .unwrap()
        .1
        .push(b' ');
    assert!(RustEvidenceProvider
        .validate_sources(&graph, &ext, &sources(&changed))
        .is_err());
    assert!(RustEvidenceProvider
        .validate_sources(&graph, &ext, &sources(&inputs)[1..])
        .is_err());
    let macros = encode_macro_evidence(&graph, graph.macro_invocations.as_ref().unwrap()).unwrap();
    let mut forged = macros.clone();
    let t = table_mut(&mut forged, "macro_invocations");
    let c = t
        .columns
        .iter()
        .position(|c| c.name == "raw_macro_path")
        .unwrap();
    t.rows[0][c] = EvidenceValue::Utf8("fake_macro".into());
    assert!(evidence_extension_checksum(&forged).is_ok());
    assert!(LegacyMacroEvidenceProvider
        .validate_sources(&graph, &forged, &sources(&inputs))
        .is_err());
    assert!(LegacyMacroEvidenceProvider
        .validate_sources(&graph, &macros, &sources(&changed))
        .is_err());
}

#[test]
fn source_domains_use_authoritative_manifest_and_physical_policy() {
    let mut graph = graph(&inputs());
    let domain = RustEvidenceProvider.source_domain(&graph).unwrap();
    assert_eq!(domain.len(), 3);
    assert!(!domain.iter().any(|s| s.file_path.ends_with(".py")));
    assert!(!RustEvidenceProvider.supports_path("external://other.rs"));
    assert!(!RustEvidenceProvider.supports_path("book.ipynb#cell:x"));
    assert!(RustEvidenceProvider.supports_path("cell_like.rs"));
    graph.file_manifests[0].content_oid = "gitlink:abc".into();
    graph.graph_content_hash = compute_graph_content_hash(
        graph
            .file_manifests
            .iter()
            .map(|f| (&f.path, &f.content_oid)),
    );
    assert!(!RustEvidenceProvider
        .source_domain(&graph)
        .unwrap()
        .iter()
        .any(|s| s.source_oid.starts_with("gitlink:")));
    graph.files.clear();
    assert!(RustEvidenceProvider.source_domain(&graph).is_err());
}

#[test]
fn absence_and_authoritative_cell_like_physical_paths_remain_distinct() {
    let mut graph = graph(&inputs());
    let mut ext = encode_rust_evidence(&graph, graph.rust_evidence.as_ref().unwrap()).unwrap();
    ext.descriptor.presence = EvidencePresence::Absent;
    ext.origins.clear();
    ext.tables.clear();
    ext.domain = EvidenceDomain::Unknown;
    assert_eq!(
        default_registry().validate(&graph, &ext).unwrap().domain,
        EvidenceDomain::Unknown
    );
    assert!(decode_rust_evidence(&graph, &ext).is_err());
    graph.rust_evidence = None;
    graph.macro_invocations = None;
    assert_eq!(RustEvidenceProvider.source_domain(&graph).unwrap().len(), 3);
    let physical = super_graph_cell_fixture();
    assert!(LegacyMacroEvidenceProvider
        .source_domain(&physical)
        .unwrap()
        .iter()
        .any(|s| s.file_path == "physical#cell:real.rs"));
    assert!(RustEvidenceProvider
        .source_domain(&physical)
        .unwrap()
        .iter()
        .any(|s| s.file_path == "physical#cell:real.rs"));
}

fn super_graph_cell_fixture() -> GraphIndexArtifact {
    graph(&[(
        "physical#cell:real.rs".into(),
        b"struct Physical;\n".to_vec(),
    )])
}

struct TestCounterProvider;
impl EvidenceProvider for TestCounterProvider {
    fn provider_id(&self) -> &'static str {
        "test.counter"
    }
    fn supports_path(&self, p: &str) -> bool {
        !p.starts_with("external://") && !p.contains("#cell:")
    }
    fn source_domain(
        &self,
        graph: &GraphIndexArtifact,
    ) -> Result<Vec<EvidenceSourceKey>, EvidenceError> {
        EvidenceInventory::empty().validate_integrity(graph)?;
        let domain: Vec<_> = graph
            .file_manifests
            .iter()
            .filter(|m| self.supports_path(&m.path) && !m.content_oid.starts_with("gitlink:"))
            .map(|m| EvidenceSourceKey {
                file_path: m.path.clone(),
                stable_file_id: m.stable_file_id.clone(),
                source_oid: m.content_oid.clone(),
            })
            .collect();
        if domain.iter().any(|s| {
            graph
                .files
                .iter()
                .filter(|f| f.file_path == s.file_path && f.stable_file_id == s.stable_file_id)
                .count()
                != 1
                || graph
                    .files
                    .iter()
                    .filter(|f| f.file_path == s.file_path)
                    .count()
                    != 1
                || graph
                    .files
                    .iter()
                    .filter(|f| f.stable_file_id == s.stable_file_id)
                    .count()
                    != 1
        }) {
            return Err(EvidenceError::Semantic {
                provider_id: self.provider_id().into(),
                code: "owner".into(),
                message: "authoritative physical file required".into(),
            });
        }
        Ok(domain)
    }
    fn validate_stored(
        &self,
        graph: &GraphIndexArtifact,
        ext: &EvidenceExtension,
    ) -> Result<EvidenceValidation, EvidenceError> {
        if ext.descriptor.provider_id != self.provider_id()
            || ext.descriptor.schema_version != "counter-v1"
        {
            return Err(EvidenceError::Unsupported {
                provider_id: ext.descriptor.provider_id.clone(),
                schema_version: ext.descriptor.schema_version.clone(),
            });
        }
        self.source_domain(graph)?;
        let lengths = match &ext.domain {
            EvidenceDomain::Known { sources } => sources
                .iter()
                .map(|s| (s.source.file_path.as_str(), s.byte_length))
                .collect::<BTreeMap<_, _>>(),
            _ => BTreeMap::new(),
        };
        let expected = counter_with_lengths(graph, 0, |p| lengths.get(p).copied().flatten());
        let valid = ext.descriptor == expected.descriptor
            && lengths.values().all(Option::is_some)
            && ext.origins == expected.origins
            && ext.domain == expected.domain
            && ext.tables.len() == 1
            && {
                let mut t = ext.tables[0].clone();
                let nonnegative = t
                    .rows
                    .iter()
                    .all(|r| matches!(r.last(), Some(EvidenceValue::Int64(n)) if *n >= 0));
                for r in &mut t.rows {
                    if let Some(last) = r.last_mut() {
                        *last = EvidenceValue::Int64(0);
                    }
                }
                nonnegative && t == expected.tables[0]
            };
        if !valid {
            return Err(EvidenceError::Semantic {
                provider_id: self.provider_id().into(),
                code: "counter".into(),
                message: "fixed schema/source bindings and nonnegative integers required".into(),
            });
        }
        Ok(EvidenceValidation {
            status: EvidenceSemanticStatus::Validated,
            domain: ext.domain.clone(),
        })
    }
    fn validate_sources(
        &self,
        graph: &GraphIndexArtifact,
        ext: &EvidenceExtension,
        sources: &[EvidenceSource<'_>],
    ) -> Result<(), EvidenceError> {
        self.validate_stored(graph, ext)?;
        let domain = self.source_domain(graph)?;
        let EvidenceDomain::Known { sources: coverage } = &ext.domain else {
            unreachable!()
        };
        if sources.len() != domain.len()
            || domain.iter().any(|s| {
                sources
                    .iter()
                    .filter(|b| {
                        b.file_path == s.file_path
                            && git_blob_oid(b.bytes) == s.source_oid
                            && coverage.iter().any(|c| {
                                c.source == *s && c.byte_length == Some(b.bytes.len() as u64)
                            })
                    })
                    .count()
                    != 1
            })
        {
            return Err(EvidenceError::Semantic {
                provider_id: self.provider_id().into(),
                code: "source".into(),
                message: "exact physical sources required".into(),
            });
        }
        Ok(())
    }
}

fn counter(graph: &GraphIndexArtifact, n: i64) -> EvidenceExtension {
    counter_for_sources(graph, n, &inputs())
}
fn counter_for_sources(
    graph: &GraphIndexArtifact,
    n: i64,
    sources: &[(String, Vec<u8>)],
) -> EvidenceExtension {
    counter_with_lengths(graph, n, |p| {
        sources
            .iter()
            .find(|(path, _)| path == p)
            .map(|(_, b)| b.len() as u64)
    })
}
fn counter_with_lengths(
    graph: &GraphIndexArtifact,
    n: i64,
    length: impl Fn(&str) -> Option<u64>,
) -> EvidenceExtension {
    let keys = TestCounterProvider.source_domain(graph).unwrap();
    let origin = EvidenceOriginKey {
        manifest_version: graph.manifest_version.clone(),
        graph_content_hash: graph.graph_content_hash.clone(),
    };
    EvidenceExtension {
        descriptor: EvidenceDescriptor {
            provider_id: "test.counter".into(),
            schema_version: "counter-v1".into(),
            extractor_version: "fixture-v1".into(),
            component_versions: BTreeMap::new(),
            presence: EvidencePresence::Captured,
            metadata: BTreeMap::new(),
        },
        origins: vec![EvidenceOrigin {
            key: origin.clone(),
            sources: graph
                .file_manifests
                .iter()
                .map(|m| EvidenceSourceKey {
                    file_path: m.path.clone(),
                    stable_file_id: m.stable_file_id.clone(),
                    source_oid: m.content_oid.clone(),
                })
                .collect(),
        }],
        domain: EvidenceDomain::Known {
            sources: keys
                .iter()
                .map(|s| EvidenceSourceCoverage {
                    source: s.clone(),
                    byte_length: length(&s.file_path),
                    origin: Some(origin.clone()),
                    coverage: EvidenceCoverage::Complete,
                })
                .collect(),
        },
        tables: vec![EvidenceTable {
            name: "values".into(),
            storage_path: "evidence/test.counter/values.parquet".into(),
            scope: EvidenceTableScope::File {
                file_path_column: "file_path".into(),
                source_oid_column: "source_oid".into(),
                origin_hash_column: None,
            },
            columns: vec![
                EvidenceColumn {
                    name: "file_path".into(),
                    data_type: EvidenceColumnType::Utf8,
                    nullable: false,
                },
                EvidenceColumn {
                    name: "source_oid".into(),
                    data_type: EvidenceColumnType::Utf8,
                    nullable: false,
                },
                EvidenceColumn {
                    name: "value".into(),
                    data_type: EvidenceColumnType::Int64,
                    nullable: false,
                },
            ],
            primary_key: vec!["file_path".into()],
            order_by: vec!["file_path".into()],
            metadata: BTreeMap::new(),
            rows: keys
                .iter()
                .map(|s| {
                    vec![
                        EvidenceValue::Utf8(s.file_path.clone()),
                        EvidenceValue::Utf8(s.source_oid.clone()),
                        EvidenceValue::Int64(n),
                    ]
                })
                .collect(),
        }],
    }
}

#[test]
fn independent_counter_and_opaque_unsupported_and_duplicate_controls() {
    let graph = graph(&inputs());
    let counter = counter(&graph, 7);
    assert!(counter.tables[0]
        .rows
        .iter()
        .any(|r| r[0] == EvidenceValue::Utf8("independent.py".into())));
    let providers: &[&dyn EvidenceProvider] = &[
        &RustEvidenceProvider,
        &LegacyMacroEvidenceProvider,
        &TestCounterProvider,
    ];
    let registry = EvidenceRegistry::new(providers).unwrap();
    assert_eq!(
        registry.validate(&graph, &counter).unwrap().status,
        EvidenceSemanticStatus::Validated
    );
    assert_eq!(
        default_registry().validate(&graph, &counter).unwrap(),
        EvidenceValidation {
            status: EvidenceSemanticStatus::UnknownProvider,
            domain: EvidenceDomain::Unknown
        }
    );
    assert!(EvidenceRegistry::new(&[&RustEvidenceProvider, &RustEvidenceProvider]).is_err());
    let mut bad = counter.clone();
    bad.tables[0].rows[0][2] = EvidenceValue::Int64(-1);
    assert!(registry.validate(&graph, &bad).is_err());
    bad.tables[0].rows[0][0] = EvidenceValue::Null;
    assert!(default_registry().validate(&graph, &bad).is_err());
    let mut unsupported =
        encode_rust_evidence(&graph, graph.rust_evidence.as_ref().unwrap()).unwrap();
    unsupported.descriptor.schema_version = "future-v999".into();
    assert_eq!(
        default_registry().validate(&graph, &unsupported).unwrap(),
        EvidenceValidation {
            status: EvidenceSemanticStatus::UnsupportedSchema,
            domain: EvidenceDomain::Unknown
        }
    );
    assert!(decode_rust_evidence(&graph, &unsupported).is_err());
    unsupported.descriptor.provider_id = "opaque.rust".into();
    assert!(decode_rust_evidence(&graph, &unsupported).is_err());
}

// The fixture backend authenticates complete stored data before exposing a
// projection. Convenience adapters never receive or invent a graph owner view.
struct GenericClient {
    inner: InMemoryClient,
    root_projection: Option<EvidenceQueryResult>,
}
impl GraphQueryClient for GenericClient {
    fn evidence(&self, filter: &EvidenceFilter) -> anyhow::Result<EvidenceQueryResult> {
        filter.validate_shape()?;
        if let Some(projection) = &self.root_projection {
            return Ok(projection.clone());
        }
        let graph = self.inner.artifact();
        let ext = if filter.provider_id == "spur.rust.physical" {
            encode_rust_evidence(graph, graph.rust_evidence.as_ref().unwrap())?
        } else {
            encode_macro_evidence(graph, graph.macro_invocations.as_ref().unwrap())?
        };
        let validation = default_registry().validate(graph, &ext)?;
        if let Some(oid) = &filter.source_oid {
            anyhow::ensure!(
                graph
                    .file_manifests
                    .iter()
                    .any(|m| Some(&m.path) == filter.file_path.as_ref() && m.content_oid == *oid),
                "stale oid"
            );
        }
        let availability = match &ext.domain {
            EvidenceDomain::Known { sources } => {
                let selected = sources
                    .iter()
                    .filter(|s| {
                        filter
                            .file_path
                            .as_ref()
                            .is_none_or(|p| p == &s.source.file_path)
                    })
                    .collect::<Vec<_>>();
                if filter.file_path.is_some() && selected.is_empty() {
                    EvidenceAvailability::Unavailable
                } else if selected
                    .iter()
                    .all(|s| s.coverage == EvidenceCoverage::Complete)
                {
                    EvidenceAvailability::Complete
                } else {
                    EvidenceAvailability::Partial
                }
            }
            EvidenceDomain::Unknown => EvidenceAvailability::Unavailable,
        };
        Ok(EvidenceQueryResult {
            availability,
            semantic_status: validation.status,
            descriptor: Some(ext.descriptor),
            domain: ext.domain,
            origins: ext.origins,
            tables: ext.tables,
        })
    }
    fn search_symbols(&self, o: &SearchOptions) -> anyhow::Result<SearchResult> {
        self.inner.search_symbols(o)
    }
    fn find_caller_edges(&self, s: &str) -> Vec<OwnedCallerRecord> {
        self.inner.find_caller_edges(s)
    }
    fn find_callee_edges(&self, s: &str) -> Vec<OwnedCalleeRecord> {
        self.inner.find_callee_edges(s)
    }
    fn resolve_selector(&self, s: &str) -> anyhow::Result<CodeSelectorResolution> {
        self.inner.resolve_selector(s)
    }
    fn symbol_by_id(&self, s: &str) -> anyhow::Result<Option<GraphSymbolArtifact>> {
        self.inner.symbol_by_id(s)
    }
    fn symbols_by_file(&self, p: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.inner.symbols_by_file(p)
    }
    fn symbols_by_path_name(&self, p: &str, n: &str) -> anyhow::Result<Vec<GraphSymbolArtifact>> {
        self.inner.symbols_by_path_name(p, n)
    }
    fn file_manifest_by_path(&self, p: &str) -> anyhow::Result<Option<GraphFileManifestEntry>> {
        self.inner.file_manifest_by_path(p)
    }
    fn file_exists(&self, p: &str) -> anyhow::Result<bool> {
        self.inner.file_exists(p)
    }
    fn temporal_index(&self) -> Arc<TemporalIndex> {
        self.inner.temporal_index()
    }
}

#[test]
fn typed_helpers_use_generic_route_and_keep_conjunctive_filters_and_defaults() {
    let graph = Arc::new(graph(&inputs()));
    let legacy = InMemoryClient::new(graph.clone());
    assert_eq!(
        query_rust_evidence(&legacy, &RustEvidenceFilter::default())
            .unwrap()
            .availability,
        RustEvidenceAvailability::Unavailable
    );
    assert_eq!(
        query_macro_evidence(&legacy, &MacroInvocationFilter::default())
            .unwrap()
            .availability,
        MacroEvidenceAvailability::Unavailable
    );
    let client = GenericClient {
        inner: legacy,
        root_projection: None,
    };
    let rust = query_rust_evidence(&client, &RustEvidenceFilter::default()).unwrap();
    assert_eq!(rust.files, graph.rust_evidence.as_ref().unwrap().files);
    assert_eq!(rust.origins, graph.rust_evidence.as_ref().unwrap().origins);
    assert_eq!(rust.availability, RustEvidenceAvailability::Partial);
    let m = graph
        .macro_invocations
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .find(|r| r.caller_stable_symbol_id.is_some())
        .unwrap();
    let filter = MacroInvocationFilter {
        file_path: Some(m.file_path.clone()),
        source_oid: Some(m.source_oid.clone()),
        caller_stable_symbol_id: m.caller_stable_symbol_id.clone(),
    };
    let actual = query_macro_evidence(&client, &filter).unwrap();
    assert_eq!(
        actual.rows,
        graph
            .macro_invocations
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .filter(|r| r.file_path == m.file_path
                && r.caller_stable_symbol_id == m.caller_stable_symbol_id)
            .cloned()
            .collect::<Vec<_>>()
    );
    assert!(!actual.scopes.is_empty() && !actual.snapshots.is_empty());
    let mut wrong = filter.clone();
    wrong.source_oid = Some("stale".into());
    assert!(query_macro_evidence(&client, &wrong).is_err());
    wrong = filter.clone();
    wrong.file_path = None;
    assert!(query_macro_evidence(&client, &wrong).is_err());
    let wrong = RustEvidenceFilter {
        file_path: None,
        source_oid: Some("oid".into()),
    };
    assert!(query_rust_evidence(&client, &wrong).is_err());
    let filter = RustEvidenceFilter {
        file_path: Some("zero.rs".into()),
        source_oid: Some(
            graph
                .file_manifests
                .iter()
                .find(|m| m.path == "zero.rs")
                .unwrap()
                .content_oid
                .clone(),
        ),
    };
    assert_eq!(
        query_rust_evidence(&client, &filter).unwrap(),
        client.inner.rust_evidence(&filter).unwrap()
    );
    let empty = GenericClient {
        inner: InMemoryClient::new(Arc::new(crate::graph(&[]))),
        root_projection: None,
    };
    assert_eq!(
        query_rust_evidence(&empty, &RustEvidenceFilter::default())
            .unwrap()
            .availability,
        RustEvidenceAvailability::Complete
    );
    assert_eq!(
        query_macro_evidence(&empty, &MacroInvocationFilter::default())
            .unwrap()
            .availability,
        MacroEvidenceAvailability::Complete
    );
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayRequest {
    fixtures: Vec<(String, Vec<u8>)>,
    cases: Vec<ReplayCase>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayCase {
    name: String,
    graph: GraphIndexArtifact,
    extension: EvidenceExtension,
    sources: Vec<(String, Vec<u8>)>,
    #[serde(default)]
    rust_checksum_set: Option<RustEvidenceSet>,
    #[serde(default)]
    macro_checksum_set: Option<MacroInvocationEvidenceSet>,
    #[serde(default)]
    counter_registered: bool,
}

fn replay(case: &ReplayCase) -> Value {
    let mut extension = case.extension.clone();
    let rehash = if let Some(set) = &case.rust_checksum_set {
        let checksum = rust_evidence_checksum_blake3_hex(set).unwrap();
        for table in &mut extension.tables {
            table
                .metadata
                .insert("checksum_blake3".into(), checksum.clone());
        }
        Some(
            json!({"prefix":"spur-rust-evidence-tables-v1\0","compact_json":serde_json::to_string(set).unwrap(),"checksum":checksum}),
        )
    } else if let Some(set) = &case.macro_checksum_set {
        let mut hash = blake3::Hasher::new();
        hash.update(b"spur-macro-evidence-tables-v1\0");
        let bytes = serde_json::to_vec(set).unwrap();
        hash.update(&bytes);
        let checksum = hash.finalize().to_hex().to_string();
        for table in &mut extension.tables {
            table
                .metadata
                .insert("checksum_blake3".into(), checksum.clone());
        }
        Some(
            json!({"prefix":"spur-macro-evidence-tables-v1\0","compact_json":String::from_utf8(bytes).unwrap(),"checksum":checksum}),
        )
    } else {
        None
    };
    let src: Vec<_> = case
        .sources
        .iter()
        .map(|(p, b)| EvidenceSource {
            file_path: p,
            bytes: b,
        })
        .collect();
    let mut inventory = EvidenceInventory::empty();
    let canonical = inventory.replace(extension.clone());
    let provider: &dyn EvidenceProvider = if case.extension.descriptor.provider_id == "test.counter"
    {
        &TestCounterProvider
    } else if case.extension.descriptor.provider_id == "spur.rust.macros" {
        &LegacyMacroEvidenceProvider
    } else {
        &RustEvidenceProvider
    };
    let providers: &[&dyn EvidenceProvider] = &[
        &RustEvidenceProvider,
        &LegacyMacroEvidenceProvider,
        &TestCounterProvider,
    ];
    let registry = if case.counter_registered {
        EvidenceRegistry::new(providers).unwrap()
    } else {
        default_registry()
    };
    json!({"name":case.name,"graph":case.graph,"provided_extension":case.extension,"extension":extension,"sources":case.sources,"legacy_rehash":rehash,"counter_registered":case.counter_registered,
        "source_bindings":case.sources.iter().map(|(p,b)|json!({"path":p,"bytes":b.len(),"git_blob_oid":git_blob_oid(b),"sha256":sha256(b)})).collect::<Vec<_>>(),
        "integrity":EvidenceInventory { schema_version:EVIDENCE_SCHEMA_VERSION.into(),extensions:vec![extension.clone()] }.validate_integrity(&case.graph),
        "registry":registry.validate(&case.graph,&extension),
        "unregistered_registry":default_registry().validate(&case.graph,&extension),
        "duplicate_registry":EvidenceRegistry::new(&[&RustEvidenceProvider,&RustEvidenceProvider]).map(|_|()),
        "stored":provider.validate_stored(&case.graph,&extension),
        "source_validation":provider.validate_sources(&case.graph,&extension,&src),
        "rust_decode":decode_rust_evidence(&case.graph,&extension),"macro_decode":decode_macro_evidence(&case.graph,&extension),
        "canonicalization":canonical,"extension_compact_json":inventory.extensions.first().map(|e|serde_json::to_string(e).unwrap()),
        "extension_prefix":"spur-evidence-extension-v1\0","checksum":evidence_extension_checksum(&extension),
        "tables":inventory.extensions.first().map(|e|e.tables.iter().map(|t|json!({"name":t.name,"prefix":"spur-evidence-table-v1\0","compact_json":serde_json::to_string(t).unwrap(),"checksum":evidence_table_checksum(t)})).collect::<Vec<_>>())})
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn compiled_bindings() -> Value {
    macro_rules! bind { ($path:literal) => {{ let bytes=include_bytes!($path); json!({"path":$path,"bytes":bytes.len(),"sha256":sha256(bytes)}) }}; }
    json!([
        bind!("../src/evidence/mod.rs"),
        bind!("../src/evidence/contract.rs"),
        bind!("../src/evidence/providers/mod.rs"),
        bind!("../src/evidence/providers/rust.rs"),
        bind!("../src/evidence/providers/legacy_macros.rs"),
        bind!("generic_evidence_providers.rs"),
        bind!("rust_evidence_contract.rs"),
        bind!("../src/rust_evidence.rs"),
        bind!("../src/rust_evidence/query.rs"),
        bind!("../src/macro_invocations.rs"),
        bind!("../src/rust_declarations.rs"),
        bind!("../src/panic_evidence/call_sites.rs"),
        bind!("../src/panic_evidence/mod.rs"),
        bind!("../src/query_client.rs"),
        bind!("../src/schema.rs"),
        bind!("../src/store/build.rs"),
        bind!("../src/store/rust_evidence.rs"),
        bind!("../src/store/macro_invocations.rs"),
        bind!("../src/extract/tree_sitter.rs"),
        bind!("../src/extract/rust_macros.rs"),
        bind!("../Cargo.toml"),
        bind!("../../../Cargo.lock"),
        bind!("../../../.cargo/config.toml"),
        bind!("../../../rust-toolchain.toml"),
        bind!("../../../scripts/spur-cargo")
    ])
}

#[test]
fn native_export_replay_from_supplied_request() {
    let Ok(request_path) = std::env::var("G02_EVIDENCE_REQUEST") else {
        return;
    };
    let raw = fs::read(&request_path).unwrap();
    let request: ReplayRequest = serde_json::from_slice(&raw).unwrap();
    let graph = graph(&request.fixtures);
    let rust = graph.rust_evidence.as_ref().unwrap();
    let macros = graph.macro_invocations.as_ref().unwrap();
    let extensions = [
        encode_rust_evidence(&graph, rust).unwrap(),
        encode_macro_evidence(&graph, macros).unwrap(),
        counter_for_sources(&graph, 7, &request.fixtures),
    ];
    let generated = extensions
        .iter()
        .map(|ext| {
            let provider: &dyn EvidenceProvider = match ext.descriptor.provider_id.as_str() {
                "test.counter" => &TestCounterProvider,
                "spur.rust.macros" => &LegacyMacroEvidenceProvider,
                _ => &RustEvidenceProvider,
            };
            let domain = provider.source_domain(&graph).unwrap();
            replay(&ReplayCase {
                name: ext.descriptor.provider_id.clone(),
                graph: graph.clone(),
                extension: ext.clone(),
                sources: request
                    .fixtures
                    .iter()
                    .filter(|(p, _)| domain.iter().any(|s| s.file_path == *p))
                    .cloned()
                    .collect(),
                rust_checksum_set: (ext.descriptor.provider_id == "spur.rust.physical")
                    .then(|| rust.clone()),
                macro_checksum_set: (ext.descriptor.provider_id == "spur.rust.macros")
                    .then(|| macros.clone()),
                counter_registered: ext.descriptor.provider_id == "test.counter",
            })
        })
        .collect::<Vec<_>>();
    let executable = std::env::current_exe().unwrap();
    let config_path = std::env::var("G02_EVIDENCE_CONFIG").unwrap();
    let config = fs::read(&config_path).unwrap();
    let output = json!({"request_path":request_path,"request_bytes":raw,"request_sha256":sha256(&raw),"graph":graph,"original_rust":rust,"original_macros":macros,"collected":generated,"supplied":request.cases.iter().map(replay).collect::<Vec<_>>(),"executable":executable,"executable_sha256":sha256(&fs::read(&executable).unwrap()),"cargo":std::env::var("CARGO").ok(),"cargo_version":String::from_utf8(std::process::Command::new(concat!(env!("CARGO_MANIFEST_DIR"),"/../../scripts/spur-cargo")).arg("--version").output().unwrap().stdout).unwrap(),"package_version":env!("CARGO_PKG_VERSION"),"rustc":String::from_utf8(std::process::Command::new("rustc").arg("-vV").output().unwrap().stdout).unwrap(),"config_path":config_path,"config_bytes":config,"config_sha256":sha256(&config),"compiled_source_bindings":compiled_bindings()});
    println!(
        "G02_NATIVE_JSON={}",
        serde_json::to_string(&output).unwrap()
    );
}

#[test]
fn root_provider_domain_keeps_authoritative_cell_like_query_source() {
    let artifact = Arc::new(super_graph_cell_fixture());
    let legacy = InMemoryClient::new(artifact.clone());
    let expected = legacy
        .rust_evidence(&RustEvidenceFilter::default())
        .unwrap()
        .source_domain
        .unwrap()
        .into_iter()
        .map(|s| EvidenceSourceKey {
            file_path: s.file_path,
            stable_file_id: s.stable_file_id,
            source_oid: s.source_oid,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        expected.len(),
        1,
        "the existing query recognizes this actual physical file"
    );
    assert_eq!(
        RustEvidenceProvider.source_domain(&artifact).unwrap(),
        expected,
        "provider domain must preserve authoritative legacy query source coverage"
    );
}

#[test]
fn root_typed_queries_preserve_captured_legacy_results_and_absence_rules() {
    for input in [inputs(), vec![]] {
        let artifact = Arc::new(graph(&input));
        let backend = GenericClient {
            inner: InMemoryClient::new(artifact),
            root_projection: None,
        };
        let paths = std::iter::once(None).chain(
            input
                .iter()
                .filter(|(p, _)| p.ends_with(".rs"))
                .map(|(p, _)| Some(p.clone())),
        );
        for file_path in paths {
            let rust_filter = RustEvidenceFilter {
                file_path: file_path.clone(),
                source_oid: None,
            };
            assert_eq!(
                query_rust_evidence(&backend, &rust_filter).unwrap(),
                backend.inner.rust_evidence(&rust_filter).unwrap(),
                "Rust filter {rust_filter:?}"
            );
            let macro_filter = MacroInvocationFilter {
                file_path,
                caller_stable_symbol_id: None,
                source_oid: None,
            };
            assert_eq!(
                query_macro_evidence(&backend, &macro_filter).unwrap(),
                backend.inner.macro_invocations(&macro_filter).unwrap(),
                "macro filter {macro_filter:?}"
            );
        }
        // Generic absence is explicitly Unavailable. The old macro API's
        // vacuous Complete for an unrecorded .rs path is not an oracle here.
        assert_eq!(
            query_macro_evidence(
                &backend,
                &MacroInvocationFilter {
                    file_path: Some("missing.rs".into()),
                    caller_stable_symbol_id: None,
                    source_oid: None,
                }
            )
            .unwrap()
            .availability,
            MacroEvidenceAvailability::Unavailable
        );
    }
}

#[test]
fn root_query_projection_keeps_unavailable_source_without_invented_origin() {
    let mut input = inputs();
    input.push((
        "physical#cell:real.rs".into(),
        b"struct Physical;\n".to_vec(),
    ));
    let artifact = Arc::new(graph(&input));
    let expected = InMemoryClient::new(artifact.clone())
        .rust_evidence(&RustEvidenceFilter::default())
        .unwrap();
    let mut backend = GenericClient {
        inner: InMemoryClient::new(artifact.clone()),
        root_projection: None,
    };
    // Start with the genuinely validated captured extension. The query view
    // additionally retains an authoritative source whose capture is unavailable.
    let mut projection = backend
        .evidence(&EvidenceFilter {
            provider_id: "spur.rust.physical".into(),
            file_path: None,
            source_oid: None,
        })
        .unwrap();
    assert_eq!(
        projection.semantic_status,
        EvidenceSemanticStatus::Validated
    );
    let EvidenceDomain::Known { sources } = &mut projection.domain else {
        panic!("known domain");
    };
    let m = artifact
        .file_manifests
        .iter()
        .find(|m| m.path == "physical#cell:real.rs")
        .unwrap();
    assert!(expected
        .source_domain
        .as_ref()
        .unwrap()
        .iter()
        .any(|s| s.file_path == m.path && s.availability == RustEvidenceAvailability::Unavailable));
    sources.push(EvidenceSourceCoverage {
        source: EvidenceSourceKey {
            file_path: m.path.clone(),
            stable_file_id: m.stable_file_id.clone(),
            source_oid: m.content_oid.clone(),
        },
        byte_length: None,
        origin: None,
        coverage: EvidenceCoverage::Unavailable,
    });
    sources.sort_by(|a, b| a.source.file_path.cmp(&b.source.file_path));
    projection.availability = EvidenceAvailability::Partial;
    backend.root_projection = Some(projection);
    let actual = query_rust_evidence(&backend, &RustEvidenceFilter::default());
    assert!(
        actual.is_ok(),
        "valid query coverage is not a stored captured source: {actual:?}"
    );
    assert_eq!(actual.unwrap(), expected);
}
