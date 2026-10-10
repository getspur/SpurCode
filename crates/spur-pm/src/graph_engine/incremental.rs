//! Retained graph views refreshed from transactional, relevant-row deltas.
//! Reports borrow the view while the engine mutex is held; no graph-wide clone.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use beads_rust::model::{DependencyType, IssueType, Status};
use chrono::{DateTime, NaiveDateTime, Utc};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::change_tracking;
use super::snapshot::{dependency_kind_from_beads, DependencyKind, GraphSnapshot, NodeData};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GraphWorkMetrics {
    pub full_builds: u64,
    pub delta_refreshes: u64,
    pub reuses: u64,
    pub recoveries: u64,
    pub changed_ids: u64,
    pub nodes_loaded: u64,
    pub edges_loaded: u64,
}

pub(super) struct IncrementalGraphStore {
    path: PathBuf,
    timeout_ms: u64,
    connected: Option<Connected>,
    pub(super) metrics: GraphWorkMetrics,
}

struct Connected {
    conn: Connection,
    identity: FileIdentity,
    schema: i64,
    scopes: HashMap<Option<String>, Scope>,
}

#[derive(PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created: Option<std::time::SystemTime>,
}

impl FileIdentity {
    fn read(path: &std::path::Path) -> std::io::Result<Self> {
        let metadata = path.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                created: metadata.created().ok(),
            })
        }
    }
}

struct Scope {
    snapshot: GraphSnapshot,
    revision: i64,
    fingerprint: [u8; 32],
}

#[derive(Default)]
struct Delta {
    nodes: Vec<(String, Option<NodeData>)>,
    edge_ids: HashSet<String>,
    edges: HashMap<(String, String), DependencyKind>,
    changed_ids: u64,
    nodes_loaded: u64,
    edges_loaded: u64,
}

impl IncrementalGraphStore {
    pub(super) fn new(path: PathBuf, timeout_ms: u64) -> Self {
        Self {
            path,
            timeout_ms,
            connected: None,
            metrics: GraphWorkMetrics::default(),
        }
    }

    pub(super) fn query<T>(
        &mut self,
        label: Option<&str>,
        compute: impl FnOnce(&GraphSnapshot) -> T,
    ) -> anyhow::Result<T> {
        loop {
            self.connect()?;
            let connected = self.connected.as_mut().expect("connected above");
            let tx = connected.conn.transaction()?;
            let schema: i64 = tx.query_row("PRAGMA schema_version", [], |r| r.get(0))?;
            if schema != connected.schema {
                // Trigger/table replacement can lose history. Reinstall tracking
                // and initialize once from a coherent new view, never serve stale.
                drop(tx);
                self.connected = None;
                self.metrics.recoveries += 1;
                continue;
            }
            let revision: i64 = tx.query_row(
                "SELECT revision FROM spur_graph_clock WHERE id=1",
                [],
                |r| r.get(0),
            )?;
            let key = label.map(str::to_owned);
            let existing = connected.scopes.get(&key);
            if existing.is_some_and(|s| s.revision > revision) {
                drop(tx);
                self.connected = None;
                self.metrics.recoveries += 1;
                continue;
            }
            let cold = existing.is_none();
            let delta = match existing {
                Some(scope) if scope.revision == revision => None,
                Some(scope) => Some(load_delta(&tx, scope, label)?),
                None => Some(load_initial(&tx, label)?),
            };
            tx.commit()?;

            // All fallible DB work finished. Apply only staged affected records.
            let scope = connected
                .scopes
                .entry(key.clone())
                .or_insert_with(|| Scope {
                    snapshot: GraphSnapshot::new(key.clone()),
                    revision: 0,
                    fingerprint: [0; 32],
                });
            if let Some(delta) = delta {
                self.metrics.changed_ids += delta.changed_ids;
                self.metrics.nodes_loaded += delta.nodes_loaded;
                self.metrics.edges_loaded += delta.edges_loaded;
                if cold {
                    self.metrics.full_builds += 1;
                } else {
                    self.metrics.delta_refreshes += 1;
                }
                scope.apply(delta);
            } else {
                self.metrics.reuses += 1;
            }
            scope.revision = revision;
            // Data retention must not freeze time-sensitive report timestamps.
            scope.snapshot.generated_at = Utc::now();
            let result = compute(&scope.snapshot);
            // Unknown label strings must not create unbounded empty cache entries.
            if key.is_some() && scope.snapshot.node_count() == 0 {
                connected.scopes.remove(&key);
            }
            tracing::debug!(target: "graph_work", ?label, revision, ?self.metrics,
                "incremental graph query");
            return Ok(result);
        }
    }

    fn connect(&mut self) -> anyhow::Result<()> {
        let identity = FileIdentity::read(&self.path)?;
        if self
            .connected
            .as_ref()
            .is_some_and(|c| c.identity != identity)
        {
            self.connected = None;
            self.metrics.recoveries += 1;
        }
        if self.connected.is_none() {
            // The adapter owns schema creation. Never create a missing database
            // silently from the read path.
            let mut conn =
                Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            conn.busy_timeout(Duration::from_millis(self.timeout_ms))?;
            change_tracking::install(&mut conn)?;
            let schema = conn.query_row("PRAGMA schema_version", [], |r| r.get(0))?;
            self.connected = Some(Connected {
                conn,
                identity,
                schema,
                scopes: HashMap::new(),
            });
        }
        Ok(())
    }
}

fn load_initial(conn: &Connection, label: Option<&str>) -> anyhow::Result<Delta> {
    let mut delta = Delta::default();
    let sql = initial_nodes_sql(label);
    let nodes = conn
        .prepare(&sql)?
        .query_map([label], read_node)?
        .collect::<Result<Vec<_>, _>>()?;
    for mut node in nodes {
        node.labels = load_labels(conn, &node.id)?;
        delta.nodes_loaded += 1;
        delta.nodes.push((node.id.clone(), Some(node)));
    }
    let mut stmt = conn.prepare(&initial_edges_sql(label))?;
    let members: HashSet<&str> = delta.nodes.iter().map(|(id, _)| id.as_str()).collect();
    let rows = stmt.query_map([label], |row| read_edge(row, |id| members.contains(id)))?;
    for row in rows {
        if let Some((from, to, kind)) = row? {
            delta.edges_loaded += 1;
            delta.edges.insert((from, to), kind);
        }
    }
    Ok(delta)
}

fn initial_nodes_sql(label: Option<&str>) -> String {
    let filter = if label.is_some() {
        "id IN (SELECT issue_id FROM labels WHERE label=?1)"
    } else {
        "?1 IS NULL"
    };
    format!("{NODE_SELECT} AND {filter} ORDER BY priority ASC, created_at DESC")
}

fn initial_edges_sql(label: Option<&str>) -> String {
    let filter = if label.is_some() {
        "issue_id IN (SELECT issue_id FROM labels WHERE label=?1)"
    } else {
        "?1 IS NULL"
    };
    format!("SELECT depends_on_id,issue_id,type FROM dependencies WHERE {filter}")
}

fn load_delta(conn: &Connection, scope: &Scope, label: Option<&str>) -> anyhow::Result<Delta> {
    let mut delta = Delta::default();
    let mut stmt = conn.prepare(
        "SELECT issue_id,node_revision>?1,edge_revision>?1
         FROM spur_graph_changes WHERE revision>?1 ORDER BY revision,issue_id",
    )?;
    let changes = stmt
        .query_map([scope.revision], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    delta.changed_ids = changes.len() as u64;
    for (id, node_changed, edges_changed) in changes {
        let was_present = scope.snapshot.by_id.contains_key(&id);
        let mut is_present = was_present;
        if node_changed {
            let sql = format!(
                "{NODE_SELECT} AND id=?1 AND (?2 IS NULL OR EXISTS(
                SELECT 1 FROM labels WHERE issue_id=issues.id AND label=?2))"
            );
            let mut node = conn
                .query_row(&sql, rusqlite::params![id, label], read_node)
                .optional()?;
            if let Some(node) = &mut node {
                node.labels = load_labels(conn, &id)?;
                delta.nodes_loaded += 1;
            }
            is_present = node.is_some();
            delta.nodes.push((id.clone(), node));
        }
        // A field-only edit leaves adjacency untouched. Membership changes
        // restore/drop incident edges, including previously dangling references.
        if was_present != is_present || (edges_changed && is_present) {
            delta.edge_ids.insert(id);
        }
    }
    let mut stmt = conn.prepare(
        "SELECT depends_on_id,issue_id,type FROM dependencies
         WHERE issue_id=?1 OR depends_on_id=?1",
    )?;
    let membership: HashMap<&str, bool> = delta
        .nodes
        .iter()
        .map(|(id, node)| (id.as_str(), node.is_some()))
        .collect();
    let is_present = |id: &str| {
        membership
            .get(id)
            .copied()
            .unwrap_or_else(|| scope.snapshot.by_id.contains_key(id))
    };
    for id in &delta.edge_ids {
        let rows = stmt.query_map([id], |row| read_edge(row, is_present))?;
        for row in rows {
            if let Some((from, to, kind)) = row? {
                delta.edges_loaded += 1;
                delta.edges.insert((from, to), kind);
            }
        }
    }
    Ok(delta)
}

const NODE_SELECT: &str =
    "SELECT id,title,status,priority,issue_type,assignee,created_at,updated_at,due_at,content_hash
     FROM issues WHERE COALESCE(LOWER(status),'open')<>'tombstone' AND COALESCE(is_template,0)=0";

fn read_node(row: &Row<'_>) -> rusqlite::Result<NodeData> {
    // Match SqliteStorage's model normalization, including historical aliases.
    let status = row
        .get::<_, Option<String>>(2)?
        .map_or_else(Status::default, |value| {
            value.parse().unwrap_or(Status::Custom(value))
        });
    let issue_type = row
        .get::<_, Option<String>>(4)?
        .and_then(|value| value.parse::<IssueType>().ok())
        .unwrap_or_default();
    Ok(NodeData {
        id: row.get(0)?,
        title: row.get(1)?,
        status: status.to_string(),
        priority: row.get::<_, Option<i32>>(3)?.unwrap_or(2),
        issue_type: issue_type.to_string(),
        assignee: row
            .get::<_, Option<String>>(5)?
            .filter(|value| !value.is_empty()),
        labels: Vec::new(),
        created_at: datetime(&row.get::<_, String>(6)?)?,
        updated_at: datetime(&row.get::<_, String>(7)?)?,
        due_at: row
            .get::<_, Option<String>>(8)?
            .as_deref()
            .map(datetime)
            .transpose()?,
        // v2 fingerprints include the actual projected fields even if an
        // external writer leaves this optional upstream content hash stale.
        content_hash: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
    })
}

fn datetime(text: &str) -> rusqlite::Result<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Ok(value.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
        .map(|n| n.and_utc())
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })
}

fn load_labels(conn: &Connection, id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt =
        conn.prepare_cached("SELECT label FROM labels WHERE issue_id=?1 ORDER BY label")?;
    let rows = stmt.query_map([id], |r| r.get(0))?.collect();
    rows
}

fn read_edge(
    row: &Row<'_>,
    is_present: impl Fn(&str) -> bool,
) -> rusqlite::Result<Option<(String, String, DependencyKind)>> {
    let from: String = row.get(0)?;
    let to: String = row.get(1)?;
    // Incoming adjacency may belong to an outside source. Check the projected
    // membership before decoding its payload, including staged removals/adds.
    if !is_present(&from) || !is_present(&to) {
        return Ok(None);
    }
    let text: String = row.get(2)?;
    let dep_type = text.parse().unwrap_or(DependencyType::Custom(text));
    Ok(Some((from, to, dependency_kind_from_beads(&dep_type))))
}

impl Scope {
    fn apply(&mut self, delta: Delta) {
        let mut remove_edges = Vec::new();
        for id in &delta.edge_ids {
            if let Some(&ix) = self.snapshot.by_id.get(id) {
                remove_edges.extend(
                    self.snapshot
                        .graph
                        .edges_directed(ix, Direction::Incoming)
                        .map(|e| e.id()),
                );
                remove_edges.extend(
                    self.snapshot
                        .graph
                        .edges_directed(ix, Direction::Outgoing)
                        .map(|e| e.id()),
                );
            }
        }
        remove_edges.sort_unstable_by_key(|id| std::cmp::Reverse(id.index()));
        remove_edges.dedup();
        for ix in remove_edges {
            let (from, to) = self
                .snapshot
                .graph
                .edge_endpoints(ix)
                .expect("staged edge exists");
            self.toggle(edge_digest(
                &self.snapshot.graph[from].id,
                &self.snapshot.graph[to].id,
                self.snapshot.graph[ix].kind,
            ));
            self.snapshot.graph.remove_edge(ix);
        }
        for (id, node) in delta.nodes {
            if let Some(&ix) = self.snapshot.by_id.get(&id) {
                self.toggle(node_digest(&self.snapshot.graph[ix]));
                if let Some(node) = node {
                    self.toggle(node_digest(&node));
                    self.snapshot.graph[ix] = node;
                } else {
                    // Incident edges have already been removed. petgraph's dense
                    // removal swaps the last node into this index.
                    self.snapshot.graph.remove_node(ix);
                    self.snapshot.by_id.remove(&id);
                    if let Some(moved) = self.snapshot.graph.node_weight(ix) {
                        self.snapshot.by_id.insert(moved.id.clone(), ix);
                    }
                }
            } else if let Some(node) = node {
                self.toggle(node_digest(&node));
                self.snapshot.add_node(node);
            }
        }
        for ((from, to), kind) in delta.edges {
            if self.snapshot.add_edge(&from, &to, kind) {
                self.toggle(edge_digest(&from, &to, kind));
            }
        }
        self.snapshot.data_hash = format!(
            "g2:{}",
            self.fingerprint
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
    }

    fn toggle(&mut self, digest: [u8; 32]) {
        for (out, part) in self.fingerprint.iter_mut().zip(digest) {
            *out ^= part;
        }
    }
}

fn digest(value: &impl Serialize) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(value).expect("graph fingerprint scalar fields serialize"))
        .into()
}

fn node_digest(node: &NodeData) -> [u8; 32] {
    digest(&(
        "node",
        &node.id,
        &node.title,
        &node.status,
        node.priority,
        &node.issue_type,
        &node.assignee,
        &node.labels,
        node.created_at,
        node.updated_at,
        node.due_at,
        &node.content_hash,
    ))
}

fn edge_digest(from: &str, to: &str, kind: DependencyKind) -> [u8; 32] {
    digest(&("edge", from, to, format!("{kind:?}")))
}

#[cfg(test)]
#[path = "incremental_tests.rs"]
mod tests;
