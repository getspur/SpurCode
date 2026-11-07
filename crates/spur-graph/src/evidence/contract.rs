//! Provider-neutral evidence transport, integrity and identity. No filesystem IO.

use crate::{compute_graph_content_hash, GraphIndexArtifact};
use std::collections::{BTreeMap, BTreeSet};

pub const EVIDENCE_SCHEMA_VERSION: &str = "spur-evidence-v1";
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceInventory {
    pub schema_version: String,
    pub extensions: Vec<EvidenceExtension>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceDescriptor {
    pub provider_id: String,
    pub schema_version: String,
    pub extractor_version: String,
    pub component_versions: std::collections::BTreeMap<String, String>,
    pub presence: EvidencePresence,
    /// A legacy adapter declares its original wire format with `original_format`.
    /// This permits an unknown byte length, never an invented observation; the
    /// provider must authenticate that declaration before semantic validation.
    pub metadata: std::collections::BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidencePresence {
    Absent,
    Captured,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSourceKey {
    pub file_path: String,
    pub stable_file_id: String,
    pub source_oid: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceOriginKey {
    pub manifest_version: String,
    pub graph_content_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceOrigin {
    pub key: EvidenceOriginKey,
    pub sources: Vec<EvidenceSourceKey>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceCoverage {
    Complete,
    Unknown { code: String },
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSourceCoverage {
    pub source: EvidenceSourceKey,
    pub byte_length: Option<u64>,
    pub origin: Option<EvidenceOriginKey>,
    pub coverage: EvidenceCoverage,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceDomain {
    Unknown,
    Known {
        sources: Vec<EvidenceSourceCoverage>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceColumnType {
    Utf8,
    Int64,
    Boolean,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceColumn {
    pub name: String,
    pub data_type: EvidenceColumnType,
    pub nullable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceValue {
    Null,
    Utf8(String),
    Int64(i64),
    Boolean(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceTableScope {
    Snapshot,
    File {
        file_path_column: String,
        source_oid_column: String,
        origin_hash_column: Option<String>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceTable {
    pub name: String,
    pub storage_path: String,
    pub scope: EvidenceTableScope,
    pub columns: Vec<EvidenceColumn>,
    pub primary_key: Vec<String>,
    pub order_by: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
    pub rows: Vec<Vec<EvidenceValue>>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceExtension {
    pub descriptor: EvidenceDescriptor,
    pub origins: Vec<EvidenceOrigin>,
    pub domain: EvidenceDomain,
    pub tables: Vec<EvidenceTable>,
}
pub struct EvidenceSource<'a> {
    pub file_path: &'a str,
    pub bytes: &'a [u8],
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceError {
    Integrity {
        code: String,
        message: String,
    },
    Semantic {
        provider_id: String,
        code: String,
        message: String,
    },
    Unsupported {
        provider_id: String,
        schema_version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceSemanticStatus {
    Validated,
    UnknownProvider,
    UnsupportedSchema,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceValidation {
    pub status: EvidenceSemanticStatus,
    pub domain: EvidenceDomain,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceTableManifest {
    pub table: EvidenceTable, // rows MUST be empty; schema/order/metadata only
    pub row_count: u64,
    pub checksum_blake3: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceExtensionManifest {
    pub descriptor: EvidenceDescriptor,
    pub origins: Vec<EvidenceOrigin>,
    pub domain: EvidenceDomain,
    pub tables: Vec<EvidenceTableManifest>,
    pub checksum_blake3: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceManifest {
    pub schema_version: String,
    pub transport_complete: bool,
    pub extensions: Vec<EvidenceExtensionManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceFilter {
    pub provider_id: String,
    pub file_path: Option<String>,
    pub source_oid: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceAvailability {
    Complete,
    Partial,
    Unavailable,
    UnsupportedSource,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceQueryResult {
    pub availability: EvidenceAvailability,
    pub semantic_status: EvidenceSemanticStatus,
    pub descriptor: Option<EvidenceDescriptor>,
    pub domain: EvidenceDomain,
    pub origins: Vec<EvidenceOrigin>,
    pub tables: Vec<EvidenceTable>,
}

impl std::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Integrity { code, message } => {
                write!(f, "evidence integrity ({code}): {message}")
            }
            Self::Semantic {
                provider_id,
                code,
                message,
            } => write!(f, "evidence provider {provider_id} ({code}): {message}"),
            Self::Unsupported {
                provider_id,
                schema_version,
            } => write!(
                f,
                "unsupported evidence schema {schema_version} for {provider_id}"
            ),
        }
    }
}
impl std::error::Error for EvidenceError {}

fn invalid(code: &str, message: impl Into<String>) -> EvidenceError {
    EvidenceError::Integrity {
        code: code.into(),
        message: message.into(),
    }
}
fn require(condition: bool, code: &str, message: &str) -> Result<(), EvidenceError> {
    if condition {
        Ok(())
    } else {
        Err(invalid(code, message))
    }
}
fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

impl EvidenceFilter {
    pub fn validate_shape(&self) -> Result<(), EvidenceError> {
        require(
            nonempty(&self.provider_id),
            "filter_provider",
            "provider ID must be nonempty",
        )?;
        require(
            self.source_oid.is_none() || self.file_path.is_some(),
            "filter_source",
            "source OID requires a file path",
        )
    }
}

pub trait EvidenceProvider: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn supports_path(&self, file_path: &str) -> bool;
    fn source_domain(
        &self,
        graph: &GraphIndexArtifact,
    ) -> Result<Vec<EvidenceSourceKey>, EvidenceError>;
    fn validate_stored(
        &self,
        graph: &GraphIndexArtifact,
        extension: &EvidenceExtension,
    ) -> Result<EvidenceValidation, EvidenceError>;
    fn validate_sources(
        &self,
        graph: &GraphIndexArtifact,
        extension: &EvidenceExtension,
        sources: &[EvidenceSource<'_>],
    ) -> Result<(), EvidenceError>;
}

pub struct EvidenceRegistry<'a> {
    providers: BTreeMap<&'a str, &'a dyn EvidenceProvider>,
}
impl<'a> EvidenceRegistry<'a> {
    pub fn new(providers: &'a [&'a dyn EvidenceProvider]) -> Result<Self, EvidenceError> {
        let mut registered = BTreeMap::new();
        for provider in providers {
            let id = provider.provider_id();
            require(nonempty(id), "provider_id", "provider ID must be nonempty")?;
            require(
                registered.insert(id, *provider).is_none(),
                "duplicate_provider",
                "duplicate registry provider ID",
            )?;
        }
        Ok(Self {
            providers: registered,
        })
    }

    pub fn validate(
        &self,
        graph: &GraphIndexArtifact,
        extension: &EvidenceExtension,
    ) -> Result<EvidenceValidation, EvidenceError> {
        EvidenceInventory {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            extensions: vec![extension.clone()],
        }
        .validate_integrity(graph)?;
        let Some(provider) = self
            .providers
            .get(extension.descriptor.provider_id.as_str())
        else {
            return Ok(EvidenceValidation {
                status: EvidenceSemanticStatus::UnknownProvider,
                domain: EvidenceDomain::Unknown,
            });
        };
        match provider.validate_stored(graph, extension) {
            Err(EvidenceError::Unsupported {
                provider_id,
                schema_version,
            }) if provider_id == extension.descriptor.provider_id
                && schema_version == extension.descriptor.schema_version =>
            {
                Ok(EvidenceValidation {
                    status: EvidenceSemanticStatus::UnsupportedSchema,
                    domain: EvidenceDomain::Unknown,
                })
            }
            Ok(mut result) => {
                if result.status != EvidenceSemanticStatus::Validated {
                    result.domain = EvidenceDomain::Unknown;
                }
                Ok(result)
            }
            Err(error) => Err(error),
        }
    }
}

impl EvidenceInventory {
    pub fn empty() -> Self {
        Self {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            extensions: vec![],
        }
    }
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }
    pub fn get(&self, provider_id: &str) -> Option<&EvidenceExtension> {
        self.extensions
            .iter()
            .find(|e| e.descriptor.provider_id == provider_id)
    }
    pub fn remove(&mut self, provider_id: &str) -> Option<EvidenceExtension> {
        let index = self
            .extensions
            .iter()
            .position(|e| e.descriptor.provider_id == provider_id)?;
        Some(self.extensions.remove(index))
    }
    /// Atomically replace one provider after checking the entire candidate inventory.
    /// Graph binding and provider semantics remain separate validation steps.
    pub fn replace(&mut self, extension: EvidenceExtension) -> Result<(), EvidenceError> {
        // Do not silently repair an already malformed inventory (including duplicate IDs).
        canonical_inventory(self)?;
        let mut candidate = self.clone();
        candidate.remove(&extension.descriptor.provider_id);
        candidate.extensions.push(extension);
        *self = canonical_inventory(&candidate)?;
        Ok(())
    }
    pub fn validate_integrity(&self, graph: &GraphIndexArtifact) -> Result<(), EvidenceError> {
        let inventory = canonical_inventory(self)?;
        let mut paths = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for file in &graph.file_manifests {
            require(
                nonempty(&file.path)
                    && nonempty(&file.stable_file_id)
                    && nonempty(&file.content_oid)
                    && paths.insert(&file.path)
                    && ids.insert(&file.stable_file_id),
                "graph_source_duplicate",
                "duplicate graph source binding",
            )?;
        }
        require(
            compute_graph_content_hash(
                graph
                    .file_manifests
                    .iter()
                    .map(|f| (&f.path, &f.content_oid)),
            ) == graph.graph_content_hash,
            "graph_source_hash",
            "graph hash does not match current source inventory",
        )?;
        let check_current = |source: &EvidenceSourceKey| -> Result<(), EvidenceError> {
            require(
                graph.file_manifests.iter().any(|file| {
                    file.path == source.file_path
                        && file.stable_file_id == source.stable_file_id
                        && file.content_oid == source.source_oid
                }),
                "source_binding",
                "evidence source disagrees with current source manifest",
            )?;
            require(
                graph
                    .files
                    .iter()
                    .filter(|file| file.file_path == source.file_path)
                    .count()
                    == 1
                    && graph
                        .files
                        .iter()
                        .filter(|file| file.stable_file_id == source.stable_file_id)
                        .count()
                        == 1
                    && graph.files.iter().any(|file| {
                        file.file_path == source.file_path
                            && file.stable_file_id == source.stable_file_id
                    }),
                "source_owner",
                "evidence source has no matching graph file owner",
            )
        };
        for extension in &inventory.extensions {
            if let EvidenceDomain::Known { sources } = &extension.domain {
                for coverage in sources {
                    check_current(&coverage.source)?;
                }
            }
            for table in &extension.tables {
                if let EvidenceTableScope::File {
                    file_path_column,
                    source_oid_column,
                    origin_hash_column,
                } = &table.scope
                {
                    let path = column_index(table, file_path_column)?;
                    let oid = column_index(table, source_oid_column)?;
                    for row in &table.rows {
                        let file_path = utf8(&row[path])?;
                        let source_oid = utf8(&row[oid])?;
                        let current = graph
                            .file_manifests
                            .iter()
                            .find(|file| file.path == file_path && file.content_oid == source_oid)
                            .ok_or_else(|| {
                                invalid("row_source", "row disagrees with current source manifest")
                            })?;
                        let source = EvidenceSourceKey {
                            file_path: file_path.into(),
                            source_oid: source_oid.into(),
                            stable_file_id: current.stable_file_id.clone(),
                        };
                        check_current(&source)?;
                        let origin_hash = origin_hash_column
                            .as_ref()
                            .map(|name| column_index(table, name).and_then(|i| utf8(&row[i])))
                            .transpose()?;
                        require(
                            extension.origins.iter().any(|origin| {
                                origin_hash.is_none_or(|hash| hash == origin.key.graph_content_hash)
                                    && origin.sources.contains(&source)
                            }),
                            "row_source",
                            "row original source disagrees with current owner",
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn source_order(a: &EvidenceSourceKey, b: &EvidenceSourceKey) -> std::cmp::Ordering {
    (&a.file_path, &a.stable_file_id, &a.source_oid).cmp(&(
        &b.file_path,
        &b.stable_file_id,
        &b.source_oid,
    ))
}
fn origin_tuple(key: &EvidenceOriginKey) -> (&str, &str) {
    (&key.manifest_version, &key.graph_content_hash)
}
fn safe_path(path: &str) -> Result<(), EvidenceError> {
    require(
        !path.is_empty()
            && !path.contains(['\\', ':', '\0'])
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "unsafe_path",
        "expected a normal relative path",
    )
}
// These names belong to the current core writer, including legacy history files.
// Provider compatibility table basenames deliberately do not appear here.
const CORE_PATHS: &[&str] = &[
    "manifest.json",
    "nodes.parquet",
    "edges.parquet",
    "edges_by_dst.parquet",
    "edges_unresolved.parquet",
    "files.parquet",
    "file_manifests.parquet",
    "tombstones.parquet",
    "commits.parquet",
    "diagnostics.parquet",
    "symbol_snapshots",
    "temporal_edges",
    "symbol_snapshots.parquet",
    "temporal_edges.parquet",
    "history",
];
fn storage_path(path: &str) -> Result<(), EvidenceError> {
    safe_path(path)?;
    require(
        !CORE_PATHS.contains(&path.split('/').next().unwrap_or_default()),
        "reserved_path",
        "evidence table overlaps the graph core namespace",
    )
}
fn unique_sources(sources: &[EvidenceSourceKey]) -> Result<(), EvidenceError> {
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for source in sources {
        require(
            nonempty(&source.file_path)
                && nonempty(&source.stable_file_id)
                && nonempty(&source.source_oid),
            "source_key",
            "source stable ID and OID must be nonempty",
        )?;
        require(
            paths.insert(&source.file_path) && ids.insert(&source.stable_file_id),
            "duplicate_source",
            "duplicate source path or stable ID",
        )?;
    }
    Ok(())
}
fn canonical_inventory(inventory: &EvidenceInventory) -> Result<EvidenceInventory, EvidenceError> {
    require(
        inventory.schema_version == EVIDENCE_SCHEMA_VERSION,
        "inventory_schema",
        "unsupported generic evidence schema",
    )?;
    let mut inventory = inventory.clone();
    let mut providers = BTreeSet::new();
    let mut paths: BTreeSet<String> = BTreeSet::new();
    for extension in &mut inventory.extensions {
        require(
            providers.insert(extension.descriptor.provider_id.clone()),
            "duplicate_provider",
            "duplicate inventory provider ID",
        )?;
        *extension = canonical_extension(extension)?;
        for table in &extension.tables {
            require(
                paths.iter().all(|p| {
                    p != &table.storage_path
                        && !p.starts_with(&format!("{}/", table.storage_path))
                        && !table.storage_path.starts_with(&format!("{p}/"))
                }),
                "duplicate_path",
                "evidence storage paths overlap",
            )?;
            paths.insert(table.storage_path.clone());
        }
    }
    inventory
        .extensions
        .sort_by(|a, b| a.descriptor.provider_id.cmp(&b.descriptor.provider_id));
    Ok(inventory)
}
fn canonical_extension(extension: &EvidenceExtension) -> Result<EvidenceExtension, EvidenceError> {
    let mut extension = extension.clone();
    let descriptor = &extension.descriptor;
    require(
        nonempty(&descriptor.provider_id)
            && nonempty(&descriptor.schema_version)
            && nonempty(&descriptor.extractor_version),
        "descriptor",
        "provider and versions must be nonempty",
    )?;
    require(
        descriptor
            .component_versions
            .iter()
            .all(|(k, v)| nonempty(k) && nonempty(v)),
        "component_version",
        "component identities must be nonempty",
    )?;
    if descriptor.presence == EvidencePresence::Absent {
        require(
            extension.origins.is_empty()
                && extension.tables.is_empty()
                && extension.domain == EvidenceDomain::Unknown,
            "absent_payload",
            "absent evidence must have Unknown domain and no tables or origins",
        )?;
    }
    let mut origins = BTreeSet::new();
    for origin in &mut extension.origins {
        require(
            nonempty(&origin.key.manifest_version) && nonempty(&origin.key.graph_content_hash),
            "origin_key",
            "origin identities must be nonempty",
        )?;
        require(
            origins.insert((
                origin.key.manifest_version.clone(),
                origin.key.graph_content_hash.clone(),
            )),
            "duplicate_origin",
            "duplicate origin key",
        )?;
        unique_sources(&origin.sources)?;
        origin.sources.sort_by(source_order);
        require(
            compute_graph_content_hash(
                origin.sources.iter().map(|s| (&s.file_path, &s.source_oid)),
            ) == origin.key.graph_content_hash,
            "origin_hash",
            "origin source inventory hash mismatch",
        )?;
    }
    extension
        .origins
        .sort_by(|a, b| origin_tuple(&a.key).cmp(&origin_tuple(&b.key)));
    if let EvidenceDomain::Known { sources } = &mut extension.domain {
        unique_sources(&sources.iter().map(|s| s.source.clone()).collect::<Vec<_>>())?;
        for coverage in sources.iter() {
            require(
                coverage.byte_length.is_some()
                    || extension
                        .descriptor
                        .metadata
                        .get("original_format")
                        .is_some_and(|format| nonempty(format)),
                "source_length",
                "captured source needs byte length",
            )?;
            let key = coverage
                .origin
                .as_ref()
                .ok_or_else(|| invalid("source_origin", "captured source needs origin"))?;
            require(
                extension
                    .origins
                    .iter()
                    .any(|o| &o.key == key && o.sources.contains(&coverage.source)),
                "source_origin",
                "coverage does not belong to its declared origin",
            )?;
            if let EvidenceCoverage::Unknown { code } = &coverage.coverage {
                require(
                    nonempty(code),
                    "coverage_code",
                    "unknown coverage needs a reason",
                )?;
            }
        }
        sources.sort_by(|a, b| source_order(&a.source, &b.source));
    }
    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for table in &mut extension.tables {
        require(
            names.insert(table.name.clone()),
            "duplicate_table",
            "duplicate logical table name",
        )?;
        require(
            paths.iter().all(|p: &String| {
                p != &table.storage_path
                    && !p.starts_with(&format!("{}/", table.storage_path))
                    && !table.storage_path.starts_with(&format!("{p}/"))
            }),
            "duplicate_path",
            "evidence storage paths overlap",
        )?;
        paths.insert(table.storage_path.clone());
        *table = canonical_table(table)?;
        validate_file_rows(&extension.origins, &extension.domain, table)?;
    }
    extension.tables.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(extension)
}
fn column_index(table: &EvidenceTable, name: &str) -> Result<usize, EvidenceError> {
    table
        .columns
        .iter()
        .position(|c| c.name == name)
        .ok_or_else(|| {
            invalid(
                "column_reference",
                format!("unknown column {name} in {}", table.name),
            )
        })
}
fn utf8(value: &EvidenceValue) -> Result<&str, EvidenceError> {
    match value {
        EvidenceValue::Utf8(s) => Ok(s),
        _ => Err(invalid("scope_column", "expected nonnull Utf8 scope cell")),
    }
}
fn indices(table: &EvidenceTable, names: &[String]) -> Result<Vec<usize>, EvidenceError> {
    let mut seen = BTreeSet::new();
    names
        .iter()
        .map(|name| {
            require(
                seen.insert(name),
                "duplicate_column_reference",
                "duplicate key/order column",
            )?;
            column_index(table, name)
        })
        .collect()
}
fn canonical_table(table: &EvidenceTable) -> Result<EvidenceTable, EvidenceError> {
    let mut table = table.clone();
    require(
        nonempty(&table.name),
        "table_name",
        "table name must be nonempty",
    )?;
    storage_path(&table.storage_path)?;
    let mut columns = BTreeSet::new();
    require(
        !table.columns.is_empty(),
        "table_schema",
        "table needs columns",
    )?;
    for column in &table.columns {
        require(
            nonempty(&column.name) && columns.insert(&column.name),
            "duplicate_column",
            "column names must be nonempty and unique",
        )?;
    }
    require(
        !table.primary_key.is_empty(),
        "primary_key",
        "primary key must be nonempty",
    )?;
    let key = indices(&table, &table.primary_key)?;
    let order = indices(&table, &table.order_by)?;
    require(
        key.iter().all(|i| !table.columns[*i].nullable),
        "primary_key_nullable",
        "primary key columns must be nonnullable",
    )?;
    if let EvidenceTableScope::File {
        file_path_column,
        source_oid_column,
        origin_hash_column,
    } = &table.scope
    {
        let mut scope = vec![file_path_column.clone(), source_oid_column.clone()];
        scope.extend(origin_hash_column.iter().cloned());
        for i in indices(&table, &scope)? {
            require(
                table.columns[i].data_type == EvidenceColumnType::Utf8
                    && !table.columns[i].nullable,
                "scope_column",
                "file scope columns must be nonnullable Utf8",
            )?;
        }
    }
    let mut keys = BTreeSet::new();
    for row in &table.rows {
        require(
            row.len() == table.columns.len(),
            "row_width",
            "row width does not match schema",
        )?;
        for (cell, column) in row.iter().zip(&table.columns) {
            let valid = match cell {
                EvidenceValue::Null => column.nullable,
                EvidenceValue::Utf8(_) => column.data_type == EvidenceColumnType::Utf8,
                EvidenceValue::Int64(_) => column.data_type == EvidenceColumnType::Int64,
                EvidenceValue::Boolean(_) => column.data_type == EvidenceColumnType::Boolean,
            };
            require(
                valid,
                "cell_type",
                "wrong cell type or null in nonnullable column",
            )?;
        }
        require(
            keys.insert(key.iter().map(|i| &row[*i]).collect::<Vec<_>>()),
            "duplicate_primary_key",
            "duplicate row primary key",
        )?;
    }
    drop(keys);
    table.rows.sort_by(|a, b| {
        order
            .iter()
            .map(|i| &a[*i])
            .cmp(order.iter().map(|i| &b[*i]))
            .then_with(|| a.cmp(b))
    });
    Ok(table)
}
fn validate_file_rows(
    origins: &[EvidenceOrigin],
    domain: &EvidenceDomain,
    table: &EvidenceTable,
) -> Result<(), EvidenceError> {
    let EvidenceTableScope::File {
        file_path_column,
        source_oid_column,
        origin_hash_column,
    } = &table.scope
    else {
        return Ok(());
    };
    let path = column_index(table, file_path_column)?;
    let oid = column_index(table, source_oid_column)?;
    let hash = origin_hash_column
        .as_ref()
        .map(|c| column_index(table, c))
        .transpose()?;
    for row in &table.rows {
        let path = utf8(&row[path])?;
        let oid = utf8(&row[oid])?;
        let origin_hash = hash.map(|i| utf8(&row[i])).transpose()?;
        let matches_source = |s: &EvidenceSourceKey| s.file_path == path && s.source_oid == oid;
        require(
            origins.iter().any(|o| {
                origin_hash.is_none_or(|h| h == o.key.graph_content_hash)
                    && o.sources.iter().any(matches_source)
            }),
            "row_origin",
            "row source/origin is not in original inventory",
        )?;
        if let EvidenceDomain::Known { sources } = domain {
            require(
                sources.iter().any(|c| {
                    matches_source(&c.source)
                        && origin_hash.is_none_or(|h| {
                            c.origin.as_ref().is_some_and(|o| o.graph_content_hash == h)
                        })
                }),
                "row_domain",
                "row is outside declared source coverage",
            )?;
        }
    }
    Ok(())
}
fn checksum<T: serde::Serialize>(prefix: &[u8], value: &T) -> Result<String, EvidenceError> {
    let bytes = serde_json::to_vec(value).map_err(|e| invalid("serialization", e.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(prefix);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}
pub fn evidence_extension_checksum(extension: &EvidenceExtension) -> Result<String, EvidenceError> {
    checksum(
        b"spur-evidence-extension-v1\0",
        &canonical_extension(extension)?,
    )
}
/// Logical checksum of the complete canonical table, independent of compression.
pub fn evidence_table_checksum(table: &EvidenceTable) -> Result<String, EvidenceError> {
    checksum(b"spur-evidence-table-v1\0", &canonical_table(table)?)
}
pub fn evidence_inventory_manifest(
    inventory: &EvidenceInventory,
) -> Result<EvidenceManifest, EvidenceError> {
    let inventory = canonical_inventory(inventory)?;
    let extensions = inventory
        .extensions
        .iter()
        .map(|extension| {
            let tables = extension
                .tables
                .iter()
                .map(|table| {
                    let row_count = u64::try_from(table.rows.len())
                        .map_err(|_| invalid("row_count", "row count exceeds u64"))?;
                    let checksum_blake3 = evidence_table_checksum(table)?;
                    let mut table = table.clone();
                    table.rows.clear();
                    Ok(EvidenceTableManifest {
                        table,
                        row_count,
                        checksum_blake3,
                    })
                })
                .collect::<Result<_, EvidenceError>>()?;
            Ok(EvidenceExtensionManifest {
                descriptor: extension.descriptor.clone(),
                origins: extension.origins.clone(),
                domain: extension.domain.clone(),
                tables,
                checksum_blake3: if extension.descriptor.presence == EvidencePresence::Absent {
                    None
                } else {
                    Some(evidence_extension_checksum(extension)?)
                },
            })
        })
        .collect::<Result<_, EvidenceError>>()?;
    Ok(EvidenceManifest {
        schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        transport_complete: true,
        extensions,
    })
}
pub fn evidence_inventory_capability_digest(
    manifest_version: &str,
    inventory: &EvidenceInventory,
) -> Result<String, EvidenceError> {
    let manifest = evidence_inventory_manifest(inventory)?;
    checksum(
        b"",
        &(
            "spur-evidence-capability-v1",
            EVIDENCE_SCHEMA_VERSION,
            manifest_version,
            manifest.extensions,
        ),
    )
}
