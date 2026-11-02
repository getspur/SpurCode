use std::fs;
use std::path::{Path, PathBuf};

use spur_graph::{
    artifact_from_facts, build_facts, resolve_artifact_location, write_current_pointer, Confidence,
    GraphEdgeArtifact, GraphEdgeKind, GraphIndexArtifact, GraphIndexPointer, GraphSymbolArtifact,
    RelationKind, ResolvedArtifact, SourceKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedChoice {
    Explicit,
    Current,
    Pointer,
    Missing,
}

#[derive(Debug)]
struct ResolverCase {
    explicit_override: bool,
    current: bool,
    pointer: bool,
}

#[test]
fn explicit_current_pointer() {
    assert_case(ResolverCase::new(true, true, true));
}

#[test]
fn explicit_current_no_pointer() {
    assert_case(ResolverCase::new(true, true, false));
}

#[test]
fn explicit_no_current_pointer() {
    assert_case(ResolverCase::new(true, false, true));
}

#[test]
fn explicit_no_current_no_pointer() {
    assert_case(ResolverCase::new(true, false, false));
}

#[test]
fn no_explicit_current_pointer() {
    assert_case(ResolverCase::new(false, true, true));
}

#[test]
fn no_explicit_current_no_pointer() {
    assert_case(ResolverCase::new(false, true, false));
}

#[test]
fn no_explicit_no_current_pointer() {
    assert_case(ResolverCase::new(false, false, true));
}

#[test]
fn no_explicit_no_current_no_pointer() {
    assert_case(ResolverCase::new(false, false, false));
}

#[test]
fn explicit_override_accepts_parquet_directory() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let worktree = tempdir.path();
    let parquet_dir = worktree.join("explicit.parquet");
    write_parquet_manifest(&parquet_dir, "explicit-parquet-hash");

    let resolved = resolve_artifact_location(worktree, Some(&parquet_dir))
        .expect("explicit parquet should resolve");

    assert_resolved(&resolved, &parquet_dir, "explicit-parquet-hash");
}

#[test]
fn bare_as_ref_call_does_not_bind_to_singleton_gitpath_trait_method() {
    let artifact = artifact_from_sources(&[
        (
            "crates/caller/src/lib.rs",
            r"
pub fn caller(bytes: &[u8]) {
    let _ = bytes.as_ref();
}
",
        ),
        (
            "crates/git-path/src/lib.rs",
            r"
pub struct GitPath(Vec<u8>);

impl AsRef<[u8]> for GitPath {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
",
        ),
    ]);
    let caller = symbol(&artifact, "caller");

    let edge = call_edge(&artifact, caller, "as_ref");

    assert_eq!(edge.target_stable_symbol_id, None);
    assert_eq!(edge.bind_method, None);
}

#[test]
fn explicitly_scoped_gitpath_as_ref_binds_to_matching_impl_scope() {
    let artifact = artifact_from_sources(&[(
        "src/lib.rs",
        r"
pub struct GitPath(Vec<u8>);

impl AsRef<[u8]> for GitPath {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub fn caller(path: &GitPath) -> &[u8] {
    <GitPath as AsRef<[u8]>>::as_ref(path)
}
",
    )]);
    let caller = symbol(&artifact, "caller");
    let as_ref = symbol(&artifact, "impl AsRef<[u8]> for GitPath::as_ref");

    let edge = call_edge(&artifact, caller, "as_ref");

    assert_eq!(
        edge.target_stable_symbol_id.as_deref(),
        Some(as_ref.stable_symbol_id.as_str())
    );
    assert_eq!(edge.bind_method.as_deref(), Some("scope_match"));
}

#[test]
fn typed_gitpath_receiver_as_ref_remains_unresolved_without_trait_authority() {
    let artifact = artifact_from_sources(&[(
        "src/lib.rs",
        r"
pub struct GitPath(Vec<u8>);

impl AsRef<[u8]> for GitPath {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub fn caller(path: &GitPath) -> &[u8] {
    path.as_ref()
}
",
    )]);
    let caller = symbol(&artifact, "caller");
    let edge = call_edge(&artifact, caller, "as_ref");
    assert_eq!(edge.target_stable_symbol_id, None);
    assert_eq!(edge.bind_method, None);
    assert_eq!(edge.scope_text.as_deref(), Some("GitPath"));
}

#[test]
fn uncertain_receiver_stays_unresolved_through_memory_parquet_and_incremental_rebind() {
    let source = r"
pub struct GitPath(Vec<u8>);
impl AsRef<[u8]> for GitPath { fn as_ref(&self)->&[u8] { &self.0 } }
impl GitPath { pub fn grounded(&self) {} }
pub fn caller(path: &GitPath) { let _ = path.as_ref(); path.grounded(); }
pub fn explicit(path: &GitPath) { let _ = <GitPath as AsRef<[u8]>>::as_ref(path); }
";
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), source).unwrap();
    fs::write(dir.path().join("src/other.rs"), "pub fn unrelated() {}\n").unwrap();
    let facts = build_facts(dir.path(), None).unwrap().0;
    let memory = facts.edges.iter().find(|e| e.relation == RelationKind::Calls && e.receiver_text.as_deref() == Some("path") && e.target_label.as_deref() == Some("as_ref")).unwrap();
    assert_eq!(memory.target_node_id, None);
    assert_eq!(memory.bind_method, None);
    let artifact = artifact_from_facts(&facts, dir.path()).unwrap();
    let storage = tempfile::tempdir().unwrap();
    spur_graph::write_artifact_parquet(&artifact, storage.path(), spur_graph::WriteOptions::default(), vec![]).unwrap();
    let persisted = spur_graph::load_artifact(storage.path()).unwrap();
    let (reused, _, _) = spur_graph::artifact_from_facts_incremental(&persisted, dir.path()).unwrap();
    fs::write(dir.path().join("src/other.rs"), "pub fn unrelated_changed() {}\n").unwrap();
    let (changed, _, _) = spur_graph::artifact_from_facts_incremental(&reused, dir.path()).unwrap();
    let mut exported = vec![];
    for (name, graph) in [("artifact", artifact), ("persisted", persisted), ("reused", reused), ("changed", changed)] {
        let caller = symbol(&graph, "caller");
        let uncertain = call_edge(&graph, caller, "as_ref");
        assert_eq!(uncertain.target_stable_symbol_id, None, "{name}");
        assert_eq!(uncertain.bind_method, None, "{name}");
        assert_eq!(uncertain.scope_text.as_deref(), Some("GitPath"));
        let grounded = call_edge(&graph, caller, "grounded");
        assert!(grounded.target_stable_symbol_id.is_some(), "{name}");
        assert_eq!(grounded.bind_method.as_deref(), Some("scope_match"));
        let explicit = call_edge(&graph, symbol(&graph,"explicit"), "as_ref");
        assert!(explicit.target_stable_symbol_id.is_some(), "{name}");
        exported.push(serde_json::json!({"path":name,"uncertain":uncertain,"grounded":grounded,"explicit":explicit}));
    }
    if let Some(path) = std::env::var_os("R8_EVIDENCE_DIR") {
        fs::create_dir_all(&path).unwrap();
        fs::write(Path::new(&path).join("dispatch-paths.json"), serde_json::to_vec_pretty(&exported).unwrap()).unwrap();
    }
}

#[test]
fn unresolved_receiver_preserves_generic_dynamic_and_nominal_scope_evidence() {
    let graph = artifact_from_sources(&[("src/lib.rs", r"
trait Runner { fn run(&self); }
struct Local;
fn generic(items: Option<i32>) { let _ = items.map(|x| x); }
fn dynamic(runner: &dyn Runner) { runner.run(); }
fn nominal(local: &Local) { local.unknown(); }
fn shadow(items: Option<i32>) { let items = unknown(); items.map(); }
fn unknown() {}
")]);
    let mut exported = vec![];
    for (owner, label, scope) in [("generic","map",Some("Option<i32>")), ("dynamic","run",Some("dyn Runner")), ("nominal","unknown",Some("Local")), ("shadow","map",None)] {
        let edge = call_edge(&graph, symbol(&graph,owner), label);
        assert_eq!(edge.scope_text.as_deref(), scope, "{owner}");
        assert_eq!(edge.target_stable_symbol_id, None);
        assert_eq!(edge.bind_method, None);
        if scope.is_some() { assert_eq!(edge.confidence, Confidence::Heuristic); assert_eq!(edge.confidence_score,0.8); }
        exported.push(serde_json::json!({"owner":owner,"edge":edge}));
    }
    if let Some(path) = std::env::var_os("R8_EVIDENCE_DIR") {
        fs::create_dir_all(&path).unwrap();
        fs::write(Path::new(&path).join("receiver-scopes.json"), serde_json::to_vec_pretty(&exported).unwrap()).unwrap();
    }
}

#[test]
fn bare_lock_call_does_not_bind_to_unrelated_singleton_method() {
    let artifact = artifact_from_sources(&[
        (
            "crates/caller/src/lib.rs",
            r"
use std::sync::Mutex;

pub fn caller(mutex: &Mutex<u8>) {
    let _guard = mutex.lock();
}
",
        ),
        (
            "crates/local-lock/src/lib.rs",
            r"
pub struct LocalLock;

impl LocalLock {
    pub fn lock(&self) {}
}
",
        ),
    ]);
    let caller = symbol(&artifact, "caller");

    let edge = call_edge(&artifact, caller, "lock");

    assert_eq!(edge.target_stable_symbol_id, None);
    assert_eq!(edge.bind_method, None);
}

#[test]
fn bare_free_function_singleton_still_resolves() {
    let artifact = artifact_from_sources(&[(
        "src/lib.rs",
        r"
pub fn caller() {
    helper();
}

fn helper() {}
",
    )]);
    let caller = symbol(&artifact, "caller");
    let helper = symbol(&artifact, "helper");

    let edge = call_edge(&artifact, caller, "helper");

    assert_eq!(
        edge.target_stable_symbol_id.as_deref(),
        Some(helper.stable_symbol_id.as_str())
    );
    assert_eq!(edge.bind_method.as_deref(), Some("singleton"));
}

#[test]
fn import_licensed_disambiguates_ambiguous_cross_crate_callable_call() {
    let artifact = artifact_from_sources(&[
        (
            "crates/crate-a/src/with_import.rs",
            r"
use crate_b::foo;

pub fn caller_with_import() {
    foo();
}
",
        ),
        (
            "crates/crate-a/src/without_import.rs",
            r"
pub fn caller_without_import() {
    foo();
}
",
        ),
        (
            "crates/crate-b/src/lib.rs",
            r"
pub fn foo() {}
",
        ),
        (
            "crates/crate-c/src/lib.rs",
            r"
pub struct Other;

impl Other {
    pub fn foo(&self) {}
}
",
        ),
    ]);
    let caller_with_import = symbol_in_file(
        &artifact,
        "crates/crate-a/src/with_import.rs",
        "caller_with_import",
        "function",
    );
    let caller_without_import = symbol_in_file(
        &artifact,
        "crates/crate-a/src/without_import.rs",
        "caller_without_import",
        "function",
    );
    let crate_b_foo = symbol_in_file(&artifact, "crates/crate-b/src/lib.rs", "foo", "function");

    let imported_call = call_edge(&artifact, caller_with_import, "foo");
    assert_eq!(
        imported_call.target_stable_symbol_id.as_deref(),
        Some(crate_b_foo.stable_symbol_id.as_str())
    );
    assert_eq!(
        imported_call.bind_method.as_deref(),
        Some("import_licensed")
    );

    let unlicensed_call = call_edge(&artifact, caller_without_import, "foo");
    assert_eq!(unlicensed_call.target_stable_symbol_id, None);
    assert_eq!(unlicensed_call.bind_method, None);
}

#[test]
fn import_licensed_binds_single_cross_crate_function_refused_by_singleton_guard() {
    let artifact = artifact_from_sources(&[
        (
            "crates/crate-a/src/lib.rs",
            r"
use crate_b::only;

pub fn caller() {
    only();
}
",
        ),
        (
            "crates/crate-b/src/lib.rs",
            r"
pub fn only() {}
",
        ),
    ]);
    let caller = symbol_in_file(&artifact, "crates/crate-a/src/lib.rs", "caller", "function");
    let only = symbol_in_file(&artifact, "crates/crate-b/src/lib.rs", "only", "function");

    let edge = call_edge(&artifact, caller, "only");

    assert_eq!(
        edge.target_stable_symbol_id.as_deref(),
        Some(only.stable_symbol_id.as_str())
    );
    assert_eq!(edge.bind_method.as_deref(), Some("import_licensed"));
}

#[test]
fn imported_type_does_not_license_cross_crate_method_call() {
    let artifact = artifact_from_sources(&[
        (
            "crates/crate-a/src/lib.rs",
            r"
use crate_b::Type;

pub fn caller(value: Unknown) {
    value.method();
}
",
        ),
        (
            "crates/crate-b/src/lib.rs",
            r"
pub struct Type;

impl Type {
    pub fn method(&self) {}
}
",
        ),
    ]);
    let caller = symbol_in_file(&artifact, "crates/crate-a/src/lib.rs", "caller", "function");

    let edge = call_edge(&artifact, caller, "method");

    assert_eq!(edge.target_stable_symbol_id, None);
    assert_eq!(edge.bind_method, None);
}

#[test]
fn typescript_imported_function_call_binds_by_import_path() {
    let artifact = artifact_from_sources(&[
        (
            "src/m.ts",
            r#"
export function helper() {}

export default function default_helper() {}
"#,
        ),
        (
            "src/app.ts",
            r#"
import { helper } from "./m";

export function caller() {
    helper();
}
"#,
        ),
    ]);

    let app = symbol_in_file(&artifact, "src/app.ts", "caller", "function");
    let helper = symbol_in_file(&artifact, "src/m.ts", "helper", "function");

    let helper_edge = call_edge(&artifact, app, "helper");
    assert_eq!(
        helper_edge.target_stable_symbol_id.as_deref(),
        Some(helper.stable_symbol_id.as_str())
    );
    assert_eq!(helper_edge.bind_method.as_deref(), Some("import_path"));
}

#[test]
fn typescript_react_frontend_imported_function_call_binds_by_import_path() {
    let artifact = artifact_from_sources(&[
        (
            "src/m.tsx",
            r#"
export function helper() {}

export default function default_helper() {}
"#,
        ),
        (
            "src/app.tsx",
            r#"
import { helper } from "./m";

export function caller() {
    return <div>{helper()}</div>;
}
"#,
        ),
    ]);

    let app = symbol_in_file(&artifact, "src/app.tsx", "caller", "function");
    let helper = symbol_in_file(&artifact, "src/m.tsx", "helper", "function");

    let helper_edge = call_edge(&artifact, app, "helper");
    assert_eq!(
        helper_edge.target_stable_symbol_id.as_deref(),
        Some(helper.stable_symbol_id.as_str())
    );
    assert_eq!(helper_edge.bind_method.as_deref(), Some("import_path"));
}

#[test]
fn cpp_qualified_call_edges_preserve_scope_and_receiver_evidence() {
    let artifact = artifact_from_sources(&[(
        "src/lib.cpp",
        r#"
namespace demo {
void helper() {}
}

struct Catalog {
    static void Load() {}
    void Touch() {}
};

void caller() {
    demo::helper();
    Catalog::Load();
    Catalog catalog;
    catalog.Touch();
}
"#,
    )]);
    let caller = symbol(&artifact, "caller");
    let helper = symbol(&artifact, "demo::helper");
    let load = symbol(&artifact, "Catalog::Load");

    let helper_edge = call_edge(&artifact, caller, "helper");
    assert_eq!(
        helper_edge.target_stable_symbol_id.as_deref(),
        Some(helper.stable_symbol_id.as_str())
    );
    assert_eq!(helper_edge.scope_text.as_deref(), Some("demo"));
    assert_eq!(helper_edge.receiver_text, None);
    assert_eq!(helper_edge.bind_method.as_deref(), Some("fqn"));

    let load_edge = call_edge(&artifact, caller, "Load");
    assert_eq!(
        load_edge.target_stable_symbol_id.as_deref(),
        Some(load.stable_symbol_id.as_str())
    );
    assert_eq!(load_edge.scope_text.as_deref(), Some("Catalog"));
    assert_eq!(load_edge.receiver_text, None);
    assert_eq!(load_edge.bind_method.as_deref(), Some("fqn"));

    let touch_edge = call_edge(&artifact, caller, "Touch");
    assert_eq!(touch_edge.receiver_text.as_deref(), Some("catalog"));
}

#[test]
fn cpp_qualified_call_bare_name_fallback_is_marked_heuristic() {
    let artifact = artifact_from_sources(&[(
        "src/lib.cpp",
        r#"
namespace alpha {
void lonely() {}
}

void caller() {
    beta::lonely();
}
"#,
    )]);
    let caller = symbol(&artifact, "caller");
    let lonely = symbol(&artifact, "alpha::lonely");

    let edge = call_edge(&artifact, caller, "lonely");

    assert_eq!(
        edge.target_stable_symbol_id.as_deref(),
        Some(lonely.stable_symbol_id.as_str())
    );
    assert_eq!(edge.scope_text.as_deref(), Some("beta"));
    assert_eq!(
        edge.bind_method.as_deref(),
        Some("bare_qualified_singleton")
    );
    assert_eq!(edge.confidence, Confidence::Heuristic);
    assert_eq!(edge.confidence_score, 0.8);
}

#[test]
fn cpp_ambiguous_qualified_call_keeps_scope_without_bare_resolution() {
    let artifact = artifact_from_sources(&[(
        "src/lib.cpp",
        r#"
namespace alpha {
void collide() {}
}

namespace beta {
void collide() {}
}

void caller() {
    gamma::collide();
}
"#,
    )]);
    let caller = symbol(&artifact, "caller");

    let edge = call_edge(&artifact, caller, "collide");

    assert_eq!(edge.target_stable_symbol_id, None);
    assert_eq!(edge.scope_text.as_deref(), Some("gamma"));
    assert_eq!(edge.bind_method, None);
    assert_eq!(edge.confidence, Confidence::Heuristic);
}

impl ResolverCase {
    fn new(explicit_override: bool, current: bool, pointer: bool) -> Self {
        Self {
            explicit_override,
            current,
            pointer,
        }
    }
}

fn assert_case(case: ResolverCase) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let worktree = tempdir.path();
    let paths = FixturePaths::new(worktree);

    if case.explicit_override {
        write_parquet_manifest(&paths.explicit, "explicit-hash");
    }
    if case.current {
        write_parquet_manifest(&paths.current, "current-hash");
        write_current_pointer(worktree, &paths.current).expect("write CURRENT");
    }
    if case.pointer {
        write_parquet_manifest(&paths.pointer, "pointer-hash");
        write_pointer_file(worktree, &paths.pointer);
    }

    let explicit = case.explicit_override.then_some(paths.explicit.as_path());
    let expected = expected_choice(case);
    let actual = resolve_artifact_location(worktree, explicit);

    match expected {
        ExpectedChoice::Explicit => {
            let resolved = actual.expect("explicit override should resolve");
            assert_resolved(&resolved, &paths.explicit, "explicit-hash");
        }
        ExpectedChoice::Current => {
            let resolved = actual.expect("CURRENT should resolve");
            assert_resolved(&resolved, &paths.current, "current-hash");
        }
        ExpectedChoice::Pointer => {
            let resolved = actual.expect("pointer file should resolve");
            assert_resolved(&resolved, &paths.pointer, "pointer-hash");
        }
        ExpectedChoice::Missing => {
            let err = actual.expect_err("missing artifacts should fail");
            assert!(
                err.to_string().contains("no valid spur graph artifact"),
                "unexpected error: {err:#}"
            );
        }
    }
}

fn expected_choice(case: ResolverCase) -> ExpectedChoice {
    if case.explicit_override {
        ExpectedChoice::Explicit
    } else if case.current {
        ExpectedChoice::Current
    } else if case.pointer {
        ExpectedChoice::Pointer
    } else {
        ExpectedChoice::Missing
    }
}

fn assert_resolved(resolved: &ResolvedArtifact, expected_path: &Path, expected_parquet_hash: &str) {
    assert_eq!(resolved.path, canonicalize(expected_path));
    assert_eq!(resolved.cache_key.graph_content_hash, expected_parquet_hash);
}

struct FixturePaths {
    explicit: PathBuf,
    current: PathBuf,
    pointer: PathBuf,
}

impl FixturePaths {
    fn new(worktree: &Path) -> Self {
        Self {
            explicit: worktree.join("artifacts/explicit.parquet"),
            current: worktree.join("artifacts/current.parquet"),
            pointer: worktree.join("artifacts/pointer.parquet"),
        }
    }
}

fn write_parquet_manifest(path: &Path, graph_content_hash: &str) {
    fs::create_dir_all(path).expect("mkdir parquet dir");
    fs::write(
        path.join("manifest.json"),
        format!(
            r#"{{
  "graph_index_version": "spur-graph-phase2",
  "schema_version": "spur-graph-schema-v6",
  "manifest_version": "manifest-test",
  "graph_content_hash": "{graph_content_hash}",
  "indexed_commit_oid": null,
  "extractor_version": "test-extractor",
  "complete": true,
  "row_counts": {{
    "nodes": 0,
    "edges": 0,
    "edges_by_dst": null,
    "edges_unresolved": 0,
    "files": 0,
    "file_manifests": 0,
    "tombstones": 0
  }},
  "parquet_writer": {{
    "compression": "zstd-3",
    "row_group_size": 16384
  }},
  "edges_by_dst_present": false
}}"#
        ),
    )
    .expect("write parquet manifest");
}

fn write_pointer_file(worktree: &Path, canonical_artifact_path: &Path) {
    let pointer_path = worktree.join(".spur/graph-index.pointer.json");
    fs::create_dir_all(pointer_path.parent().expect("pointer parent")).expect("mkdir .spur");
    let pointer = GraphIndexPointer {
        schema: "spur-graph-pointer-v1".to_owned(),
        graph_content_hash: "pointer-hash".to_owned(),
        manifest_version: "manifest-test".to_owned(),
        source_kind: SourceKind::Git,
        indexed_commit_oid: Some("test-head".to_owned()),
        canonical_artifact_path: canonical_artifact_path.to_path_buf(),
    };
    fs::write(
        pointer_path,
        serde_json::to_vec_pretty(&pointer).expect("encode pointer"),
    )
    .expect("write pointer");
}

fn canonicalize(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|err| panic!("canonicalize `{}`: {err}", path.display()))
}

fn artifact_from_sources(files: &[(&str, &str)]) -> GraphIndexArtifact {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    for (relative_path, source) in files {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("source parent")).expect("mkdir source parent");
        fs::write(&path, source).unwrap_or_else(|err| {
            panic!("write `{}`: {err}", path.display());
        });
    }
    let (facts, _counts) = build_facts(root, None).expect("build facts");
    artifact_from_facts(&facts, root).expect("build artifact")
}

fn symbol<'a>(artifact: &'a GraphIndexArtifact, qualified_name: &str) -> &'a GraphSymbolArtifact {
    artifact
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == qualified_name)
        .unwrap_or_else(|| {
            let names = artifact
                .symbols
                .iter()
                .map(|symbol| symbol.qualified_name.as_str())
                .collect::<Vec<_>>();
            panic!("missing symbol `{qualified_name}` in {names:?}");
        })
}

fn symbol_in_file<'a>(
    artifact: &'a GraphIndexArtifact,
    file_path: &str,
    entity_name: &str,
    symbol_kind: &str,
) -> &'a GraphSymbolArtifact {
    artifact
        .symbols
        .iter()
        .find(|symbol| {
            symbol.file_path == file_path
                && symbol.entity_name == entity_name
                && symbol.symbol_kind == symbol_kind
        })
        .unwrap_or_else(|| {
            let symbols = artifact
                .symbols
                .iter()
                .map(|symbol| {
                    (
                        symbol.file_path.as_str(),
                        symbol.entity_name.as_str(),
                        symbol.symbol_kind.as_str(),
                        symbol.qualified_name.as_str(),
                    )
                })
                .collect::<Vec<_>>();
            panic!("missing {symbol_kind} `{entity_name}` in `{file_path}` from {symbols:?}");
        })
}

fn call_edge<'a>(
    artifact: &'a GraphIndexArtifact,
    source: &GraphSymbolArtifact,
    target_label: &str,
) -> &'a GraphEdgeArtifact {
    artifact
        .edges
        .iter()
        .find(|edge| {
            edge.source_stable_symbol_id == source.stable_symbol_id
                && edge.relation == RelationKind::Calls
                && edge.edge_kind == Some(GraphEdgeKind::Calls)
                && edge.target_label.as_deref() == Some(target_label)
        })
        .unwrap_or_else(|| {
            let edges = artifact
                .edges
                .iter()
                .filter(|edge| edge.source_stable_symbol_id == source.stable_symbol_id)
                .map(|edge| {
                    (
                        edge.target_label.as_deref(),
                        edge.target_stable_symbol_id.as_deref(),
                        edge.relation,
                        edge.edge_kind,
                    )
                })
                .collect::<Vec<_>>();
            panic!(
                "missing call edge `{target_label}` from `{}` in {edges:?}",
                source.qualified_name
            );
        })
}
