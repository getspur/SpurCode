use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{json, Value};
use spur_analyst::mcp::{
    local_project_tool_definitions, tool_definitions, AnalystMcpModule, McpHandlerError,
};
use spur_graph::store::lance_sections::write_sections_dataset_skipping_embeddings;
use spur_graph::{
    artifact_from_facts, build_facts, write_artifact_parquet, write_current_pointer, WriteOptions,
};
use spur_mcp::local_projects::{
    LocalProjectCatalogStore, LocalProjectError, LocalProjectHealth, LocalProjectResolver,
    LocalProjectValidator, ValidatedLocalProject,
};

const INIT_SEARCH_SQL: &str = include_str!("../../../spur-context/analyst/init_search.sql");
const EMBED_MODE_ENV: &str = "SPUR_ANALYST_EMBED_MODE";
const EXPECTED_TOOL_NAMES: &[&str] = &[
    "doc_navigate",
    "knowledge_context_pack",
    "knowledge_context_pack_2",
    "query",
];
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct AnalystFixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    db_path: PathBuf,
}

struct ProjectAnalystFixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl ProjectAnalystFixture {
    fn new(label: &str) -> Self {
        let dir = tempfile::tempdir().expect("project tempdir");
        let root = dir.path().join("repo");
        fs::create_dir_all(root.join(".spur")).expect("create .spur");
        fs::create_dir_all(root.join("docs")).expect("create docs");
        fs::write(
            root.join("docs/project.md"),
            format!("# {label} Guide\n\n{label} navigation evidence.\n"),
        )
        .expect("write project doc");
        let db_path = root.join(".spur/analyst.duckdb");
        seed_analyst_db(&db_path);
        let (facts, _) = build_facts(&root, None).expect("build graph facts");
        let artifact = artifact_from_facts(&facts, &root).expect("build graph artifact");
        let artifact_dir = write_artifact_parquet(
            &artifact,
            &root.join(".spur/graph"),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write graph artifact");
        write_sections_dataset_skipping_embeddings(&artifact, &root, &artifact_dir)
            .expect("write sections dataset");
        write_current_pointer(&root, &artifact_dir).expect("write graph pointer");
        Self { _dir: dir, root }
    }
}

#[derive(Clone, Default)]
struct FixtureValidator;

impl LocalProjectValidator for FixtureValidator {
    fn validate(&self, requested_path: &Path) -> Result<ValidatedLocalProject, LocalProjectError> {
        let canonical_root =
            requested_path
                .canonicalize()
                .map_err(|error| LocalProjectError::InvalidPath {
                    path: requested_path.to_path_buf(),
                    reason: error.to_string(),
                })?;
        Ok(ValidatedLocalProject {
            canonical_root,
            health: LocalProjectHealth::ready(),
        })
    }
}

impl AnalystFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repo");
        fs::create_dir_all(root.join(".spur")).expect("create .spur");
        let db_path = root.join(".spur").join("analyst.duckdb");
        seed_analyst_db(&db_path);
        Self {
            _dir: dir,
            root,
            db_path,
        }
    }
}

#[cfg(feature = "embed")]
#[tokio::test]
async fn bounded_pack_keeps_completed_response_after_cleanup_failure() {
    const CHILD: &str = "SPUR_S6_PACK_RESULT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .try_init()
            .unwrap();
        let fixture = AnalystFixture::new();
        let pack = dispatch_in_fixture(
            &fixture,
            "knowledge_context_pack",
            json!({"query": "dispatch approval evidence"}),
        )
        .await
        .expect("completed pack must survive cleanup failure");
        assert!(pack.get("error").is_none(), "{pack:#}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/embedding/fixtures/owned_fault_worker.py");
    let config = json!({
        "executable": "/usr/bin/python3", "args": [script], "artifact_budget": 10000,
        "limits": {"max_frame_bytes": 65536, "max_texts": 2, "max_text_bytes": 8192,
            "max_total_text_bytes": 16384, "max_dimensions": 768, "max_identity_bytes": 256, "max_pending": 1},
        "queue_capacity": 1, "max_restarts": 0, "startup_timeout_ms": 10000,
        "request_timeout_ms": 10000, "shutdown_timeout_ms": 10000, "shutdown_poll_interval_ms": 10
    });
    let log = dir.path().join("worker.log");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "mcp_tools_characterization::bounded_pack_keeps_completed_response_after_cleanup_failure", "--nocapture"])
        .env(CHILD, "1").env("SPUR_GRAPH_TEST_WORKER_LOG", &log)
        .env("SPUR_GRAPH_TEST_WORKER_FAULT", "shutdown_abort").env("SPUR_ANALYST_EMBED_MODE", "inprocess")
        .env("SPUR_EMBEDDING_HELPER_CONFIG", config.to_string()).output().unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("embedding compatibility shutdown failed"),
        "cleanup error was not reported: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = std::fs::read_to_string(log).unwrap();
    assert!(events.lines().any(|line| line == "shutdown"), "{events}");
    let pid = events
        .lines()
        .find_map(|line| line.strip_prefix("service_pid "))
        .unwrap();
    let probe = std::process::Command::new("/usr/bin/python3").args(["-c", "import os,sys\ntry: os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError: sys.exit(0)\nelse: sys.exit(1)", pid]).status().unwrap();
    assert!(probe.success(), "helper still exists: {events}");
}

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self {
            _lock: lock,
            key,
            previous,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn knowledge_context_pack_2_schema_defaults_compact_format_and_zero_bodies() {
    let tool = tool_definitions()
        .into_iter()
        .find(|tool| tool.name == "knowledge_context_pack_2")
        .expect("kcp2 tool");
    assert_eq!(
        tool.input_schema["properties"]["response_format"]["default"],
        json!("compact")
    );
    assert_eq!(
        tool.input_schema["properties"]["response_format"]["enum"],
        json!(["full", "compact"])
    );
    assert_eq!(
        tool.input_schema["properties"]["max_symbol_bodies"]["default"],
        json!(0)
    );
}

#[test]
fn knowledge_context_pack_schemas_describe_optional_symbol_body_limit() {
    for name in ["knowledge_context_pack", "knowledge_context_pack_2"] {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool.name == name)
            .expect("context pack tool");
        let max_symbol_bodies = &tool.input_schema["properties"]["max_symbol_bodies"];

        assert_eq!(
            max_symbol_bodies["description"],
            json!("Optional maximum number of symbol bodies to include. Defaults to 0; accepted range is 0..=5."),
            "tool={name}"
        );
        assert!(!tool.input_schema["required"]
            .as_array()
            .expect("required fields")
            .iter()
            .any(|field| field.as_str() == Some("max_symbol_bodies")));
    }
}

#[test]
fn analyst_mcp_module_advertises_exact_public_tool_names() {
    let module = AnalystMcpModule::new();
    let names = module
        .tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert_eq!(names, EXPECTED_TOOL_NAMES);
}

#[test]
fn analyst_project_routing_is_opt_in_for_all_four_schemas() {
    for module in [AnalystMcpModule::new(), AnalystMcpModule::read_only()] {
        assert_eq!(
            serde_json::to_value(module.tools()).expect("serialize module tools"),
            serde_json::to_value(tool_definitions()).expect("serialize default tools")
        );
        assert!(module
            .tools()
            .iter()
            .all(|tool| tool.input_schema.pointer("/properties/project").is_none()));
    }

    let catalog = tempfile::tempdir().expect("catalog tempdir");
    let resolver = LocalProjectResolver::new(
        LocalProjectCatalogStore::new(catalog.path().join("projects.toml")),
        Arc::new(FixtureValidator),
    );
    let tools = AnalystMcpModule::with_local_projects(resolver).tools();
    assert_eq!(
        serde_json::to_value(&tools).expect("serialize routed tools"),
        serde_json::to_value(local_project_tool_definitions()).expect("serialize routed defs")
    );
    assert_eq!(tools.len(), 4);
    assert!(tools.iter().all(|tool| {
        tool.input_schema.pointer("/properties/project/type") == Some(&json!("string"))
    }));
}

#[test]
fn doc_navigate_is_split_into_application_modules_and_thin_mcp_adapter() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let doc_nav = src.join("doc_nav");

    for module in ["mod.rs", "artifact.rs", "query.rs", "projection.rs"] {
        let path = doc_nav.join(module);
        assert!(path.is_file(), "missing doc_nav module {}", path.display());
        assert!(
            line_count(&path) < 300,
            "{} should stay below 300 lines",
            path.display()
        );
    }

    let adapter = src.join("mcp").join("tools").join("doc_navigate.rs");
    assert!(
        adapter.is_file(),
        "missing thin MCP adapter {}",
        adapter.display()
    );
    assert!(
        line_count(&adapter) <= 80,
        "{} should remain a thin adapter",
        adapter.display()
    );

    let old_module = src.join("mcp").join("doc_navigate.rs");
    assert!(
        !old_module.exists(),
        "{} should move into doc_nav/* plus mcp/tools/doc_navigate.rs",
        old_module.display()
    );
}

#[test]
fn query_is_split_into_thin_tool_adapter_and_arrow_value_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let adapter = src.join("mcp").join("tools").join("query.rs");
    let arrow_values = src.join("mcp").join("value").join("arrow.rs");

    for path in [&adapter, &arrow_values] {
        assert!(
            path.is_file(),
            "missing query split module {}",
            path.display()
        );
        assert!(
            line_count(path) < 260,
            "{} should stay below 260 lines",
            path.display()
        );
    }

    let old_module = src.join("mcp").join("query.rs");
    assert!(
        !old_module.exists(),
        "{} should move into mcp/tools/query.rs plus mcp/value/arrow.rs",
        old_module.display()
    );
}

#[test]
fn knowledge_context_is_split_into_pack_service_and_thin_mcp_adapter() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let adapter = src.join("mcp").join("tools").join("knowledge_context.rs");
    let service = src.join("pack").join("service.rs");

    for path in [&adapter, &service] {
        assert!(
            path.is_file(),
            "missing knowledge context split module {}",
            path.display()
        );
    }

    let adapter_source = fs::read_to_string(&adapter)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", adapter.display()));
    assert!(
        adapter_source.contains("service::knowledge_context_pack(request).await")
            && adapter_source.contains("service::knowledge_context_pack_2(request).await"),
        "{} should delegate parsed requests to pack::service",
        adapter.display()
    );
    assert!(
        !adapter_source.contains("EmbeddingRuntime")
            && !adapter_source.contains("query_context_candidates"),
        "{} should stay a thin MCP adapter",
        adapter.display()
    );
    assert!(
        rust_symbolish_count(&adapter) < 90,
        "{} should remain under the 90-symbol adapter budget",
        adapter.display()
    );
    assert!(
        rust_symbolish_count(&service) < 260,
        "{} should remain under the 260-symbol service budget",
        service.display()
    );

    let old_module = src.join("mcp").join("knowledge_context.rs");
    assert!(
        !old_module.exists(),
        "{} should move into mcp/tools/knowledge_context.rs plus pack/service.rs",
        old_module.display()
    );
}

#[tokio::test]
async fn analyst_mcp_dispatch_keeps_all_public_tool_names_reachable() {
    let module = AnalystMcpModule::new();

    for tool_name in EXPECTED_TOOL_NAMES {
        let error = module
            .dispatch(tool_name, json!({}))
            .await
            .expect_err("empty args should fail inside the routed tool");
        assert!(
            !is_unknown_tool_error(&error),
            "{tool_name} should route to its handler, got {error:?}"
        );
    }

    let unknown = module
        .dispatch("__missing_tool__", json!({}))
        .await
        .expect_err("unknown tool should fail");
    assert!(is_unknown_tool_error(&unknown));
}

#[tokio::test]
async fn knowledge_context_pack_v1_response_shape_matches_snapshot() {
    let _embed_mode = EnvGuard::set(EMBED_MODE_ENV, "off");
    let fixture = AnalystFixture::new();
    let pack = dispatch_in_fixture(
        &fixture,
        "knowledge_context_pack",
        json!({
            "query": "dispatch approval evidence",
            "intent": "review",
            "scope": "all",
            "limit": 5,
            "max_symbol_bodies": 0
        }),
    )
    .await
    .expect("v1 fixture response");

    assert!(pack.get("error").is_none(), "{pack:#}");
    assert_json_snapshot(
        "knowledge_context_pack_v1_shape",
        &normalize_pack_snapshot(pack, &fixture.db_path),
    );
}

#[tokio::test]
async fn knowledge_context_pack_v2_response_shape_matches_snapshot() {
    let _embed_mode = EnvGuard::set(EMBED_MODE_ENV, "off");
    let fixture = AnalystFixture::new();
    let pack = dispatch_in_fixture(
        &fixture,
        "knowledge_context_pack_2",
        json!({
            "query": "dispatch approval evidence",
            "intent": "review",
            "scope": "all",
            "limit": 5,
            "max_symbol_bodies": 0,
            "response_format": "full",
            "graph_reasoning": {
                "paths": true,
                "communities": true,
                "risk": true,
                "max_path_hops": 2,
                "max_paths": 1
            }
        }),
    )
    .await
    .expect("v2 fixture response");

    assert!(pack.get("error").is_none(), "{pack:#}");
    assert_json_snapshot(
        "knowledge_context_pack_v2_shape",
        &normalize_pack_snapshot(pack, &fixture.db_path),
    );
}

#[tokio::test]
async fn selected_project_scopes_doc_navigation_and_both_knowledge_pack_names() {
    let _embed_mode = EnvGuard::set(EMBED_MODE_ENV, "off");
    let fixture = ProjectAnalystFixture::new("alpha_unique");
    let catalog = tempfile::tempdir().expect("catalog tempdir");
    let store = LocalProjectCatalogStore::new(catalog.path().join("projects.toml"));
    store
        .add("alpha", &fixture.root, false)
        .expect("register project");
    let module = AnalystMcpModule::with_local_projects(LocalProjectResolver::new(
        store,
        Arc::new(FixtureValidator),
    ));

    let docs = module
        .dispatch(
            "doc_navigate",
            json!({"query": "alpha_unique", "project": "alpha"}),
        )
        .await
        .expect("project doc navigation");
    assert_eq!(docs["project"]["name"], "alpha");
    assert!(
        docs["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
        "{docs:#}"
    );

    for tool_name in ["knowledge_context_pack", "knowledge_context_pack_2"] {
        let pack = module
            .dispatch(
                tool_name,
                json!({
                    "query": "dispatch approval evidence",
                    "intent": "review",
                    "scope": "all",
                    "limit": 5,
                    "max_symbol_bodies": 0,
                    "project": "alpha"
                }),
            )
            .await
            .expect("project knowledge pack");
        assert_eq!(pack["project"]["name"], "alpha");
        assert_project_on_pack_suggestions(&pack, "alpha");
    }
}

#[tokio::test]
async fn empty_project_pack_suppresses_uncallable_project_followups() {
    let _embed_mode = EnvGuard::set(EMBED_MODE_ENV, "off");
    let fixture = ProjectAnalystFixture::new("alpha_unique");
    let catalog = tempfile::tempdir().expect("catalog tempdir");
    let store = LocalProjectCatalogStore::new(catalog.path().join("projects.toml"));
    store
        .add("alpha", &fixture.root, false)
        .expect("register project");
    let module = AnalystMcpModule::with_local_projects(LocalProjectResolver::new(
        store,
        Arc::new(FixtureValidator),
    ));

    let pack = module
        .dispatch(
            "knowledge_context_pack_2",
            json!({
                "query": "definitely_missing_project_symbol_xyz",
                "intent": "explain",
                "scope": "code",
                "limit": 5,
                "max_symbol_bodies": 0,
                "project": "alpha"
            }),
        )
        .await
        .expect("empty project knowledge pack");

    assert_eq!(pack["project"]["name"], "alpha");
    assert_eq!(pack["answerable"], false, "{pack:#}");
    let recommendations = pack["recommended_next_tools"]
        .as_array()
        .expect("recommended next tools");
    assert!(
        recommendations
            .iter()
            .all(|suggestion| suggestion["tool"] != "code_semantic_search"),
        "uncallable fallback leaked into project response: {pack:#}"
    );
    assert!(
        recommendations
            .iter()
            .all(|suggestion| suggestion["project"] == "alpha"),
        "project-aware fallback lost scope: {pack:#}"
    );
}

#[tokio::test]
async fn analyst_only_project_packs_emit_only_same_surface_followups() {
    let _embed_mode = EnvGuard::set(EMBED_MODE_ENV, "off");
    let fixture = ProjectAnalystFixture::new("alpha_unique");
    let catalog = tempfile::tempdir().expect("catalog tempdir");
    let store = LocalProjectCatalogStore::new(catalog.path().join("projects.toml"));
    store
        .add("alpha", &fixture.root, false)
        .expect("register project");
    let resolver = LocalProjectResolver::new(store, Arc::new(FixtureValidator));
    let analyst_only = AnalystMcpModule::with_local_projects_for_analyst_server(resolver.clone());
    let graph_enabled = AnalystMcpModule::with_local_projects(resolver);
    let args = json!({
        "query": "dispatch approval evidence",
        "intent": "review",
        "scope": "all",
        "limit": 5,
        "max_symbol_bodies": 0,
        "project": "alpha"
    });

    let analyst_pack = analyst_only
        .dispatch("knowledge_context_pack_2", args.clone())
        .await
        .expect("analyst-only project pack");
    for suggestion in project_pack_suggestions(&analyst_pack) {
        let tool = suggestion["tool"].as_str().expect("suggested tool name");
        assert!(
            EXPECTED_TOOL_NAMES.contains(&tool),
            "analyst-only server emitted unsupported follow-up `{tool}`: {analyst_pack:#}"
        );
        assert_eq!(suggestion["project"], "alpha", "{analyst_pack:#}");
    }

    let graph_enabled_pack = graph_enabled
        .dispatch("knowledge_context_pack_2", args)
        .await
        .expect("graph-enabled project pack");
    assert!(
        project_pack_suggestions(&graph_enabled_pack)
            .iter()
            .any(|suggestion| suggestion["tool"]
                .as_str()
                .is_some_and(|tool| tool.starts_with("code_"))),
        "bundled/brain policy must preserve callable graph follow-ups: {graph_enabled_pack:#}"
    );
}

fn project_pack_suggestions(pack: &Value) -> Vec<&Value> {
    let mut suggestions = ["next", "recommended_next_tools"]
        .into_iter()
        .filter_map(|key| pack[key].as_array())
        .flatten()
        .collect::<Vec<_>>();
    suggestions.extend(
        ["primary_evidence", "supporting_docs"]
            .into_iter()
            .filter_map(|key| pack[key].as_array())
            .flatten()
            .flat_map(|evidence| {
                ["next", "recommended_next_tools"]
                    .into_iter()
                    .filter_map(|key| evidence[key].as_array())
                    .flatten()
            }),
    );
    suggestions
}

fn assert_project_on_pack_suggestions(pack: &Value, expected: &str) {
    let recommended = pack["recommended_next_tools"]
        .as_array()
        .expect("recommended next tools");
    assert!(!recommended.is_empty(), "{pack:#}");
    assert!(
        recommended
            .iter()
            .all(|suggestion| suggestion["project"] == expected),
        "{pack:#}"
    );

    let nested = ["primary_evidence", "supporting_docs"]
        .into_iter()
        .filter_map(|key| pack[key].as_array())
        .flatten()
        .filter_map(|evidence| evidence["next"].as_array())
        .flatten()
        .collect::<Vec<_>>();
    assert!(!nested.is_empty(), "{pack:#}");
    assert!(
        nested
            .iter()
            .all(|suggestion| suggestion["project"] == expected),
        "{pack:#}"
    );
}

async fn dispatch_in_fixture(
    fixture: &AnalystFixture,
    tool_name: &'static str,
    args: Value,
) -> Result<Value, McpHandlerError> {
    let module = AnalystMcpModule::new();
    let root = fixture.root.clone();
    spur_graph::mcp::with_worktree_root_for_request(root, async move {
        module.dispatch(tool_name, args).await
    })
    .await
}

fn is_unknown_tool_error(error: &McpHandlerError) -> bool {
    matches!(error, McpHandlerError::InvalidParams(message) if message.contains("unknown analyst MCP tool"))
}

fn assert_json_snapshot(name: &str, actual: &Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join(format!("{name}.json"));
    let actual = format!(
        "{}\n",
        serde_json::to_string_pretty(actual).expect("snapshot JSON should serialize")
    );
    let expected = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "failed to read snapshot {}: {error}\nactual snapshot:\n{actual}",
            path.display()
        );
    });
    assert_eq!(expected, actual, "snapshot mismatch for {}", path.display());
}

fn line_count(path: &Path) -> usize {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .lines()
        .count()
}

fn rust_symbolish_count(path: &Path) -> usize {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    source
        .lines()
        .map(str::trim_start)
        .filter(|line| {
            [
                "fn ",
                "async fn ",
                "pub fn ",
                "pub async fn ",
                "pub(crate) fn ",
                "pub(crate) async fn ",
                "struct ",
                "pub struct ",
                "enum ",
                "pub enum ",
                "impl ",
                "const ",
                "pub const ",
                "mod ",
                "pub(crate) mod ",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        })
        .count()
}

fn normalize_pack_snapshot(mut value: Value, db_path: &Path) -> Value {
    replace_string_value(
        &mut value,
        &db_path.display().to_string(),
        "<fixture-analyst-db>",
    );
    value
}

fn replace_string_value(value: &mut Value, needle: &str, replacement: &str) {
    match value {
        Value::String(string) if string == needle => {
            *string = replacement.to_owned();
        }
        Value::Array(values) => {
            for value in values {
                replace_string_value(value, needle, replacement);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                replace_string_value(value, needle, replacement);
            }
        }
        _ => {}
    }
}

fn seed_analyst_db(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open fixture db");
    if let Ok(directory) = std::env::var("SPUR_DUCKDB_EXTENSION_DIR") {
        let escaped = directory.replace('\'', "''");
        conn.execute_batch(&format!("SET extension_directory = '{escaped}'"))
            .expect("set isolated fixture extension directory");
    }
    conn.execute_batch("INSTALL fts; LOAD fts; INSTALL icu; LOAD icu;")
        .expect("load fixture extensions");
    conn.execute_batch(
        r"
        CREATE TABLE _meta (graph_content_hash VARCHAR);
        INSERT INTO _meta VALUES ('kcp-fixture-hash');

        CREATE TABLE sections_search (
            stable_symbol_id VARCHAR,
            qualified_name VARCHAR,
            file_path VARCHAR,
            heading_level INTEGER,
            content_hash VARCHAR,
            body_text VARCHAR,
            embedding FLOAT[768]
        );
        INSERT INTO sections_search (
            stable_symbol_id, qualified_name, file_path, heading_level, content_hash, body_text
        ) VALUES
            ('doc-dispatch', 'Dispatch Approval Reading Path', 'docs/dispatch.md', 2, 'doc-hash',
             'dispatch approval evidence reading path');

        CREATE TABLE symbol_text (
            stable_symbol_id VARCHAR,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            file_path VARCHAR,
            symbol_kind VARCHAR,
            doc_text VARCHAR,
            embedding FLOAT[768]
        );
        INSERT INTO symbol_text (
            stable_symbol_id, entity_name, qualified_name, file_path, symbol_kind, doc_text
        ) VALUES
            ('sym-dispatch', 'dispatch_plan', 'fixture::dispatch_plan',
             'src/dispatch.rs', 'function', 'dispatch approval evidence entry point'),
            ('sym-review', 'review_approval', 'fixture::review_approval',
             'src/review.rs', 'function', 'dispatch approval evidence review path');

        CREATE TABLE v_symbol_scorecard (
            stable_symbol_id VARCHAR,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            symbol_kind VARCHAR,
            file_path VARCHAR,
            pagerank DOUBLE,
            in_degree BIGINT,
            out_degree BIGINT,
            callers BIGINT,
            importers BIGINT,
            inbound_total BIGINT,
            churn_90d BIGINT,
            last_touched TIMESTAMP,
            blast_radius_score DOUBLE,
            posture VARCHAR
        );
        INSERT INTO v_symbol_scorecard VALUES
            ('sym-dispatch', 'dispatch_plan', 'fixture::dispatch_plan', 'function', 'src/dispatch.rs',
             0.42, 7, 3, 11, 2, 13, 9, TIMESTAMP '2026-06-17 12:00:00', 0.91, 'load-bearing wall'),
            ('sym-review', 'review_approval', 'fixture::review_approval', 'function', 'src/review.rs',
             0.21, 2, 1, 3, 0, 3, 1, TIMESTAMP '2026-06-16 09:30:00', 0.33, 'stable');

        CREATE TABLE v_symbol_inbound (
            stable_symbol_id VARCHAR,
            callers BIGINT
        );
        INSERT INTO v_symbol_inbound VALUES
            ('sym-dispatch', 11),
            ('sym-review', 3);

        CREATE TABLE nodes (
            stable_symbol_id VARCHAR,
            node_id BIGINT,
            file_path VARCHAR,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            symbol_kind VARCHAR
        );
        INSERT INTO nodes VALUES
            ('sym-dispatch', 1, 'src/dispatch.rs', 'dispatch_plan', 'fixture::dispatch_plan', 'function'),
            ('sym-review', 2, 'src/review.rs', 'review_approval', 'fixture::review_approval', 'function');

        CREATE TABLE edges (
            source_stable_id VARCHAR,
            target_stable_id VARCHAR,
            src_id BIGINT,
            dst_id BIGINT,
            target_label VARCHAR,
            relation VARCHAR,
            confidence VARCHAR,
            confidence_score FLOAT,
            edge_kind VARCHAR,
            bind_method VARCHAR
        );
        INSERT INTO edges VALUES
            ('sym-dispatch', 'sym-review', 1, 2, 'review_approval', 'calls', 'syntax_exact', 1.0, 'calls', 'singleton');

        CREATE TABLE v_symbol_component (
            stable_symbol_id VARCHAR,
            component_id BIGINT,
            component_size BIGINT
        );
        INSERT INTO v_symbol_component VALUES
            ('sym-dispatch', 10, 2),
            ('sym-review', 10, 2);

        CREATE TABLE v_symbol_community (
            stable_symbol_id VARCHAR,
            community_id BIGINT
        );
        INSERT INTO v_symbol_community VALUES
            ('sym-dispatch', 20),
            ('sym-review', 20);

        CREATE TABLE v_graph_metrics (
            calls_edges BIGINT,
            connected_nodes BIGINT,
            components BIGINT,
            largest_component BIGINT,
            communities BIGINT,
            density DOUBLE
        );
        INSERT INTO v_graph_metrics VALUES (1, 2, 1, 2, 1, 0.5);
        ",
    )
    .expect("create fixture schema");
    conn.execute_batch(
        r"
        PRAGMA create_fts_index('main.sections_search', 'stable_symbol_id', 'body_text', overwrite=1, stemmer='porter');
        PRAGMA create_fts_index('main.symbol_text', 'stable_symbol_id', 'doc_text', overwrite=1);
        ",
    )
    .expect("create fixture fts indexes");
    conn.execute_batch(&context_candidate_macro_sql())
        .expect("define context candidate macro");
}

fn context_candidate_macro_sql() -> String {
    INIT_SEARCH_SQL
        .split("CREATE OR REPLACE MACRO search_context_candidates(q, requested_scope, intent) AS TABLE")
        .nth(1)
        .and_then(|rest| rest.split("-- Graph-augmented:").next())
        .map(|body| {
            let start =
                "CREATE OR REPLACE MACRO search_context_candidates(q, requested_scope, intent) AS TABLE";
            format!("{start}{body}")
        })
        .expect("context candidate macro should be present in init_search.sql")
}
