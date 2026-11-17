use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

fn code(id: &str, lang: &str, source: &str) -> Value {
    json!({"id":id,"cell_type":"code","metadata":{"spur":{"code_type":lang}},"source":source})
}
fn notebook(cells: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({"cells":cells,"metadata":{}})).unwrap()
}
fn one(source: &str) -> Vec<u8> {
    notebook(vec![code("r", "rust", source)])
}
fn compare(a: &[u8], b: &[u8]) -> NotebookComparison {
    compare_notebook_sources("n.ipynb", None, "M", Some(a), Some(b)).unwrap()
}
fn count(doc: &NotebookSymbolDocument) -> usize {
    doc.cells.iter().map(|c| c.symbols.len()).sum()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn resolve<'a>(doc: &'a NotebookSymbolDocument, r: &SourceRef) -> &'a str {
    assert_eq!(r.notebook_path, doc.path);
    assert_eq!(r.notebook_source_sha256, doc.source_sha256);
    assert_eq!(r.coordinate_space, CoordinateSpace::DecodedCodeCellUtf8);
    let c = &doc.cells[r.cell_ordinal].cell;
    assert_eq!(r.cell_id, c.id);
    assert_eq!(Some(r.cell_language), c.language);
    let p = c.source.as_ref().unwrap();
    assert_eq!(r.cell_source_sha256, hash(p.original.as_bytes()));
    let [start, end] = r.byte_range;
    assert!(start < end);
    let slice = &p.original[start..end];
    assert_eq!(
        r.line_range,
        [
            p.original[..start].bytes().filter(|b| *b == b'\n').count() + 1,
            p.original[..end].bytes().filter(|b| *b == b'\n').count() + 1
        ]
    );
    slice
}
fn cell_event(c: &NotebookComparison, k: NotebookCellChangeKind) -> bool {
    c.cell_changes.iter().any(|e| e.kind == k)
}

#[test]
fn frozen_app_symbols_and_all_28_independent_function_anchors() {
    let input: Value = serde_json::from_str(include_str!(
        "../../../../docs/research/artifacts/2026-11-17-notebook-rust-poc-bd-26sp/real-inputs.json"
    ))
    .unwrap();
    let frozen:Value=serde_json::from_str(include_str!("../../../../docs/research/artifacts/2026-11-17-notebook-symbol-diagnosis-bd-nb4c/frozen-cell-model-input.json")).unwrap();
    let mut docs = vec![];
    let mut anchors = 0;
    for (i, snapshot) in input["snapshots"].as_array().unwrap().iter().enumerate() {
        let bytes = snapshot["source"].as_str().unwrap().as_bytes();
        let doc =
            extract_notebook_symbols(snapshot["notebook_path"].as_str().unwrap(), bytes).unwrap();
        assert_eq!(
            doc.source_sha256,
            snapshot["source_sha256"].as_str().unwrap()
        );
        assert_eq!(
            doc.status,
            NotebookStatus::Complete,
            "{:?}",
            doc.diagnostics
        );
        assert_eq!(count(&doc), [46, 24][i], "frozen real notebook symbols");
        let functions: Vec<_> = doc
            .cells
            .iter()
            .flat_map(|c| &c.symbols)
            .filter(|s| s.symbol.symbol_kind == "function")
            .collect();
        assert_eq!(functions.len(), [17, 11][i]);
        let mut ids = HashSet::new();
        for s in doc.cells.iter().flat_map(|c| &c.symbols) {
            assert!(ids.insert(&s.id));
            assert!(!s.id.contains(".rs\""));
            assert_eq!(s.symbol.byte_range, s.source_ref.byte_range);
            assert_eq!(s.symbol.line_range, s.source_ref.line_range);
            assert!(!s.symbol.anchor_hash.is_empty());
            assert!(!resolve(&doc, &s.source_ref).is_empty());
        }
        for anchor in frozen["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["snapshot_id"] == snapshot["snapshot_id"])
        {
            let matches: Vec<_> = functions
                .iter()
                .filter(|s| {
                    s.symbol.entity_name == anchor["entity_name"].as_str().unwrap()
                        && s.source_ref.cell_id == anchor["cell_id"].as_str().unwrap()
                })
                .collect();
            assert_eq!(matches.len(), 1);
            let s = matches[0];
            assert_eq!(
                s.symbol.byte_range,
                [
                    anchor["byte_start"].as_u64().unwrap() as usize,
                    anchor["byte_end"].as_u64().unwrap() as usize
                ]
            );
            assert_eq!(
                s.symbol.line_range,
                [
                    anchor["line_start"].as_u64().unwrap() as usize,
                    anchor["line_end"].as_u64().unwrap() as usize
                ]
            );
            anchors += 1;
        }
        let noop = compare(bytes, bytes);
        assert!(noop.symbol_changes.is_empty());
        assert!(noop.cell_changes.is_empty());
        docs.push(bytes);
    }
    assert_eq!(anchors, 28);
    let diff = compare(docs[0], docs[1]);
    assert!(!diff.symbol_changes.is_empty());
    for change in &diff.symbol_changes {
        if let Some(s) = &change.before {
            resolve(diff.before.as_ref().unwrap(), &s.source_ref);
        }
        if let Some(s) = &change.after {
            resolve(diff.after.as_ref().unwrap(), &s.source_ref);
        }
    }
    eprintln!(
        "frozen real comparison: {} symbol changes; {} cell events",
        diff.symbol_changes.len(),
        diff.cell_changes.len()
    );
}

#[test]
fn duplicate_helpers_in_distinct_cells_do_not_collide() {
    let input = notebook(vec![
        code("a", "python", "def helper():\n    return 1\n"),
        code("b", "python", "def helper():\n    return 2\n"),
    ]);
    let doc = extract_notebook_symbols("n.ipynb", &input).unwrap();
    assert_eq!(count(&doc), 2);
    assert_ne!(doc.cells[0].symbols[0].id, doc.cells[1].symbols[0].id);
    let diff = compare(&input, &input);
    assert_eq!(diff.status, NotebookStatus::Complete);
    assert!(diff.symbol_changes.is_empty());
}

#[test]
fn canonical_ids_separate_delimiter_and_percent_witnesses() {
    let a = notebook(vec![code("b.ipynb#cell:c", "rust", "fn helper() {}")]);
    let b = notebook(vec![code("c", "rust", "fn helper() {}")]);
    let x = extract_notebook_symbols("a.ipynb", &a).unwrap();
    let y = extract_notebook_symbols("a.ipynb#cell:b.ipynb", &b).unwrap();
    assert_eq!(count(&x), 1);
    assert_eq!(count(&y), 1);
    assert_ne!(x.cells[0].symbols[0].id, y.cells[0].symbols[0].id);
    let mut ids = HashSet::new();
    for id in [
        "a#cell:b",
        "a%23cell%3Ab",
        "a:b",
        "a/b",
        "é",
        "a\\b",
        "a\"b",
    ] {
        let d = extract_notebook_symbols(
            "p#cell:q.ipynb",
            &notebook(vec![code(id, "rust", "fn f() {}")]),
        )
        .unwrap();
        assert!(ids.insert(d.cells[0].symbols[0].id.clone()));
        assert_eq!(d.cells[0].symbols[0].source_ref.cell_id, id);
    }
}

#[test]
fn same_cell_duplicate_match_keys_are_partial_without_symbol_loss() {
    let input = notebook(vec![code(
        "a",
        "python",
        "def helper():\n    return 1\ndef helper():\n    return 2\n",
    )]);
    let doc = extract_notebook_symbols("n.ipynb", &input).unwrap();
    assert_eq!(count(&doc), 2);
    assert_eq!(doc.status, NotebookStatus::Partial);
    assert!(doc
        .diagnostics
        .iter()
        .any(|d| d.code == SymbolDiagnosticCode::AmbiguousSymbolIdentity));
    let diff = compare(&input, &input);
    assert_eq!(diff.status, NotebookStatus::Partial);
    assert!(
        diff.symbol_changes.is_empty(),
        "ambiguous cell matching must be skipped explicitly"
    );
}

#[test]
fn original_utf8_crlf_refs_survive_preparation_and_changes() {
    let a = one(":dep serde = \"1\" # café\r\nfn café() -> i32 { 1 }\r\n");
    let b = one(":dep serde = \"1.0\" # café\r\nfn café() -> i32 { 2 }\r\n");
    let diff = compare(&a, &b);
    assert_eq!(diff.symbol_changes.len(), 1);
    let c = &diff.symbol_changes[0];
    assert_eq!(c.kind, NotebookSymbolChangeKind::Modified);
    assert_eq!(
        resolve(
            diff.before.as_ref().unwrap(),
            &c.before.as_ref().unwrap().source_ref
        ),
        "fn café() -> i32 { 1 }"
    );
    assert_eq!(
        resolve(
            diff.after.as_ref().unwrap(),
            &c.after.as_ref().unwrap().source_ref
        ),
        "fn café() -> i32 { 2 }"
    );
    let p = diff.after.as_ref().unwrap().cells[0]
        .cell
        .source
        .as_ref()
        .unwrap();
    assert_eq!(p.policy, RUST_PREPARATION_POLICY);
    assert_ne!(p.original_sha256, p.analysis_sha256);
    assert_eq!(p.directives.len(), 1);
}

#[test]
fn dep_version_length_change_keeps_identical_original_function() {
    let a = one(":dep serde = \"1\"\nfn target() -> i32 { 1 }\n");
    let b = one(":dep serde = \"1.0\"\nfn target() -> i32 { 1 }\n");
    let d = compare(&a, &b);
    assert!(
        d.symbol_changes.is_empty(),
        "offset movement is not a notebook symbol edit: {:?}",
        d.symbol_changes
    );
    assert!(cell_event(&d, NotebookCellChangeKind::SourceChanged));
    assert!(cell_event(
        &d,
        NotebookCellChangeKind::DependencyDirectivesChanged
    ));
}

#[test]
fn inserting_or_removing_dep_header_keeps_identical_original_function() {
    let a = one("fn target() -> i32 { 1 }\n");
    let b = one(":dep serde = \"1\"\nfn target() -> i32 { 1 }\n");
    for (l, r) in [(&a, &b), (&b, &a)] {
        let d = compare(l, r);
        assert!(
            d.symbol_changes.is_empty(),
            "header movement: {:?}",
            d.symbol_changes
        );
        assert!(cell_event(
            &d,
            NotebookCellChangeKind::DependencyDirectivesChanged
        ));
    }
}

#[test]
fn comment_prefix_movement_retains_current_refs_without_symbol_edits() {
    let a = one("fn target() -> i32 { 1 }\n");
    let b = one("// moved by café\r\n\r\nfn target() -> i32 { 1 }\n");
    let d = compare(&a, &b);
    assert!(
        d.symbol_changes.is_empty(),
        "prefix movement: {:?}",
        d.symbol_changes
    );
    assert!(cell_event(&d, NotebookCellChangeKind::SourceChanged));
    let after = d.after.as_ref().unwrap();
    let s = &after.cells[0].symbols[0];
    assert_eq!(s.source_ref.line_range, [3, 3]);
    resolve(after, &s.source_ref);
}

#[test]
fn symbol_rename_keeps_exact_before_snapshot_amid_other_declarations() {
    let a = one("fn old_name(x: u32, y: u32) -> u32 { x + y * 2 }\nfn keep() -> bool { true }\n");
    let b = one(
        "// prefix\nfn keep() -> bool { true }\nfn new_name(x: u32, y: u32) -> u32 { x + y * 2 }\n",
    );
    let d = compare(&a, &b);
    assert_eq!(d.symbol_changes.len(), 1);
    let c = &d.symbol_changes[0];
    assert_eq!(c.kind, NotebookSymbolChangeKind::Renamed);
    assert!(resolve(
        d.before.as_ref().unwrap(),
        &c.before.as_ref().unwrap().source_ref
    )
    .starts_with("fn old_name"));
    assert!(resolve(
        d.after.as_ref().unwrap(),
        &c.after.as_ref().unwrap().source_ref
    )
    .starts_with("fn new_name"));
}

#[test]
fn inventory_events_survive_import_only_empty_and_output_cells() {
    let a = notebook(vec![
        code("imports", "python", "import os\n"),
        code("empty", "rust", ""),
    ]);
    let b = notebook(vec![
        code("empty", "rust", ""),
        code("new", "python", "import sys\n"),
        code("imports", "python", "import pathlib\n"),
    ]);
    let d = compare(&a, &b);
    assert!(d.symbol_changes.is_empty());
    for k in [
        NotebookCellChangeKind::Added,
        NotebookCellChangeKind::Moved,
        NotebookCellChangeKind::SourceChanged,
    ] {
        assert!(cell_event(&d, k), "{k:?}");
    }
    let removed = compare(&b, &a);
    assert!(cell_event(&removed, NotebookCellChangeKind::Deleted));
    let mut output: Value = serde_json::from_slice(&a).unwrap();
    output["cells"][0]["outputs"] = json!([{"text":"changed"}]);
    output["cells"][0]["execution_count"] = json!(17);
    let d = compare(&a, &serde_json::to_vec(&output).unwrap());
    assert!(d.cell_changes.is_empty());
    assert!(d.symbol_changes.is_empty());
}

#[test]
fn type_language_profile_and_root_metadata_events_are_separate() {
    let a = notebook(vec![
        code("a", "python", "def f():\n    return 1\n"),
        code("b", "rust", "fn g() {}"),
    ]);
    let mut b: Value = serde_json::from_slice(&a).unwrap();
    b["cells"][0]["metadata"]["spur"]["code_type"] = json!("javascript");
    b["cells"][0]["source"] = json!("function f() { return 1; }");
    b["cells"][1]["cell_type"] = json!("raw");
    b["cells"][1]["metadata"]["profile"] = json!("saved");
    b["metadata"] = json!({"kernel_profile":"saved"});
    let d = compare(&a, &serde_json::to_vec(&b).unwrap());
    for k in [
        NotebookCellChangeKind::TypeChanged,
        NotebookCellChangeKind::LanguageChanged,
        NotebookCellChangeKind::MetadataChanged,
    ] {
        assert!(cell_event(&d, k));
    }
    assert!(d
        .document_changes
        .iter()
        .any(|e| e.kind == NotebookDocumentChangeKind::MetadataChanged));
    assert!(d
        .symbol_changes
        .iter()
        .any(|c| c.kind == NotebookSymbolChangeKind::Added));
    assert!(d
        .symbol_changes
        .iter()
        .any(|c| c.kind == NotebookSymbolChangeKind::Deleted));
    assert!(!d
        .symbol_changes
        .iter()
        .any(|c| c.kind == NotebookSymbolChangeKind::Renamed));
}

#[test]
fn anonymous_edit_is_conservative_and_unknown_language_stays_visible() {
    let mut cell = code("x", "python", "def f():\n    return 1\n");
    cell.as_object_mut().unwrap().remove("id");
    let a = notebook(vec![cell.clone(), code("unknown", "made-up", "x")]);
    cell["source"] = json!("def f():\n    return 2\n");
    let b = notebook(vec![cell, code("unknown", "made-up", "x")]);
    let d = compare(&a, &b);
    assert_eq!(d.status, NotebookStatus::Partial);
    assert_eq!(d.before.as_ref().unwrap().cells.len(), 2);
    assert!(cell_event(&d, NotebookCellChangeKind::Added));
    assert!(cell_event(&d, NotebookCellChangeKind::Deleted));
    assert!(d.diagnostics.iter().any(|e| e.code
        == SymbolDiagnosticCode::Normalization(NotebookDiagnosticCode::SourceDerivedIdentity)));
    assert_eq!(d.symbol_changes.len(), 2);
}

fn ambiguous_cells(explicit: bool) -> Vec<Value> {
    let mut cell = code("duplicate", "python", "def f():\n    return 1\n");
    if !explicit {
        cell.as_object_mut().unwrap().remove("id");
    }
    vec![cell.clone(), cell]
}

fn assert_ambiguous_inventory(d: &NotebookComparison, expected_symbols: usize) {
    assert_eq!(d.status, NotebookStatus::Partial);
    assert!(d
        .diagnostics
        .iter()
        .any(|e| e.code == SymbolDiagnosticCode::AmbiguousCellIdentity));
    for (side, doc) in [
        (NotebookSide::Before, d.before.as_ref().unwrap()),
        (NotebookSide::After, d.after.as_ref().unwrap()),
    ] {
        assert_eq!(doc.status, NotebookStatus::Partial);
        assert_eq!(count(doc), expected_symbols);
        assert!(d.diagnostics.iter().any(|e| e.side == Some(side)
            && matches!(
                e.code,
                SymbolDiagnosticCode::Normalization(
                    NotebookDiagnosticCode::RepeatedAnonymousSource
                        | NotebookDiagnosticCode::DuplicateEffectiveId
                )
            )));
        let mut ids = HashSet::new();
        for symbol in doc.cells.iter().flat_map(|c| &c.symbols) {
            assert!(ids.insert(&symbol.id));
            assert!(!resolve(doc, &symbol.source_ref).is_empty());
        }
    }
}

fn ambiguous_noop(explicit: bool) {
    let input = notebook(ambiguous_cells(explicit));
    let d = compare(&input, &input);
    assert_ambiguous_inventory(&d, 2);
    assert!(d.symbol_changes.is_empty());
    assert!(d.document_changes.is_empty());
    assert_eq!(
        d.cell_changes.len(),
        0,
        "unchanged ambiguous inventory must not manufacture cell D/A"
    );
}

#[test]
fn ambiguous_group_repeated_anonymous_noop() {
    ambiguous_noop(false);
}

#[test]
fn ambiguous_group_duplicate_explicit_noop() {
    ambiguous_noop(true);
}

fn ambiguous_output_only(explicit: bool) {
    let before = notebook(ambiguous_cells(explicit));
    let mut after: Value = serde_json::from_slice(&before).unwrap();
    after["cells"][0]["outputs"] = json!([{"output_type":"stream","text":"changed"}]);
    after["cells"][1]["execution_count"] = json!(17);
    // Source string/list and JSON formatting preserve the decoded inventory.
    after["cells"][1]["source"] = json!(["def f():\n", "    return 1\n"]);
    let after = serde_json::to_vec_pretty(&after).unwrap();
    let d = compare(&before, &after);
    assert_ambiguous_inventory(&d, 2);
    assert_ne!(
        d.before.as_ref().unwrap().source_sha256,
        d.after.as_ref().unwrap().source_sha256
    );
    assert!(d.symbol_changes.is_empty());
    assert!(d.document_changes.is_empty());
    assert_eq!(
        d.cell_changes.len(),
        0,
        "output-only edits must not manufacture ambiguous-cell D/A"
    );
}

#[test]
fn ambiguous_group_repeated_anonymous_output_only() {
    ambiguous_output_only(false);
}

#[test]
fn ambiguous_group_duplicate_explicit_output_only() {
    ambiguous_output_only(true);
}

fn ambiguous_alongside_stable_edit(explicit: bool) {
    let mut cells = ambiguous_cells(explicit);
    cells.push(code(
        "stable",
        "rust",
        ":dep serde = \"1\"\nfn g() -> u32 { 1 }\n",
    ));
    let before = notebook(cells);
    let mut after: Value = serde_json::from_slice(&before).unwrap();
    after["cells"][2]["source"] = json!(":dep serde = \"1.0\"\nfn g() -> u32 { 2 }\n");
    after["cells"][2]["metadata"]["profile"] = json!("saved");
    after["metadata"] = json!({"profile":"saved"});
    let after = serde_json::to_vec(&after).unwrap();
    let d = compare(&before, &after);
    assert_ambiguous_inventory(&d, 3);
    assert_eq!(d.symbol_changes.len(), 1);
    let change = &d.symbol_changes[0];
    assert_eq!(change.kind, NotebookSymbolChangeKind::Modified);
    for (doc, symbol, expected) in [
        (
            d.before.as_ref().unwrap(),
            change.before.as_ref().unwrap(),
            "fn g() -> u32 { 1 }",
        ),
        (
            d.after.as_ref().unwrap(),
            change.after.as_ref().unwrap(),
            "fn g() -> u32 { 2 }",
        ),
    ] {
        assert_eq!(symbol.source_ref.cell_id, "stable");
        assert_eq!(resolve(doc, &symbol.source_ref), expected);
    }
    assert_eq!(
        d.cell_changes.len(),
        3,
        "only genuine stable-cell events survive"
    );
    for kind in [
        NotebookCellChangeKind::SourceChanged,
        NotebookCellChangeKind::MetadataChanged,
        NotebookCellChangeKind::DependencyDirectivesChanged,
    ] {
        assert!(cell_event(&d, kind));
    }
    for change in &d.cell_changes {
        assert_eq!(change.before.as_ref().unwrap().cell_id, "stable");
        assert_eq!(change.after.as_ref().unwrap().cell_id, "stable");
    }
    assert_eq!(d.document_changes.len(), 1);
    assert_eq!(
        d.document_changes[0].kind,
        NotebookDocumentChangeKind::MetadataChanged
    );
}

#[test]
fn ambiguous_group_repeated_anonymous_preserves_independent_edit() {
    ambiguous_alongside_stable_edit(false);
}

#[test]
fn ambiguous_group_duplicate_explicit_preserves_independent_edit() {
    ambiguous_alongside_stable_edit(true);
}

#[test]
fn ambiguous_group_actual_edits_keep_conservative_cell_events() {
    for explicit in [false, true] {
        let mut cells = ambiguous_cells(explicit);
        // Distinguishable duplicate explicit sources also permit a real reorder witness.
        if explicit {
            cells[1]["source"] = json!("def g():\n    return 2\n");
        }
        let before = notebook(cells);
        for edit in [
            "source", "metadata", "language", "type", "remove", "add", "move",
        ] {
            let mut after: Value = serde_json::from_slice(&before).unwrap();
            match edit {
                "source" => after["cells"][0]["source"] = json!("def f():\n    return 3\n"),
                "metadata" => after["cells"][0]["metadata"]["profile"] = json!("saved"),
                "language" => {
                    after["cells"][0]["metadata"]["spur"]["code_type"] = json!("javascript")
                }
                "type" => after["cells"][0]["cell_type"] = json!("raw"),
                "remove" => {
                    after["cells"].as_array_mut().unwrap().pop();
                }
                "add" => {
                    let cell = after["cells"][0].clone();
                    after["cells"].as_array_mut().unwrap().push(cell);
                }
                "move" => after["cells"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, code("prefix", "rust", "")),
                _ => unreachable!(),
            }
            let d = compare(&before, &serde_json::to_vec(&after).unwrap());
            assert_eq!(d.status, NotebookStatus::Partial, "{explicit}/{edit}");
            if !explicit && edit == "add" {
                // Existing occurrence IDs and inventories are unchanged; only
                // the new occurrence warrants an event.
                assert_eq!(d.cell_changes.len(), 1);
                assert_eq!(d.cell_changes[0].kind, NotebookCellChangeKind::Added);
                assert_eq!(d.cell_changes[0].after.as_ref().unwrap().cell_ordinal, 2);
                continue;
            }
            assert!(
                cell_event(&d, NotebookCellChangeKind::Deleted),
                "{explicit}/{edit}"
            );
            assert!(
                cell_event(&d, NotebookCellChangeKind::Added),
                "{explicit}/{edit}"
            );
        }
        if explicit {
            let mut after: Value = serde_json::from_slice(&before).unwrap();
            after["cells"].as_array_mut().unwrap().swap(0, 1);
            let d = compare(&before, &serde_json::to_vec(&after).unwrap());
            assert_eq!(
                d.cell_changes.len(),
                4,
                "ambiguous reorder remains conservative"
            );
        }
    }
}

#[test]
fn add_delete_and_notebook_rename_keep_exact_paths_and_absent_sides() {
    let a = one("fn target() {}\n");
    let add = compare_notebook_sources("n.ipynb", None, "A", None, Some(&a)).unwrap();
    assert!(add.before.is_none());
    assert_eq!(add.symbol_changes.len(), 1);
    assert!(cell_event(&add, NotebookCellChangeKind::Added));
    let del = compare_notebook_sources("n.ipynb", None, "D", Some(&a), None).unwrap();
    assert!(del.after.is_none());
    assert_eq!(del.symbol_changes.len(), 1);
    assert!(cell_event(&del, NotebookCellChangeKind::Deleted));
    let rename =
        compare_notebook_sources("new.ipynb", Some("old.ipynb"), "R100", Some(&a), Some(&a))
            .unwrap();
    assert_eq!(rename.symbol_changes.len(), 1);
    let s = &rename.symbol_changes[0];
    assert_eq!(s.kind, NotebookSymbolChangeKind::Renamed);
    assert_eq!(
        s.before.as_ref().unwrap().source_ref.notebook_path,
        "old.ipynb"
    );
    assert_eq!(
        s.after.as_ref().unwrap().source_ref.notebook_path,
        "new.ipynb"
    );
}

#[test]
fn malformed_missing_sides_and_unsupported_changes_are_explicit() {
    let good = one("fn f() {}");
    for (kind, l, r) in [
        ("M", None, Some(good.as_slice())),
        ("M", Some(b"{".as_slice()), Some(good.as_slice())),
        ("A", Some(good.as_slice()), Some(good.as_slice())),
        ("D", Some(good.as_slice()), Some(good.as_slice())),
        ("R", Some(good.as_slice()), Some(good.as_slice())),
    ] {
        let d = compare_notebook_sources("n.ipynb", None, kind, l, r).unwrap();
        assert_eq!(d.status, NotebookStatus::Invalid, "{kind}");
        assert!(!d.diagnostics.is_empty());
        assert!(d.symbol_changes.is_empty());
    }
    let d = compare_notebook_sources("n.ipynb", None, "C100", Some(&good), Some(&good)).unwrap();
    assert_eq!(d.status, NotebookStatus::Partial);
    assert!(d
        .diagnostics
        .iter()
        .any(|e| e.code == SymbolDiagnosticCode::UnsupportedChangeKind));
    let d = extract_notebook_symbols(
        "n.ipynb",
        br#"{"cells":[{"id":"a","cell_type":"code","source":["valid",17]}]}"#,
    )
    .unwrap();
    assert_eq!(d.status, NotebookStatus::Invalid);
    assert!(d.cells[0].cell.source.is_none());
    let bad = notebook(vec![
        code("a", "rust", "fn f() {}"),
        code("a", "rust", "fn g() {}"),
    ]);
    let d = compare(&bad, &bad);
    assert_ne!(d.status, NotebookStatus::Complete);
    assert!(!d.diagnostics.is_empty());
}

#[test]
fn unsupported_commands_and_nested_notebook_language_are_partial() {
    for source in [
        ":load never-read.rs\nfn f() {}",
        ":dep serde = {\n version = \"1\"\n}\nfn f() {}",
    ] {
        let d = extract_notebook_symbols("n.ipynb", &one(source)).unwrap();
        assert_eq!(d.status, NotebookStatus::Partial);
        assert!(!d.diagnostics.is_empty());
    }
    let input = notebook(vec![code("nested", "ipynb", "{}")]);
    let d = extract_notebook_symbols("n.ipynb", &input).unwrap();
    assert_eq!(d.status, NotebookStatus::Partial);
}
