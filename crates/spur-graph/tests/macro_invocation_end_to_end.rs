use std::{collections::BTreeSet, fs, path::Path, sync::Arc, time::Instant};

use arrow_array::StringArray;
use parquet::{
    arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter},
    file::properties::WriterProperties,
};
use serde_json::{json, Value};
use spur_graph::{
    artifact_from_facts, artifact_from_facts_incremental, build_facts, git_blob_oid,
    macro_invocation_site_id,
    query_client::{GraphQueryClient, InMemoryClient, OverlayClient, ParquetClient},
    read_artifact_parquet, write_artifact_parquet, BuildMode, GraphIndexArtifact,
    MacroEvidenceAvailability as Availability, MacroEvidenceSourceScope, MacroExecutionPhase,
    MacroInvocationFilter as Filter, MacroInvocationQueryResult, MacroResolutionEvidence,
    MacroScopeKind, MacroTestContext, WriteOptions,
};

mod support;
use support::git_repo::GitRepo;

// Independent held-out source; PRE coordinates were enumerated from these exact bytes.
const SOURCE: &str = r##"#![allow(dead_code)]
use external::boom as explode;
#[cfg_attr(feature = "checks", allow(unused))]
mod diff {
    pub fn parse_raw_diff(input: &str) {
        if input.is_empty() { unreachable!("empty diff"); }
        debug_assert!(input.len() > 1);
        debug_assert!(input.len() > 1);
    }
}
fn aliases() { explode!(); }
macro_rules! panic { () => {}; }
fn shadowed() { panic!(); }
fn token_only() { wrapper!(panic!("nested")); stringify!(unreachable!()); helper(); }
fn helper() {}
#[cfg(feature = "schema")]
const C: () = item!();
#[cfg_attr(feature = "schema", allow(dead_code))]
type T = kind!();
fn evidence_scopes() { const { assert!(true); }; let _: [u8; size!()]; }
"##;
const SCOPES: &str = "#![allow(dead_code)]\nconst C: () = item!();\ntype T = kind!();\n";

fn fixture() -> GitRepo {
    let repo = GitRepo::new();
    repo.write("src/diff.rs", SOURCE);
    for file in ["keep", "remove", "rename"] {
        repo.write(&format!("src/{file}.rs"), SCOPES);
    }
    repo.write(
        "physical.ipynb#cell:real.rs",
        "fn physical() { real!(); }\n",
    );
    repo.write("empty.rs", "fn empty() {}\n");
    repo.write("note.ipynb", r#"{"cells":[{"cell_type":"code","id":"rust-cell.rs","metadata":{"language":"rust"},"source":["fn cell_helper() {}\nfn cell() { cell_helper(); panic!(); }"],"outputs":[],"execution_count":null}],"metadata":{"kernelspec":{"language":"rust","name":"rust","display_name":"Rust"}},"nbformat":4,"nbformat_minor":5}"#);
    repo.commit_all("held-out macro source");
    repo
}

fn full(root: &Path) -> (GraphIndexArtifact, Value) {
    let start = Instant::now();
    let (facts, _) = build_facts(root, None).unwrap();
    let extract = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let artifact = artifact_from_facts(&facts, root).unwrap();
    (
        artifact,
        json!({"extract_s":extract,"artifact_validation_s":start.elapsed().as_secs_f64()}),
    )
}

fn file(path: &str) -> Filter {
    Filter {
        file_path: Some(path.into()),
        ..Filter::default()
    }
}

fn caller(artifact: &GraphIndexArtifact) -> Filter {
    Filter {
        caller_stable_symbol_id: Some(
            artifact
                .symbols
                .iter()
                .find(|s| s.entity_name == "parse_raw_diff")
                .unwrap()
                .stable_symbol_id
                .clone(),
        ),
        ..file("src/diff.rs")
    }
}

fn record(name: &str, value: Value) {
    println!("M6_{name} {value}");
    if let Ok(dir) = std::env::var("M6_EVIDENCE_DIR") {
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            Path::new(&dir).join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

fn disk(artifact: &GraphIndexArtifact) -> (tempfile::TempDir, ParquetClient, Value) {
    let out = tempfile::tempdir().unwrap();
    let start = Instant::now();
    write_artifact_parquet(artifact, out.path(), WriteOptions::default(), vec![]).unwrap();
    let write_s = start.elapsed().as_secs_f64();
    let sizes: std::collections::BTreeMap<_, _> = fs::read_dir(out.path())
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                e.metadata().unwrap().len(),
            )
        })
        .collect();
    let client = ParquetClient::open(out.path()).unwrap();
    (out, client, json!({"write_s":write_s,"sizes_bytes":sizes}))
}

fn assert_scopes(result: &MacroInvocationQueryResult, root: &Path) {
    let required: BTreeSet<_> = result
        .rows
        .iter()
        .flat_map(|r| std::iter::once(&r.enclosing).chain(r.attributes.iter().map(|a| &a.owner)))
        .map(|s| &s.stable_symbol_id)
        .collect();
    assert_eq!(
        required,
        result
            .scopes
            .iter()
            .map(|s| &s.scope.stable_symbol_id)
            .collect()
    );
    for scope in &result.scopes {
        let bytes = fs::read(root.join(&scope.file_path)).unwrap();
        assert_eq!(scope.source_oid, git_blob_oid(&bytes));
        assert!(scope.byte_range[0] < scope.byte_range[1]);
        assert!(scope.byte_range[1] <= bytes.len());
    }
    for row in &result.rows {
        let text = fs::read_to_string(root.join(&row.file_path)).unwrap();
        assert_eq!(row.source_oid, git_blob_oid(text.as_bytes()));
        assert_eq!(
            row.site_id,
            macro_invocation_site_id(
                &row.file_path,
                &row.source_oid,
                row.byte_range,
                row.line_range
            )
        );
        assert!(text[row.byte_range[0]..row.byte_range[1]].contains('!'));
        for attr in &row.attributes {
            assert_eq!(&text[attr.byte_range[0]..attr.byte_range[1]], attr.raw_text);
            assert!(result.scopes.iter().any(|s| s.scope == attr.owner
                && s.file_path == row.file_path
                && s.source_oid == row.source_oid));
        }
    }
}

#[test]
fn held_out_caller_sites_scopes_and_unsupported_cells_round_trip() {
    let repo = fixture();
    let (artifact, cost) = full(repo.path());
    let (out, parquet, storage) = disk(&artifact);
    let round_trip = read_artifact_parquet(out.path()).unwrap();
    assert_eq!(round_trip.macro_invocations, artifact.macro_invocations);
    let memory = InMemoryClient::new(Arc::new(artifact.clone()));
    let filter = caller(&artifact);
    let start = Instant::now();
    let selected = parquet.macro_invocations(&filter).unwrap();
    let first_context_s = start.elapsed().as_secs_f64();
    let start = Instant::now();
    assert_eq!(parquet.macro_invocations(&filter).unwrap(), selected);
    let subsequent_query_s = start.elapsed().as_secs_f64();
    assert_eq!(memory.macro_invocations(&filter).unwrap(), selected);
    assert_eq!(selected.availability, Availability::Complete);
    assert_eq!(
        selected.source_scope,
        Some(MacroEvidenceSourceScope::PhysicalRustFiles)
    );
    assert_eq!(
        selected
            .rows
            .iter()
            .map(|r| (r.byte_range, r.line_range, r.debug_assertions_candidate))
            .collect::<Vec<_>>(),
        vec![
            ([181, 207], [6, 6], false),
            ([219, 249], [7, 7], true),
            ([259, 289], [8, 8], true)
        ]
    );
    assert_eq!(
        selected
            .rows
            .iter()
            .map(|r| &r.site_id)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    for row in &selected.rows {
        assert_eq!(row.test_context, MacroTestContext::Conditional);
        assert_eq!(row.execution_phase, MacroExecutionPhase::FunctionBodySyntax);
        assert_eq!(row.caller_stable_symbol_id, filter.caller_stable_symbol_id);
        assert!(row
            .attributes
            .iter()
            .any(|a| a.raw_text.contains("cfg_attr") && a.owner.kind == MacroScopeKind::Module));
    }
    // A scoped stable-ID join finds all three; no macro runtime callee is invented.
    let actual_caller = parquet
        .symbol_by_id(filter.caller_stable_symbol_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(actual_caller.entity_name, "parse_raw_diff");
    let callees = artifact
        .edges
        .iter()
        .filter(|e| {
            e.source_stable_symbol_id == actual_caller.stable_symbol_id
                && e.relation == spur_graph::RelationKind::Calls
        })
        .collect::<Vec<_>>();
    for ordinary in ["is_empty", "len"] {
        assert!(callees
            .iter()
            .any(|e| e.target_label.as_deref() == Some(ordinary)));
    }
    assert!(!callees.iter().any(|e| matches!(
        e.target_label.as_deref(),
        Some("unreachable" | "debug_assert")
    )));
    let all = parquet.macro_invocations(&file("src/diff.rs")).unwrap();
    assert_eq!(
        all.rows
            .iter()
            .map(|r| r.normalized_macro_path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "unreachable",
            "debug_assert",
            "debug_assert",
            "explode",
            "panic",
            "wrapper",
            "stringify",
            "item",
            "kind",
            "assert",
            "size"
        ]
    );
    assert!(matches!(
        all.rows
            .iter()
            .find(|r| r.normalized_macro_path == "explode")
            .unwrap()
            .resolution,
        MacroResolutionEvidence::Unresolved
    ));
    assert!(matches!(
        all.rows
            .iter()
            .find(|r| r.normalized_macro_path == "panic")
            .unwrap()
            .resolution,
        MacroResolutionEvidence::LocalDefinitionCandidate { .. }
    ));
    for kind in [MacroScopeKind::Const, MacroScopeKind::Type] {
        let row = all
            .rows
            .iter()
            .find(|r| {
                r.enclosing.kind == kind
                    && r.enclosing.stable_symbol_id.starts_with("macro-scope-v1:")
            })
            .unwrap();
        assert!(row.caller_stable_symbol_id.is_none());
        assert!(memory
            .symbol_by_id(&row.enclosing.stable_symbol_id)
            .unwrap()
            .is_none());
    }
    let start = Instant::now();
    assert_scopes(&all, repo.path());
    let scope_verification_io_hash_s = start.elapsed().as_secs_f64();
    let cell = artifact
        .symbols
        .iter()
        .find(|s| s.entity_name == "cell")
        .unwrap();
    assert!(
        !memory.find_callee_edges(&cell.stable_symbol_id).is_empty(),
        "notebook Rust calls remain supported"
    );
    for f in [
        file(&cell.file_path),
        Filter {
            caller_stable_symbol_id: Some(cell.stable_symbol_id.clone()),
            ..Filter::default()
        },
    ] {
        assert_eq!(
            parquet.macro_invocations(&f).unwrap().availability,
            Availability::UnsupportedSource
        );
        assert_eq!(
            memory.macro_invocations(&f).unwrap().availability,
            Availability::UnsupportedSource
        );
    }
    assert_eq!(
        parquet
            .macro_invocations(&file("physical.ipynb#cell:real.rs"))
            .unwrap()
            .rows
            .len(),
        1
    );
    let empty = parquet.macro_invocations(&file("empty.rs")).unwrap();
    assert_eq!(empty.availability, Availability::Complete);
    assert!(empty.rows.is_empty());
    let mut legacy = artifact.clone();
    legacy.macro_invocations = None;
    assert_eq!(
        InMemoryClient::new(Arc::new(legacy))
            .macro_invocations(&file("empty.rs"))
            .unwrap()
            .availability,
        Availability::Unavailable
    );
    record(
        "full",
        json!({"artifact":artifact,"selected":selected,"source":SOURCE,"cost":cost,"storage":storage,"first_context_s":first_context_s,"subsequent_query_s":subsequent_query_s,"scope_verification_io_hash_s":scope_verification_io_hash_s}),
    );
    // Optional exported artifacts let the README SQL be executed with DuckDB.
    if let Ok(dir) = std::env::var("M6_EVIDENCE_DIR") {
        let dest = Path::new(&dir).join("parquet");
        fs::create_dir_all(&dest).unwrap();
        for entry in fs::read_dir(out.path()).unwrap() {
            let e = entry.unwrap();
            fs::copy(e.path(), dest.join(e.file_name())).unwrap();
        }
    }
}

#[test]
fn incremental_delete_rename_move_and_reused_scopes_match_full_and_overlay() {
    let repo = fixture();
    let (base, _) = full(repo.path());
    let old = base.macro_invocations.as_ref().unwrap().clone();
    let (_base_out, base_disk, _) = disk(&base);
    let changed_source = SOURCE.replacen("        debug_assert!(input.len() > 1);\n", "", 1);
    repo.write("src/diff.rs", &changed_source);
    repo.remove("src/remove.rs");
    repo.git(&["mv", "src/rename.rs", "src/renamed.rs"]);
    repo.write("src/moved.rs", SCOPES);
    let start = Instant::now();
    let (incremental, mode, stats) = artifact_from_facts_incremental(&base, repo.path()).unwrap();
    let incremental_s = start.elapsed().as_secs_f64();
    assert_eq!(mode, BuildMode::Incremental);
    assert!(stats.reused_buckets > 0);
    let (oracle, full_cost) = full(repo.path());
    assert_eq!(incremental.macro_invocations, oracle.macro_invocations);
    let (out, persisted, storage) = disk(&incremental);
    assert_eq!(
        read_artifact_parquet(out.path()).unwrap().macro_invocations,
        oracle.macro_invocations
    );
    let root = repo.path().canonicalize().unwrap();
    let changed = [
        "src/diff.rs",
        "src/remove.rs",
        "src/rename.rs",
        "src/renamed.rs",
        "src/moved.rs",
    ]
    .map(|p| root.join(p));
    let overlay = OverlayClient::new(base_disk, repo.path(), &changed).unwrap();
    let memory = InMemoryClient::new(Arc::new(oracle.clone()));
    let mut scope_verification_io_hash_s = 0.0;
    for filter in [
        Filter::default(),
        caller(&oracle),
        file("src/keep.rs"),
        file("src/remove.rs"),
        file("src/rename.rs"),
        file("src/renamed.rs"),
        file("src/moved.rs"),
    ] {
        let expected = memory.macro_invocations(&filter).unwrap();
        assert_eq!(persisted.macro_invocations(&filter).unwrap(), expected);
        let actual = overlay.macro_invocations(&filter).unwrap();
        assert_eq!(actual.rows, expected.rows);
        assert_eq!(actual.scopes, expected.scopes);
        assert_eq!(actual.availability, expected.availability);
        let start = Instant::now();
        assert_scopes(&actual, repo.path());
        scope_verification_io_hash_s += start.elapsed().as_secs_f64();
    }
    let next = incremental.macro_invocations.as_ref().unwrap();
    assert!(!next
        .scopes
        .iter()
        .any(|s| ["src/remove.rs", "src/rename.rs"].contains(&s.file_path.as_str())));
    for scope in old.scopes.iter().filter(|s| s.file_path == "src/keep.rs") {
        assert!(next.scopes.contains(scope));
    }
    for row in old.rows.iter().filter(|r| r.file_path == "src/keep.rs") {
        assert!(next.rows.contains(row));
    }
    let selected = persisted.macro_invocations(&caller(&incremental)).unwrap();
    assert_eq!(selected.rows.len(), 2);
    assert!(selected
        .rows
        .iter()
        .all(|r| !old.rows.iter().any(|old| old.site_id == r.site_id)));
    assert!(persisted
        .macro_invocations(&Filter {
            source_oid: Some(git_blob_oid(SOURCE.as_bytes())),
            ..file("src/diff.rs")
        })
        .is_err());
    record(
        "incremental",
        json!({"artifact":incremental,"selected":selected,"source":changed_source,"incremental_s":incremental_s,"full_cost":full_cost,"storage":storage,"base":old,"overlay":overlay.macro_invocations(&Filter::default()).unwrap(),"scope_verification_io_hash_s":scope_verification_io_hash_s}),
    );
}

// Keep every logical row and schema/footer binding; change only physical groups.
fn regroup(path: &Path, rows_per_group: usize) -> usize {
    let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).unwrap()).unwrap();
    let schema = reader.schema().clone();
    let batches = reader
        .build()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let props = WriterProperties::builder()
        .set_max_row_group_row_count(Some(rows_per_group))
        .build();
    let mut writer =
        ArrowWriter::try_new(fs::File::create(path).unwrap(), schema, Some(props)).unwrap();
    for batch in batches {
        writer.write(&batch).unwrap();
    }
    writer.close().unwrap();
    ParquetRecordBatchReaderBuilder::try_new(fs::File::open(path).unwrap())
        .unwrap()
        .metadata()
        .num_row_groups()
}

#[test]
fn filtered_parquet_crosses_real_row_groups_and_matches_full_oracle() {
    let repo = fixture();
    let (artifact, _) = full(repo.path());
    let (out, _, _) = disk(&artifact);
    // Three required caller sites must span groups of at most two rows.
    let groups = regroup(&out.path().join("macro_invocations.parquet"), 2);
    let scope_groups = regroup(&out.path().join("macro_scopes.parquet"), 2);
    assert!(groups > 1 && scope_groups > 1);
    let filter = caller(&artifact);
    let mut matched_groups = 0;
    for group in 0..groups {
        let reader = ParquetRecordBatchReaderBuilder::try_new(
            fs::File::open(out.path().join("macro_invocations.parquet")).unwrap(),
        )
        .unwrap();
        let mut matched = false;
        for batch in reader.with_row_groups(vec![group]).build().unwrap() {
            let batch = batch.unwrap();
            let callers = batch
                .column_by_name("caller_stable_symbol_id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            matched |= callers
                .iter()
                .any(|id| id == filter.caller_stable_symbol_id.as_deref());
        }
        matched_groups += usize::from(matched);
    }
    assert!(
        matched_groups >= 2,
        "selected caller must cross an actual row-group boundary"
    );
    let client = ParquetClient::open(out.path()).unwrap();
    let oracle = InMemoryClient::new(Arc::new(artifact));
    for filter in [filter.clone(), file("src/keep.rs"), file("empty.rs")] {
        assert_eq!(
            client.macro_invocations(&filter).unwrap(),
            oracle.macro_invocations(&filter).unwrap()
        );
    }
    // Context has frozen both tables; subsequent filtered queries need no evidence disk IO.
    for table in ["macro_invocations.parquet", "macro_scopes.parquet"] {
        fs::remove_file(out.path().join(table)).unwrap();
    }
    assert_eq!(
        client.macro_invocations(&filter).unwrap(),
        oracle.macro_invocations(&filter).unwrap()
    );
    record(
        "row_groups",
        json!({"invocation_groups":groups,"scope_groups":scope_groups,"selected_groups":matched_groups,"rows_per_group":2}),
    );
}
