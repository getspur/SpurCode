use std::collections::{HashSet, VecDeque};

use crate::{
    graph_edge_kind_or_default, GraphEdgeArtifact, GraphEdgeKind, GraphIndexArtifact,
    GraphSymbolArtifact, RelationKind,
};

const MAX_SUBGRAPH_RADIUS: u8 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum CalleeRecord<'a> {
    Resolved {
        symbol: &'a GraphSymbolArtifact,
        edge: &'a GraphEdgeArtifact,
    },
    Unresolved {
        edge: &'a GraphEdgeArtifact,
        target_label: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallerRecord<'a> {
    Resolved {
        caller: &'a GraphSymbolArtifact,
        edge: &'a GraphEdgeArtifact,
    },
    Unresolved {
        caller: &'a GraphSymbolArtifact,
        edge: &'a GraphEdgeArtifact,
        target_label: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OwnedCalleeRecord {
    Resolved {
        symbol: GraphSymbolArtifact,
        edge: GraphEdgeArtifact,
    },
    Unresolved {
        edge: GraphEdgeArtifact,
        target_label: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OwnedCallerRecord {
    Resolved {
        caller: GraphSymbolArtifact,
        edge: GraphEdgeArtifact,
    },
    Unresolved {
        caller: GraphSymbolArtifact,
        edge: GraphEdgeArtifact,
        target_label: String,
    },
}

impl CalleeRecord<'_> {
    pub fn edge(&self) -> &GraphEdgeArtifact {
        match self {
            CalleeRecord::Resolved { edge, .. } | CalleeRecord::Unresolved { edge, .. } => edge,
        }
    }

    pub fn edge_kind(&self) -> GraphEdgeKind {
        edge_kind(self.edge())
    }

    pub fn is_resolved(&self) -> bool {
        matches!(self, CalleeRecord::Resolved { .. })
    }
}

impl CallerRecord<'_> {
    pub fn edge(&self) -> &GraphEdgeArtifact {
        match self {
            CallerRecord::Resolved { edge, .. } | CallerRecord::Unresolved { edge, .. } => edge,
        }
    }

    pub fn edge_kind(&self) -> GraphEdgeKind {
        edge_kind(self.edge())
    }

    pub fn is_resolved(&self) -> bool {
        matches!(self, CallerRecord::Resolved { .. })
    }
}

impl OwnedCalleeRecord {
    pub fn edge(&self) -> &GraphEdgeArtifact {
        match self {
            Self::Resolved { edge, .. } | Self::Unresolved { edge, .. } => edge,
        }
    }

    pub fn edge_kind(&self) -> GraphEdgeKind {
        edge_kind(self.edge())
    }

    pub fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved { .. })
    }
}

impl OwnedCallerRecord {
    pub fn edge(&self) -> &GraphEdgeArtifact {
        match self {
            Self::Resolved { edge, .. } | Self::Unresolved { edge, .. } => edge,
        }
    }

    pub fn edge_kind(&self) -> GraphEdgeKind {
        edge_kind(self.edge())
    }

    pub fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved { .. })
    }
}

impl From<CalleeRecord<'_>> for OwnedCalleeRecord {
    fn from(record: CalleeRecord<'_>) -> Self {
        match record {
            CalleeRecord::Resolved { symbol, edge } => Self::Resolved {
                symbol: symbol.clone(),
                edge: edge.clone(),
            },
            CalleeRecord::Unresolved { edge, target_label } => Self::Unresolved {
                edge: edge.clone(),
                target_label,
            },
        }
    }
}

impl From<CallerRecord<'_>> for OwnedCallerRecord {
    fn from(record: CallerRecord<'_>) -> Self {
        match record {
            CallerRecord::Resolved { caller, edge } => Self::Resolved {
                caller: caller.clone(),
                edge: edge.clone(),
            },
            CallerRecord::Unresolved {
                caller,
                edge,
                target_label,
            } => Self::Unresolved {
                caller: caller.clone(),
                edge: edge.clone(),
                target_label,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubgraphView<'a> {
    pub nodes: Vec<&'a GraphSymbolArtifact>,
    pub edges: Vec<&'a GraphEdgeArtifact>,
    pub truncated_frontier: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubgraphBudget {
    pub max_nodes: usize,
    pub max_edges: usize,
}

pub fn find_symbol<'a>(
    artifact: &'a GraphIndexArtifact,
    symbol_id: &str,
) -> Option<&'a GraphSymbolArtifact> {
    artifact
        .symbols
        .iter()
        .find(|symbol| symbol.stable_symbol_id == symbol_id)
}

pub fn find_callers<'a>(
    artifact: &'a GraphIndexArtifact,
    symbol_id: &str,
) -> Vec<&'a GraphSymbolArtifact> {
    find_caller_edges(artifact, symbol_id)
        .into_iter()
        .filter_map(|record| match record {
            CallerRecord::Resolved { caller, .. } => Some(caller),
            CallerRecord::Unresolved { .. } => None,
        })
        .collect()
}

pub fn find_callees<'a>(
    artifact: &'a GraphIndexArtifact,
    symbol_id: &str,
) -> Vec<&'a GraphSymbolArtifact> {
    find_callee_edges(artifact, symbol_id)
        .into_iter()
        .filter_map(|record| match record {
            CalleeRecord::Resolved { symbol, .. } => Some(symbol),
            CalleeRecord::Unresolved { .. } => None,
        })
        .collect()
}

pub fn find_caller_edges<'a>(
    artifact: &'a GraphIndexArtifact,
    symbol_id: &str,
) -> Vec<CallerRecord<'a>> {
    let Some(target_symbol) = find_symbol(artifact, symbol_id) else {
        return Vec::new();
    };
    let unresolved_labels = unresolved_target_labels_for_symbol(target_symbol);

    artifact
        .edges
        .iter()
        .filter(|edge| is_caller_relation(edge.relation))
        .filter_map(|edge| {
            let caller = find_symbol(artifact, &edge.source_stable_symbol_id)?;
            if edge.target_stable_symbol_id.as_deref() == Some(symbol_id) {
                Some(CallerRecord::Resolved { caller, edge })
            } else if edge.target_stable_symbol_id.is_none()
                && edge
                    .target_label
                    .as_deref()
                    .is_some_and(|label| unresolved_labels.contains(label))
            {
                Some(CallerRecord::Unresolved {
                    caller,
                    edge,
                    target_label: edge.target_label.clone().unwrap_or_default(),
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn find_callee_edges<'a>(
    artifact: &'a GraphIndexArtifact,
    symbol_id: &str,
) -> Vec<CalleeRecord<'a>> {
    if find_symbol(artifact, symbol_id).is_none() {
        return Vec::new();
    }

    artifact
        .edges
        .iter()
        .filter(|edge| is_caller_relation(edge.relation))
        .filter(|edge| edge.source_stable_symbol_id == symbol_id)
        .filter_map(|edge| match edge.target_stable_symbol_id.as_deref() {
            Some(target_id) => find_symbol(artifact, target_id)
                .map(|symbol| CalleeRecord::Resolved { symbol, edge }),
            None => edge
                .target_label
                .as_ref()
                .map(|target_label| CalleeRecord::Unresolved {
                    edge,
                    target_label: target_label.clone(),
                }),
        })
        .collect()
}

pub fn edge_kind(edge: &GraphEdgeArtifact) -> GraphEdgeKind {
    graph_edge_kind_or_default(edge.relation, edge.edge_kind)
}

pub fn bounded_subgraph<'a>(
    artifact: &'a GraphIndexArtifact,
    root_id: &str,
    radius: u8,
    edge_kinds: Option<&[GraphEdgeKind]>,
    include_unresolved: bool,
) -> SubgraphView<'a> {
    bounded_subgraph_from_roots(
        artifact,
        &[root_id],
        radius,
        edge_kinds,
        include_unresolved,
        None,
    )
}

pub fn bounded_subgraph_with_budget<'a>(
    artifact: &'a GraphIndexArtifact,
    root_ids: &[&str],
    radius: u8,
    edge_kinds: Option<&[GraphEdgeKind]>,
    include_unresolved: bool,
    budget: SubgraphBudget,
) -> SubgraphView<'a> {
    bounded_subgraph_from_roots(
        artifact,
        root_ids,
        radius,
        edge_kinds,
        include_unresolved,
        Some(budget),
    )
}

fn bounded_subgraph_from_roots<'a>(
    artifact: &'a GraphIndexArtifact,
    root_ids: &[&str],
    radius: u8,
    edge_kinds: Option<&[GraphEdgeKind]>,
    include_unresolved: bool,
    budget: Option<SubgraphBudget>,
) -> SubgraphView<'a> {
    let mut traversal = SubgraphTraversal::new(artifact, edge_kinds, include_unresolved, budget);

    for root_id in root_ids {
        traversal.seed_root(root_id);
    }

    traversal.run(radius.min(MAX_SUBGRAPH_RADIUS));
    traversal.finish()
}

struct SubgraphTraversal<'a, 'k> {
    artifact: &'a GraphIndexArtifact,
    edge_kinds: Option<&'k [GraphEdgeKind]>,
    include_unresolved: bool,
    budget: Option<SubgraphBudget>,
    nodes: Vec<&'a GraphSymbolArtifact>,
    edges: Vec<&'a GraphEdgeArtifact>,
    visited_nodes: HashSet<String>,
    visited_edges: HashSet<usize>,
    queue: VecDeque<(String, u8)>,
    truncated_frontier: Vec<String>,
    frontier_seen: HashSet<String>,
    truncated: bool,
}

impl<'a, 'k> SubgraphTraversal<'a, 'k> {
    fn new(
        artifact: &'a GraphIndexArtifact,
        edge_kinds: Option<&'k [GraphEdgeKind]>,
        include_unresolved: bool,
        budget: Option<SubgraphBudget>,
    ) -> Self {
        Self {
            artifact,
            edge_kinds,
            include_unresolved,
            budget,
            nodes: Vec::new(),
            edges: Vec::new(),
            visited_nodes: HashSet::new(),
            visited_edges: HashSet::new(),
            queue: VecDeque::new(),
            truncated_frontier: Vec::new(),
            frontier_seen: HashSet::new(),
            truncated: false,
        }
    }

    fn seed_root(&mut self, root_id: &str) {
        if self.visited_nodes.contains(root_id) {
            return;
        }
        let Some(root) = find_symbol(self.artifact, root_id) else {
            return;
        };
        if self.node_budget_full() {
            self.truncated = true;
            self.add_frontier(root_id);
            return;
        }

        self.visited_nodes.insert(root_id.to_owned());
        self.nodes.push(root);
        self.queue.push_back((root_id.to_owned(), 0));
    }

    fn run(&mut self, radius: u8) {
        while let Some((current_id, depth)) = self.queue.pop_front() {
            if depth >= radius {
                continue;
            }
            self.expand_node(&current_id, depth);
        }
    }

    fn expand_node(&mut self, current_id: &str, depth: u8) {
        let unresolved_current_labels = self
            .include_unresolved
            .then(|| {
                find_symbol(self.artifact, current_id).map(unresolved_target_labels_for_symbol)
            })
            .flatten();

        for (edge_index, edge) in self.artifact.edges.iter().enumerate() {
            if !edge_matches_filter(edge, self.edge_kinds) {
                continue;
            }
            if !self.include_unresolved && edge.target_stable_symbol_id.is_none() {
                continue;
            }

            // Outgoing edges with unresolved targets carry a `target_label` but
            // no symbol to enqueue. Record them as boundary edges so subgraph
            // consumers see the same neighbor set that `find_callee_edges` does;
            // don't advance depth since there is no target node to expand.
            if edge.source_stable_symbol_id == current_id && edge.target_stable_symbol_id.is_none()
            {
                self.try_add_edge(edge_index, edge);
                continue;
            }

            if edge.target_stable_symbol_id.is_none() {
                let Some(labels) = unresolved_current_labels.as_ref() else {
                    continue;
                };
                if !edge
                    .target_label
                    .as_deref()
                    .is_some_and(|label| labels.contains(label))
                {
                    continue;
                }

                let caller_id = edge.source_stable_symbol_id.as_str();
                self.try_add_neighbor_edge(edge_index, edge, caller_id, depth + 1);
                continue;
            }

            let Some(neighbor_id) = incident_neighbor_id(edge, current_id) else {
                continue;
            };

            self.try_add_neighbor_edge(edge_index, edge, neighbor_id, depth + 1);
        }
    }

    fn try_add_neighbor_edge(
        &mut self,
        edge_index: usize,
        edge: &'a GraphEdgeArtifact,
        neighbor_id: &str,
        depth: u8,
    ) {
        if self.visited_nodes.contains(neighbor_id) {
            self.try_add_edge(edge_index, edge);
            return;
        }

        let Some(symbol) = find_symbol(self.artifact, neighbor_id) else {
            return;
        };
        if self.node_budget_full() || self.edge_budget_full() {
            self.truncated = true;
            self.add_frontier(neighbor_id);
            return;
        }

        self.visited_edges.insert(edge_index);
        self.edges.push(edge);
        self.visited_nodes.insert(neighbor_id.to_owned());
        self.nodes.push(symbol);
        self.queue.push_back((neighbor_id.to_owned(), depth));
    }

    fn try_add_edge(&mut self, edge_index: usize, edge: &'a GraphEdgeArtifact) -> bool {
        if self.visited_edges.contains(&edge_index) {
            return true;
        }
        if self.edge_budget_full() {
            self.truncated = true;
            return false;
        }

        self.visited_edges.insert(edge_index);
        self.edges.push(edge);
        true
    }

    fn node_budget_full(&self) -> bool {
        self.budget
            .is_some_and(|budget| self.nodes.len() >= budget.max_nodes)
    }

    fn edge_budget_full(&self) -> bool {
        self.budget
            .is_some_and(|budget| self.edges.len() >= budget.max_edges)
    }

    fn add_frontier(&mut self, symbol_id: &str) {
        if self.visited_nodes.contains(symbol_id) {
            return;
        }
        if self.frontier_seen.insert(symbol_id.to_owned()) {
            self.truncated_frontier.push(symbol_id.to_owned());
        }
    }

    fn finish(self) -> SubgraphView<'a> {
        SubgraphView {
            nodes: self.nodes,
            edges: self.edges,
            truncated_frontier: self.truncated_frontier,
            truncated: self.truncated,
        }
    }
}

/// Edges that contribute to caller/callee relationships.
///
/// Includes `RelationKind::References` so that function-value passes such as
/// `.map(fn_name)` flow through `find_callers`/`find_callee_edges` — without
/// this, a function that is only ever referenced via higher-order method
/// calls would appear to have zero callers (Flaw A in the code_* tool audit).
/// Consumers that need strict-Calls-only behavior can filter at the
/// `code_subgraph` layer via public `GraphEdgeKind` values such as
/// `edge_kinds=["calls"]`.
fn is_caller_relation(relation: RelationKind) -> bool {
    matches!(relation, RelationKind::Calls | RelationKind::References)
}

fn edge_matches_filter(edge: &GraphEdgeArtifact, edge_kinds: Option<&[GraphEdgeKind]>) -> bool {
    edge_kinds.is_none_or(|kinds| kinds.contains(&edge_kind(edge)))
}

fn incident_neighbor_id<'a>(edge: &'a GraphEdgeArtifact, current_id: &str) -> Option<&'a str> {
    let target_id = edge.target_stable_symbol_id.as_deref()?;
    if edge.source_stable_symbol_id == current_id {
        Some(target_id)
    } else if target_id == current_id {
        Some(edge.source_stable_symbol_id.as_str())
    } else {
        None
    }
}

fn unresolved_target_labels_for_symbol(symbol: &GraphSymbolArtifact) -> HashSet<&str> {
    let mut labels = HashSet::new();
    labels.insert(symbol.entity_name.as_str());
    labels.insert(symbol.qualified_name.as_str());
    labels.insert(symbol.stable_symbol_id.as_str());
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Confidence, GraphEdgeKind, GraphIndexHeader};

    fn symbol(id: &str) -> GraphSymbolArtifact {
        GraphSymbolArtifact {
            stable_symbol_id: id.to_owned(),
            file_path: format!("src/{id}.rs"),
            byte_range: [0, 8],
            line_range: [1, 1],
            entity_name: id.to_owned(),
            qualified_name: id.to_owned(),
            symbol_kind: "function".to_owned(),
            anchor_hash: format!("hash-{id}"),
            enclosing_scope: None,
        }
    }

    fn edge(source: &str, target: &str, relation: RelationKind) -> GraphEdgeArtifact {
        edge_with_kind(source, target, relation, None)
    }

    fn edge_with_kind(
        source: &str,
        target: &str,
        relation: RelationKind,
        edge_kind: Option<GraphEdgeKind>,
    ) -> GraphEdgeArtifact {
        GraphEdgeArtifact {
            source_stable_symbol_id: source.to_owned(),
            target_stable_symbol_id: Some(target.to_owned()),
            target_label: None,
            import_path: None,
            receiver_text: None,
            scope_text: None,
            relation,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind,
            bind_method: None,
        }
    }

    fn unresolved_call_edge(source: &str, target_label: &str) -> GraphEdgeArtifact {
        GraphEdgeArtifact {
            source_stable_symbol_id: source.to_owned(),
            target_stable_symbol_id: None,
            target_label: Some(target_label.to_owned()),
            import_path: None,
            receiver_text: None,
            scope_text: None,
            relation: RelationKind::Calls,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            change_kind: None,
            edge_kind: None,
            bind_method: None,
        }
    }

    fn artifact() -> GraphIndexArtifact {
        GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "caller_a", "caller_b", "callee_a", "isolated"]
                .into_iter()
                .map(symbol)
                .collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("caller_a", "root", RelationKind::Calls),
                edge("caller_b", "root", RelationKind::Calls),
                edge("root", "caller_a", RelationKind::Calls),
                edge("root", "caller_b", RelationKind::Calls),
                edge("root", "callee_a", RelationKind::Calls),
                edge("callee_a", "caller_b", RelationKind::Calls),
                edge("root", "caller_a", RelationKind::References),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    fn wide_artifact(child_count: usize) -> GraphIndexArtifact {
        let mut symbols = vec![symbol("root")];
        let mut edges = Vec::new();
        for index in 0..child_count {
            let child_id = format!("child_{index:02}");
            symbols.push(symbol(&child_id));
            edges.push(edge("root", &child_id, RelationKind::Calls));
        }

        GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols,
            symbol_node_ids: Vec::new(),
            edges,
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        }
    }

    fn ids<'a>(symbols: &'a [&'a GraphSymbolArtifact]) -> Vec<&'a str> {
        symbols
            .iter()
            .map(|symbol| symbol.stable_symbol_id.as_str())
            .collect()
    }

    #[test]
    fn find_symbol_returns_match_and_none_for_unknown_id() {
        let artifact = artifact();

        assert_eq!(
            find_symbol(&artifact, "root").map(|symbol| symbol.entity_name.as_str()),
            Some("root")
        );
        assert!(find_symbol(&artifact, "missing").is_none());
    }

    #[test]
    fn callers_and_callees_follow_calls_and_references_edges() {
        // Fixture has a Calls edge root → caller_a AND a References edge root →
        // caller_a. Both flow through find_callees, so caller_a appears twice.
        let artifact = artifact();

        assert_eq!(
            ids(&find_callers(&artifact, "root")),
            vec!["caller_a", "caller_b"]
        );
        assert_eq!(
            ids(&find_callees(&artifact, "root")),
            vec!["caller_a", "caller_b", "callee_a", "caller_a"]
        );
        assert!(find_callers(&artifact, "missing").is_empty());
        assert!(find_callees(&artifact, "missing").is_empty());
    }

    #[test]
    fn callee_edges_include_unresolved_target_labels() {
        let mut artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "callee"].into_iter().map(symbol).collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("root", "callee", RelationKind::Calls),
                unresolved_call_edge("root", "into"),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };
        artifact
            .edges
            .push(edge("root", "callee", RelationKind::References));

        let callees = find_callee_edges(&artifact, "root");

        // 3 records: one Calls edge to `callee`, one unresolved Calls edge with
        // label "into", and one References edge to `callee` (the same symbol
        // surfaces twice because Calls and References both contribute to
        // find_callee_edges since Flaw-A's API broadening landed).
        assert_eq!(callees.len(), 3);
        assert!(matches!(
            callees[0],
            CalleeRecord::Resolved { symbol, .. } if symbol.stable_symbol_id == "callee"
        ));
        assert_eq!(
            callees[1],
            CalleeRecord::Unresolved {
                edge: &artifact.edges[1],
                target_label: "into".to_owned()
            }
        );
        assert!(matches!(
            callees[2],
            CalleeRecord::Resolved { symbol, .. } if symbol.stable_symbol_id == "callee"
        ));
    }

    #[test]
    fn symbol_with_no_edges_returns_empty_callers_and_callees() {
        let artifact = artifact();

        assert!(find_callers(&artifact, "isolated").is_empty());
        assert!(find_callees(&artifact, "isolated").is_empty());
    }

    #[test]
    fn radius_zero_subgraph_returns_just_the_root() {
        let artifact = artifact();

        let view = bounded_subgraph(&artifact, "root", 0, None, false);

        assert_eq!(ids(&view.nodes), vec!["root"]);
        assert!(view.edges.is_empty());
    }

    #[test]
    fn radius_one_subgraph_returns_immediate_neighbors_in_discovery_order() {
        let artifact = artifact();

        let view = bounded_subgraph(&artifact, "root", 1, Some(&[GraphEdgeKind::Calls]), false);

        assert_eq!(
            ids(&view.nodes),
            vec!["root", "caller_a", "caller_b", "callee_a"]
        );
        assert_eq!(view.edges.len(), 5);

        let unfiltered_view = bounded_subgraph(&artifact, "root", 1, None, false);

        assert_eq!(unfiltered_view.edges.len(), 6);
    }

    #[test]
    fn budgeted_subgraph_limits_nodes_and_returns_frontier_in_bfs_order() {
        let artifact = wide_artifact(6);

        let view = bounded_subgraph_with_budget(
            &artifact,
            &["root"],
            1,
            Some(&[GraphEdgeKind::Calls]),
            false,
            SubgraphBudget {
                max_nodes: 4,
                max_edges: 20,
            },
        );

        assert_eq!(
            ids(&view.nodes),
            vec!["root", "child_00", "child_01", "child_02"]
        );
        assert_eq!(view.edges.len(), 3);
        assert_eq!(
            view.truncated_frontier,
            vec!["child_03", "child_04", "child_05"]
        );
        assert!(view.truncated);
    }

    #[test]
    fn budgeted_subgraph_limits_edges_and_frontiers_excluded_neighbors() {
        let artifact = wide_artifact(5);

        let view = bounded_subgraph_with_budget(
            &artifact,
            &["root"],
            1,
            Some(&[GraphEdgeKind::Calls]),
            false,
            SubgraphBudget {
                max_nodes: 20,
                max_edges: 2,
            },
        );

        assert_eq!(ids(&view.nodes), vec!["root", "child_00", "child_01"]);
        assert_eq!(view.edges.len(), 2);
        assert_eq!(
            view.truncated_frontier,
            vec!["child_02", "child_03", "child_04"]
        );
        assert!(view.truncated);
    }

    #[test]
    fn edge_budget_exhaustion_does_not_frontier_returned_nodes() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "a", "b", "c"].into_iter().map(symbol).collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("root", "a", RelationKind::Calls),
                edge("root", "b", RelationKind::Calls),
                edge("a", "root", RelationKind::Calls),
                edge("root", "c", RelationKind::Calls),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let view = bounded_subgraph_with_budget(
            &artifact,
            &["root"],
            1,
            Some(&[GraphEdgeKind::Calls]),
            false,
            SubgraphBudget {
                max_nodes: 20,
                max_edges: 2,
            },
        );
        let node_ids: HashSet<_> = ids(&view.nodes).into_iter().collect();

        assert_eq!(ids(&view.nodes), vec!["root", "a", "b"]);
        assert_eq!(view.edges.len(), 2);
        assert_eq!(view.truncated_frontier, vec!["c"]);
        assert!(view
            .truncated_frontier
            .iter()
            .all(|symbol_id| !node_ids.contains(symbol_id.as_str())));
    }

    #[test]
    fn budgeted_subgraph_reports_empty_frontier_when_not_truncated() {
        let artifact = wide_artifact(3);

        let view = bounded_subgraph_with_budget(
            &artifact,
            &["root"],
            1,
            Some(&[GraphEdgeKind::Calls]),
            false,
            SubgraphBudget {
                max_nodes: 20,
                max_edges: 20,
            },
        );

        assert_eq!(
            ids(&view.nodes),
            vec!["root", "child_00", "child_01", "child_02"]
        );
        assert_eq!(view.edges.len(), 3);
        assert!(view.truncated_frontier.is_empty());
        assert!(!view.truncated);
    }

    #[test]
    fn radius_two_subgraph_fans_out_one_more_hop() {
        let artifact = artifact();

        let view = bounded_subgraph(
            &artifact,
            "caller_a",
            2,
            Some(&[GraphEdgeKind::Calls]),
            false,
        );

        assert_eq!(
            ids(&view.nodes),
            vec!["caller_a", "root", "caller_b", "callee_a"]
        );
        assert_eq!(view.edges.len(), 5);
    }

    #[test]
    fn subgraph_handles_cycles_without_revisiting_nodes_forever() {
        let artifact = artifact();

        let view = bounded_subgraph(&artifact, "root", 5, Some(&[GraphEdgeKind::Calls]), false);

        assert_eq!(
            ids(&view.nodes),
            vec!["root", "caller_a", "caller_b", "callee_a"]
        );
        assert_eq!(view.edges.len(), 6);
    }

    #[test]
    fn subgraph_for_unknown_root_is_empty() {
        let artifact = artifact();

        let view = bounded_subgraph(&artifact, "missing", 1, None, false);

        assert!(view.nodes.is_empty());
        assert!(view.edges.is_empty());
    }

    #[test]
    fn subgraph_filters_outgoing_unresolved_edges_by_default() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "callee"].into_iter().map(symbol).collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("root", "callee", RelationKind::Calls),
                unresolved_call_edge("root", "as_ref"),
                unresolved_call_edge("root", "map"),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let view = bounded_subgraph(&artifact, "root", 1, Some(&[GraphEdgeKind::Calls]), false);

        assert_eq!(ids(&view.nodes), vec!["root", "callee"]);
        assert_eq!(view.edges.len(), 1);
        assert!(view
            .edges
            .iter()
            .all(|edge| edge.target_stable_symbol_id.is_some()));
    }

    #[test]
    fn subgraph_can_include_outgoing_unresolved_edges_without_enqueueing_neighbor() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "callee"].into_iter().map(symbol).collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("root", "callee", RelationKind::Calls),
                unresolved_call_edge("root", "as_ref"),
                unresolved_call_edge("root", "map"),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let view = bounded_subgraph(&artifact, "root", 1, Some(&[GraphEdgeKind::Calls]), true);

        assert_eq!(ids(&view.nodes), vec!["root", "callee"]);
        assert_eq!(view.edges.len(), 3);
        let unresolved_labels: Vec<&str> = view
            .edges
            .iter()
            .filter(|edge| edge.target_stable_symbol_id.is_none())
            .filter_map(|edge| edge.target_label.as_deref())
            .collect();
        assert_eq!(unresolved_labels, vec!["as_ref", "map"]);
    }

    #[test]
    fn subgraph_can_include_incoming_unresolved_caller_edges() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "caller", "unresolved_caller"]
                .into_iter()
                .map(symbol)
                .collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("caller", "root", RelationKind::Calls),
                unresolved_call_edge("unresolved_caller", "root"),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let default_view =
            bounded_subgraph(&artifact, "root", 1, Some(&[GraphEdgeKind::Calls]), false);
        assert_eq!(ids(&default_view.nodes), vec!["root", "caller"]);
        assert_eq!(default_view.edges.len(), 1);

        let included_view =
            bounded_subgraph(&artifact, "root", 1, Some(&[GraphEdgeKind::Calls]), true);

        assert_eq!(
            ids(&included_view.nodes),
            vec!["root", "caller", "unresolved_caller"]
        );
        assert_eq!(included_view.edges.len(), 2);
        assert!(included_view.edges.iter().any(|edge| {
            edge.source_stable_symbol_id == "unresolved_caller"
                && edge.target_stable_symbol_id.is_none()
                && edge.target_label.as_deref() == Some("root")
        }));
    }

    #[test]
    fn unresolved_edge_does_not_advance_radius_depth() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root"].into_iter().map(symbol).collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![unresolved_call_edge("root", "external")],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let r0 = bounded_subgraph(&artifact, "root", 0, None, true);
        assert!(r0.edges.is_empty(), "radius 0 must skip edge scanning");

        let r1 = bounded_subgraph(&artifact, "root", 1, None, true);
        assert_eq!(r1.edges.len(), 1);
        assert_eq!(ids(&r1.nodes), vec!["root"]);
    }

    #[test]
    fn callers_and_callees_include_references_edges() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["caller", "target"].into_iter().map(symbol).collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![edge("caller", "target", RelationKind::References)],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        assert_eq!(ids(&find_callers(&artifact, "target")), vec!["caller"]);
        assert_eq!(ids(&find_callees(&artifact, "caller")), vec!["target"]);

        let view = bounded_subgraph(&artifact, "target", 1, None, false);
        assert_eq!(ids(&view.nodes), vec!["target", "caller"]);
    }

    #[test]
    fn subgraph_edge_kind_filter_is_strict_for_static_calls() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "direct", "dyn_trait", "hof"]
                .into_iter()
                .map(symbol)
                .collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("root", "direct", RelationKind::Calls),
                edge_with_kind(
                    "root",
                    "dyn_trait",
                    RelationKind::Calls,
                    Some(GraphEdgeKind::CallsDyn),
                ),
                edge_with_kind(
                    "root",
                    "hof",
                    RelationKind::References,
                    Some(GraphEdgeKind::ReferencesHof),
                ),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let static_calls =
            bounded_subgraph(&artifact, "root", 1, Some(&[GraphEdgeKind::Calls]), false);
        assert_eq!(ids(&static_calls.nodes), vec!["root", "direct"]);
        assert_eq!(static_calls.edges.len(), 1);
        assert_eq!(edge_kind(static_calls.edges[0]), GraphEdgeKind::Calls);

        let dyn_calls = bounded_subgraph(
            &artifact,
            "root",
            1,
            Some(&[GraphEdgeKind::CallsDyn]),
            false,
        );
        assert_eq!(ids(&dyn_calls.nodes), vec!["root", "dyn_trait"]);
        assert_eq!(dyn_calls.edges.len(), 1);
        assert_eq!(edge_kind(dyn_calls.edges[0]), GraphEdgeKind::CallsDyn);
    }

    #[test]
    fn legacy_reference_edges_without_edge_kind_default_to_references_other() {
        let legacy_reference = edge("root", "callee", RelationKind::References);

        assert_eq!(edge_kind(&legacy_reference), GraphEdgeKind::ReferencesOther);
    }

    #[test]
    fn caller_and_callee_records_carry_edge_kind_for_all_public_values() {
        let artifact = GraphIndexArtifact {
            rust_evidence: None,
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: "test".to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "test".to_owned(),
            graph_content_hash: "test".to_owned(),
            file_manifests: Vec::new(),
            files: Vec::new(),
            file_node_ids: Vec::new(),
            symbols: ["root", "direct", "dyn_trait", "hof", "other"]
                .into_iter()
                .map(symbol)
                .collect(),
            symbol_node_ids: Vec::new(),
            edges: vec![
                edge("root", "direct", RelationKind::Calls),
                edge_with_kind(
                    "root",
                    "dyn_trait",
                    RelationKind::Calls,
                    Some(GraphEdgeKind::CallsDyn),
                ),
                edge_with_kind(
                    "root",
                    "hof",
                    RelationKind::References,
                    Some(GraphEdgeKind::ReferencesHof),
                ),
                edge_with_kind(
                    "root",
                    "other",
                    RelationKind::References,
                    Some(GraphEdgeKind::ReferencesOther),
                ),
            ],
            tombstones: Vec::new(),
            diagnostics: Vec::new(),
            commits: Vec::new(),
            symbol_snapshots: Vec::new(),
            temporal_edges: Vec::new(),
        };

        let callee_kinds: Vec<_> = find_callee_edges(&artifact, "root")
            .into_iter()
            .map(|record| record.edge_kind())
            .collect();
        assert_eq!(
            callee_kinds,
            vec![
                GraphEdgeKind::Calls,
                GraphEdgeKind::CallsDyn,
                GraphEdgeKind::ReferencesHof,
                GraphEdgeKind::ReferencesOther,
            ]
        );

        let caller_records = find_caller_edges(&artifact, "dyn_trait");
        assert_eq!(caller_records.len(), 1);
        assert_eq!(caller_records[0].edge_kind(), GraphEdgeKind::CallsDyn);
        assert!(matches!(
            caller_records[0],
            CallerRecord::Resolved { caller, .. } if caller.stable_symbol_id == "root"
        ));
    }
}
