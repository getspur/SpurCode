use spur_graph::{
    git_blob_oid, macro_invocation_site_id, GraphFacts, MacroAttributeEvidence,
    MacroDefinitionLocation, MacroEvidenceSource, MacroExecutionPhase, MacroInvocationEvidence,
    MacroInvocationEvidenceSet, MacroResolutionEvidence, MacroScopeEvidence, MacroScopeKind,
    MacroScopeRef, MacroSinkFamily, MacroTestContext, MACRO_INVOCATION_SCHEMA_VERSION,
};

const SOURCE: &str = "#[cfg_attr(feature = \"checks\", test)]\nfn demo() {\n    debug_assert!(true);\n    debug_assert!(false);\n}\nregister!();\n";
const PATH: &str = "src/lib.rs";
const GRAPH: &str = "fixture-graph";

fn scope(id: &str, kind: MacroScopeKind) -> MacroScopeRef {
    MacroScopeRef {
        stable_symbol_id: id.into(),
        kind,
    }
}

fn scopes() -> Vec<MacroScopeEvidence> {
    vec![
        MacroScopeEvidence {
            scope: scope("demo", MacroScopeKind::Function),
            file_path: PATH.into(),
            source_oid: git_blob_oid(SOURCE.as_bytes()),
            byte_range: [38, 102],
            line_range: [2, 5],
        },
        MacroScopeEvidence {
            scope: scope("file:src/lib.rs", MacroScopeKind::File),
            file_path: PATH.into(),
            source_oid: git_blob_oid(SOURCE.as_bytes()),
            byte_range: [0, SOURCE.len()],
            line_range: [1, 6],
        },
    ]
}

fn fixture() -> MacroInvocationEvidenceSet {
    let oid = git_blob_oid(SOURCE.as_bytes());
    let rows = [([54, 73], [3, 3]), ([79, 99], [4, 4]), ([103, 114], [6, 6])]
        .into_iter()
        .enumerate()
        .map(|(index, (byte_range, line_range))| {
            let in_function = index < 2;
            MacroInvocationEvidence {
                // M2-M5 must use this helper, with git_blob_oid of the exact source bytes.
                site_id: macro_invocation_site_id(PATH, &oid, byte_range, line_range),
                file_path: PATH.into(),
                source_oid: oid.clone(),
                caller_stable_symbol_id: in_function.then(|| "demo".into()),
                enclosing: if in_function {
                    scope("demo", MacroScopeKind::Function)
                } else {
                    scope("file:src/lib.rs", MacroScopeKind::File)
                },
                raw_macro_path: if in_function {
                    "debug_assert"
                } else {
                    "register"
                }
                .into(),
                normalized_macro_path: if in_function {
                    "debug_assert"
                } else {
                    "register"
                }
                .into(),
                byte_range,
                line_range,
                attributes: if in_function {
                    vec![MacroAttributeEvidence {
                        raw_text: SOURCE[..37].into(),
                        owner: scope("demo", MacroScopeKind::Function),
                        byte_range: [0, 37],
                        line_range: [1, 1],
                    }]
                } else {
                    vec![]
                },
                test_context: if in_function {
                    MacroTestContext::Conditional
                } else {
                    MacroTestContext::Unspecified
                },
                debug_assertions_candidate: in_function,
                execution_phase: if in_function {
                    MacroExecutionPhase::FunctionBodySyntax
                } else {
                    MacroExecutionPhase::Unknown
                },
                resolution: MacroResolutionEvidence::Unresolved,
                sink_family_candidate: in_function.then_some(MacroSinkFamily::DebugAssertion),
            }
        })
        .collect();
    MacroInvocationEvidenceSet {
        schema_version: MACRO_INVOCATION_SCHEMA_VERSION.into(),
        extractor_version: "fixture-extractor-v1".into(),
        graph_content_hash: GRAPH.into(),
        source_scope: spur_graph::MacroEvidenceSourceScope::PhysicalRustFiles,
        scopes: scopes(),
        rows,
    }
}

fn validate(set: &MacroInvocationEvidenceSet) -> anyhow::Result<()> {
    set.validate(
        GRAPH,
        &[MacroEvidenceSource {
            file_path: PATH,
            bytes: SOURCE.as_bytes(),
        }],
        &scopes(),
    )
}

#[test]
fn fixture_coordinates_match_independently_enumerated_source_lines() {
    assert_eq!(SOURCE.len(), 116);
    // Intersect whole line byte intervals with each half-open span. This is
    // independent of the validator's newline-prefix counts, including at EOF.
    let covered_lines = |[start, end]: [usize; 2]| {
        let mut offset = 0;
        let mut covered = Vec::new();
        for (index, line) in SOURCE.split_inclusive('\n').enumerate() {
            let next = offset + line.len();
            if start.max(offset) < end.min(next) {
                covered.push(index + 1);
            }
            offset = next;
        }
        [covered[0], *covered.last().unwrap()]
    };
    for (bytes, lines, text) in [
        ([54, 73], [3, 3], "debug_assert!(true)"),
        ([79, 99], [4, 4], "debug_assert!(false)"),
        ([103, 114], [6, 6], "register!()"),
    ] {
        assert_eq!(&SOURCE[bytes[0]..bytes[1]], text);
        assert_eq!(covered_lines(bytes), lines);
    }
    for location in scopes() {
        assert_eq!(covered_lines(location.byte_range), location.line_range);
    }
    for row in fixture().rows {
        assert_eq!(covered_lines(row.byte_range), row.line_range);
        for attribute in row.attributes {
            assert_eq!(covered_lines(attribute.byte_range), attribute.line_range);
        }
    }
}

#[test]
fn function_body_syntax_requires_function_scope_and_exact_caller() {
    let mut set = fixture();
    validate(&set).unwrap();
    assert_eq!(set.rows[2].enclosing.kind, MacroScopeKind::File);
    assert_eq!(set.rows[2].caller_stable_symbol_id, None);
    set.rows[2].execution_phase = MacroExecutionPhase::FunctionBodySyntax;
    assert!(
        validate(&set).is_err(),
        "FunctionBodySyntax must reject file enclosing with no exact function caller"
    );
}

#[test]
fn unknown_execution_phase_preserves_scope_and_caller_uncertainty() {
    let mut set = fixture();
    for row in &mut set.rows {
        row.execution_phase = MacroExecutionPhase::Unknown;
    }
    validate(&set).unwrap();
    assert_eq!(set.rows[0].caller_stable_symbol_id.as_deref(), Some("demo"));
    assert_eq!(set.rows[2].enclosing.kind, MacroScopeKind::File);
    assert_eq!(set.rows[2].caller_stable_symbol_id, None);
}

#[test]
fn roundtrip_preserves_all_evidence_and_explicit_unknown_states() {
    let mut set = fixture();
    validate(&set).unwrap();
    set.rows[0].resolution = MacroResolutionEvidence::ExplicitPathCandidate;
    set.rows[0].raw_macro_path = "::std /* retained */ :: debug_assert".into();
    set.rows[0].normalized_macro_path = "::std::debug_assert".into();
    set.rows[1].resolution = MacroResolutionEvidence::LocalDefinitionCandidate {
        definition: MacroDefinitionLocation {
            file_path: PATH.into(),
            source_oid: git_blob_oid(SOURCE.as_bytes()),
            byte_range: [38, 102],
            line_range: [2, 5],
        },
    };
    // This is a location contract test, not a claim that this fixture declares a macro.
    set.rows[1].test_context = MacroTestContext::TestAttributeCandidate;
    let decoded: MacroInvocationEvidenceSet =
        serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
    assert_eq!(decoded, set);
    assert_eq!(decoded.rows[2].test_context, MacroTestContext::Unspecified);
    assert_eq!(
        decoded.rows[2].execution_phase,
        MacroExecutionPhase::Unknown
    );
    assert_eq!(
        decoded.rows[2].resolution,
        MacroResolutionEvidence::Unresolved
    );
    assert_eq!(decoded.rows[2].caller_stable_symbol_id, None);
}

#[test]
fn identity_is_full_digest_order_independent_and_snapshot_scoped() {
    let set = fixture();
    assert_eq!(
        set.rows[0].source_oid,
        "2d20325ee56a0c1ece1588f3adca308693e93f11"
    );
    assert_eq!(set.rows[0].raw_macro_path, set.rows[1].raw_macro_path);
    assert_ne!(set.rows[0].site_id, set.rows[1].site_id);
    for row in set.rows.iter().rev() {
        assert_eq!(
            row.site_id,
            macro_invocation_site_id(PATH, &row.source_oid, row.byte_range, row.line_range)
        );
        assert_eq!(row.site_id.len(), blake3::OUT_LEN * 2);
        assert_ne!(
            row.site_id,
            macro_invocation_site_id(
                PATH,
                &git_blob_oid(b"changed bytes"),
                row.byte_range,
                row.line_range
            )
        );
        assert_ne!(
            row.site_id,
            macro_invocation_site_id(
                "src/renamed.rs",
                &row.source_oid,
                row.byte_range,
                row.line_range
            )
        );
        assert_ne!(
            row.site_id,
            macro_invocation_site_id(
                PATH,
                &row.source_oid,
                [row.byte_range[0] + 1, row.byte_range[1]],
                row.line_range
            )
        );
        assert_ne!(
            row.site_id,
            macro_invocation_site_id(PATH, &row.source_oid, row.byte_range, [1, 1])
        );
    }
    // Length prefixes distinguish inputs that would collide under naive concatenation.
    assert_ne!(
        macro_invocation_site_id("ab", "c", [0, 1], [1, 1]),
        macro_invocation_site_id("a", "bc", [0, 1], [1, 1])
    );
    let mut other = set.clone();
    other.graph_content_hash = "other-snapshot".into();
    assert_eq!(other.rows, set.rows);
    assert_ne!(
        (&other.graph_content_hash, &other.rows[0].site_id),
        (&set.graph_content_hash, &set.rows[0].site_id)
    );
}

#[test]
fn rejects_invalid_spans_and_mismatching_line_coordinates() {
    for (bytes, lines) in [
        ([73, 54], [3, 3]),
        ([54, 54], [3, 3]),
        ([54, SOURCE.len() + 1], [3, 3]),
        ([54, 73], [0, 3]),
        ([54, 73], [4, 3]),
        ([54, 73], [4, 4]),
    ] {
        let mut set = fixture();
        set.rows[0].byte_range = bytes;
        set.rows[0].line_range = lines;
        set.rows[0].site_id = macro_invocation_site_id(PATH, &set.rows[0].source_oid, bytes, lines);
        assert!(
            validate(&set).is_err(),
            "accepted spans {bytes:?} {lines:?}"
        );
    }
}

#[test]
fn rejects_empty_stale_missing_and_mismatching_provenance() {
    for field in ["schema_version", "extractor_version", "graph_content_hash"] {
        let mut json = serde_json::to_value(fixture()).unwrap();
        json[field] = "".into();
        assert!(
            validate(&serde_json::from_value(json).unwrap()).is_err(),
            "accepted empty {field}"
        );
    }
    for field in [
        "file_path",
        "source_oid",
        "site_id",
        "raw_macro_path",
        "normalized_macro_path",
    ] {
        let mut json = serde_json::to_value(fixture()).unwrap();
        json["rows"][0][field] = "".into();
        assert!(
            validate(&serde_json::from_value(json).unwrap()).is_err(),
            "accepted empty {field}"
        );
    }
    let mut set = fixture();
    set.graph_content_hash = "stale-graph".into();
    assert!(validate(&set).is_err());
    let mut set = fixture();
    set.rows[0].source_oid = git_blob_oid(b"stale source");
    assert!(validate(&set).is_err());
    assert!(fixture().validate(GRAPH, &[], &scopes()).is_err());
    assert!(fixture()
        .validate(
            GRAPH,
            &[MacroEvidenceSource {
                file_path: PATH,
                bytes: b"different bytes"
            }],
            &scopes()
        )
        .is_err());
    let mut set = fixture();
    set.rows[0].site_id = "forged".into();
    assert!(validate(&set).is_err());
}

#[test]
fn rejects_duplicate_sites_and_inconsistent_enclosing_or_caller_references() {
    let mut set = fixture();
    set.rows.push(set.rows[0].clone());
    assert!(validate(&set).is_err());
    for caller in [
        None,
        Some("unrelated".into()),
        Some("file:src/lib.rs".into()),
    ] {
        let mut set = fixture();
        set.rows[0].caller_stable_symbol_id = caller;
        assert!(validate(&set).is_err());
    }
    let mut set = fixture();
    set.rows[2].caller_stable_symbol_id = Some("demo".into());
    assert!(validate(&set).is_err(), "file scope cannot invent a caller");
    let mut set = fixture();
    set.rows[0].enclosing.stable_symbol_id = "missing".into();
    assert!(validate(&set).is_err());
    let mut wrong_scopes = scopes();
    wrong_scopes[0].byte_range = [79, 99];
    wrong_scopes[0].line_range = [4, 4];
    assert!(fixture()
        .validate(
            GRAPH,
            &[MacroEvidenceSource {
                file_path: PATH,
                bytes: SOURCE.as_bytes()
            }],
            &wrong_scopes
        )
        .is_err());
    let mut wrong_scopes = scopes();
    wrong_scopes[0].source_oid = git_blob_oid(b"stale");
    assert!(fixture()
        .validate(
            GRAPH,
            &[MacroEvidenceSource {
                file_path: PATH,
                bytes: SOURCE.as_bytes()
            }],
            &wrong_scopes
        )
        .is_err());
}

#[test]
fn validates_attribute_ownership_raw_text_and_local_definition_provenance() {
    for mutation in 0..4 {
        let mut set = fixture();
        let attr = &mut set.rows[0].attributes[0];
        match mutation {
            0 => attr.owner.stable_symbol_id = "missing".into(),
            1 => attr.raw_text = "#[test]".into(),
            2 => attr.byte_range = [0, SOURCE.len() + 1],
            _ => attr.owner = scope("demo", MacroScopeKind::Module),
        }
        assert!(
            validate(&set).is_err(),
            "accepted invalid attribute {mutation}"
        );
    }
    let mut set = fixture();
    set.rows[0].resolution = MacroResolutionEvidence::LocalDefinitionCandidate {
        definition: MacroDefinitionLocation {
            file_path: PATH.into(),
            source_oid: git_blob_oid(b"stale"),
            byte_range: [38, 102],
            line_range: [2, 5],
        },
    };
    assert!(validate(&set).is_err());
}

#[test]
fn empty_facts_and_captured_empty_set_are_distinct_from_unavailable() {
    assert!(GraphFacts::empty().macro_invocations.is_empty());
    let mut set = fixture();
    set.rows.clear();
    set.scopes.clear();
    set.validate(GRAPH, &[], &[]).unwrap();
    let captured = Some(set);
    let unavailable: Option<MacroInvocationEvidenceSet> = None;
    assert_ne!(
        serde_json::to_value(captured).unwrap(),
        serde_json::to_value(unavailable).unwrap()
    );
}

#[test]
fn emits_observed_contract_rows_for_solver_post() {
    let set = fixture();
    validate(&set).unwrap();
    println!(
        "M1_OBSERVED_CONTRACT={}",
        serde_json::to_string(&set).unwrap()
    );
}
