//! Statically composed typed adapters. No discovery, storage or parser registration.

pub mod legacy_macros;
pub mod rust;

use crate::{evidence::*, GraphIndexArtifact};
use anyhow::{ensure, Context as _};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

static RUST: rust::RustEvidenceProvider = rust::RustEvidenceProvider;
static MACROS: legacy_macros::LegacyMacroEvidenceProvider =
    legacy_macros::LegacyMacroEvidenceProvider;
static PROVIDERS: [&dyn EvidenceProvider; 2] = [&RUST, &MACROS];

pub fn default_registry() -> EvidenceRegistry<'static> {
    EvidenceRegistry::new(&PROVIDERS).expect("built-in provider IDs are distinct")
}

fn semantic(id: &str, error: impl std::fmt::Display) -> EvidenceError {
    EvidenceError::Semantic {
        provider_id: id.into(),
        code: "typed_evidence".into(),
        message: error.to_string(),
    }
}

fn physical_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with("external://")
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn source_domain(
    graph: &GraphIndexArtifact,
    supports: impl Fn(&str) -> bool,
) -> anyhow::Result<Vec<EvidenceSourceKey>> {
    EvidenceInventory::empty().validate_integrity(graph)?;
    ensure!(
        graph.manifest_version == crate::store::current_manifest_version(),
        "incompatible graph manifest version"
    );
    let mut domain = Vec::new();
    let mut gitlinks = BTreeSet::new();
    for m in &graph.file_manifests {
        if m.content_oid.starts_with("gitlink:") {
            gitlinks.insert(m.path.as_str());
            continue;
        }
        if !supports(&m.path) {
            continue;
        }
        ensure!(
            graph.files.iter().filter(|f| f.file_path == m.path).count() == 1
                && graph
                    .files
                    .iter()
                    .filter(|f| f.stable_file_id == m.stable_file_id)
                    .count()
                    == 1
                && graph
                    .files
                    .iter()
                    .any(|f| f.file_path == m.path && f.stable_file_id == m.stable_file_id),
            "source is not an authoritative physical graph file"
        );
        domain.push(EvidenceSourceKey {
            file_path: m.path.clone(),
            stable_file_id: m.stable_file_id.clone(),
            source_oid: m.content_oid.clone(),
        });
    }
    for f in &graph.files {
        ensure!(
            !supports(&f.file_path)
                || f.file_path.contains("#cell:")
                || domain.iter().any(|s| s.file_path == f.file_path)
                || gitlinks.contains(f.file_path.as_str()),
            "physical source has no manifest"
        );
    }
    domain.sort_by(|a, b| {
        (&a.file_path, &a.stable_file_id, &a.source_oid).cmp(&(
            &b.file_path,
            &b.stable_file_id,
            &b.source_oid,
        ))
    });
    Ok(domain)
}

fn canonical(extension: &EvidenceExtension) -> Result<EvidenceExtension, EvidenceError> {
    let mut inventory = EvidenceInventory::empty();
    inventory.replace(extension.clone())?;
    Ok(inventory.extensions.remove(0))
}

fn integrity(
    graph: &GraphIndexArtifact,
    extension: &EvidenceExtension,
) -> Result<(), EvidenceError> {
    EvidenceInventory {
        schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        extensions: vec![extension.clone()],
    }
    .validate_integrity(graph)
}

fn supported(extension: &EvidenceExtension, id: &str, schema: &str) -> Result<(), EvidenceError> {
    if extension.descriptor.provider_id != id || extension.descriptor.schema_version != schema {
        return Err(EvidenceError::Unsupported {
            provider_id: extension.descriptor.provider_id.clone(),
            schema_version: extension.descriptor.schema_version.clone(),
        });
    }
    Ok(())
}

fn layout(
    name: &str,
    columns: Vec<EvidenceColumn>,
    primary_key: &[&str],
    order_by: &[&str],
    snapshot: bool,
) -> EvidenceTable {
    EvidenceTable {
        name: name.into(),
        storage_path: format!("{name}.parquet"),
        columns,
        scope: if snapshot {
            EvidenceTableScope::Snapshot
        } else {
            EvidenceTableScope::File {
                file_path_column: "file_path".into(),
                source_oid_column: "source_oid".into(),
                origin_hash_column: (name != "macro_invocations" && name != "macro_scopes")
                    .then(|| "origin_graph_content_hash".into()),
            }
        },
        primary_key: primary_key.iter().map(|s| (*s).into()).collect(),
        order_by: order_by.iter().map(|s| (*s).into()).collect(),
        metadata: Default::default(),
        rows: vec![],
    }
}

fn write_rows(table: &mut EvidenceTable, rows: &[Value]) -> anyhow::Result<()> {
    table.rows = rows
        .iter()
        .map(|row| {
            table
                .columns
                .iter()
                .map(|c| {
                    let value = row.get(&c.name).context("missing typed column")?;
                    Ok(match value {
                        Value::Null if c.nullable => EvidenceValue::Null,
                        Value::String(s) if c.data_type == EvidenceColumnType::Utf8 => {
                            EvidenceValue::Utf8(s.clone())
                        }
                        Value::Number(n) if c.data_type == EvidenceColumnType::Int64 => {
                            EvidenceValue::Int64(n.as_i64().context("coordinate exceeds Int64")?)
                        }
                        Value::Bool(b) if c.data_type == EvidenceColumnType::Boolean => {
                            EvidenceValue::Boolean(*b)
                        }
                        _ => anyhow::bail!("wrong typed column {}", c.name),
                    })
                })
                .collect()
        })
        .collect::<anyhow::Result<_>>()?;
    Ok(())
}

fn read_rows(
    extension: &EvidenceExtension,
    layouts: &[EvidenceTable],
) -> anyhow::Result<Vec<Vec<Value>>> {
    ensure!(
        extension.tables.len() == layouts.len(),
        "missing or extra provider table"
    );
    layouts
        .iter()
        .map(|expected| {
            let table = extension
                .tables
                .iter()
                .find(|t| t.name == expected.name)
                .context("missing provider table")?;
            ensure!(
                table.storage_path == expected.storage_path
                    && table.scope == expected.scope
                    && table.columns == expected.columns
                    && table.primary_key == expected.primary_key
                    && table.order_by == expected.order_by,
                "provider table schema/key/order mismatch: {}",
                table.name
            );
            table
                .rows
                .iter()
                .map(|row| {
                    ensure!(row.len() == table.columns.len(), "row width mismatch");
                    let mut object = serde_json::Map::new();
                    for (c, v) in table.columns.iter().zip(row) {
                        let value = match v {
                            EvidenceValue::Null => Value::Null,
                            EvidenceValue::Utf8(s) => Value::String(s.clone()),
                            EvidenceValue::Int64(n) => Value::from(*n),
                            EvidenceValue::Boolean(b) => Value::Bool(*b),
                        };
                        object.insert(c.name.clone(), value);
                    }
                    Ok(Value::Object(object))
                })
                .collect()
        })
        .collect()
}

fn availability(value: &EvidenceAvailability) -> crate::RustEvidenceAvailability {
    match value {
        EvidenceAvailability::Complete => crate::RustEvidenceAvailability::Complete,
        EvidenceAvailability::Partial => crate::RustEvidenceAvailability::Partial,
        EvidenceAvailability::Unavailable => crate::RustEvidenceAvailability::Unavailable,
        EvidenceAvailability::UnsupportedSource => {
            crate::RustEvidenceAvailability::UnsupportedSource
        }
    }
}

// Query results are already validated backend projections, not stored sets.
// Decode their rows without inventing graph owners or revalidating a subset as
// a complete snapshot. Backend forwarding/frozen validation is a later stage.
fn query_extension(result: &EvidenceQueryResult) -> Option<EvidenceExtension> {
    result
        .descriptor
        .as_ref()
        .map(|descriptor| EvidenceExtension {
            descriptor: descriptor.clone(),
            origins: result.origins.clone(),
            domain: result.domain.clone(),
            tables: result.tables.clone(),
        })
}
