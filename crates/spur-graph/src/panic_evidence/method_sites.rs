use super::*;

pub(super) fn collect(
    graph: &GraphIndexArtifact,
    _sources: &[PanicEvidenceSource<'_>],
) -> anyhow::Result<MethodCallSiteEvidenceSet> {
    Ok(MethodCallSiteEvidenceSet {
        schema_version: METHOD_CALL_SITE_SCHEMA_VERSION.into(),
        extractor_version: METHOD_CALL_SITE_EXTRACTOR_VERSION.into(),
        graph_content_hash: graph.graph_content_hash.clone(),
        source_scope: MethodCallSourceScope::SuppliedPhysicalRustFiles,
        sources: Vec::new(),
        sites: Vec::new(),
    })
}
