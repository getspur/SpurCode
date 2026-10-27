use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context as _, Result};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::put_object::PutObjectError;
use aws_sdk_s3::primitives::ByteStream;
use duckdb::{params, Connection};
use sha2::{Digest, Sha256};

use crate::serving_registry::{
    ArtifactRef, ServingCatalogRow, ServingRegistry, ServingRegistryError,
};

pub(crate) const DUCKLAKE_DATA_PATH_ENV: &str = "SPUR_CONTEXT_DUCKLAKE_DATA_PATH";
pub(crate) const DUCKDB_EXTENSION_DIR_ENV: &str = "SPUR_CONTEXT_DUCKDB_EXTENSION_DIR";
const POSTGRES_DUCKLAKE_WRITE_LOCK_KEY: i64 = 7_830_668_896_113_191_951;
const CATALOG_PASSWORD_ENV: &str = "SPUR_CATALOG_PASSWORD";
// sol_33f7c9ded2f042c0: 2 attempts × 30s connect + 1s backoff = 61s, covering
// Aurora Serverless v2 resume-from-0 ACU without eating the 900s worker budget.
const POSTGRES_PAUSE_RESUME_CONNECT_TIMEOUT_SECS: u64 = 30;
const POSTGRES_PAUSE_RESUME_ATTEMPTS: u32 = 2;
const POSTGRES_PAUSE_RESUME_BACKOFF: Duration = Duration::from_secs(1);
const SNAPSHOT_POINTER_RELATIVE_PATH: &str = "gold/catalog-snapshot/current.json";
const SNAPSHOT_GENERATIONS_RELATIVE_DIR: &str = "gold/catalog-snapshot/generations";
const SNAPSHOT_FILE_NAME: &str = "spur_context.ducklake";
const SNAPSHOT_MANIFEST_FILE_NAME: &str = "manifest.json";
const SERVING_REGISTRY_FILE_NAME: &str = "serving-registry.json";
const SNAPSHOT_MANIFEST_SCHEMA_VERSION: u32 = 1;
const SNAPSHOT_PUBLISHED_STATUS: &str = "published";
const SNAPSHOT_INDEXES: &[(&str, &str, &str)] = &[
    (
        "ducklake_data_file_data_file_id",
        "ducklake_data_file",
        "data_file_id",
    ),
    (
        "ducklake_delete_file_delete_file_id",
        "ducklake_delete_file",
        "delete_file_id",
    ),
    ("ducklake_schema_schema_id", "ducklake_schema", "schema_id"),
    (
        "ducklake_snapshot_snapshot_id",
        "ducklake_snapshot",
        "snapshot_id",
    ),
    (
        "ducklake_snapshot_changes_snapshot_id",
        "ducklake_snapshot_changes",
        "snapshot_id",
    ),
];

pub(crate) fn is_remote_catalog(catalog_dsn: &str) -> bool {
    catalog_dsn.starts_with("s3://")
        || catalog_dsn.starts_with("https://")
        || catalog_dsn.starts_with("http://")
}

pub(crate) fn duckdb_extension_load_sql(extension: &str) -> String {
    debug_assert!(extension
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'));
    match duckdb_extension_dir() {
        Some(dir) => format!(
            "SET extension_directory = '{}'; \
             SET autoinstall_known_extensions = false; \
             LOAD {extension};",
            escape_sql_literal(&dir)
        ),
        None => format!("INSTALL {extension}; LOAD {extension};"),
    }
}

pub(crate) fn load_duckdb_extension(
    conn: &Connection,
    extension: &str,
    context: &'static str,
) -> Result<()> {
    conn.execute_batch(&duckdb_extension_load_sql(extension))
        .context(context)
}

fn load_duckdb_extensions(
    conn: &Connection,
    extensions: &[&str],
    context: &'static str,
) -> Result<()> {
    let sql = extensions
        .iter()
        .map(|extension| duckdb_extension_load_sql(extension))
        .collect::<Vec<_>>()
        .join(" ");
    conn.execute_batch(&sql).context(context)
}

fn duckdb_extension_dir() -> Option<String> {
    std::env::var(DUCKDB_EXTENSION_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[derive(Debug)]
pub struct CatalogResolver {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedRevision {
    pub source: String,
    pub package: String,
    pub revision: String,
    pub revision_kind: String,
    pub snapshot_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RevisionInfo {
    pub source: String,
    pub package: String,
    pub revision: String,
    pub revision_kind: String,
    pub semver_major: Option<i32>,
    pub semver_minor: Option<i32>,
    pub semver_patch: Option<i32>,
    pub snapshot_id: i64,
    pub index_status: String,
    pub embeddings_status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotCleanupOptions {
    pub older_than: Duration,
    pub republish_lag: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrozenSnapshotManifest {
    pub schema_version: u32,
    pub generation: i64,
    pub snapshot_uri: String,
    pub data_path: String,
    pub sha256: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_registry_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_registry_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_registry_bytes: Option<u64>,
    pub status: String,
}

impl FrozenSnapshotManifest {
    pub fn published(
        generation: i64,
        snapshot_uri: String,
        data_path: String,
        sha256: String,
        bytes: u64,
    ) -> Self {
        Self {
            schema_version: SNAPSHOT_MANIFEST_SCHEMA_VERSION,
            generation,
            snapshot_uri,
            data_path,
            sha256,
            bytes,
            serving_registry_uri: None,
            serving_registry_sha256: None,
            serving_registry_bytes: None,
            status: SNAPSHOT_PUBLISHED_STATUS.to_owned(),
        }
    }

    fn published_with_registry(
        generation: i64,
        snapshot: ArtifactRef,
        data_path: String,
        registry: ArtifactRef,
    ) -> Self {
        Self {
            schema_version: SNAPSHOT_MANIFEST_SCHEMA_VERSION,
            generation,
            snapshot_uri: snapshot.uri,
            data_path,
            sha256: snapshot.sha256,
            bytes: snapshot.bytes,
            serving_registry_uri: Some(registry.uri),
            serving_registry_sha256: Some(registry.sha256),
            serving_registry_bytes: Some(registry.bytes),
            status: SNAPSHOT_PUBLISHED_STATUS.to_owned(),
        }
    }

    pub fn ensure_published(&self) -> Result<()> {
        if self.schema_version != SNAPSHOT_MANIFEST_SCHEMA_VERSION {
            bail!(
                "unsupported frozen snapshot manifest schema version {}",
                self.schema_version
            );
        }
        if self.status != SNAPSHOT_PUBLISHED_STATUS {
            bail!(
                "frozen snapshot manifest for generation {} is not published: {}",
                self.generation,
                self.status
            );
        }
        if self.generation <= 0 {
            bail!("frozen snapshot manifest generation must be positive");
        }
        if self.snapshot_uri.trim().is_empty() {
            bail!("frozen snapshot manifest snapshot_uri must be non-empty");
        }
        if self.data_path.trim().is_empty() {
            bail!("frozen snapshot manifest data_path must be non-empty");
        }
        if self.sha256.trim().is_empty() {
            bail!("frozen snapshot manifest sha256 must be non-empty");
        }
        Ok(())
    }

    fn ensure_serving_published(&self) -> Result<()> {
        self.ensure_published()?;
        validate_strong_ref("snapshot", &self.snapshot_uri, &self.sha256, self.bytes)?;
        validate_strong_ref(
            "serving_registry",
            self.serving_registry_uri
                .as_deref()
                .ok_or_else(|| anyhow!("published pointer is missing serving_registry_uri"))?,
            self.serving_registry_sha256
                .as_deref()
                .ok_or_else(|| anyhow!("published pointer is missing serving_registry_sha256"))?,
            self.serving_registry_bytes
                .ok_or_else(|| anyhow!("published pointer is missing serving_registry_bytes"))?,
        )
    }

    fn same_serving_identity(&self, other: &Self) -> bool {
        self == other
    }

    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let manifest: Self =
            serde_json::from_slice(bytes).context("failed to parse frozen snapshot manifest")?;
        manifest.ensure_published()?;
        Ok(manifest)
    }

    fn to_json_bytes(&self) -> Result<Vec<u8>> {
        self.ensure_published()?;
        let mut bytes =
            serde_json::to_vec_pretty(self).context("failed to encode frozen snapshot manifest")?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationPublication {
    pub generation: i64,
    pub data_path: String,
    pub snapshot_uri: String,
    pub snapshot_bytes: Vec<u8>,
    pub snapshot_manifest_uri: String,
    pub serving_registry_uri: String,
    pub pointer_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationObject {
    pub bytes: Vec<u8>,
    pub etag: String,
    pub version_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerPrecondition {
    Absent,
    Matches { etag: String },
}

#[derive(Debug, thiserror::Error)]
pub enum PublicationStoreError {
    #[error("conditional object-write precondition failed")]
    PreconditionFailed,
    #[error("publication storage operation failed")]
    Storage,
}

impl PublicationStoreError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PreconditionFailed => "conditional_precondition_failed",
            Self::Storage => "storage_error",
        }
    }
}

pub trait PublicationStore {
    fn read_object(
        &self,
        uri: &str,
    ) -> std::result::Result<Option<PublicationObject>, PublicationStoreError>;

    fn put_immutable_object(
        &self,
        uri: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> std::result::Result<(), PublicationStoreError>;

    fn compare_and_swap_pointer(
        &self,
        uri: &str,
        bytes: &[u8],
        content_type: &str,
        precondition: &PointerPrecondition,
    ) -> std::result::Result<(), PublicationStoreError>;
}

#[derive(Debug, thiserror::Error)]
pub enum GenerationPublicationError {
    #[error("serving registry validation failed ({code})")]
    Registry { code: &'static str },
    #[error("catalog publication query failed")]
    Catalog,
    #[error("invalid generation publication request")]
    InvalidPublication,
    #[error("publication generation {requested} does not match catalog generation {current}")]
    CatalogGenerationMismatch { requested: i64, current: i64 },
    #[error("required {kind} publication artifact is missing")]
    MissingArtifact { kind: &'static str },
    #[error("{kind} publication artifact SHA-256 mismatch")]
    ArtifactHashMismatch { kind: &'static str },
    #[error("{kind} publication artifact byte-length mismatch")]
    ArtifactByteMismatch { kind: &'static str },
    #[error("cannot publish stale generation {requested}; live pointer is generation {current}")]
    StaleGeneration { requested: i64, current: i64 },
    #[error("live pointer already contains a conflicting publication for generation {generation}")]
    SameGenerationConflict { generation: i64 },
    #[error("live pointer changed after it was observed; retry from the new pointer")]
    StalePointer,
    #[error("live pointer is invalid")]
    InvalidPointer,
    #[error("required immutable {kind} publication output is missing")]
    MissingImmutableOutput { kind: &'static str },
    #[error("immutable {kind} publication output does not match the live pointer")]
    ImmutableOutputMismatch { kind: &'static str },
    #[error(transparent)]
    Store(#[from] PublicationStoreError),
}

impl From<ServingRegistryError> for GenerationPublicationError {
    fn from(error: ServingRegistryError) -> Self {
        Self::Registry { code: error.code() }
    }
}

impl GenerationPublicationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Registry { code } => code,
            Self::Catalog => "catalog_error",
            Self::InvalidPublication => "invalid_publication",
            Self::CatalogGenerationMismatch { .. } => "catalog_generation_mismatch",
            Self::MissingArtifact { kind, .. } if *kind == "source_sidecar" => {
                "missing_source_sidecar"
            }
            Self::MissingArtifact { kind, .. } if *kind == "graph_manifest" => {
                "missing_graph_manifest"
            }
            Self::MissingArtifact { .. } => "missing_artifact",
            Self::ArtifactHashMismatch { .. } => "artifact_hash_mismatch",
            Self::ArtifactByteMismatch { .. } => "artifact_byte_mismatch",
            Self::StaleGeneration { .. } => "stale_generation",
            Self::SameGenerationConflict { .. } => "same_generation_conflict",
            Self::StalePointer => "stale_pointer",
            Self::InvalidPointer => "invalid_pointer",
            Self::MissingImmutableOutput { .. } => "missing_immutable_output",
            Self::ImmutableOutputMismatch { .. } => "immutable_output_mismatch",
            Self::Store(PublicationStoreError::PreconditionFailed) => "stale_pointer",
            Self::Store(PublicationStoreError::Storage) => "storage_error",
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct S3PublicationStore;

impl CatalogResolver {
    pub fn new(catalog_dsn: &str) -> Result<Self> {
        let data_path = ducklake_data_path(catalog_dsn)?;
        Self::new_with_data_path(catalog_dsn, &data_path)
    }

    pub fn new_with_data_path(catalog_dsn: &str, data_path: &str) -> Result<Self> {
        Ok(Self::from_connection(connect_ducklake_with_data_path(
            catalog_dsn,
            data_path,
        )?))
    }

    pub fn from_connection(conn: Connection) -> Self {
        Self { conn }
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn resolve(
        &self,
        source: &str,
        package: &str,
        revision_or_ref: &str,
    ) -> Result<ResolvedRevision> {
        if let Some(revision) = self.lookup_ref_revision(source, package, revision_or_ref)? {
            return self.lookup_revision(source, package, &revision)?.ok_or_else(|| {
                anyhow!(
                    "revision not found after resolving ref: {source}/{package}@{revision_or_ref} -> {revision}"
                )
            });
        }

        self.lookup_revision(source, package, revision_or_ref)?
            .ok_or_else(|| anyhow!("revision not found: {source}/{package}@{revision_or_ref}"))
    }

    pub fn resolve_latest(&self, source: &str, package: &str) -> Result<ResolvedRevision> {
        self.resolve(source, package, "latest")
    }

    pub fn list_revisions(&self, source: &str, package: &str) -> Result<Vec<RevisionInfo>> {
        let package_catalog = readable_table(&self.conn, "package_catalog")?;
        let sql = format!(
            r"
            SELECT
                source,
                package,
                revision,
                revision_kind,
                semver_major,
                semver_minor,
                semver_patch,
                snapshot_id,
                index_status,
                embeddings_status
            FROM {package_catalog}
            WHERE source = ? AND package = ?
            ORDER BY
                CASE WHEN revision_kind = 'semver' THEN 0 ELSE 1 END,
                semver_major DESC NULLS LAST,
                semver_minor DESC NULLS LAST,
                semver_patch DESC NULLS LAST,
                indexed_at DESC NULLS LAST,
                revision DESC
            "
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("failed to prepare revision list query")?;

        stmt.query_map(params![source, package], revision_info_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list revisions")
    }

    fn lookup_ref_revision(
        &self,
        source: &str,
        package: &str,
        ref_name: &str,
    ) -> Result<Option<String>> {
        let refs = readable_table(&self.conn, "refs")?;
        let sql = format!(
            r"
            SELECT revision
            FROM {refs}
            WHERE source = ? AND package = ? AND ref_name = ?
            LIMIT 1
            "
        );
        optional_no_rows(
            self.conn
                .query_row(&sql, params![source, package, ref_name], |row| row.get(0)),
            "failed to resolve catalog ref",
        )
    }

    fn lookup_revision(
        &self,
        source: &str,
        package: &str,
        revision: &str,
    ) -> Result<Option<ResolvedRevision>> {
        let package_catalog = readable_table(&self.conn, "package_catalog")?;
        let sql = format!(
            r"
            SELECT source, package, revision, revision_kind, snapshot_id
            FROM {package_catalog}
            WHERE source = ? AND package = ? AND revision = ?
            LIMIT 1
            "
        );
        optional_no_rows(
            self.conn
                .query_row(&sql, params![source, package, revision], |row| {
                    Ok(ResolvedRevision {
                        source: row.get(0)?,
                        package: row.get(1)?,
                        revision: row.get(2)?,
                        revision_kind: row.get(3)?,
                        snapshot_id: row.get(4)?,
                    })
                }),
            "failed to resolve catalog revision",
        )
    }
}

pub fn connect_ducklake(catalog_dsn: &str) -> Result<Connection> {
    let data_path = ducklake_data_path(catalog_dsn)?;
    connect_ducklake_with_data_path(catalog_dsn, &data_path)
}

pub fn connect_frozen_snapshot(snapshot_path: &Path, data_path: &str) -> Result<Connection> {
    let conn = Connection::open_in_memory().context("failed to open in-memory DuckDB")?;
    load_ducklake_extensions(&conn, &snapshot_path.display().to_string())?;

    if data_path.starts_with("s3://") {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        load_duckdb_extension(
            &conn,
            "httpfs",
            "failed to load httpfs for frozen snapshot S3 data path",
        )?;
        conn.execute_batch(&format!(
            "CREATE OR REPLACE SECRET s3_creds (TYPE s3, PROVIDER credential_chain, REGION '{region}');",
        ))
        .context("failed to configure S3 credentials for frozen snapshot data path")?;
    }

    attach_frozen_snapshot(&conn, snapshot_path, data_path)?;
    Ok(conn)
}

pub fn connect_ducklake_with_data_path(catalog_dsn: &str, data_path: &str) -> Result<Connection> {
    connect_ducklake_with_data_path_inner(catalog_dsn, data_path, false)
}

pub(crate) fn connect_ducklake_with_data_path_serialized(
    catalog_dsn: &str,
    data_path: &str,
) -> Result<Connection> {
    connect_ducklake_with_data_path_inner(catalog_dsn, data_path, true)
}

fn connect_ducklake_with_data_path_inner(
    catalog_dsn: &str,
    data_path: &str,
    serialize_postgres_writes: bool,
) -> Result<Connection> {
    let catalog_dsn = catalog_dsn_with_env_password(catalog_dsn);
    let conn = Connection::open_in_memory().context("failed to open in-memory DuckDB")?;
    load_ducklake_extensions(&conn, &catalog_dsn)?;

    if data_path.starts_with("s3://") && !is_remote_catalog(&catalog_dsn) {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        load_duckdb_extension(&conn, "httpfs", "failed to load httpfs for S3 data path")?;
        conn.execute_batch(&format!(
            "CREATE OR REPLACE SECRET s3_creds (TYPE s3, PROVIDER credential_chain, REGION '{region}');",
        ))
            .context("failed to configure S3 credentials for DuckLake data path")?;
    }

    if is_remote_catalog(&catalog_dsn) {
        conn.execute_batch("SET unsafe_disable_etag_checks = true;")
            .context("failed to disable etag checks for remote catalog")?;
    }

    if serialize_postgres_writes && is_postgres_catalog(&catalog_dsn) {
        acquire_postgres_ducklake_write_lock(&conn, &catalog_dsn)?;
    }

    attach_ducklake(&conn, &catalog_dsn, data_path)?;

    Ok(conn)
}

fn acquire_postgres_ducklake_write_lock(conn: &Connection, catalog_dsn: &str) -> Result<()> {
    let alias = format!("spur_catalog_lock_{}", uuid::Uuid::new_v4().simple());
    let dsn = postgres_metadata_dsn(catalog_dsn);
    attach_postgres_alias(conn, &alias, &dsn)
        .context("failed to attach Postgres catalog for DuckLake write lock")?;
    conn.query_row(&postgres_ducklake_write_lock_sql(&alias), [], |_| Ok(()))
        .context("failed to acquire Postgres DuckLake write lock")
}

pub(crate) fn attach_postgres_alias(conn: &Connection, alias: &str, dsn: &str) -> Result<()> {
    retry_postgres_pause_resume(
        || {
            let _ = conn.execute_batch(&format!("DETACH DATABASE IF EXISTS {alias};"));
            conn.execute_batch(&format!(
                "ATTACH '{}' AS {alias} (TYPE postgres);",
                escape_sql_literal(dsn)
            ))
            .map_err(|error| anyhow!("{}", redact_libpq_secrets(&error.to_string())))
            .map(|_| ())
        },
        thread::sleep,
    )
}

pub(crate) fn postgres_ducklake_write_lock_sql(alias: &str) -> String {
    format!(
        "SELECT locked FROM postgres_query('{alias}', 'SELECT TRUE AS locked FROM pg_advisory_lock({POSTGRES_DUCKLAKE_WRITE_LOCK_KEY})')"
    )
}

pub fn compact_gold_and_export_snapshot(
    catalog_dsn: &str,
    data_path: &str,
    options: SnapshotCleanupOptions,
) -> Result<PathBuf> {
    if options.older_than < options.republish_lag {
        bail!("cleanup older_than must be >= republish_lag");
    }

    let conn = connect_ducklake_with_data_path(catalog_dsn, data_path)?;
    run_optional_maintenance_call(&conn, "CALL ducklake_merge_adjacent_files('spur_context')")?;
    let older_than = interval_literal(options.older_than);
    run_optional_maintenance_call(
        &conn,
        &format!(
            "CALL ducklake_expire_snapshots('spur_context', older_than => CAST(now() AS TIMESTAMP) - INTERVAL '{older_than}')"
        ),
    )?;
    run_optional_maintenance_call(
        &conn,
        &format!(
            "CALL ducklake_cleanup_old_files('spur_context', older_than => CAST(now() AS TIMESTAMP) - INTERVAL '{older_than}')"
        ),
    )?;
    conn.execute("FORCE CHECKPOINT", [])
        .context("failed to checkpoint DuckLake before snapshot export")?;

    let generation = current_snapshot_generation(&conn)?;
    export_frozen_snapshot(catalog_dsn, data_path, generation)
}

pub(crate) fn export_frozen_snapshot(
    catalog_dsn: &str,
    data_path: &str,
    generation: i64,
) -> Result<PathBuf> {
    let location = SnapshotLocation::for_data_path_and_generation(data_path, generation)?;
    if let Some(parent) = location.local_staging_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create snapshot dir `{}`", parent.display()))?;
    }
    if location.s3_uri.is_none()
        && (location.local_path.exists() || location.local_manifest_path.exists())
    {
        if location.local_path.is_file() && location.local_manifest_path.is_file() {
            let manifest = read_snapshot_manifest(&location)?;
            if manifest.generation != generation || manifest.snapshot_uri != location.snapshot_uri {
                bail!(
                    "existing frozen snapshot manifest does not match generation {generation}: `{}`",
                    location.local_manifest_path.display()
                );
            }
            publish_snapshot_pointer_if_current_not_newer(&location, &manifest)?;
            return Ok(location.local_path);
        }
        bail!(
            "partial immutable frozen snapshot exists for generation {generation}: snapshot={}, manifest={}",
            location.local_path.display(),
            location.local_manifest_path.display()
        );
    }
    if location.local_staging_path.exists() {
        fs::remove_file(&location.local_staging_path).with_context(|| {
            format!(
                "failed to remove stale snapshot staging file `{}`",
                location.local_staging_path.display()
            )
        })?;
    }

    copy_ducklake_metadata_tables(catalog_dsn, &location.local_staging_path)?;
    replay_snapshot_indexes(&location.local_staging_path)?;
    validate_snapshot_attaches(&location.local_staging_path, data_path)?;
    verify_snapshot_referenced_files_exist(&location.local_staging_path, data_path)?;

    if location.s3_uri.is_some() {
        let snapshot_bytes = fs::read(&location.local_staging_path).with_context(|| {
            format!(
                "failed to read frozen snapshot `{}`",
                location.local_staging_path.display()
            )
        })?;
        let frozen = connect_frozen_snapshot(&location.local_staging_path, data_path)?;
        publish_generation(
            &S3PublicationStore,
            &frozen,
            GenerationPublication {
                generation,
                data_path: data_path.to_owned(),
                snapshot_uri: location.snapshot_uri.clone(),
                snapshot_bytes,
                snapshot_manifest_uri: location
                    .s3_manifest_uri
                    .clone()
                    .ok_or_else(|| anyhow!("S3 snapshot manifest URI is missing"))?,
                serving_registry_uri: location
                    .s3_serving_registry_uri
                    .clone()
                    .ok_or_else(|| anyhow!("S3 serving registry URI is missing"))?,
                pointer_uri: location
                    .s3_pointer_uri
                    .clone()
                    .ok_or_else(|| anyhow!("S3 live pointer URI is missing"))?,
            },
        )?;
        return Ok(location.local_path);
    }

    let (sha256, bytes) = file_sha256_and_len(&location.local_staging_path)?;
    let manifest = FrozenSnapshotManifest::published(
        generation,
        location.snapshot_uri.clone(),
        data_path.to_owned(),
        sha256,
        bytes,
    );

    publish_snapshot_file(&location)?;
    publish_snapshot_manifest(&location, &manifest)?;
    publish_snapshot_pointer_if_current_not_newer(&location, &manifest)?;

    Ok(location.local_path)
}

pub fn publish_generation<S: PublicationStore>(
    store: &S,
    catalog: &Connection,
    publication: GenerationPublication,
) -> std::result::Result<FrozenSnapshotManifest, GenerationPublicationError> {
    validate_publication_request(&publication)?;

    let registry = serving_registry_for_generation(catalog, publication.generation)?;
    let registry_bytes = registry.to_canonical_json()?;
    validate_registry_artifacts(store, &registry)?;

    let snapshot_ref = ArtifactRef {
        uri: publication.snapshot_uri.clone(),
        sha256: sha256_bytes(&publication.snapshot_bytes),
        bytes: publication.snapshot_bytes.len() as u64,
    };
    let registry_ref = ArtifactRef {
        uri: publication.serving_registry_uri.clone(),
        sha256: sha256_bytes(&registry_bytes),
        bytes: registry_bytes.len() as u64,
    };
    let manifest = FrozenSnapshotManifest::published_with_registry(
        publication.generation,
        snapshot_ref,
        publication.data_path.clone(),
        registry_ref,
    );
    manifest
        .ensure_serving_published()
        .map_err(|_| GenerationPublicationError::InvalidPublication)?;
    let manifest_bytes = manifest
        .to_json_bytes()
        .map_err(|_| GenerationPublicationError::InvalidPublication)?;

    let precondition = match observe_live_pointer(store, &publication.pointer_uri, &manifest)? {
        PointerDecision::AlreadyPublished(live_pointer) => {
            verify_already_published_outputs(
                store,
                &publication,
                &manifest,
                &manifest_bytes,
                &registry_bytes,
                &live_pointer,
            )?;
            return Ok(manifest);
        }
        PointerDecision::Publish(precondition) => precondition,
    };

    put_immutable_exact(
        store,
        &publication.snapshot_uri,
        &publication.snapshot_bytes,
        "application/octet-stream",
    )?;
    put_immutable_exact(
        store,
        &publication.snapshot_manifest_uri,
        &manifest_bytes,
        "application/json",
    )?;
    put_immutable_exact(
        store,
        &publication.serving_registry_uri,
        &registry_bytes,
        "application/json",
    )?;
    match store.compare_and_swap_pointer(
        &publication.pointer_uri,
        &manifest_bytes,
        "application/json",
        &precondition,
    ) {
        Ok(()) => Ok(manifest),
        Err(PublicationStoreError::PreconditionFailed) => {
            Err(GenerationPublicationError::StalePointer)
        }
        Err(error) => Err(GenerationPublicationError::Store(error)),
    }
}

fn validate_publication_request(
    publication: &GenerationPublication,
) -> std::result::Result<(), GenerationPublicationError> {
    if publication.generation <= 0 {
        return Err(GenerationPublicationError::InvalidPublication);
    }
    if publication.snapshot_bytes.is_empty() {
        return Err(GenerationPublicationError::InvalidPublication);
    }
    for (name, uri) in [
        ("data_path", publication.data_path.as_str()),
        ("snapshot_uri", publication.snapshot_uri.as_str()),
        (
            "snapshot_manifest_uri",
            publication.snapshot_manifest_uri.as_str(),
        ),
        (
            "serving_registry_uri",
            publication.serving_registry_uri.as_str(),
        ),
        ("pointer_uri", publication.pointer_uri.as_str()),
    ] {
        let _ = name;
        parse_s3_uri(uri).map_err(|_| GenerationPublicationError::InvalidPublication)?;
    }
    let generation_segment = format!("/{:020}/", publication.generation);
    for (name, uri) in [
        ("snapshot_uri", publication.snapshot_uri.as_str()),
        (
            "snapshot_manifest_uri",
            publication.snapshot_manifest_uri.as_str(),
        ),
        (
            "serving_registry_uri",
            publication.serving_registry_uri.as_str(),
        ),
    ] {
        if !uri.contains(&generation_segment) {
            let _ = name;
            return Err(GenerationPublicationError::InvalidPublication);
        }
    }
    Ok(())
}

fn serving_registry_for_generation(
    catalog: &Connection,
    generation: i64,
) -> std::result::Result<ServingRegistry, GenerationPublicationError> {
    let table = readable_table(catalog, "package_catalog")
        .map_err(|_| GenerationPublicationError::Catalog)?;
    let max_sql = format!("SELECT MAX(generation)::BIGINT FROM {table}");
    let current_generation = catalog
        .query_row(&max_sql, [], |row| row.get::<_, Option<i64>>(0))
        .map_err(|_| GenerationPublicationError::Catalog)?
        .filter(|current| *current > 0)
        .ok_or(GenerationPublicationError::Catalog)?;
    if generation != current_generation {
        return Err(GenerationPublicationError::CatalogGenerationMismatch {
            requested: generation,
            current: current_generation,
        });
    }
    let sql = format!(
        r"
        SELECT
            source,
            package,
            revision,
            revision_kind,
            generation,
            index_status,
            graph_manifest_uri,
            graph_manifest_sha256,
            graph_manifest_bytes,
            source_sidecar_uri,
            source_sidecar_sha256,
            source_sidecar_bytes
        FROM {table}
        WHERE NOT (
            graph_manifest_uri IS NULL
            AND graph_manifest_sha256 IS NULL
            AND graph_manifest_bytes IS NULL
            AND source_sidecar_uri IS NULL
            AND source_sidecar_sha256 IS NULL
            AND source_sidecar_bytes IS NULL
        )
        ORDER BY source, package, revision
        "
    );
    let mut stmt = catalog
        .prepare(&sql)
        .map_err(|_| GenerationPublicationError::Catalog)?;
    let mut rows = stmt
        .query_map([], |row| {
            let graph_manifest_bytes = row
                .get::<_, Option<i64>>(8)?
                .and_then(|bytes| u64::try_from(bytes).ok());
            let source_sidecar_bytes = row
                .get::<_, Option<i64>>(11)?
                .and_then(|bytes| u64::try_from(bytes).ok());
            Ok(ServingCatalogRow {
                source: row.get(0)?,
                package: row.get(1)?,
                revision: row.get(2)?,
                revision_kind: row.get(3)?,
                refs: Vec::new(),
                generation: row.get::<_, Option<i64>>(4)?,
                index_status: row.get(5)?,
                graph_manifest_uri: row.get(6)?,
                graph_manifest_sha256: row.get(7)?,
                graph_manifest_bytes,
                source_sidecar_uri: row.get(9)?,
                source_sidecar_sha256: row.get(10)?,
                source_sidecar_bytes,
            })
        })
        .map_err(|_| GenerationPublicationError::Catalog)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| GenerationPublicationError::Catalog)?;

    let refs_table =
        readable_table(catalog, "refs").map_err(|_| GenerationPublicationError::Catalog)?;
    let refs_sql = format!(
        r"
        SELECT source, package, revision, ref_name
        FROM {refs_table}
        ORDER BY source, package, revision, ref_name
        "
    );
    let mut refs_stmt = catalog
        .prepare(&refs_sql)
        .map_err(|_| GenerationPublicationError::Catalog)?;
    let refs = refs_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| GenerationPublicationError::Catalog)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| GenerationPublicationError::Catalog)?;
    let identities_sql = format!("SELECT source, package, revision FROM {table}");
    let mut identities_stmt = catalog
        .prepare(&identities_sql)
        .map_err(|_| GenerationPublicationError::Catalog)?;
    let identities = identities_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| GenerationPublicationError::Catalog)?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| GenerationPublicationError::Catalog)?;
    let mut refs_by_revision = BTreeMap::<(String, String, String), Vec<String>>::new();
    for (source, package, revision, reference) in refs {
        if !identities.contains(&(source.clone(), package.clone(), revision.clone())) {
            return Err(ServingRegistryError::IncompleteServingGeneration {
                generation,
                reason: "a package ref targets an absent package revision".to_owned(),
            }
            .into());
        }
        refs_by_revision
            .entry((source, package, revision))
            .or_default()
            .push(reference);
    }
    for row in &mut rows {
        row.refs = refs_by_revision
            .remove(&(
                row.source.clone(),
                row.package.clone(),
                row.revision.clone(),
            ))
            .unwrap_or_default();
    }
    ServingRegistry::from_current_rows(generation, rows).map_err(Into::into)
}

fn validate_registry_artifacts<S: PublicationStore>(
    store: &S,
    registry: &ServingRegistry,
) -> std::result::Result<(), GenerationPublicationError> {
    registry.validate()?;
    for package in &registry.packages {
        validate_publication_artifact(store, "graph_manifest", &package.graph_manifest)?;
        validate_publication_artifact(store, "source_sidecar", &package.source_sidecar)?;
    }
    Ok(())
}

fn validate_publication_artifact<S: PublicationStore>(
    store: &S,
    kind: &'static str,
    artifact: &ArtifactRef,
) -> std::result::Result<(), GenerationPublicationError> {
    let object = store
        .read_object(&artifact.uri)?
        .ok_or_else(|| GenerationPublicationError::MissingArtifact { kind })?;
    let actual_bytes = object.bytes.len() as u64;
    if actual_bytes != artifact.bytes {
        return Err(GenerationPublicationError::ArtifactByteMismatch { kind });
    }
    let actual_hash = sha256_bytes(&object.bytes);
    if actual_hash != artifact.sha256 {
        return Err(GenerationPublicationError::ArtifactHashMismatch { kind });
    }
    Ok(())
}

enum PointerDecision {
    AlreadyPublished(PublicationObject),
    Publish(PointerPrecondition),
}

fn observe_live_pointer<S: PublicationStore>(
    store: &S,
    pointer_uri: &str,
    manifest: &FrozenSnapshotManifest,
) -> std::result::Result<PointerDecision, GenerationPublicationError> {
    let Some(object) = store.read_object(pointer_uri)? else {
        return Ok(PointerDecision::Publish(PointerPrecondition::Absent));
    };
    let current = FrozenSnapshotManifest::from_json_slice(&object.bytes)
        .map_err(|_| GenerationPublicationError::InvalidPointer)?;
    if current.generation > manifest.generation {
        return Err(GenerationPublicationError::StaleGeneration {
            requested: manifest.generation,
            current: current.generation,
        });
    }
    if current.generation == manifest.generation {
        return if current.same_serving_identity(manifest) {
            Ok(PointerDecision::AlreadyPublished(object))
        } else {
            Err(GenerationPublicationError::SameGenerationConflict {
                generation: manifest.generation,
            })
        };
    }
    if object.etag.trim().is_empty() {
        return Err(GenerationPublicationError::InvalidPointer);
    }
    Ok(PointerDecision::Publish(PointerPrecondition::Matches {
        etag: object.etag,
    }))
}

fn verify_already_published_outputs<S: PublicationStore>(
    store: &S,
    publication: &GenerationPublication,
    manifest: &FrozenSnapshotManifest,
    manifest_bytes: &[u8],
    registry_bytes: &[u8],
    live_pointer: &PublicationObject,
) -> std::result::Result<(), GenerationPublicationError> {
    if live_pointer.bytes != manifest_bytes {
        return Err(GenerationPublicationError::ImmutableOutputMismatch {
            kind: "generation_manifest",
        });
    }

    verify_immutable_output(
        store,
        "snapshot",
        &manifest.snapshot_uri,
        &manifest.sha256,
        manifest.bytes,
        &publication.snapshot_bytes,
    )?;

    let stored_manifest = store
        .read_object(&publication.snapshot_manifest_uri)?
        .ok_or(GenerationPublicationError::MissingImmutableOutput {
            kind: "generation_manifest",
        })?;
    let decoded_manifest = FrozenSnapshotManifest::from_json_slice(&stored_manifest.bytes)
        .map_err(|_| GenerationPublicationError::ImmutableOutputMismatch {
            kind: "generation_manifest",
        })?;
    if stored_manifest.bytes != live_pointer.bytes
        || stored_manifest.bytes != manifest_bytes
        || decoded_manifest != *manifest
    {
        return Err(GenerationPublicationError::ImmutableOutputMismatch {
            kind: "generation_manifest",
        });
    }

    verify_immutable_output(
        store,
        "serving_registry",
        manifest
            .serving_registry_uri
            .as_deref()
            .ok_or(GenerationPublicationError::InvalidPointer)?,
        manifest
            .serving_registry_sha256
            .as_deref()
            .ok_or(GenerationPublicationError::InvalidPointer)?,
        manifest
            .serving_registry_bytes
            .ok_or(GenerationPublicationError::InvalidPointer)?,
        registry_bytes,
    )
}

fn verify_immutable_output<S: PublicationStore>(
    store: &S,
    kind: &'static str,
    uri: &str,
    expected_sha256: &str,
    expected_bytes: u64,
    canonical_bytes: &[u8],
) -> std::result::Result<(), GenerationPublicationError> {
    let object = store
        .read_object(uri)?
        .ok_or(GenerationPublicationError::MissingImmutableOutput { kind })?;
    if object.bytes.len() as u64 != expected_bytes
        || sha256_bytes(&object.bytes) != expected_sha256
        || object.bytes != canonical_bytes
    {
        return Err(GenerationPublicationError::ImmutableOutputMismatch { kind });
    }
    Ok(())
}

fn put_immutable_exact<S: PublicationStore>(
    store: &S,
    uri: &str,
    bytes: &[u8],
    content_type: &str,
) -> std::result::Result<(), GenerationPublicationError> {
    match store.put_immutable_object(uri, bytes, content_type) {
        Ok(()) => Ok(()),
        Err(error) => match store.read_object(uri) {
            Ok(Some(existing)) if existing.bytes == bytes => Ok(()),
            Ok(Some(_)) => Err(GenerationPublicationError::ImmutableOutputMismatch {
                kind: "immutable_write",
            }),
            _ => Err(GenerationPublicationError::Store(error)),
        },
    }
}

pub fn rollback_frozen_snapshot_pointer(
    data_path: &str,
    generation: i64,
) -> Result<FrozenSnapshotManifest> {
    let location = SnapshotLocation::for_data_path_and_generation(data_path, generation)?;
    if location.s3_uri.is_some() {
        bail!(
            "stale_generation: shared S3 live-pointer rollback is forbidden; retain the current complete generation"
        );
    }
    let manifest = read_snapshot_manifest(&location)?;
    manifest.ensure_published()?;

    if !location.local_path.is_file() {
        bail!(
            "cannot roll back to missing frozen snapshot `{}`",
            location.local_path.display()
        );
    }

    publish_snapshot_pointer(&location, &manifest)?;
    Ok(manifest)
}

pub(crate) fn readable_table(conn: &Connection, table: &str) -> Result<String> {
    if table_exists_in_schema(conn, "gold", table)? {
        Ok(format!("gold.{table}"))
    } else {
        Ok(table.to_owned())
    }
}

pub(crate) fn gold_table(table: &str) -> String {
    format!("gold.{table}")
}

fn revision_info_from_row(row: &duckdb::Row<'_>) -> duckdb::Result<RevisionInfo> {
    Ok(RevisionInfo {
        source: row.get(0)?,
        package: row.get(1)?,
        revision: row.get(2)?,
        revision_kind: row.get(3)?,
        semver_major: row.get(4)?,
        semver_minor: row.get(5)?,
        semver_patch: row.get(6)?,
        snapshot_id: row.get(7)?,
        index_status: row.get(8)?,
        embeddings_status: row.get(9)?,
    })
}

fn optional_no_rows<T>(result: duckdb::Result<T>, context: &'static str) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error).context(context),
    }
}

fn load_ducklake_extensions(conn: &Connection, catalog_dsn: &str) -> Result<()> {
    load_duckdb_extension(conn, "ducklake", "failed to load ducklake extension")?;

    if is_remote_catalog(catalog_dsn) {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        load_duckdb_extension(
            conn,
            "httpfs",
            "failed to load httpfs extension for remote DuckLake catalog",
        )?;
        conn.execute_batch(&format!(
            "CREATE OR REPLACE SECRET s3_creds (TYPE s3, PROVIDER credential_chain, REGION '{region}');",
        ))
        .context("failed to configure S3 credentials for remote DuckLake catalog")?;
    } else if catalog_dsn.starts_with("sqlite:") || catalog_dsn.starts_with("ducklake:sqlite:") {
        load_duckdb_extension(
            conn,
            "sqlite",
            "failed to load sqlite extension for DuckLake catalog",
        )?;
    } else if catalog_dsn.starts_with("postgres:")
        || catalog_dsn.starts_with("postgresql:")
        || catalog_dsn.starts_with("postgresql://")
        || catalog_dsn.starts_with("ducklake:postgres:")
        || catalog_dsn.starts_with("ducklake:postgresql:")
        || catalog_dsn.starts_with("ducklake:postgresql://")
    {
        load_duckdb_extension(
            conn,
            "postgres",
            "failed to load postgres extension for DuckLake catalog",
        )?;
    }

    Ok(())
}

pub(crate) fn ducklake_data_path(catalog_dsn: &str) -> Result<String> {
    if let Ok(path) = std::env::var(DUCKLAKE_DATA_PATH_ENV) {
        if !path.trim().is_empty() {
            create_local_data_path_if_needed(&path)?;
            return Ok(path);
        }
    }

    if let Some(sqlite_path) = sqlite_catalog_path(catalog_dsn) {
        let path = sqlite_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("data");
        fs::create_dir_all(&path)
            .with_context(|| format!("failed to create DuckLake data path `{}`", path.display()))?;
        return Ok(path.display().to_string());
    }

    bail!("{DUCKLAKE_DATA_PATH_ENV} must be set for non-local DuckLake catalogs")
}

fn create_local_data_path_if_needed(path: &str) -> Result<()> {
    if path.contains("://") || path == ":memory:" {
        return Ok(());
    }
    fs::create_dir_all(path)
        .with_context(|| format!("failed to create DuckLake data path `{path}`"))?;
    Ok(())
}

/// DuckLake ATTACH URI for a catalog DSN.
///
/// Postgres catalogs reuse the solved pause-resume `connect_timeout=30`
/// (`sol_f9b97ca9d2a94eef` / `sol_1744e36a489a4b49`) so bronze/translate
/// attach fail-fasts instead of hanging on an unguarded TCP timeout.
pub(crate) fn ducklake_attach_uri(catalog_dsn: &str) -> String {
    let rest = catalog_dsn.strip_prefix("ducklake:").unwrap_or(catalog_dsn);
    if is_postgres_catalog(catalog_dsn) {
        format!(
            "ducklake:{}",
            with_postgres_connect_timeout(rest, POSTGRES_PAUSE_RESUME_CONNECT_TIMEOUT_SECS)
        )
    } else if catalog_dsn.starts_with("ducklake:") {
        catalog_dsn.to_owned()
    } else {
        format!("ducklake:{catalog_dsn}")
    }
}

fn attach_ducklake(conn: &Connection, catalog_dsn: &str, data_path: &str) -> Result<()> {
    if is_remote_catalog(catalog_dsn) {
        conn.execute_batch(&format!(
            "ATTACH '{}' AS spur_context (TYPE ducklake); USE spur_context;",
            escape_sql_literal(catalog_dsn)
        ))
        .context("failed to attach remote DuckLake catalog")
    } else {
        let attach_uri = ducklake_attach_uri(catalog_dsn);
        retry_postgres_pause_resume(
            || {
                conn.execute_batch(&format!(
                    "ATTACH '{}' AS spur_context (DATA_PATH '{}'); USE spur_context;",
                    escape_sql_literal(&attach_uri),
                    escape_sql_literal(data_path)
                ))
                .map_err(|error| anyhow!("{}", redact_libpq_secrets(&error.to_string())))
                .map(|_| ())
            },
            thread::sleep,
        )
        .context("failed to attach DuckLake catalog")
    }
}

fn attach_frozen_snapshot(conn: &Connection, snapshot_path: &Path, data_path: &str) -> Result<()> {
    let attach_uri = format!("ducklake:{}", snapshot_path.display());
    conn.execute_batch(&format!(
        "ATTACH '{}' AS spur_context (DATA_PATH '{}', READ_ONLY); USE spur_context;",
        escape_sql_literal(&attach_uri),
        escape_sql_literal(data_path)
    ))
    .context("failed to attach frozen DuckLake snapshot")
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn table_exists_in_schema(conn: &Connection, schema: &str, table: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            r"
            SELECT COUNT(*)
            FROM information_schema.tables
            WHERE table_schema = ? AND table_name = ?
            ",
            params![schema, table],
            |row| row.get(0),
        )
        .with_context(|| format!("failed to inspect table {schema}.{table}"))?;
    Ok(count > 0)
}

fn run_optional_maintenance_call(conn: &Connection, sql: &str) -> Result<()> {
    match conn.prepare(sql) {
        Ok(mut stmt) => {
            let mut rows = stmt
                .query([])
                .with_context(|| format!("failed to execute DuckLake maintenance `{sql}`"))?;
            while rows.next()?.is_some() {}
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            if message.contains("does not exist") {
                eprintln!(
                    "[catalog] skipping unavailable DuckLake maintenance call `{sql}`: {message}"
                );
                Ok(())
            } else {
                Err(error)
                    .with_context(|| format!("failed to prepare DuckLake maintenance `{sql}`"))
            }
        }
    }
}

fn interval_literal(duration: Duration) -> String {
    format!("{} seconds", duration.as_secs())
}

#[derive(Debug)]
struct SnapshotLocation {
    local_path: PathBuf,
    local_staging_path: PathBuf,
    local_manifest_path: PathBuf,
    local_pointer_path: PathBuf,
    snapshot_uri: String,
    s3_uri: Option<String>,
    s3_manifest_uri: Option<String>,
    s3_serving_registry_uri: Option<String>,
    s3_pointer_uri: Option<String>,
}

impl SnapshotLocation {
    fn for_data_path_and_generation(data_path: &str, generation: i64) -> Result<Self> {
        if generation <= 0 {
            bail!("frozen snapshot generation must be positive");
        }

        let generation_dir = format!("{generation:020}");
        if data_path.starts_with("s3://") {
            let uri = snapshot_s3_uri(data_path, generation);
            let manifest_uri = snapshot_manifest_s3_uri(data_path, generation);
            let serving_registry_uri = snapshot_serving_registry_s3_uri(data_path, generation);
            let pointer_uri = snapshot_pointer_s3_uri(data_path);
            let mut local_path = std::env::temp_dir();
            local_path.push(format!(
                "spur_context_snapshot_{}.ducklake",
                uuid::Uuid::new_v4()
            ));
            let local_manifest_path = local_path.with_extension("manifest.json");
            let local_pointer_path = local_path.with_extension("current.json");
            Ok(Self {
                local_staging_path: local_path.clone(),
                local_path,
                local_manifest_path,
                local_pointer_path,
                snapshot_uri: uri.clone(),
                s3_uri: Some(uri),
                s3_manifest_uri: Some(manifest_uri),
                s3_serving_registry_uri: Some(serving_registry_uri),
                s3_pointer_uri: Some(pointer_uri),
            })
        } else {
            let snapshot_dir = Path::new(data_path)
                .join(SNAPSHOT_GENERATIONS_RELATIVE_DIR)
                .join(generation_dir);
            let local_path = snapshot_dir.join(SNAPSHOT_FILE_NAME);
            let local_staging_path = snapshot_dir.join(format!(
                ".{SNAPSHOT_FILE_NAME}.{}.tmp",
                uuid::Uuid::new_v4()
            ));
            let local_manifest_path = snapshot_dir.join(SNAPSHOT_MANIFEST_FILE_NAME);
            let local_pointer_path = Path::new(data_path).join(SNAPSHOT_POINTER_RELATIVE_PATH);
            Ok(Self {
                snapshot_uri: local_path.display().to_string(),
                local_path,
                local_staging_path,
                local_manifest_path,
                local_pointer_path,
                s3_uri: None,
                s3_manifest_uri: None,
                s3_serving_registry_uri: None,
                s3_pointer_uri: None,
            })
        }
    }
}

fn snapshot_s3_uri(data_path: &str, generation: i64) -> String {
    format!(
        "{}/{}/{:020}/{}",
        snapshot_base_uri(data_path).trim_end_matches('/'),
        SNAPSHOT_GENERATIONS_RELATIVE_DIR,
        generation,
        SNAPSHOT_FILE_NAME
    )
}

fn snapshot_manifest_s3_uri(data_path: &str, generation: i64) -> String {
    format!(
        "{}/{}/{:020}/{}",
        snapshot_base_uri(data_path).trim_end_matches('/'),
        SNAPSHOT_GENERATIONS_RELATIVE_DIR,
        generation,
        SNAPSHOT_MANIFEST_FILE_NAME
    )
}

fn snapshot_serving_registry_s3_uri(data_path: &str, generation: i64) -> String {
    format!(
        "{}/{}/{:020}/{}",
        snapshot_base_uri(data_path).trim_end_matches('/'),
        SNAPSHOT_GENERATIONS_RELATIVE_DIR,
        generation,
        SERVING_REGISTRY_FILE_NAME
    )
}

fn snapshot_pointer_s3_uri(data_path: &str) -> String {
    format!(
        "{}/{}",
        snapshot_base_uri(data_path).trim_end_matches('/'),
        SNAPSHOT_POINTER_RELATIVE_PATH
    )
}

fn snapshot_base_uri(data_path: &str) -> String {
    let trimmed = data_path.trim_end_matches('/');
    if let Some(base) = trimmed.strip_suffix("/gold/data") {
        base.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn current_snapshot_generation(conn: &Connection) -> Result<i64> {
    let generation: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(generation), MAX(snapshot_id), 0)::BIGINT FROM gold.package_catalog",
            [],
            |row| row.get(0),
        )
        .context("failed to read current published snapshot generation")?;
    if generation <= 0 {
        bail!("cannot export frozen snapshot before a gold generation is published");
    }
    Ok(generation)
}

fn publish_snapshot_file(location: &SnapshotLocation) -> Result<()> {
    if let Some(uri) = &location.s3_uri {
        upload_file_to_s3(&location.local_staging_path, uri)?;
        return Ok(());
    }

    if let Some(parent) = location.local_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create snapshot dir `{}`", parent.display()))?;
    }
    if location.local_path.exists() {
        bail!(
            "immutable frozen snapshot already exists: `{}`",
            location.local_path.display()
        );
    }
    fs::rename(&location.local_staging_path, &location.local_path).with_context(|| {
        format!(
            "failed to publish frozen snapshot `{}`",
            location.local_path.display()
        )
    })
}

fn publish_snapshot_manifest(
    location: &SnapshotLocation,
    manifest: &FrozenSnapshotManifest,
) -> Result<()> {
    let bytes = manifest.to_json_bytes()?;
    if let Some(uri) = &location.s3_manifest_uri {
        put_s3_object(uri, bytes, "application/json", false)?;
    } else {
        write_json_file_atomic(&location.local_manifest_path, &bytes, false)?;
    }
    Ok(())
}

fn publish_snapshot_pointer(
    location: &SnapshotLocation,
    manifest: &FrozenSnapshotManifest,
) -> Result<()> {
    let bytes = manifest.to_json_bytes()?;
    if let Some(uri) = &location.s3_pointer_uri {
        put_s3_object(uri, bytes, "application/json", true)?;
    } else {
        publish_local_snapshot_pointer(&location.local_pointer_path, manifest)?;
    }
    Ok(())
}

fn publish_snapshot_pointer_if_current_not_newer(
    location: &SnapshotLocation,
    manifest: &FrozenSnapshotManifest,
) -> Result<()> {
    if let Some(current) = read_snapshot_pointer(location)? {
        current.ensure_published()?;
        if current.generation > manifest.generation {
            return Ok(());
        }
        if current.generation == manifest.generation
            && current.snapshot_uri != manifest.snapshot_uri
        {
            bail!(
                "live frozen snapshot pointer for generation {} points at `{}`, not `{}`",
                current.generation,
                current.snapshot_uri,
                manifest.snapshot_uri
            );
        }
    }
    publish_snapshot_pointer(location, manifest)
}

fn read_snapshot_pointer(location: &SnapshotLocation) -> Result<Option<FrozenSnapshotManifest>> {
    let bytes = if let Some(uri) = &location.s3_pointer_uri {
        match get_s3_object_bytes_optional(uri)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        }
    } else {
        match fs::read(&location.local_pointer_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to read frozen snapshot pointer `{}`",
                        location.local_pointer_path.display()
                    )
                });
            }
        }
    };
    FrozenSnapshotManifest::from_json_slice(&bytes).map(Some)
}

fn read_snapshot_manifest(location: &SnapshotLocation) -> Result<FrozenSnapshotManifest> {
    let bytes = if let Some(uri) = &location.s3_manifest_uri {
        get_s3_object_bytes(uri)?
    } else {
        fs::read(&location.local_manifest_path).with_context(|| {
            format!(
                "failed to read frozen snapshot manifest `{}`",
                location.local_manifest_path.display()
            )
        })?
    };
    FrozenSnapshotManifest::from_json_slice(&bytes)
}

fn publish_local_snapshot_pointer(
    pointer_path: &Path,
    manifest: &FrozenSnapshotManifest,
) -> Result<()> {
    let bytes = manifest.to_json_bytes()?;
    write_json_file_atomic(pointer_path, &bytes, true)
}

#[cfg(test)]
fn read_local_snapshot_pointer(pointer_path: &Path) -> Result<FrozenSnapshotManifest> {
    let bytes = fs::read(pointer_path).with_context(|| {
        format!(
            "failed to read snapshot pointer `{}`",
            pointer_path.display()
        )
    })?;
    FrozenSnapshotManifest::from_json_slice(&bytes)
}

fn write_json_file_atomic(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create json dir `{}`", parent.display()))?;
    }
    if !overwrite && path.exists() {
        bail!("immutable json object already exists: `{}`", path.display());
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot.json");
    let tmp_path = path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, bytes)
        .with_context(|| format!("failed to write temp json `{}`", tmp_path.display()))?;
    if !overwrite && path.exists() {
        let _ = fs::remove_file(&tmp_path);
        bail!("immutable json object already exists: `{}`", path.display());
    }
    fs::rename(&tmp_path, path)
        .with_context(|| format!("failed to atomically publish json `{}`", path.display()))
}

fn file_sha256_and_len(path: &Path) -> Result<(String, u64)> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read snapshot `{}`", path.display()))?;
    Ok((sha256_bytes(&bytes), bytes.len() as u64))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn validate_strong_ref(name: &str, uri: &str, sha256: &str, bytes: u64) -> Result<()> {
    parse_s3_uri(uri).with_context(|| format!("{name} URI is not an S3 object URI"))?;
    if sha256.len() != 64 || !sha256.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        bail!("{name} SHA-256 must be 64 ASCII hexadecimal characters");
    }
    if bytes == 0 {
        bail!("{name} byte length must be positive");
    }
    Ok(())
}

fn copy_ducklake_metadata_tables(catalog_dsn: &str, snapshot_path: &Path) -> Result<()> {
    let catalog_dsn = catalog_dsn_with_env_password(catalog_dsn);
    let conn = Connection::open_in_memory().context("failed to open snapshot exporter DuckDB")?;
    load_duckdb_extensions(
        &conn,
        &["sqlite", "postgres"],
        "failed to load metadata backend extensions",
    )?;
    let source = attach_metadata_catalog(&conn, &catalog_dsn)?;
    conn.execute_batch(&format!(
        "ATTACH '{}' AS snap;",
        escape_sql_literal(&snapshot_path.display().to_string())
    ))
    .context("failed to attach snapshot DuckDB file")?;

    let tables = list_ducklake_metadata_tables(&conn, &source)?;
    if tables.is_empty() {
        bail!("catalog metadata backend has no ducklake_* tables");
    }
    copy_metadata_tables_in_transaction(&conn, &source, &tables)
}

fn copy_metadata_tables_in_transaction(
    conn: &Connection,
    source: &MetadataSource,
    tables: &[String],
) -> Result<()> {
    // Keep all attached metadata reads on one snapshot. Without this, each
    // CTAS can observe a different Postgres/Aurora catalog commit.
    conn.execute_batch("BEGIN TRANSACTION")
        .context("failed to begin consistent DuckLake metadata snapshot copy")?;
    let result = (|| {
        for table in tables {
            conn.execute_batch(&format!(
                "CREATE TABLE snap.\"{table}\" AS SELECT * FROM {}.\"{table}\";",
                source.select_prefix
            ))
            .with_context(|| format!("failed to copy DuckLake metadata table `{table}`"))?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .context("failed to commit DuckLake metadata snapshot copy"),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

#[derive(Debug)]
struct MetadataSource {
    select_prefix: String,
    list_sql: &'static str,
}

fn attach_metadata_catalog(conn: &Connection, catalog_dsn: &str) -> Result<MetadataSource> {
    if let Some(path) = sqlite_catalog_path(catalog_dsn) {
        conn.execute_batch(&format!(
            "ATTACH '{}' AS pg (TYPE sqlite);",
            escape_sql_literal(&path.display().to_string())
        ))
        .context("failed to attach sqlite DuckLake metadata catalog")?;
        return Ok(MetadataSource {
            select_prefix: "pg".to_owned(),
            list_sql: "SHOW TABLES FROM pg",
        });
    }

    if is_postgres_catalog(catalog_dsn) {
        let dsn = postgres_metadata_dsn(catalog_dsn);
        conn.execute_batch(&format!(
            "ATTACH '{}' AS pg (TYPE postgres);",
            escape_sql_literal(&dsn)
        ))
        .context("failed to attach postgres DuckLake metadata catalog")?;
        return Ok(MetadataSource {
            select_prefix: "pg.public".to_owned(),
            list_sql: "SHOW TABLES FROM pg.public",
        });
    }

    bail!("snapshot export supports sqlite and postgres DuckLake catalogs, got `{catalog_dsn}`")
}

fn list_ducklake_metadata_tables(
    conn: &Connection,
    source: &MetadataSource,
) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(source.list_sql)
        .context("failed to prepare DuckLake metadata table listing")?;
    let mut tables = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to collect DuckLake metadata table names")?;
    tables.retain(|table| table.starts_with("ducklake_"));
    tables.sort();
    Ok(tables)
}

fn replay_snapshot_indexes(snapshot_path: &Path) -> Result<()> {
    let conn = Connection::open(snapshot_path)
        .with_context(|| format!("failed to open snapshot `{}`", snapshot_path.display()))?;
    for (index_name, table, column) in SNAPSHOT_INDEXES {
        conn.execute_batch(&format!(
            "CREATE UNIQUE INDEX {index_name} ON \"{table}\"({column});"
        ))
        .with_context(|| format!("failed to replay DuckLake metadata index `{index_name}`"))?;
    }
    Ok(())
}

fn validate_snapshot_attaches(snapshot_path: &Path, data_path: &str) -> Result<()> {
    let conn = connect_ducklake_with_data_path(&snapshot_path.display().to_string(), data_path)?;
    let _count: i64 = conn
        .query_row(
            "SELECT COUNT(*)::BIGINT FROM information_schema.tables",
            [],
            |row| row.get(0),
        )
        .context("failed to query attached frozen snapshot")?;
    Ok(())
}

fn verify_snapshot_referenced_files_exist(snapshot_path: &Path, data_path: &str) -> Result<()> {
    let conn = Connection::open(snapshot_path)
        .with_context(|| format!("failed to open snapshot `{}`", snapshot_path.display()))?;
    for (table, id_column) in [
        ("ducklake_data_file", "data_file_id"),
        ("ducklake_delete_file", "delete_file_id"),
    ] {
        let sql = format!(
            r#"
            SELECT
                f.path,
                COALESCE(f.path_is_relative, 0)::BIGINT,
                t.path,
                COALESCE(t.path_is_relative, 0)::BIGINT,
                s.path,
                COALESCE(s.path_is_relative, 0)::BIGINT
            FROM "{table}" f
            JOIN ducklake_table t ON t.table_id = f.table_id
            JOIN ducklake_schema s ON s.schema_id = t.schema_id
            ORDER BY f.{id_column}
            "#
        );
        let mut stmt = conn
            .prepare(&sql)
            .with_context(|| format!("failed to prepare referenced-file query for {table}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ReferencedFile {
                    file_path: row.get(0)?,
                    file_path_is_relative: row.get::<_, i64>(1)? != 0,
                    table_path: row.get(2)?,
                    table_path_is_relative: row.get::<_, i64>(3)? != 0,
                    schema_path: row.get(4)?,
                    schema_path_is_relative: row.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("failed to collect referenced files from {table}"))?;
        for referenced in rows {
            verify_referenced_file(data_path, &referenced)?;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ReferencedFile {
    file_path: String,
    file_path_is_relative: bool,
    table_path: String,
    table_path_is_relative: bool,
    schema_path: String,
    schema_path_is_relative: bool,
}

fn verify_referenced_file(data_path: &str, referenced: &ReferencedFile) -> Result<()> {
    if data_path.starts_with("s3://") {
        let uri = referenced_s3_uri(data_path, referenced);
        return head_s3_object(&uri);
    }

    let path = referenced_local_path(data_path, referenced);
    if !path.is_file() {
        bail!("snapshot references missing data file `{}`", path.display());
    }
    Ok(())
}

fn referenced_local_path(data_path: &str, referenced: &ReferencedFile) -> PathBuf {
    if !referenced.file_path_is_relative {
        return PathBuf::from(&referenced.file_path);
    }
    let schema_base = if referenced.schema_path_is_relative {
        Path::new(data_path).join(&referenced.schema_path)
    } else {
        PathBuf::from(&referenced.schema_path)
    };
    let table_base = if referenced.table_path_is_relative {
        schema_base.join(&referenced.table_path)
    } else {
        PathBuf::from(&referenced.table_path)
    };
    table_base.join(&referenced.file_path)
}

fn referenced_s3_uri(data_path: &str, referenced: &ReferencedFile) -> String {
    if !referenced.file_path_is_relative {
        return referenced.file_path.clone();
    }
    let schema_base = if referenced.schema_path_is_relative {
        join_uri_path(data_path, &referenced.schema_path)
    } else {
        referenced.schema_path.clone()
    };
    let table_base = if referenced.table_path_is_relative {
        join_uri_path(&schema_base, &referenced.table_path)
    } else {
        referenced.table_path.clone()
    };
    join_uri_path(&table_base, &referenced.file_path)
}

fn join_uri_path(base: &str, child: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        child.trim_start_matches('/')
    )
}

impl PublicationStore for S3PublicationStore {
    fn read_object(
        &self,
        uri: &str,
    ) -> std::result::Result<Option<PublicationObject>, PublicationStoreError> {
        get_s3_publication_object(uri)
    }

    fn put_immutable_object(
        &self,
        uri: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> std::result::Result<(), PublicationStoreError> {
        put_s3_publication_object(
            uri,
            bytes.to_vec(),
            content_type,
            PointerPrecondition::Absent,
        )
    }

    fn compare_and_swap_pointer(
        &self,
        uri: &str,
        bytes: &[u8],
        content_type: &str,
        precondition: &PointerPrecondition,
    ) -> std::result::Result<(), PublicationStoreError> {
        put_s3_publication_object(uri, bytes.to_vec(), content_type, precondition.clone())
    }
}

fn s3_get_is_not_found(error: &SdkError<GetObjectError>) -> bool {
    let modeled_code = error
        .as_service_error()
        .and_then(|service_error| service_error.meta().code());
    let status = error
        .raw_response()
        .map(|response| response.status().as_u16());
    is_s3_get_not_found(modeled_code, status)
}

pub(crate) fn is_s3_get_not_found(modeled_code: Option<&str>, status: Option<u16>) -> bool {
    matches!(modeled_code, Some("NoSuchKey" | "NotFound")) || status == Some(404)
}

#[allow(dead_code)] // Used by the Lambda feature's typed SDK error seam.
pub(crate) fn is_s3_not_found_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<SdkError<GetObjectError>>()
        .is_some_and(s3_get_is_not_found)
}

fn s3_conditional_write_is_stale(error: &SdkError<PutObjectError>) -> bool {
    let modeled_code = error
        .as_service_error()
        .and_then(|service_error| service_error.meta().code());
    let status = error
        .raw_response()
        .map(|response| response.status().as_u16());
    is_s3_conditional_write_stale(modeled_code, status)
}

pub(crate) fn is_s3_conditional_write_stale(
    modeled_code: Option<&str>,
    status: Option<u16>,
) -> bool {
    matches!(
        modeled_code,
        Some("PreconditionFailed" | "ConditionalRequestConflict" | "NoSuchKey" | "NotFound")
    ) || matches!(status, Some(404 | 409 | 412))
}

fn get_s3_publication_object(
    uri: &str,
) -> std::result::Result<Option<PublicationObject>, PublicationStoreError> {
    let parsed = parse_s3_uri(uri).map_err(|_| PublicationStoreError::Storage)?;
    run_s3_publication_blocking(move |client| async move {
        let output = match client
            .get_object()
            .bucket(parsed.bucket)
            .key(parsed.key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(error) if s3_get_is_not_found(&error) => return Ok(None),
            Err(_) => return Err(PublicationStoreError::Storage),
        };
        let etag = output.e_tag().unwrap_or_default().to_owned();
        let version_id = output.version_id().map(str::to_owned);
        let bytes = output
            .body
            .collect()
            .await
            .map_err(|_| PublicationStoreError::Storage)?;
        Ok(Some(PublicationObject {
            bytes: bytes.into_bytes().to_vec(),
            etag,
            version_id,
        }))
    })
}

fn put_s3_publication_object(
    uri: &str,
    bytes: Vec<u8>,
    content_type: &str,
    precondition: PointerPrecondition,
) -> std::result::Result<(), PublicationStoreError> {
    let parsed = parse_s3_uri(uri).map_err(|_| PublicationStoreError::Storage)?;
    let content_type = content_type.to_owned();
    run_s3_publication_blocking(move |client| async move {
        let request = client
            .put_object()
            .bucket(parsed.bucket)
            .key(parsed.key)
            .content_type(content_type)
            .body(ByteStream::from(bytes));
        let request = match precondition {
            PointerPrecondition::Absent => request.if_none_match("*"),
            PointerPrecondition::Matches { etag } => request.if_match(etag),
        };
        match request.send().await {
            Ok(_) => Ok(()),
            Err(error) if s3_conditional_write_is_stale(&error) => {
                Err(PublicationStoreError::PreconditionFailed)
            }
            Err(_) => Err(PublicationStoreError::Storage),
        }
    })
}

fn run_s3_publication_blocking<T, F, Fut>(f: F) -> std::result::Result<T, PublicationStoreError>
where
    F: FnOnce(aws_sdk_s3::Client) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = std::result::Result<T, PublicationStoreError>>
        + Send
        + 'static,
    T: Send + 'static,
{
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().map_err(|_| PublicationStoreError::Storage)?;
        runtime.block_on(async move {
            let client = s3_client_from_env();
            f(client).await
        })
    })
    .join()
    .map_err(|_| PublicationStoreError::Storage)?
}

fn upload_file_to_s3(local_path: &Path, uri: &str) -> Result<()> {
    let bytes = fs::read(local_path)
        .with_context(|| format!("failed to read snapshot `{}`", local_path.display()))?;
    put_s3_object(uri, bytes, "application/octet-stream", false)
}

fn put_s3_object(
    uri: &str,
    bytes: Vec<u8>,
    content_type: &str,
    allow_overwrite: bool,
) -> Result<()> {
    let parsed = parse_s3_uri(uri)?;
    let uri = uri.to_owned();
    let content_type = content_type.to_owned();
    run_s3_blocking(move |client| async move {
        let mut request = client
            .put_object()
            .bucket(parsed.bucket)
            .key(parsed.key)
            .content_type(content_type)
            .body(ByteStream::from(bytes));
        if !allow_overwrite {
            request = request.if_none_match("*");
        }
        request
            .send()
            .await
            .with_context(|| format!("failed to upload frozen snapshot object `{uri}`"))?;
        Ok(())
    })
}

fn head_s3_object(uri: &str) -> Result<()> {
    let parsed = parse_s3_uri(uri)?;
    let uri = uri.to_owned();
    run_s3_blocking(move |client| async move {
        client
            .head_object()
            .bucket(parsed.bucket)
            .key(parsed.key)
            .send()
            .await
            .with_context(|| format!("snapshot references missing S3 object `{uri}`"))?;
        Ok(())
    })
}

fn get_s3_object_bytes(uri: &str) -> Result<Vec<u8>> {
    get_s3_object_bytes_optional(uri)?.ok_or_else(|| anyhow!("required S3 object is missing"))
}

fn get_s3_object_bytes_optional(uri: &str) -> Result<Option<Vec<u8>>> {
    get_s3_publication_object(uri)
        .map(|object| object.map(|object| object.bytes))
        .map_err(|_| anyhow!("S3 object read failed"))
}

fn run_s3_blocking<T, F, Fut>(f: F) -> Result<T>
where
    F: FnOnce(aws_sdk_s3::Client) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().context("failed to create S3 runtime")?;
        runtime.block_on(async move {
            let client = s3_client_from_env();
            f(client).await
        })
    })
    .join()
    .map_err(|_| anyhow!("S3 helper thread panicked"))?
}

#[derive(Debug)]
struct ParsedS3Uri {
    bucket: String,
    key: String,
}

fn parse_s3_uri(uri: &str) -> Result<ParsedS3Uri> {
    let rest = uri
        .strip_prefix("s3://")
        .ok_or_else(|| anyhow!("not an s3 URI: {uri}"))?;
    let (bucket, key) = rest
        .split_once('/')
        .ok_or_else(|| anyhow!("s3 URI must include a key: {uri}"))?;
    if bucket.is_empty() || key.is_empty() {
        bail!("s3 URI must include non-empty bucket and key: {uri}");
    }
    Ok(ParsedS3Uri {
        bucket: bucket.to_owned(),
        key: key.to_owned(),
    })
}

fn s3_client_from_env() -> aws_sdk_s3::Client {
    let region = std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_owned());
    let mut config = aws_sdk_s3::Config::builder()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new(region));
    if let (Ok(access_key), Ok(secret_key)) = (
        std::env::var("AWS_ACCESS_KEY_ID"),
        std::env::var("AWS_SECRET_ACCESS_KEY"),
    ) {
        config = config.credentials_provider(aws_sdk_s3::config::Credentials::new(
            access_key,
            secret_key,
            std::env::var("AWS_SESSION_TOKEN").ok(),
            None,
            "Env",
        ));
    }
    aws_sdk_s3::Client::from_conf(config.build())
}

pub(crate) fn postgres_metadata_dsn(catalog_dsn: &str) -> String {
    let dsn = catalog_dsn.strip_prefix("ducklake:").unwrap_or(catalog_dsn);
    let stripped = if let Some(rest) = dsn.strip_prefix("postgres:") {
        rest.to_owned()
    } else if let Some(rest) = dsn.strip_prefix("postgresql:") {
        format!("postgresql:{rest}")
    } else {
        dsn.to_owned()
    };
    with_postgres_connect_timeout(&stripped, POSTGRES_PAUSE_RESUME_CONNECT_TIMEOUT_SECS)
}

fn with_postgres_connect_timeout(dsn: &str, timeout_secs: u64) -> String {
    if postgres_keyword_dsn_has(dsn, "connect_timeout") {
        return dsn.to_owned();
    }
    format!("{dsn} connect_timeout={timeout_secs}")
}

fn postgres_keyword_dsn_has(dsn: &str, key: &str) -> bool {
    let needle = format!("{key}=");
    dsn.split_whitespace().any(|token| {
        token
            .strip_prefix('\'')
            .unwrap_or(token)
            .starts_with(&needle)
    })
}

fn is_postgres_pause_resume_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("connection timed out")
        || lower.contains("timeout expired")
        || lower.contains("the database system is starting up")
        || lower.contains("the database system is not yet accepting connections")
        || lower.contains("could not connect to server")
        || lower.contains("connection refused")
        || lower.contains("server closed the connection unexpectedly")
}

pub(crate) fn retry_postgres_pause_resume<T, E>(
    mut op: impl FnMut() -> std::result::Result<T, E>,
    mut sleep: impl FnMut(Duration),
) -> std::result::Result<T, E>
where
    E: std::fmt::Display,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match op() {
            Ok(value) => return Ok(value),
            Err(error)
                if attempt < POSTGRES_PAUSE_RESUME_ATTEMPTS
                    && is_postgres_pause_resume_error(&error.to_string()) =>
            {
                sleep(POSTGRES_PAUSE_RESUME_BACKOFF);
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn redact_libpq_secrets(message: &str) -> String {
    let mut redacted = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(idx) = rest.find("password=") {
        redacted.push_str(&rest[..idx]);
        redacted.push_str("password=REDACTED");
        let after = &rest[idx + "password=".len()..];
        rest = if after.starts_with('\'') {
            after
                .get(1..)
                .and_then(|quoted| {
                    let mut chars = quoted.char_indices();
                    while let Some((i, ch)) = chars.next() {
                        if ch == '\\' {
                            chars.next();
                            continue;
                        }
                        if ch == '\'' {
                            return Some(&quoted[i + 1..]);
                        }
                    }
                    Some("")
                })
                .unwrap_or("")
        } else {
            after
                .split_once(|ch: char| ch.is_whitespace() || ch == '\'' || ch == '"')
                .map(|(_, tail)| tail)
                .unwrap_or("")
        };
    }
    redacted.push_str(rest);
    redacted
}

fn sqlite_catalog_path(catalog_dsn: &str) -> Option<PathBuf> {
    let dsn = catalog_dsn.strip_prefix("ducklake:").unwrap_or(catalog_dsn);
    let path = dsn.strip_prefix("sqlite:")?;
    (path != ":memory:").then(|| PathBuf::from(path))
}

fn is_postgres_catalog(catalog_dsn: &str) -> bool {
    let dsn = catalog_dsn.strip_prefix("ducklake:").unwrap_or(catalog_dsn);
    dsn.starts_with("postgres:")
        || dsn.starts_with("postgresql:")
        || dsn.starts_with("postgresql://")
}

pub(crate) fn catalog_dsn_with_env_password(catalog_dsn: &str) -> String {
    if !is_postgres_catalog(catalog_dsn) || postgres_dsn_has_password(catalog_dsn) {
        return catalog_dsn.to_owned();
    }

    let Ok(password) = std::env::var(CATALOG_PASSWORD_ENV) else {
        return catalog_dsn.to_owned();
    };
    if password.is_empty() {
        return catalog_dsn.to_owned();
    }

    if catalog_dsn.starts_with("postgres:") || catalog_dsn.starts_with("ducklake:postgres:") {
        format!(
            "{catalog_dsn} password='{}'",
            escape_libpq_keyword_value(&password)
        )
    } else {
        catalog_dsn.to_owned()
    }
}

fn postgres_dsn_has_password(catalog_dsn: &str) -> bool {
    let dsn = catalog_dsn.strip_prefix("ducklake:").unwrap_or(catalog_dsn);
    dsn.contains(" password=")
        || dsn.starts_with("postgres:password=")
        || dsn.starts_with("postgresql:password=")
        || dsn.contains("://")
            && dsn.split_once("://").is_some_and(|(_, rest)| {
                rest.split_once('@')
                    .map(|(authority, _)| authority.contains(':'))
                    .unwrap_or(false)
            })
}

fn escape_libpq_keyword_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn credential_test_signature(client: aws_sdk_s3::Client) -> Result<String> {
        let request = client
            .head_object()
            .bucket("catalog-credential-test")
            .key("gold/snapshot.parquet")
            .presigned(aws_sdk_s3::presigning::PresigningConfig::expires_in(
                Duration::from_secs(60),
            )?)
            .await?;
        Ok(request.uri().to_owned())
    }

    #[test]
    fn snapshot_s3_signing_uses_ecs_task_role_credentials() -> Result<()> {
        check_snapshot_s3_credentials("ecs")
    }

    #[test]
    fn snapshot_s3_signing_preserves_lambda_environment_credentials() -> Result<()> {
        check_snapshot_s3_credentials("lambda")
    }

    fn check_snapshot_s3_credentials(provider: &str) -> Result<()> {
        // Isolate the credential chain from the developer's AWS credentials and
        // from other tests: only the subprocess changes credential environment.
        if std::env::var("SPUR_CATALOG_CREDENTIAL_TEST_CHILD").as_deref() == Ok(provider) {
            let ordinary = run_s3_blocking(credential_test_signature)?;
            let publication = run_s3_publication_blocking(|client| async move {
                credential_test_signature(client)
                    .await
                    .map_err(|_| PublicationStoreError::Storage)
            })
            .map_err(|error| anyhow!("publication credentials: {error:?}"))?;
            for uri in [ordinary, publication] {
                assert!(uri.contains("X-Amz-Credential=AKIATESTCATALOG%2F"));
                assert!(uri.contains("X-Amz-Security-Token=catalog-test-token"));
                assert!(uri.contains("ap-southeast-5%2Fs3%2Faws4_request"));
                assert!(uri.contains("X-Amz-Signature="));
            }
            return Ok(());
        }

        use std::io::{Read as _, Write as _};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        let root = unique_temp_dir("snapshot-credential-chain")?;
        let empty_config = root.join("empty-aws-config");
        fs::write(&empty_config, "")?;
        let test_name = match provider {
            "ecs" => "catalog::tests::snapshot_s3_signing_uses_ecs_task_role_credentials",
            _ => "catalog::tests::snapshot_s3_signing_preserves_lambda_environment_credentials",
        };
        let mut child = std::process::Command::new(std::env::current_exe()?);
        child
            .args(["--exact", test_name, "--nocapture"])
            .env_clear()
            .env("SPUR_CATALOG_CREDENTIAL_TEST_CHILD", provider)
            .env("AWS_REGION", "ap-southeast-5")
            .env("AWS_CONFIG_FILE", &empty_config)
            .env("AWS_SHARED_CREDENTIALS_FILE", &empty_config)
            .env("AWS_EC2_METADATA_DISABLED", "true");
        let stopped = Arc::new(AtomicBool::new(false));
        let server = if provider == "ecs" {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            child.env(
                "AWS_CONTAINER_CREDENTIALS_FULL_URI",
                format!("http://{}/credentials", listener.local_addr()?),
            );
            let stopped = Arc::clone(&stopped);
            Some(thread::spawn(move || -> Result<()> {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let deadline = std::time::Instant::now() + Duration::from_secs(5);
                            let mut request = Vec::new();
                            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                                let remaining = deadline
                                    .checked_duration_since(std::time::Instant::now())
                                    .ok_or_else(|| anyhow!("credential request timed out"))?;
                                stream.set_read_timeout(Some(remaining))?;
                                let mut chunk = [0; 1024];
                                let read = stream.read(&mut chunk)?;
                                if read == 0 || request.len() + read > 4096 {
                                    bail!("incomplete or oversized credential request");
                                }
                                request.extend_from_slice(&chunk[..read]);
                            }
                            assert!(
                                String::from_utf8_lossy(&request).starts_with("GET /credentials ")
                            );
                            let body = r#"{"AccessKeyId":"AKIATESTCATALOG","SecretAccessKey":"catalog-test-secret","Token":"catalog-test-token","Expiration":"2099-01-01T00:00:00Z"}"#;
                            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Ok(())
            }))
        } else {
            child
                .env("AWS_ACCESS_KEY_ID", "AKIATESTCATALOG")
                .env("AWS_SECRET_ACCESS_KEY", "catalog-test-secret")
                .env("AWS_SESSION_TOKEN", "catalog-test-token");
            None
        };
        let output = (|| -> Result<std::process::Output> {
            let mut child = child
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            loop {
                if child.try_wait()?.is_some() {
                    return Ok(child.wait_with_output()?);
                }
                if std::time::Instant::now() >= deadline {
                    child.kill()?;
                    child.wait()?;
                    bail!("credential subprocess timed out after 60 seconds");
                }
                thread::sleep(Duration::from_millis(10));
            }
        })();
        stopped.store(true, Ordering::Relaxed);
        if let Some(server) = server {
            server.join().expect("credential mock thread panicked")?;
        }
        fs::remove_dir_all(root)?;
        let output = output?;
        assert!(
            output.status.success(),
            "credential subprocess failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().expect("env lock should not be poisoned")
    }

    #[test]
    fn postgres_ducklake_write_lock_uses_session_advisory_lock() {
        let sql = postgres_ducklake_write_lock_sql("metadata");

        assert!(sql.contains("pg_advisory_lock"));
        assert!(sql.contains("7830668896113191951"));
    }

    #[test]
    fn metadata_copy_failure_does_not_leave_partial_snapshot_tables() -> Result<()> {
        let root = unique_temp_dir("metadata-copy-rollback")?;
        let catalog_path = root.join("catalog.sqlite");
        let snapshot_path = root.join("snapshot.ducklake");

        let setup = Connection::open_in_memory().context("open sqlite metadata setup DuckDB")?;
        setup
            .execute_batch("INSTALL sqlite; LOAD sqlite;")
            .context("load sqlite extension for setup")?;
        setup
            .execute_batch(&format!(
                r#"
                ATTACH '{}' AS pg (TYPE sqlite);
                CREATE TABLE pg.ducklake_a_valid (id BIGINT);
                INSERT INTO pg.ducklake_a_valid VALUES (1);
                CREATE TABLE pg."ducklake_z_bad""identifier" (id BIGINT);
                "#,
                escape_sql_literal(&catalog_path.display().to_string())
            ))
            .context("create sqlite metadata fixture")?;

        let err = copy_ducklake_metadata_tables(
            &format!("sqlite:{}", catalog_path.display()),
            &snapshot_path,
        )
        .expect_err("malformed metadata identifier should fail copy");
        assert!(
            format!("{err:#}").contains("ducklake_z_bad"),
            "unexpected error: {err:#}"
        );

        if snapshot_path.exists() {
            let snap = Connection::open(&snapshot_path)
                .with_context(|| format!("open snapshot `{}`", snapshot_path.display()))?;
            let copied_tables: i64 = snap
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'ducklake_a_valid'",
                    [],
                    |row| row.get(0),
                )
                .context("inspect partial snapshot tables")?;
            assert_eq!(
                copied_tables, 0,
                "metadata copy must roll back earlier tables if a later table fails"
            );
        }
        Ok(())
    }

    #[test]
    fn snapshot_location_uses_immutable_generation_path_and_live_pointer() -> Result<()> {
        let root = unique_temp_dir("snapshot-location")?;
        let data_path = root.join("data");
        let location =
            SnapshotLocation::for_data_path_and_generation(&data_path.display().to_string(), 42)?;

        assert_eq!(
            location.local_path,
            data_path
                .join("gold")
                .join("catalog-snapshot")
                .join("generations")
                .join("00000000000000000042")
                .join("spur_context.ducklake")
        );
        assert_eq!(
            location.local_manifest_path,
            data_path
                .join("gold")
                .join("catalog-snapshot")
                .join("generations")
                .join("00000000000000000042")
                .join("manifest.json")
        );
        assert_eq!(
            location.local_pointer_path,
            data_path
                .join("gold")
                .join("catalog-snapshot")
                .join("current.json")
        );
        assert_eq!(location.s3_uri, None);
        Ok(())
    }

    #[test]
    fn pointer_publish_refuses_incomplete_manifest_without_live_marker() -> Result<()> {
        let root = unique_temp_dir("snapshot-pointer-incomplete")?;
        let pointer_path = root.join("current.json");
        let manifest = FrozenSnapshotManifest {
            schema_version: 1,
            generation: 7,
            snapshot_uri: root.join("snapshot.ducklake").display().to_string(),
            data_path: root.join("data").display().to_string(),
            sha256: "abc123".to_owned(),
            bytes: 12,
            serving_registry_uri: None,
            serving_registry_sha256: None,
            serving_registry_bytes: None,
            status: "staging".to_owned(),
        };

        let err = publish_local_snapshot_pointer(&pointer_path, &manifest)
            .expect_err("staging manifests must not become the live pointer");

        assert!(
            format!("{err:#}").contains("published"),
            "unexpected error: {err:#}"
        );
        assert!(
            !pointer_path.exists(),
            "failed publishes must not leave a pointer"
        );
        Ok(())
    }

    #[test]
    fn rollback_rewrites_pointer_to_previous_published_generation() -> Result<()> {
        let root = unique_temp_dir("snapshot-pointer-rollback")?;
        let pointer_path = root.join("current.json");
        let first = FrozenSnapshotManifest::published(
            10,
            root.join("generations/10/spur_context.ducklake")
                .display()
                .to_string(),
            root.join("data").display().to_string(),
            "sha10".to_owned(),
            10,
        );
        let second = FrozenSnapshotManifest::published(
            11,
            root.join("generations/11/spur_context.ducklake")
                .display()
                .to_string(),
            root.join("data").display().to_string(),
            "sha11".to_owned(),
            11,
        );

        publish_local_snapshot_pointer(&pointer_path, &second)?;
        publish_local_snapshot_pointer(&pointer_path, &first)?;

        let current = read_local_snapshot_pointer(&pointer_path)?;
        assert_eq!(current.generation, 10);
        assert_eq!(current.snapshot_uri, first.snapshot_uri);
        assert_eq!(current.status, "published");
        Ok(())
    }

    #[test]
    fn export_does_not_move_live_pointer_to_older_generation() -> Result<()> {
        let root = unique_temp_dir("snapshot-pointer-monotonic")?;
        let data_path = root.join("data");
        let data_path = data_path.display().to_string();
        let older_location = SnapshotLocation::for_data_path_and_generation(&data_path, 10)?;
        let newer_location = SnapshotLocation::for_data_path_and_generation(&data_path, 11)?;
        seed_published_snapshot(&older_location, &data_path, 10)?;
        seed_published_snapshot(&newer_location, &data_path, 11)?;

        let newer_manifest = read_snapshot_manifest(&newer_location)?;
        publish_local_snapshot_pointer(&newer_location.local_pointer_path, &newer_manifest)?;

        export_frozen_snapshot("sqlite:/unused/catalog.sqlite", &data_path, 10)?;

        let current = read_local_snapshot_pointer(&newer_location.local_pointer_path)?;
        assert_eq!(current.generation, 11);
        assert_eq!(current.snapshot_uri, newer_manifest.snapshot_uri);
        Ok(())
    }

    fn unique_temp_dir(prefix: &str) -> Result<PathBuf> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before epoch")?
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{nanos}-{}", std::process::id()));
        fs::create_dir_all(&dir).with_context(|| format!("create temp dir `{}`", dir.display()))?;
        Ok(dir)
    }

    fn seed_published_snapshot(
        location: &SnapshotLocation,
        data_path: &str,
        generation: i64,
    ) -> Result<()> {
        if let Some(parent) = location.local_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create snapshot dir `{}`", parent.display()))?;
        }
        fs::write(
            &location.local_path,
            format!("snapshot generation {generation}"),
        )
        .with_context(|| format!("write snapshot `{}`", location.local_path.display()))?;
        let (sha256, bytes) = file_sha256_and_len(&location.local_path)?;
        let manifest = FrozenSnapshotManifest::published(
            generation,
            location.snapshot_uri.clone(),
            data_path.to_owned(),
            sha256,
            bytes,
        );
        publish_snapshot_manifest(location, &manifest)
    }

    #[test]
    fn postgres_catalog_dsn_uses_secret_password_env() {
        let _guard = lock_env();
        let previous = std::env::var_os("SPUR_CATALOG_PASSWORD");
        std::env::set_var("SPUR_CATALOG_PASSWORD", "sec'ret\\value");

        let dsn = catalog_dsn_with_env_password(
            "postgres:host=aurora.example port=5432 dbname=spur_context user=spur_context",
        );

        match previous {
            Some(value) => std::env::set_var("SPUR_CATALOG_PASSWORD", value),
            None => std::env::remove_var("SPUR_CATALOG_PASSWORD"),
        }

        assert_eq!(
            dsn,
            "postgres:host=aurora.example port=5432 dbname=spur_context user=spur_context password='sec\\'ret\\\\value'"
        );
    }

    #[test]
    fn postgres_catalog_dsn_keeps_existing_password() {
        let _guard = lock_env();
        let previous = std::env::var_os("SPUR_CATALOG_PASSWORD");
        std::env::set_var("SPUR_CATALOG_PASSWORD", "from-secret");

        let dsn = catalog_dsn_with_env_password("postgres:host=aurora password=already-present");

        match previous {
            Some(value) => std::env::set_var("SPUR_CATALOG_PASSWORD", value),
            None => std::env::remove_var("SPUR_CATALOG_PASSWORD"),
        }

        assert_eq!(dsn, "postgres:host=aurora password=already-present");
    }

    #[test]
    fn postgres_metadata_dsn_sets_pause_resume_connect_timeout() {
        // sol_33f7c9ded2f042c0: connect_timeout_s=30 covers Aurora resume-from-0 ACU.
        let dsn = postgres_metadata_dsn(
            "postgres:host=aurora.example port=5432 dbname=spur_context user=spur_context sslmode=require",
        );
        assert!(
            dsn.contains("connect_timeout=30"),
            "pause-resume attach DSN should wait 30s per attempt, got `{dsn}`"
        );
        assert!(dsn.contains("host=aurora.example"));
        assert!(dsn.contains("sslmode=require"));
    }

    #[test]
    fn postgres_metadata_dsn_keeps_existing_connect_timeout() {
        let dsn = postgres_metadata_dsn("postgres:host=aurora connect_timeout=12");
        assert!(dsn.contains("connect_timeout=12"));
        assert!(
            !dsn.contains("connect_timeout=30"),
            "explicit connect_timeout must not be overwritten, got `{dsn}`"
        );
    }

    #[test]
    fn ducklake_attach_uri_sets_pause_resume_connect_timeout() {
        // sol_1744e36a489a4b49 / sol_f9b97ca9d2a94eef: DuckLake ATTACH must
        // reuse the 30s libpq connect_timeout so bronze lookup fail-fasts
        // instead of hanging on an unguarded TCP timeout.
        let uri = ducklake_attach_uri(
            "postgres:host=aurora.example port=5432 dbname=spur_context user=spur_context sslmode=require",
        );
        assert!(
            uri.starts_with("ducklake:"),
            "DuckLake ATTACH URI must keep the ducklake: prefix, got `{uri}`"
        );
        assert!(
            uri.contains("connect_timeout=30"),
            "DuckLake postgres ATTACH must wait 30s per attempt, got `{uri}`"
        );
        assert!(uri.contains("host=aurora.example"));
        assert!(uri.contains("sslmode=require"));
    }

    #[test]
    fn ducklake_attach_uri_keeps_existing_connect_timeout() {
        let uri = ducklake_attach_uri("ducklake:postgres:host=aurora connect_timeout=12");
        assert!(uri.contains("connect_timeout=12"));
        assert!(
            !uri.contains("connect_timeout=30"),
            "explicit connect_timeout must not be overwritten, got `{uri}`"
        );
    }

    #[test]
    fn ducklake_attach_uri_leaves_sqlite_catalogs_unchanged() {
        let uri = ducklake_attach_uri("sqlite:/tmp/spur-context.db");
        assert_eq!(uri, "ducklake:sqlite:/tmp/spur-context.db");
        assert!(
            !uri.contains("connect_timeout"),
            "sqlite catalogs must not gain a postgres connect_timeout, got `{uri}`"
        );
    }

    #[test]
    fn attach_ducklake_retries_pause_resume_errors() {
        let source = include_str!("catalog.rs");
        let start = source
            .find("fn attach_ducklake(")
            .expect("catalog attach_ducklake must exist");
        let body = &source[start..];
        let end = body
            .find("\nfn attach_frozen_snapshot")
            .unwrap_or(body.len());
        let body = &body[..end];
        assert!(
            body.contains("ducklake_attach_uri"),
            "catalog DuckLake ATTACH must inject pause-resume connect_timeout via ducklake_attach_uri"
        );
        assert!(
            body.contains("retry_postgres_pause_resume"),
            "catalog DuckLake ATTACH must retry Aurora pause-resume errors"
        );
    }

    #[test]
    fn pause_resume_retry_succeeds_on_second_attempt() {
        let mut calls = 0u32;
        let mut sleeps = 0u32;
        let result = retry_postgres_pause_resume(
            || {
                calls += 1;
                if calls == 1 {
                    Err(anyhow!("connection timed out"))
                } else {
                    Ok(())
                }
            },
            |_| {
                sleeps += 1;
            },
        );
        assert!(result.is_ok());
        assert_eq!(calls, POSTGRES_PAUSE_RESUME_ATTEMPTS);
        assert_eq!(sleeps, POSTGRES_PAUSE_RESUME_ATTEMPTS - 1);
    }

    #[test]
    fn pause_resume_retry_exhausts_budgeted_attempts() {
        let mut calls = 0u32;
        let result: Result<()> = retry_postgres_pause_resume(
            || {
                calls += 1;
                Err(anyhow!("could not connect to server"))
            },
            |_| {},
        );
        assert!(result.is_err());
        assert_eq!(calls, POSTGRES_PAUSE_RESUME_ATTEMPTS);
    }

    #[test]
    fn pause_resume_retry_does_not_retry_permanent_errors() {
        let mut calls = 0u32;
        let result: Result<()> = retry_postgres_pause_resume(
            || {
                calls += 1;
                Err(anyhow!("password authentication failed"))
            },
            |_| {},
        );
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn attach_error_redacts_libpq_password() {
        let redacted = redact_libpq_secrets(
            "Unable to connect to Postgres at \"host=aurora password='s3cret\\\\value' dbname=spur_context\"",
        );
        assert!(!redacted.contains("s3cret"));
        assert!(redacted.contains("password=REDACTED"));
    }

    #[test]
    fn s3_status_classifiers_use_modeled_codes_and_raw_statuses() {
        assert!(is_s3_get_not_found(Some("NoSuchKey"), None));
        assert!(is_s3_get_not_found(None, Some(404)));
        assert!(!is_s3_get_not_found(Some("AccessDenied"), Some(403)));
        assert!(!is_s3_get_not_found(Some("SlowDown"), Some(503)));

        for status in [404, 409, 412] {
            assert!(is_s3_conditional_write_stale(None, Some(status)));
        }
        assert!(is_s3_conditional_write_stale(
            Some("PreconditionFailed"),
            None
        ));
        assert!(!is_s3_conditional_write_stale(
            Some("AccessDenied"),
            Some(403)
        ));
    }

    #[test]
    fn duckdb_extension_loading_uses_env_directory_offline() {
        let _guard = lock_env();
        let previous = std::env::var_os(DUCKDB_EXTENSION_DIR_ENV);
        std::env::set_var(
            DUCKDB_EXTENSION_DIR_ENV,
            "/opt/duckdb/extensions/with ' quote",
        );

        let ducklake_sql = duckdb_extension_load_sql("ducklake");
        let httpfs_sql = duckdb_extension_load_sql("httpfs");

        match previous {
            Some(value) => std::env::set_var(DUCKDB_EXTENSION_DIR_ENV, value),
            None => std::env::remove_var(DUCKDB_EXTENSION_DIR_ENV),
        }

        assert_eq!(
            ducklake_sql,
            "SET extension_directory = '/opt/duckdb/extensions/with '' quote'; \
             SET autoinstall_known_extensions = false; \
             LOAD ducklake;"
        );
        assert_eq!(
            httpfs_sql,
            "SET extension_directory = '/opt/duckdb/extensions/with '' quote'; \
             SET autoinstall_known_extensions = false; \
             LOAD httpfs;"
        );
    }

    #[test]
    fn duckdb_extension_loading_preserves_home_directory_flow_when_env_unset() {
        let _guard = lock_env();
        let previous = std::env::var_os(DUCKDB_EXTENSION_DIR_ENV);
        std::env::remove_var(DUCKDB_EXTENSION_DIR_ENV);

        let sql = duckdb_extension_load_sql("httpfs");

        match previous {
            Some(value) => std::env::set_var(DUCKDB_EXTENSION_DIR_ENV, value),
            None => std::env::remove_var(DUCKDB_EXTENSION_DIR_ENV),
        }

        assert_eq!(sql, "INSTALL httpfs; LOAD httpfs;");
    }
}
