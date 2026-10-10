//! Durable issue-read tracking, independent of the graph clock.
//!
//! `read` supplies the watermark, changed IDs and a SQLite read transaction to
//! one callback. Load summaries/comments in that transaction and associate the
//! result with *that* cursor. A later commit is a later snapshot: consumers must
//! recheck before claiming an in-flight result is current (S5), and retain old
//! membership as well as testing new membership. This feed is not a result cache.
//!
//! One journal row per ID retains the latest issue, summary, timestamp and comment/audit
//! revisions independently. IDs of deleted issues are never pruned. Therefore
//! every cursor in the same generation at/above `floor` has complete changed-ID
//! coverage, even after coalescing and even for lagging independent readers.
//! `deleted` describes physical absence in the snapshot; tombstones remain rows.
//! `summary` covers payload, ordering and membership except the `since` filter.
//! `timestamp` independently covers `updated_at`: queries with `since` must treat
//! `summary || timestamp` as candidates (or use direct SQL). Without `since`, an
//! ordinary adapter comment is unrelated to summaries, even though upstream also
//! updates `updated_at`. Hygiene still observes `issue` and `comments`.
//! Structural parent-child edges mark OLD/NEW child IDs as hygiene-only issue
//! changes; pure edge edits never change summary or timestamp revisions.
//!
//! Recovery: validate schema cookies, exact owned DDL, clock integrity and journal
//! head before reuse. Exact DDL validation is reused only for the same physical
//! connection and schema cookie; clock/journal checks always run. Direct journal
//! edits set `dirty` via protection triggers.
//! Missing/altered objects, a dirty/corrupt clock or schema drift cause an atomic
//! reinstall with a new UUID generation and history floor. Every old cursor then
//! requires reseeding from SQL; an empty delta is never substituted for a gap.
//! A surviving valid revision is advanced on repair; loss of the clock starts a
//! new epoch at zero. Revisions are monotonic *within* an epoch, not comparable
//! across epochs. Integer exhaustion fails the write rather than wrapping.
//!
//! The persistent reader also checks physical database identity before/after the
//! transaction, including replacement by a clone retaining the UUID. Unix uses
//! device/inode. Platforms lacking this identity primitive reject feed reads;
//! they must use direct SQL until an equivalent identity implementation exists.
//! No timestamps or cross-connection data_version tokens are used. DDL cookies
//! and protection triggers detect ordinary external SQL drift/corruption, not a
//! hostile database administrator deliberately forging both data and metadata.
//! SQLite corruption/I/O/locking errors propagate; they never authorize reuse.
//! An external writer that dropped the journal can still fail with "no such
//! table" AFTER another connection repairs it. That rejected statement leaves
//! rows unchanged. On that same writer, execute a schema-reading query such as
//! `SELECT name FROM sqlite_schema WHERE type='table' AND name='spur_issue_changes'`
//! before retrying the rejected statement; reconnecting also refreshes the cache.
//! `SELECT 1` and `PRAGMA schema_version` alone do not refresh it. The feed cannot
//! repair another connection's schema cache. No failed write is silently retried.
//!
//! This synchronous crate-local API must run on a blocking thread. S5/S6 own
//! async dispatch, result publication, subscription bounds and cursor persistence.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{ensure, Context};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 3;
const CLOCK: &str = "spur_issue_clock";
const JOURNAL: &str = "spur_issue_changes";
const SUMMARY_FIELDS: &[&str] = &[
    "id",
    "title",
    "description",
    "status",
    "priority",
    "issue_type",
    "assignee",
    "created_at",
    "is_template",
];

// Shared with query-plan/work-count regressions; these are the executed queries.
const CLOCK_QUERY: &str = "SELECT generation,revision,floor,schema_cookie FROM spur_issue_clock
         WHERE id=1 AND version=?2 AND dirty=0 AND writing=0
           AND typeof(revision)='integer' AND revision>=0
           AND typeof(floor)='integer' AND floor>=0 AND floor<=revision
           AND schema_cookie=?1
           AND revision=MAX(floor,COALESCE((SELECT revision FROM spur_issue_changes
                                          ORDER BY revision DESC LIMIT 1),0))";
const CHANGES_QUERY: &str = "SELECT issue_id,issue_revision>?1,summary_revision>?1,timestamp_revision>?1,comment_revision>?1,
                NOT EXISTS(SELECT 1 FROM issues WHERE issues.id=spur_issue_changes.issue_id)
         FROM spur_issue_changes WHERE revision>?1 ORDER BY revision,issue_id";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IssueCursor {
    pub(crate) version: u32,
    pub(crate) generation: String,
    pub(crate) revision: i64,
    pub(crate) schema: i64,
    identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueChange {
    pub(crate) issue_id: String,
    /// Any issue-column, label or structural-parent mutation, including hygiene data.
    pub(crate) issue: bool,
    /// Summary payload/order/membership, excluding the `since` timestamp filter.
    pub(crate) summary: bool,
    /// `updated_at` or issue insertion/removal/ID replacement. Since-filtered
    /// summaries must consider this in addition to `summary` (or use direct SQL).
    pub(crate) timestamp: bool,
    /// Comments and audit events; also row insertion/removal/ID replacement.
    pub(crate) comments: bool,
    pub(crate) deleted: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct IssueChanges {
    pub(crate) cursor: IssueCursor,
    /// The caller must seed its entire relevant result in the supplied snapshot.
    pub(crate) reseed: bool,
    /// Empty on reseed. Otherwise complete affected IDs since the supplied cursor.
    pub(crate) changes: Vec<IssueChange>,
}

pub(crate) fn install_at_path(path: &Path, timeout_ms: u64) -> anyhow::Result<()> {
    let mut conn = open(path, timeout_ms)?;
    install(&mut conn)
}

/// Installation/repair is serialized with writers. Healthy installs are read-only.
pub(crate) fn install(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    if healthy(&tx)?.is_some() {
        tx.commit()?;
        return Ok(());
    }
    drop(tx);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another process may already have repaired it while we acquired the lock.
    if healthy(&tx)?.is_none() {
        rebuild(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) struct IssueChangeFeed {
    path: PathBuf,
    timeout_ms: u64,
    connected: Option<(String, Connection)>,
    // Exact DDL depends only on this connection's schema snapshot. Clock and
    // journal integrity are still checked in EVERY transaction.
    validated_schema: Option<i64>,
    pub(crate) connection_opens: u64,
}

impl IssueChangeFeed {
    pub(crate) fn new(path: &Path, timeout_ms: u64) -> Self {
        Self {
            path: path.into(),
            timeout_ms,
            connected: None,
            validated_schema: None,
            connection_opens: 0,
        }
    }

    fn connect(&mut self) -> anyhow::Result<String> {
        let identity = file_identity(&self.path)?;
        if self
            .connected
            .as_ref()
            .is_none_or(|(id, _)| *id != identity)
        {
            // Both tracked and direct reads must abandon an old inode.
            self.connected = None;
            self.validated_schema = None;
            let conn = open(&self.path, self.timeout_ms)?;
            self.connection_opens += 1;
            ensure!(
                same_file(&self.path, &identity)?,
                "issue database replaced while connecting"
            );
            self.connected = Some((identity.clone(), conn));
        }
        Ok(identity)
    }

    /// Replacement-safe S3 fallback. Does not inspect/repair the feed: source
    /// schema/query errors must come from the original summary query. It never
    /// yields a cursor and therefore cannot authorize subscription reuse.
    pub(crate) fn read_direct<T>(
        &mut self,
        read: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        #[cfg(unix)]
        {
            let identity = self.connect()?;
            let (_, conn) = self.connected.as_mut().expect("connected above");
            let tx = conn.transaction()?;
            let result = read(&tx)?;
            tx.commit()?;
            ensure!(
                same_file(&self.path, &identity)?,
                "issue database replaced during snapshot; discard result"
            );
            Ok(result)
        }
        #[cfg(not(unix))]
        {
            // No trustworthy physical identity on this platform: never retain
            // a connection or subscription across direct calls.
            let mut conn = open(&self.path, self.timeout_ms)?;
            self.connection_opens += 1;
            let tx = conn.transaction()?;
            let result = read(&tx)?;
            tx.commit()?;
            Ok(result)
        }
    }

    /// Cursor and delta are read atomically with all callback SQL. Callback errors
    /// or replacement during the callback return an error, never a publishable result.
    pub(crate) fn read<T>(
        &mut self,
        since: Option<&IssueCursor>,
        read: impl FnOnce(&Transaction<'_>, &IssueChanges) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        loop {
            let identity = self.connect()?;
            let (_, conn) = self.connected.as_mut().expect("connected above");
            let tx = conn.transaction()?;
            let Some(clock) = healthy_with_schema(&tx, &mut self.validated_schema)? else {
                self.validated_schema = None;
                drop(tx);
                install(conn)?;
                continue;
            };
            let cursor = IssueCursor {
                version: VERSION,
                generation: clock.generation,
                revision: clock.revision,
                schema: clock.schema,
                identity: identity.clone(),
            };
            let reseed = since.is_none_or(|old| {
                old.version != cursor.version
                    || old.generation != cursor.generation
                    || old.schema != cursor.schema
                    || old.identity != cursor.identity
                    || old.revision < clock.floor
                    || old.revision > cursor.revision
            });
            // Complete history plus an equal validated watermark proves that
            // there are no changed IDs. Keep the integrity check above even on
            // this path: an empty query cannot substitute for a damaged feed.
            let changes = if reseed || since.is_some_and(|old| old.revision == cursor.revision) {
                Vec::new()
            } else {
                changed_ids(&tx, since.expect("checked above").revision)?
            };
            let batch = IssueChanges {
                cursor,
                reseed,
                changes,
            };
            let result = read(&tx, &batch)?;
            tx.commit()?;
            ensure!(
                same_file(&self.path, &identity)?,
                "issue database replaced during snapshot; discard result"
            );
            return Ok(result);
        }
    }
}

fn open(path: &Path, timeout_ms: u64) -> anyhow::Result<Connection> {
    // Never manufacture a missing database on the read/recovery path.
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(Duration::from_millis(timeout_ms))?;
    Ok(conn)
}

#[cfg(unix)]
fn file_identity(path: &Path) -> anyhow::Result<String> {
    use std::os::unix::fs::MetadataExt;
    let m = path.metadata()?;
    Ok(format!("{}:{}", m.dev(), m.ino()))
}
#[cfg(not(unix))]
fn file_identity(path: &Path) -> anyhow::Result<String> {
    let _ = path;
    anyhow::bail!("issue feed requires a supported physical database identity")
}
fn same_file(path: &Path, identity: &str) -> anyhow::Result<bool> {
    #[cfg(unix)]
    {
        Ok(file_identity(path)? == identity)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, identity);
        anyhow::bail!("issue feed requires a supported physical database identity")
    }
}

struct Clock {
    generation: String,
    revision: i64,
    floor: i64,
    schema: i64,
}

fn healthy(tx: &Transaction<'_>) -> anyhow::Result<Option<Clock>> {
    healthy_with_schema(tx, &mut None)
}

fn healthy_with_schema(
    tx: &Transaction<'_>,
    validated_schema: &mut Option<i64>,
) -> anyhow::Result<Option<Clock>> {
    let schema: i64 = tx.query_row("PRAGMA schema_version", [], |r| r.get(0))?;
    if *validated_schema != Some(schema) {
        tracing::debug!(target: "issue_probe", site = "beads_issue_schema_validate", "validate exact issue-feed DDL");
        let expected = objects(tx)?;
        let actual = owned_objects(tx)?;
        if expected.len() != actual.len()
            || expected
                .iter()
                .any(|(name, sql)| actual.get(name).is_none_or(|(_, stored)| stored != sql))
        {
            return Ok(None);
        }
        *validated_schema = Some(schema);
    }
    // One fixed statement key on this connection; no dynamic query diversity
    // or new capacity setting. SQLite revalidates cached bytecode on DDL changes.
    let clock = tx
        .prepare_cached(CLOCK_QUERY)?
        .query_row(rusqlite::params![schema, VERSION], |r| {
            Ok(Clock {
                generation: r.get(0)?,
                revision: r.get(1)?,
                floor: r.get(2)?,
                schema: r.get(3)?,
            })
        })
        .optional();
    match clock {
        Ok(Some(clock)) if uuid::Uuid::parse_str(&clock.generation).is_ok() => Ok(Some(clock)),
        Ok(_) | Err(rusqlite::Error::InvalidColumnType(..)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn changed_ids(tx: &Transaction<'_>, since: i64) -> anyhow::Result<Vec<IssueChange>> {
    tracing::debug!(target: "issue_probe", site = "beads_issue_delta_query", "query changed issue IDs");
    let mut stmt = tx.prepare(CHANGES_QUERY)?;
    let changes = stmt
        .query_map([since], |r| {
            Ok(IssueChange {
                issue_id: r.get(0)?,
                issue: r.get(1)?,
                summary: r.get(2)?,
                timestamp: r.get(3)?,
                comments: r.get(4)?,
                deleted: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(changes)
}

fn rebuild(tx: &Transaction<'_>) -> anyhow::Result<()> {
    let objects = objects(tx)?;
    // Preserve a trustworthy numeric high-water value when it survives. A UUID
    // change is authoritative when the old clock cannot be recovered.
    let previous = tx.query_row(
        "SELECT revision FROM spur_issue_clock WHERE id=1 AND typeof(revision)='integer' AND revision>=0",
        [], |r| r.get::<_, i64>(0),
    ).optional();
    let revision = match previous {
        Ok(Some(n)) => n.checked_add(1).context("issue watermark exhausted")?,
        Ok(None) => 0,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::Unknown =>
        {
            0
        }
        Err(e) => return Err(e.into()),
    };
    // Only our namespace is owned here. Do not change graph tracking or upstream DDL.
    for (name, (kind, _)) in owned_objects(tx)? {
        ensure!(
            ["table", "index", "trigger", "view"].contains(&kind.as_str()),
            "unknown feed object type"
        );
        tx.execute_batch(&format!("DROP {kind} IF EXISTS {}", quote(&name)))?;
    }
    // Tables before indexes/triggers, regardless of name order.
    for table in [CLOCK, JOURNAL] {
        tx.execute_batch(&objects[table])?;
    }
    for (name, sql) in &objects {
        if name != CLOCK && name != JOURNAL {
            tx.execute_batch(sql)?;
        }
    }
    let schema: i64 = tx.query_row("PRAGMA schema_version", [], |r| r.get(0))?;
    tx.execute(
        "INSERT INTO spur_issue_clock(id,version,generation,revision,floor,schema_cookie,writing,dirty)
         VALUES(1,?1,?2,?3,?3,?4,0,0)",
        rusqlite::params![VERSION, uuid::Uuid::new_v4().to_string(), revision, schema],
    )?;
    Ok(())
}

fn owned_objects(conn: &Connection) -> anyhow::Result<BTreeMap<String, (String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT name,type,sql FROM sqlite_schema WHERE name GLOB 'spur_issue_*' AND sql IS NOT NULL",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn columns(conn: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let fields = stmt
        .query_map([table], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ensure!(
        !fields.is_empty(),
        "missing issue-feed source table {table}"
    );
    Ok(fields)
}
fn changed(fields: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    fields
        .into_iter()
        .map(|f| {
            let q = quote(f.as_ref());
            format!("OLD.{q} IS NOT NEW.{q}")
        })
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Known uniqueness victims are handled below. New/expression/collated unique
/// constraints conservatively dirty the feed on inserts/updates until supported;
/// writes still succeed and the next reader reseeds, never silently misses victims.
fn supported_uniqueness(conn: &Connection, table: &str) -> anyhow::Result<bool> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_index_list(?1) WHERE \"unique\"=1")?;
    let indices = stmt
        .query_map([table], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for index in indices {
        let mut stmt = conn
            .prepare("SELECT name,coll FROM pragma_index_xinfo(?1) WHERE key=1 ORDER BY seqno")?;
        let fields = stmt
            .query_map([index], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if fields
            .iter()
            .any(|(name, coll)| name.is_none() || coll != "BINARY")
        {
            return Ok(false);
        }
        let names = fields
            .iter()
            .map(|(name, _)| name.as_deref().unwrap())
            .collect::<Vec<_>>();
        let supported = match table {
            "issues" => names == ["id"] || names == ["external_ref"],
            "labels" => names == ["issue_id", "label"],
            "dependencies" => names == ["issue_id", "depends_on_id"],
            _ => names == ["id"],
        };
        if !supported {
            return Ok(false);
        }
    }
    Ok(true)
}

fn objects(conn: &Connection) -> anyhow::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    out.insert(CLOCK.into(), "CREATE TABLE spur_issue_clock (
        id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL,
        generation TEXT NOT NULL, revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0),
        floor INTEGER NOT NULL CHECK(typeof(floor)='integer' AND floor>=0 AND floor<=revision),
        schema_cookie INTEGER NOT NULL, writing INTEGER NOT NULL CHECK(writing IN(0,1)),
        dirty INTEGER NOT NULL CHECK(dirty IN(0,1)))".into());
    out.insert(JOURNAL.into(), "CREATE TABLE spur_issue_changes (
        issue_id TEXT PRIMARY KEY NOT NULL,
        revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
        issue_revision INTEGER NOT NULL CHECK(typeof(issue_revision)='integer' AND issue_revision>=0 AND issue_revision<=revision),
        summary_revision INTEGER NOT NULL CHECK(typeof(summary_revision)='integer' AND summary_revision>=0 AND summary_revision<=revision),
        timestamp_revision INTEGER NOT NULL CHECK(typeof(timestamp_revision)='integer' AND timestamp_revision>=0 AND timestamp_revision<=revision),
        comment_revision INTEGER NOT NULL CHECK(typeof(comment_revision)='integer' AND comment_revision>=0 AND comment_revision<=revision))".into());
    out.insert(
        "spur_issue_changes_revision".into(),
        "CREATE INDEX spur_issue_changes_revision ON spur_issue_changes(revision)".into(),
    );
    for operation in ["insert", "update", "delete"] {
        let name = format!("spur_issue_guard_{operation}");
        out.insert(
            name.clone(),
            format!(
                "CREATE TRIGGER {name} AFTER {operation} ON spur_issue_changes
            WHEN (SELECT writing FROM spur_issue_clock WHERE id=1)=0
            BEGIN UPDATE spur_issue_clock SET dirty=1 WHERE id=1; END"
            ),
        );
    }
    out.insert("spur_issue_clock_guard".into(), "CREATE TRIGGER spur_issue_clock_guard AFTER UPDATE ON spur_issue_clock
        WHEN NEW.generation IS NOT OLD.generation OR NEW.version IS NOT OLD.version
          OR NEW.floor IS NOT OLD.floor OR NEW.schema_cookie IS NOT OLD.schema_cookie
          OR NEW.dirty<OLD.dirty
          OR (NEW.revision IS NOT OLD.revision AND NOT(OLD.writing=0 AND NEW.writing=1 AND NEW.revision=OLD.revision+1))
        BEGIN UPDATE spur_issue_clock SET dirty=1 WHERE id=1; END".into());

    for table in ["issues", "labels", "comments", "events"] {
        let fields = columns(conn, table)?;
        let all_changed = changed(&fields);
        let summary_changed = changed(SUMMARY_FIELDS);
        let known_unique = supported_uniqueness(conn, table)?;
        for operation in ["insert", "update", "delete"] {
            let id = if table == "issues" { "id" } else { "issue_id" };
            let ids = match operation {
                "insert" => format!("SELECT NEW.{id} AS changed_id"),
                "delete" => format!("SELECT OLD.{id} AS changed_id"),
                _ => format!("SELECT OLD.{id} AS changed_id UNION SELECT NEW.{id}"),
            };
            let (issue, summary, timestamp, comments) = match table {
                "issues" => (
                    "1",
                    if operation == "update" {
                        summary_changed.as_str()
                    } else {
                        "1"
                    },
                    if operation == "update" {
                        "OLD.updated_at IS NOT NEW.updated_at OR OLD.id IS NOT NEW.id"
                    } else {
                        "1"
                    },
                    if operation == "update" {
                        "OLD.id IS NOT NEW.id"
                    } else {
                        "1"
                    },
                ),
                "labels" => ("1", "1", "0", "0"),
                _ => ("0", "0", "0", "1"),
            };
            let guard = if operation == "update" {
                format!(" WHEN {all_changed}")
            } else {
                String::new()
            };
            let dirty = if !known_unique && operation != "delete" {
                "UPDATE spur_issue_clock SET dirty=1 WHERE id=1;"
            } else {
                ""
            };
            let body = mark(&ids, issue, summary, timestamp, comments);
            let name = format!("spur_issue_{table}_{operation}");
            out.insert(name.clone(), format!("CREATE TRIGGER {name} AFTER {operation} ON {table}{guard} BEGIN {body} {dirty} END"));
        }
        // With recursive_triggers=OFF REPLACE deletes do not fire DELETE triggers.
        // Capture victims before either INSERT OR REPLACE or UPDATE OR REPLACE.
        // ABORT/ROLLBACK undo capture; IGNORE/FAIL may conservatively mark IDs.
        let (victim_id, collision) = match table {
            "issues" => (
                "id",
                "rowid=NEW.rowid OR id=NEW.id OR external_ref=NEW.external_ref",
            ),
            "labels" => (
                "issue_id",
                "rowid=NEW.rowid OR (issue_id=NEW.issue_id AND label=NEW.label)",
            ),
            _ => ("issue_id", "id=NEW.id"),
        };
        for operation in ["insert", "update"] {
            let exclude_self = if operation == "update" {
                " AND rowid<>OLD.rowid"
            } else {
                ""
            };
            let victims = format!(
                "SELECT {victim_id} AS changed_id FROM {table} WHERE ({collision}){exclude_self}"
            );
            let issue = if table == "issues" || table == "labels" {
                "1"
            } else {
                "0"
            };
            let comments = if table == "labels" { "0" } else { "1" };
            let timestamp = if table == "issues" { "1" } else { "0" };
            let body = mark(&victims, issue, issue, timestamp, comments);
            let name = format!("spur_issue_{table}_replace_{operation}");
            out.insert(
                name.clone(),
                format!(
                    "CREATE TRIGGER {name} BEFORE {operation} ON {table}
                WHEN EXISTS({victims}) BEGIN {body} END"
                ),
            );
        }
    }
    dependency_triggers(conn, &mut out)?;
    Ok(out)
}

/// Only structural ownership affects plan donation. Capture BOTH children on
/// retarget/type transitions, including REPLACE victims when recursive triggers
/// are disabled. Pure edges never invalidate plain summaries or timestamps.
/// Normalize edge types like DependencyType::from_str, including external SQL.
fn dependency_triggers(
    conn: &Connection,
    out: &mut BTreeMap<String, String>,
) -> anyhow::Result<()> {
    columns(conn, "dependencies")?;
    let known_unique = supported_uniqueness(conn, "dependencies")?;
    for operation in ["insert", "update", "delete"] {
        let ids = match operation {
            "insert" => "SELECT NEW.issue_id AS changed_id WHERE LOWER(NEW.type)='parent-child'",
            "delete" => "SELECT OLD.issue_id AS changed_id WHERE LOWER(OLD.type)='parent-child'",
            _ => "SELECT OLD.issue_id AS changed_id WHERE LOWER(OLD.type)='parent-child' OR LOWER(NEW.type)='parent-child' UNION SELECT NEW.issue_id WHERE LOWER(OLD.type)='parent-child' OR LOWER(NEW.type)='parent-child'",
        };
        let changed = if operation == "update" {
            " AND (OLD.issue_id IS NOT NEW.issue_id OR OLD.depends_on_id IS NOT NEW.depends_on_id OR OLD.type IS NOT NEW.type)"
        } else {
            ""
        };
        let body = mark(ids, "1", "0", "0", "0");
        let name = format!("spur_issue_dependencies_{operation}");
        out.insert(name.clone(), format!("CREATE TRIGGER {name} AFTER {operation} ON dependencies WHEN EXISTS({ids}){changed} BEGIN {body} END"));
    }
    for operation in ["insert", "update"] {
        let exclude_self = if operation == "update" {
            " AND rowid<>OLD.rowid"
        } else {
            ""
        };
        let victims = format!("SELECT issue_id AS changed_id FROM dependencies WHERE LOWER(type)='parent-child' AND (rowid=NEW.rowid OR (issue_id=NEW.issue_id AND depends_on_id=NEW.depends_on_id)){exclude_self}");
        let body = mark(&victims, "1", "0", "0", "0");
        let name = format!("spur_issue_dependencies_replace_{operation}");
        out.insert(name.clone(), format!("CREATE TRIGGER {name} BEFORE {operation} ON dependencies WHEN EXISTS({victims}) BEGIN {body} END"));
        if !known_unique {
            // Unknown constraints can replace a structural victim even when NEW
            // is a blocker. Dirty the feed for every insert/update in that case.
            let name = format!("spur_issue_dependencies_unknown_{operation}");
            out.insert(name.clone(), format!("CREATE TRIGGER {name} BEFORE {operation} ON dependencies BEGIN UPDATE spur_issue_clock SET dirty=1 WHERE id=1; END"));
        }
    }
    Ok(())
}

fn mark(ids: &str, issue: &str, summary: &str, timestamp: &str, comments: &str) -> String {
    let revision = "(SELECT revision FROM spur_issue_clock WHERE id=1)";
    format!("UPDATE spur_issue_clock SET writing=1,revision=revision+1 WHERE id=1;
        INSERT INTO spur_issue_changes(issue_id,revision,issue_revision,summary_revision,timestamp_revision,comment_revision)
        SELECT changed_id,{revision},CASE WHEN {issue} THEN {revision} ELSE 0 END,
          CASE WHEN {summary} THEN {revision} ELSE 0 END,CASE WHEN {timestamp} THEN {revision} ELSE 0 END,CASE WHEN {comments} THEN {revision} ELSE 0 END
        FROM ({ids}) WHERE changed_id IS NOT NULL
        ON CONFLICT(issue_id) DO UPDATE SET revision=excluded.revision,
          issue_revision=MAX(spur_issue_changes.issue_revision,excluded.issue_revision),
          summary_revision=MAX(spur_issue_changes.summary_revision,excluded.summary_revision),
          timestamp_revision=MAX(spur_issue_changes.timestamp_revision,excluded.timestamp_revision),
          comment_revision=MAX(spur_issue_changes.comment_revision,excluded.comment_revision);
        UPDATE spur_issue_clock SET writing=0 WHERE id=1;")
}

#[cfg(test)]
#[path = "issue_changes_tests.rs"]
mod tests;
