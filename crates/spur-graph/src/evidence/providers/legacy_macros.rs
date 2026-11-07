//! Pure legacy macro table adapter. Derived carrier origins describe the
//! supplied validated graph manifest; the original format recorded no lengths.
use super::{canonical, integrity, layout, read_rows, semantic, supported, write_rows};
use crate::{evidence::*, *};
use anyhow::{ensure, Context as _};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, path::Path};

pub const PROVIDER_ID: &str = "spur.rust.macros";
pub struct LegacyMacroEvidenceProvider;

fn layouts() -> Vec<EvidenceTable> {
    vec![
        layout(
            "macro_invocations",
            fields(false),
            &["site_id"],
            &["file_path", "byte_range_start", "byte_range_end", "site_id"],
            false,
        ),
        layout(
            "macro_scopes",
            fields(true),
            &["scope_id"],
            &[
                "file_path",
                "byte_range_start",
                "byte_range_end",
                "scope_id",
            ],
            false,
        ),
    ]
}
fn carrier(graph: &GraphIndexArtifact) -> anyhow::Result<(Vec<EvidenceOrigin>, EvidenceDomain)> {
    let domain = LegacyMacroEvidenceProvider.source_domain(graph)?;
    let key = EvidenceOriginKey {
        manifest_version: graph.manifest_version.clone(),
        graph_content_hash: graph.graph_content_hash.clone(),
    };
    let origins = vec![EvidenceOrigin {
        key: key.clone(),
        sources: graph
            .file_manifests
            .iter()
            .map(|m| EvidenceSourceKey {
                file_path: m.path.clone(),
                stable_file_id: m.stable_file_id.clone(),
                source_oid: m.content_oid.clone(),
            })
            .collect(),
    }];
    Ok((
        origins,
        EvidenceDomain::Known {
            sources: domain
                .into_iter()
                .map(|source| EvidenceSourceCoverage {
                    source,
                    byte_length: None,
                    origin: Some(key.clone()),
                    coverage: EvidenceCoverage::Complete,
                })
                .collect(),
        },
    ))
}
fn metadata(
    set: &MacroInvocationEvidenceSet,
    manifest_version: &str,
) -> anyhow::Result<BTreeMap<String, String>> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"spur-macro-evidence-tables-v1\0");
    hash.update(&serde_json::to_vec(set)?);
    Ok(BTreeMap::from([
        ("schema_version".into(), set.schema_version.clone()),
        (
            "graph_schema_version".into(),
            crate::store::SCHEMA_VERSION.into(),
        ),
        ("extractor_version".into(), set.extractor_version.clone()),
        ("graph_content_hash".into(), set.graph_content_hash.clone()),
        ("manifest_version".into(), manifest_version.into()),
        ("source_scope".into(), "physical_rust_files".into()),
        (
            "checksum_blake3".into(),
            hash.finalize().to_hex().to_string(),
        ),
    ]))
}
fn extension(
    graph: &GraphIndexArtifact,
    set: &MacroInvocationEvidenceSet,
) -> anyhow::Result<EvidenceExtension> {
    let (origins, domain) = carrier(graph)?;
    let mut meta = metadata(set, &graph.manifest_version)?;
    meta.remove("checksum_blake3");
    meta.insert(
        "original_format".into(),
        MACRO_INVOCATION_SCHEMA_VERSION.into(),
    );
    meta.insert("origin_binding".into(), "validated_graph_manifest".into());
    let descriptor = EvidenceDescriptor {
        provider_id: PROVIDER_ID.into(),
        schema_version: set.schema_version.clone(),
        extractor_version: set.extractor_version.clone(),
        component_versions: BTreeMap::new(),
        presence: EvidencePresence::Captured,
        metadata: meta,
    };
    let rows = encode(set)?;
    let mut tables = layouts();
    for (t, rows) in tables.iter_mut().zip(rows) {
        t.metadata = metadata(set, &graph.manifest_version)?;
        write_rows(t, &rows)?;
    }
    Ok(canonical(&EvidenceExtension {
        descriptor,
        origins,
        domain,
        tables,
    })?)
}
fn decode_projection(
    extension: &EvidenceExtension,
) -> Result<MacroInvocationEvidenceSet, EvidenceError> {
    supported(extension, PROVIDER_ID, MACRO_INVOCATION_SCHEMA_VERSION)?;
    (|| -> anyhow::Result<_> {
        ensure!(
            extension.descriptor.presence == EvidencePresence::Captured,
            "macro evidence is absent"
        );
        ensure!(
            extension.descriptor.extractor_version == crate::store::EXTRACTOR_VERSION
                && extension.descriptor.component_versions.is_empty(),
            "incompatible macro extractor"
        );
        ensure!(
            extension
                .descriptor
                .metadata
                .get("original_format")
                .map(String::as_str)
                == Some(MACRO_INVOCATION_SCHEMA_VERSION),
            "unauthenticated legacy original format"
        );
        let ext = canonical(extension)?;
        let mut rows = read_rows(&ext, &layouts())?.into_iter();
        let set = MacroInvocationEvidenceSet {
            schema_version: ext.descriptor.schema_version.clone(),
            extractor_version: ext.descriptor.extractor_version.clone(),
            graph_content_hash: ext
                .descriptor
                .metadata
                .get("graph_content_hash")
                .context("missing macro graph hash")?
                .clone(),
            source_scope: MacroEvidenceSourceScope::PhysicalRustFiles,
            rows: decode_invocations(rows.next().unwrap())?,
            scopes: decode_scopes(rows.next().unwrap())?,
        };
        Ok(set)
    })()
    .map_err(|e| semantic(PROVIDER_ID, e))
}
pub fn encode_macro_evidence(
    graph: &GraphIndexArtifact,
    set: &MacroInvocationEvidenceSet,
) -> Result<EvidenceExtension, EvidenceError> {
    set.validate_stored(graph)
        .map_err(|e| semantic(PROVIDER_ID, e))?;
    let ext = extension(graph, set).map_err(|e| semantic(PROVIDER_ID, e))?;
    integrity(graph, &ext)?;
    Ok(ext)
}
pub fn decode_macro_evidence(
    graph: &GraphIndexArtifact,
    ext: &EvidenceExtension,
) -> Result<MacroInvocationEvidenceSet, EvidenceError> {
    integrity(graph, ext)?;
    let set = decode_projection(ext)?;
    set.validate_stored(graph)
        .map_err(|e| semantic(PROVIDER_ID, e))?;
    let expected = extension(graph, &set).map_err(|e| semantic(PROVIDER_ID, e))?;
    if canonical(ext)? != expected {
        return Err(semantic(
            PROVIDER_ID,
            "macro metadata, scalar, origin or source domain mismatch",
        ));
    }
    Ok(set)
}
impl EvidenceProvider for LegacyMacroEvidenceProvider {
    fn provider_id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn supports_path(&self, path: &str) -> bool {
        super::physical_path(path)
            && Path::new(path)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rs"))
    }
    fn source_domain(
        &self,
        graph: &GraphIndexArtifact,
    ) -> Result<Vec<EvidenceSourceKey>, EvidenceError> {
        super::source_domain(graph, |p| self.supports_path(p)).map_err(|e| semantic(PROVIDER_ID, e))
    }
    fn validate_stored(
        &self,
        graph: &GraphIndexArtifact,
        ext: &EvidenceExtension,
    ) -> Result<EvidenceValidation, EvidenceError> {
        integrity(graph, ext)?;
        supported(ext, PROVIDER_ID, MACRO_INVOCATION_SCHEMA_VERSION)?;
        if ext.descriptor.presence == EvidencePresence::Absent {
            ensure_absent(ext).map_err(|e| semantic(PROVIDER_ID, e))?;
            self.source_domain(graph)?;
            return Ok(EvidenceValidation {
                status: EvidenceSemanticStatus::Validated,
                domain: EvidenceDomain::Unknown,
            });
        }
        decode_macro_evidence(graph, ext)?;
        Ok(EvidenceValidation {
            status: EvidenceSemanticStatus::Validated,
            domain: ext.domain.clone(),
        })
    }
    fn validate_sources(
        &self,
        graph: &GraphIndexArtifact,
        ext: &EvidenceExtension,
        sources: &[EvidenceSource<'_>],
    ) -> Result<(), EvidenceError> {
        let set = decode_macro_evidence(graph, ext)?;
        (|| -> anyhow::Result<()> {
            let domain = self.source_domain(graph)?;
            ensure!(
                sources.len() == domain.len()
                    && domain.iter().all(|s| sources
                        .iter()
                        .filter(
                            |b| b.file_path == s.file_path && git_blob_oid(b.bytes) == s.source_oid
                        )
                        .count()
                        == 1),
                "exact original macro source domain/OIDs required"
            );
            let original = sources
                .iter()
                .map(|s| MacroEvidenceSource {
                    file_path: s.file_path,
                    bytes: s.bytes,
                })
                .collect::<Vec<_>>();
            // Original saved-byte validator remains the first semantic boundary.
            set.validate(&graph.graph_content_hash, &original, &set.scopes)?;
            // Reuse the existing pure extractor to authenticate path text,
            // attributes, owners and completeness; no disk reads or new parser.
            let root = Path::new("/");
            let mut builder = crate::extract::tree_sitter::FactBuilder::new(root);
            let mut extractor = crate::extract::tree_sitter::BytesExtractor::for_language(
                crate::extract::languages::Language::Rust,
            )?;
            let mut ordered = sources.iter().collect::<Vec<_>>();
            ordered.sort_by_key(|s| s.file_path);
            for source in ordered {
                match std::str::from_utf8(source.bytes) {
                    Ok(_) => extractor.extract_graph_facts(
                        &mut builder,
                        &root.join(source.file_path),
                        source.bytes,
                    )?,
                    Err(_) => ensure!(
                        !set.rows.iter().any(|r| r.file_path == source.file_path)
                            && !set.scopes.iter().any(|s| s.file_path == source.file_path),
                        "invalid UTF-8 source has macro records"
                    ),
                }
            }
            let facts = builder.into_facts();
            let mut expected = set.clone();
            expected.rows = facts.macro_invocations;
            expected.scopes = facts.macro_scopes;
            sort(&mut expected);
            ensure!(
                set == expected,
                "macro evidence differs from original source recollection"
            );
            Ok(())
        })()
        .map_err(|e| semantic(PROVIDER_ID, e))
    }
}
fn ensure_absent(ext: &EvidenceExtension) -> anyhow::Result<()> {
    ensure!(
        ext.descriptor.extractor_version == crate::store::EXTRACTOR_VERSION
            && ext.descriptor.component_versions.is_empty(),
        "incompatible absent macro descriptor"
    );
    Ok(())
}
fn sort(set: &mut MacroInvocationEvidenceSet) {
    set.rows.sort_by(|a, b| {
        (&a.file_path, a.byte_range, &a.site_id).cmp(&(&b.file_path, b.byte_range, &b.site_id))
    });
    set.scopes.sort_by(|a, b| {
        (&a.file_path, a.byte_range, &a.scope.stable_symbol_id).cmp(&(
            &b.file_path,
            b.byte_range,
            &b.scope.stable_symbol_id,
        ))
    });
}

pub fn query_macro_evidence<C: GraphQueryClient + ?Sized>(
    client: &C,
    filter: &MacroInvocationFilter,
) -> anyhow::Result<MacroInvocationQueryResult> {
    filter.validate_shape()?;
    let result = client.evidence(&EvidenceFilter {
        provider_id: PROVIDER_ID.into(),
        file_path: filter.file_path.clone(),
        source_oid: filter.source_oid.clone(),
    })?;
    let mut output = MacroInvocationQueryResult::unavailable();
    output.availability = match result.availability {
        EvidenceAvailability::Complete => MacroEvidenceAvailability::Complete,
        EvidenceAvailability::Partial => MacroEvidenceAvailability::Partial,
        EvidenceAvailability::Unavailable => MacroEvidenceAvailability::Unavailable,
        EvidenceAvailability::UnsupportedSource => MacroEvidenceAvailability::UnsupportedSource,
    };
    if result.semantic_status != EvidenceSemanticStatus::Validated {
        output.availability = MacroEvidenceAvailability::Unavailable;
        return Ok(output);
    }
    let Some(ext) = super::query_extension(&result) else {
        return Ok(output);
    };
    if ext.descriptor.presence == EvidencePresence::Absent {
        output.availability = MacroEvidenceAvailability::Unavailable;
        return Ok(output);
    }
    let set = decode_projection(&ext)?;
    output.source_scope = Some(set.source_scope);
    if let EvidenceDomain::Known { sources } = &result.domain {
        for origin in &result.origins {
            let source_oids = sources
                .iter()
                .filter(|s| {
                    s.origin.as_ref() == Some(&origin.key)
                        && filter
                            .file_path
                            .as_ref()
                            .is_none_or(|p| p == &s.source.file_path)
                })
                .map(|s| (s.source.file_path.clone(), s.source.source_oid.clone()))
                .collect();
            output.snapshots.push(MacroEvidenceSnapshot {
                graph_content_hash: origin.key.graph_content_hash.clone(),
                schema_version: set.schema_version.clone(),
                extractor_version: set.extractor_version.clone(),
                source_oids,
            });
        }
    }
    output.rows = set.rows.into_iter().filter(|r| filter.matches(r)).collect();
    output.scopes = set.scopes;
    output.retain_referenced_scopes();
    output.sort();
    Ok(output)
}

fn encode(set: &MacroInvocationEvidenceSet) -> anyhow::Result<[Vec<Value>; 2]> {
    let invocations = set
        .rows
        .iter()
        .map(|row| {
            let mut value = serde_json::to_value(row)?;
            let object = value.as_object_mut().context("invalid invocation object")?;
            let scope = object
                .remove("enclosing")
                .context("missing enclosing scope")?;
            object.insert(
                "enclosing_scope_id".into(),
                scope["stable_symbol_id"].clone(),
            );
            object.insert("enclosing_scope_kind".into(), scope["kind"].clone());
            flatten_ranges(object)?;
            for name in ["attributes", "resolution"] {
                let nested = object.remove(name).context("missing nested macro field")?;
                object.insert(name.into(), Value::String(serde_json::to_string(&nested)?));
            }
            Ok(value)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let scopes = set
        .scopes
        .iter()
        .map(|scope| {
            let mut value = serde_json::to_value(scope)?;
            let object = value.as_object_mut().context("invalid scope object")?;
            let scope = object.remove("scope").context("missing scope")?;
            object.insert("scope_id".into(), scope["stable_symbol_id"].clone());
            object.insert("kind".into(), scope["kind"].clone());
            flatten_ranges(object)?;
            Ok(value)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok([invocations, scopes])
}
fn flatten_ranges(object: &mut Map<String, Value>) -> anyhow::Result<()> {
    for (name, start, end) in [
        ("byte_range", "byte_range_start", "byte_range_end"),
        ("line_range", "line_start", "line_end"),
    ] {
        let range = object.remove(name).context("missing macro range")?;
        object.insert(start.into(), range[0].clone());
        object.insert(end.into(), range[1].clone());
    }
    Ok(())
}

fn unflatten_ranges(object: &mut Map<String, Value>) -> anyhow::Result<()> {
    for (name, start, end) in [
        ("byte_range", "byte_range_start", "byte_range_end"),
        ("line_range", "line_start", "line_end"),
    ] {
        let start = object.remove(start).context("missing macro range start")?;
        let end = object.remove(end).context("missing macro range end")?;
        object.insert(name.into(), Value::Array(vec![start, end]));
    }
    Ok(())
}
fn fields(scopes: bool) -> Vec<EvidenceColumn> {
    let mut fields = vec![
        column("file_path", EvidenceColumnType::Utf8, false),
        column("source_oid", EvidenceColumnType::Utf8, false),
    ];
    for name in [
        "byte_range_start",
        "byte_range_end",
        "line_start",
        "line_end",
    ] {
        fields.push(column(name, EvidenceColumnType::Int64, false));
    }
    let names: &[&str] = if scopes {
        &["scope_id", "kind"]
    } else {
        &[
            "site_id",
            "enclosing_scope_id",
            "enclosing_scope_kind",
            "raw_macro_path",
            "normalized_macro_path",
            "attributes",
            "test_context",
            "execution_phase",
            "resolution",
        ]
    };
    fields.extend(
        names
            .iter()
            .map(|name| column(*name, EvidenceColumnType::Utf8, false)),
    );
    if !scopes {
        fields.extend([
            column("caller_stable_symbol_id", EvidenceColumnType::Utf8, true),
            column("sink_family_candidate", EvidenceColumnType::Utf8, true),
            column(
                "debug_assertions_candidate",
                EvidenceColumnType::Boolean,
                false,
            ),
        ]);
    }
    fields
}
fn decode_invocations(values: Vec<Value>) -> anyhow::Result<Vec<crate::MacroInvocationEvidence>> {
    values
        .into_iter()
        .map(|mut value| {
            let object = value.as_object_mut().context("invalid invocation object")?;
            let id = object
                .remove("enclosing_scope_id")
                .context("missing enclosing scope ID")?;
            let kind = object
                .remove("enclosing_scope_kind")
                .context("missing enclosing scope kind")?;
            object.insert(
                "enclosing".into(),
                serde_json::json!({"stable_symbol_id":id,"kind":kind}),
            );
            unflatten_ranges(object)?;
            for name in ["attributes", "resolution"] {
                let nested = object.remove(name).context("missing nested macro field")?;
                object.insert(
                    name.into(),
                    serde_json::from_str(nested.as_str().context("invalid macro JSON column")?)?,
                );
            }
            Ok(serde_json::from_value(value)?)
        })
        .collect::<anyhow::Result<Vec<_>>>()
}

fn decode_scopes(values: Vec<Value>) -> anyhow::Result<Vec<crate::MacroScopeEvidence>> {
    values
        .into_iter()
        .map(|mut value| {
            let object = value.as_object_mut().context("invalid scope object")?;
            let id = object.remove("scope_id").context("missing scope ID")?;
            let kind = object.remove("kind").context("missing scope kind")?;
            object.insert(
                "scope".into(),
                serde_json::json!({"stable_symbol_id":id,"kind":kind}),
            );
            unflatten_ranges(object)?;
            Ok(serde_json::from_value(value)?)
        })
        .collect::<anyhow::Result<Vec<_>>>()
}

fn column(name: &str, data_type: EvidenceColumnType, nullable: bool) -> EvidenceColumn {
    EvidenceColumn {
        name: name.into(),
        data_type,
        nullable,
    }
}
