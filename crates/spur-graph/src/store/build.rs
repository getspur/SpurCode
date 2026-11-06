use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str;
use std::time::Instant;

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};

use crate::content_hash::{compute_graph_content_hash, git_blob_oid};
use crate::discovery::discover_files;
use crate::extract::GraphFacts;
use crate::extract::{
    build_facts_for_paths,
    languages::all_supported_extensions,
    tree_sitter::{
        classify_import_origin, function_singleton_safe,
        is_import_resolution_fallback_candidate_kind, is_web_import_source_file, language_family,
        web_import_target_path, ImportOriginClassification, ImportWorkspaceIndex,
    },
};
use crate::identity::{stable_symbol_id_for_external_path, EXTERNAL_FILE_PATH};
use crate::schema::GRAPH_INDEX_VERSION_TEMPORAL;
use crate::validation::compute_anchor_hash;
use crate::{
    git, graph_edge_kind_or_default, DirtyEntry, GitCtx, GraphEdgeArtifact, GraphEdgeKind,
    GraphFileArtifact, GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphNode,
    GraphSymbolArtifact, GraphTombstoneEntry, NodeId, NodeKind, RelationKind, SourceSpan,
};

pub const SCHEMA_VERSION: &str = "spur-graph-schema-v12";
pub const EXTRACTOR_VERSION: &str = "2026-11-03-rust-foreign-declarations-v1";
/// Bump when resolver semantics change without query, extractor, or schema changes.
pub const RESOLVER_VERSION: &str = "2026-11-02-rust-receiver-rebind-v2";

#[derive(Debug, Clone, Copy)]
struct ManifestQueryBytes<'a> {
    language: &'a str,
    query: &'a str,
    bytes: &'a [u8],
}

const MANIFEST_QUERY_BYTES: &[ManifestQueryBytes<'static>] = &[
    ManifestQueryBytes {
        language: "c",
        query: "tags",
        bytes: include_bytes!("../../queries/c/tags.scm"),
    },
    ManifestQueryBytes {
        language: "c",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/c/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "cpp",
        query: "tags",
        bytes: include_bytes!("../../queries/cpp/tags.scm"),
    },
    ManifestQueryBytes {
        language: "cpp",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/cpp/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "go",
        query: "tags",
        bytes: include_bytes!("../../queries/go/tags.scm"),
    },
    ManifestQueryBytes {
        language: "go",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/go/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "hcl",
        query: "tags",
        bytes: include_bytes!("../../queries/hcl/tags.scm"),
    },
    ManifestQueryBytes {
        language: "hcl",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/hcl/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "markdown",
        query: "tags",
        bytes: include_bytes!("../../queries/markdown/tags.scm"),
    },
    ManifestQueryBytes {
        language: "markdown",
        query: "symbols",
        bytes: include_bytes!("../../queries/markdown/symbols.scm"),
    },
    ManifestQueryBytes {
        language: "markdown",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/markdown/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "markdown",
        query: "inline-spur-edges",
        bytes: include_bytes!("../../queries/markdown/inline-spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "lua",
        query: "tags",
        bytes: include_bytes!("../../queries/lua/tags.scm"),
    },
    ManifestQueryBytes {
        language: "lua",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/lua/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "python",
        query: "tags",
        bytes: include_bytes!("../../queries/python/tags.scm"),
    },
    ManifestQueryBytes {
        language: "python",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/python/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "python",
        query: "spur-notebook-facts",
        bytes: include_bytes!("../../queries/python/spur-notebook-facts.scm"),
    },
    ManifestQueryBytes {
        language: "rust",
        query: "tags",
        bytes: include_bytes!("../../queries/rust/tags.scm"),
    },
    ManifestQueryBytes {
        language: "rust",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/rust/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "shell",
        query: "tags",
        bytes: include_bytes!("../../queries/shell/tags.scm"),
    },
    ManifestQueryBytes {
        language: "shell",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/shell/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "sql",
        query: "tags",
        bytes: include_bytes!("../../queries/sql/tags.scm"),
    },
    ManifestQueryBytes {
        language: "sql",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/sql/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "typescript",
        query: "tags",
        bytes: include_bytes!("../../queries/typescript/tags.scm"),
    },
    ManifestQueryBytes {
        language: "typescript",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/typescript/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "typescript",
        query: "spur-notebook-facts",
        bytes: include_bytes!("../../queries/typescript/spur-notebook-facts.scm"),
    },
    ManifestQueryBytes {
        language: "typescript",
        query: "jsx-edges",
        bytes: include_bytes!("../../queries/typescript/jsx-edges.scm"),
    },
    ManifestQueryBytes {
        language: "json",
        query: "tags",
        bytes: include_bytes!("../../queries/json/tags.scm"),
    },
    ManifestQueryBytes {
        language: "toml",
        query: "tags",
        bytes: include_bytes!("../../queries/toml/tags.scm"),
    },
    ManifestQueryBytes {
        language: "yaml",
        query: "tags",
        bytes: include_bytes!("../../queries/yaml/tags.scm"),
    },
    ManifestQueryBytes {
        language: "mermaid",
        query: "tags",
        bytes: include_bytes!("../../queries/mermaid/tags.scm"),
    },
    ManifestQueryBytes {
        language: "html",
        query: "tags",
        bytes: include_bytes!("../../queries/html/tags.scm"),
    },
    ManifestQueryBytes {
        language: "html",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/html/spur-edges.scm"),
    },
    ManifestQueryBytes {
        language: "css",
        query: "tags",
        bytes: include_bytes!("../../queries/css/tags.scm"),
    },
    ManifestQueryBytes {
        language: "css",
        query: "spur-edges",
        bytes: include_bytes!("../../queries/css/spur-edges.scm"),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    Full,
    Incremental,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildStats {
    pub reused_buckets: usize,
    pub changed_paths: usize,
    pub changed_or_added_paths: BTreeSet<String>,
    pub removed_paths: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeDeltaArtifact {
    pub artifact: GraphIndexArtifact,
    pub changed_files: usize,
    pub new_symbols: usize,
    pub tombstoned_symbols: usize,
}

#[derive(Debug, Clone)]
struct FileBucket {
    file: GraphFileArtifact,
    file_node_id: Option<NodeId>,
    manifest: GraphFileManifestEntry,
    symbols: Vec<GraphSymbolArtifact>,
    symbol_node_ids: Vec<NodeId>,
    edges: Vec<GraphEdgeArtifact>,
    macro_invocations: Vec<crate::MacroInvocationEvidence>,
    macro_scopes: Vec<crate::MacroScopeEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentFileEntry {
    path: String,
    content_oid: String,
    extractable: bool,
}

pub fn current_manifest_version() -> String {
    manifest_version_from_query_bytes(
        SCHEMA_VERSION,
        EXTRACTOR_VERSION,
        RESOLVER_VERSION,
        MANIFEST_QUERY_BYTES,
    )
}

fn manifest_version_from_query_bytes(
    schema_version: &str,
    extractor_version: &str,
    resolver_version: &str,
    query_bytes: &[ManifestQueryBytes<'_>],
) -> String {
    let mut hasher = Sha256::new();
    update_manifest_hash_field(&mut hasher, schema_version.as_bytes());
    update_manifest_hash_field(&mut hasher, extractor_version.as_bytes());
    update_manifest_hash_field(&mut hasher, resolver_version.as_bytes());
    let mut query_bytes_by_language = query_bytes.iter().collect::<Vec<_>>();
    query_bytes_by_language.sort_by_key(|query| (query.language, query.query));
    for query in query_bytes_by_language {
        update_manifest_hash_field(&mut hasher, query.language.as_bytes());
        update_manifest_hash_field(&mut hasher, query.query.as_bytes());
        update_manifest_hash_field(&mut hasher, query.bytes);
    }
    format!("{:x}", hasher.finalize())
}

fn update_manifest_hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

pub fn artifact_from_facts(
    facts: &GraphFacts,
    worktree_root: &Path,
) -> anyhow::Result<GraphIndexArtifact> {
    artifact_from_facts_in_domain(facts, worktree_root, None)
}

/// Compose selected facts against exactly the requested current file domain.
/// Missing paths have no active manifest; deletion transport belongs to deltas.
pub fn artifact_from_facts_for_paths(
    facts: &GraphFacts,
    root: &Path,
    paths: &[PathBuf],
) -> anyhow::Result<GraphIndexArtifact> {
    artifact_from_facts_in_domain(facts, root, Some(paths))
}

fn artifact_from_facts_in_domain(
    facts: &GraphFacts,
    worktree_root: &Path,
    paths: Option<&[PathBuf]>,
) -> anyhow::Result<GraphIndexArtifact> {
    let build_started = Instant::now();
    let worktree_root = worktree_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", worktree_root.display()))?;
    let discover_started = Instant::now();
    let discover_span = tracing::info_span!(
        target: "spur_graph::build::discover",
        "discover_current_entries",
        root = %worktree_root.display(),
        files = tracing::field::Empty
    );
    let current_entries = {
        let _entered = discover_span.enter();
        let result = match paths {
            Some(paths) => discover_selected_entries(&worktree_root, paths),
            None => discover_current_entries(&worktree_root),
        };
        match &result {
            Ok(entries) => {
                discover_span.record("files", entries.len());
                tracing::info!(
                    target: "spur_graph::build::discover",
                    files = entries.len(),
                    elapsed_ms = elapsed_ms(discover_started),
                    "spur-graph build phase completed"
                );
            }
            Err(error) => {
                tracing::info!(
                    target: "spur_graph::build::discover",
                    error = %error,
                    elapsed_ms = elapsed_ms(discover_started),
                    "spur-graph build phase failed"
                );
            }
        }
        result?
    };
    // Nodes and edges are already restricted by buckets_from_facts. Restrict
    // the exact macro carriers too, before composing a selected artifact.
    let selected_facts = paths.map(|_| {
        let mut selected = facts.clone();
        selected
            .macro_invocations
            .retain(|row| current_entries.contains_key(&row.file_path));
        selected
            .macro_scopes
            .retain(|scope| current_entries.contains_key(&scope.file_path));
        selected
    });
    let facts = selected_facts.as_ref().unwrap_or(facts);
    let buckets = buckets_from_facts(facts, &worktree_root, &current_entries)?;
    let compose_started = Instant::now();
    let compose_span = tracing::info_span!(
        target: "spur_graph::build::compose",
        "compose_artifact",
        files = current_entries.len(),
        buckets = buckets.len()
    );
    let artifact = {
        let _entered = compose_span.enter();
        let mut artifact = compose_artifact(
            buckets,
            &current_entries,
            current_manifest_version(),
            Vec::new(),
        );
        validate_macro_evidence(&artifact, &worktree_root, &facts.macro_scopes)?;
        attach_rust_evidence(&mut artifact, &worktree_root)?;
        tracing::info!(
            target: "spur_graph::build::compose",
            files = artifact.files.len(),
            symbols = artifact.symbols.len(),
            edges = artifact.edges.len(),
            tombstones = artifact.tombstones.len(),
            elapsed_ms = elapsed_ms(compose_started),
            "spur-graph build phase completed"
        );
        artifact
    };
    tracing::info!(
        target: "spur_graph::build",
        mode = "Full",
        files = artifact.files.len(),
        symbols = artifact.symbols.len(),
        edges = artifact.edges.len(),
        changed = 0_u64,
        elapsed_ms = elapsed_ms(build_started),
        "spur-graph build completed"
    );
    Ok(artifact)
}

pub fn artifact_from_facts_incremental(
    prev: &GraphIndexArtifact,
    root: &Path,
) -> anyhow::Result<(GraphIndexArtifact, BuildMode, BuildStats)> {
    let build_started = Instant::now();
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", root.display()))?;
    let manifest_version = current_manifest_version();
    if prev.manifest_version != manifest_version
        || !prev.macro_invocations.as_ref().is_some_and(|set| {
            set.schema_version == crate::MACRO_INVOCATION_SCHEMA_VERSION
                && set.extractor_version == EXTRACTOR_VERSION
        })
    {
        tracing::info!(
            expected = %manifest_version,
            found = %prev.manifest_version,
            "spur-graph: full rebuild selected: incompatible manifest or macro evidence"
        );
        let extract_started = Instant::now();
        let extract_span = tracing::info_span!(
            target: "spur_graph::build::extract_full",
            "build_facts",
            root = %root.display()
        );
        let facts = {
            let _entered = extract_span.enter();
            let result = crate::build_facts(&root, None);
            match &result {
                Ok((facts, file_counts)) => {
                    tracing::info!(
                        target: "spur_graph::build::extract_full",
                        files = file_counts.values().sum::<usize>(),
                        nodes = facts.nodes.len(),
                        edges = facts.edges.len(),
                        elapsed_ms = elapsed_ms(extract_started),
                        "spur-graph build phase completed"
                    );
                }
                Err(error) => {
                    tracing::info!(
                        target: "spur_graph::build::extract_full",
                        error = %error,
                        elapsed_ms = elapsed_ms(extract_started),
                        "spur-graph build phase failed"
                    );
                }
            }
            result?.0
        };
        let artifact_started = Instant::now();
        let artifact_span = tracing::info_span!(
            target: "spur_graph::build::artifact",
            "artifact_from_facts",
            root = %root.display()
        );
        let artifact = {
            let _entered = artifact_span.enter();
            let result = artifact_from_facts(&facts, &root);
            match &result {
                Ok(artifact) => {
                    tracing::info!(
                        target: "spur_graph::build::artifact",
                        files = artifact.files.len(),
                        symbols = artifact.symbols.len(),
                        edges = artifact.edges.len(),
                        elapsed_ms = elapsed_ms(artifact_started),
                        "spur-graph build phase completed"
                    );
                }
                Err(error) => {
                    tracing::info!(
                        target: "spur_graph::build::artifact",
                        error = %error,
                        elapsed_ms = elapsed_ms(artifact_started),
                        "spur-graph build phase failed"
                    );
                }
            }
            result?
        };
        return Ok((artifact, BuildMode::Full, BuildStats::default()));
    }

    let discover_started = Instant::now();
    let discover_span = tracing::info_span!(
        target: "spur_graph::build::discover",
        "discover_current_entries",
        root = %root.display(),
        files = tracing::field::Empty
    );
    let current_entries = {
        let _entered = discover_span.enter();
        let result = discover_current_entries(&root);
        match &result {
            Ok(entries) => {
                discover_span.record("files", entries.len());
                tracing::info!(
                    target: "spur_graph::build::discover",
                    files = entries.len(),
                    elapsed_ms = elapsed_ms(discover_started),
                    "spur-graph build phase completed"
                );
            }
            Err(error) => {
                tracing::info!(
                    target: "spur_graph::build::discover",
                    error = %error,
                    elapsed_ms = elapsed_ms(discover_started),
                    "spur-graph build phase failed"
                );
            }
        }
        result?
    };
    let rebucket_started = Instant::now();
    let rebucket_span = tracing::info_span!(
        target: "spur_graph::build::rebucket",
        "buckets_from_artifact",
        prev_files = prev.file_manifests.len(),
        prev_symbols = prev.symbols.len(),
        prev_edges = prev.edges.len()
    );
    anyhow::ensure!(
        prev.macro_invocations
            .as_ref()
            .is_some_and(|set| set.graph_content_hash == prev.graph_content_hash),
        "previous macro evidence snapshot mismatch"
    );
    let prev_buckets = {
        let _entered = rebucket_span.enter();
        let buckets = buckets_from_artifact(prev);
        tracing::info!(
            target: "spur_graph::build::rebucket",
            buckets = buckets.len(),
            elapsed_ms = elapsed_ms(rebucket_started),
            "spur-graph build phase completed"
        );
        buckets
    };

    let changed_started = Instant::now();
    let changed_span = tracing::info_span!(
        target: "spur_graph::build::changed_paths",
        "compute_changed_paths",
        files = current_entries.len(),
        prev_files = prev.file_manifests.len(),
        changed_paths = tracing::field::Empty
    );
    let (mut buckets, changed_paths, changed_or_added_paths, reused_buckets) = {
        let _entered = changed_span.enter();
        let diff = changed_paths_from_previous(prev, &root, &current_entries, &prev_buckets);
        changed_span.record("changed_paths", diff.changed_paths.len());
        tracing::info!(
            target: "spur_graph::build::changed_paths",
            changed_paths = diff.changed_paths.len(),
            reused = diff.reused_buckets,
            elapsed_ms = elapsed_ms(changed_started),
            "spur-graph build phase completed"
        );
        (
            diff.buckets,
            diff.changed_paths,
            diff.changed_or_added_paths,
            diff.reused_buckets,
        )
    };
    let changed_count = changed_paths.len();
    let removed_paths = removed_paths_from_previous(prev, &current_entries);
    let stats = BuildStats {
        reused_buckets,
        changed_paths: changed_count,
        changed_or_added_paths,
        removed_paths: removed_paths.clone(),
    };

    let mut changed_tombstones = Vec::new();
    if !changed_paths.is_empty() {
        let extract_started = Instant::now();
        let extract_span = tracing::info_span!(
            target: "spur_graph::build::extract_changed",
            "build_facts_for_paths",
            changed_paths = changed_paths.len()
        );
        let changed_facts = {
            let _entered = extract_span.enter();
            let result = build_facts_for_paths(&root, &changed_paths);
            match &result {
                Ok(facts) => {
                    tracing::info!(
                        target: "spur_graph::build::extract_changed",
                        changed_paths = changed_paths.len(),
                        nodes = facts.nodes.len(),
                        edges = facts.edges.len(),
                        elapsed_ms = elapsed_ms(extract_started),
                        "spur-graph build phase completed"
                    );
                }
                Err(error) => {
                    tracing::info!(
                        target: "spur_graph::build::extract_changed",
                        changed_paths = changed_paths.len(),
                        error = %error,
                        elapsed_ms = elapsed_ms(extract_started),
                        "spur-graph build phase failed"
                    );
                }
            }
            result?
        };
        changed_tombstones =
            tombstones_from_changed_paths(prev, &root, &changed_paths, &changed_facts)?;
        let changed_buckets = buckets_from_facts(&changed_facts, &root, &current_entries)?;
        buckets.extend(changed_buckets);
    }

    let compose_started = Instant::now();
    let compose_span = tracing::info_span!(
        target: "spur_graph::build::compose",
        "compose_artifact",
        files = current_entries.len(),
        buckets = buckets.len()
    );
    let artifact = {
        let _entered = compose_span.enter();
        add_missing_manifest_buckets(&mut buckets, &current_entries);

        let tombstones = tombstones_for_worktree(prev, &current_entries, changed_tombstones);
        let artifact = compose_artifact(buckets, &current_entries, manifest_version, tombstones);
        validate_macro_evidence(
            &artifact,
            &root,
            &artifact
                .macro_invocations
                .as_ref()
                .context("composed artifact must carry captured macro evidence")?
                .scopes,
        )?;
        tracing::info!(
            target: "spur_graph::build::compose",
            files = artifact.files.len(),
            symbols = artifact.symbols.len(),
            edges = artifact.edges.len(),
            tombstones = artifact.tombstones.len(),
            elapsed_ms = elapsed_ms(compose_started),
            "spur-graph build phase completed"
        );
        artifact
    };
    tracing::info!(
        target: "spur_graph::build",
        mode = "Incremental",
        files = artifact.files.len(),
        symbols = artifact.symbols.len(),
        edges = artifact.edges.len(),
        changed = changed_count,
        elapsed_ms = elapsed_ms(build_started),
        "spur-graph build completed"
    );
    Ok((artifact, BuildMode::Incremental, stats))
}

pub(crate) fn build_worktree_delta_artifact(
    prev: &GraphIndexArtifact,
    root: &Path,
) -> anyhow::Result<WorktreeDeltaArtifact> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", root.display()))?;
    let manifest_version = current_manifest_version();
    if prev.manifest_version != manifest_version {
        anyhow::bail!(
            "cannot write worktree delta for manifest_version {}; expected {}",
            prev.manifest_version,
            manifest_version
        );
    }

    let current_entries = discover_current_entries(&root)?;
    let prev_buckets = buckets_from_artifact(prev);
    let diff = changed_paths_from_previous(prev, &root, &current_entries, &prev_buckets);

    let changed_facts = if diff.changed_paths.is_empty() {
        GraphFacts::empty()
    } else {
        build_facts_for_paths(&root, &diff.changed_paths)?
    };
    let changed_tombstones =
        tombstones_from_changed_paths(prev, &root, &diff.changed_paths, &changed_facts)?;
    let tombstones = tombstones_for_worktree(prev, &current_entries, changed_tombstones);

    let changed_entries: BTreeMap<_, _> = current_entries
        .iter()
        .filter(|(path, _entry)| diff.changed_or_added_paths.contains(*path))
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect();
    let mut buckets = if diff.changed_paths.is_empty() {
        BTreeMap::new()
    } else {
        buckets_from_facts(&changed_facts, &root, &current_entries)?
    };
    add_missing_manifest_buckets(&mut buckets, &changed_entries);
    rebind_cross_file_edges(&mut buckets);

    let graph_content_hash = compute_graph_content_hash(
        changed_entries
            .values()
            .map(|entry| (entry.path.as_str(), entry.content_oid.as_str())),
    );
    let artifact = rebuild_from_buckets(buckets, manifest_version, graph_content_hash, tombstones);
    validate_macro_evidence(&artifact, &root, &changed_facts.macro_scopes)?;
    let prev_symbol_ids: HashSet<_> = prev
        .symbols
        .iter()
        .map(|symbol| symbol.stable_symbol_id.as_str())
        .collect();
    let tombstoned_symbols = artifact
        .tombstones
        .iter()
        .filter(|tombstone| prev_symbol_ids.contains(tombstone.stable_file_id.as_str()))
        .count();
    Ok(WorktreeDeltaArtifact {
        changed_files: diff.changed_paths.len(),
        new_symbols: artifact.symbols.len(),
        tombstoned_symbols,
        artifact,
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn buckets_from_facts(
    facts: &GraphFacts,
    worktree_root: &Path,
    current_entries: &BTreeMap<String, CurrentFileEntry>,
) -> anyhow::Result<BTreeMap<String, FileBucket>> {
    let spans_by_id: HashMap<_, _> = facts
        .spans
        .iter()
        .map(|span| (span.span_id, span))
        .collect();
    let nodes_by_id: HashMap<_, _> = facts
        .nodes
        .iter()
        .map(|node| (node.node_id, node))
        .collect();
    let parent_by_target = parent_by_target(facts);

    let mut buckets: BTreeMap<String, FileBucket> = BTreeMap::new();
    for node in &facts.nodes {
        let Some(span_id) = node.source_span_id else {
            continue;
        };
        let Some(span) = spans_by_id.get(&span_id).copied() else {
            continue;
        };

        match node.kind {
            NodeKind::File => {
                if !current_entries.contains_key(&node.label) {
                    continue;
                }
                let stable_file_id = node.stable_key.clone();
                let entry = buckets
                    .entry(node.label.clone())
                    .or_insert_with(|| FileBucket {
                        file: GraphFileArtifact {
                            stable_file_id: stable_file_id.clone(),
                            file_path: node.label.clone(),
                        },
                        file_node_id: Some(node.node_id),
                        manifest: GraphFileManifestEntry {
                            stable_file_id: stable_file_id.clone(),
                            path: node.label.clone(),
                            content_oid: content_oid_for(current_entries, &node.label),
                            node_ids: vec![node.node_id],
                        },
                        symbols: Vec::new(),
                        symbol_node_ids: Vec::new(),
                        edges: Vec::new(),
                        macro_invocations: Vec::new(),
                        macro_scopes: Vec::new(),
                    });
                entry.file = GraphFileArtifact {
                    stable_file_id: stable_file_id.clone(),
                    file_path: node.label.clone(),
                };
                entry.file_node_id = Some(node.node_id);
                entry.manifest.stable_file_id = stable_file_id;
                if !entry.manifest.node_ids.contains(&node.node_id) {
                    entry.manifest.node_ids.push(node.node_id);
                }
            }
            NodeKind::Module
            | NodeKind::ExternBlock
            | NodeKind::Function
            | NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Struct
            | NodeKind::Impl
            | NodeKind::Trait
            | NodeKind::Enum
            | NodeKind::EnumVariant
            | NodeKind::Method
            | NodeKind::Field
            | NodeKind::Constant
            | NodeKind::TypeAlias
            | NodeKind::Macro
            | NodeKind::Section
            | NodeKind::McpTool
            | NodeKind::Cell
            | NodeKind::Port
            | NodeKind::Resource => {
                let file_path = file_path_for_span(facts, span).unwrap_or_default();
                if !current_entries.contains_key(&file_path) {
                    continue;
                }
                let anchor_hash = anchor_hash(worktree_root, &file_path, span);
                let symbol = GraphSymbolArtifact {
                    stable_symbol_id: node.stable_key.clone(),
                    file_path: file_path.clone(),
                    byte_range: [span.start_byte as usize, span.end_byte as usize],
                    line_range: [span.start_line as usize, span.end_line as usize],
                    entity_name: symbol_entity_name(&node.label),
                    qualified_name: qualified_name(&parent_by_target, &nodes_by_id, node),
                    symbol_kind: symbol_kind(node.kind).to_owned(),
                    anchor_hash,
                    enclosing_scope: enclosing_scope(&parent_by_target, &nodes_by_id, node),
                };
                let entry = buckets.entry(file_path.clone()).or_insert_with(|| {
                    let stable_file_id = stable_file_id_from_path(&file_path);
                    FileBucket {
                        file: GraphFileArtifact {
                            stable_file_id: stable_file_id.clone(),
                            file_path: file_path.clone(),
                        },
                        file_node_id: None,
                        manifest: GraphFileManifestEntry {
                            stable_file_id,
                            path: file_path.clone(),
                            content_oid: content_oid_for(current_entries, &file_path),
                            node_ids: Vec::new(),
                        },
                        symbols: Vec::new(),
                        symbol_node_ids: Vec::new(),
                        edges: Vec::new(),
                        macro_invocations: Vec::new(),
                        macro_scopes: Vec::new(),
                    }
                });
                entry.manifest.node_ids.push(node.node_id);
                entry.symbol_node_ids.push(node.node_id);
                entry.symbols.push(symbol);
            }
            NodeKind::Commit => {
                // Commit nodes belong in artifact.commits, not artifact.symbols.
                continue;
            }
            NodeKind::External => {
                // External nodes are introduced as a bodyless class before any extractor emits them.
                continue;
            }
        }
    }

    for edge in &facts.edges {
        let Some(source_node) = nodes_by_id.get(&edge.source_node_id).copied() else {
            tracing::warn!(
                source_node_id = edge.source_node_id.get(),
                "spur-graph: dropping edge with unknown source node"
            );
            continue;
        };
        let Some(source_file_path) = node_file_path(facts, source_node, &spans_by_id) else {
            tracing::warn!(
                source_stable_key = %source_node.stable_key,
                source_node_id = edge.source_node_id.get(),
                relation = %relation_discriminator(edge.relation),
                "spur-graph: dropping edge with unknown source file path"
            );
            continue;
        };
        if !current_entries.contains_key(&source_file_path) {
            continue;
        }
        let target_stable_symbol_id = edge
            .target_node_id
            .and_then(|target_node_id| nodes_by_id.get(&target_node_id))
            .map(|node| node.stable_key.clone());
        let entry = buckets.entry(source_file_path.clone()).or_insert_with(|| {
            let stable_file_id = stable_file_id_from_path(&source_file_path);
            FileBucket {
                file: GraphFileArtifact {
                    stable_file_id: stable_file_id.clone(),
                    file_path: source_file_path.clone(),
                },
                file_node_id: None,
                manifest: GraphFileManifestEntry {
                    stable_file_id,
                    path: source_file_path.clone(),
                    content_oid: content_oid_for(current_entries, &source_file_path),
                    node_ids: Vec::new(),
                },
                symbols: Vec::new(),
                symbol_node_ids: Vec::new(),
                edges: Vec::new(),
                macro_invocations: Vec::new(),
                macro_scopes: Vec::new(),
            }
        });
        entry.edges.push(GraphEdgeArtifact {
            source_stable_symbol_id: source_node.stable_key.clone(),
            target_stable_symbol_id,
            target_label: edge.target_label.clone(),
            import_path: edge.import_path.clone(),
            receiver_text: edge.receiver_text.clone(),
            scope_text: edge.scope_text.clone(),
            relation: edge.relation,
            confidence: edge.confidence,
            confidence_score: edge.confidence_score,
            change_kind: None,
            edge_kind: Some(graph_edge_kind_or_default(edge.relation, edge.edge_kind)),
            bind_method: edge.bind_method.clone(),
        });
    }

    // Transport the already collected records; never infer scopes from symbol IDs.
    for row in &facts.macro_invocations {
        let entry = current_entries
            .get(&row.file_path)
            .context("missing macro source entry")?;
        anyhow::ensure!(
            entry.content_oid == row.source_oid,
            "macro source OID mismatch"
        );
        buckets
            .entry(row.file_path.clone())
            .or_insert_with(|| empty_bucket(&row.file_path, &entry.content_oid))
            .macro_invocations
            .push(row.clone());
    }
    for scope in &facts.macro_scopes {
        let entry = current_entries
            .get(&scope.file_path)
            .context("missing macro scope source entry")?;
        anyhow::ensure!(
            entry.content_oid == scope.source_oid,
            "macro scope source OID mismatch"
        );
        buckets
            .entry(scope.file_path.clone())
            .or_insert_with(|| empty_bucket(&scope.file_path, &entry.content_oid))
            .macro_scopes
            .push(scope.clone());
    }
    for bucket in buckets.values_mut() {
        bucket.manifest.node_ids.sort_by_key(|id| id.get());
        sort_bucket_symbols(bucket);
        bucket
            .edges
            .sort_by(|a, b| edge_sort_key(a).cmp(&edge_sort_key(b)));
    }

    Ok(buckets)
}

fn discover_current_entries(root: &Path) -> anyhow::Result<BTreeMap<String, CurrentFileEntry>> {
    let allowed_extensions = all_supported_extensions();
    if let Some(ctx) = git::detect(root) {
        discover_git_entries(root, &ctx, &allowed_extensions)
    } else {
        discover_fs_entries(root, &allowed_extensions)
    }
}

fn discover_selected_entries(
    root: &Path,
    paths: &[PathBuf],
) -> anyhow::Result<BTreeMap<String, CurrentFileEntry>> {
    let allowed_extensions = all_supported_extensions();
    let mut entries = BTreeMap::new();
    for normalized in crate::extract::normalize_selected_paths(root, paths)? {
        let path = relative_path(root, &root.join(&normalized))?;
        if path.contains("#cell:")
            || !is_supported_path(&path, &allowed_extensions)
            || entries.contains_key(&path)
        {
            continue;
        }
        let full_path = root.join(&normalized);
        match full_path.canonicalize() {
            Ok(resolved) => anyhow::ensure!(
                resolved.starts_with(root),
                "selected path escapes worktree root"
            ),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("failed to resolve `{path}`")),
        }
        // Propagate I/O failures. Only genuinely absent requested paths above
        // are omitted; a failed read cannot stand for complete-zero evidence.
        let bytes = fs::read(&full_path).with_context(|| format!("failed to read `{path}`"))?;
        entries.insert(
            path.clone(),
            CurrentFileEntry {
                path,
                content_oid: git_blob_oid(&bytes),
                extractable: true,
            },
        );
    }
    Ok(entries)
}

fn attach_rust_evidence(artifact: &mut GraphIndexArtifact, root: &Path) -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    for manifest in &artifact.file_manifests {
        let path = manifest.path.as_str();
        // Keep the D1 physical-source policy exact. Virtual/external lanes
        // must never be interpreted as disk paths.
        if path.starts_with("external://")
            || path.contains("#cell:")
            || Path::new(path)
                .extension()
                .is_none_or(|extension| extension != "rs")
        {
            continue;
        }
        anyhow::ensure!(
            Path::new(path)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "source is not a physical Rust path"
        );
        let source = fs::read(root.join(path))
            .with_context(|| format!("failed to read Rust evidence source `{path}`"))?;
        anyhow::ensure!(
            git_blob_oid(&source) == manifest.content_oid,
            "Rust evidence source OID mismatch for `{path}` (source changed during build)"
        );
        bytes.push((path, source));
    }
    let sources = bytes
        .iter()
        .map(|(file_path, bytes)| crate::RustDeclarationSource { file_path, bytes })
        .collect::<Vec<_>>();
    let evidence = crate::collect_rust_evidence(artifact, &sources)?;
    artifact.rust_evidence = Some(evidence);
    Ok(())
}

fn validate_macro_evidence(
    artifact: &GraphIndexArtifact,
    root: &Path,
    scopes: &[crate::MacroScopeEvidence],
) -> anyhow::Result<()> {
    let Some(set) = &artifact.macro_invocations else {
        return Ok(());
    };
    let paths: BTreeSet<_> = set
        .rows
        .iter()
        .map(|row| row.file_path.as_str())
        .chain(set.scopes.iter().map(|scope| scope.file_path.as_str()))
        .collect();
    let bytes: Vec<_> = paths
        .into_iter()
        .map(|path| fs::read(root.join(path)).map(|bytes| (path, bytes)))
        .collect::<Result<_, _>>()?;
    let sources: Vec<_> = bytes
        .iter()
        .map(|(file_path, bytes)| crate::MacroEvidenceSource { file_path, bytes })
        .collect();
    artifact.validate_macro_invocations(&sources, scopes)
}

fn discover_git_entries(
    root: &Path,
    _ctx: &GitCtx,
    allowed_extensions: &[&str],
) -> anyhow::Result<BTreeMap<String, CurrentFileEntry>> {
    let dirty_entries = git::status_dirty_paths(root)?;
    let dirty_paths: BTreeMap<String, DirtyEntry> = dirty_entries
        .into_iter()
        .filter(|entry| is_supported_path(&entry.path, allowed_extensions))
        .map(|entry| (entry.path.clone(), entry))
        .collect();
    let mut entries = BTreeMap::new();

    for tracked in git::ls_files_with_oids(root)? {
        if !is_supported_path(&tracked.path, allowed_extensions) {
            continue;
        }

        let content_oid = if tracked.is_gitlink {
            tracked.content_oid
        } else if dirty_paths.contains_key(&tracked.path) {
            let Some(content_oid) = read_worktree_content_oid(root, &tracked.path)? else {
                continue;
            };
            content_oid
        } else {
            tracked.content_oid
        };
        entries.insert(
            tracked.path.clone(),
            CurrentFileEntry {
                path: tracked.path,
                content_oid,
                extractable: !tracked.is_gitlink,
            },
        );
    }

    for dirty in dirty_paths.values() {
        if entries.contains_key(&dirty.path) {
            continue;
        }
        let Some(content_oid) = read_worktree_content_oid(root, &dirty.path)? else {
            continue;
        };
        entries.insert(
            dirty.path.clone(),
            CurrentFileEntry {
                path: dirty.path.clone(),
                content_oid,
                extractable: true,
            },
        );
    }

    Ok(entries)
}

fn discover_fs_entries(
    root: &Path,
    allowed_extensions: &[&str],
) -> anyhow::Result<BTreeMap<String, CurrentFileEntry>> {
    let mut entries = BTreeMap::new();
    for path in discover_files(root, allowed_extensions)? {
        let relative_path = relative_path(root, &path)?;
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read `{}`", path.display()))?;
        entries.insert(
            relative_path.clone(),
            CurrentFileEntry {
                path: relative_path,
                content_oid: git_blob_oid(&bytes),
                extractable: true,
            },
        );
    }
    Ok(entries)
}

fn read_worktree_content_oid(root: &Path, path: &str) -> anyhow::Result<Option<String>> {
    match fs::read(root.join(path)) {
        Ok(bytes) => Ok(Some(git_blob_oid(&bytes))),
        Err(err) if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::IsADirectory) => Ok(None),
        Err(err) => {
            Err(err).with_context(|| format!("failed to read `{}`", root.join(path).display()))
        }
    }
}

fn is_supported_path(path: &str, allowed_extensions: &[&str]) -> bool {
    if crate::discovery::is_graph_owned_path(Path::new(path)) {
        return false;
    }
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            allowed_extensions
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn relative_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    let relative = path.strip_prefix(root).with_context(|| {
        format!(
            "failed to make `{}` relative to `{}`",
            path.display(),
            root.display()
        )
    })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn content_oid_for(current_entries: &BTreeMap<String, CurrentFileEntry>, path: &str) -> String {
    current_entries
        .get(path)
        .map(|entry| entry.content_oid.clone())
        .unwrap_or_default()
}

struct ChangedPathDiff {
    buckets: BTreeMap<String, FileBucket>,
    changed_paths: Vec<PathBuf>,
    changed_or_added_paths: BTreeSet<String>,
    reused_buckets: usize,
}

fn changed_paths_from_previous(
    prev: &GraphIndexArtifact,
    root: &Path,
    current_entries: &BTreeMap<String, CurrentFileEntry>,
    prev_buckets: &BTreeMap<String, FileBucket>,
) -> ChangedPathDiff {
    let prev_content_oids: BTreeMap<_, _> = prev
        .file_manifests
        .iter()
        .map(|entry| (entry.path.as_str(), entry.content_oid.as_str()))
        .collect();

    let mut buckets = BTreeMap::new();
    let mut changed_paths = Vec::new();
    let mut changed_or_added_paths = BTreeSet::new();
    let mut reused_buckets = 0_usize;
    for current in current_entries.values() {
        if prev_content_oids
            .get(current.path.as_str())
            .is_some_and(|content_oid| *content_oid == current.content_oid)
        {
            if let Some(bucket) = prev_buckets.get(&current.path) {
                buckets.insert(current.path.clone(), bucket.clone());
                reused_buckets += 1;
                continue;
            }
        }
        if current.extractable {
            changed_or_added_paths.insert(current.path.clone());
            changed_paths.push(root.join(&current.path));
        }
    }

    ChangedPathDiff {
        buckets,
        changed_paths,
        changed_or_added_paths,
        reused_buckets,
    }
}

fn buckets_from_artifact(artifact: &GraphIndexArtifact) -> BTreeMap<String, FileBucket> {
    let mut manifest_by_path = BTreeMap::new();
    for manifest in &artifact.file_manifests {
        manifest_by_path.insert(manifest.path.as_str(), manifest);
    }

    let mut file_by_path = HashMap::new();
    let mut file_node_id_by_path = HashMap::new();
    for file in &artifact.files {
        file_by_path.entry(file.file_path.as_str()).or_insert(file);
    }
    if artifact.file_node_ids.len() == artifact.files.len() {
        for (file, node_id) in artifact.files.iter().zip(&artifact.file_node_ids) {
            file_node_id_by_path
                .entry(file.file_path.as_str())
                .or_insert(*node_id);
        }
    }

    let mut symbols_by_path: HashMap<&str, Vec<&GraphSymbolArtifact>> = HashMap::new();
    let mut symbol_node_id_by_stable_id = HashMap::new();
    if artifact.symbol_node_ids.len() == artifact.symbols.len() {
        for (symbol, node_id) in artifact.symbols.iter().zip(&artifact.symbol_node_ids) {
            symbol_node_id_by_stable_id.insert(symbol.stable_symbol_id.as_str(), *node_id);
        }
    }
    let mut source_paths: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut seen_source_paths = HashSet::new();
    for manifest in manifest_by_path.values() {
        insert_source_path(
            &mut source_paths,
            &mut seen_source_paths,
            manifest.stable_file_id.as_str(),
            manifest.path.as_str(),
        );
    }
    for symbol in &artifact.symbols {
        let path = symbol.file_path.as_str();
        symbols_by_path.entry(path).or_default().push(symbol);
        if manifest_by_path.contains_key(path) {
            insert_source_path(
                &mut source_paths,
                &mut seen_source_paths,
                symbol.stable_symbol_id.as_str(),
                path,
            );
        }
    }

    let mut edges_by_path: HashMap<&str, Vec<&GraphEdgeArtifact>> = HashMap::new();
    for edge in &artifact.edges {
        if let Some(paths) = source_paths.get(edge.source_stable_symbol_id.as_str()) {
            for path in paths {
                edges_by_path.entry(path).or_default().push(edge);
            }
        }
    }

    let mut buckets = BTreeMap::new();
    for (path, manifest) in manifest_by_path {
        buckets.insert(
            path.to_owned(),
            FileBucket {
                file: file_by_path.get(path).copied().cloned().unwrap_or_else(|| {
                    GraphFileArtifact {
                        stable_file_id: manifest.stable_file_id.clone(),
                        file_path: manifest.path.clone(),
                    }
                }),
                file_node_id: file_node_id_by_path.get(path).copied(),
                manifest: manifest.clone(),
                symbols: symbols_by_path
                    .get(path)
                    .map(|symbols| symbols.iter().copied().cloned().collect())
                    .unwrap_or_default(),
                symbol_node_ids: symbols_by_path
                    .get(path)
                    .map(|symbols| {
                        symbols
                            .iter()
                            .filter_map(|symbol| {
                                symbol_node_id_by_stable_id
                                    .get(symbol.stable_symbol_id.as_str())
                                    .copied()
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                edges: edges_by_path
                    .get(path)
                    .map(|edges| edges.iter().copied().cloned().collect())
                    .unwrap_or_default(),
                macro_invocations: artifact
                    .macro_invocations
                    .as_ref()
                    .map(|set| {
                        set.rows
                            .iter()
                            .filter(|row| row.file_path == path)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
                macro_scopes: artifact
                    .macro_invocations
                    .as_ref()
                    .map(|set| {
                        set.scopes
                            .iter()
                            .filter(|scope| scope.file_path == path)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
            },
        );
    }
    if let Some(symbols) = symbols_by_path.get(EXTERNAL_FILE_PATH) {
        let mut bucket = empty_bucket(EXTERNAL_FILE_PATH, "");
        bucket.symbols = symbols.iter().copied().cloned().collect();
        bucket.symbol_node_ids = symbols
            .iter()
            .filter_map(|symbol| {
                symbol_node_id_by_stable_id
                    .get(symbol.stable_symbol_id.as_str())
                    .copied()
            })
            .collect();
        buckets.insert(EXTERNAL_FILE_PATH.to_owned(), bucket);
    }
    buckets
}

fn insert_source_path<'a>(
    source_paths: &mut HashMap<&'a str, Vec<&'a str>>,
    seen_source_paths: &mut HashSet<(&'a str, &'a str)>,
    source_id: &'a str,
    path: &'a str,
) {
    if seen_source_paths.insert((source_id, path)) {
        source_paths.entry(source_id).or_default().push(path);
    }
}

fn add_missing_manifest_buckets(
    buckets: &mut BTreeMap<String, FileBucket>,
    current_entries: &BTreeMap<String, CurrentFileEntry>,
) {
    for current in current_entries.values() {
        buckets
            .entry(current.path.clone())
            .or_insert_with(|| empty_bucket(&current.path, &current.content_oid));
    }
}

fn empty_bucket(path: &str, content_oid: &str) -> FileBucket {
    let stable_file_id = stable_file_id_from_path(path);
    FileBucket {
        file: GraphFileArtifact {
            stable_file_id: stable_file_id.clone(),
            file_path: path.to_owned(),
        },
        file_node_id: None,
        manifest: GraphFileManifestEntry {
            stable_file_id,
            path: path.to_owned(),
            content_oid: content_oid.to_owned(),
            node_ids: Vec::new(),
        },
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        macro_invocations: Vec::new(),
        macro_scopes: Vec::new(),
    }
}

fn tombstones_from_removed_paths(
    prev: &GraphIndexArtifact,
    current_entries: &BTreeMap<String, CurrentFileEntry>,
) -> Vec<GraphTombstoneEntry> {
    let mut tombstones: Vec<_> = prev
        .file_manifests
        .iter()
        .filter(|entry| !current_entries.contains_key(&entry.path))
        .map(|entry| GraphTombstoneEntry {
            path: entry.path.clone(),
            stable_file_id: entry.stable_file_id.clone(),
        })
        .collect();
    tombstones.sort_by(|a, b| a.path.cmp(&b.path));
    tombstones
}

fn tombstones_from_changed_paths(
    prev: &GraphIndexArtifact,
    root: &Path,
    changed_paths: &[PathBuf],
    changed_facts: &GraphFacts,
) -> anyhow::Result<Vec<GraphTombstoneEntry>> {
    let current_symbol_ids_by_path = symbol_ids_by_path_from_facts(changed_facts);
    let changed_path_keys: BTreeSet<String> = changed_paths
        .iter()
        .map(|path| relative_path(root, path))
        .collect::<anyhow::Result<_>>()?;

    let mut tombstones = Vec::new();
    let empty_current_ids = BTreeSet::new();
    for path in changed_path_keys {
        let current_symbol_ids = current_symbol_ids_by_path
            .get(&path)
            .unwrap_or(&empty_current_ids);
        tombstones.extend(
            prev.symbols
                .iter()
                .filter(|symbol| symbol.file_path == path)
                .filter(|symbol| !current_symbol_ids.contains(&symbol.stable_symbol_id))
                .map(|symbol| GraphTombstoneEntry {
                    path: path.clone(),
                    stable_file_id: symbol.stable_symbol_id.clone(),
                }),
        );
    }
    sort_and_dedup_tombstones(&mut tombstones);
    Ok(tombstones)
}

fn tombstones_for_worktree(
    prev: &GraphIndexArtifact,
    current_entries: &BTreeMap<String, CurrentFileEntry>,
    changed_tombstones: Vec<GraphTombstoneEntry>,
) -> Vec<GraphTombstoneEntry> {
    let mut tombstones = tombstones_from_removed_paths(prev, current_entries);
    tombstones.extend(changed_tombstones);
    sort_and_dedup_tombstones(&mut tombstones);
    tombstones
}

fn sort_and_dedup_tombstones(tombstones: &mut Vec<GraphTombstoneEntry>) {
    tombstones.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.stable_file_id.cmp(&right.stable_file_id))
    });
    tombstones.dedup_by(|left, right| {
        left.path == right.path && left.stable_file_id == right.stable_file_id
    });
}

fn symbol_ids_by_path_from_facts(facts: &GraphFacts) -> BTreeMap<String, BTreeSet<String>> {
    let spans_by_id: HashMap<_, _> = facts
        .spans
        .iter()
        .map(|span| (span.span_id, span))
        .collect();
    let mut symbol_ids_by_path: BTreeMap<String, BTreeSet<String>> = facts
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
        .map(|node| (node.label.clone(), BTreeSet::new()))
        .collect();

    for node in &facts.nodes {
        if !is_symbol_tombstone_candidate(node.kind) {
            continue;
        }
        let Some(span_id) = node.source_span_id else {
            continue;
        };
        let Some(span) = spans_by_id.get(&span_id).copied() else {
            continue;
        };
        let Some(file_path) = file_path_for_span(facts, span) else {
            continue;
        };
        symbol_ids_by_path
            .entry(file_path)
            .or_default()
            .insert(node.stable_key.clone());
    }

    symbol_ids_by_path
}

fn is_symbol_tombstone_candidate(kind: NodeKind) -> bool {
    !matches!(kind, NodeKind::File | NodeKind::Commit | NodeKind::External)
}

fn removed_paths_from_previous(
    prev: &GraphIndexArtifact,
    current_entries: &BTreeMap<String, CurrentFileEntry>,
) -> BTreeSet<String> {
    prev.file_manifests
        .iter()
        .filter(|entry| !current_entries.contains_key(&entry.path))
        .map(|entry| entry.path.clone())
        .collect()
}

fn compose_artifact(
    mut buckets: BTreeMap<String, FileBucket>,
    current_entries: &BTreeMap<String, CurrentFileEntry>,
    manifest_version: String,
    tombstones: Vec<GraphTombstoneEntry>,
) -> GraphIndexArtifact {
    add_missing_manifest_buckets(&mut buckets, current_entries);
    rebind_cross_file_edges(&mut buckets);
    let graph_content_hash = compute_graph_content_hash(
        current_entries
            .values()
            .map(|entry| (entry.path.as_str(), entry.content_oid.as_str())),
    );
    rebuild_from_buckets(buckets, manifest_version, graph_content_hash, tombstones)
}

#[derive(Clone)]
struct RebindTarget {
    stable_symbol_id: String,
    file_path: String,
    entity_name: String,
    qualified_name: String,
    symbol_kind: String,
}

type FileImportIndex = BTreeMap<String, BTreeMap<String, Vec<RebindTarget>>>;

struct ExternalImportEntry {
    symbol: GraphSymbolArtifact,
    node_id: Option<NodeId>,
}

struct ExternalImportRegistry {
    entries_by_path: BTreeMap<String, ExternalImportEntry>,
    path_by_stable_id: HashMap<String, String>,
    used_paths: BTreeSet<String>,
    next_node_id: u64,
}

impl ExternalImportRegistry {
    fn from_buckets(buckets: &BTreeMap<String, FileBucket>) -> Self {
        let mut entries_by_path = BTreeMap::new();
        let mut path_by_stable_id = HashMap::new();

        for bucket in buckets.values() {
            let node_ids_are_aligned = bucket.symbol_node_ids.len() == bucket.symbols.len();
            for (index, symbol) in bucket.symbols.iter().enumerate() {
                if symbol.symbol_kind != "external" {
                    continue;
                }
                let path = external_symbol_path(symbol);
                let node_id = node_ids_are_aligned.then(|| bucket.symbol_node_ids[index]);
                path_by_stable_id.insert(symbol.stable_symbol_id.clone(), path.clone());
                entries_by_path
                    .entry(path)
                    .or_insert_with(|| ExternalImportEntry {
                        symbol: symbol.clone(),
                        node_id,
                    });
            }
        }

        Self {
            entries_by_path,
            path_by_stable_id,
            used_paths: BTreeSet::new(),
            next_node_id: max_existing_bucket_node_id(buckets)
                .and_then(|id| id.checked_add(1))
                .unwrap_or(1),
        }
    }

    fn bind_external(&mut self, path: String, name: String) -> String {
        let stable_symbol_id = stable_symbol_id_for_external_path(&path);
        self.used_paths.insert(path.clone());
        self.path_by_stable_id
            .insert(stable_symbol_id.clone(), path.clone());
        self.entries_by_path
            .entry(path.clone())
            .or_insert_with(|| ExternalImportEntry {
                symbol: GraphSymbolArtifact {
                    stable_symbol_id: stable_symbol_id.clone(),
                    file_path: EXTERNAL_FILE_PATH.to_owned(),
                    byte_range: [0, 0],
                    line_range: [0, 0],
                    entity_name: name,
                    qualified_name: path,
                    symbol_kind: "external".to_owned(),
                    anchor_hash: String::new(),
                    enclosing_scope: None,
                },
                node_id: None,
            });
        stable_symbol_id
    }

    fn mark_stable_id_used(&mut self, stable_symbol_id: &str) -> bool {
        let Some(path) = self.path_by_stable_id.get(stable_symbol_id).cloned() else {
            return false;
        };
        self.used_paths.insert(path);
        true
    }

    fn write_to_buckets(mut self, buckets: &mut BTreeMap<String, FileBucket>) {
        if self.used_paths.is_empty() {
            buckets.remove(EXTERNAL_FILE_PATH);
            return;
        }

        let mut bucket = empty_bucket(EXTERNAL_FILE_PATH, "");
        for path in self.used_paths.clone() {
            let Some(entry) = self.entries_by_path.remove(&path) else {
                continue;
            };
            let node_id = entry.node_id.unwrap_or_else(|| {
                let node_id = NodeId(self.next_node_id);
                self.next_node_id = self
                    .next_node_id
                    .checked_add(1)
                    .expect("exhausted synthetic external NodeId space");
                node_id
            });
            bucket.symbols.push(entry.symbol);
            bucket.symbol_node_ids.push(node_id);
        }
        buckets.insert(EXTERNAL_FILE_PATH.to_owned(), bucket);
    }
}

fn rebind_cross_file_edges(buckets: &mut BTreeMap<String, FileBucket>) {
    let workspace_index = import_workspace_index_from_buckets(buckets);
    let mut external_imports = ExternalImportRegistry::from_buckets(buckets);
    let symbols_by_entity_name = symbols_by_entity_name_from_buckets(buckets);
    let mut current_symbol_ids = current_symbol_ids_from_buckets(buckets);
    let (file_import_index, import_ambiguous_unresolved) = rebind_import_edges(
        buckets,
        &workspace_index,
        &symbols_by_entity_name,
        &mut external_imports,
        &mut current_symbol_ids,
    );
    let remaining_ambiguous_unresolved = rebind_remaining_edges(
        buckets,
        &workspace_index,
        &symbols_by_entity_name,
        &file_import_index,
        &mut external_imports,
        &mut current_symbol_ids,
    );
    external_imports.write_to_buckets(buckets);
    let ambiguous_unresolved = import_ambiguous_unresolved + remaining_ambiguous_unresolved;
    if ambiguous_unresolved > 0 {
        tracing::info!(
            ambiguous_unresolved,
            "spur-graph: left ambiguous cross-file edges unresolved"
        );
    }
}

fn current_symbol_ids_from_buckets(buckets: &BTreeMap<String, FileBucket>) -> HashSet<String> {
    buckets
        .values()
        .flat_map(|bucket| {
            std::iter::once(bucket.file.stable_file_id.clone()).chain(
                bucket
                    .symbols
                    .iter()
                    .map(|symbol| symbol.stable_symbol_id.clone()),
            )
        })
        .collect()
}

fn symbols_by_entity_name_from_buckets(
    buckets: &BTreeMap<String, FileBucket>,
) -> BTreeMap<String, Vec<RebindTarget>> {
    let mut symbols_by_entity_name: BTreeMap<String, Vec<RebindTarget>> = BTreeMap::new();
    for bucket in buckets.values() {
        for symbol in &bucket.symbols {
            if symbol.symbol_kind == "external" {
                continue;
            }
            symbols_by_entity_name
                .entry(symbol.entity_name.clone())
                .or_default()
                .push(RebindTarget {
                    stable_symbol_id: symbol.stable_symbol_id.clone(),
                    file_path: symbol.file_path.clone(),
                    entity_name: symbol.entity_name.clone(),
                    qualified_name: symbol.qualified_name.clone(),
                    symbol_kind: symbol.symbol_kind.clone(),
                });
        }
    }
    symbols_by_entity_name
}

fn rebind_import_edges(
    buckets: &mut BTreeMap<String, FileBucket>,
    workspace_index: &ImportWorkspaceIndex,
    symbols_by_entity_name: &BTreeMap<String, Vec<RebindTarget>>,
    external_imports: &mut ExternalImportRegistry,
    current_symbol_ids: &mut HashSet<String>,
) -> (FileImportIndex, usize) {
    let workspace_targets_by_stable_id = workspace_targets_by_stable_id(symbols_by_entity_name);
    let workspace_files_by_path = buckets
        .values()
        .filter(|bucket| bucket.file.file_path != EXTERNAL_FILE_PATH)
        .map(|bucket| {
            (
                bucket.file.file_path.clone(),
                bucket.file.stable_file_id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut file_import_index = FileImportIndex::new();
    let mut ambiguous_unresolved = 0usize;

    for bucket in buckets.values_mut() {
        let source_file_path = bucket.file.file_path.as_str();
        for edge in &mut bucket.edges {
            if edge.relation != RelationKind::Imports {
                continue;
            }
            let Some(target_label) = edge.target_label.clone() else {
                continue;
            };
            if is_web_import_source_file(source_file_path) {
                let workspace_file = edge
                    .import_path
                    .as_deref()
                    .and_then(|path| web_import_target_path(source_file_path, path))
                    .and_then(|path| workspace_files_by_path.get(&path));
                if let Some(stable_file_id) = workspace_file {
                    edge.target_stable_symbol_id = Some(stable_file_id.clone());
                    edge.bind_method = Some("import_path".to_owned());
                } else if !bind_external_import_edge(
                    edge,
                    source_file_path,
                    workspace_index,
                    external_imports,
                    current_symbol_ids,
                ) {
                    edge.target_stable_symbol_id = None;
                    edge.bind_method = None;
                }
                continue;
            }
            let resolution_is_stamped = resolution_is_stamped(edge);
            let skip_rebind = edge.edge_kind == Some(GraphEdgeKind::CallsDyn)
                || target_label.contains("::")
                || resolution_is_stamped;
            if edge.target_stable_symbol_id.is_some() && skip_rebind {
                if edge.bind_method.as_deref() == Some("external")
                    && !edge
                        .target_stable_symbol_id
                        .as_deref()
                        .is_some_and(|id| external_imports.mark_stable_id_used(id))
                {
                    bind_external_import_edge(
                        edge,
                        source_file_path,
                        workspace_index,
                        external_imports,
                        current_symbol_ids,
                    );
                }
                drop_dangling_stamped_target(edge, current_symbol_ids);
                record_resolved_workspace_import(
                    &mut file_import_index,
                    source_file_path,
                    &target_label,
                    edge,
                    &workspace_targets_by_stable_id,
                );
                continue;
            }
            let Some(matches) = symbols_by_entity_name.get(target_label.as_str()) else {
                if bind_external_import_edge(
                    edge,
                    source_file_path,
                    workspace_index,
                    external_imports,
                    current_symbol_ids,
                ) {
                    continue;
                }
                edge.target_stable_symbol_id = None;
                continue;
            };
            let (matches, bind_method) = import_rebind_candidates(
                matches,
                source_file_path,
                &target_label,
                edge.import_path.as_deref(),
                workspace_index,
            );
            // Candidate filtering above can remove every match, so the
            // bucket lookup being non-empty does not guarantee `matches` is.
            let Some(resolved) = matches.first() else {
                if bind_external_import_edge(
                    edge,
                    source_file_path,
                    workspace_index,
                    external_imports,
                    current_symbol_ids,
                ) {
                    continue;
                }
                edge.target_stable_symbol_id = None;
                continue;
            };
            if matches.len() > 1 {
                if bind_external_import_edge(
                    edge,
                    source_file_path,
                    workspace_index,
                    external_imports,
                    current_symbol_ids,
                ) {
                    continue;
                }
                ambiguous_unresolved += 1;
                tracing::debug!(
                    target_label = target_label.as_str(),
                    candidates = matches.len(),
                    "spur-graph: ambiguous cross-file target_label; leaving unresolved"
                );
                edge.target_stable_symbol_id = None;
            } else {
                edge.target_stable_symbol_id = Some(resolved.stable_symbol_id.clone());
                if let Some(bind_method) = bind_method {
                    edge.bind_method = Some(bind_method.to_owned());
                }
            }
            record_resolved_workspace_import(
                &mut file_import_index,
                source_file_path,
                &target_label,
                edge,
                &workspace_targets_by_stable_id,
            );
        }
    }

    sort_file_import_index(&mut file_import_index);
    (file_import_index, ambiguous_unresolved)
}

fn rebind_remaining_edges(
    buckets: &mut BTreeMap<String, FileBucket>,
    workspace_index: &ImportWorkspaceIndex,
    symbols_by_entity_name: &BTreeMap<String, Vec<RebindTarget>>,
    file_import_index: &FileImportIndex,
    external_imports: &mut ExternalImportRegistry,
    current_symbol_ids: &mut HashSet<String>,
) -> usize {
    let mut ambiguous_unresolved = 0usize;

    for bucket in buckets.values_mut() {
        let source_file_path = bucket.file.file_path.as_str();
        for edge in &mut bucket.edges {
            if edge.relation == RelationKind::Imports {
                continue;
            }
            if edge.relation == RelationKind::Links {
                edge.target_stable_symbol_id = None;
                edge.bind_method = None;
                continue;
            }
            let Some(target_label) = edge.target_label.clone() else {
                continue;
            };
            let resolution_is_stamped = resolution_is_stamped(edge);
            let skip_rebind = edge.edge_kind == Some(GraphEdgeKind::CallsDyn)
                || target_label.contains("::")
                || resolution_is_stamped;
            if edge.target_stable_symbol_id.is_some() && skip_rebind {
                if edge.bind_method.as_deref() == Some("external")
                    && !edge
                        .target_stable_symbol_id
                        .as_deref()
                        .is_some_and(|id| external_imports.mark_stable_id_used(id))
                {
                    bind_external_import_edge(
                        edge,
                        source_file_path,
                        workspace_index,
                        external_imports,
                        current_symbol_ids,
                    );
                }
                drop_dangling_stamped_target(edge, current_symbol_ids);
                continue;
            }
            // Physical Rust receiver/qualified lookup already adjudicated this call.
            // Nominal scope is evidence, not trait/autoderef authority. Neither
            // a same-directory singleton nor an import license may resurrect
            // a target rejected by that lookup. Grounded stamped targets above
            // and explicit UFCS paths retain their established behavior.
            if language_family(source_file_path) == Some("rust")
                && edge.relation == RelationKind::Calls
                && (edge.receiver_text.is_some() || edge.scope_text.is_some())
            {
                edge.target_stable_symbol_id = None;
                edge.bind_method = None;
                continue;
            }
            if edge.edge_kind == Some(GraphEdgeKind::ReferencesAddress) {
                ambiguous_unresolved += rebind_address_reference_edge(
                    edge,
                    source_file_path,
                    &target_label,
                    symbols_by_entity_name,
                );
                continue;
            }
            let Some(matches) = symbols_by_entity_name.get(target_label.as_str()) else {
                edge.target_stable_symbol_id = None;
                continue;
            };
            let matches = if let Some(allowed) = rebind_candidate_kinds(edge.relation) {
                matches
                    .iter()
                    .filter(|target| allowed.contains(&target.symbol_kind.as_str()))
                    .collect::<Vec<_>>()
            } else {
                matches.iter().collect::<Vec<_>>()
            };
            // Candidate filtering above can remove every match, so the
            // bucket lookup being non-empty does not guarantee `matches` is.
            let Some(resolved) = matches.first() else {
                edge.target_stable_symbol_id = None;
                continue;
            };
            if matches.len() > 1 {
                if let Some(resolved) = import_licensed_call_target(
                    edge,
                    source_file_path,
                    &target_label,
                    &matches,
                    file_import_index,
                ) {
                    edge.target_stable_symbol_id = Some(resolved.stable_symbol_id.clone());
                    edge.bind_method = Some("import_licensed".to_owned());
                } else {
                    ambiguous_unresolved += 1;
                    tracing::debug!(
                        target_label = target_label.as_str(),
                        candidates = matches.len(),
                        "spur-graph: ambiguous cross-file target_label; leaving unresolved"
                    );
                    edge.target_stable_symbol_id = None;
                }
            } else if (edge.relation == RelationKind::Calls
                || edge.relation == RelationKind::References)
                && resolved.symbol_kind == "method"
                && !same_directory_path(&resolved.file_path, source_file_path)
            {
                edge.target_stable_symbol_id = None;
            } else if (edge.relation == RelationKind::Calls
                || edge.relation == RelationKind::References)
                && resolved.symbol_kind == "function"
                && !function_singleton_safe(source_file_path, &resolved.file_path)
            {
                if let Some(resolved) = import_licensed_call_target(
                    edge,
                    source_file_path,
                    &target_label,
                    &matches,
                    file_import_index,
                ) {
                    edge.target_stable_symbol_id = Some(resolved.stable_symbol_id.clone());
                    edge.bind_method = Some("import_licensed".to_owned());
                } else {
                    edge.target_stable_symbol_id = None;
                }
            } else {
                edge.target_stable_symbol_id = Some(resolved.stable_symbol_id.clone());
                if edge.relation == RelationKind::Calls && resolved.symbol_kind == "function" {
                    edge.bind_method
                        .get_or_insert_with(|| "singleton".to_owned());
                }
            }
        }
    }
    ambiguous_unresolved
}

fn rebind_address_reference_edge(
    edge: &mut GraphEdgeArtifact,
    source_file_path: &str,
    target_label: &str,
    symbols_by_entity_name: &BTreeMap<String, Vec<RebindTarget>>,
) -> usize {
    let candidates: Vec<&RebindTarget> = if language_family(source_file_path) == Some("hcl") {
        symbols_by_entity_name
            .get(target_label)
            .into_iter()
            .flatten()
            .filter(|target| matches!(target.symbol_kind.as_str(), "resource" | "constant"))
            .filter(|target| language_family(&target.file_path) == Some("hcl"))
            .collect()
    } else {
        Vec::new()
    };
    let module_scoped: Vec<&RebindTarget> = candidates
        .iter()
        .copied()
        .filter(|target| same_directory_path(&target.file_path, source_file_path))
        .collect();
    match module_scoped.as_slice() {
        [target] => {
            edge.target_stable_symbol_id = Some(target.stable_symbol_id.clone());
            edge.bind_method = Some("address_module_scope".to_owned());
            0
        }
        [] => match candidates.as_slice() {
            [target] => {
                edge.target_stable_symbol_id = Some(target.stable_symbol_id.clone());
                edge.bind_method = Some("address_singleton".to_owned());
                0
            }
            [] => {
                edge.target_stable_symbol_id = None;
                edge.bind_method = None;
                0
            }
            candidates => {
                tracing::debug!(
                    target_label,
                    candidates = candidates.len(),
                    "spur-graph: ambiguous address reference target; leaving unresolved"
                );
                edge.target_stable_symbol_id = None;
                edge.bind_method = None;
                1
            }
        },
        duplicates => {
            tracing::debug!(
                target_label,
                candidates = duplicates.len(),
                "spur-graph: duplicate address within one module directory; leaving unresolved"
            );
            edge.target_stable_symbol_id = None;
            edge.bind_method = None;
            1
        }
    }
}

fn drop_dangling_stamped_target(
    edge: &mut GraphEdgeArtifact,
    current_symbol_ids: &HashSet<String>,
) {
    if resolution_is_stamped(edge)
        && edge
            .target_stable_symbol_id
            .as_deref()
            .is_some_and(|target_id| !current_symbol_ids.contains(target_id))
    {
        edge.target_stable_symbol_id = None;
    }
}

fn import_licensed_call_target<'a>(
    edge: &GraphEdgeArtifact,
    source_file_path: &str,
    target_label: &str,
    matches: &[&'a RebindTarget],
    file_import_index: &FileImportIndex,
) -> Option<&'a RebindTarget> {
    if edge.relation != RelationKind::Calls
        || edge.edge_kind == Some(GraphEdgeKind::CallsDyn)
        || edge.target_stable_symbol_id.is_some()
    {
        return None;
    }
    let imported_targets = file_import_index.get(source_file_path)?.get(target_label)?;
    let [imported_target] = imported_targets.as_slice() else {
        return None;
    };
    let mut licensed_matches = matches
        .iter()
        .copied()
        .filter(|target| target.stable_symbol_id == imported_target.stable_symbol_id);
    let resolved = licensed_matches.next()?;
    if licensed_matches.next().is_some() {
        return None;
    }
    if resolved.symbol_kind != "function" {
        return None;
    }
    debug_assert_eq!(resolved.symbol_kind, "function");
    if !same_import_rebind_language(source_file_path, &resolved.file_path) {
        return None;
    }
    debug_assert!(same_import_rebind_language(
        source_file_path,
        &resolved.file_path
    ));
    if function_singleton_safe(source_file_path, &resolved.file_path) {
        return None;
    }
    Some(resolved)
}

fn resolution_is_stamped(edge: &GraphEdgeArtifact) -> bool {
    matches!(
        edge.bind_method.as_deref(),
        Some(
            "fqn"
                | "scope_match"
                | "singleton"
                | "macro_body_singleton"
                | "relational"
                | "constructs_type_singleton"
                | "method_crate_singleton"
                | "import_path"
                | "import_licensed"
                | "external"
        )
    )
}

fn workspace_targets_by_stable_id(
    symbols_by_entity_name: &BTreeMap<String, Vec<RebindTarget>>,
) -> BTreeMap<&str, &RebindTarget> {
    symbols_by_entity_name
        .values()
        .flat_map(|targets| targets.iter())
        .map(|target| (target.stable_symbol_id.as_str(), target))
        .collect()
}

fn record_resolved_workspace_import(
    file_import_index: &mut FileImportIndex,
    source_file_path: &str,
    imported_entity_name: &str,
    edge: &GraphEdgeArtifact,
    workspace_targets_by_stable_id: &BTreeMap<&str, &RebindTarget>,
) {
    let Some(target_stable_symbol_id) = edge.target_stable_symbol_id.as_deref() else {
        return;
    };
    let Some(target) = workspace_targets_by_stable_id.get(target_stable_symbol_id) else {
        return;
    };
    let targets = file_import_index
        .entry(source_file_path.to_owned())
        .or_default()
        .entry(imported_entity_name.to_owned())
        .or_default();
    if targets
        .iter()
        .any(|existing| existing.stable_symbol_id == target.stable_symbol_id)
    {
        return;
    }
    targets.push((*target).clone());
}

fn sort_file_import_index(file_import_index: &mut FileImportIndex) {
    for targets in file_import_index
        .values_mut()
        .flat_map(|imports| imports.values_mut())
    {
        targets.sort_by(|left, right| {
            left.stable_symbol_id
                .cmp(&right.stable_symbol_id)
                .then_with(|| left.file_path.cmp(&right.file_path))
                .then_with(|| left.entity_name.cmp(&right.entity_name))
                .then_with(|| left.qualified_name.cmp(&right.qualified_name))
                .then_with(|| left.symbol_kind.cmp(&right.symbol_kind))
        });
    }
}

fn import_rebind_candidates<'a>(
    matches: &'a [RebindTarget],
    source_file_path: &str,
    target_label: &str,
    import_path: Option<&str>,
    workspace_index: &ImportWorkspaceIndex,
) -> (Vec<&'a RebindTarget>, Option<&'static str>) {
    let raw_candidates = matches
        .iter()
        .filter(|target| is_import_rebind_candidate_kind(&target.symbol_kind))
        .collect::<Vec<_>>();

    if let Some(import_path) = import_path {
        match rust_workspace_import_path(
            import_path,
            target_label,
            source_file_path,
            workspace_index,
        ) {
            Some(RustWorkspaceImportPath::WorkspaceCrate {
                crate_name,
                remaining_segments,
            }) => {
                return (
                    raw_candidates
                        .into_iter()
                        .filter(|target| {
                            is_import_rebind_type_like_candidate_kind(&target.symbol_kind)
                        })
                        .filter(|target| {
                            same_import_rebind_language(source_file_path, &target.file_path)
                        })
                        .filter(|target| {
                            rust_workspace_import_matches_target(
                                target,
                                &crate_name,
                                &remaining_segments,
                            )
                        })
                        .collect(),
                    Some("import_path"),
                );
            }
            Some(RustWorkspaceImportPath::External) => {
                return (Vec::new(), None);
            }
            None => {}
        }

        return (
            raw_candidates
                .into_iter()
                .filter(|target| is_import_rebind_type_like_candidate_kind(&target.symbol_kind))
                .filter(|target| same_import_rebind_language(source_file_path, &target.file_path))
                .collect(),
            None,
        );
    }

    let mut candidates = raw_candidates
        .iter()
        .copied()
        .filter(|target| is_import_rebind_type_like_candidate_kind(&target.symbol_kind))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates = raw_candidates
            .into_iter()
            .filter(|target| is_import_rebind_fallback_candidate_kind(&target.symbol_kind))
            .collect();
    }
    (candidates, None)
}

enum RustWorkspaceImportPath {
    WorkspaceCrate {
        crate_name: String,
        remaining_segments: Vec<String>,
    },
    External,
}

fn rust_workspace_import_path(
    import_path: &str,
    target_label: &str,
    source_file_path: &str,
    workspace_index: &ImportWorkspaceIndex,
) -> Option<RustWorkspaceImportPath> {
    if import_rebind_language(source_file_path) != Some("rust") {
        return None;
    }
    let segments = rust_import_path_segments(import_path, target_label);
    let (first, rest) = segments.split_first()?;
    if matches!(first.as_str(), "crate" | "self" | "super" | "$crate") {
        return None;
    }
    let crate_name = normalize_rust_crate_name(first);
    if rust_workspace_crate_name_from_path(source_file_path).as_deref() == Some(crate_name.as_str())
    {
        return None;
    }
    if workspace_index.rust_crates.contains(&crate_name) {
        return Some(RustWorkspaceImportPath::WorkspaceCrate {
            crate_name,
            remaining_segments: rest.to_vec(),
        });
    }
    Some(RustWorkspaceImportPath::External)
}

fn rust_import_path_segments(import_path: &str, target_label: &str) -> Vec<String> {
    let mut path = import_path.trim();
    if let Some((prefix, _)) = path.split_once('{') {
        path = prefix.trim_end_matches(':').trim();
    }
    let mut segments = path
        .split("::")
        .map(clean_rust_path_segment)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let target_label = clean_rust_path_segment(target_label);
    if !target_label.is_empty()
        && segments
            .last()
            .is_none_or(|segment| segment != &target_label)
    {
        segments.push(target_label);
    }
    segments
}

fn rust_workspace_import_matches_target(
    target: &RebindTarget,
    crate_name: &str,
    remaining_segments: &[String],
) -> bool {
    if remaining_segments.is_empty() {
        return false;
    }
    if rust_workspace_crate_name_from_path(&target.file_path).as_deref() != Some(crate_name) {
        return false;
    }
    let remaining_path = remaining_segments.join("::");
    if remaining_segments.len() == 1
        && (clean_rust_path_segment(&target.entity_name) == remaining_path
            || clean_rust_qualified_name(&target.qualified_name) == remaining_path)
    {
        return true;
    }
    rust_workspace_target_import_paths(target).contains(&remaining_path)
}

fn rust_workspace_target_import_paths(target: &RebindTarget) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let qualified_name = clean_rust_qualified_name(&target.qualified_name);
    let entity_name = clean_rust_path_segment(&target.entity_name);
    let module_segments = rust_file_module_segments(&target.file_path);

    if module_segments.is_empty() {
        insert_non_empty(&mut paths, qualified_name);
        insert_non_empty(&mut paths, entity_name);
        return paths;
    }

    let module_path = module_segments.join("::");
    if qualified_name == module_path || qualified_name.starts_with(&format!("{module_path}::")) {
        insert_non_empty(&mut paths, qualified_name);
    } else {
        insert_qualified_under_module(&mut paths, &module_path, &qualified_name);
    }
    insert_qualified_under_module(&mut paths, &module_path, &entity_name);
    paths
}

fn insert_qualified_under_module(paths: &mut BTreeSet<String>, module_path: &str, name: &str) {
    if name.is_empty() {
        return;
    }
    paths.insert(format!("{module_path}::{name}"));
}

fn insert_non_empty(paths: &mut BTreeSet<String>, value: String) {
    if !value.is_empty() {
        paths.insert(value);
    }
}

fn clean_rust_qualified_name(qualified_name: &str) -> String {
    qualified_name
        .split("::")
        .map(clean_rust_path_segment)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("::")
}

fn clean_rust_path_segment(segment: &str) -> String {
    segment.trim().trim_start_matches("r#").to_owned()
}

fn rust_workspace_crate_name_from_path(path: &str) -> Option<String> {
    let segments = path.split('/').collect::<Vec<_>>();
    let crates_index = segments.iter().position(|segment| *segment == "crates")?;
    let crate_name = segments.get(crates_index + 1)?;
    if crate_name.is_empty() || *crate_name == "src" {
        return None;
    }
    Some(normalize_rust_crate_name(crate_name))
}

fn normalize_rust_crate_name(crate_name: &str) -> String {
    clean_rust_path_segment(crate_name).replace('-', "_")
}

fn rust_file_module_segments(path: &str) -> Vec<String> {
    let Some(src_relative) = rust_src_relative_path(path) else {
        return Vec::new();
    };
    if matches!(src_relative, "lib.rs" | "main.rs") {
        return Vec::new();
    }
    let module_path = src_relative
        .strip_suffix("/mod.rs")
        .or_else(|| src_relative.strip_suffix(".rs"))
        .unwrap_or(src_relative);
    module_path
        .split('/')
        .map(clean_rust_path_segment)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn rust_src_relative_path(path: &str) -> Option<&str> {
    path.strip_prefix("src/")
        .or_else(|| path.split_once("/src/").map(|(_, path)| path))
}

fn bind_external_import_edge(
    edge: &mut GraphEdgeArtifact,
    source_file_path: &str,
    workspace_index: &ImportWorkspaceIndex,
    external_imports: &mut ExternalImportRegistry,
    current_symbol_ids: &mut HashSet<String>,
) -> bool {
    if edge.relation != RelationKind::Imports {
        return false;
    }
    let Some(import_path) = edge.import_path.as_deref() else {
        return false;
    };
    let ImportOriginClassification::External { path, name, .. } =
        classify_import_origin(import_path, source_file_path, workspace_index)
    else {
        return false;
    };

    let stable_symbol_id = external_imports.bind_external(path, name);
    current_symbol_ids.insert(stable_symbol_id.clone());
    edge.target_stable_symbol_id = Some(stable_symbol_id);
    edge.bind_method = Some("external".to_owned());
    true
}

fn import_workspace_index_from_buckets(
    buckets: &BTreeMap<String, FileBucket>,
) -> ImportWorkspaceIndex {
    let mut index = ImportWorkspaceIndex::default();
    for path in buckets.keys().map(String::as_str) {
        if path == EXTERNAL_FILE_PATH {
            continue;
        }
        add_rust_workspace_crate(path, &mut index);
        add_python_workspace_modules(path, &mut index);
        add_javascript_workspace_roots(path, &mut index);
    }
    index
}

fn add_rust_workspace_crate(path: &str, index: &mut ImportWorkspaceIndex) {
    if import_rebind_language(path) != Some("rust") {
        return;
    }
    let segments = path.split('/').collect::<Vec<_>>();
    let Some(crates_index) = segments.iter().position(|segment| *segment == "crates") else {
        return;
    };
    let Some(crate_name) = segments.get(crates_index + 1) else {
        return;
    };
    if crate_name.is_empty() || *crate_name == "src" {
        return;
    }
    index
        .rust_crates
        .insert(normalize_rust_crate_name(crate_name));
}

fn add_python_workspace_modules(path: &str, index: &mut ImportWorkspaceIndex) {
    if !path.ends_with(".py") {
        return;
    }
    let mut module_path = path.trim_end_matches(".py");
    if let Some(stripped) = module_path.strip_suffix("/__init__") {
        module_path = stripped;
    }
    let segments = module_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    add_python_module_segments(&segments, index);
    if matches!(segments.first().copied(), Some("src" | "lib" | "python")) {
        add_python_module_segments(&segments[1..], index);
    }
}

fn add_python_module_segments(segments: &[&str], index: &mut ImportWorkspaceIndex) {
    let mut module = String::new();
    for segment in segments {
        if module.is_empty() {
            module.push_str(segment);
        } else {
            module.push('.');
            module.push_str(segment);
        }
        index.python_modules.insert(module.clone());
    }
}

fn add_javascript_workspace_roots(path: &str, index: &mut ImportWorkspaceIndex) {
    if import_rebind_language(path) != Some("javascript") {
        return;
    }
    let dirs = path.rsplit_once('/').map(|(dirs, _)| dirs).unwrap_or("");
    let segments = dirs
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let mut prefix = String::new();
    for segment in &segments {
        if prefix.is_empty() {
            prefix.push_str(segment);
        } else {
            prefix.push('/');
            prefix.push_str(segment);
        }
        index.javascript_workspace_roots.insert(prefix.clone());
    }
    for (idx, segment) in segments.iter().enumerate() {
        if *segment == "src" {
            index.javascript_workspace_roots.insert("src".to_owned());
        } else if *segment == "packages" {
            if let Some(package) = segments.get(idx + 1) {
                index
                    .javascript_workspace_roots
                    .insert(format!("packages/{package}"));
            }
        }
    }
    if !segments.is_empty() {
        index.javascript_path_aliases.insert("@/".to_owned());
        index.javascript_path_aliases.insert("@/*".to_owned());
    }
}

fn rebind_candidate_kinds(relation: RelationKind) -> Option<&'static [&'static str]> {
    match relation {
        RelationKind::Extends => Some(&["trait", "interface", "class"]),
        RelationKind::Implements => Some(&["trait", "interface"]),
        RelationKind::Calls => Some(&["function", "method"]),
        _ => None,
    }
}

fn is_import_rebind_candidate_kind(kind: &str) -> bool {
    is_import_rebind_type_like_candidate_kind(kind)
        || is_import_rebind_fallback_candidate_kind(kind)
}

fn is_import_rebind_type_like_candidate_kind(kind: &str) -> bool {
    // Impl blocks share the type's import surface; they are containers, not
    // independently importable definitions.
    matches!(
        kind,
        "module"
            | "function"
            | "class"
            | "interface"
            | "struct"
            | "enum"
            | "trait"
            | "type_alias"
            | "macro"
    )
}

fn is_import_rebind_fallback_candidate_kind(kind: &str) -> bool {
    NodeKind::from_discriminator(kind).is_some_and(is_import_resolution_fallback_candidate_kind)
}

fn same_import_rebind_language(source_file_path: &str, target_file_path: &str) -> bool {
    let Some(source_language) = import_rebind_language(source_file_path) else {
        return false;
    };
    import_rebind_language(target_file_path) == Some(source_language)
}

fn import_rebind_language(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("rs") => Some("rust"),
        Some("py") => Some("python"),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") => Some("javascript"),
        _ => None,
    }
}

fn same_directory_path(left: &str, right: &str) -> bool {
    parent_path(left) == parent_path(right)
}

fn parent_path(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

fn external_symbol_path(symbol: &GraphSymbolArtifact) -> String {
    if symbol.qualified_name.is_empty() {
        symbol.entity_name.clone()
    } else {
        symbol.qualified_name.clone()
    }
}

fn max_existing_bucket_node_id(buckets: &BTreeMap<String, FileBucket>) -> Option<u64> {
    buckets
        .values()
        .filter_map(|bucket| bucket.file_node_id.map(NodeId::get))
        .chain(
            buckets
                .values()
                .flat_map(|bucket| bucket.symbol_node_ids.iter().map(|id| id.get())),
        )
        .chain(
            buckets
                .values()
                .flat_map(|bucket| bucket.manifest.node_ids.iter().map(|id| id.get())),
        )
        .max()
}

fn rebuild_from_buckets(
    mut buckets: BTreeMap<String, FileBucket>,
    manifest_version: String,
    graph_content_hash: String,
    tombstones: Vec<GraphTombstoneEntry>,
) -> GraphIndexArtifact {
    let mut files = Vec::new();
    let mut file_node_ids = Vec::new();
    let mut symbols = Vec::new();
    let mut symbol_node_ids = Vec::new();
    let mut manifests = Vec::new();
    let mut edges = Vec::new();
    let mut macro_rows = Vec::new();
    let mut macro_scopes = Vec::new();

    for bucket in buckets.values_mut() {
        sort_bucket_symbols(bucket);
        bucket.manifest.node_ids.sort_by_key(|id| id.get());
        bucket
            .edges
            .sort_by(|a, b| edge_sort_key(a).cmp(&edge_sort_key(b)));
        if bucket.file.file_path == EXTERNAL_FILE_PATH {
            if bucket.symbol_node_ids.len() == bucket.symbols.len() {
                symbols.extend(
                    bucket
                        .symbols
                        .iter()
                        .cloned()
                        .zip(bucket.symbol_node_ids.iter().copied().map(Some)),
                );
            } else {
                symbols.extend(bucket.symbols.iter().cloned().map(|symbol| (symbol, None)));
            }
            continue;
        }
        files.push((bucket.file.clone(), bucket.file_node_id));
        if bucket.symbol_node_ids.len() == bucket.symbols.len() {
            symbols.extend(
                bucket
                    .symbols
                    .iter()
                    .cloned()
                    .zip(bucket.symbol_node_ids.iter().copied().map(Some)),
            );
        } else {
            symbols.extend(bucket.symbols.iter().cloned().map(|symbol| (symbol, None)));
        }
        manifests.push(bucket.manifest.clone());
        edges.extend(bucket.edges.clone());
        macro_rows.extend(bucket.macro_invocations.clone());
        macro_scopes.extend(bucket.macro_scopes.clone());
    }

    files.sort_by(|a, b| a.0.file_path.cmp(&b.0.file_path));
    symbols.sort_by(|a, b| {
        a.0.file_path
            .cmp(&b.0.file_path)
            .then(a.0.byte_range.cmp(&b.0.byte_range))
            .then(a.0.entity_name.cmp(&b.0.entity_name))
            .then(a.0.stable_symbol_id.cmp(&b.0.stable_symbol_id))
    });
    manifests.sort_by(|a, b| a.path.cmp(&b.path));
    edges.sort_by(|a, b| edge_sort_key(a).cmp(&edge_sort_key(b)));
    let mut next_synthetic_file_node_id = max_existing_node_id(&files, &symbols, &manifests)
        .and_then(|id| id.checked_add(1))
        .unwrap_or(1);
    let files = files
        .into_iter()
        .map(|(file, node_id)| {
            let node_id = node_id.unwrap_or_else(|| {
                let synthetic = NodeId(next_synthetic_file_node_id);
                next_synthetic_file_node_id = next_synthetic_file_node_id
                    .checked_add(1)
                    .expect("exhausted synthetic file NodeId space");
                synthetic
            });
            file_node_ids.push(node_id);
            file
        })
        .collect();
    let symbols = symbols
        .into_iter()
        .map(|(symbol, node_id)| {
            if let Some(node_id) = node_id {
                symbol_node_ids.push(node_id);
            }
            symbol
        })
        .collect();

    macro_rows.sort_by(|a, b| {
        (&a.file_path, a.byte_range, &a.site_id).cmp(&(&b.file_path, b.byte_range, &b.site_id))
    });
    macro_scopes.sort_by(|a, b| {
        (&a.file_path, a.byte_range, &a.scope.stable_symbol_id).cmp(&(
            &b.file_path,
            b.byte_range,
            &b.scope.stable_symbol_id,
        ))
    });
    GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: Some(crate::MacroInvocationEvidenceSet {
            schema_version: crate::MACRO_INVOCATION_SCHEMA_VERSION.into(),
            extractor_version: EXTRACTOR_VERSION.into(),
            graph_content_hash: graph_content_hash.clone(),
            source_scope: crate::MacroEvidenceSourceScope::PhysicalRustFiles,
            rows: macro_rows,
            scopes: macro_scopes,
        }),
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
            // Content hash is stamped at write time, after body serialization input is finalized.
            content_hash_blake3: None,
        },
        manifest_version,
        graph_content_hash,
        file_manifests: manifests,
        files,
        file_node_ids,
        symbols,
        symbol_node_ids,
        edges,
        tombstones,
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
    }
}

fn max_existing_node_id(
    files: &[(GraphFileArtifact, Option<NodeId>)],
    symbols: &[(GraphSymbolArtifact, Option<NodeId>)],
    manifests: &[GraphFileManifestEntry],
) -> Option<u64> {
    files
        .iter()
        .filter_map(|(_, node_id)| node_id.map(NodeId::get))
        .chain(
            symbols
                .iter()
                .filter_map(|(_, node_id)| node_id.map(NodeId::get)),
        )
        .chain(
            manifests
                .iter()
                .flat_map(|manifest| manifest.node_ids.iter().map(|id| id.get())),
        )
        .max()
}

fn sort_bucket_symbols(bucket: &mut FileBucket) {
    if bucket.symbol_node_ids.len() == bucket.symbols.len() {
        let mut symbols: Vec<_> = bucket
            .symbols
            .drain(..)
            .zip(bucket.symbol_node_ids.drain(..))
            .collect();
        symbols.sort_by(|a, b| {
            a.0.byte_range
                .cmp(&b.0.byte_range)
                .then(a.0.entity_name.cmp(&b.0.entity_name))
                .then(a.0.stable_symbol_id.cmp(&b.0.stable_symbol_id))
        });
        for (symbol, node_id) in symbols {
            bucket.symbols.push(symbol);
            bucket.symbol_node_ids.push(node_id);
        }
    } else {
        bucket.symbols.sort_by(|a, b| {
            a.byte_range
                .cmp(&b.byte_range)
                .then(a.entity_name.cmp(&b.entity_name))
                .then(a.stable_symbol_id.cmp(&b.stable_symbol_id))
        });
    }
}

fn edge_sort_key(edge: &GraphEdgeArtifact) -> (&str, &str, &str, &str) {
    (
        edge.source_stable_symbol_id.as_str(),
        edge.target_stable_symbol_id.as_deref().unwrap_or(""),
        relation_discriminator(edge.relation),
        edge.target_label.as_deref().unwrap_or(""),
    )
}

fn relation_discriminator(relation: RelationKind) -> &'static str {
    match relation {
        RelationKind::Imports => "imports",
        RelationKind::Calls => "calls",
        RelationKind::Constructs => "constructs",
        RelationKind::Contains => "contains",
        RelationKind::Implements => "implements",
        RelationKind::Defines => "defines",
        RelationKind::References => "references",
        RelationKind::Extends => "extends",
        RelationKind::Links => "links",
        RelationKind::Touches => "touches",
        RelationKind::Produces => "produces",
        RelationKind::Consumes => "consumes",
        RelationKind::Binds => "binds",
        RelationKind::Emits => "emits",
    }
}

fn node_file_path(
    facts: &GraphFacts,
    node: &GraphNode,
    spans_by_id: &HashMap<crate::SpanId, &SourceSpan>,
) -> Option<String> {
    if node.kind == NodeKind::File {
        return Some(node.label.clone());
    }
    let span_id = node.source_span_id?;
    let span = spans_by_id.get(&span_id).copied()?;
    file_path_for_span(facts, span)
}

fn file_path_for_span(facts: &GraphFacts, span: &SourceSpan) -> Option<String> {
    facts
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::File && node.file_id == Some(span.file_id))
        .map(|node| node.label.clone())
}

fn stable_file_id_from_path(path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    format!(
        "{:016x}",
        u64::from_be_bytes(hasher.finalize()[..8].try_into().unwrap())
    )
}

fn anchor_hash(root: &Path, file_path: &str, span: &SourceSpan) -> String {
    let full_path = root.join(file_path);
    let bytes = match fs::read(&full_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                file_path,
                full_path = %full_path.display(),
                error = %error,
                "spur-graph anchor hash fallback to sentinel: source read failed"
            );
            return "0".to_owned();
        }
    };
    let start = span.start_byte as usize;
    let end = span.end_byte as usize;
    let Some(bytes) = bytes.get(start..end) else {
        tracing::warn!(
            file_path,
            full_path = %full_path.display(),
            span_start = start,
            span_end = end,
            "spur-graph anchor hash fallback to sentinel: byte range mismatch"
        );
        return "0".to_owned();
    };
    let slice = if let Ok(slice) = str::from_utf8(bytes) {
        slice
    } else {
        tracing::warn!(
            file_path,
            full_path = %full_path.display(),
            span_start = start,
            span_end = end,
            "spur-graph anchor hash fallback to sentinel: UTF-8 boundary mismatch"
        );
        return "0".to_owned();
    };
    compute_anchor_hash(slice).to_string()
}

fn symbol_kind(kind: NodeKind) -> &'static str {
    kind.discriminator()
}

fn symbol_entity_name(label: &str) -> String {
    label.strip_prefix("impl ").unwrap_or(label).to_owned()
}

fn parent_by_target(facts: &GraphFacts) -> HashMap<crate::NodeId, crate::NodeId> {
    let mut parent_by_target = HashMap::new();
    for edge in &facts.edges {
        if edge.relation != RelationKind::Contains {
            continue;
        }
        let Some(target_node_id) = edge.target_node_id else {
            continue;
        };
        parent_by_target
            .entry(target_node_id)
            .or_insert(edge.source_node_id);
    }
    parent_by_target
}

fn containing_parent<'a>(
    parent_by_target: &HashMap<crate::NodeId, crate::NodeId>,
    nodes_by_id: &HashMap<crate::NodeId, &'a GraphNode>,
    node: &GraphNode,
) -> Option<&'a GraphNode> {
    let parent_id = parent_by_target.get(&node.node_id)?;
    nodes_by_id.get(parent_id).copied()
}

fn qualified_name(
    parent_by_target: &HashMap<crate::NodeId, crate::NodeId>,
    nodes_by_id: &HashMap<crate::NodeId, &GraphNode>,
    node: &GraphNode,
) -> String {
    if node.kind == NodeKind::McpTool {
        return symbol_entity_name(&node.label);
    }

    let mut segments = vec![qualified_scope_segment(node)];
    let mut current = node;
    let mut seen = HashSet::new();
    seen.insert(node.node_id);

    while let Some(parent) = containing_parent(parent_by_target, nodes_by_id, current) {
        if !seen.insert(parent.node_id) {
            break;
        }
        if parent.kind == NodeKind::File {
            break;
        }

        if parent.kind != NodeKind::ExternBlock {
            segments.push(qualified_scope_segment(parent));
        }
        current = parent;
    }

    segments.reverse();
    segments.join("::")
}

fn qualified_scope_segment(node: &GraphNode) -> String {
    match node.kind {
        NodeKind::Impl => format!("impl {}", symbol_entity_name(&node.label)),
        _ => symbol_entity_name(&node.label),
    }
}

fn enclosing_scope(
    parent_by_target: &HashMap<crate::NodeId, crate::NodeId>,
    nodes_by_id: &HashMap<crate::NodeId, &GraphNode>,
    node: &GraphNode,
) -> Option<String> {
    let mut parent = containing_parent(parent_by_target, nodes_by_id, node)?;
    let foreign_scope = node.kind == NodeKind::ExternBlock || parent.kind == NodeKind::ExternBlock;
    let mut seen = HashSet::from([node.node_id]);
    while parent.kind == NodeKind::ExternBlock {
        if !seen.insert(parent.node_id) {
            return None;
        }
        parent = containing_parent(parent_by_target, nodes_by_id, parent)?;
    }
    match parent.kind {
        NodeKind::File => None,
        _ if foreign_scope => Some(qualified_name(parent_by_target, nodes_by_id, parent)),
        NodeKind::Impl => Some(format!("impl {}", parent.label)),
        _ => Some(parent.label.clone()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn macro_evidence_survives_worktree_delta_with_all_owner_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("changed.rs"), "fn old() { panic!(); }\n").unwrap();
        std::fs::write(root.join("keep.rs"), "fn keep() { todo!(); }\n").unwrap();
        let facts = crate::build_facts(&root, None).unwrap().0;
        let baseline = super::artifact_from_facts(&facts, &root).unwrap();
        let source = "#![allow(dead_code)]\nconst C: () = assert!(true);\n";
        std::fs::write(root.join("changed.rs"), source).unwrap();
        let delta = super::build_worktree_delta_artifact(&baseline, &root)
            .unwrap()
            .artifact;
        let json = serde_json::to_value(&delta).unwrap();
        assert!(
            json["macro_invocations"].is_object(),
            "delta must carry captured macro evidence"
        );
        let set = &json["macro_invocations"];
        assert_eq!(set["source_scope"], "physical_rust_files");
        assert_eq!(set["graph_content_hash"], delta.graph_content_hash);
        assert_eq!(set["rows"].as_array().unwrap().len(), 1);
        assert_eq!(set["rows"][0]["file_path"], "changed.rs");
        assert_eq!(
            set["rows"][0]["source_oid"],
            crate::git_blob_oid(source.as_bytes())
        );
        assert_eq!(set["rows"][0]["enclosing"]["kind"], "const");
        assert!(set["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["scope"]["kind"] == "file"));
        assert!(set["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["file_path"] == "changed.rs"));
    }

    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{
        artifact_from_facts, artifact_from_facts_incremental, buckets_from_artifact,
        compose_artifact, current_symbol_ids_from_buckets, empty_bucket,
        import_workspace_index_from_buckets, is_import_rebind_fallback_candidate_kind,
        manifest_version_from_query_bytes, rebind_cross_file_edges, rebind_import_edges,
        rebind_remaining_edges, symbols_by_entity_name_from_buckets, BuildMode, CurrentFileEntry,
        ExternalImportRegistry, FileBucket, FileImportIndex, ManifestQueryBytes, RebindTarget,
        GRAPH_INDEX_VERSION_TEMPORAL, MANIFEST_QUERY_BYTES,
    };
    use crate::content_hash::{compute_graph_content_hash, git_blob_oid};
    use crate::extract::{build_facts, build_facts_for_paths, GraphFacts};
    use crate::identity::{stable_symbol_id_for_external_path, EXTERNAL_FILE_PATH};
    use crate::{
        graph_edge_kind_or_default, Confidence, FileId, GraphEdgeArtifact, GraphEdgeKind,
        GraphFileArtifact, GraphFileManifestEntry, GraphIndexArtifact, GraphIndexHeader, GraphNode,
        GraphSymbolArtifact, NodeId, NodeKind, RelationKind, RunId, SourceSpan, SpanId,
    };

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Record};
    use tracing::{Event, Id, Metadata, Subscriber};

    #[test]
    fn manifest_includes_html_and_css_query_bytes() {
        let included = MANIFEST_QUERY_BYTES
            .iter()
            .map(|query| (query.language, query.query))
            .collect::<BTreeSet<_>>();

        for expected in [
            ("html", "tags"),
            ("html", "spur-edges"),
            ("css", "tags"),
            ("css", "spur-edges"),
        ] {
            assert!(
                included.contains(&expected),
                "manifest hash must include {expected:?}"
            );
        }
    }

    #[test]
    fn web_imports_bind_to_workspace_files_instead_of_external_nodes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(
            root.join("index.html"),
            r##"<script src="app.js"></script>
<script src="data:text/javascript;base64,AAAA"></script>
<script src=" DATA:text/javascript;base64,BBBB"></script>
<script src="#bootstrap"></script>
<link rel="stylesheet" href="theme.css">
<link rel="stylesheet" href="/theme.css">"##,
        )
        .expect("write html");
        fs::write(root.join("app.js"), "export const app = true;\n").expect("write js");
        fs::write(
            root.join("theme.css"),
            "@import \"base.css\";\n@import \"data:text/css,body{}\";\n@import \" DATA:text/css,p{}\";\n@import \"#print\";\n@import \"/base.css\";\n",
        )
        .expect("write css");
        fs::write(root.join("base.css"), ":root { --brand: red; }\n").expect("write css");

        let facts = build_facts(root, None).expect("extract web files").0;
        let artifact = artifact_from_facts(&facts, root).expect("build artifact");
        let file_ids = artifact
            .files
            .iter()
            .map(|file| (file.file_path.as_str(), file.stable_file_id.as_str()))
            .collect::<BTreeMap<_, _>>();

        for (source, target) in [
            ("index.html", "app.js"),
            ("index.html", "theme.css"),
            ("theme.css", "base.css"),
        ] {
            let edge = artifact
                .edges
                .iter()
                .find(|edge| {
                    edge.relation == RelationKind::Imports
                        && edge.target_label.as_deref() == Some(target)
                        && edge.source_stable_symbol_id == file_ids[source]
                })
                .unwrap_or_else(|| panic!("missing {source} import of {target}"));
            assert_eq!(
                edge.target_stable_symbol_id.as_deref(),
                Some(file_ids[target])
            );
            assert_eq!(edge.bind_method.as_deref(), Some("import_path"));
        }

        assert!(
            artifact.edges.iter().all(|edge| {
                !edge
                    .target_label
                    .as_deref()
                    .is_some_and(|target| target.to_ascii_lowercase().starts_with("data:"))
            }),
            "data imports must be omitted before artifact binding"
        );
        for (source, target) in [
            ("index.html", "#bootstrap"),
            ("index.html", "/theme.css"),
            ("theme.css", "#print"),
            ("theme.css", "/base.css"),
        ] {
            let edge = artifact
                .edges
                .iter()
                .find(|edge| {
                    edge.relation == RelationKind::Imports
                        && edge.target_label.as_deref() == Some(target)
                        && edge.source_stable_symbol_id == file_ids[source]
                })
                .unwrap_or_else(|| panic!("missing unresolved {source} import of {target}"));
            assert_eq!(edge.target_stable_symbol_id, None, "{source} -> {target}");
            assert_eq!(edge.bind_method, None, "{source} -> {target}");
        }

        assert!(
            artifact
                .symbols
                .iter()
                .all(|symbol| symbol.symbol_kind != "external"),
            "workspace web imports must not create external nodes"
        );
    }

    #[test]
    fn import_rebind_fallback_candidate_kind_agrees_with_node_kind_discriminators() {
        for (kind, expected) in [
            (NodeKind::Module, false),
            (NodeKind::ExternBlock, false),
            (NodeKind::Function, false),
            (NodeKind::Class, false),
            (NodeKind::Interface, false),
            (NodeKind::Struct, false),
            (NodeKind::Impl, false),
            (NodeKind::Trait, false),
            (NodeKind::Enum, false),
            (NodeKind::EnumVariant, true),
            (NodeKind::File, false),
            (NodeKind::External, false),
            (NodeKind::Method, false),
            (NodeKind::Field, false),
            (NodeKind::Constant, true),
            (NodeKind::TypeAlias, false),
            (NodeKind::Macro, false),
            (NodeKind::Section, false),
            (NodeKind::Commit, false),
            (NodeKind::McpTool, false),
            (NodeKind::Cell, false),
            (NodeKind::Port, false),
            (NodeKind::Resource, false),
        ] {
            assert_eq!(
                is_import_rebind_fallback_candidate_kind(kind.discriminator()),
                expected,
                "{kind:?}"
            );
            assert_eq!(
                NodeKind::from_discriminator(kind.discriminator()),
                Some(kind)
            );
        }

        assert!(!is_import_rebind_fallback_candidate_kind("future_kind"));
    }

    #[test]
    fn schema_version_carries_phase_1b_vocabulary() {
        assert_eq!(super::SCHEMA_VERSION, "spur-graph-schema-v12");
    }

    #[test]
    fn manifest_version_changes_when_spur_edges_query_bytes_change() {
        let base = [
            ManifestQueryBytes {
                language: "rust",
                query: "tags",
                bytes: b"rust tags",
            },
            ManifestQueryBytes {
                language: "rust",
                query: "spur-edges",
                bytes: b"rust edges v1",
            },
            ManifestQueryBytes {
                language: "typescript",
                query: "tags",
                bytes: b"typescript tags",
            },
            ManifestQueryBytes {
                language: "typescript",
                query: "spur-edges",
                bytes: b"typescript edges",
            },
        ];
        let changed = [
            ManifestQueryBytes {
                language: "rust",
                query: "tags",
                bytes: b"rust tags",
            },
            ManifestQueryBytes {
                language: "rust",
                query: "spur-edges",
                bytes: b"rust edges v2",
            },
            ManifestQueryBytes {
                language: "typescript",
                query: "tags",
                bytes: b"typescript tags",
            },
            ManifestQueryBytes {
                language: "typescript",
                query: "spur-edges",
                bytes: b"typescript edges",
            },
        ];

        assert_ne!(
            manifest_version_from_query_bytes("schema", "extractor", "resolver", &base),
            manifest_version_from_query_bytes("schema", "extractor", "resolver", &changed)
        );
    }

    #[test]
    fn manifest_version_changes_when_resolver_version_changes() {
        let inputs = [ManifestQueryBytes {
            language: "rust",
            query: "spur-edges",
            bytes: b"rust edges",
        }];

        assert_ne!(
            manifest_version_from_query_bytes("schema", "extractor", "resolver-v1", &inputs),
            manifest_version_from_query_bytes("schema", "extractor", "resolver-v2", &inputs)
        );
    }

    #[test]
    fn manifest_version_is_deterministic_for_identical_query_inputs() {
        let inputs = [
            ManifestQueryBytes {
                language: "typescript",
                query: "spur-edges",
                bytes: b"typescript edges",
            },
            ManifestQueryBytes {
                language: "rust",
                query: "tags",
                bytes: b"rust tags",
            },
        ];

        let first = manifest_version_from_query_bytes("schema", "extractor", "resolver", &inputs);
        let second = manifest_version_from_query_bytes("schema", "extractor", "resolver", &inputs);

        assert_eq!(first, second);

        let reordered = [inputs[1], inputs[0]];
        assert_eq!(
            first,
            manifest_version_from_query_bytes("schema", "extractor", "resolver", &reordered)
        );
    }

    #[test]
    fn buckets_from_artifact_rebuckets_edges_by_file_or_symbol_source() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test-manifest".to_owned(),
            graph_content_hash: "test-hash".to_owned(),
            file_manifests: vec![
                manifest("file:a", "src/a.rs"),
                manifest("file:b", "src/b.rs"),
            ],
            files: vec![file("file:a", "src/a.rs"), file("file:b", "src/b.rs")],
            file_node_ids: Vec::new(),
            symbols: vec![
                symbol("sym:a1", "src/a.rs"),
                symbol("sym:a2", "src/a.rs"),
                symbol("sym:b1", "src/b.rs"),
            ],
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("file:a", Some("sym:a1"), RelationKind::Contains),
                edge("sym:a2", Some("sym:b1"), RelationKind::Calls),
                edge("sym:b1", Some("sym:a1"), RelationKind::References),
                edge("unknown", Some("sym:a1"), RelationKind::Links),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let actual = buckets_from_artifact(&artifact);
        assert_eq!(
            actual.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["src/a.rs", "src/b.rs"]
        );

        let a_bucket = actual.get("src/a.rs").expect("a bucket");
        assert_eq!(a_bucket.file, file("file:a", "src/a.rs"));
        assert_eq!(a_bucket.manifest, manifest("file:a", "src/a.rs"));
        assert_eq!(
            a_bucket.symbols,
            vec![symbol("sym:a1", "src/a.rs"), symbol("sym:a2", "src/a.rs")]
        );
        assert_eq!(
            a_bucket.edges,
            vec![
                edge("file:a", Some("sym:a1"), RelationKind::Contains),
                edge("sym:a2", Some("sym:b1"), RelationKind::Calls),
            ]
        );

        let b_bucket = actual.get("src/b.rs").expect("b bucket");
        assert_eq!(b_bucket.file, file("file:b", "src/b.rs"));
        assert_eq!(b_bucket.manifest, manifest("file:b", "src/b.rs"));
        assert_eq!(b_bucket.symbols, vec![symbol("sym:b1", "src/b.rs")]);
        assert_eq!(
            b_bucket.edges,
            vec![edge("sym:b1", Some("sym:a1"), RelationKind::References)]
        );
    }

    #[test]
    fn rebind_leaves_import_unresolved_when_filter_empties_candidates() {
        // Regression: an Imports edge whose target_label matches exactly one
        // symbol whose kind is filtered out by is_import_rebind_candidate_kind
        // (here a `method`) used to leave `matches` empty and panic on
        // `matches.first().expect("matches is non-empty")` (build.rs:1051).
        let mut def_bucket = empty_bucket("src/def.rs", "oid-def");
        let mut method_symbol = symbol("sym:method", "src/def.rs");
        method_symbol.entity_name = "shared".to_owned();
        method_symbol.symbol_kind = "method".to_owned();
        def_bucket.symbols.push(method_symbol);

        let mut use_bucket = empty_bucket("src/use.rs", "oid-use");
        let mut import_edge = edge("sym:importer", None, RelationKind::Imports);
        import_edge.target_label = Some("shared".to_owned());
        use_bucket.edges.push(import_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("src/def.rs".to_owned(), def_bucket);
        buckets.insert("src/use.rs".to_owned(), use_bucket);

        // Must not panic.
        rebind_cross_file_edges(&mut buckets);

        let resolved_edge = &buckets["src/use.rs"].edges[0];
        assert_eq!(
            resolved_edge.target_stable_symbol_id, None,
            "import to a filtered-out kind must stay unresolved"
        );
    }

    #[test]
    fn rebind_import_candidate_hygiene_prefers_type_like_shadow() {
        let mut def_bucket = empty_bucket("src/def.rs", "oid-def");
        let mut struct_symbol = symbol("sym:foo-struct", "src/def.rs");
        struct_symbol.entity_name = "Foo".to_owned();
        struct_symbol.symbol_kind = "struct".to_owned();
        let mut impl_symbol = symbol("sym:foo-impl", "src/def.rs");
        impl_symbol.entity_name = "Foo".to_owned();
        impl_symbol.symbol_kind = "impl".to_owned();
        let mut variant_symbol = symbol("sym:foo-variant", "src/def.rs");
        variant_symbol.entity_name = "Foo".to_owned();
        variant_symbol.symbol_kind = "enum_variant".to_owned();
        let mut constant_symbol = symbol("sym:foo-constant", "src/def.rs");
        constant_symbol.entity_name = "Foo".to_owned();
        constant_symbol.symbol_kind = "constant".to_owned();
        def_bucket
            .symbols
            .extend([struct_symbol, impl_symbol, variant_symbol, constant_symbol]);

        let mut use_bucket = empty_bucket("src/use.rs", "oid-use");
        let mut import_edge = edge("sym:importer", None, RelationKind::Imports);
        import_edge.target_label = Some("Foo".to_owned());
        use_bucket.edges.push(import_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("src/def.rs".to_owned(), def_bucket);
        buckets.insert("src/use.rs".to_owned(), use_bucket);

        rebind_cross_file_edges(&mut buckets);

        let resolved_edge = &buckets["src/use.rs"].edges[0];
        assert_eq!(
            resolved_edge.target_stable_symbol_id.as_deref(),
            Some("sym:foo-struct")
        );
    }

    #[test]
    fn rebind_import_keeps_enum_variant_candidate_when_no_type_like_exists() {
        let mut def_bucket = empty_bucket("src/def.rs", "oid-def");
        let mut variant_symbol = symbol("sym:variant", "src/def.rs");
        variant_symbol.entity_name = "Variant".to_owned();
        variant_symbol.symbol_kind = "enum_variant".to_owned();
        def_bucket.symbols.push(variant_symbol);

        let mut use_bucket = empty_bucket("src/use.rs", "oid-use");
        let mut import_edge = edge("sym:importer", None, RelationKind::Imports);
        import_edge.target_label = Some("Variant".to_owned());
        use_bucket.edges.push(import_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("src/def.rs".to_owned(), def_bucket);
        buckets.insert("src/use.rs".to_owned(), use_bucket);

        rebind_cross_file_edges(&mut buckets);

        let resolved_edge = &buckets["src/use.rs"].edges[0];
        assert_eq!(
            resolved_edge.target_stable_symbol_id.as_deref(),
            Some("sym:variant")
        );
    }

    #[test]
    fn rebind_import_path_stamps_cross_crate_workspace_path() {
        let mut def_bucket = empty_bucket("crates/spur-acp/src/types.rs", "oid-def");
        let mut session_symbol = symbol("sym:session", "crates/spur-acp/src/types.rs");
        session_symbol.entity_name = "SessionId".to_owned();
        session_symbol.qualified_name = "SessionId".to_owned();
        session_symbol.symbol_kind = "struct".to_owned();
        def_bucket.symbols.push(session_symbol);

        let mut shadow_bucket = empty_bucket("crates/spur-worktree/src/types.rs", "oid-shadow");
        let mut shadow_symbol = symbol("sym:shadow-session", "crates/spur-worktree/src/types.rs");
        shadow_symbol.entity_name = "SessionId".to_owned();
        shadow_symbol.qualified_name = "SessionId".to_owned();
        shadow_symbol.symbol_kind = "struct".to_owned();
        shadow_bucket.symbols.push(shadow_symbol);

        let mut use_bucket = empty_bucket("crates/spur-core/src/lib.rs", "oid-use");
        let mut import_edge = edge("sym:importer", None, RelationKind::Imports);
        import_edge.target_label = Some("SessionId".to_owned());
        import_edge.import_path = Some("spur_acp::types::SessionId".to_owned());
        use_bucket.edges.push(import_edge);
        let mut root_import_edge = edge("sym:importer", None, RelationKind::Imports);
        root_import_edge.target_label = Some("SessionId".to_owned());
        root_import_edge.import_path = Some("spur_acp::SessionId".to_owned());
        use_bucket.edges.push(root_import_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("crates/spur-acp/src/types.rs".to_owned(), def_bucket);
        buckets.insert("crates/spur-core/src/lib.rs".to_owned(), use_bucket);
        buckets.insert(
            "crates/spur-worktree/src/types.rs".to_owned(),
            shadow_bucket,
        );

        rebind_cross_file_edges(&mut buckets);

        let edges = &buckets["crates/spur-core/src/lib.rs"].edges;
        for resolved_edge in edges {
            assert_eq!(
                resolved_edge.target_stable_symbol_id.as_deref(),
                Some("sym:session")
            );
            assert_eq!(resolved_edge.bind_method.as_deref(), Some("import_path"));
        }
    }

    #[test]
    fn rebind_import_path_keeps_non_workspace_rust_import_external() {
        let mut shadow_bucket = empty_bucket("src/shadow.rs", "oid-shadow");
        let mut shadow_symbol = symbol("sym:deserialize", "src/shadow.rs");
        shadow_symbol.entity_name = "Deserialize".to_owned();
        shadow_symbol.qualified_name = "Deserialize".to_owned();
        shadow_symbol.symbol_kind = "struct".to_owned();
        shadow_bucket.symbols.push(shadow_symbol);

        let mut use_bucket = empty_bucket("src/lib.rs", "oid-use");
        let mut import_edge = edge("sym:importer", None, RelationKind::Imports);
        import_edge.target_label = Some("Deserialize".to_owned());
        import_edge.import_path = Some("serde::Deserialize".to_owned());
        use_bucket.edges.push(import_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("src/lib.rs".to_owned(), use_bucket);
        buckets.insert("src/shadow.rs".to_owned(), shadow_bucket);

        rebind_cross_file_edges(&mut buckets);

        let resolved_edge = &buckets["src/lib.rs"].edges[0];
        let expected = stable_symbol_id_for_external_path("serde::Deserialize");
        assert_eq!(
            resolved_edge.target_stable_symbol_id.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(resolved_edge.bind_method.as_deref(), Some("external"));
    }

    #[test]
    fn rebind_import_path_rejects_cross_language_type_fallback_to_external() {
        let mut def_bucket = empty_bucket("src/lib.rs", "oid-def");
        let mut widget_symbol = symbol("sym:widget", "src/lib.rs");
        widget_symbol.entity_name = "Widget".to_owned();
        widget_symbol.symbol_kind = "struct".to_owned();
        def_bucket.symbols.push(widget_symbol);

        let mut use_bucket = empty_bucket("consumer.py", "oid-use");
        let mut import_edge = edge("sym:importer", None, RelationKind::Imports);
        import_edge.target_label = Some("Widget".to_owned());
        import_edge.import_path = Some("external.widget".to_owned());
        use_bucket.edges.push(import_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("src/lib.rs".to_owned(), def_bucket);
        buckets.insert("consumer.py".to_owned(), use_bucket);

        rebind_cross_file_edges(&mut buckets);

        let resolved_edge = &buckets["consumer.py"].edges[0];
        assert_ne!(
            resolved_edge.target_stable_symbol_id.as_deref(),
            Some("sym:widget")
        );
        assert_eq!(resolved_edge.bind_method.as_deref(), Some("external"));
        let external_symbols = &buckets[EXTERNAL_FILE_PATH].symbols;
        assert_eq!(external_symbols.len(), 1);
        assert_eq!(external_symbols[0].qualified_name, "external.widget");
        assert_eq!(external_symbols[0].entity_name, "widget");
    }

    #[test]
    fn rebind_import_path_binds_std_collision_traps_to_external() {
        let mut def_bucket = empty_bucket("src/shadow.rs", "oid-def");
        let mut variant_symbol = symbol("sym:path", "src/shadow.rs");
        variant_symbol.entity_name = "Path".to_owned();
        variant_symbol.symbol_kind = "enum_variant".to_owned();
        let mut command_symbol = symbol("sym:command", "src/shadow.rs");
        command_symbol.entity_name = "Command".to_owned();
        command_symbol.symbol_kind = "constant".to_owned();
        let mut ordering_symbol = symbol("sym:ordering", "src/shadow.rs");
        ordering_symbol.entity_name = "Ordering".to_owned();
        ordering_symbol.symbol_kind = "enum_variant".to_owned();
        let mut write_symbol = symbol("sym:write", "src/shadow.rs");
        write_symbol.entity_name = "Write".to_owned();
        write_symbol.symbol_kind = "section".to_owned();
        def_bucket.symbols.extend([
            variant_symbol,
            command_symbol,
            ordering_symbol,
            write_symbol,
        ]);

        let mut use_bucket = empty_bucket("src/lib.rs", "oid-use");
        for (label, import_path) in [
            ("Path", "std::path::Path"),
            ("Command", "std::process::Command"),
            ("Ordering", "std::cmp::Ordering"),
            ("Write", "std::io::Write"),
        ] {
            let mut import_edge = edge("sym:importer", None, RelationKind::Imports);
            import_edge.target_label = Some(label.to_owned());
            import_edge.import_path = Some(import_path.to_owned());
            use_bucket.edges.push(import_edge);
        }

        let mut buckets = BTreeMap::new();
        buckets.insert("src/shadow.rs".to_owned(), def_bucket);
        buckets.insert("src/lib.rs".to_owned(), use_bucket);

        rebind_cross_file_edges(&mut buckets);

        let edges = &buckets["src/lib.rs"].edges;
        for (edge, import_path) in edges.iter().zip([
            "std::path::Path",
            "std::process::Command",
            "std::cmp::Ordering",
            "std::io::Write",
        ]) {
            let expected = stable_symbol_id_for_external_path(import_path);
            assert_eq!(
                edge.target_stable_symbol_id.as_deref(),
                Some(expected.as_str())
            );
            assert_eq!(edge.bind_method.as_deref(), Some("external"));
        }
        let external_symbols = &buckets[EXTERNAL_FILE_PATH].symbols;
        assert_eq!(external_symbols.len(), 4);
        assert!(external_symbols.iter().all(
            |symbol| symbol.file_path == EXTERNAL_FILE_PATH && symbol.symbol_kind == "external"
        ));
    }

    #[test]
    fn import_phase_builds_workspace_only_file_import_index() {
        let mut def_bucket = empty_bucket("crates/crate-b/src/lib.rs", "oid-def");
        let mut foo_symbol = symbol("sym:crate-b-foo", "crates/crate-b/src/lib.rs");
        foo_symbol.entity_name = "foo".to_owned();
        foo_symbol.qualified_name = "foo".to_owned();
        foo_symbol.symbol_kind = "function".to_owned();
        def_bucket.symbols.push(foo_symbol);

        let mut use_bucket = empty_bucket("crates/crate-a/src/lib.rs", "oid-use");
        let mut workspace_import = edge("sym:importer", None, RelationKind::Imports);
        workspace_import.target_label = Some("foo".to_owned());
        workspace_import.import_path = Some("crate_b::foo".to_owned());
        use_bucket.edges.push(workspace_import);

        let mut external_import = edge("sym:importer", None, RelationKind::Imports);
        external_import.target_label = Some("Deserialize".to_owned());
        external_import.import_path = Some("serde::Deserialize".to_owned());
        use_bucket.edges.push(external_import);

        let mut buckets = BTreeMap::new();
        buckets.insert("crates/crate-a/src/lib.rs".to_owned(), use_bucket);
        buckets.insert("crates/crate-b/src/lib.rs".to_owned(), def_bucket);

        let workspace_index = import_workspace_index_from_buckets(&buckets);
        let symbols_by_entity_name = symbols_by_entity_name_from_buckets(&buckets);
        let mut external_imports = ExternalImportRegistry::from_buckets(&buckets);
        let mut current_symbol_ids = current_symbol_ids_from_buckets(&buckets);

        let (file_import_index, ambiguous_unresolved) = rebind_import_edges(
            &mut buckets,
            &workspace_index,
            &symbols_by_entity_name,
            &mut external_imports,
            &mut current_symbol_ids,
        );

        assert_eq!(ambiguous_unresolved, 0);
        let workspace_import_edge = buckets["crates/crate-a/src/lib.rs"]
            .edges
            .iter()
            .find(|edge| edge.import_path.as_deref() == Some("crate_b::foo"))
            .expect("workspace import edge");
        assert_eq!(
            workspace_import_edge.target_stable_symbol_id.as_deref(),
            Some("sym:crate-b-foo")
        );
        assert_eq!(
            workspace_import_edge.bind_method.as_deref(),
            Some("import_path")
        );

        let imports = file_import_index
            .get("crates/crate-a/src/lib.rs")
            .expect("crate-a imports");
        assert_eq!(
            imports.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["foo"]
        );

        let foo_targets = imports.get("foo").expect("foo import target");
        assert_eq!(foo_targets.len(), 1);
        assert_eq!(foo_targets[0].stable_symbol_id, "sym:crate-b-foo");
        assert_eq!(foo_targets[0].file_path, "crates/crate-b/src/lib.rs");
        assert_eq!(foo_targets[0].entity_name, "foo");
        assert_eq!(foo_targets[0].qualified_name, "foo");
        assert_eq!(foo_targets[0].symbol_kind, "function");
        assert!(
            !imports.contains_key("Deserialize"),
            "external imports must not be indexed as workspace licenses"
        );
    }

    #[test]
    fn remaining_phase_licenses_ambiguous_function_call_from_import_index() {
        let mut crate_b_bucket = empty_bucket("crates/crate-b/src/lib.rs", "oid-crate-b");
        let mut crate_b_foo = symbol("sym:crate-b-foo", "crates/crate-b/src/lib.rs");
        crate_b_foo.entity_name = "foo".to_owned();
        crate_b_foo.symbol_kind = "function".to_owned();
        crate_b_bucket.symbols.push(crate_b_foo);

        let mut crate_c_bucket = empty_bucket("crates/crate-c/src/lib.rs", "oid-crate-c");
        let mut crate_c_foo = symbol("sym:crate-c-foo", "crates/crate-c/src/lib.rs");
        crate_c_foo.entity_name = "foo".to_owned();
        crate_c_foo.symbol_kind = "function".to_owned();
        crate_c_bucket.symbols.push(crate_c_foo);

        let mut use_bucket = empty_bucket("crates/crate-a/src/lib.rs", "oid-use");
        use_bucket
            .symbols
            .push(symbol("sym:caller", "crates/crate-a/src/lib.rs"));
        let mut call_edge = edge("sym:caller", None, RelationKind::Calls);
        call_edge.target_label = Some("foo".to_owned());
        use_bucket.edges.push(call_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("crates/crate-a/src/lib.rs".to_owned(), use_bucket);
        buckets.insert("crates/crate-b/src/lib.rs".to_owned(), crate_b_bucket);
        buckets.insert("crates/crate-c/src/lib.rs".to_owned(), crate_c_bucket);

        let workspace_index = import_workspace_index_from_buckets(&buckets);
        let symbols_by_entity_name = symbols_by_entity_name_from_buckets(&buckets);
        let mut external_imports = ExternalImportRegistry::from_buckets(&buckets);
        let mut current_symbol_ids = current_symbol_ids_from_buckets(&buckets);
        let file_import_index = FileImportIndex::from([(
            "crates/crate-a/src/lib.rs".to_owned(),
            BTreeMap::from([(
                "foo".to_owned(),
                vec![RebindTarget {
                    stable_symbol_id: "sym:crate-b-foo".to_owned(),
                    file_path: "crates/crate-b/src/lib.rs".to_owned(),
                    entity_name: "foo".to_owned(),
                    qualified_name: "foo".to_owned(),
                    symbol_kind: "function".to_owned(),
                }],
            )]),
        )]);

        let ambiguous_unresolved = rebind_remaining_edges(
            &mut buckets,
            &workspace_index,
            &symbols_by_entity_name,
            &file_import_index,
            &mut external_imports,
            &mut current_symbol_ids,
        );

        assert_eq!(ambiguous_unresolved, 0);
        let resolved_edge = &buckets["crates/crate-a/src/lib.rs"].edges[0];
        assert_eq!(
            resolved_edge.target_stable_symbol_id.as_deref(),
            Some("sym:crate-b-foo")
        );
        assert_eq!(
            resolved_edge.bind_method.as_deref(),
            Some("import_licensed")
        );
    }

    #[test]
    fn remaining_phase_preserves_stamped_call_targets_before_import_licensing() {
        let mut crate_b_bucket = empty_bucket("crates/crate-b/src/lib.rs", "oid-crate-b");
        let mut crate_b_foo = symbol("sym:crate-b-foo", "crates/crate-b/src/lib.rs");
        crate_b_foo.entity_name = "foo".to_owned();
        crate_b_foo.symbol_kind = "function".to_owned();
        crate_b_bucket.symbols.push(crate_b_foo);

        let mut crate_c_bucket = empty_bucket("crates/crate-c/src/lib.rs", "oid-crate-c");
        let mut crate_c_foo = symbol("sym:crate-c-foo", "crates/crate-c/src/lib.rs");
        crate_c_foo.entity_name = "foo".to_owned();
        crate_c_foo.symbol_kind = "function".to_owned();
        crate_c_bucket.symbols.push(crate_c_foo);

        let mut use_bucket = empty_bucket("crates/crate-a/src/lib.rs", "oid-use");
        use_bucket.symbols.extend([
            symbol("sym:singleton-caller", "crates/crate-a/src/lib.rs"),
            symbol("sym:licensed-caller", "crates/crate-a/src/lib.rs"),
        ]);
        let mut singleton_edge = edge(
            "sym:singleton-caller",
            Some("sym:crate-c-foo"),
            RelationKind::Calls,
        );
        singleton_edge.target_label = Some("foo".to_owned());
        singleton_edge.bind_method = Some("singleton".to_owned());
        use_bucket.edges.push(singleton_edge);

        let mut licensed_edge = edge(
            "sym:licensed-caller",
            Some("sym:crate-b-foo"),
            RelationKind::Calls,
        );
        licensed_edge.target_label = Some("foo".to_owned());
        licensed_edge.bind_method = Some("import_licensed".to_owned());
        use_bucket.edges.push(licensed_edge);

        let mut buckets = BTreeMap::new();
        buckets.insert("crates/crate-a/src/lib.rs".to_owned(), use_bucket);
        buckets.insert("crates/crate-b/src/lib.rs".to_owned(), crate_b_bucket);
        buckets.insert("crates/crate-c/src/lib.rs".to_owned(), crate_c_bucket);

        let workspace_index = import_workspace_index_from_buckets(&buckets);
        let symbols_by_entity_name = symbols_by_entity_name_from_buckets(&buckets);
        let mut external_imports = ExternalImportRegistry::from_buckets(&buckets);
        let mut current_symbol_ids = current_symbol_ids_from_buckets(&buckets);
        let file_import_index = FileImportIndex::from([(
            "crates/crate-a/src/lib.rs".to_owned(),
            BTreeMap::from([(
                "foo".to_owned(),
                vec![RebindTarget {
                    stable_symbol_id: "sym:crate-b-foo".to_owned(),
                    file_path: "crates/crate-b/src/lib.rs".to_owned(),
                    entity_name: "foo".to_owned(),
                    qualified_name: "foo".to_owned(),
                    symbol_kind: "function".to_owned(),
                }],
            )]),
        )]);

        rebind_remaining_edges(
            &mut buckets,
            &workspace_index,
            &symbols_by_entity_name,
            &file_import_index,
            &mut external_imports,
            &mut current_symbol_ids,
        );

        let edges = &buckets["crates/crate-a/src/lib.rs"].edges;
        assert_eq!(
            edges[0].target_stable_symbol_id.as_deref(),
            Some("sym:crate-c-foo")
        );
        assert_eq!(edges[0].bind_method.as_deref(), Some("singleton"));
        assert_eq!(
            edges[1].target_stable_symbol_id.as_deref(),
            Some("sym:crate-b-foo")
        );
        assert_eq!(edges[1].bind_method.as_deref(), Some("import_licensed"));
    }

    fn hcl_symbol(
        stable_symbol_id: &str,
        path: &str,
        entity_name: &str,
        symbol_kind: &str,
    ) -> GraphSymbolArtifact {
        let mut hcl_symbol = symbol(stable_symbol_id, path);
        hcl_symbol.entity_name = entity_name.to_owned();
        hcl_symbol.qualified_name = entity_name.to_owned();
        hcl_symbol.symbol_kind = symbol_kind.to_owned();
        hcl_symbol
    }

    fn address_edge(source_stable_symbol_id: &str, target_label: &str) -> GraphEdgeArtifact {
        let mut address_edge = edge(source_stable_symbol_id, None, RelationKind::References);
        address_edge.target_label = Some(target_label.to_owned());
        address_edge.edge_kind = Some(GraphEdgeKind::ReferencesAddress);
        address_edge
    }

    fn run_rebind_remaining(buckets: &mut BTreeMap<String, FileBucket>) -> usize {
        let workspace_index = import_workspace_index_from_buckets(buckets);
        let symbols_by_entity_name = symbols_by_entity_name_from_buckets(buckets);
        let mut external_imports = ExternalImportRegistry::from_buckets(buckets);
        let mut current_symbol_ids = current_symbol_ids_from_buckets(buckets);
        rebind_remaining_edges(
            buckets,
            &workspace_index,
            &symbols_by_entity_name,
            &FileImportIndex::new(),
            &mut external_imports,
            &mut current_symbol_ids,
        )
    }

    #[test]
    fn rebind_prefers_the_module_directory_for_address_references() {
        let mut module_a = empty_bucket("modules/a/main.tf", "oid-a");
        module_a.symbols.push(hcl_symbol(
            "sym:a-web",
            "modules/a/main.tf",
            "aws_instance.web",
            "resource",
        ));
        module_a.symbols.push(hcl_symbol(
            "sym:a-eip",
            "modules/a/main.tf",
            "aws_eip.ip",
            "resource",
        ));
        module_a
            .edges
            .push(address_edge("sym:a-eip", "aws_instance.web"));

        let mut module_b = empty_bucket("modules/b/main.tf", "oid-b");
        module_b.symbols.push(hcl_symbol(
            "sym:b-web",
            "modules/b/main.tf",
            "aws_instance.web",
            "resource",
        ));

        let mut buckets = BTreeMap::new();
        buckets.insert("modules/a/main.tf".to_owned(), module_a);
        buckets.insert("modules/b/main.tf".to_owned(), module_b);

        let ambiguous_unresolved = run_rebind_remaining(&mut buckets);

        assert_eq!(ambiguous_unresolved, 0);
        let rebound = &buckets["modules/a/main.tf"].edges[0];
        assert_eq!(
            rebound.target_stable_symbol_id.as_deref(),
            Some("sym:a-web")
        );
        assert_eq!(rebound.bind_method.as_deref(), Some("address_module_scope"));
    }

    #[test]
    fn rebind_falls_back_to_workspace_singleton_and_keeps_ambiguity_unresolved() {
        let mut root = empty_bucket("main.tf", "oid-root");
        root.symbols.push(hcl_symbol(
            "sym:trail",
            "main.tf",
            "aws_cloudtrail.t",
            "resource",
        ));
        root.edges
            .push(address_edge("sym:trail", "aws_s3_bucket.logs"));
        root.edges
            .push(address_edge("sym:trail", "aws_instance.web"));

        let mut storage = empty_bucket("modules/storage/main.tf", "oid-storage");
        storage.symbols.push(hcl_symbol(
            "sym:logs",
            "modules/storage/main.tf",
            "aws_s3_bucket.logs",
            "resource",
        ));

        let mut module_a = empty_bucket("modules/a/main.tf", "oid-a");
        module_a.symbols.push(hcl_symbol(
            "sym:a-web",
            "modules/a/main.tf",
            "aws_instance.web",
            "resource",
        ));
        let mut module_b = empty_bucket("modules/b/main.tf", "oid-b");
        module_b.symbols.push(hcl_symbol(
            "sym:b-web",
            "modules/b/main.tf",
            "aws_instance.web",
            "resource",
        ));

        let mut buckets = BTreeMap::new();
        buckets.insert("main.tf".to_owned(), root);
        buckets.insert("modules/storage/main.tf".to_owned(), storage);
        buckets.insert("modules/a/main.tf".to_owned(), module_a);
        buckets.insert("modules/b/main.tf".to_owned(), module_b);

        let ambiguous_unresolved = run_rebind_remaining(&mut buckets);

        assert_eq!(ambiguous_unresolved, 1);
        let edges = &buckets["main.tf"].edges;
        let singleton = edges
            .iter()
            .find(|edge| edge.target_label.as_deref() == Some("aws_s3_bucket.logs"))
            .expect("singleton address edge");
        assert_eq!(
            singleton.target_stable_symbol_id.as_deref(),
            Some("sym:logs")
        );
        assert_eq!(singleton.bind_method.as_deref(), Some("address_singleton"));

        let ambiguous = edges
            .iter()
            .find(|edge| edge.target_label.as_deref() == Some("aws_instance.web"))
            .expect("ambiguous address edge");
        assert_eq!(ambiguous.target_stable_symbol_id, None);
    }

    #[test]
    fn rebind_never_binds_addresses_to_foreign_kinds_or_families() {
        let mut root = empty_bucket("main.tf", "oid-root");
        root.symbols
            .push(hcl_symbol("sym:eip", "main.tf", "aws_eip.ip", "resource"));
        root.edges
            .push(address_edge("sym:eip", "aws_instance.ghost"));

        let mut docs = empty_bucket("docs/notes.md", "oid-docs");
        docs.symbols.push(hcl_symbol(
            "sym:md-section",
            "docs/notes.md",
            "aws_instance.ghost",
            "section",
        ));

        let mut buckets = BTreeMap::new();
        buckets.insert("main.tf".to_owned(), root);
        buckets.insert("docs/notes.md".to_owned(), docs);

        run_rebind_remaining(&mut buckets);

        let rebound = &buckets["main.tf"].edges[0];
        assert_eq!(
            rebound.target_stable_symbol_id, None,
            "a markdown section sharing the address text must never satisfy the rebind"
        );
    }

    #[test]
    fn compose_artifact_dedupes_external_imports_and_keeps_workspace_imports() {
        let mut local_bucket = empty_bucket("src/local.rs", "oid-local");
        let mut local_symbol = symbol("sym:local", "src/local.rs");
        local_symbol.entity_name = "Local".to_owned();
        local_symbol.qualified_name = "local::Local".to_owned();
        local_symbol.symbol_kind = "struct".to_owned();
        local_bucket.symbols.push(local_symbol);
        local_bucket.symbol_node_ids.push(NodeId(10));

        let mut shadow_bucket = empty_bucket("src/shadow.rs", "oid-shadow");
        for (stable_symbol_id, node_id) in [("sym:shadow-a", 11), ("sym:shadow-b", 12)] {
            let mut shadow_symbol = symbol(stable_symbol_id, "src/shadow.rs");
            shadow_symbol.entity_name = "Deserialize".to_owned();
            shadow_symbol.qualified_name = format!("shadow::{stable_symbol_id}");
            shadow_symbol.symbol_kind = "struct".to_owned();
            shadow_bucket.symbols.push(shadow_symbol);
            shadow_bucket.symbol_node_ids.push(NodeId(node_id));
        }

        let mut a_bucket = empty_bucket("src/a.rs", "oid-a");
        a_bucket.symbols.push(symbol("sym:a", "src/a.rs"));
        a_bucket.symbol_node_ids.push(NodeId(20));
        let mut a_external = edge("sym:a", None, RelationKind::Imports);
        a_external.target_label = Some("Deserialize".to_owned());
        a_external.import_path = Some("serde::Deserialize".to_owned());
        a_bucket.edges.push(a_external);
        let mut a_workspace = edge("sym:a", None, RelationKind::Imports);
        a_workspace.target_label = Some("Local".to_owned());
        a_workspace.import_path = Some("crate::local::Local".to_owned());
        a_bucket.edges.push(a_workspace);

        let mut b_bucket = empty_bucket("src/b.rs", "oid-b");
        b_bucket.symbols.push(symbol("sym:b", "src/b.rs"));
        b_bucket.symbol_node_ids.push(NodeId(30));
        let mut b_external = edge("sym:b", None, RelationKind::Imports);
        b_external.target_label = Some("Deserialize".to_owned());
        b_external.import_path = Some("serde::Deserialize".to_owned());
        b_bucket.edges.push(b_external);

        let buckets = BTreeMap::from([
            ("src/a.rs".to_owned(), a_bucket),
            ("src/b.rs".to_owned(), b_bucket),
            ("src/local.rs".to_owned(), local_bucket),
            ("src/shadow.rs".to_owned(), shadow_bucket),
        ]);
        let current_entries = current_entries([
            ("src/a.rs", "oid-a"),
            ("src/b.rs", "oid-b"),
            ("src/local.rs", "oid-local"),
            ("src/shadow.rs", "oid-shadow"),
        ]);

        let artifact = compose_artifact(
            buckets,
            &current_entries,
            "test-manifest".to_owned(),
            Vec::new(),
        );

        let external_id = stable_symbol_id_for_external_path("serde::Deserialize");
        let external_symbols = artifact
            .symbols
            .iter()
            .filter(|symbol| symbol.symbol_kind == "external")
            .collect::<Vec<_>>();
        assert_eq!(external_symbols.len(), 1);
        assert_eq!(external_symbols[0].stable_symbol_id, external_id);
        assert_eq!(external_symbols[0].file_path, EXTERNAL_FILE_PATH);
        assert_eq!(external_symbols[0].qualified_name, "serde::Deserialize");
        assert_eq!(external_symbols[0].entity_name, "Deserialize");

        let external_symbol_index = artifact
            .symbols
            .iter()
            .position(|symbol| symbol.stable_symbol_id == external_id)
            .expect("external symbol");
        let external_node_id = artifact.symbol_node_ids[external_symbol_index];
        assert!(!artifact
            .file_manifests
            .iter()
            .any(|manifest| manifest.node_ids.contains(&external_node_id)));
        assert!(artifact.symbol_snapshots.is_empty());
        assert!(artifact.temporal_edges.is_empty());

        let external_import_edges = artifact
            .edges
            .iter()
            .filter(|edge| edge.import_path.as_deref() == Some("serde::Deserialize"))
            .collect::<Vec<_>>();
        assert_eq!(external_import_edges.len(), 2);
        assert!(external_import_edges.iter().all(|edge| {
            edge.target_stable_symbol_id.as_deref() == Some(external_id.as_str())
                && edge.bind_method.as_deref() == Some("external")
        }));

        let workspace_import = artifact
            .edges
            .iter()
            .find(|edge| edge.import_path.as_deref() == Some("crate::local::Local"))
            .expect("workspace import");
        assert_eq!(
            workspace_import.target_stable_symbol_id.as_deref(),
            Some("sym:local")
        );
        assert_ne!(workspace_import.bind_method.as_deref(), Some("external"));
    }

    #[test]
    fn build_facts_resolves_cross_crate_workspace_import_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("crates/spur-graph/src")).expect("mkdir spur-graph");
        fs::create_dir_all(root.join("crates/spur-core/src")).expect("mkdir spur-core");
        fs::write(
            root.join("crates/spur-graph/src/lib.rs"),
            "pub fn build_facts() {}\n",
        )
        .expect("write spur-graph lib.rs");
        fs::write(
            root.join("crates/spur-core/src/lib.rs"),
            "use spur_graph::build_facts;\npub fn run() { build_facts(); }\n",
        )
        .expect("write spur-core lib.rs");

        let facts = build_facts(root, None).expect("extract").0;
        let artifact = artifact_from_facts(&facts, root).expect("artifact");
        let target = artifact
            .symbols
            .iter()
            .find(|symbol| {
                symbol.file_path == "crates/spur-graph/src/lib.rs"
                    && symbol.entity_name == "build_facts"
                    && symbol.symbol_kind == "function"
            })
            .expect("spur_graph::build_facts symbol");
        let import_edge = artifact
            .edges
            .iter()
            .find(|edge| {
                edge.relation == RelationKind::Imports
                    && edge.import_path.as_deref() == Some("spur_graph::build_facts")
            })
            .expect("spur_graph::build_facts import");

        assert_eq!(
            import_edge.target_stable_symbol_id.as_deref(),
            Some(target.stable_symbol_id.as_str())
        );
        assert_eq!(import_edge.bind_method.as_deref(), Some("import_path"));
    }

    #[test]
    fn build_facts_external_imports_share_one_synthetic_external_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
mod a;
mod b;
mod local;
"#,
        )
        .expect("write lib.rs");
        fs::write(
            root.join("src/a.rs"),
            r#"
use serde::Deserialize;
use crate::local::Local;

pub fn consume_a(_local: Local) {}
"#,
        )
        .expect("write a.rs");
        fs::write(
            root.join("src/b.rs"),
            r#"
use serde::Deserialize;

pub fn consume_b() {}
"#,
        )
        .expect("write b.rs");
        fs::write(root.join("src/local.rs"), "pub struct Local;\n").expect("write local.rs");

        let facts = build_facts(root, None).expect("extract").0;
        let artifact = artifact_from_facts(&facts, root).expect("artifact");

        let external_id = stable_symbol_id_for_external_path("serde::Deserialize");
        let external_symbols = artifact
            .symbols
            .iter()
            .filter(|symbol| symbol.symbol_kind == "external")
            .collect::<Vec<_>>();
        assert_eq!(external_symbols.len(), 1);
        assert_eq!(external_symbols[0].stable_symbol_id, external_id);

        let inbound_external_edges = artifact
            .edges
            .iter()
            .filter(|edge| edge.target_stable_symbol_id.as_deref() == Some(external_id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(inbound_external_edges.len(), 2);
        assert!(inbound_external_edges.iter().all(|edge| {
            edge.relation == RelationKind::Imports
                && edge.bind_method.as_deref() == Some("external")
                && edge.import_path.as_deref() == Some("serde::Deserialize")
        }));

        let local_symbol = artifact
            .symbols
            .iter()
            .find(|symbol| symbol.entity_name == "Local" && symbol.symbol_kind == "struct")
            .expect("Local struct");
        let local_import = artifact
            .edges
            .iter()
            .find(|edge| edge.import_path.as_deref() == Some("crate::local::Local"))
            .expect("Local import");
        assert_eq!(
            local_import.target_stable_symbol_id.as_deref(),
            Some(local_symbol.stable_symbol_id.as_str())
        );
        assert_ne!(local_import.bind_method.as_deref(), Some("external"));
    }

    #[test]
    fn rebind_applies_range_guards_to_relational_edges() {
        let mut source_bucket = empty_bucket("src/source.rs", "oid-source");
        let mut foo_symbol = symbol("sym:foo", "src/source.rs");
        foo_symbol.entity_name = "Foo".to_owned();
        foo_symbol.qualified_name = "Foo".to_owned();
        foo_symbol.symbol_kind = "trait".to_owned();
        source_bucket.symbols.push(foo_symbol);

        let mut extends_send = edge("sym:foo", None, RelationKind::Extends);
        extends_send.target_label = Some("Send".to_owned());
        source_bucket.edges.push(extends_send);

        let mut extends_base = edge("sym:foo", None, RelationKind::Extends);
        extends_base.target_label = Some("Base".to_owned());
        source_bucket.edges.push(extends_base);

        let mut stamped_extends = edge("sym:foo", Some("sym:send_trait"), RelationKind::Extends);
        stamped_extends.target_label = Some("Send".to_owned());
        stamped_extends.bind_method = Some("relational".to_owned());
        source_bucket.edges.push(stamped_extends);

        let mut calls_field = edge("sym:foo", None, RelationKind::Calls);
        calls_field.target_label = Some("field".to_owned());
        source_bucket.edges.push(calls_field);

        let mut def_bucket = empty_bucket("src/def.rs", "oid-def");
        let mut send_variant = symbol("sym:send_variant", "src/def.rs");
        send_variant.entity_name = "Send".to_owned();
        send_variant.qualified_name = "E::Send".to_owned();
        send_variant.symbol_kind = "enum_variant".to_owned();
        def_bucket.symbols.push(send_variant);

        let mut send_trait = symbol("sym:send_trait", "src/def.rs");
        send_trait.entity_name = "SendTrait".to_owned();
        send_trait.qualified_name = "SendTrait".to_owned();
        send_trait.symbol_kind = "trait".to_owned();
        def_bucket.symbols.push(send_trait);

        let mut base_trait = symbol("sym:base_trait", "src/def.rs");
        base_trait.entity_name = "Base".to_owned();
        base_trait.qualified_name = "Base".to_owned();
        base_trait.symbol_kind = "trait".to_owned();
        def_bucket.symbols.push(base_trait);

        let mut field_symbol = symbol("sym:field", "src/def.rs");
        field_symbol.entity_name = "field".to_owned();
        field_symbol.qualified_name = "S::field".to_owned();
        field_symbol.symbol_kind = "field".to_owned();
        def_bucket.symbols.push(field_symbol);

        let mut buckets = BTreeMap::new();
        buckets.insert("src/def.rs".to_owned(), def_bucket);
        buckets.insert("src/source.rs".to_owned(), source_bucket);

        rebind_cross_file_edges(&mut buckets);

        let edges = &buckets["src/source.rs"].edges;
        assert_eq!(
            edges[0].target_stable_symbol_id, None,
            "extends must not rebind to an enum_variant"
        );
        assert_eq!(
            edges[1].target_stable_symbol_id.as_deref(),
            Some("sym:base_trait"),
            "extends should still rebind to an in-range trait"
        );
        assert_eq!(
            edges[2].target_stable_symbol_id.as_deref(),
            Some("sym:send_trait"),
            "relational stamped edges must not be re-resolved"
        );
        assert_eq!(
            edges[3].target_stable_symbol_id, None,
            "calls must not rebind to a field"
        );
    }

    #[test]
    fn compose_artifact_assigns_file_node_ids_for_manifest_only_files() {
        let mut buckets = BTreeMap::new();
        let mut lib_bucket = empty_bucket("src/lib.rs", "oid-lib");
        lib_bucket.file_node_id = Some(NodeId(10));
        buckets.insert("src/lib.rs".to_owned(), lib_bucket);

        let current_entries = BTreeMap::from([
            (
                "README.txt".to_owned(),
                CurrentFileEntry {
                    path: "README.txt".to_owned(),
                    content_oid: "oid-readme".to_owned(),
                    extractable: false,
                },
            ),
            (
                "src/lib.rs".to_owned(),
                CurrentFileEntry {
                    path: "src/lib.rs".to_owned(),
                    content_oid: "oid-lib".to_owned(),
                    extractable: true,
                },
            ),
        ]);

        let artifact = compose_artifact(
            buckets,
            &current_entries,
            "test-manifest".to_owned(),
            Vec::new(),
        );

        assert_eq!(artifact.files.len(), 2);
        assert_eq!(artifact.file_node_ids.len(), artifact.files.len());
        let readme_index = artifact
            .files
            .iter()
            .position(|file| file.file_path == "README.txt")
            .expect("README file");
        assert_ne!(artifact.file_node_ids[readme_index], NodeId(10));
        let readme_manifest = artifact
            .file_manifests
            .iter()
            .find(|entry| entry.path == "README.txt")
            .expect("README manifest");
        assert!(readme_manifest.node_ids.is_empty());
    }

    #[test]
    fn mcp_tool_symbols_persist_through_artifact_build() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join("src/lib.rs"),
            r#"
struct ToolDefinition {
    name: String,
}

fn submit_plan_def() -> ToolDefinition {
    ToolDefinition {
        name: "submit_plan".into(),
        description: "".into(),
        input_schema: json!({}),
    }
}
"#,
        )
        .expect("write lib.rs");

        let facts = build_facts_for_paths(root, &[PathBuf::from("src/lib.rs")]).expect("extract");
        let artifact = artifact_from_facts(&facts, root).expect("artifact");

        assert!(
            artifact
                .symbols
                .iter()
                .any(|symbol| symbol.symbol_kind == "mcp_tool"),
            "artifact should contain at least one persisted MCP tool symbol"
        );
    }

    #[test]
    fn commit_nodes_do_not_persist_as_symbols() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), "pub fn alpha() {}\n").expect("write lib.rs");
        let facts = GraphFacts {
            nodes: vec![
                GraphNode {
                    node_id: NodeId(1),
                    stable_key: "file:src/lib.rs".to_owned(),
                    label: "src/lib.rs".to_owned(),
                    kind: NodeKind::File,
                    file_id: Some(FileId(1)),
                    source_span_id: Some(SpanId(1)),
                    first_seen_run_id: RunId(1),
                },
                GraphNode {
                    node_id: NodeId(2),
                    stable_key: "commit:abc123".to_owned(),
                    label: "abc123".to_owned(),
                    kind: NodeKind::Commit,
                    file_id: None,
                    source_span_id: Some(SpanId(1)),
                    first_seen_run_id: RunId(1),
                },
            ],
            edges: Vec::new(),
            spans: vec![SourceSpan {
                span_id: SpanId(1),
                file_id: FileId(1),
                start_byte: 0,
                end_byte: 0,
                start_line: 0,
                end_line: 0,
            }],
            macro_invocations: Vec::new(),
            macro_scopes: Vec::new(),
        };

        let artifact = artifact_from_facts(&facts, root).expect("artifact");

        assert!(
            !artifact
                .symbols
                .iter()
                .any(|symbol| symbol.symbol_kind == "commit"),
            "commit nodes belong in artifact.commits, not artifact.symbols"
        );
    }

    #[test]
    fn full_artifact_build_emits_phase_tracing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").expect("write a.rs");
        let facts = build_facts(root, None).expect("extract").0;
        let subscriber = RecordingSubscriber::default();
        let _guard = tracing::subscriber::set_default(subscriber.clone());
        tracing::callsite::rebuild_interest_cache();

        let artifact = artifact_from_facts(&facts, root).expect("artifact");

        assert!(!artifact.files.is_empty());
        subscriber
            .assert_span_targets(["spur_graph::build::discover", "spur_graph::build::compose"]);
        subscriber.assert_summary("Full", "0");
    }

    #[test]
    fn incremental_artifact_build_emits_phase_tracing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").expect("write a.rs");
        fs::write(root.join("src/b.rs"), "pub fn beta() {}\n").expect("write b.rs");
        let subscriber = RecordingSubscriber::default();
        let _guard = tracing::subscriber::set_default(subscriber.clone());
        tracing::callsite::rebuild_interest_cache();
        let prev = artifact_from_facts(&build_facts(root, None).expect("extract").0, root)
            .expect("artifact");
        subscriber.clear();
        tracing::callsite::rebuild_interest_cache();
        fs::write(root.join("src/a.rs"), "pub fn alpha_changed() {}\n").expect("rewrite a.rs");

        let (_next, mode, _stats) =
            artifact_from_facts_incremental(&prev, root).expect("incremental");

        assert_eq!(mode, BuildMode::Incremental);
        subscriber.assert_span_targets([
            "spur_graph::build::discover",
            "spur_graph::build::rebucket",
            "spur_graph::build::changed_paths",
            "spur_graph::build::extract_changed",
            "spur_graph::build::compose",
        ]);
        subscriber.assert_summary("Incremental", "1");
    }

    #[test]
    fn incremental_reuses_bucket_when_content_oid_is_identical() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").expect("write a.rs");
        fs::write(root.join("src/b.rs"), "pub fn beta() {}\n").expect("write b.rs");

        let mut prev = artifact_from_facts(&build_facts(root, None).expect("extract").0, root)
            .expect("artifact");
        let b_content_oid = prev
            .file_manifests
            .iter()
            .find(|entry| entry.path == "src/b.rs")
            .expect("b manifest")
            .content_oid
            .clone();
        assert!(
            !b_content_oid.is_empty(),
            "full build must stamp content_oid"
        );

        let marker = "reused-bucket-marker".to_owned();
        prev.symbols
            .iter_mut()
            .find(|symbol| symbol.file_path == "src/b.rs")
            .expect("b symbol")
            .enclosing_scope = Some(marker.clone());

        fs::write(root.join("src/a.rs"), "pub fn alpha_changed() {}\n").expect("rewrite a.rs");

        let (next, mode, _stats) =
            artifact_from_facts_incremental(&prev, root).expect("incremental");
        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(
            next.file_manifests
                .iter()
                .find(|entry| entry.path == "src/b.rs")
                .expect("next b manifest")
                .content_oid,
            b_content_oid
        );
        assert_eq!(
            next.symbols
                .iter()
                .find(|symbol| symbol.file_path == "src/b.rs")
                .expect("next b symbol")
                .enclosing_scope
                .as_deref(),
            Some(marker.as_str())
        );
        assert!(next
            .symbols
            .iter()
            .any(|symbol| symbol.file_path == "src/a.rs" && symbol.entity_name == "alpha_changed"));
    }

    #[test]
    fn incremental_uses_git_blob_oid_for_dirty_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_repo(root);
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").expect("write a.rs");
        git(root, &["add", "src/a.rs"]);
        git(root, &["commit", "-m", "add a"]);

        let prev = artifact_from_facts(&build_facts(root, None).expect("extract").0, root)
            .expect("artifact");

        let dirty_bytes = b"pub fn alpha_dirty() {}\n";
        fs::write(root.join("src/a.rs"), dirty_bytes).expect("dirty a.rs");

        let (next, mode, _stats) =
            artifact_from_facts_incremental(&prev, root).expect("incremental");
        assert_eq!(mode, BuildMode::Incremental);
        let expected_oid = git_blob_oid(dirty_bytes);
        assert_eq!(
            next.file_manifests
                .iter()
                .find(|entry| entry.path == "src/a.rs")
                .expect("dirty manifest")
                .content_oid,
            expected_oid
        );
        assert_eq!(
            next.graph_content_hash,
            compute_graph_content_hash([("src/a.rs", expected_oid.as_str())])
        );
    }

    #[test]
    fn incremental_emits_tombstone_for_removed_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").expect("write a.rs");
        fs::write(root.join("src/b.rs"), "pub fn beta() {}\n").expect("write b.rs");

        let prev = artifact_from_facts(&build_facts(root, None).expect("extract").0, root)
            .expect("artifact");
        let removed_stable_file_id = prev
            .file_manifests
            .iter()
            .find(|entry| entry.path == "src/a.rs")
            .expect("a manifest")
            .stable_file_id
            .clone();

        fs::remove_file(root.join("src/a.rs")).expect("remove a.rs");
        let (next, mode, _stats) =
            artifact_from_facts_incremental(&prev, root).expect("incremental");

        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(next.tombstones.len(), 1);
        assert_eq!(next.tombstones[0].path, "src/a.rs");
        assert_eq!(next.tombstones[0].stable_file_id, removed_stable_file_id);
        assert!(!next
            .file_manifests
            .iter()
            .any(|entry| entry.path == "src/a.rs"));
    }

    #[test]
    fn incremental_emits_tombstone_for_symbol_removed_from_changed_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/lib.rs"), "pub fn alpha() {}\n").expect("write lib.rs");

        let prev = artifact_from_facts(&build_facts(root, None).expect("extract").0, root)
            .expect("artifact");
        let alpha_stable_symbol_id = prev
            .symbols
            .iter()
            .find(|symbol| symbol.file_path == "src/lib.rs" && symbol.entity_name == "alpha")
            .expect("alpha symbol")
            .stable_symbol_id
            .clone();

        fs::write(root.join("src/lib.rs"), "pub fn beta() {}\n").expect("rewrite lib.rs");
        let (next, mode, _stats) =
            artifact_from_facts_incremental(&prev, root).expect("incremental");
        let beta_stable_symbol_id = next
            .symbols
            .iter()
            .find(|symbol| symbol.file_path == "src/lib.rs" && symbol.entity_name == "beta")
            .expect("beta symbol")
            .stable_symbol_id
            .clone();

        assert_eq!(mode, BuildMode::Incremental);
        let tombstone_ids: BTreeSet<_> = next
            .tombstones
            .iter()
            .map(|tombstone| tombstone.stable_file_id.as_str())
            .collect();
        assert!(tombstone_ids.contains(alpha_stable_symbol_id.as_str()));
        assert!(!tombstone_ids.contains(beta_stable_symbol_id.as_str()));
    }

    #[test]
    fn incremental_stats_include_relative_changed_and_removed_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").expect("write a.rs");
        fs::write(root.join("src/b.rs"), "pub fn beta() {}\n").expect("write b.rs");

        let prev = artifact_from_facts(&build_facts(root, None).expect("extract").0, root)
            .expect("artifact");

        fs::write(root.join("src/b.rs"), "pub fn beta_changed() {}\n").expect("rewrite b.rs");
        fs::write(root.join("src/c.rs"), "pub fn gamma() {}\n").expect("write c.rs");
        fs::remove_file(root.join("src/a.rs")).expect("remove a.rs");

        let (_next, mode, stats) =
            artifact_from_facts_incremental(&prev, root).expect("incremental");

        assert_eq!(mode, BuildMode::Incremental);
        assert_eq!(
            stats.changed_or_added_paths,
            std::collections::BTreeSet::from(["src/b.rs".to_owned(), "src/c.rs".to_owned()])
        );
        assert_eq!(
            stats.removed_paths,
            std::collections::BTreeSet::from(["src/a.rs".to_owned()])
        );
    }

    #[derive(Clone, Default)]
    struct RecordingSubscriber {
        inner: Arc<RecordingInner>,
    }

    #[derive(Default)]
    struct RecordingInner {
        next_id: AtomicU64,
        spans: Mutex<Vec<TraceRecord>>,
        events: Mutex<Vec<TraceRecord>>,
    }

    #[derive(Debug, Clone)]
    struct TraceRecord {
        target: &'static str,
        name: &'static str,
        fields: BTreeMap<String, String>,
    }

    impl RecordingSubscriber {
        fn clear(&self) {
            self.inner.spans.lock().expect("spans").clear();
            self.inner.events.lock().expect("events").clear();
        }

        fn assert_span_targets<const N: usize>(&self, expected_targets: [&str; N]) {
            let actual = self
                .inner
                .spans
                .lock()
                .expect("spans")
                .iter()
                .map(|record| record.target)
                .collect::<Vec<_>>();
            for expected in expected_targets {
                assert!(
                    actual.contains(&expected),
                    "missing span target {expected}; actual targets: {actual:?}"
                );
            }
        }

        fn assert_summary(&self, expected_mode: &str, expected_changed: &str) {
            let events = self.inner.events.lock().expect("events");
            let summary = events
                .iter()
                .find(|record| {
                    record.target == "spur_graph::build"
                        && record.name.starts_with("event")
                        && record.fields.get("mode").map(String::as_str) == Some(expected_mode)
                })
                .unwrap_or_else(|| {
                    panic!(
                        "missing {expected_mode} summary event; actual events: {:?}",
                        *events
                    )
                });
            assert_eq!(
                summary.fields.get("changed").map(String::as_str),
                Some(expected_changed)
            );
            assert!(summary.fields.contains_key("elapsed_ms"));
            assert!(summary.fields.contains_key("files"));
            assert!(summary.fields.contains_key("symbols"));
            assert!(summary.fields.contains_key("edges"));
        }
    }

    impl Subscriber for RecordingSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, attrs: &Attributes<'_>) -> Id {
            let mut visitor = FieldVisitor::default();
            attrs.record(&mut visitor);
            let metadata = attrs.metadata();
            self.inner.spans.lock().expect("spans").push(TraceRecord {
                target: metadata.target(),
                name: metadata.name(),
                fields: visitor.fields,
            });
            Id::from_u64(self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            let metadata = event.metadata();
            self.inner.events.lock().expect("events").push(TraceRecord {
                target: metadata.target(),
                name: metadata.name(),
                fields: visitor.fields,
            });
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[derive(Default)]
    struct FieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl Visit for FieldVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    fn file(stable_file_id: &str, path: &str) -> GraphFileArtifact {
        GraphFileArtifact {
            stable_file_id: stable_file_id.to_owned(),
            file_path: path.to_owned(),
        }
    }

    fn manifest(stable_file_id: &str, path: &str) -> GraphFileManifestEntry {
        GraphFileManifestEntry {
            stable_file_id: stable_file_id.to_owned(),
            path: path.to_owned(),
            content_oid: format!("oid:{path}"),
            node_ids: Vec::new(),
        }
    }

    fn symbol(stable_symbol_id: &str, path: &str) -> GraphSymbolArtifact {
        GraphSymbolArtifact {
            stable_symbol_id: stable_symbol_id.to_owned(),
            file_path: path.to_owned(),
            byte_range: [0, 8],
            line_range: [1, 1],
            entity_name: stable_symbol_id.to_owned(),
            qualified_name: stable_symbol_id.to_owned(),
            symbol_kind: "function".to_owned(),
            anchor_hash: format!("hash:{stable_symbol_id}"),
            enclosing_scope: None,
        }
    }

    fn edge(
        source_stable_symbol_id: &str,
        target_stable_symbol_id: Option<&str>,
        relation: RelationKind,
    ) -> GraphEdgeArtifact {
        GraphEdgeArtifact {
            source_stable_symbol_id: source_stable_symbol_id.to_owned(),
            target_stable_symbol_id: target_stable_symbol_id.map(str::to_string),
            target_label: None,
            import_path: None,
            receiver_text: None,
            scope_text: None,
            relation,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: Some(graph_edge_kind_or_default(relation, None)),
            bind_method: None,
        }
    }

    fn current_entries<const N: usize>(
        entries: [(&str, &str); N],
    ) -> BTreeMap<String, CurrentFileEntry> {
        entries
            .into_iter()
            .map(|(path, content_oid)| {
                (
                    path.to_owned(),
                    CurrentFileEntry {
                        path: path.to_owned(),
                        content_oid: content_oid.to_owned(),
                        extractable: true,
                    },
                )
            })
            .collect()
    }

    fn init_repo(root: &Path) {
        git(root, &["init"]);
        git(root, &["config", "user.name", "Spur Test"]);
        git(root, &["config", "user.email", "spur@example.invalid"]);
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap_or_else(|err| panic!("failed to run git {args:?}: {err}"));
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
