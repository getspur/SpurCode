use anyhow::Context as _;
use serde::Serialize;

use crate::{
    CommitArtifact, GraphEdgeArtifact, GraphFileArtifact, GraphFileManifestEntry,
    GraphIndexArtifact, GraphSymbolArtifact, GraphTombstoneEntry, SymbolSnapshotArtifact,
    TemporalEdgeArtifact,
};

#[derive(Serialize)]
pub(crate) struct GraphArtifactBodyForHash<'a> {
    pub files: &'a [GraphFileArtifact],
    pub symbols: &'a [GraphSymbolArtifact],
    pub edges: &'a [GraphEdgeArtifact],
    pub file_manifests: &'a [GraphFileManifestEntry],
    pub graph_content_hash: &'a str,
    pub manifest_version: &'a str,
    pub tombstones: &'a [GraphTombstoneEntry],
    pub commits: &'a [CommitArtifact],
    pub symbol_snapshots: &'a [SymbolSnapshotArtifact],
    pub temporal_edges: &'a [TemporalEdgeArtifact],
    pub diagnostics: &'a [String],
    // Omit absent evidence to preserve content hashes of legitimate legacy bodies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macro_invocations: Option<&'a crate::MacroInvocationEvidenceSet>,
}

pub fn artifact_content_hash_blake3_hex(artifact: &GraphIndexArtifact) -> anyhow::Result<String> {
    let body = GraphArtifactBodyForHash {
        files: &artifact.files,
        symbols: &artifact.symbols,
        edges: &artifact.edges,
        file_manifests: &artifact.file_manifests,
        graph_content_hash: &artifact.graph_content_hash,
        manifest_version: &artifact.manifest_version,
        tombstones: &artifact.tombstones,
        commits: &artifact.commits,
        symbol_snapshots: &artifact.symbol_snapshots,
        temporal_edges: &artifact.temporal_edges,
        diagnostics: &artifact.diagnostics,
        macro_invocations: artifact.macro_invocations.as_ref(),
    };
    let canonical_json = serde_json::to_vec(&body)
        .context("failed to encode graph artifact body for content hash")?;
    Ok(blake3::hash(&canonical_json).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CommitArtifact, Confidence, EdgeEndpoint, GraphEdgeKind, GraphIndexHeader, NodeId,
        RelationKind, SnapshotKey, SymbolSnapshotArtifact, TemporalEdgeArtifact,
    };

    #[test]
    fn canonical_bytes_snapshot_guard() {
        let artifact = GraphIndexArtifact {
            macro_invocations: Some(crate::MacroInvocationEvidenceSet {
                schema_version: crate::MACRO_INVOCATION_SCHEMA_VERSION.into(),
                extractor_version: crate::store::EXTRACTOR_VERSION.into(),
                graph_content_hash: "graph-content-hash".into(),
                source_scope: crate::MacroEvidenceSourceScope::PhysicalRustFiles,
                rows: Vec::new(),
                scopes: Vec::new(),
            }),
            header: GraphIndexHeader {
                graph_index_version: "v5".to_owned(),
                content_hash_blake3: Some("header-content-hash".to_owned()),
            },
            manifest_version: "manifest-v1".to_owned(),
            graph_content_hash: "graph-content-hash".to_owned(),
            file_manifests: vec![GraphFileManifestEntry {
                stable_file_id: "file:src/lib.rs".to_owned(),
                path: "src/lib.rs".to_owned(),
                content_oid: "blob-oid".to_owned(),
                node_ids: vec![NodeId(1), NodeId(2)],
            }],
            files: vec![GraphFileArtifact {
                stable_file_id: "file:src/lib.rs".to_owned(),
                file_path: "src/lib.rs".to_owned(),
            }],
            file_node_ids: vec![NodeId(1)],
            symbols: vec![GraphSymbolArtifact {
                stable_symbol_id: "sym:src/lib.rs:demo".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                byte_range: [10, 42],
                line_range: [2, 5],
                entity_name: "demo".to_owned(),
                qualified_name: "crate::demo".to_owned(),
                symbol_kind: "function".to_owned(),
                anchor_hash: "anchor-hash".to_owned(),
                enclosing_scope: Some("crate".to_owned()),
            }],
            symbol_node_ids: vec![NodeId(2)],
            edges: vec![GraphEdgeArtifact {
                source_stable_symbol_id: "sym:src/lib.rs:demo".to_owned(),
                target_stable_symbol_id: Some("sym:src/lib.rs:helper".to_owned()),
                target_label: Some("helper".to_owned()),
                import_path: None,
                receiver_text: None,
                scope_text: None,
                relation: RelationKind::Calls,
                confidence: Confidence::SyntaxExact,
                confidence_score: 0.875,
                change_kind: None,
                edge_kind: Some(GraphEdgeKind::Calls),
                bind_method: None,
            }],
            tombstones: vec![GraphTombstoneEntry {
                path: "src/old.rs".to_owned(),
                stable_file_id: "file:src/old.rs".to_owned(),
            }],
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };
        let body = GraphArtifactBodyForHash {
            files: &artifact.files,
            symbols: &artifact.symbols,
            edges: &artifact.edges,
            file_manifests: &artifact.file_manifests,
            graph_content_hash: &artifact.graph_content_hash,
            manifest_version: &artifact.manifest_version,
            tombstones: &artifact.tombstones,
            commits: &artifact.commits,
            symbol_snapshots: &artifact.symbol_snapshots,
            temporal_edges: &artifact.temporal_edges,
            diagnostics: &artifact.diagnostics,
            macro_invocations: artifact.macro_invocations.as_ref(),
        };
        let bytes = serde_json::to_vec(&body).expect("canonical JSON should serialize");

        insta::assert_snapshot!(
            "canonical_bytes_layout",
            std::str::from_utf8(&bytes).expect("canonical JSON is UTF-8")
        );
    }

    #[test]
    fn artifact_hash_changes_when_commits_change() {
        let artifact = minimal_hash_artifact();
        let mut mutated = artifact.clone();
        mutated.commits.push(commit("c1"));

        assert_hash_changes(&artifact, &mutated);
    }

    #[test]
    fn artifact_hash_changes_when_symbol_snapshots_change() {
        let artifact = minimal_hash_artifact();
        let mut mutated = artifact.clone();
        mutated
            .symbol_snapshots
            .push(symbol_snapshot("sym:src/lib.rs:demo", "c1"));

        assert_hash_changes(&artifact, &mutated);
    }

    #[test]
    fn artifact_hash_changes_when_temporal_edges_change() {
        let artifact = minimal_hash_artifact();
        let mut mutated = artifact.clone();
        mutated.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::File {
                path: "src/lib.rs".to_owned().into(),
            },
            target: EdgeEndpoint::Commit {
                sha: "c1".to_owned(),
            },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: None,
        });

        assert_hash_changes(&artifact, &mutated);
    }

    #[test]
    fn artifact_hash_changes_when_diagnostics_change() {
        let artifact = minimal_hash_artifact();
        let mut mutated = artifact.clone();
        mutated
            .diagnostics
            .push("parse_failed path=src/lib.rs sha=abc123".to_owned());

        assert_hash_changes(&artifact, &mutated);
    }

    fn assert_hash_changes(original: &GraphIndexArtifact, mutated: &GraphIndexArtifact) {
        let original_hash =
            artifact_content_hash_blake3_hex(original).expect("original hash should compute");
        let mutated_hash =
            artifact_content_hash_blake3_hex(mutated).expect("mutated hash should compute");

        assert_ne!(
            original_hash, mutated_hash,
            "canonical content hash should include persisted artifact collections"
        );
    }

    fn minimal_hash_artifact() -> GraphIndexArtifact {
        GraphIndexArtifact {
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "v5".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "manifest-v1".to_owned(),
            graph_content_hash: "graph-content-hash".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
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

    fn commit(sha: &str) -> CommitArtifact {
        CommitArtifact {
            sha: sha.to_owned(),
            parents: Vec::new(),
            author_time: 1,
            author_name: String::new(),
            author_email: String::new(),
            summary: format!("commit {sha}"),
        }
    }

    fn symbol_snapshot(stable_symbol_id: &str, commit: &str) -> SymbolSnapshotArtifact {
        SymbolSnapshotArtifact {
            key: SnapshotKey {
                stable_symbol_id: stable_symbol_id.to_owned(),
                commit: commit.to_owned(),
            },
            file_path: "src/lib.rs".to_owned().into(),
            entity_name: "demo".to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [10, 42],
            line_range: [2, 5],
            anchor_hash: "anchor-hash".to_owned(),
            tokens: Vec::new(),
        }
    }
}
