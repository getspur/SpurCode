use std::{collections::HashSet, sync::Arc};

use spur_graph::{
    artifact_from_facts, build_facts,
    query_client::{GraphQueryClient, InMemoryClient, OverlayClient},
    GraphIndexArtifact, RustEvidenceAvailability, RustEvidenceFilter,
};

fn captured_empty_artifact() -> GraphIndexArtifact {
    let root = tempfile::tempdir().unwrap();
    let graph =
        artifact_from_facts(&build_facts(root.path(), None).unwrap().0, root.path()).unwrap();
    assert!(graph.file_manifests.is_empty());
    assert!(graph.rust_evidence.as_ref().unwrap().files.is_empty());
    graph
}

#[test]
fn irrelevant_shadow_preserves_absent_empty_capability() {
    let mut graph = captured_empty_artifact();
    graph.rust_evidence = None;
    let base = InMemoryClient::new(Arc::new(graph.clone()));
    let filter = RustEvidenceFilter::default();
    let original = base.rust_evidence(&filter).unwrap();
    assert_eq!(original.availability, RustEvidenceAvailability::Unavailable);

    let overlay = OverlayClient::from_artifacts(
        base,
        Arc::new(graph),
        HashSet::from(["README.md".to_owned()]),
    )
    .unwrap();
    let observed = overlay.rust_evidence(&filter).unwrap();
    assert_eq!(observed, original);
}

#[test]
fn irrelevant_shadow_preserves_captured_empty_coverage() {
    let graph = captured_empty_artifact();
    let overlay = OverlayClient::from_artifacts(
        InMemoryClient::new(Arc::new(graph.clone())),
        Arc::new(graph),
        HashSet::from(["README.md".to_owned()]),
    )
    .unwrap();
    let observed = overlay
        .rust_evidence(&RustEvidenceFilter::default())
        .unwrap();
    assert_eq!(observed.availability, RustEvidenceAvailability::Complete);
    assert_eq!(observed.source_domain, Some(Vec::new()));
    assert!(observed.files.is_empty());
    assert!(observed.origins.is_empty());
}
