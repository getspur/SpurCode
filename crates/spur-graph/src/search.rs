use crate::{GraphIndexArtifact, GraphSymbolArtifact};
use globset::{Glob, GlobMatcher};
use std::cmp::Ordering;

pub(crate) const PUBLIC_SEARCH_LIMIT: usize = 200;
pub(crate) const INTERNAL_SEARCH_UNBOUNDED: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Exact,
    Prefix,
    Substring,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    pub symbol_kind: Option<String>,
    pub file: Option<String>,
    pub file_glob: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    pub query: String,
    pub mode: SearchMode,
    pub filters: SearchFilters,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchSymbol {
    pub stable_symbol_id: String,
    pub entity_name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line_range: [usize; 2],
    pub symbol_kind: String,
    pub enclosing_scope: Option<String>,
}

impl From<&GraphSymbolArtifact> for SearchSymbol {
    fn from(symbol: &GraphSymbolArtifact) -> Self {
        Self {
            stable_symbol_id: symbol.stable_symbol_id.clone(),
            entity_name: symbol.entity_name.clone(),
            qualified_name: symbol.qualified_name.clone(),
            file_path: symbol.file_path.clone(),
            line_range: symbol.line_range,
            symbol_kind: symbol.symbol_kind.clone(),
            enclosing_scope: symbol.enclosing_scope.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub candidates: Vec<SearchSymbol>,
    pub total_matches: usize,
    pub truncated: bool,
}

pub(crate) fn internal_unbounded_search_options(options: &SearchOptions) -> SearchOptions {
    let mut options = options.clone();
    options.limit = INTERNAL_SEARCH_UNBOUNDED;
    options
}

pub(crate) fn limited_search_result(
    mut candidates: Vec<SearchSymbol>,
    total_matches: usize,
    requested_limit: usize,
) -> SearchResult {
    let truncated = if requested_limit == INTERNAL_SEARCH_UNBOUNDED {
        false
    } else {
        let limit = requested_limit.clamp(1, PUBLIC_SEARCH_LIMIT);
        candidates.truncate(limit);
        total_matches > limit
    };

    SearchResult {
        candidates,
        total_matches,
        truncated,
    }
}

pub fn search_symbols(artifact: &GraphIndexArtifact, options: &SearchOptions) -> SearchResult {
    let glob = options
        .filters
        .file_glob
        .as_deref()
        .and_then(|pattern| Glob::new(pattern).ok())
        .map(|glob| glob.compile_matcher());
    let mut candidates = artifact
        .symbols
        .iter()
        .map(SearchSymbol::from)
        .filter(|symbol| matches_query(symbol, options))
        .filter(|symbol| matches_filters(symbol, &options.filters, glob.as_ref()))
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| compare_symbols(left, right, options));

    let total_matches = candidates.len();
    limited_search_result(candidates, total_matches, options.limit)
}

pub(crate) fn matches_query(symbol: &SearchSymbol, options: &SearchOptions) -> bool {
    match options.mode {
        SearchMode::Exact => {
            symbol.entity_name == options.query || symbol.qualified_name == options.query
        }
        SearchMode::Prefix => symbol.entity_name.starts_with(&options.query),
        SearchMode::Substring => symbol.entity_name.contains(&options.query),
    }
}

pub(crate) fn matches_filters(
    symbol: &SearchSymbol,
    filters: &SearchFilters,
    glob: Option<&GlobMatcher>,
) -> bool {
    if filters
        .symbol_kind
        .as_deref()
        .is_some_and(|symbol_kind| symbol.symbol_kind != symbol_kind)
    {
        return false;
    }

    if filters
        .file
        .as_deref()
        .is_some_and(|file| symbol.file_path != file)
    {
        return false;
    }

    if filters.file_glob.is_some()
        && !glob.is_some_and(|glob| glob.is_match(symbol.file_path.as_str()))
    {
        return false;
    }

    true
}

pub(crate) fn compare_symbols(
    left: &SearchSymbol,
    right: &SearchSymbol,
    options: &SearchOptions,
) -> Ordering {
    match options.mode {
        SearchMode::Exact => compare_exact(left, right, &options.query),
        SearchMode::Prefix => compare_prefix(left, right),
        SearchMode::Substring => compare_substring(left, right, &options.query),
    }
}

fn compare_exact(left: &SearchSymbol, right: &SearchSymbol, query: &str) -> Ordering {
    let left_rank = exact_rank(left, query);
    let right_rank = exact_rank(right, query);
    left_rank
        .cmp(&right_rank)
        .then_with(|| compare_location(left, right))
}

fn exact_rank(symbol: &SearchSymbol, query: &str) -> u8 {
    u8::from(symbol.entity_name != query)
}

fn compare_prefix(left: &SearchSymbol, right: &SearchSymbol) -> Ordering {
    left.entity_name
        .len()
        .cmp(&right.entity_name.len())
        .then_with(|| compare_location(left, right))
}

fn compare_substring(left: &SearchSymbol, right: &SearchSymbol, query: &str) -> Ordering {
    let left_position = left
        .entity_name
        .find(query)
        .expect("substring comparator only receives matches");
    let right_position = right
        .entity_name
        .find(query)
        .expect("substring comparator only receives matches");

    left_position
        .cmp(&right_position)
        .then_with(|| left.entity_name.len().cmp(&right.entity_name.len()))
        .then_with(|| compare_location(left, right))
}

fn compare_location(left: &SearchSymbol, right: &SearchSymbol) -> Ordering {
    left.file_path
        .cmp(&right.file_path)
        .then_with(|| left.line_range[0].cmp(&right.line_range[0]))
        .then_with(|| left.line_range[1].cmp(&right.line_range[1]))
        .then_with(|| left.stable_symbol_id.cmp(&right.stable_symbol_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphFileArtifact, GraphIndexHeader};
    use std::collections::BTreeSet;

    fn artifact(symbols: Vec<GraphSymbolArtifact>) -> GraphIndexArtifact {
        let files = symbols
            .iter()
            .map(|symbol| symbol.file_path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(index, file_path)| GraphFileArtifact {
                stable_file_id: format!("file-{index}"),
                file_path,
            })
            .collect();

        GraphIndexArtifact {
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files,
            file_node_ids: Vec::new(),
            symbols,
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    fn symbol(
        id: &str,
        file_path: &str,
        line_range: [usize; 2],
        entity_name: &str,
        qualified_name: &str,
        symbol_kind: &str,
    ) -> GraphSymbolArtifact {
        GraphSymbolArtifact {
            stable_symbol_id: id.to_owned(),
            file_path: file_path.to_owned(),
            byte_range: [0, 8],
            line_range,
            entity_name: entity_name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            symbol_kind: symbol_kind.to_owned(),
            anchor_hash: format!("hash-{id}"),
            enclosing_scope: None,
        }
    }

    fn options(query: &str, mode: SearchMode) -> SearchOptions {
        SearchOptions {
            query: query.to_owned(),
            mode,
            filters: SearchFilters::default(),
            limit: 20,
        }
    }

    fn entity_names(result: &SearchResult) -> Vec<String> {
        result
            .candidates
            .iter()
            .map(|symbol| symbol.entity_name.clone())
            .collect()
    }

    fn ids(result: &SearchResult) -> Vec<String> {
        result
            .candidates
            .iter()
            .map(|symbol| symbol.stable_symbol_id.clone())
            .collect()
    }

    #[test]
    fn exact_mode_returns_entity_then_qualified_matches() {
        let artifact = artifact(vec![
            symbol(
                "qualified-match",
                "src/b.rs",
                [2, 3],
                "helper",
                "target",
                "function",
            ),
            symbol(
                "entity-match",
                "src/a.rs",
                [10, 11],
                "target",
                "module::target",
                "function",
            ),
            symbol(
                "non-match",
                "src/c.rs",
                [1, 2],
                "targetish",
                "targetish",
                "function",
            ),
        ]);

        let result = search_symbols(&artifact, &options("target", SearchMode::Exact));

        assert_eq!(ids(&result), vec!["entity-match", "qualified-match"]);
        assert_eq!(result.total_matches, 2);
        assert!(!result.truncated);
    }

    #[test]
    fn prefix_mode_orders_shorter_targets_first() {
        let artifact = artifact(vec![
            symbol(
                "submit-plan",
                "src/lib.rs",
                [30, 31],
                "submit_plan",
                "submit_plan",
                "function",
            ),
            symbol(
                "submitter",
                "src/lib.rs",
                [20, 21],
                "submitter",
                "submitter",
                "function",
            ),
            symbol(
                "submit",
                "src/lib.rs",
                [10, 11],
                "submit",
                "submit",
                "function",
            ),
        ]);

        let result = search_symbols(&artifact, &options("sub", SearchMode::Prefix));

        assert_eq!(
            entity_names(&result),
            vec!["submit", "submitter", "submit_plan"]
        );
    }

    #[test]
    fn substring_mode_orders_by_match_position_then_length() {
        let artifact = artifact(vec![
            symbol(
                "alpha-def",
                "src/lib.rs",
                [40, 41],
                "alpha_def",
                "alpha_def",
                "function",
            ),
            symbol(
                "z-def-long",
                "src/lib.rs",
                [30, 31],
                "zdeflong",
                "zdeflong",
                "function",
            ),
            symbol("a-def", "src/lib.rs", [20, 21], "adef", "adef", "function"),
            symbol("def", "src/lib.rs", [10, 11], "def", "def", "function"),
        ]);

        let result = search_symbols(&artifact, &options("def", SearchMode::Substring));

        assert_eq!(
            entity_names(&result),
            vec!["def", "adef", "zdeflong", "alpha_def"]
        );
    }

    #[test]
    fn symbol_kind_filter_drops_non_matching_kinds() {
        let artifact = artifact(vec![
            symbol(
                "function",
                "src/tools.rs",
                [10, 11],
                "submit_plan",
                "submit_plan",
                "function",
            ),
            symbol(
                "mcp-tool",
                "src/tools.rs",
                [12, 13],
                "submit_plan",
                "submit_plan",
                "mcp_tool",
            ),
        ]);
        let mut options = options("submit", SearchMode::Substring);
        options.filters.symbol_kind = Some("mcp_tool".to_owned());

        let result = search_symbols(&artifact, &options);

        assert_eq!(ids(&result), vec!["mcp-tool"]);
        assert_eq!(result.total_matches, 1);
    }

    #[test]
    fn file_exact_filter_drops_non_matching_paths() {
        let artifact = artifact(vec![
            symbol(
                "a",
                "src/a.rs",
                [1, 2],
                "run_query",
                "run_query",
                "function",
            ),
            symbol(
                "b",
                "src/b.rs",
                [1, 2],
                "run_query",
                "run_query",
                "function",
            ),
        ]);
        let mut options = options("run", SearchMode::Substring);
        options.filters.file = Some("src/b.rs".to_owned());

        let result = search_symbols(&artifact, &options);

        assert_eq!(ids(&result), vec!["b"]);
        assert_eq!(result.total_matches, 1);
    }

    #[test]
    fn file_glob_filter_matches_with_globset() {
        let artifact = artifact(vec![
            symbol(
                "foo-lib",
                "crates/foo/src/lib.rs",
                [1, 2],
                "run_query",
                "run_query",
                "function",
            ),
            symbol(
                "foo-nested",
                "crates/foo/src/nested/mod.rs",
                [1, 2],
                "run_query",
                "run_query",
                "function",
            ),
            symbol(
                "bar-lib",
                "crates/bar/src/lib.rs",
                [1, 2],
                "run_query",
                "run_query",
                "function",
            ),
        ]);
        let mut options = options("run", SearchMode::Substring);
        options.filters.file_glob = Some("crates/foo/**/*.rs".to_owned());

        let result = search_symbols(&artifact, &options);

        assert_eq!(ids(&result), vec!["foo-lib", "foo-nested"]);
        assert_eq!(result.total_matches, 2);
    }

    #[test]
    fn limit_clamps_and_truncated_flag() {
        let artifact = artifact(
            (0..25)
                .map(|index| {
                    symbol(
                        &format!("match-{index:02}"),
                        "src/lib.rs",
                        [index + 1, index + 1],
                        &format!("match_{index:02}"),
                        &format!("match_{index:02}"),
                        "function",
                    )
                })
                .collect(),
        );
        let mut options = options("match", SearchMode::Substring);
        options.limit = 10;

        let result = search_symbols(&artifact, &options);

        assert_eq!(result.candidates.len(), 10);
        assert_eq!(result.total_matches, 25);
        assert!(result.truncated);
    }
}
