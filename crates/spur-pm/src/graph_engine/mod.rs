pub mod insights;
pub mod metrics;
pub mod plan;
pub mod raw;
pub mod score;
pub mod snapshot;
pub mod triage;

pub use insights::compute_insights;
pub use metrics::hits;
pub use plan::compute_plan;
pub use raw::{
    serialize_alerts, serialize_insights, serialize_plan, serialize_subgraph, serialize_triage,
};
pub use score::{
    is_actionable, score_all, score_node, transitive_unblocks, ScoreBreakdown, ScoreConfig,
};
pub use snapshot::{DependencyKind, EdgeData, GraphSnapshot, NodeData};
pub use triage::compute_triage;

#[cfg(test)]
mod facade_tests {
    use super::{GraphEngine, GraphEngineConfig};
    use crate::adapter::IssueTracker;
    use crate::beads_crate::{AdapterConfig, BeadsCrateAdapter};
    use crate::types::IssueCreate;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn seeded_engine() -> (TempDir, GraphEngine, String, String) {
        let dir = TempDir::new().expect("create temp beads db");
        let beads = Arc::new(
            BeadsCrateAdapter::open(dir.path(), AdapterConfig::default())
                .await
                .expect("open beads adapter"),
        );

        let root = beads
            .create_issue(IssueCreate {
                title: "Facade root".into(),
                priority: Some(1),
                labels: vec!["scope-a".into()],
                ..IssueCreate::default()
            })
            .await
            .expect("create root issue");
        let _child = beads
            .create_issue(IssueCreate {
                title: "Facade child".into(),
                priority: Some(2),
                labels: vec!["scope-a".into()],
                depends_on: vec![root.clone()],
                ..IssueCreate::default()
            })
            .await
            .expect("create child issue");
        let outside = beads
            .create_issue(IssueCreate {
                title: "Outside issue".into(),
                priority: Some(3),
                labels: vec!["scope-b".into()],
                ..IssueCreate::default()
            })
            .await
            .expect("create outside issue");

        let engine = GraphEngine::new(beads, GraphEngineConfig::default());
        (dir, engine, root, outside)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unchanged_graph_requests_reuse_materialization() {
        use std::sync::atomic::Ordering;

        let (_dir, engine, root, _) = seeded_engine().await;
        let first = engine.subgraph(&root, Some(2), Some("json")).await.unwrap();
        let reads = engine.beads.metrics().read_total.load(Ordering::Relaxed);
        let second = engine.subgraph(&root, Some(2), Some("json")).await.unwrap();
        assert_eq!(first.data_hash, second.data_hash);
        assert_eq!(
            engine.beads.metrics().read_total.load(Ordering::Relaxed),
            reads,
            "unchanged graph requests must not reload the graph through the beads adapter"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn facade_methods_populate_raw_passthrough_reports() {
        let (_dir, engine, root, _outside) = seeded_engine().await;

        let triage = engine.triage(None).await.expect("triage report");
        assert_eq!(
            triage.raw["triage"]["quick_ref"]["open_count"], 3,
            "triage raw is populated from the typed report"
        );
        assert!(triage.raw.get("raw").is_none(), "raw must not recurse");

        let plan = engine.plan(None).await.expect("execution plan");
        assert_eq!(
            plan.raw["plan"]["tracks"][0]["items"][0]["id"], root,
            "plan raw is populated from the typed report"
        );
        assert!(plan.raw.get("raw").is_none(), "raw must not recurse");

        let insights = engine.insights(None).await.expect("graph insights");
        assert_eq!(
            insights.raw["Orphans"]
                .as_array()
                .expect("orphans are serialized")
                .len(),
            1,
            "insights raw is populated with Go-compatible wire fields"
        );
        assert!(insights.raw.get("raw").is_none(), "raw must not recurse");

        let alerts = engine.alerts().await.expect("alert report");
        assert_eq!(
            alerts.raw["summary"]["total"], 0,
            "alerts raw is populated from the typed report"
        );
        assert!(alerts.raw.get("raw").is_none(), "raw must not recurse");

        let subgraph = engine
            .subgraph(&root, Some(1), Some("json"))
            .await
            .expect("issue subgraph");
        assert_eq!(
            subgraph.raw["adjacency"]["nodes"]
                .as_array()
                .expect("subgraph nodes are serialized")
                .len(),
            2,
            "subgraph raw is populated from the typed report"
        );
        assert!(subgraph.raw.get("raw").is_none(), "raw must not recurse");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn facade_label_methods_scope_snapshot_before_computing() {
        let (_dir, engine, root, outside) = seeded_engine().await;

        let triage = engine
            .triage(Some("scope-a"))
            .await
            .expect("label triage report");
        assert_eq!(triage.triage.quick_ref.open_count, 2);
        assert!(
            triage
                .triage
                .recommendations
                .iter()
                .all(|item| item.labels.iter().any(|label| label == "scope-a")),
            "label-filtered triage only contains matching issues"
        );

        let label_graph = engine
            .graph_by_label("scope-a", Some("json"))
            .await
            .expect("label subgraph");
        let node_ids: Vec<&str> = label_graph
            .adjacency
            .as_ref()
            .expect("json graph has adjacency")
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();

        assert!(node_ids.contains(&root.as_str()));
        assert!(!node_ids.contains(&outside.as_str()));
        assert_eq!(label_graph.raw["nodes"], 2);
    }
}

pub use insights::InsightConfig;

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::beads_crate::BeadsCrateAdapter;
use crate::graph::{
    AdjacencyData, Alert, AlertReport, AlertSummary, DependencyGraph, ExecutionPlan, GraphEdge,
    GraphInsights, GraphNode, TriageReport,
};

#[derive(Debug, Clone)]
pub struct AlertConfig {
    pub stale_threshold_days: i64,
    pub cascade_threshold: usize,
    pub now: DateTime<Utc>,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            stale_threshold_days: 14,
            cascade_threshold: 5,
            now: Utc::now(),
        }
    }
}

pub fn compute_alerts(snap: &GraphSnapshot, cfg: &AlertConfig) -> AlertReport {
    let score_cfg = ScoreConfig {
        now: cfg.now,
        ..ScoreConfig::default()
    };
    let triage = compute_triage(snap, &score_cfg);
    let insights = compute_insights(snap, &InsightConfig::default());

    let mut alerts = Vec::new();
    for ix in snap.graph.node_indices() {
        let node = &snap.graph[ix];
        let is_open = matches!(node.status.as_str(), "open" | "in_progress");

        if is_open {
            let stale_days = (cfg.now - node.updated_at).num_days();
            if stale_days > cfg.stale_threshold_days {
                alerts.push(Alert {
                    alert_type: "stale".into(),
                    severity: "warning".into(),
                    message: format!(
                        "Issue {} has not been updated in {stale_days} days",
                        node.id
                    ),
                    issue_id: Some(node.id.clone()),
                    baseline_value: Some(cfg.stale_threshold_days as f64),
                    current_value: Some(stale_days as f64),
                    delta: Some((stale_days - cfg.stale_threshold_days) as f64),
                    ..Alert::default()
                });
            }
        }

        for edge in snap.graph.edges(ix) {
            if !snap.dependency_blocks(ix, edge.weight().kind) {
                continue;
            }
            let blocked = &snap.graph[edge.target()];
            if blocked.priority < node.priority {
                alerts.push(Alert {
                    alert_type: "priority_inversion".into(),
                    severity: "warning".into(),
                    message: format!(
                        "Issue {} (P{}) blocks higher-priority issue {} (P{})",
                        node.id, node.priority, blocked.id, blocked.priority
                    ),
                    issue_id: Some(node.id.clone()),
                    issue_ids: vec![node.id.clone(), blocked.id.clone()],
                    ..Alert::default()
                });
            }
        }
    }

    for cycle in &insights.cycles {
        for id in cycle {
            alerts.push(Alert {
                alert_type: "cycle".into(),
                severity: "critical".into(),
                message: format!("Issue {id} is in a dependency cycle"),
                issue_id: Some(id.clone()),
                issue_ids: cycle.clone(),
                ..Alert::default()
            });
        }
    }

    for blocker in &triage.triage.blockers_to_clear {
        if blocker.unblocks_count >= cfg.cascade_threshold {
            alerts.push(Alert {
                alert_type: "cascade".into(),
                severity: "warning".into(),
                message: format!(
                    "Issue {} blocks {} downstream items",
                    blocker.id, blocker.unblocks_count
                ),
                issue_id: Some(blocker.id.clone()),
                issue_ids: blocker.unblocks_ids.clone(),
                baseline_value: Some(cfg.cascade_threshold as f64),
                current_value: Some(blocker.unblocks_count as f64),
                ..Alert::default()
            });
        }
    }

    for id in &insights.orphans {
        let Some(&ix) = snap.by_id.get(id) else {
            continue;
        };
        let node = &snap.graph[ix];
        if matches!(node.status.as_str(), "open" | "in_progress") && node.priority <= 1 {
            alerts.push(Alert {
                alert_type: "orphan_high_priority".into(),
                severity: "info".into(),
                message: format!("High-priority issue {} has no dependencies", node.id),
                issue_id: Some(node.id.clone()),
                ..Alert::default()
            });
        }
    }

    alerts.sort_by(|a, b| {
        a.alert_type
            .cmp(&b.alert_type)
            .then_with(|| a.issue_id.cmp(&b.issue_id))
            .then_with(|| a.message.cmp(&b.message))
    });

    let summary = AlertSummary {
        total: alerts.len(),
        critical: alerts
            .iter()
            .filter(|alert| alert.severity == "critical")
            .count(),
        warning: alerts
            .iter()
            .filter(|alert| alert.severity == "warning")
            .count(),
        info: alerts
            .iter()
            .filter(|alert| alert.severity == "info")
            .count(),
    };

    AlertReport {
        generated_at: Some(snap.generated_at.to_rfc3339()),
        data_hash: Some(snap.data_hash.clone()),
        alerts,
        summary: Some(summary),
        usage_hints: vec![
            "Filter by severity: jq '.alerts[] | select(.severity==\"critical\")'".into(),
            "Group by type: jq '.alerts | group_by(.type)'".into(),
        ],
        raw: serde_json::Value::Null,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphFormat {
    Json,
    Dot,
    Mermaid,
}

impl GraphFormat {
    pub fn parse(s: Option<&str>) -> Self {
        match s.unwrap_or("json").to_ascii_lowercase().as_str() {
            "dot" => Self::Dot,
            "mermaid" => Self::Mermaid,
            _ => Self::Json,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Dot => "dot",
            Self::Mermaid => "mermaid",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SubgraphRoot<'a> {
    Issue(&'a str),
    AllIssues,
}

pub fn compute_subgraph(
    snap: &GraphSnapshot,
    root: SubgraphRoot<'_>,
    depth: Option<u32>,
    format: GraphFormat,
) -> DependencyGraph {
    let included: HashSet<NodeIndex> = match root {
        SubgraphRoot::AllIssues => snap.graph.node_indices().collect(),
        SubgraphRoot::Issue(id) => bfs_bidirectional(snap, id, depth.unwrap_or(2)),
    };

    let mut nodes: Vec<GraphNode> = included
        .iter()
        .map(|&ix| {
            let node = &snap.graph[ix];
            GraphNode {
                id: node.id.clone(),
                title: Some(node.title.clone()),
                status: Some(node.status.clone()),
                priority: Some(node.priority),
                labels: node.labels.clone(),
                pagerank: None,
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut edges = Vec::new();
    for &ix in &included {
        for edge in snap.graph.edges(ix) {
            if !included.contains(&edge.target()) {
                continue;
            }
            edges.push(GraphEdge {
                from: snap.graph[ix].id.clone(),
                to: snap.graph[edge.target()].id.clone(),
                edge_type: Some(edge_type_name(edge.weight().kind).into()),
            });
        }
    }
    edges.sort_by(|a, b| {
        (a.from.as_str(), a.to.as_str(), a.edge_type.as_deref()).cmp(&(
            b.from.as_str(),
            b.to.as_str(),
            b.edge_type.as_deref(),
        ))
    });

    let node_count = nodes.len();
    let edge_count = edges.len();
    match format {
        GraphFormat::Json => DependencyGraph {
            format: Some(format.as_str().into()),
            graph: None,
            nodes: node_count,
            edges: edge_count,
            data_hash: Some(snap.data_hash.clone()),
            adjacency: Some(AdjacencyData {
                nodes,
                edges: if edges.is_empty() { None } else { Some(edges) },
            }),
            raw: serde_json::Value::Null,
        },
        GraphFormat::Dot => DependencyGraph {
            format: Some(format.as_str().into()),
            graph: Some(render_dot(&nodes, &edges)),
            nodes: node_count,
            edges: edge_count,
            data_hash: Some(snap.data_hash.clone()),
            adjacency: None,
            raw: serde_json::Value::Null,
        },
        GraphFormat::Mermaid => DependencyGraph {
            format: Some(format.as_str().into()),
            graph: Some(render_mermaid(&nodes, &edges)),
            nodes: node_count,
            edges: edge_count,
            data_hash: Some(snap.data_hash.clone()),
            adjacency: None,
            raw: serde_json::Value::Null,
        },
    }
}

fn bfs_bidirectional(snap: &GraphSnapshot, root_id: &str, depth: u32) -> HashSet<NodeIndex> {
    let mut included = HashSet::new();
    let Some(&root) = snap.by_id.get(root_id) else {
        return included;
    };

    let mut queue = VecDeque::new();
    included.insert(root);
    queue.push_back((root, 0));

    while let Some((ix, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }

        for edge in snap.graph.edges(ix) {
            if included.insert(edge.target()) {
                queue.push_back((edge.target(), current_depth + 1));
            }
        }
        for edge in snap.graph.edges_directed(ix, petgraph::Direction::Incoming) {
            if included.insert(edge.source()) {
                queue.push_back((edge.source(), current_depth + 1));
            }
        }
    }

    included
}

fn edge_type_name(kind: DependencyKind) -> &'static str {
    match kind {
        DependencyKind::Blocks => "blocks",
        DependencyKind::ParentChild => "parent-child",
        DependencyKind::ConditionalBlocks => "conditional-blocks",
        DependencyKind::WaitsFor => "waits-for",
        DependencyKind::RelatedTo => "related-to",
        DependencyKind::Discovered => "discovered",
        DependencyKind::Unknown => "unknown",
    }
}

fn render_dot(nodes: &[GraphNode], edges: &[GraphEdge]) -> String {
    let mut out = String::from("digraph G {\n  rankdir=LR;\n");
    for node in nodes {
        let title = node.title.as_deref().unwrap_or("").replace('"', "\\\"");
        out.push_str(&format!(
            "  \"{}\" [label=\"{}\\n{}\"];\n",
            node.id, node.id, title
        ));
    }
    for edge in edges {
        out.push_str(&format!("  \"{}\" -> \"{}\";\n", edge.from, edge.to));
    }
    out.push_str("}\n");
    out
}

fn render_mermaid(nodes: &[GraphNode], edges: &[GraphEdge]) -> String {
    let mut out = String::from("graph TD\n");
    for node in nodes {
        let title = node.title.as_deref().unwrap_or("").replace('"', "'");
        out.push_str(&format!(
            "  {}[\"{}: {}\"]\n",
            node.id.replace('-', "_"),
            node.id,
            title
        ));
    }
    for edge in edges {
        out.push_str(&format!(
            "  {} --> {}\n",
            edge.from.replace('-', "_"),
            edge.to.replace('-', "_")
        ));
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct GraphEngineConfig {
    pub score: ScoreConfig,
    pub insight: InsightConfig,
    pub alert: AlertConfig,
}

pub struct GraphEngine {
    beads: Arc<BeadsCrateAdapter>,
    cfg: GraphEngineConfig,
}

impl GraphEngine {
    pub fn new(beads: Arc<BeadsCrateAdapter>, cfg: GraphEngineConfig) -> Self {
        Self { beads, cfg }
    }

    async fn snapshot(&self, label: Option<String>) -> anyhow::Result<GraphSnapshot> {
        self.beads
            .read(move |storage| {
                let mut snap = snapshot::load_graph_snapshot(storage, label.as_deref())?;
                snap.data_hash = snap.compute_data_hash();
                Ok(snap)
            })
            .await
    }

    pub async fn triage(&self, label: Option<&str>) -> anyhow::Result<TriageReport> {
        let snap = self.snapshot(label.map(str::to_string)).await?;
        let mut report = compute_triage(&snap, &self.cfg.score);
        report.raw = raw::serialize_triage(&report);
        Ok(report)
    }

    pub async fn plan(&self, label: Option<&str>) -> anyhow::Result<ExecutionPlan> {
        let snap = self.snapshot(label.map(str::to_string)).await?;
        let mut report = compute_plan(&snap, &self.cfg.score);
        report.raw = raw::serialize_plan(&report);
        Ok(report)
    }

    pub async fn insights(&self, label: Option<&str>) -> anyhow::Result<GraphInsights> {
        let snap = self.snapshot(label.map(str::to_string)).await?;
        let mut report = compute_insights(&snap, &self.cfg.insight);
        report.raw = raw::serialize_insights(&report);
        Ok(report)
    }

    pub async fn alerts(&self) -> anyhow::Result<AlertReport> {
        let snap = self.snapshot(None).await?;
        let mut report = compute_alerts(&snap, &self.cfg.alert);
        report.raw = raw::serialize_alerts(&report);
        Ok(report)
    }

    pub async fn subgraph(
        &self,
        root_id: &str,
        depth: Option<u32>,
        format: Option<&str>,
    ) -> anyhow::Result<DependencyGraph> {
        let snap = self.snapshot(None).await?;
        let mut report = compute_subgraph(
            &snap,
            SubgraphRoot::Issue(root_id),
            depth,
            GraphFormat::parse(format),
        );
        report.raw = raw::serialize_subgraph(&report);
        Ok(report)
    }

    pub async fn graph_by_label(
        &self,
        label: &str,
        format: Option<&str>,
    ) -> anyhow::Result<DependencyGraph> {
        let snap = self.snapshot(Some(label.to_string())).await?;
        let mut report = compute_subgraph(
            &snap,
            SubgraphRoot::AllIssues,
            None,
            GraphFormat::parse(format),
        );
        report.raw = raw::serialize_subgraph(&report);
        Ok(report)
    }
}
