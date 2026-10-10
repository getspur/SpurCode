use chrono::{DateTime, Utc};
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::{Directed, Direction};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::beads_crate::dependency_compat::get_dependencies_full_for_issues;

const MAX_PARENT_CHILD_BLOCKING_DEPTH: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyKind {
    Blocks,
    ParentChild,
    ConditionalBlocks,
    WaitsFor,
    RelatedTo,
    Discovered,
    Unknown,
}

impl DependencyKind {
    pub fn parse(s: &str) -> Self {
        match s {
            "blocks" => Self::Blocks,
            "parent-child" => Self::ParentChild,
            "conditional-blocks" => Self::ConditionalBlocks,
            "waits-for" => Self::WaitsFor,
            "related-to" => Self::RelatedTo,
            "discovered" => Self::Discovered,
            _ => Self::Unknown,
        }
    }

    pub fn is_blocking(self) -> bool {
        matches!(
            self,
            Self::Blocks | Self::ParentChild | Self::ConditionalBlocks | Self::WaitsFor
        )
    }

    fn is_direct_blocking(self) -> bool {
        matches!(
            self,
            Self::Blocks | Self::ConditionalBlocks | Self::WaitsFor
        )
    }
}

#[derive(Debug, Clone)]
pub struct NodeData {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: i32,
    pub issue_type: String,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub due_at: Option<DateTime<Utc>>,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct EdgeData {
    pub kind: DependencyKind,
}

pub struct GraphSnapshot {
    pub graph: Graph<NodeData, EdgeData, Directed>,
    pub by_id: HashMap<String, NodeIndex>,
    pub generated_at: DateTime<Utc>,
    pub data_hash: String,
    pub label_filter: Option<String>,
}

impl GraphSnapshot {
    pub fn new(label_filter: Option<String>) -> Self {
        Self {
            graph: Graph::new(),
            by_id: HashMap::new(),
            generated_at: Utc::now(),
            data_hash: String::new(),
            label_filter,
        }
    }

    pub fn add_node(&mut self, data: NodeData) -> NodeIndex {
        let id = data.id.clone();
        let ix = self.graph.add_node(data);
        self.by_id.insert(id, ix);
        ix
    }

    pub fn add_edge(&mut self, from: &str, to: &str, kind: DependencyKind) -> bool {
        let (Some(&a), Some(&b)) = (self.by_id.get(from), self.by_id.get(to)) else {
            return false;
        };
        self.graph.add_edge(a, b, EdgeData { kind });
        true
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub(crate) fn is_blocked_after_closing(
        &self,
        ix: NodeIndex,
        closing: Option<NodeIndex>,
    ) -> bool {
        self.is_blocked_from(ix, closing, 0)
    }

    fn is_blocked_from(
        &self,
        ix: NodeIndex,
        closing: Option<NodeIndex>,
        initial_parent_child_depth: usize,
    ) -> bool {
        let mut queue = VecDeque::from([(ix, initial_parent_child_depth)]);
        let mut visited = HashSet::new();

        while let Some((current, parent_child_depth)) = queue.pop_front() {
            if closing == Some(current) || !visited.insert(current) {
                continue;
            }

            for edge in self.graph.edges_directed(current, Direction::Incoming) {
                let source = edge.source();
                match edge.weight().kind {
                    DependencyKind::ParentChild
                        if parent_child_depth < MAX_PARENT_CHILD_BLOCKING_DEPTH =>
                    {
                        queue.push_back((source, parent_child_depth + 1));
                    }
                    kind if kind.is_direct_blocking() => {
                        if closing != Some(source) && self.graph[source].status != "closed" {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }

        false
    }

    pub(crate) fn dependency_blocks(&self, source: NodeIndex, kind: DependencyKind) -> bool {
        self.dependency_blocks_after_closing(source, kind, None)
    }

    pub(crate) fn dependency_blocks_after_closing(
        &self,
        source: NodeIndex,
        kind: DependencyKind,
        closing: Option<NodeIndex>,
    ) -> bool {
        match kind {
            DependencyKind::ParentChild => self.is_blocked_from(source, closing, 1),
            kind if kind.is_direct_blocking() => {
                closing != Some(source) && self.graph[source].status != "closed"
            }
            _ => false,
        }
    }

    pub fn compute_data_hash(&self) -> String {
        let mut rows: Vec<String> = self
            .graph
            .node_indices()
            .map(|ix| {
                let node = &self.graph[ix];

                let mut labels = node.labels.clone();
                labels.sort_unstable();

                let mut blocking_deps: Vec<&str> = self
                    .graph
                    .edges_directed(ix, Direction::Incoming)
                    .filter(|edge| edge.weight().kind.is_blocking())
                    .map(|edge| self.graph[edge.source()].id.as_str())
                    .collect();
                blocking_deps.sort_unstable();

                format!(
                    "{}|{}|{}|{}",
                    node.id,
                    node.content_hash,
                    labels.join(","),
                    blocking_deps.join(",")
                )
            })
            .collect();
        rows.sort_unstable();

        sha256_hex(rows.join("\n").as_bytes())
    }
}

fn sha256_hex(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

/// Load the full graph snapshot from a SqliteStorage handle.
///
/// MUST be called inside a `BeadsCrateAdapter::read(|s| ...)` closure so the
/// connection-pool discipline holds. Synchronous; the caller is on a Tokio
/// blocking-pool thread.
pub fn load_graph_snapshot(
    storage: &beads_rust::storage::sqlite::SqliteStorage,
    label_filter: Option<&str>,
) -> anyhow::Result<GraphSnapshot> {
    let mut filters = beads_rust::storage::sqlite::ListFilters {
        include_closed: true,
        include_deferred: true,
        ..Default::default()
    };
    if let Some(label) = label_filter {
        filters.labels = Some(vec![label.to_string()]);
    }

    let mut issues = storage.list_issues(&filters)?;
    issues.retain(|issue| issue.status != beads_rust::model::Status::Tombstone);
    let ids: Vec<String> = issues.iter().map(|issue| issue.id.clone()).collect();
    let mut labels_by_id = storage.get_labels_for_issues(&ids)?;
    let mut deps_by_id = if label_filter.is_some() {
        // Keep small filtered snapshots scoped to their selected sources. A
        // full-table read would also decode unrelated, potentially corrupt rows.
        let mut dependencies = HashMap::new();
        for id in &ids {
            let records = storage.get_dependencies_full(id)?;
            if !records.is_empty() {
                dependencies.insert(id.clone(), records);
            }
        }
        dependencies
    } else {
        get_dependencies_full_for_issues(storage, &ids)?
    };

    let mut snap = GraphSnapshot::new(label_filter.map(|s| s.to_string()));
    for issue in &mut issues {
        issue.labels = labels_by_id.remove(&issue.id).unwrap_or_default();
        issue.dependencies = deps_by_id.remove(&issue.id).unwrap_or_default();

        if let Some(label) = label_filter {
            if !issue.labels.iter().any(|candidate| candidate == label) {
                continue;
            }
        }

        let data = NodeData {
            id: issue.id.clone(),
            title: issue.title.clone(),
            status: issue.status.to_string(),
            priority: issue.priority.0,
            issue_type: issue.issue_type.to_string(),
            assignee: issue.assignee.clone(),
            labels: issue.labels.clone(),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            due_at: issue.due_at,
            content_hash: issue
                .content_hash
                .clone()
                .unwrap_or_else(|| issue.compute_content_hash()),
        };
        snap.add_node(data);
    }

    for issue in issues {
        if !snap.by_id.contains_key(&issue.id) {
            continue;
        }
        for dep in issue.dependencies {
            let kind = dependency_kind_from_beads(&dep.dep_type);
            snap.add_edge(&dep.depends_on_id, &issue.id, kind);
        }
    }

    Ok(snap)
}

pub(super) fn dependency_kind_from_beads(
    dep_type: &beads_rust::model::DependencyType,
) -> DependencyKind {
    match dep_type {
        beads_rust::model::DependencyType::Blocks => DependencyKind::Blocks,
        beads_rust::model::DependencyType::ParentChild => DependencyKind::ParentChild,
        beads_rust::model::DependencyType::ConditionalBlocks => DependencyKind::ConditionalBlocks,
        beads_rust::model::DependencyType::WaitsFor => DependencyKind::WaitsFor,
        beads_rust::model::DependencyType::Related
        | beads_rust::model::DependencyType::RelatesTo => DependencyKind::RelatedTo,
        beads_rust::model::DependencyType::DiscoveredFrom => DependencyKind::Discovered,
        _ => DependencyKind::parse(dep_type.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_engine::score::is_actionable;

    fn node(id: &str) -> NodeData {
        NodeData {
            id: id.to_string(),
            title: format!("Issue {id}"),
            status: "open".into(),
            priority: 2,
            issue_type: "task".into(),
            assignee: None,
            labels: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            due_at: None,
            content_hash: format!("hash-{id}"),
        }
    }

    #[test]
    fn add_node_and_edge() {
        let mut s = GraphSnapshot::new(None);
        s.add_node(node("a"));
        s.add_node(node("b"));
        assert_eq!(s.node_count(), 2);
        assert!(s.add_edge("a", "b", DependencyKind::Blocks));
        assert_eq!(s.edge_count(), 1);
    }

    #[test]
    fn add_edge_with_unknown_node_returns_false() {
        let mut s = GraphSnapshot::new(None);
        s.add_node(node("a"));
        assert!(!s.add_edge("a", "missing", DependencyKind::Blocks));
        assert_eq!(s.edge_count(), 0);
    }

    #[test]
    fn dependency_kind_parsing_and_is_blocking() {
        assert!(DependencyKind::parse("blocks").is_blocking());
        assert!(DependencyKind::parse("parent-child").is_blocking());
        assert!(DependencyKind::parse("conditional-blocks").is_blocking());
        assert!(DependencyKind::parse("waits-for").is_blocking());
        assert!(!DependencyKind::parse("related-to").is_blocking());
        assert!(!DependencyKind::parse("discovered").is_blocking());
        assert!(!DependencyKind::parse("garbage").is_blocking());
    }

    #[test]
    fn parent_blocking_stops_after_fifty_hops() {
        let mut snap = GraphSnapshot::new(None);
        let blocker = snap.add_node(node("blocker"));
        let root = snap.add_node(node("root"));
        snap.graph.add_edge(
            blocker,
            root,
            EdgeData {
                kind: DependencyKind::Blocks,
            },
        );

        let mut chain = vec![root];
        for hop in 1..=51 {
            let child = snap.add_node(node(&format!("child-{hop}")));
            snap.graph.add_edge(
                *chain.last().expect("chain has a parent"),
                child,
                EdgeData {
                    kind: DependencyKind::ParentChild,
                },
            );
            chain.push(child);
        }

        assert!(snap.is_blocked_after_closing(chain[50], None));
        assert!(!snap.is_blocked_after_closing(chain[51], None));
        assert!(!is_actionable(&snap, chain[50]));
        assert!(is_actionable(&snap, chain[51]));
    }

    #[test]
    fn parent_blocking_cycle_terminates_when_closing_cuts_the_blocker() {
        let mut snap = GraphSnapshot::new(None);
        let blocker = snap.add_node(node("blocker"));
        let parent = snap.add_node(node("parent"));
        let child = snap.add_node(node("child"));

        snap.graph.add_edge(
            blocker,
            parent,
            EdgeData {
                kind: DependencyKind::Blocks,
            },
        );
        snap.graph.add_edge(
            parent,
            child,
            EdgeData {
                kind: DependencyKind::ParentChild,
            },
        );
        snap.graph.add_edge(
            child,
            parent,
            EdgeData {
                kind: DependencyKind::ParentChild,
            },
        );

        assert!(snap.is_blocked_after_closing(child, None));
        assert!(!snap.is_blocked_after_closing(child, Some(blocker)));
    }

    #[test]
    fn data_hash_is_deterministic_and_content_sensitive() {
        let mut first = GraphSnapshot::new(None);
        let mut first_a = node("a");
        first_a.labels = vec!["beta".into(), "alpha".into()];
        first.add_node(first_a);
        first.add_node(node("b"));
        assert!(first.add_edge("a", "b", DependencyKind::Blocks));

        let first_hash = first.compute_data_hash();
        assert!(!first_hash.is_empty());

        let mut reordered = GraphSnapshot::new(None);
        reordered.add_node(node("b"));
        let mut reordered_a = node("a");
        reordered_a.labels = vec!["alpha".into(), "beta".into()];
        reordered.add_node(reordered_a);
        assert!(reordered.add_edge("a", "b", DependencyKind::Blocks));

        assert_eq!(first_hash, reordered.compute_data_hash());

        let mut changed = GraphSnapshot::new(None);
        let mut changed_a = node("a");
        changed_a.labels = vec!["alpha".into(), "beta".into()];
        changed_a.content_hash = "changed".into();
        changed.add_node(changed_a);
        changed.add_node(node("b"));
        assert!(changed.add_edge("a", "b", DependencyKind::Blocks));

        assert_ne!(first_hash, changed.compute_data_hash());
    }
}

#[cfg(test)]
mod loader_tests {
    use super::*;
    use beads_rust::model::{Issue as BrIssue, IssueType, Priority, Status};
    use petgraph::visit::EdgeRef;
    use petgraph::Direction;

    pub struct FakeBeadsRows {
        pub issues: Vec<FakeBeadsIssue>,
    }

    pub struct FakeBeadsIssue {
        pub data: NodeData,
        pub dependencies: Vec<FakeBeadsDependency>,
    }

    pub struct FakeBeadsDependency {
        pub depends_on_id: String,
        pub dep_type: DependencyKind,
    }

    pub fn load_from_rows(rows: FakeBeadsRows, label_filter: Option<&str>) -> GraphSnapshot {
        let mut snap = GraphSnapshot::new(label_filter.map(|s| s.to_string()));
        for issue in &rows.issues {
            if let Some(label) = label_filter {
                if !issue.data.labels.iter().any(|candidate| candidate == label) {
                    continue;
                }
            }
            snap.add_node(issue.data.clone());
        }
        for issue in rows.issues {
            if !snap.by_id.contains_key(&issue.data.id) {
                continue;
            }
            for dep in issue.dependencies {
                snap.add_edge(&dep.depends_on_id, &issue.data.id, dep.dep_type);
            }
        }
        snap
    }

    fn issue(id: &str, labels: &[&str], dependencies: Vec<FakeBeadsDependency>) -> FakeBeadsIssue {
        FakeBeadsIssue {
            data: node(id, labels),
            dependencies,
        }
    }

    fn dep(depends_on_id: &str, dep_type: DependencyKind) -> FakeBeadsDependency {
        FakeBeadsDependency {
            depends_on_id: depends_on_id.to_string(),
            dep_type,
        }
    }

    fn node(id: &str, labels: &[&str]) -> NodeData {
        NodeData {
            id: id.to_string(),
            title: format!("Issue {id}"),
            status: "open".into(),
            priority: 2,
            issue_type: "task".into(),
            assignee: None,
            labels: labels.iter().map(|s| s.to_string()).collect(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            due_at: None,
            content_hash: format!("hash-{id}"),
        }
    }

    fn br_issue(id: &str, title: &str) -> BrIssue {
        let now = Utc::now();
        BrIssue {
            id: id.into(),
            title: title.into(),
            description: None,
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            created_at: now,
            updated_at: now,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            ephemeral: false,
            content_hash: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            source_system: None,
            source_repo: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            pinned: false,
            is_template: false,
            labels: Vec::new(),
            dependencies: Vec::new(),
            comments: Vec::new(),
        }
    }

    #[test]
    fn load_graph_snapshot_excludes_tombstones() {
        let mut storage = beads_rust::storage::sqlite::SqliteStorage::open_memory().unwrap();

        storage
            .create_issue(&br_issue("bd-open", "Open issue"), "test")
            .unwrap();
        storage
            .create_issue(&br_issue("bd-deleted", "Deleted issue"), "test")
            .unwrap();
        storage
            .delete_issue("bd-deleted", "test", "deleted", None)
            .unwrap();

        let snap = load_graph_snapshot(&storage, None).unwrap();

        assert!(snap.by_id.contains_key("bd-open"));
        assert!(
            !snap.by_id.contains_key("bd-deleted"),
            "graph snapshots should not include tombstones"
        );
    }

    #[test]
    fn filtered_snapshot_ignores_corrupt_dependencies_outside_its_label() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("beads.db");
        let mut storage = beads_rust::storage::sqlite::SqliteStorage::open(&path).unwrap();
        for id in ["bd-a", "bd-b", "bd-unselected"] {
            storage.create_issue(&br_issue(id, id), "test").unwrap();
        }
        for id in ["bd-a", "bd-b"] {
            storage.add_label(id, "selected", "test").unwrap();
        }
        for id in ["bd-a", "bd-unselected"] {
            storage
                .add_dependency(id, "bd-b", "blocks", "test")
                .unwrap();
        }
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute(
                "UPDATE dependencies SET created_at = 'invalid-date' WHERE issue_id = 'bd-unselected'",
                [],
            )
            .unwrap();

        let snap = load_graph_snapshot(&storage, Some("selected")).unwrap();
        assert_eq!(snap.node_count(), 2);
        assert_eq!(snap.edge_count(), 1);
        assert!(!snap.by_id.contains_key("bd-unselected"));
        // Full snapshots deliberately report corruption instead of losing edges.
        assert!(load_graph_snapshot(&storage, None).is_err());
    }

    #[test]
    fn loader_filters_by_label() {
        let rows = FakeBeadsRows {
            issues: vec![
                issue("a", &["spur:plan-id:P1"], vec![]),
                issue("b", &[], vec![]),
                issue(
                    "c",
                    &["spur:plan-id:P1"],
                    vec![dep("a", DependencyKind::Blocks)],
                ),
            ],
        };

        let snap = load_from_rows(rows, Some("spur:plan-id:P1"));

        assert_eq!(snap.node_count(), 2);
        assert_eq!(snap.edge_count(), 1);
    }

    #[test]
    fn loader_skips_dangling_edges() {
        let rows = FakeBeadsRows {
            issues: vec![issue(
                "a",
                &["L"],
                vec![dep("missing", DependencyKind::Blocks)],
            )],
        };

        let snap = load_from_rows(rows, Some("L"));

        assert_eq!(snap.node_count(), 1);
        assert_eq!(snap.edge_count(), 0);
    }

    #[test]
    fn loader_edge_direction_blocker_to_blocked() {
        let rows = FakeBeadsRows {
            issues: vec![
                issue("A", &[], vec![]),
                issue("B", &[], vec![dep("A", DependencyKind::Blocks)]),
            ],
        };

        let snap = load_from_rows(rows, None);
        let bx = snap.by_id["B"];
        let mut incoming: Vec<&str> = snap
            .graph
            .edges_directed(bx, Direction::Incoming)
            .map(|e| snap.graph[e.source()].id.as_str())
            .collect();
        incoming.sort();
        assert_eq!(incoming, vec!["A"]);
    }
}
