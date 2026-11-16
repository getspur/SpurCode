use anyhow::{Context as _, Result};
use duckdb::{params, Connection};
use spur_context_service::query::{
    find_callees, find_callers, list_file_symbols, list_packages, list_revisions_and_refs,
    list_tree_entries, read_symbol, resolve_selector, search_symbols, CatalogLevel, SearchMode,
    SearchOptions, SelectorResolution,
};

const SOURCE: &str = "registry:crates-io";
const PACKAGE: &str = "demo";
const REVISION: &str = "1.0.0";

#[test]
fn search_supports_exact_prefix_substring_filters_and_truncation() -> Result<()> {
    let fixture = QueryFixture::new()?;

    let exact = search_symbols(
        &fixture.conn,
        &search_options("beta", SearchMode::Exact, 20),
    )?;
    assert_eq!(exact.total_matches, 1);
    assert!(!exact.truncated);
    assert_eq!(exact.candidates[0].stable_symbol_id, "bbbbbbbbbbbbbbbb");

    let prefix = search_symbols(
        &fixture.conn,
        &search_options("alp", SearchMode::Prefix, 20),
    )?;
    assert_eq!(
        prefix
            .candidates
            .iter()
            .map(|candidate| candidate.entity_name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "alphabet"]
    );

    let mut substring_opts = search_options("pha", SearchMode::Substring, 1);
    substring_opts.file_glob = Some("src/*.rs".to_owned());
    let substring = search_symbols(&fixture.conn, &substring_opts)?;
    assert_eq!(substring.total_matches, 2);
    assert!(substring.truncated);
    assert_eq!(substring.candidates.len(), 1);
    assert_eq!(substring.candidates[0].entity_name, "alpha");
    Ok(())
}

#[test]
fn read_symbol_returns_exact_source_byte_range() -> Result<()> {
    let fixture = QueryFixture::new()?;

    let source = read_symbol(&fixture.conn, "pkg:demo@1.0.0::demo::beta", 0)?
        .context("expected beta source")?;

    assert_eq!(source.stable_symbol_id, "bbbbbbbbbbbbbbbb");
    assert_eq!(source.file_path, "src/lib.rs");
    assert_eq!(source.line_range, [6, 7]);
    assert_eq!(source.source, "pub fn beta() {\n}\n");
    Ok(())
}

#[test]
fn find_callers_returns_resolved_and_unresolved_edges() -> Result<()> {
    let fixture = QueryFixture::new()?;

    let callers = find_callers(&fixture.conn, "pkg:demo@1.0.0::demo::beta", true)?;

    let caller_ids = callers
        .callers
        .iter()
        .map(|caller| caller.caller.stable_symbol_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        caller_ids,
        ["aaaaaaaaaaaaaaaa", "dddddddddddddddd", "eeeeeeeeeeeeeeee"]
    );
    assert_eq!(callers.counts_by_kind.calls, 1);
    assert_eq!(callers.counts_by_kind.calls_dyn, 1);
    assert_eq!(callers.counts_by_kind.references_hof, 1);
    assert_eq!(callers.counts_by_kind.references_other, 0);
    assert_eq!(callers.counts_by_kind.unresolved, 1);
    assert_eq!(callers.unresolved_sample, ["demo::beta"]);
    Ok(())
}

#[test]
fn find_callees_returns_resolved_and_unresolved_edges() -> Result<()> {
    let fixture = QueryFixture::new()?;

    let callees = find_callees(&fixture.conn, "pkg:demo@1.0.0::demo::alpha", true)?;

    assert_eq!(callees.callees.len(), 2);
    assert_eq!(
        callees.callees[0]
            .callee
            .as_ref()
            .map(|candidate| candidate.stable_symbol_id.as_str()),
        Some("bbbbbbbbbbbbbbbb")
    );
    assert_eq!(callees.callees[1].callee, None);
    assert_eq!(
        callees.callees[1].edge.target_label.as_deref(),
        Some("external::Thing")
    );
    assert_eq!(callees.counts_by_kind.calls, 2);
    assert_eq!(callees.counts_by_kind.unresolved, 1);
    assert_eq!(callees.unresolved_sample, ["external::Thing"]);
    Ok(())
}

#[test]
fn resolve_selector_handles_exact_latest_ambiguous_and_missing() -> Result<()> {
    let fixture = QueryFixture::new()?;

    let exact = resolve_selector(&fixture.conn, "pkg:demo@1.0.0::demo::beta")?;
    assert!(matches!(
        exact,
        SelectorResolution::Resolved(symbol) if symbol.stable_symbol_id == "bbbbbbbbbbbbbbbb"
    ));

    let latest = resolve_selector(&fixture.conn, "pkg:demo::demo::beta")?;
    assert!(matches!(
        latest,
        SelectorResolution::Resolved(symbol) if symbol.stable_symbol_id == "bbbbbbbbbbbbbbbb"
    ));

    let ambiguous = resolve_selector(&fixture.conn, "pkg:demo@1.0.0::dup")?;
    assert!(matches!(
        ambiguous,
        SelectorResolution::Ambiguous { ref candidates } if candidates.len() == 2
    ));

    assert_eq!(
        resolve_selector(&fixture.conn, "pkg:demo@1.0.0::missing")?,
        SelectorResolution::NotFound
    );
    Ok(())
}

#[test]
fn catalog_queries_list_packages_revisions_tree_and_symbols() -> Result<()> {
    let fixture = QueryFixture::new()?;

    let first_page = list_packages(&fixture.conn, SOURCE, None, 1, None)?;
    assert_eq!(first_page.level, CatalogLevel::Packages);
    assert_eq!(first_page.total_matches, 2);
    assert!(first_page.truncated);
    assert_eq!(first_page.catalog_generation, Some(9));
    assert_eq!(first_page.rows[0].package, "demo");
    assert_eq!(first_page.rows[0].latest_revision.as_deref(), Some("1.0.0"));
    assert_eq!(first_page.rows[0].revision_count, 2);

    let second_page = list_packages(
        &fixture.conn,
        SOURCE,
        None,
        1,
        first_page.next_cursor.as_deref(),
    )?;
    assert!(!second_page.truncated);
    assert_eq!(second_page.rows[0].package, "zebra");

    let filtered = list_packages(&fixture.conn, SOURCE, Some("dem"), 50, None)?;
    assert_eq!(filtered.total_matches, 1);
    assert_eq!(filtered.rows[0].package, "demo");

    let revisions = list_revisions_and_refs(&fixture.conn, SOURCE, PACKAGE, 50, None)?;
    assert_eq!(revisions.level, CatalogLevel::Revisions);
    assert_eq!(revisions.total_matches, 2);
    let current = revisions
        .rows
        .iter()
        .find(|row| row.revision == REVISION)
        .context("current revision row")?;
    assert_eq!(current.semver.as_deref(), Some("1.0.0"));
    assert_eq!(current.embeddings_status.as_deref(), Some("complete"));
    assert_eq!(current.row_counts["nodes"], 8);
    assert_eq!(
        current
            .refs
            .iter()
            .map(|row| row.ref_name.as_str())
            .collect::<Vec<_>>(),
        ["latest", "stable"]
    );

    let root = list_tree_entries(&fixture.conn, SOURCE, PACKAGE, REVISION, None, 50, None)?;
    assert_eq!(root.level, CatalogLevel::Tree);
    assert_eq!(
        root.rows
            .iter()
            .map(|row| (row.name.as_str(), row.kind.as_str(), row.file_count))
            .collect::<Vec<_>>(),
        [("README.md", "file", 1), ("src", "dir", 3)]
    );

    let src = list_tree_entries(
        &fixture.conn,
        SOURCE,
        PACKAGE,
        REVISION,
        Some("src"),
        50,
        None,
    )?;
    assert_eq!(
        src.rows
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        ["src/a.rs", "src/b.rs", "src/lib.rs"]
    );

    let symbols = list_file_symbols(
        &fixture.conn,
        SOURCE,
        PACKAGE,
        REVISION,
        "src/lib.rs",
        Some("beta"),
        50,
        None,
    )?;
    assert_eq!(symbols.level, CatalogLevel::Symbols);
    assert_eq!(symbols.total_matches, 1);
    assert_eq!(symbols.rows[0].entity_name, "beta");
    assert_eq!(symbols.rows[0].line_range, [6, 7]);
    assert_eq!(symbols.rows[0].selector, "pkg:demo@1.0.0::demo::beta");
    assert!(symbols.rows[0]
        .next
        .iter()
        .any(|entry| entry.tool == "external_code_read"));
    Ok(())
}

#[test]
fn catalog_package_search_matches_names_keywords_and_typos() -> Result<()> {
    let fixture = QueryFixture::new()?;
    seed_catalog_search_packages(&fixture.conn)?;
    for (filter, expected) in [
        (
            "DUCK",
            vec!["duckdb", "duckdb-loadable-macros", "libduckdb-sys"],
        ),
        ("sys duck", vec!["libduckdb-sys"]),
        ("DUCK_sys", vec!["libduckdb-sys"]),
        ("dukdb", vec!["duckdb", "duckdb-loadable-macros"]),
        ("duckbd", vec!["duckdb", "duckdb-loadable-macros"]),
        ("axm", vec!["axum"]),
        ("dk", vec![]),
        ("dukxx", vec![]),
        ("duckzz", vec![]),
        ("unrelated", vec![]),
        ("CAFÉ", vec!["café"]),
        ("cafe", vec!["café"]),
        ("---", vec![]),
        ("' OR 1=1 --", vec![]),
    ] {
        let result = list_packages(&fixture.conn, SOURCE, Some(filter), 50, None)?;
        let actual = result
            .rows
            .iter()
            .map(|row| row.package.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "filter {filter:?}");
        assert_eq!(result.total_matches, expected.len(), "filter {filter:?}");
    }
    let all = list_packages(&fixture.conn, SOURCE, None, 50, None)?;
    for filter in ["", "   "] {
        let result = list_packages(&fixture.conn, SOURCE, Some(filter), 50, None)?;
        assert_eq!(result.rows, all.rows, "filter {filter:?}");
    }
    Ok(())
}

#[test]
fn catalog_package_search_preserves_source_order_and_cursor_totals() -> Result<()> {
    let fixture = QueryFixture::new()?;
    seed_catalog_search_packages(&fixture.conn)?;
    let mut cursor = None;
    let mut packages = Vec::new();
    loop {
        let page = list_packages(&fixture.conn, SOURCE, Some("DUCK"), 1, cursor.as_deref())?;
        assert_eq!(page.total_matches, 3);
        assert_eq!(page.catalog_generation, Some(9));
        packages.extend(page.rows.into_iter().map(|row| row.package));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        packages,
        ["duckdb", "duckdb-loadable-macros", "libduckdb-sys"]
    );
    let other = list_packages(
        &fixture.conn,
        "git:github.com/example/duck",
        Some("DUCK"),
        50,
        None,
    )?;
    assert_eq!(other.total_matches, 1);
    assert_eq!(other.rows[0].package, "duck-private");
    Ok(())
}

fn seed_catalog_search_packages(conn: &Connection) -> Result<()> {
    for (source, package) in [
        (SOURCE, "duckdb"),
        (SOURCE, "duckdb-loadable-macros"),
        (SOURCE, "libduckdb-sys"),
        (SOURCE, "axum"),
        (SOURCE, "café"),
        ("git:github.com/example/duck", "duck-private"),
    ] {
        conn.execute(
            "INSERT INTO package_catalog (source, package, revision, indexed_at, generation) VALUES ($1, $2, '1.0.0', TIMESTAMP '2026-06-23 00:00:00', 9)",
            params![source, package],
        )?;
    }
    Ok(())
}

#[test]
fn catalog_queries_tolerate_missing_refs_table() -> Result<()> {
    let fixture = QueryFixture::new()?;
    fixture
        .conn
        .execute("DROP TABLE refs", [])
        .context("drop refs table")?;

    let packages = list_packages(&fixture.conn, SOURCE, Some("dem"), 50, None)?;
    assert_eq!(packages.total_matches, 1);
    assert_eq!(packages.rows[0].latest_revision, None);

    let revisions = list_revisions_and_refs(&fixture.conn, SOURCE, PACKAGE, 50, None)?;
    let current = revisions
        .rows
        .iter()
        .find(|row| row.revision == REVISION)
        .context("current revision row")?;
    assert!(current.refs.is_empty());
    Ok(())
}

struct QueryFixture {
    conn: Connection,
}

impl QueryFixture {
    fn new() -> Result<Self> {
        let conn = Connection::open_in_memory().context("open in-memory duckdb")?;
        create_schema(&conn)?;
        seed_fixture(&conn)?;
        Ok(Self { conn })
    }
}

fn search_options(query: &str, mode: SearchMode, limit: usize) -> SearchOptions {
    SearchOptions {
        source: SOURCE.to_owned(),
        package: PACKAGE.to_owned(),
        revision: REVISION.to_owned(),
        query: query.to_owned(),
        mode,
        symbol_kind: Some("function".to_owned()),
        file_glob: None,
        limit,
    }
}

fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r"
        CREATE TABLE nodes (
            stable_symbol_id VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            file_path VARCHAR,
            byte_range_start INTEGER,
            byte_range_end INTEGER,
            line_start INTEGER,
            line_end INTEGER,
            entity_name VARCHAR,
            qualified_name VARCHAR,
            symbol_kind VARCHAR,
            anchor_hash VARCHAR,
            enclosing_scope VARCHAR
        );

        CREATE TABLE edges (
            source_stable_id VARCHAR,
            target_stable_id VARCHAR,
            target_package VARCHAR,
            target_label VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            relation VARCHAR,
            edge_kind VARCHAR,
            confidence VARCHAR,
            confidence_score DOUBLE,
            bind_method VARCHAR,
            receiver_text VARCHAR,
            scope_text VARCHAR
        );

        CREATE TABLE edges_unresolved (
            source_stable_id VARCHAR,
            target_label VARCHAR,
            target_package VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            relation VARCHAR,
            edge_kind VARCHAR,
            confidence VARCHAR,
            confidence_score DOUBLE,
            bind_method VARCHAR,
            receiver_text VARCHAR,
            scope_text VARCHAR
        );

        CREATE TABLE files (
            stable_file_id VARCHAR,
            file_path VARCHAR,
            source_text VARCHAR,
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER
        );

        CREATE TABLE file_manifests (
            stable_file_id VARCHAR,
            path VARCHAR,
            content_oid VARCHAR,
            node_ids VARCHAR[],
            package VARCHAR,
            source VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER
        );

        CREATE TABLE package_catalog (
            source VARCHAR,
            package VARCHAR,
            revision VARCHAR,
            revision_kind VARCHAR,
            semver_major INTEGER,
            semver_minor INTEGER,
            semver_patch INTEGER,
            snapshot_id BIGINT,
            indexed_at TIMESTAMP,
            index_status VARCHAR,
            embeddings_status VARCHAR,
            row_counts JSON,
            generation BIGINT,
            bronze_content_sha256 VARCHAR,
            silver_graph_content_hash VARCHAR,
            builder_version VARCHAR,
            translate_schema_version VARCHAR,
            embed_text_version VARCHAR
        );

        CREATE TABLE refs (
            source VARCHAR,
            package VARCHAR,
            ref_name VARCHAR,
            revision VARCHAR,
            updated_at TIMESTAMP
        );
        ",
    )
    .context("create query schema")?;
    Ok(())
}

fn seed_fixture(conn: &Connection) -> Result<()> {
    let source_text = concat!(
        "pub fn alpha() {\n",
        "    beta();\n",
        "    external::Thing::new();\n",
        "}\n",
        "\n",
        "pub fn beta() {\n",
        "}\n",
        "\n",
        "pub fn alphabet() {}\n",
        "\n",
        "pub fn caller() {\n",
        "    beta();\n",
        "}\n",
        "\n",
        "pub fn dynamic_caller() {}\n",
        "\n",
        "pub fn hof_caller() {}\n",
    );
    conn.execute(
        r"
        INSERT INTO files VALUES
            ('file-lib', 'src/lib.rs', $1, 'demo', 'registry:crates-io', '1.0.0',
             'semver', 1, 0, 0)
        ",
        params![source_text],
    )
    .context("insert file")?;

    insert_node(
        conn,
        "aaaaaaaaaaaaaaaa",
        source_text,
        "alpha",
        "demo::alpha",
        1,
        4,
    )?;
    insert_node(
        conn,
        "bbbbbbbbbbbbbbbb",
        source_text,
        "beta",
        "demo::beta",
        6,
        7,
    )?;
    insert_node(
        conn,
        "cccccccccccccccc",
        source_text,
        "alphabet",
        "demo::alphabet",
        9,
        9,
    )?;
    insert_node(
        conn,
        "dddddddddddddddd",
        source_text,
        "caller",
        "demo::caller",
        11,
        13,
    )?;
    insert_node(
        conn,
        "eeeeeeeeeeeeeeee",
        source_text,
        "dynamic_caller",
        "demo::dynamic_caller",
        15,
        15,
    )?;
    insert_node(
        conn,
        "ffffffffffffffff",
        source_text,
        "hof_caller",
        "demo::hof_caller",
        17,
        17,
    )?;

    insert_synthetic_node(conn, "1111111111111111", "src/a.rs", "dup", "a::dup")?;
    insert_synthetic_node(conn, "2222222222222222", "src/b.rs", "dup", "b::dup")?;
    insert_old_revision_node(conn)?;

    insert_edge(
        conn,
        "aaaaaaaaaaaaaaaa",
        Some("bbbbbbbbbbbbbbbb"),
        None,
        None,
        "calls",
        "calls",
    )?;
    insert_edge(
        conn,
        "dddddddddddddddd",
        Some("bbbbbbbbbbbbbbbb"),
        None,
        None,
        "calls",
        "references_hof",
    )?;
    insert_edge(
        conn,
        "ffffffffffffffff",
        Some("bbbbbbbbbbbbbbbb"),
        None,
        None,
        "calls",
        "references_other",
    )?;
    insert_edge(
        conn,
        "aaaaaaaaaaaaaaaa",
        Some("bbbbbbbbbbbbbbbb"),
        None,
        None,
        "contains",
        "calls",
    )?;
    insert_unresolved_edge(
        conn,
        "eeeeeeeeeeeeeeee",
        "demo::beta",
        None,
        "calls",
        "calls_dyn",
    )?;
    insert_unresolved_edge(
        conn,
        "aaaaaaaaaaaaaaaa",
        "external::Thing",
        Some("external"),
        "calls",
        "calls",
    )?;

    conn.execute(
        r"
        INSERT INTO refs VALUES
            ('registry:crates-io', 'demo', 'latest', '1.0.0',
             TIMESTAMP '2026-06-22 00:00:00'),
            ('registry:crates-io', 'demo', 'stable', '1.0.0',
             TIMESTAMP '2026-06-22 00:01:00'),
            ('registry:crates-io', 'demo', 'old', '0.9.0',
             TIMESTAMP '2026-06-21 00:00:00')
        ",
        [],
    )
    .context("insert latest ref")?;

    conn.execute_batch(
        r#"
        INSERT INTO file_manifests VALUES
            ('file-readme', 'README.md', 'oid-readme', []::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0),
            ('file-lib', 'src/lib.rs', 'oid-lib',
             ['aaaaaaaaaaaaaaaa', 'bbbbbbbbbbbbbbbb', 'cccccccccccccccc',
              'dddddddddddddddd', 'eeeeeeeeeeeeeeee', 'ffffffffffffffff']::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0),
            ('file-a', 'src/a.rs', 'oid-a', ['1111111111111111']::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0),
            ('file-b', 'src/b.rs', 'oid-b', ['2222222222222222']::VARCHAR[],
             'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0);

        INSERT INTO package_catalog (
            source, package, revision, revision_kind,
            semver_major, semver_minor, semver_patch,
            snapshot_id, indexed_at, index_status, embeddings_status,
            row_counts, generation, bronze_content_sha256,
            silver_graph_content_hash, builder_version,
            translate_schema_version, embed_text_version
        )
        VALUES
            ('registry:crates-io', 'demo', '0.9.0', 'semver',
             0, 9, 0, 90, TIMESTAMP '2026-06-21 00:00:00',
             'complete', 'skipped', '{"nodes": 1}', 5,
             'bronze-old', 'graph-old', 'builder', 'translate', 'embed'),
            ('registry:crates-io', 'demo', '1.0.0', 'semver',
             1, 0, 0, 100, TIMESTAMP '2026-06-22 00:00:00',
             'complete', 'complete', '{"nodes": 8, "files": 4}', 7,
             'bronze-current', 'graph-current', 'builder', 'translate', 'embed'),
            ('registry:crates-io', 'zebra', '0.1.0', 'semver',
             0, 1, 0, 10, TIMESTAMP '2026-06-23 00:00:00',
             'complete', 'complete', '{"nodes": 0}', 9,
             'bronze-zebra', 'graph-zebra', 'builder', 'translate', 'embed');
        "#,
    )
    .context("insert catalog rows")?;
    Ok(())
}

fn insert_node(
    conn: &Connection,
    id: &str,
    source_text: &str,
    entity_name: &str,
    qualified_name: &str,
    line_start: i64,
    line_end: i64,
) -> Result<()> {
    let marker = format!("pub fn {entity_name}");
    let byte_start = source_text
        .find(&marker)
        .with_context(|| format!("find marker {marker}"))? as i64;
    let byte_end = source_text[byte_start as usize..]
        .find("\n\n")
        .map(|offset| byte_start + offset as i64 + 1)
        .unwrap_or(source_text.len() as i64);
    conn.execute(
        r"
        INSERT INTO nodes VALUES
            ($1, 'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0,
             'src/lib.rs', $2, $3, $4, $5, $6, $7, 'function', $8, NULL)
        ",
        params![
            id,
            byte_start,
            byte_end,
            line_start,
            line_end,
            entity_name,
            qualified_name,
            format!("anchor-{id}")
        ],
    )
    .with_context(|| format!("insert node {qualified_name}"))?;
    Ok(())
}

fn insert_synthetic_node(
    conn: &Connection,
    id: &str,
    file_path: &str,
    entity_name: &str,
    qualified_name: &str,
) -> Result<()> {
    conn.execute(
        r"
        INSERT INTO nodes VALUES
            ($1, 'demo', 'registry:crates-io', '1.0.0', 'semver', 1, 0, 0,
             $2, 0, 1, 1, 1, $3, $4, 'function', $5, NULL)
        ",
        params![
            id,
            file_path,
            entity_name,
            qualified_name,
            format!("anchor-{id}")
        ],
    )
    .with_context(|| format!("insert synthetic node {qualified_name}"))?;
    Ok(())
}

fn insert_old_revision_node(conn: &Connection) -> Result<()> {
    conn.execute(
        r"
        INSERT INTO nodes VALUES
            ('9999999999999999', 'demo', 'registry:crates-io', '0.9.0', 'semver',
             0, 9, 0, 'src/lib.rs', 0, 1, 1, 1, 'beta', 'demo::beta',
             'function', 'anchor-old', NULL)
        ",
        [],
    )
    .context("insert old revision node")?;
    Ok(())
}

fn insert_edge(
    conn: &Connection,
    source_id: &str,
    target_id: Option<&str>,
    target_package: Option<&str>,
    target_label: Option<&str>,
    relation: &str,
    edge_kind: &str,
) -> Result<()> {
    conn.execute(
        r"
        INSERT INTO edges VALUES
            ($1, $2, $3, $4, 'demo', 'registry:crates-io', '1.0.0',
             'semver', 1, 0, 0, $5, $6, 'syntax_exact', 0.99,
             'singleton', NULL, NULL)
        ",
        params![
            source_id,
            target_id,
            target_package,
            target_label,
            relation,
            edge_kind
        ],
    )
    .context("insert resolved edge")?;
    Ok(())
}

fn insert_unresolved_edge(
    conn: &Connection,
    source_id: &str,
    target_label: &str,
    target_package: Option<&str>,
    relation: &str,
    edge_kind: &str,
) -> Result<()> {
    conn.execute(
        r"
        INSERT INTO edges_unresolved VALUES
            ($1, $2, $3, 'demo', 'registry:crates-io', '1.0.0',
             'semver', 1, 0, 0, $4, $5, 'heuristic', 0.50,
             'label', NULL, NULL)
        ",
        params![source_id, target_label, target_package, relation, edge_kind],
    )
    .context("insert unresolved edge")?;
    Ok(())
}
