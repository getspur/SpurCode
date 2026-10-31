//! Graph-backed code mention source (2026-08-27 mentions spec §3, shared
//! `code` module; relocated from `spur-tui` by the sud-m4 decoupling
//! phase — behavior is byte-for-byte the TUI's except where noted).
//!
//! Discovery resolves the artifact (explicit override, then the worktree's
//! `.spur/graph/CURRENT` / pointer file), loads it either as a slim single
//! artifact or as a compact parquet directory, materializes only file rows
//! eagerly, and keeps symbols as compact [`CodeMentionCandidate`]s with lazy
//! payload hydration. Row dressing (tags, secondary lines, atom text) is
//! frontend policy and stays with the consumers (the TUI keeps
//! `spur_tui::mentions::code_graph::source::entry_for_candidate`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use spur_graph::{
    load_artifact_slim, read_artifact_header_parquet, resolve_artifact_location,
    CodeMentionAuthoritative, CodeMentionDisplayMeta, CodeMentionExtractionHints, CodeMentionKind,
    CodeMentionPayload, CodeMentionValidationSpec, GraphFileArtifact, GraphIndexArtifact,
    GraphQueryClient as _, GraphSymbolArtifact, ParquetClient, ResolvedArtifact, SearchFilters,
    SearchMode, SearchOptions, SearchSymbol, CODE_FILE_URI_PREFIX, CODE_SYMBOL_URI_PREFIX,
};

use crate::entry::{
    MentionEntry, MentionId, MentionKind, MentionSource, SourceBuildError, SourceContext,
    SourceSnapshot,
};

/// One compact symbol row: everything ranking and lazy hydration need,
/// with the heavy payload left on disk until a picker actually selects the
/// symbol. Frontends dress this into their own richer row type.
#[derive(Debug, Clone)]
pub struct CodeMentionCandidate {
    /// Stable symbol id (the `graph://symbol/<id>` suffix).
    pub stable_symbol_id: Box<str>,
    /// Primary picker label (the bare entity name).
    pub entity_name: Box<str>,
    /// Interned relative file path.
    pub file_path: Arc<str>,
    /// 1-based inclusive line range.
    pub line_range: [usize; 2],
    /// Interned symbol kind (`fn`, `struct`, …).
    pub symbol_kind: Arc<str>,
    /// Interned enclosing scope, when the symbol is nested.
    pub enclosing_scope: Option<Arc<str>>,
}

struct CodeGraphCacheKey {
    path: PathBuf,
    mtime: Option<SystemTime>,
    len: Option<u64>,
    content_hash: Option<String>,
}

pub struct CodeGraphMentionSource {
    worktree_root: Option<PathBuf>,
    explicit_override: Option<PathBuf>,
    cache_key: Option<CodeGraphCacheKey>,
    cached_entries: Vec<MentionEntry>,
    payloads: Vec<(String, Arc<CodeMentionPayload>)>,
    candidates: Arc<Vec<CodeMentionCandidate>>,
    payload_backend: Option<CodePayloadSnapshot>,
    #[cfg(test)]
    reload_count: usize,
}

enum CodePayloadBackend {
    Parquet {
        client: Box<ParquetClient>,
        graph_index_version: String,
    },
    InMemory {
        symbols: HashMap<String, GraphSymbolArtifact>,
        graph_index_version: String,
    },
}

/// Lazy payload hydration for one loaded graph generation. Cloning this
/// handle shares its backend, independently of later source rebuilds. Parquet
/// handles retain their canonical artifact directory and reader metadata;
/// artifact files must continue to obey the graph publication contract.
#[derive(Clone)]
pub struct CodePayloadSnapshot(Arc<CodePayloadBackend>);

impl CodeGraphMentionSource {
    pub fn new(artifact_path: impl Into<PathBuf>) -> Self {
        Self {
            worktree_root: None,
            explicit_override: Some(artifact_path.into()),
            cache_key: None,
            cached_entries: Vec::new(),
            payloads: Vec::new(),
            candidates: Arc::default(),
            payload_backend: None,
            #[cfg(test)]
            reload_count: 0,
        }
    }

    /// Source rooted at a worktree: the artifact is discovered from the
    /// worktree's graph pointers unless `explicit_override` pins one.
    pub fn for_worktree(
        worktree_root: impl Into<PathBuf>,
        explicit_override: Option<PathBuf>,
    ) -> Self {
        Self {
            worktree_root: Some(worktree_root.into()),
            explicit_override,
            cache_key: None,
            cached_entries: Vec::new(),
            payloads: Vec::new(),
            candidates: Arc::default(),
            payload_backend: None,
            #[cfg(test)]
            reload_count: 0,
        }
    }

    fn clear_loaded_state(&mut self) {
        self.cache_key = None;
        self.cached_entries.clear();
        self.payloads.clear();
        self.candidates = Arc::default();
        self.payload_backend = None;
    }

    /// Eagerly materialized payloads (file rows only; symbols hydrate
    /// lazily through [`Self::hydrate_code_payloads`]).
    pub fn code_payloads(&self) -> &[(String, Arc<CodeMentionPayload>)] {
        &self.payloads
    }

    /// Compact symbol index shared with the frontend's ranking pass.
    pub fn code_candidates(&self) -> Arc<Vec<CodeMentionCandidate>> {
        Arc::clone(&self.candidates)
    }

    /// Capture the hydration backend belonging to the current candidates.
    pub fn code_payload_snapshot(&self) -> Option<CodePayloadSnapshot> {
        self.payload_backend.clone()
    }

    /// Hydrate payloads for just the selected stable symbol ids (parquet
    /// backend: a bounded column read; slim backend: a map lookup).
    pub fn hydrate_code_payloads(
        &self,
        stable_symbol_ids: &[String],
    ) -> anyhow::Result<Vec<(String, Arc<CodeMentionPayload>)>> {
        let Some(backend) = &self.payload_backend else {
            return Ok(Vec::new());
        };
        backend.hydrate_code_payloads(stable_symbol_ids)
    }
}

impl CodePayloadSnapshot {
    /// Hydrate only the selected IDs using this generation's backend.
    pub fn hydrate_code_payloads(
        &self,
        stable_symbol_ids: &[String],
    ) -> anyhow::Result<Vec<(String, Arc<CodeMentionPayload>)>> {
        let (symbols, graph_index_version) = match self.0.as_ref() {
            CodePayloadBackend::Parquet {
                client,
                graph_index_version,
            } => (
                client.symbols_by_stable_ids(stable_symbol_ids)?,
                graph_index_version.as_str(),
            ),
            CodePayloadBackend::InMemory {
                symbols,
                graph_index_version,
            } => (
                stable_symbol_ids
                    .iter()
                    .filter_map(|stable_id| symbols.get(stable_id).cloned())
                    .collect(),
                graph_index_version.as_str(),
            ),
        };
        Ok(symbols
            .into_iter()
            .map(|symbol| {
                let uri = format!("{}{}", CODE_SYMBOL_URI_PREFIX, symbol.stable_symbol_id);
                let display = symbol.entity_name.clone();
                let payload = symbol_payload(&symbol, &uri, &display, graph_index_version);
                (uri, Arc::new(payload))
            })
            .collect())
    }
}

impl MentionSource for CodeGraphMentionSource {
    fn key(&self) -> &'static str {
        "code_graph"
    }

    fn build(
        &mut self,
        cwd: &Path,
        _context: &SourceContext,
    ) -> Result<SourceSnapshot, SourceBuildError> {
        let worktree_root = self
            .worktree_root
            .clone()
            .unwrap_or_else(|| spur_graph::resolve_worktree_root_from(cwd.to_path_buf()));
        let resolved =
            match resolve_artifact_location(&worktree_root, self.explicit_override.as_deref()) {
                Ok(resolved) => resolved,
                Err(error) => {
                    let explicit_override = self
                        .explicit_override
                        .as_ref()
                        .map(|path| path.display().to_string());
                    tracing::warn!(
                        error = %error,
                        worktree_root = %worktree_root.display(),
                        explicit_override = explicit_override.as_deref(),
                        "code graph mention source disabled; no readable artifact found"
                    );
                    self.clear_loaded_state();
                    return Ok(SourceSnapshot::new(Vec::new(), 0));
                }
            };
        let metadata = fs::metadata(&resolved.path).ok();
        let modified = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok());
        let len = metadata.as_ref().map(|metadata| metadata.len());

        if let Some(cache_key) = self.cache_key.as_mut() {
            if cache_key.path == resolved.path {
                if let Ok(current_hash) = read_resolved_content_hash(&resolved) {
                    if let (Some(cached_hash), Some(current_hash)) =
                        (cache_key.content_hash.as_deref(), current_hash.as_deref())
                    {
                        if cached_hash == current_hash {
                            cache_key.path = resolved.path.clone();
                            cache_key.mtime = modified;
                            cache_key.len = len;
                            let span = tracing::debug_span!(
                                "code_graph_build",
                                path = %resolved.path.display(),
                                cached = "hash"
                            );
                            let _guard = span.enter();
                            tracing::debug!(
                                path = %resolved.path.display(),
                                "code graph mention source cached by content hash"
                            );
                            return Ok(SourceSnapshot::new(self.cached_entries.clone(), 0));
                        }
                    }
                }
            }
        }

        let span = tracing::debug_span!(
            "code_graph_build",
            path = %resolved.path.display(),
            cached = "miss"
        );
        let _guard = span.enter();

        let loaded = match tracing::debug_span!("load_code_mention_index")
            .in_scope(|| load_code_index(&resolved.path))
        {
            Ok(loaded) => loaded,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    path = %resolved.path.display(),
                    "code graph mention source disabled for unreadable artifact"
                );
                self.clear_loaded_state();
                return Ok(SourceSnapshot::new(Vec::new(), 0));
            }
        };

        #[cfg(test)]
        {
            self.reload_count += 1;
        }

        let content_hash = Some(loaded.content_hash.clone());
        for diagnostic in &loaded.diagnostics {
            tracing::warn!(
                diagnostic = diagnostic.as_str(),
                path = %resolved.path.display(),
                "code graph mention source loaded artifact with diagnostic"
            );
        }

        let mut payloads = Vec::new();
        let entries = tracing::debug_span!("materialize_entries").in_scope(|| {
            file_entries_and_payloads(loaded.files, &loaded.graph_index_version, &mut payloads)
        });
        self.payloads = payloads;
        self.candidates = Arc::new(compact_candidates(loaded.candidates));
        self.payload_backend = Some(CodePayloadSnapshot(Arc::new(loaded.payload_backend)));
        self.cached_entries = entries.clone();
        tracing::info!(
            path = %resolved.path.display(),
            entries = entries.len(),
            candidates = self.candidates.len(),
            payloads = self.payloads.len(),
            "code graph mention source reloaded"
        );
        self.cache_key = Some(CodeGraphCacheKey {
            path: resolved.path,
            mtime: modified,
            len,
            content_hash,
        });
        Ok(SourceSnapshot::new(entries, 0))
    }
}

fn read_resolved_content_hash(resolved: &ResolvedArtifact) -> anyhow::Result<Option<String>> {
    Ok(Some(
        read_artifact_header_parquet(&resolved.path)?.graph_content_hash,
    ))
}

struct LoadedCodeIndex {
    files: Vec<GraphFileArtifact>,
    candidates: Vec<SearchSymbol>,
    payload_backend: CodePayloadBackend,
    graph_index_version: String,
    content_hash: String,
    diagnostics: Vec<String>,
}

fn load_code_index(path: &Path) -> anyhow::Result<LoadedCodeIndex> {
    if path.is_dir() {
        let client = ParquetClient::open(path)?;
        let manifest = client.manifest();
        let graph_index_version = manifest.graph_index_version.clone();
        let content_hash = manifest.graph_content_hash.clone();
        let files = client.files()?;
        let diagnostics = client.diagnostics()?;
        let candidates = client
            .search_symbols(&SearchOptions {
                query: String::new(),
                mode: SearchMode::Substring,
                filters: SearchFilters::default(),
                limit: usize::MAX,
            })?
            .candidates;
        return Ok(LoadedCodeIndex {
            files,
            candidates,
            payload_backend: CodePayloadBackend::Parquet {
                client: Box::new(client),
                graph_index_version: graph_index_version.clone(),
            },
            graph_index_version,
            content_hash,
            diagnostics,
        });
    }

    let artifact: GraphIndexArtifact = load_artifact_slim(path)?;
    let graph_index_version = artifact.header.graph_index_version.clone();
    let content_hash = artifact.graph_content_hash.clone();
    let diagnostics = artifact.diagnostics.clone();
    let candidates = artifact.symbols.iter().map(SearchSymbol::from).collect();
    let symbols = artifact
        .symbols
        .into_iter()
        .map(|symbol| (symbol.stable_symbol_id.clone(), symbol))
        .collect();
    Ok(LoadedCodeIndex {
        files: artifact.files,
        candidates,
        payload_backend: CodePayloadBackend::InMemory {
            symbols,
            graph_index_version: graph_index_version.clone(),
        },
        graph_index_version,
        content_hash,
        diagnostics,
    })
}

fn compact_candidates(candidates: Vec<SearchSymbol>) -> Vec<CodeMentionCandidate> {
    let mut interned = HashMap::<String, Arc<str>>::new();
    let mut compact = candidates
        .into_iter()
        .map(|candidate| CodeMentionCandidate {
            stable_symbol_id: candidate.stable_symbol_id.into_boxed_str(),
            entity_name: candidate.entity_name.into_boxed_str(),
            file_path: intern(&mut interned, candidate.file_path),
            line_range: candidate.line_range,
            symbol_kind: intern(&mut interned, candidate.symbol_kind),
            enclosing_scope: candidate
                .enclosing_scope
                .map(|scope| intern(&mut interned, scope)),
        })
        .collect::<Vec<_>>();
    compact.sort_by(|left, right| {
        left.entity_name
            .len()
            .cmp(&right.entity_name.len())
            .then(left.entity_name.cmp(&right.entity_name))
            .then(left.stable_symbol_id.cmp(&right.stable_symbol_id))
    });
    compact
}

fn intern(interned: &mut HashMap<String, Arc<str>>, value: String) -> Arc<str> {
    if let Some(existing) = interned.get(&value) {
        return Arc::clone(existing);
    }
    let shared = Arc::<str>::from(value.as_str());
    interned.insert(value, Arc::clone(&shared));
    shared
}

fn file_entries_and_payloads(
    files: Vec<GraphFileArtifact>,
    graph_index_version: &str,
    payloads: &mut Vec<(String, Arc<CodeMentionPayload>)>,
) -> Vec<MentionEntry> {
    let mut entries = Vec::with_capacity(files.len());
    let mut next_id = MentionId::new(0);

    for file in files {
        let uri = format!("{}{}", CODE_FILE_URI_PREFIX, file.stable_file_id);
        let display = file.file_path.clone();
        payloads.push((
            uri.clone(),
            Arc::new(file_payload(&file, &uri, &display, graph_index_version)),
        ));
        entries.push(MentionEntry {
            id: next_id,
            kind: MentionKind::CodeFile,
            uri,
            display,
            secondary: None,
            search_text: Some(file.file_path),
            insert_text: None,
        });
        next_id = next_id.next();
    }

    entries
}

fn file_payload(
    file: &GraphFileArtifact,
    uri: &str,
    display: &str,
    graph_index_version: &str,
) -> CodeMentionPayload {
    CodeMentionPayload {
        authoritative: CodeMentionAuthoritative {
            display: display.to_owned(),
            uri: uri.to_owned(),
            kind: CodeMentionKind::File,
            file_path: file.file_path.clone(),
            validation: CodeMentionValidationSpec::FileExists {
                path: file.file_path.clone(),
            },
        },
        extraction_hints: CodeMentionExtractionHints {
            line_range: None,
            byte_range: None,
            symbol_kind: None,
            entity_name: None,
            qualified_name: String::new(),
        },
        display_meta: CodeMentionDisplayMeta {
            enclosing_scope: None,
            graph_index_version: graph_index_version.to_owned(),
        },
    }
}

fn symbol_payload(
    symbol: &GraphSymbolArtifact,
    uri: &str,
    display: &str,
    graph_index_version: &str,
) -> CodeMentionPayload {
    CodeMentionPayload {
        authoritative: CodeMentionAuthoritative {
            display: display.to_owned(),
            uri: uri.to_owned(),
            kind: CodeMentionKind::Symbol,
            file_path: symbol.file_path.clone(),
            validation: CodeMentionValidationSpec::SymbolRange {
                path: symbol.file_path.clone(),
                line_range: symbol.line_range,
                byte_range: symbol.byte_range,
                entity_name: symbol.entity_name.clone(),
                anchor_hash: symbol.anchor_hash.clone(),
            },
        },
        extraction_hints: CodeMentionExtractionHints {
            line_range: Some(symbol.line_range),
            byte_range: Some(symbol.byte_range),
            symbol_kind: Some(symbol.symbol_kind.clone()),
            entity_name: Some(symbol.entity_name.clone()),
            qualified_name: symbol.qualified_name.clone(),
        },
        display_meta: CodeMentionDisplayMeta {
            enclosing_scope: symbol.enclosing_scope.clone(),
            graph_index_version: graph_index_version.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use spur_graph::{
        write_artifact_parquet, GraphFileArtifact, GraphFileManifestEntry, GraphIndexArtifact,
        GraphIndexHeader, NodeId, WriteOptions,
    };
    use std::fs;
    use std::time::Duration;

    /// Neutral-trait build through the default (TUI-compatible) context.
    fn build_entries(source: &mut CodeGraphMentionSource) -> Vec<MentionEntry> {
        MentionSource::build(source, Path::new("."), &SourceContext::default())
            .expect("build")
            .entries
    }

    fn write_fixture(path: &Path, file_path: &str) {
        let content_hash = format!("hash-{}", file_path.replace('/', "-"));
        write_fixture_with_hash(path, file_path, &content_hash);
    }

    fn write_fixture_with_hash(path: &Path, file_path: &str, content_hash: &str) {
        write_artifact_parquet(
            &graph_artifact("file-1", file_path, content_hash),
            path,
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write fixture");
    }

    fn rewrite_until_mtime_changes(path: &Path, file_path: &str, previous: SystemTime) {
        rewrite_until_mtime_changes_with(path, previous, || write_fixture(path, file_path));
    }

    fn rewrite_until_mtime_changes_with(
        path: &Path,
        previous: SystemTime,
        mut rewrite: impl FnMut(),
    ) {
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            rewrite();
            let current = fs::metadata(path)
                .expect("metadata")
                .modified()
                .expect("modified");
            if current != previous {
                return;
            }
        }
        panic!("artifact mtime did not change");
    }

    #[test]
    fn build_uses_mtime_cache_and_reloads_on_change() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact_path = temp.path().join("graph-index.parquet");
        write_fixture(&artifact_path, "src/main.rs");
        let mut source = CodeGraphMentionSource::new(&artifact_path);

        let first = build_entries(&mut source);
        assert_eq!(first.len(), 1);
        assert_eq!(source.reload_count, 1);

        let second = build_entries(&mut source);
        assert_eq!(second.len(), 1);
        assert_eq!(source.reload_count, 1);

        let previous_mtime = fs::metadata(&artifact_path)
            .expect("metadata")
            .modified()
            .expect("modified");
        rewrite_until_mtime_changes(&artifact_path, "src/lib.rs", previous_mtime);

        let third = build_entries(&mut source);
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].display, "src/lib.rs");
        assert_eq!(source.reload_count, 2);

        let stable_metadata = fs::metadata(&artifact_path).expect("metadata");
        let stable_mtime = stable_metadata.modified().expect("modified");
        write_fixture(&artifact_path, "src/lib_rewritten_with_longer_name.rs");
        let stable_filetime = FileTime::from_system_time(stable_mtime);
        set_file_mtime(&artifact_path, stable_filetime).expect("restore mtime");

        let fourth = build_entries(&mut source);
        assert_eq!(fourth.len(), 1);
        assert_eq!(fourth[0].display, "src/lib_rewritten_with_longer_name.rs");
        assert_eq!(source.reload_count, 3);
    }

    #[test]
    fn code_payloads_cache_hit_reuses_arc_instances() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact_path = temp.path().join("graph-index.parquet");
        write_fixture(&artifact_path, "src/main.rs");
        let mut source = CodeGraphMentionSource::new(&artifact_path);

        let _ = build_entries(&mut source);
        let first = source.code_payloads();
        let (uri, first_payload) = first.first().expect("first payload");
        let uri = uri.clone();
        let first_payload = Arc::clone(first_payload);

        let _ = build_entries(&mut source);
        let second = source.code_payloads();
        let second_payload = second
            .iter()
            .find(|(candidate_uri, _)| candidate_uri == &uri)
            .map(|(_, payload)| payload)
            .expect("second payload");

        assert!(Arc::ptr_eq(&first_payload, second_payload));
    }

    #[test]
    fn parquet_build_keeps_symbols_compact_and_hydrates_only_selected_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut artifact = graph_artifact("file-1", "src/lib.rs", "hash-compact");
        artifact.symbols = vec![
            GraphSymbolArtifact {
                stable_symbol_id: "symbol-alpha".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                byte_range: [0, 10],
                line_range: [1, 1],
                entity_name: "alpha".to_owned(),
                qualified_name: "alpha".to_owned(),
                symbol_kind: "fn".to_owned(),
                anchor_hash: "anchor-alpha".to_owned(),
                enclosing_scope: None,
            },
            GraphSymbolArtifact {
                stable_symbol_id: "symbol-beta".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                byte_range: [11, 20],
                line_range: [2, 2],
                entity_name: "beta".to_owned(),
                qualified_name: "module::beta".to_owned(),
                symbol_kind: "fn".to_owned(),
                anchor_hash: "anchor-beta".to_owned(),
                enclosing_scope: Some("module".to_owned()),
            },
        ];
        artifact.symbol_node_ids = vec![NodeId(2), NodeId(3)];
        artifact.file_manifests[0].node_ids = vec![NodeId(1), NodeId(2), NodeId(3)];
        let parquet_dir =
            write_artifact_parquet(&artifact, temp.path(), WriteOptions::default(), Vec::new())
                .expect("write parquet artifact");
        let mut source = CodeGraphMentionSource::new(&parquet_dir);

        let entries = build_entries(&mut source);
        let candidates = source.code_candidates();
        let payloads = source
            .hydrate_code_payloads(&["symbol-beta".to_owned()])
            .expect("hydrate selected symbol");

        assert_eq!(
            entries.len(),
            1,
            "only the file row is materialized eagerly"
        );
        assert_eq!(entries[0].kind, MentionKind::CodeFile);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            source.code_payloads().len(),
            1,
            "only the file payload is eager"
        );
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].0, "graph://symbol/symbol-beta");
        assert_eq!(
            payloads[0].1.extraction_hints.qualified_name,
            "module::beta"
        );
        assert!(payloads
            .iter()
            .all(|(uri, _)| uri != "graph://symbol/symbol-alpha"));

        let _ = build_entries(&mut source);
        let cached_candidates = source.code_candidates();
        assert!(Arc::ptr_eq(&candidates, &cached_candidates));
        assert_eq!(source.reload_count, 1);
    }

    #[test]
    fn build_uses_content_hash_when_metadata_changes_but_content_does_not() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact_path = temp.path().join("graph-index.parquet");
        write_fixture_with_hash(&artifact_path, "src/main.rs", "hash-same");
        let mut source = CodeGraphMentionSource::new(&artifact_path);

        let first = build_entries(&mut source);
        assert_eq!(first.len(), 1);
        assert_eq!(source.reload_count, 1);

        let previous_mtime = fs::metadata(&artifact_path)
            .expect("metadata")
            .modified()
            .expect("modified");
        rewrite_until_mtime_changes_with(&artifact_path, previous_mtime, || {
            write_fixture_with_hash(&artifact_path, "src/main.rs", "hash-same");
        });

        let second = build_entries(&mut source);
        assert_eq!(second.len(), 1);
        assert_eq!(source.reload_count, 1);
    }

    #[test]
    fn build_uses_parquet_manifest_hash_when_directory_metadata_changes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let parquet_dir = write_artifact_parquet(
            &graph_artifact("file-1", "src/main.rs", "hash-same"),
            temp.path(),
            WriteOptions::default(),
            Vec::new(),
        )
        .expect("write parquet artifact");
        let mut source = CodeGraphMentionSource::new(&parquet_dir);

        let first = build_entries(&mut source);
        assert_eq!(first.len(), 1);
        assert_eq!(source.reload_count, 1);

        let previous_mtime = fs::metadata(&parquet_dir)
            .expect("metadata")
            .modified()
            .expect("modified");
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            let changed = previous_mtime + Duration::from_secs(1);
            set_file_mtime(&parquet_dir, FileTime::from_system_time(changed))
                .expect("update parquet dir mtime");
            let current = fs::metadata(&parquet_dir)
                .expect("metadata")
                .modified()
                .expect("modified");
            if current != previous_mtime {
                break;
            }
        }

        let second = build_entries(&mut source);
        assert_eq!(second.len(), 1);
        assert_eq!(source.reload_count, 1);
    }

    #[test]
    fn build_reloads_when_hash_changes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact_path = temp.path().join("graph-index.parquet");
        write_fixture_with_hash(&artifact_path, "src/main.rs", "hash-before");
        let mut source = CodeGraphMentionSource::new(&artifact_path);

        let first = build_entries(&mut source);
        assert_eq!(first.len(), 1);
        assert_eq!(source.reload_count, 1);

        let previous_mtime = fs::metadata(&artifact_path)
            .expect("metadata")
            .modified()
            .expect("modified");
        rewrite_until_mtime_changes_with(&artifact_path, previous_mtime, || {
            write_fixture_with_hash(&artifact_path, "src/lib.rs", "hash-after");
        });

        let second = build_entries(&mut source);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].display, "src/lib.rs");
        assert_eq!(source.reload_count, 2);
    }

    fn graph_artifact(
        stable_file_id: &str,
        file_path: &str,
        content_hash: &str,
    ) -> GraphIndexArtifact {
        GraphIndexArtifact {
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "source-test".to_owned(),
                content_hash_blake3: Some(content_hash.to_owned()),
            },
            manifest_version: "source-test-manifest".to_owned(),
            graph_content_hash: content_hash.to_owned(),
            file_manifests: vec![GraphFileManifestEntry {
                stable_file_id: stable_file_id.to_owned(),
                path: file_path.to_owned(),
                content_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                node_ids: Vec::new(),
            }],
            files: vec![GraphFileArtifact {
                stable_file_id: stable_file_id.to_owned(),
                file_path: file_path.to_owned(),
            }],
            file_node_ids: vec![NodeId(1)],
            symbols: Vec::new(),
            symbol_node_ids: Vec::new(),
            edges: Vec::new(),
            tombstones: Vec::new(),
            diagnostics: Vec::new(),

            commits: Vec::new(),

            symbol_snapshots: Vec::new(),

            temporal_edges: Vec::new(),
        }
    }
}
