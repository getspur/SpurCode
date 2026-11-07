use spur_graph::evidence::*;
use std::collections::BTreeMap;

// A finite, test-only replay protocol. Each supplied case calls the public
// helpers directly; typed failures are output data, not substituted successes.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayCase {
    name: String,
    graph: spur_graph::GraphIndexArtifact,
    inventory: EvidenceInventory,
    extensions: Vec<EvidenceExtension>,
    filter: EvidenceFilter,
    replacements: Vec<EvidenceExtension>,
    removals: Vec<String>,
    lookups: Vec<String>,
    duplicate_registry: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayRequest {
    cases: Vec<ReplayCase>,
}

fn replay_case(input: &ReplayCase) -> serde_json::Value {
    use serde_json::json;
    use spur_graph::GraphQueryClient;
    let counter = TestProvider { id: "test.counter" };
    let independent = FlagProvider;
    let mut providers: Vec<&dyn EvidenceProvider> = vec![&counter, &independent];
    if input.duplicate_registry {
        providers.push(&counter);
    }
    let registry = EvidenceRegistry::new(&providers);
    let extensions = input
        .extensions
        .iter()
        .map(|extension| {
            let mut canonical = EvidenceInventory::empty();
            let canonical_result = canonical.replace(extension.clone());
            let checksum_inputs = canonical_result.as_ref().ok().map(|()| {
                let e = &canonical.extensions[0];
                json!({
                    "extension_prefix":"spur-evidence-extension-v1\0",
                    "extension_compact_json":serde_json::to_string(e).unwrap(),
                    "tables":e.tables.iter().map(|table| json!({
                        "name":table.name,"prefix":"spur-evidence-table-v1\0",
                        "compact_json":serde_json::to_string(table).unwrap(),
                    })).collect::<Vec<_>>(),
                })
            });
            json!({
                "provider_id":extension.descriptor.provider_id,
                "canonicalization":canonical_result,
                "checksum_inputs":checksum_inputs,
                "checksum":evidence_extension_checksum(extension),
                "tables":extension.tables.iter().map(|t| json!({
                    "name":t.name,"checksum":evidence_table_checksum(t),
                })).collect::<Vec<_>>(),
                "registry":registry.as_ref().map_err(Clone::clone)
                    .and_then(|r|r.validate(&input.graph,extension)),
            })
        })
        .collect::<Vec<_>>();
    let manifest = evidence_inventory_manifest(&input.inventory);
    // Serialize the typed tuple itself. Serializing a JSON Value would reorder
    // object fields and would not be the actual production digest preimage.
    let capability_input = manifest.as_ref().ok().map(|m| {
        serde_json::to_string(&(
            "spur-evidence-capability-v1",
            EVIDENCE_SCHEMA_VERSION,
            &input.graph.manifest_version,
            &m.extensions,
        ))
        .unwrap()
    });
    let mut state = input.inventory.clone();
    let replacements = input
        .replacements
        .iter()
        .map(|e| {
            let result = state.replace(e.clone());
            json!({"result":result,"state_after":state})
        })
        .collect::<Vec<_>>();
    let lookups = input
        .lookups
        .iter()
        .map(|id| json!({"provider_id":id,"result":state.get(id)}))
        .collect::<Vec<_>>();
    let removals = input
        .removals
        .iter()
        .map(|id| {
            let removed = state.remove(id);
            json!({"provider_id":id,"result":removed,"state_after":state})
        })
        .collect::<Vec<_>>();
    let client = spur_graph::InMemoryClient::new(std::sync::Arc::new(input.graph.clone()));
    let query: Result<EvidenceQueryResult, EvidenceError> =
        client.evidence(&input.filter).map_err(|e| {
            e.downcast::<EvidenceError>()
                .expect("default must preserve typed cause")
        });
    json!({
        "name":input.name,"integrity":input.inventory.validate_integrity(&input.graph),
        "manifest":manifest,"capability_compact_json":capability_input,
        "capability_digest":evidence_inventory_capability_digest(&input.graph.manifest_version,&input.inventory),
        "extensions":extensions,"registry_construction":registry.as_ref().map(|_|()).map_err(Clone::clone),
        "filter_shape":input.filter.validate_shape(),"default_query":query,
        "operations":{"empty":EvidenceInventory::empty(),"empty_is_empty":EvidenceInventory::empty().is_empty(),
            "initial_is_empty":input.inventory.is_empty(),"replacements":replacements,
            "lookups":lookups,"removals":removals,"final":state,"final_is_empty":state.is_empty()},
    })
}

fn default_replay_request() -> ReplayRequest {
    let (g, bound) = source_fixture();
    let positive = ReplayCase {
        name: "positive".into(),
        graph: g,
        inventory: inventory(vec![bound.clone(), second_extension()]),
        extensions: vec![bound.clone(), second_extension()],
        filter: EvidenceFilter {
            provider_id: "test.counter".into(),
            file_path: None,
            source_oid: None,
        },
        replacements: vec![counter_extension(2)],
        removals: vec![
            "test.counter".into(),
            "test.independent".into(),
            "missing".into(),
        ],
        lookups: vec!["test.counter".into(), "missing".into()],
        duplicate_registry: false,
    };
    let mut cases = vec![positive.clone()];
    for (name, mutate) in [
        (
            "payload",
            (|e: &mut EvidenceExtension| e.tables[0].rows[0][1] = EvidenceValue::Int64(8))
                as fn(&mut EvidenceExtension),
        ),
        ("provider", |e| {
            e.descriptor.provider_id = "opaque.fixture".into()
        }),
        ("schema", |e| {
            e.descriptor.schema_version = "future-v2".into()
        }),
        ("extractor", |e| {
            e.descriptor.extractor_version = "fixture-v2".into()
        }),
        ("component", |e| {
            e.descriptor
                .component_versions
                .insert("parser".into(), "v2".into());
        }),
        ("metadata", |e| {
            e.descriptor
                .metadata
                .insert("note".into(), "changed".into());
        }),
        ("tableMetadata", |e| {
            e.tables[0].metadata.insert("unit".into(), "items".into());
        }),
        ("tableName", |e| e.tables[0].name = "other".into()),
        ("tablePath", |e| {
            e.tables[0].storage_path = "evidence/test.counter/other.parquet".into()
        }),
        ("tableSchema", |e| e.tables[0].columns[1].nullable = true),
        ("tableOrder", |e| {
            e.tables[0].order_by = vec!["value".into()]
        }),
        ("tableKey", |e| e.tables[0].primary_key.push("value".into())),
        ("scope", |e| {
            e.tables[0].scope = EvidenceTableScope::Snapshot
        }),
        ("domainUnknown", |e| e.domain = EvidenceDomain::Unknown),
        ("coverageUnknown", |e| {
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources[0].coverage = EvidenceCoverage::Unknown {
                    code: "fixture_unknown".into(),
                };
            }
        }),
        ("coverageUnavailable", |e| {
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources[0].coverage = EvidenceCoverage::Unavailable;
            }
        }),
        ("length", |e| {
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources[0].byte_length = Some(0);
            }
        }),
        ("originVersion", |e| {
            e.origins[0].key.manifest_version = "historical".into();
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources[0].origin = Some(e.origins[0].key.clone());
            }
        }),
        ("duplicateKey", |e| {
            let duplicate = e.tables[0].rows[0].clone();
            e.tables[0].rows.push(duplicate);
        }),
        ("wrongType", |e| {
            e.tables[0].rows[0][1] = EvidenceValue::Boolean(true)
        }),
        ("nonnullable", |e| {
            e.tables[0].rows[0][1] = EvidenceValue::Null
        }),
        ("duplicateTable", |e| e.tables.push(e.tables[0].clone())),
        ("forgedOrigin", |e| {
            e.origins[0].key.graph_content_hash = "forged".into()
        }),
        ("wrongOid", |e| {
            e.tables[0].rows[0][3] = EvidenceValue::Utf8("wrong".into())
        }),
        ("wrongPath", |e| {
            e.tables[0].rows[0][2] = EvidenceValue::Utf8("wrong.txt".into())
        }),
        ("semanticError", |e| {
            e.tables[0].rows[0][1] = EvidenceValue::Int64(-1)
        }),
    ] {
        let mut case = positive.clone();
        case.name = name.into();
        mutate(&mut case.inventory.extensions[0]);
        case.extensions = case.inventory.extensions.clone();
        cases.push(case);
    }
    for path in [
        "../escape",
        "manifest.json",
        "nodes.parquet",
        "edges.parquet",
        "files.parquet",
        "tombstones.parquet",
        "diagnostics.parquet",
        "history/part.parquet",
    ] {
        let mut case = positive.clone();
        case.name = format!("reservedOrUnsafe:{path}");
        case.inventory.extensions[0].tables[0].storage_path = path.into();
        case.extensions = case.inventory.extensions.clone();
        cases.push(case);
    }
    for name in [
        "providerOrder",
        "providerRemoved",
        "duplicateInventory",
        "duplicateRegistry",
        "duplicatePath",
        "missingOwner",
        "duplicateOwner",
        "wrongOwner",
        "staleGraphOid",
        "sourceStableId",
        "sourceOid",
        "sourcePath",
        "manifestVersion",
        "emptyProviderFilter",
        "oidWithoutPath",
    ] {
        let mut case = positive.clone();
        case.name = name.into();
        match name {
            "providerOrder" => case.inventory.extensions.reverse(),
            "providerRemoved" => {
                case.inventory.extensions.pop();
            }
            "duplicateInventory" => case.inventory.extensions.push(bound.clone()),
            "duplicateRegistry" => case.duplicate_registry = true,
            "duplicatePath" => {
                case.inventory.extensions[1].tables[0].storage_path =
                    bound.tables[0].storage_path.clone()
            }
            "missingOwner" => case.graph.files.clear(),
            "duplicateOwner" => case.graph.files.push(case.graph.files[0].clone()),
            "wrongOwner" => case.graph.files[0].stable_file_id = "wrong".into(),
            "staleGraphOid" => case.graph.file_manifests[0].content_oid = "wrong".into(),
            "manifestVersion" => case.graph.manifest_version = "other-manifest".into(),
            "emptyProviderFilter" => case.filter.provider_id = "".into(),
            "oidWithoutPath" => case.filter.source_oid = Some("oid".into()),
            _ => {
                let source = &mut case.graph.file_manifests[0];
                match name {
                    "sourceStableId" => source.stable_file_id = "new-id".into(),
                    "sourceOid" => source.content_oid = "new-oid".into(),
                    _ => source.path = "new.txt".into(),
                }
                let source = EvidenceSourceKey {
                    file_path: source.path.clone(),
                    stable_file_id: source.stable_file_id.clone(),
                    source_oid: source.content_oid.clone(),
                };
                case.graph.files[0].file_path = source.file_path.clone();
                case.graph.files[0].stable_file_id = source.stable_file_id.clone();
                case.graph.graph_content_hash = spur_graph::compute_graph_content_hash([(
                    &source.file_path,
                    &source.source_oid,
                )]);
                let e = &mut case.inventory.extensions[0];
                e.origins[0].sources = vec![source.clone()];
                e.origins[0].key.graph_content_hash = case.graph.graph_content_hash.clone();
                if let EvidenceDomain::Known { sources } = &mut e.domain {
                    sources[0].source = source.clone();
                    sources[0].origin = Some(e.origins[0].key.clone());
                }
                e.tables[0].rows[0][2] = EvidenceValue::Utf8(source.file_path);
                e.tables[0].rows[0][3] = EvidenceValue::Utf8(source.source_oid);
                e.tables[0].rows[0][4] = EvidenceValue::Utf8(case.graph.graph_content_hash.clone());
            }
        }
        case.extensions = case.inventory.extensions.clone();
        cases.push(case);
    }
    for name in ["beforeCapturedZero", "afterAbsent", "capturedUnknown"] {
        let mut case = positive.clone();
        case.name = name.into();
        case.graph = graph();
        let mut extension = counter_extension(0);
        extension.tables[0].rows.clear();
        if name == "afterAbsent" {
            extension.descriptor.presence = EvidencePresence::Absent;
            extension.tables.clear();
            extension.domain = EvidenceDomain::Unknown;
        }
        if name == "capturedUnknown" {
            extension.domain = EvidenceDomain::Unknown;
        }
        case.inventory = inventory(vec![extension.clone()]);
        case.extensions = vec![extension];
        cases.push(case);
    }
    ReplayRequest { cases }
}

#[test]
fn replay_serialized_contract_request() {
    let supplied = std::env::var_os("G01_EVIDENCE_REQUEST");
    let bytes = if let Some(path) = &supplied {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::read(root.join(path)).expect("read supplied replay request")
    } else {
        serde_json::to_vec(&default_replay_request()).unwrap()
    };
    let request: ReplayRequest =
        serde_json::from_slice(&bytes).expect("deserialize complete typed replay request");
    assert!(
        !request.cases.is_empty(),
        "request needs at least one real case"
    );
    let results = request.cases.iter().map(replay_case).collect::<Vec<_>>();
    if supplied.is_none() {
        let case = |name: &str| results.iter().find(|r| r["name"] == name).unwrap();
        assert_eq!(
            case("positive")["integrity"],
            serde_json::json!({"Ok":null})
        );
        assert_eq!(
            case("positive")["extensions"][0]["registry"]["Ok"]["status"],
            "validated"
        );
        assert_eq!(
            case("positive")["default_query"]["Ok"]["availability"],
            "unavailable"
        );
        assert_eq!(case("positive")["operations"]["final_is_empty"], true);
        assert_eq!(
            case("positive")["capability_digest"],
            case("providerOrder")["capability_digest"]
        );
        for name in [
            "payload",
            "provider",
            "schema",
            "extractor",
            "component",
            "metadata",
            "tableMetadata",
            "tableName",
            "tablePath",
            "tableSchema",
            "tableOrder",
            "tableKey",
            "scope",
            "domainUnknown",
            "coverageUnknown",
            "coverageUnavailable",
            "length",
            "originVersion",
            "providerRemoved",
            "sourceStableId",
            "sourceOid",
            "sourcePath",
            "manifestVersion",
        ] {
            assert!(case(name)["integrity"].get("Ok").is_some(), "{name}");
            assert_ne!(
                case("positive")["capability_digest"],
                case(name)["capability_digest"],
                "{name}"
            );
        }
        for name in [
            "duplicateKey",
            "wrongType",
            "nonnullable",
            "duplicateTable",
            "forgedOrigin",
            "wrongOid",
            "wrongPath",
            "duplicateInventory",
            "duplicatePath",
            "missingOwner",
            "duplicateOwner",
            "wrongOwner",
            "staleGraphOid",
        ] {
            assert!(
                case(name)["integrity"]["Err"].get("integrity").is_some(),
                "{name}"
            );
        }
        assert!(case("duplicateRegistry")["registry_construction"]
            .get("Err")
            .is_some());
        assert_eq!(
            case("provider")["extensions"][0]["registry"]["Ok"]["status"],
            "unknown_provider"
        );
        assert_eq!(
            case("schema")["extensions"][0]["registry"]["Ok"]["status"],
            "unsupported_schema"
        );
        assert!(case("semanticError")["extensions"][0]["registry"]["Err"]
            .get("semantic")
            .is_some());
        for name in ["emptyProviderFilter", "oidWithoutPath"] {
            assert!(case(name)["default_query"].get("Err").is_some());
        }
        assert_ne!(
            case("beforeCapturedZero")["capability_digest"],
            case("afterAbsent")["capability_digest"]
        );
        assert_ne!(
            case("beforeCapturedZero")["capability_digest"],
            case("capturedUnknown")["capability_digest"]
        );
    }
    println!("G01_REPLAY_EXPORT={}",serde_json::to_string(&serde_json::json!({
        "fixture_kind":"synthetic contract requests; no collector/artifact/disk/cache observations",
        "supplied_path":supplied,"request":request,"results":results,
    })).unwrap());
}

fn counter_extension(value: i64) -> EvidenceExtension {
    EvidenceExtension {
        descriptor: EvidenceDescriptor {
            provider_id: "test.counter".into(),
            schema_version: "counter-v1".into(),
            extractor_version: "fixture-v1".into(),
            component_versions: BTreeMap::new(),
            presence: EvidencePresence::Captured,
            metadata: BTreeMap::new(),
        },
        origins: vec![],
        domain: EvidenceDomain::Known { sources: vec![] },
        tables: vec![EvidenceTable {
            name: "values".into(),
            storage_path: "evidence/test.counter/values.parquet".into(),
            scope: EvidenceTableScope::Snapshot,
            columns: vec![
                EvidenceColumn {
                    name: "key".into(),
                    data_type: EvidenceColumnType::Utf8,
                    nullable: false,
                },
                EvidenceColumn {
                    name: "value".into(),
                    data_type: EvidenceColumnType::Int64,
                    nullable: false,
                },
            ],
            primary_key: vec!["key".into()],
            order_by: vec!["key".into()],
            metadata: BTreeMap::new(),
            rows: vec![vec![
                EvidenceValue::Utf8("fixture".into()),
                EvidenceValue::Int64(value),
            ]],
        }],
    }
}

#[test]
fn counter_only_change_changes_inventory_capability() {
    let mut a = EvidenceInventory::empty();
    a.replace(counter_extension(1)).unwrap();
    let mut b = a.clone();
    b.replace(counter_extension(2)).unwrap();
    assert_ne!(
        evidence_inventory_capability_digest("fixture-manifest", &a).unwrap(),
        evidence_inventory_capability_digest("fixture-manifest", &b).unwrap()
    );
}

fn inventory(extensions: Vec<EvidenceExtension>) -> EvidenceInventory {
    EvidenceInventory {
        schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        extensions,
    }
}
fn digest(extensions: Vec<EvidenceExtension>) -> String {
    evidence_inventory_capability_digest("fixture-manifest", &inventory(extensions)).unwrap()
}
fn second_extension() -> EvidenceExtension {
    let mut e = counter_extension(9);
    e.descriptor.provider_id = "test.independent".into();
    e.descriptor.schema_version = "flags-v1".into();
    e.tables[0].columns[1].data_type = EvidenceColumnType::Boolean;
    e.tables[0].rows[0][1] = EvidenceValue::Boolean(true);
    e.tables[0].storage_path = "evidence/test.independent/values.parquet".into();
    e
}
fn graph() -> spur_graph::GraphIndexArtifact {
    // A construction fixture, not a collector or a generic artifact-field claim.
    spur_graph::GraphIndexArtifact {
        header: spur_graph::GraphIndexHeader {
            graph_index_version: spur_graph::GRAPH_INDEX_VERSION_TEMPORAL.into(),
            content_hash_blake3: None,
        },
        manifest_version: "fixture-manifest".into(),
        graph_content_hash: spur_graph::compute_graph_content_hash(Vec::<(String, String)>::new()),
        file_manifests: vec![],
        files: vec![],
        file_node_ids: vec![],
        symbols: vec![],
        symbol_node_ids: vec![],
        edges: vec![],
        macro_invocations: None,
        rust_evidence: None,
        tombstones: vec![],
        diagnostics: vec![],
        commits: vec![],
        symbol_snapshots: vec![],
        temporal_edges: vec![],
    }
}
fn source_fixture() -> (spur_graph::GraphIndexArtifact, EvidenceExtension) {
    let mut graph = graph();
    let bytes = b"independent physical fixture\n";
    let source = EvidenceSourceKey {
        file_path: "fixture/input.txt".into(),
        stable_file_id: "physical-fixture".into(),
        source_oid: spur_graph::git_blob_oid(bytes),
    };
    graph
        .file_manifests
        .push(spur_graph::GraphFileManifestEntry {
            stable_file_id: source.stable_file_id.clone(),
            path: source.file_path.clone(),
            content_oid: source.source_oid.clone(),
            node_ids: vec![],
        });
    graph.files.push(spur_graph::GraphFileArtifact {
        stable_file_id: source.stable_file_id.clone(),
        file_path: source.file_path.clone(),
    });
    graph.graph_content_hash =
        spur_graph::compute_graph_content_hash([(&source.file_path, &source.source_oid)]);
    let origin = EvidenceOriginKey {
        manifest_version: graph.manifest_version.clone(),
        graph_content_hash: graph.graph_content_hash.clone(),
    };
    let mut e = counter_extension(7);
    e.origins.push(EvidenceOrigin {
        key: origin.clone(),
        sources: vec![source.clone()],
    });
    e.domain = EvidenceDomain::Known {
        sources: vec![EvidenceSourceCoverage {
            source: source.clone(),
            byte_length: Some(u64::try_from(bytes.len()).unwrap()),
            origin: Some(origin.clone()),
            coverage: EvidenceCoverage::Complete,
        }],
    };
    let table = &mut e.tables[0];
    table.scope = EvidenceTableScope::File {
        file_path_column: "path".into(),
        source_oid_column: "oid".into(),
        origin_hash_column: Some("origin".into()),
    };
    for name in ["path", "oid", "origin"] {
        table.columns.push(EvidenceColumn {
            name: name.into(),
            data_type: EvidenceColumnType::Utf8,
            nullable: false,
        });
    }
    table.rows[0].extend([
        EvidenceValue::Utf8(source.file_path),
        EvidenceValue::Utf8(source.source_oid),
        EvidenceValue::Utf8(origin.graph_content_hash),
    ]);
    (graph, e)
}
fn assert_integrity_error(error: EvidenceError) {
    assert!(matches!(error, EvidenceError::Integrity { .. }), "{error}");
    let cause: anyhow::Error = error.into();
    assert!(cause.downcast_ref::<EvidenceError>().is_some());
}

#[test]
fn identity_tracks_every_provider_descriptor_and_payload_dependency() {
    let a = counter_extension(1);
    let b = second_extension();
    let original = digest(vec![a.clone(), b.clone()]);
    assert_eq!(original, digest(vec![b.clone(), a.clone()]));
    assert_ne!(original, digest(vec![a.clone()]));
    assert_ne!(original, digest(vec![b.clone()]));
    assert_ne!(
        original,
        evidence_inventory_capability_digest(
            "different-manifest",
            &inventory(vec![a.clone(), b.clone()])
        )
        .unwrap()
    );
    let mutations: Vec<fn(&mut EvidenceExtension)> = vec![
        |e| e.descriptor.provider_id = "test.renamed".into(),
        |e| e.descriptor.schema_version = "counter-v2".into(),
        |e| e.descriptor.extractor_version = "fixture-v2".into(),
        |e| {
            e.descriptor
                .component_versions
                .insert("component".into(), "v2".into());
        },
        |e| {
            e.descriptor.metadata.insert("note".into(), "new".into());
        },
        |e| e.tables[0].name = "renamed".into(),
        |e| e.tables[0].storage_path = "evidence/test.counter/other.parquet".into(),
        |e| e.tables[0].columns[1].nullable = true,
        |e| e.tables[0].primary_key = vec!["key".into(), "value".into()],
        |e| e.tables[0].order_by = vec!["value".into()],
        |e| {
            e.tables[0].metadata.insert("unit".into(), "items".into());
        },
        |e| e.tables[0].rows[0][1] = EvidenceValue::Int64(2),
        |e| {
            e.tables[0].rows.push(vec![
                EvidenceValue::Utf8("second".into()),
                EvidenceValue::Int64(1),
            ])
        },
        |e| e.domain = EvidenceDomain::Unknown,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = a.clone();
        mutate(&mut changed);
        assert_ne!(
            original,
            digest(vec![changed, b.clone()]),
            "dependency {index}"
        );
    }
}

#[test]
fn captured_zero_absent_and_unknown_are_distinct() {
    let mut zero = counter_extension(0);
    zero.tables[0].rows.clear();
    let mut absent = zero.clone();
    absent.descriptor.presence = EvidencePresence::Absent;
    absent.tables.clear();
    absent.domain = EvidenceDomain::Unknown;
    let mut unknown = zero.clone();
    unknown.domain = EvidenceDomain::Unknown;
    let z = evidence_inventory_manifest(&inventory(vec![zero.clone()])).unwrap();
    assert_eq!(z.extensions[0].tables[0].row_count, 0);
    assert!(z.extensions[0].tables[0].table.rows.is_empty());
    assert!(z.extensions[0].checksum_blake3.is_some());
    assert_eq!(
        z.extensions[0].domain,
        EvidenceDomain::Known { sources: vec![] }
    );
    assert_eq!(
        evidence_inventory_manifest(&inventory(vec![absent.clone()]))
            .unwrap()
            .extensions[0]
            .checksum_blake3,
        None
    );
    assert_ne!(digest(vec![zero.clone()]), digest(vec![absent.clone()]));
    assert_ne!(digest(vec![zero]), digest(vec![unknown.clone()]));
    assert_ne!(digest(vec![absent.clone()]), digest(vec![unknown]));
    assert_ne!(digest(vec![]), digest(vec![absent.clone()]));
    absent.tables = counter_extension(0).tables;
    assert!(evidence_extension_checksum(&absent).is_err());
}

#[test]
fn canonical_order_preserves_schema_and_uses_rows_as_total_tiebreak() {
    let mut a = counter_extension(1);
    a.tables[0].order_by.clear();
    a.tables[0].rows.push(vec![
        EvidenceValue::Utf8("aaa".into()),
        EvidenceValue::Int64(2),
    ]);
    let mut reversed = a.clone();
    reversed.tables[0].rows.reverse();
    let mut stored = EvidenceInventory::empty();
    stored.replace(reversed).unwrap();
    assert_eq!(
        stored.extensions[0].tables[0].rows[0][0],
        EvidenceValue::Utf8("aaa".into())
    );
    assert_eq!(digest(vec![a.clone()]), digest(stored.extensions));
    let mut reordered_schema = a.clone();
    reordered_schema.tables[0].columns.reverse();
    for row in &mut reordered_schema.tables[0].rows {
        row.reverse();
    }
    assert_ne!(digest(vec![a]), digest(vec![reordered_schema]));
}

#[test]
fn replace_is_atomic_and_does_not_hide_duplicates() {
    let e = counter_extension(1);
    let mut inv = inventory(vec![e.clone()]);
    let saved = inv.clone();
    let mut bad = e.clone();
    let duplicate = bad.tables[0].rows[0].clone();
    bad.tables[0].rows.push(duplicate);
    assert_integrity_error(inv.replace(bad).unwrap_err());
    assert_eq!(inv, saved);
    let mut duplicate = inventory(vec![e.clone(), e.clone()]);
    assert!(duplicate.validate_integrity(&graph()).is_err());
    assert!(duplicate.replace(e).is_err());
    assert_eq!(inv.remove("test.counter"), Some(counter_extension(1)));
    assert!(inv.is_empty());
    assert_eq!(inv.remove("missing"), None);
    assert!(inv.get("missing").is_none());
}

#[test]
fn malformed_generic_tables_are_rejected_before_hashing() {
    let mutations: Vec<fn(&mut EvidenceExtension)> = vec![
        |e| {
            let duplicate = e.tables[0].rows[0].clone();
            e.tables[0].rows.push(duplicate);
        },
        |e| {
            e.tables[0].rows[0].pop();
        },
        |e| e.tables[0].rows[0][1] = EvidenceValue::Utf8("7".into()),
        |e| e.tables[0].rows[0][1] = EvidenceValue::Boolean(true),
        |e| e.tables[0].rows[0][1] = EvidenceValue::Null,
        |e| e.tables[0].primary_key.clear(),
        |e| e.tables[0].primary_key = vec!["missing".into()],
        |e| e.tables[0].primary_key = vec!["key".into(), "key".into()],
        |e| e.tables[0].order_by = vec!["missing".into()],
        |e| e.tables[0].order_by = vec!["key".into(), "key".into()],
        |e| e.tables[0].columns[0].nullable = true,
        |e| e.tables[0].columns[1].name = "key".into(),
        |e| {
            e.tables.push(e.tables[0].clone());
        },
    ];
    for (i, mutate) in mutations.iter().enumerate() {
        let mut e = counter_extension(1);
        mutate(&mut e);
        assert!(evidence_extension_checksum(&e).is_err(), "malformed {i}");
        assert!(
            evidence_inventory_manifest(&inventory(vec![e])).is_err(),
            "manifest {i}"
        );
    }
    let mut nullable = counter_extension(1);
    nullable.tables[0].columns[1].nullable = true;
    nullable.tables[0].rows[0][1] = EvidenceValue::Null;
    assert!(evidence_extension_checksum(&nullable).is_ok());
}

#[test]
fn lexical_paths_cannot_escape_or_overwrite_core_transport() {
    for path in [
        "",
        "/absolute",
        "../escape",
        "a/../b",
        "a/./b",
        "a//b",
        "a/",
        "C:/a",
        "a\\b",
        "a\0b",
        "manifest.json",
        "nodes.parquet",
        "edges.parquet",
        "edges_by_dst.parquet",
        "edges_unresolved.parquet",
        "files.parquet",
        "file_manifests.parquet",
        "tombstones.parquet",
        "diagnostics.parquet",
        "commits.parquet",
        "symbol_snapshots/00000.parquet",
        "temporal_edges/00000.parquet",
        "history/data.parquet",
        "symbol_snapshots.parquet",
        "temporal_edges.parquet",
        "nodes.parquet/child",
    ] {
        let mut e = counter_extension(1);
        e.tables[0].storage_path = path.into();
        assert!(evidence_extension_checksum(&e).is_err(), "{path:?}");
    }
    let a = counter_extension(1);
    let mut b = second_extension();
    b.tables[0].storage_path = a.tables[0].storage_path.clone();
    assert!(evidence_inventory_manifest(&inventory(vec![a.clone(), b.clone()])).is_err());
    b.tables[0].storage_path.push_str("/child");
    assert!(evidence_inventory_manifest(&inventory(vec![a, b])).is_err());
    for path in [
        "rust_declarations.parquet",
        "macro_invocations.parquet",
        "evidence/custom/nodes.parquet",
    ] {
        let mut e = counter_extension(1);
        e.tables[0].storage_path = path.into();
        assert!(evidence_extension_checksum(&e).is_ok());
    }
}

#[test]
fn source_origin_and_current_owner_integrity_are_independent_of_language() {
    let (g, e) = source_fixture();
    inventory(vec![e.clone()]).validate_integrity(&g).unwrap();
    let mut wrong_oid = g.clone();
    wrong_oid.file_manifests[0].content_oid = spur_graph::git_blob_oid(b"different");
    wrong_oid.graph_content_hash = spur_graph::compute_graph_content_hash(
        wrong_oid
            .file_manifests
            .iter()
            .map(|f| (&f.path, &f.content_oid)),
    );
    assert!(inventory(vec![e.clone()])
        .validate_integrity(&wrong_oid)
        .is_err());
    let mut wrong_owner = g.clone();
    wrong_owner.files[0].stable_file_id = "forged".into();
    assert!(inventory(vec![e.clone()])
        .validate_integrity(&wrong_owner)
        .is_err());
    let mut wrong_hash = g.clone();
    wrong_hash.graph_content_hash = "forged".into();
    assert!(inventory(vec![e.clone()])
        .validate_integrity(&wrong_hash)
        .is_err());
    let mutations: Vec<fn(&mut EvidenceExtension)> = vec![
        |e| e.origins[0].key.graph_content_hash = "forged".into(),
        |e| {
            let duplicate = e.origins[0].sources[0].clone();
            e.origins[0].sources.push(duplicate);
        },
        |e| e.origins.push(e.origins[0].clone()),
        |e| e.tables[0].rows[0][2] = EvidenceValue::Utf8("missing.txt".into()),
        |e| e.tables[0].rows[0][3] = EvidenceValue::Utf8("wrong_oid".into()),
        |e| e.tables[0].rows[0][4] = EvidenceValue::Utf8("wrong_origin".into()),
        |e| {
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources[0].origin = None;
            }
        },
        |e| {
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources[0].byte_length = None;
            }
        },
        |e| {
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources[0].source.stable_file_id = "forged".into();
            }
        },
        |e| {
            if let EvidenceDomain::Known { sources } = &mut e.domain {
                sources.clear();
            }
        },
    ];
    for (i, mutate) in mutations.iter().enumerate() {
        let mut bad = e.clone();
        mutate(&mut bad);
        assert!(
            inventory(vec![bad]).validate_integrity(&g).is_err(),
            "source mutation {i}"
        );
    }
    let mut unknown = e.clone();
    if let EvidenceDomain::Known { sources } = &mut unknown.domain {
        sources[0].coverage = EvidenceCoverage::Unknown {
            code: "fixture_opaque".into(),
        };
    }
    inventory(vec![unknown.clone()])
        .validate_integrity(&g)
        .unwrap();
    assert_ne!(digest(vec![e]), digest(vec![unknown]));
}

struct TestProvider {
    id: &'static str,
}
impl EvidenceProvider for TestProvider {
    fn provider_id(&self) -> &'static str {
        self.id
    }
    fn supports_path(&self, path: &str) -> bool {
        !path.is_empty()
    }
    fn source_domain(
        &self,
        g: &spur_graph::GraphIndexArtifact,
    ) -> Result<Vec<EvidenceSourceKey>, EvidenceError> {
        Ok(g.file_manifests
            .iter()
            .map(|f| EvidenceSourceKey {
                file_path: f.path.clone(),
                stable_file_id: f.stable_file_id.clone(),
                source_oid: f.content_oid.clone(),
            })
            .collect())
    }
    fn validate_stored(
        &self,
        _g: &spur_graph::GraphIndexArtifact,
        e: &EvidenceExtension,
    ) -> Result<EvidenceValidation, EvidenceError> {
        if e.descriptor.schema_version != "counter-v1" {
            return Err(EvidenceError::Unsupported {
                provider_id: self.id.into(),
                schema_version: e.descriptor.schema_version.clone(),
            });
        }
        if e.tables
            .iter()
            .flat_map(|t| &t.rows)
            .any(|r| matches!(r.get(1),Some(EvidenceValue::Int64(v)) if *v<0))
        {
            return Err(EvidenceError::Semantic {
                provider_id: self.id.into(),
                code: "negative_counter".into(),
                message: "fixture counter must be nonnegative".into(),
            });
        }
        Ok(EvidenceValidation {
            status: EvidenceSemanticStatus::Validated,
            domain: e.domain.clone(),
        })
    }
    fn validate_sources(
        &self,
        _g: &spur_graph::GraphIndexArtifact,
        e: &EvidenceExtension,
        sources: &[EvidenceSource<'_>],
    ) -> Result<(), EvidenceError> {
        for original in e.origins.iter().flat_map(|o| &o.sources) {
            if !sources.iter().any(|s| {
                s.file_path == original.file_path
                    && spur_graph::git_blob_oid(s.bytes) == original.source_oid
            }) {
                return Err(EvidenceError::Semantic {
                    provider_id: self.id.into(),
                    code: "source_bytes".into(),
                    message: "saved bytes differ".into(),
                });
            }
        }
        Ok(())
    }
}

#[test]
fn registry_checks_transport_before_dispatch_and_keeps_opaque_status_explicit() {
    let g = graph();
    let provider = TestProvider { id: "test.counter" };
    let other = FlagProvider;
    let providers: [&dyn EvidenceProvider; 2] = [&provider, &other];
    let registry = EvidenceRegistry::new(&providers).unwrap();
    let duplicates: [&dyn EvidenceProvider; 2] = [&provider, &provider];
    assert!(EvidenceRegistry::new(&duplicates).is_err());
    assert_eq!(
        registry.validate(&g, &counter_extension(7)).unwrap().status,
        EvidenceSemanticStatus::Validated
    );
    assert_eq!(
        registry.validate(&g, &second_extension()).unwrap().status,
        EvidenceSemanticStatus::Validated
    );
    let mut opaque = counter_extension(7);
    opaque.descriptor.provider_id = "unregistered.opaque".into();
    let result = registry.validate(&g, &opaque).unwrap();
    assert_eq!(result.status, EvidenceSemanticStatus::UnknownProvider);
    assert_eq!(result.domain, EvidenceDomain::Unknown);
    let mut unsupported = counter_extension(7);
    unsupported.descriptor.schema_version = "future-v2".into();
    let result = registry.validate(&g, &unsupported).unwrap();
    assert_eq!(result.status, EvidenceSemanticStatus::UnsupportedSchema);
    assert_eq!(result.domain, EvidenceDomain::Unknown);
    unsupported.tables[0].rows[0][1] = EvidenceValue::Null;
    assert_integrity_error(registry.validate(&g, &unsupported).unwrap_err());
    opaque.tables[0].rows[0][1] = EvidenceValue::Null;
    assert_integrity_error(registry.validate(&g, &opaque).unwrap_err());
    let error = registry.validate(&g, &counter_extension(-1)).unwrap_err();
    assert!(matches!(error, EvidenceError::Semantic { .. }));
    let cause: anyhow::Error = error.into();
    assert!(cause.downcast_ref::<EvidenceError>().is_some());
}

#[test]
fn optional_query_default_validates_shape_before_unavailable() {
    use spur_graph::GraphQueryClient;
    let client = spur_graph::InMemoryClient::new(std::sync::Arc::new(graph()));
    let valid = EvidenceFilter {
        provider_id: "test.counter".into(),
        file_path: None,
        source_oid: None,
    };
    let result = client.evidence(&valid).unwrap();
    assert_eq!(result.availability, EvidenceAvailability::Unavailable);
    assert_eq!(
        result.semantic_status,
        EvidenceSemanticStatus::UnknownProvider
    );
    assert_eq!(result.domain, EvidenceDomain::Unknown);
    assert!(result.descriptor.is_none() && result.tables.is_empty() && result.origins.is_empty());
    for bad in [
        EvidenceFilter {
            provider_id: "".into(),
            ..valid.clone()
        },
        EvidenceFilter {
            source_oid: Some("oid".into()),
            ..valid.clone()
        },
    ] {
        assert!(client
            .evidence(&bad)
            .unwrap_err()
            .downcast_ref::<EvidenceError>()
            .is_some());
    }
}

#[test]
fn wire_rejects_unknown_fields_and_wrong_integer_or_enum_types() {
    let e = counter_extension(1);
    let mut json = serde_json::to_value(&e).unwrap();
    json["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvidenceExtension>(json).is_err());
    let mut json = serde_json::to_value(&e).unwrap();
    json["descriptor"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvidenceExtension>(json).is_err());
    assert!(serde_json::from_str::<EvidenceValue>("{\"int64\":9223372036854775808}").is_err());
    assert!(serde_json::from_str::<EvidenceColumnType>("\"float64\"").is_err());
    assert_eq!(
        serde_json::from_value::<EvidenceExtension>(serde_json::to_value(&e).unwrap()).unwrap(),
        e
    );
}

#[test]
fn overlapping_tables_within_one_extension_are_rejected() {
    let mut extension = counter_extension(1);
    let mut child = extension.tables[0].clone();
    child.name = "child".into();
    child.storage_path.push_str("/child.parquet");
    extension.tables.push(child);
    assert!(evidence_extension_checksum(&extension).is_err());
}

struct FlagProvider;
impl EvidenceProvider for FlagProvider {
    fn provider_id(&self) -> &'static str {
        "test.independent"
    }
    fn supports_path(&self, _path: &str) -> bool {
        false
    }
    fn source_domain(
        &self,
        _g: &spur_graph::GraphIndexArtifact,
    ) -> Result<Vec<EvidenceSourceKey>, EvidenceError> {
        Ok(vec![])
    }
    fn validate_stored(
        &self,
        _g: &spur_graph::GraphIndexArtifact,
        e: &EvidenceExtension,
    ) -> Result<EvidenceValidation, EvidenceError> {
        if e.descriptor.schema_version != "flags-v1" {
            return Err(EvidenceError::Unsupported {
                provider_id: self.provider_id().into(),
                schema_version: e.descriptor.schema_version.clone(),
            });
        }
        if e.tables.len() != 1
            || e.tables[0].columns.len() != 2
            || e.tables[0].columns[1].data_type != EvidenceColumnType::Boolean
        {
            return Err(EvidenceError::Semantic {
                provider_id: self.provider_id().into(),
                code: "flag_schema".into(),
                message: "expected Boolean flag".into(),
            });
        }
        Ok(EvidenceValidation {
            status: EvidenceSemanticStatus::Validated,
            domain: e.domain.clone(),
        })
    }
    fn validate_sources(
        &self,
        _g: &spur_graph::GraphIndexArtifact,
        _e: &EvidenceExtension,
        _sources: &[EvidenceSource<'_>],
    ) -> Result<(), EvidenceError> {
        Ok(())
    }
}

#[test]
fn provenance_versions_lengths_scope_and_coverage_participate_in_identity() {
    let (g, extension) = source_fixture();
    let baseline = digest(vec![extension.clone()]);
    let mut length = extension.clone();
    if let EvidenceDomain::Known { sources } = &mut length.domain {
        sources[0].byte_length = Some(0);
    }
    assert_ne!(baseline, digest(vec![length]));
    let mut version = extension.clone();
    version.origins[0].key.manifest_version = "historical-version".into();
    if let EvidenceDomain::Known { sources } = &mut version.domain {
        sources[0].origin = Some(version.origins[0].key.clone());
    }
    assert_ne!(baseline, digest(vec![version.clone()]));
    inventory(vec![version]).validate_integrity(&g).unwrap();
    let mut snapshot = extension.clone();
    snapshot.tables[0].scope = EvidenceTableScope::Snapshot;
    assert_ne!(baseline, digest(vec![snapshot]));
    let mut unknown = extension.clone();
    unknown.domain = EvidenceDomain::Unknown;
    inventory(vec![unknown.clone()])
        .validate_integrity(&g)
        .unwrap();
    assert_ne!(baseline, digest(vec![unknown]));
    let provider = TestProvider { id: "test.counter" };
    provider
        .validate_sources(
            &g,
            &extension,
            &[EvidenceSource {
                file_path: "fixture/input.txt",
                bytes: b"independent physical fixture\n",
            }],
        )
        .unwrap();
    assert!(provider
        .validate_sources(
            &g,
            &extension,
            &[EvidenceSource {
                file_path: "fixture/input.txt",
                bytes: b"forged"
            }]
        )
        .is_err());
}

#[test]
fn export_native_contract_packet() {
    use serde_json::json;
    use spur_graph::GraphQueryClient;
    let (bound_graph, bound) = source_fixture();
    let counter = counter_extension(1);
    let second = second_extension();
    let mut zero = counter.clone();
    zero.tables[0].rows.clear();
    let mut absent = zero.clone();
    absent.descriptor.presence = EvidencePresence::Absent;
    absent.tables.clear();
    absent.domain = EvidenceDomain::Unknown;
    let mut unknown = zero.clone();
    unknown.domain = EvidenceDomain::Unknown;
    let mut unsupported = counter.clone();
    unsupported.descriptor.schema_version = "future-v2".into();
    let mut opaque = counter.clone();
    opaque.descriptor.provider_id = "opaque.fixture".into();
    let mut cases = vec![
        ("empty", inventory(vec![])),
        (
            "twoProviders",
            inventory(vec![counter.clone(), second.clone()]),
        ),
        (
            "providerOrderReversed",
            inventory(vec![second.clone(), counter.clone()]),
        ),
        (
            "counterPayloadMutation",
            inventory(vec![counter_extension(2), second.clone()]),
        ),
        ("providerRemoved", inventory(vec![second])),
        ("beforeCapturedZero", inventory(vec![zero])),
        ("afterAbsent", inventory(vec![absent])),
        ("capturedUnknown", inventory(vec![unknown])),
        ("unsupportedSchema", inventory(vec![unsupported.clone()])),
        ("opaqueProvider", inventory(vec![opaque.clone()])),
        ("fileScoped", inventory(vec![bound.clone()])),
    ];
    for (name, mutate) in [
        (
            "schemaMutation",
            (|e: &mut EvidenceExtension| e.descriptor.schema_version = "counter-v2".into())
                as fn(&mut EvidenceExtension),
        ),
        ("extractorMutation", |e: &mut EvidenceExtension| {
            e.descriptor.extractor_version = "fixture-v2".into()
        }),
        ("componentMutation", |e: &mut EvidenceExtension| {
            e.descriptor
                .component_versions
                .insert("component".into(), "v2".into());
        }),
        ("metadataMutation", |e: &mut EvidenceExtension| {
            e.descriptor.metadata.insert("key".into(), "value".into());
        }),
        ("tableMetadataMutation", |e: &mut EvidenceExtension| {
            e.tables[0].metadata.insert("unit".into(), "items".into());
        }),
        ("tableSchemaMutation", |e: &mut EvidenceExtension| {
            e.tables[0].columns[1].nullable = true
        }),
    ] {
        let mut changed = counter.clone();
        mutate(&mut changed);
        cases.push((name, inventory(vec![changed, second_extension()])));
    }
    let mut outputs = Vec::new();
    for (name, input) in cases {
        let manifest = evidence_inventory_manifest(&input).unwrap();
        let extensions = input
            .extensions
            .iter()
            .map(|e| {
                json!({
                    "provider_id":e.descriptor.provider_id,
                    "checksum":evidence_extension_checksum(e).unwrap(),
                    "tables":e.tables.iter().map(|t| json!({"name":t.name,
                        "checksum":evidence_table_checksum(t).unwrap()})).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        outputs.push(json!({"name":name,"fixture":input,"manifest":manifest,
            "capability_input_tuple":["spur-evidence-capability-v1",EVIDENCE_SCHEMA_VERSION,
                "fixture-manifest",manifest.extensions],
            "capability_digest":evidence_inventory_capability_digest("fixture-manifest",&input).unwrap(),
            "extension_helper_calls":extensions}));
    }
    let provider = TestProvider { id: "test.counter" };
    let providers: [&dyn EvidenceProvider; 1] = [&provider];
    let registry = EvidenceRegistry::new(&providers).unwrap();
    let mut duplicate = counter.clone();
    let row = duplicate.tables[0].rows[0].clone();
    duplicate.tables[0].rows.push(row);
    let mut wrong_type = counter.clone();
    wrong_type.tables[0].rows[0][1] = EvidenceValue::Utf8("1".into());
    let mut reserved = counter.clone();
    reserved.tables[0].storage_path = "manifest.json".into();
    let mut wrong_origin = bound.clone();
    wrong_origin.origins[0].key.graph_content_hash = "forged".into();
    let negatives = vec![
        json!({"name":"duplicatePrimaryKey","input":duplicate,"error":evidence_extension_checksum(&duplicate).unwrap_err()}),
        json!({"name":"wrongCellType","input":wrong_type,"error":evidence_extension_checksum(&wrong_type).unwrap_err()}),
        json!({"name":"reservedCorePath","input":reserved,"error":evidence_extension_checksum(&reserved).unwrap_err()}),
        json!({"name":"forgedOriginalHash","input":wrong_origin,"error":inventory(vec![wrong_origin.clone()]).validate_integrity(&bound_graph).unwrap_err()}),
    ];
    let client = spur_graph::InMemoryClient::new(std::sync::Arc::new(graph()));
    let filter = EvidenceFilter {
        provider_id: "test.counter".into(),
        file_path: None,
        source_oid: None,
    };
    let invalid_filter = EvidenceFilter {
        source_oid: Some("missing_path".into()),
        ..filter.clone()
    };
    let executable = std::env::current_exe().unwrap();
    let executable_bytes = std::fs::read(&executable).unwrap();
    let compiler = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .unwrap();
    let source_files: [(&str, &[u8]); 10] = [
        (
            "crates/spur-graph/src/evidence/contract.rs",
            include_bytes!("../src/evidence/contract.rs"),
        ),
        (
            "crates/spur-graph/src/evidence/mod.rs",
            include_bytes!("../src/evidence/mod.rs"),
        ),
        (
            "crates/spur-graph/src/lib.rs",
            include_bytes!("../src/lib.rs"),
        ),
        (
            "crates/spur-graph/src/query_client.rs",
            include_bytes!("../src/query_client.rs"),
        ),
        (
            "crates/spur-graph/tests/generic_evidence_contract.rs",
            include_bytes!("generic_evidence_contract.rs"),
        ),
        (
            "scripts/spur-cargo",
            include_bytes!("../../../scripts/spur-cargo"),
        ),
        (
            ".cargo/config.toml",
            include_bytes!("../../../.cargo/config.toml"),
        ),
        ("Cargo.toml", include_bytes!("../../../Cargo.toml")),
        ("Cargo.lock", include_bytes!("../../../Cargo.lock")),
        (
            "crates/spur-graph/Cargo.toml",
            include_bytes!("../Cargo.toml"),
        ),
    ];
    use sha2::{Digest, Sha256};
    let packet = json!({
        "fixture_kind":"synthetic provider-neutral contract fixtures; no collector/artifact/disk/cache results",
        "cases":outputs,"negative_mutations":negatives,
        "graph_binding_fixture":bound_graph,"physical_fixture_bytes":"independent physical fixture\n",
        "registry":{
            "validated":registry.validate(&graph(),&counter).unwrap(),
            "opaque":registry.validate(&graph(),&opaque).unwrap(),
            "unsupported":registry.validate(&graph(),&unsupported).unwrap(),
            "semantic_error":registry.validate(&graph(),&counter_extension(-1)).unwrap_err(),
        },
        "default_query":{"filter":filter,"result":client.evidence(&filter).unwrap(),
            "invalid_filter":invalid_filter,"shape_error":invalid_filter.validate_shape().unwrap_err()},
        "native_identity":{"executable":executable,"executable_blake3":blake3::hash(&executable_bytes).to_hex().to_string(),
            "executable_sha256":format!("{:x}",Sha256::digest(&executable_bytes)),
            "compiler_probe_status":compiler.status.code(),"rustc_vV":String::from_utf8_lossy(&compiler.stdout),
            "arch":std::env::consts::ARCH,"os":std::env::consts::OS,
            "compiled_source":source_files.iter().map(|(path,bytes)| json!({"path":path,"byte_length":bytes.len(),
                "blake3":blake3::hash(bytes).to_hex().to_string(),
                "sha256":format!("{:x}",Sha256::digest(bytes))})).collect::<Vec<_>>()},
    });
    // Keep the complete inputs and outputs reloadable, not just their digest strings.
    let encoded = serde_json::to_string(&packet).unwrap();
    let reloaded: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(reloaded, packet);
    println!("G01_NATIVE_EXPORT={encoded}");
}

#[test]
fn duplicate_or_contradictory_current_file_owners_reject() {
    let (mut graph, e) = source_fixture();
    graph.files.push(graph.files[0].clone());
    assert!(inventory(vec![e.clone()])
        .validate_integrity(&graph)
        .is_err());
    graph.files[1].stable_file_id = "contradictory".into();
    assert!(inventory(vec![e]).validate_integrity(&graph).is_err());
}

#[test]
fn original_inventory_keeps_graph_keys_without_inventing_storage_path_policy() {
    let (mut graph, mut e) = source_fixture();
    // Original inventories contain all graph bindings, including nonphysical keys.
    // Only the provider's active physical source below needs a graph file owner.
    let foreign = EvidenceSourceKey {
        file_path: "external://snapshot/item".into(),
        stable_file_id: "external-key".into(),
        source_oid: "opaque-source-key".into(),
    };
    graph
        .file_manifests
        .push(spur_graph::GraphFileManifestEntry {
            path: foreign.file_path.clone(),
            stable_file_id: foreign.stable_file_id.clone(),
            content_oid: foreign.source_oid.clone(),
            node_ids: vec![],
        });
    graph.graph_content_hash = spur_graph::compute_graph_content_hash(
        graph
            .file_manifests
            .iter()
            .map(|f| (&f.path, &f.content_oid)),
    );
    e.origins[0].sources.push(foreign);
    e.origins[0].key.graph_content_hash = graph.graph_content_hash.clone();
    if let EvidenceDomain::Known { sources } = &mut e.domain {
        sources[0].origin = Some(e.origins[0].key.clone());
    }
    e.tables[0].rows[0][4] = EvidenceValue::Utf8(graph.graph_content_hash.clone());
    inventory(vec![e]).validate_integrity(&graph).unwrap();
}

#[test]
fn legacy_length_unknown_is_explicit_metadata_and_never_fabricated_zero() {
    let (graph, mut extension) = source_fixture();
    if let EvidenceDomain::Known { sources } = &mut extension.domain {
        sources[0].byte_length = None;
    }
    assert!(inventory(vec![extension.clone()])
        .validate_integrity(&graph)
        .is_err());
    extension
        .descriptor
        .metadata
        .insert("original_format".into(), "fixture-old-format".into());
    inventory(vec![extension.clone()])
        .validate_integrity(&graph)
        .unwrap();
    let manifest = evidence_inventory_manifest(&inventory(vec![extension.clone()])).unwrap();
    if let EvidenceDomain::Known { sources } = &manifest.extensions[0].domain {
        assert_eq!(sources[0].byte_length, None);
    }
    let unregistered = EvidenceRegistry::new(&[]).unwrap();
    assert_eq!(
        unregistered.validate(&graph, &extension).unwrap().domain,
        EvidenceDomain::Unknown
    );
    extension
        .descriptor
        .metadata
        .insert("original_format".into(), String::new());
    assert!(inventory(vec![extension])
        .validate_integrity(&graph)
        .is_err());
}
