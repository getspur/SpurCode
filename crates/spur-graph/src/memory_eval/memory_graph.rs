//! Question-blind memory graph construction and traversal rankers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Confidence, EdgeId, EvidenceId, GraphEdge, GraphFacts, GraphNode, NodeId, NodeKind,
    RelationKind, RunId,
};

use super::ranking::{
    Bm25Ranker, CorpusDocument, Granularity, RankRequest, RankedHit, Ranker, Ranking, Variant,
};

/// A question-blind turn view used to construct memory graph facts.
///
/// The type can carry only the shared corpus document and source speaker. It
/// has no place for a question, answer, evidence label, question type, or
/// `has_answer` value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryTurn {
    pub document: CorpusDocument,
    pub speaker: Option<String>,
}

/// A question-blind canonical session occurrence and its ordered turns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySession {
    pub document: CorpusDocument,
    pub turns: Vec<MemoryTurn>,
}

/// Relations that may be explicitly enabled for query-time traversal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRelation {
    Contains,
    NextTurn,
    PreviousTurn,
    SpokenBy,
}

/// Complete traversal policy recorded by the benchmark manifest.
///
/// This type intentionally has no [`Default`] implementation and no serde
/// defaults. Callers must provide and serialize every traversal choice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TraversalConfig {
    pub seed_k: usize,
    pub max_depth: usize,
    pub relations: BTreeSet<MemoryRelation>,
}

/// Corpus-only graph build timing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphBuildTelemetry {
    pub graph_build_nanoseconds: u128,
}

/// Graph and lexical-index build telemetry for one graph ranker.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexBuildTelemetry {
    pub graph_build_nanoseconds: u128,
    pub index_build_nanoseconds: u128,
    /// Exact byte length of the deterministic graph-index serialization.
    pub index_size_bytes: usize,
}

/// Telemetry for one rank request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryTelemetry {
    pub query_nanoseconds: u128,
    pub scored_nodes: usize,
    pub visited_nodes: usize,
    pub traversed_edges: usize,
}

#[derive(Debug, Clone, Serialize)]
struct CanonicalProvenance {
    occurrence_id: String,
    granularity: Granularity,
}

/// A deterministic memory graph built only from canonical corpus records.
#[derive(Debug, Clone)]
pub struct MemoryGraph {
    pub facts: GraphFacts,
    provenance: BTreeMap<NodeId, CanonicalProvenance>,
    node_by_stable_key: BTreeMap<String, NodeId>,
    relations: Vec<MemoryRelation>,
    adjacency: BTreeMap<(NodeId, MemoryRelation), Vec<NodeId>>,
    session_corpus: Vec<CorpusDocument>,
    turn_corpus: Vec<CorpusDocument>,
    content_hash: String,
    build_telemetry: GraphBuildTelemetry,
}

impl MemoryGraph {
    /// Build corpus-only session, turn, speaker, and chronology facts.
    pub fn build(sessions: &[MemorySession]) -> Result<Self> {
        let started = Instant::now();
        let mut builder = MemoryGraphBuilder::default();
        let mut occurrence_ids = BTreeSet::new();
        let mut session_corpus = Vec::with_capacity(sessions.len());
        let mut turn_corpus = Vec::new();

        for session in sessions {
            validate_occurrence_id(&session.document.occurrence_id, &mut occurrence_ids)?;
            session_corpus.push(session.document.clone());

            let session_key = format!("memory:session:{}", session.document.occurrence_id);
            let chronology_label = session
                .document
                .chronology_key
                .map(|key| serde_json::to_string(&key))
                .transpose()?
                .unwrap_or_default();
            let session_label = if chronology_label.is_empty() {
                session.document.text.clone()
            } else {
                format!("{} {chronology_label}", session.document.text)
            };
            let session_node = builder.add_node(session_key, session_label);
            builder.add_provenance(
                session_node,
                &session.document.occurrence_id,
                Granularity::Session,
            );

            if !chronology_label.is_empty() {
                let chronology_node = builder.add_node(
                    format!("memory:chronology:{}", session.document.occurrence_id),
                    chronology_label,
                );
                builder.add_provenance(
                    chronology_node,
                    &session.document.occurrence_id,
                    Granularity::Session,
                );
                builder.add_edge(session_node, chronology_node, MemoryRelation::Contains);
            }

            let mut previous_turn = None;
            for turn in &session.turns {
                validate_occurrence_id(&turn.document.occurrence_id, &mut occurrence_ids)?;
                turn_corpus.push(turn.document.clone());

                let turn_label = match turn.speaker.as_deref() {
                    Some(speaker) => format!("{} {speaker}", turn.document.text),
                    None => turn.document.text.clone(),
                };
                let turn_node = builder.add_node(
                    format!("memory:turn:{}", turn.document.occurrence_id),
                    turn_label,
                );
                builder.add_provenance(turn_node, &turn.document.occurrence_id, Granularity::Turn);
                builder.add_edge(session_node, turn_node, MemoryRelation::Contains);

                if let Some(previous_node) = previous_turn {
                    builder.add_edge(previous_node, turn_node, MemoryRelation::NextTurn);
                    builder.add_edge(turn_node, previous_node, MemoryRelation::PreviousTurn);
                }
                previous_turn = Some(turn_node);

                if let Some(speaker) = &turn.speaker {
                    let speaker_key = format!("memory:speaker:{speaker}");
                    let speaker_node = match builder.node_by_stable_key.get(&speaker_key) {
                        Some(node_id) => *node_id,
                        None => builder.add_node(speaker_key, speaker.clone()),
                    };
                    builder.add_edge(turn_node, speaker_node, MemoryRelation::SpokenBy);
                }
            }
        }

        for neighbors in builder.adjacency.values_mut() {
            neighbors.sort_unstable();
            neighbors.dedup();
        }

        let facts = GraphFacts {
            nodes: builder.nodes,
            edges: builder.edges,
            spans: Vec::new(),
            macro_invocations: Vec::new(),
        };
        let content_hash = graph_content_hash(
            &facts,
            &builder.relations,
            &builder.provenance,
            &session_corpus,
            &turn_corpus,
        )?;

        Ok(Self {
            facts,
            provenance: builder.provenance,
            node_by_stable_key: builder.node_by_stable_key,
            relations: builder.relations,
            adjacency: builder.adjacency,
            session_corpus,
            turn_corpus,
            content_hash,
            build_telemetry: GraphBuildTelemetry {
                graph_build_nanoseconds: started.elapsed().as_nanos(),
            },
        })
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn corpus(&self, granularity: Granularity) -> &[CorpusDocument] {
        match granularity {
            Granularity::Turn => &self.turn_corpus,
            Granularity::Session => &self.session_corpus,
        }
    }

    pub fn relations(&self) -> impl Iterator<Item = MemoryRelation> + '_ {
        self.relations.iter().copied()
    }

    pub const fn build_telemetry(&self) -> GraphBuildTelemetry {
        self.build_telemetry
    }

    /// Resolve a graph stable key to exactly one canonical provenance unit.
    pub fn resolve_provenance(&self, stable_key: &str) -> Option<(&str, Granularity)> {
        let node_id = self.node_by_stable_key.get(stable_key)?;
        let provenance = self.provenance.get(node_id)?;
        Some((provenance.occurrence_id.as_str(), provenance.granularity))
    }

    fn provenance(&self, node_id: NodeId) -> Option<&CanonicalProvenance> {
        self.provenance.get(&node_id)
    }

    fn stable_key(&self, node_id: NodeId) -> Option<&str> {
        self.facts
            .nodes
            .get(usize::try_from(node_id.get()).ok()?.checked_sub(1)?)
            .map(|node| node.stable_key.as_str())
    }

    fn neighbors(&self, node_id: NodeId, relation: MemoryRelation) -> &[NodeId] {
        self.adjacency
            .get(&(node_id, relation))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct MemoryGraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    provenance: BTreeMap<NodeId, CanonicalProvenance>,
    node_by_stable_key: BTreeMap<String, NodeId>,
    relations: Vec<MemoryRelation>,
    adjacency: BTreeMap<(NodeId, MemoryRelation), Vec<NodeId>>,
}

impl MemoryGraphBuilder {
    fn add_node(&mut self, stable_key: String, label: String) -> NodeId {
        let node_id = NodeId(u64::try_from(self.nodes.len() + 1).expect("node count fits u64"));
        self.node_by_stable_key.insert(stable_key.clone(), node_id);
        self.nodes.push(GraphNode {
            node_id,
            stable_key,
            label,
            kind: NodeKind::Resource,
            file_id: None,
            source_span_id: None,
            first_seen_run_id: RunId(0),
        });
        node_id
    }

    fn add_provenance(&mut self, node_id: NodeId, occurrence_id: &str, granularity: Granularity) {
        self.provenance.insert(
            node_id,
            CanonicalProvenance {
                occurrence_id: occurrence_id.to_owned(),
                granularity,
            },
        );
    }

    fn add_edge(&mut self, source: NodeId, target: NodeId, relation: MemoryRelation) {
        let edge_id = EdgeId(u64::try_from(self.edges.len() + 1).expect("edge count fits u64"));
        let directed = !matches!(
            relation,
            MemoryRelation::Contains | MemoryRelation::SpokenBy
        );
        self.edges.push(GraphEdge {
            edge_id,
            source_node_id: source,
            target_node_id: Some(target),
            relation: native_relation(relation),
            target_label: None,
            import_path: None,
            receiver_text: None,
            scope_text: None,
            confidence: Confidence::SyntaxExact,
            confidence_score: 1.0,
            edge_kind: None,
            bind_method: Some("memory_corpus".to_owned()),
            evidence_id: EvidenceId(edge_id.get()),
            directed,
            change_kind: None,
        });
        self.relations.push(relation);
        self.adjacency
            .entry((source, relation))
            .or_default()
            .push(target);
        if !directed {
            self.adjacency
                .entry((target, relation))
                .or_default()
                .push(source);
        }
    }
}

fn validate_occurrence_id(id: &str, seen: &mut BTreeSet<String>) -> Result<()> {
    ensure!(!id.is_empty(), "corpus occurrence IDs must not be empty");
    ensure!(seen.insert(id.to_owned()), "duplicate occurrence ID `{id}`");
    Ok(())
}

const fn native_relation(relation: MemoryRelation) -> RelationKind {
    match relation {
        MemoryRelation::Contains => RelationKind::Contains,
        MemoryRelation::NextTurn | MemoryRelation::PreviousTurn => RelationKind::Links,
        MemoryRelation::SpokenBy => RelationKind::Binds,
    }
}

fn graph_content_hash(
    facts: &GraphFacts,
    relations: &[MemoryRelation],
    provenance: &BTreeMap<NodeId, CanonicalProvenance>,
    session_corpus: &[CorpusDocument],
    turn_corpus: &[CorpusDocument],
) -> Result<String> {
    let provenance = provenance.iter().collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&(
        &facts.nodes,
        &facts.edges,
        relations,
        provenance,
        session_corpus,
        turn_corpus,
    ))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, Clone)]
struct GraphLexicalIndex {
    ranker: Bm25Ranker,
    node_by_document_id: BTreeMap<String, NodeId>,
    documents: Vec<CorpusDocument>,
    session_hash_ranker: Bm25Ranker,
    turn_hash_ranker: Bm25Ranker,
    index_build_nanoseconds: u128,
    index_size_bytes: usize,
}

impl GraphLexicalIndex {
    fn build(graph: &MemoryGraph) -> Result<Self> {
        let started = Instant::now();
        let documents = graph
            .facts
            .nodes
            .iter()
            .map(|node| CorpusDocument {
                occurrence_id: node.stable_key.clone(),
                text: node.label.clone(),
                chronology_key: None,
            })
            .collect::<Vec<_>>();
        let index_size_bytes = serde_json::to_vec(&documents)?.len();
        let node_by_document_id = graph
            .facts
            .nodes
            .iter()
            .map(|node| (node.stable_key.clone(), node.node_id))
            .collect();
        let ranker = Bm25Ranker::build(documents.clone())?;
        let session_hash_ranker = Bm25Ranker::build(graph.session_corpus.clone())?;
        let turn_hash_ranker = Bm25Ranker::build(graph.turn_corpus.clone())?;

        Ok(Self {
            ranker,
            node_by_document_id,
            documents,
            session_hash_ranker,
            turn_hash_ranker,
            index_build_nanoseconds: started.elapsed().as_nanos(),
            index_size_bytes,
        })
    }

    fn rank_nodes(&self, request: &RankRequest<'_>, k: usize) -> Result<Ranking> {
        let graph_request = RankRequest {
            query: request.query,
            granularity: request.granularity,
            corpus: &self.documents,
        };
        self.ranker.rank(&graph_request, k)
    }

    fn ranking_metadata(&self, request: &RankRequest<'_>) -> Result<Ranking> {
        match request.granularity {
            Granularity::Turn => self.turn_hash_ranker.rank(request, 0),
            Granularity::Session => self.session_hash_ranker.rank(request, 0),
        }
    }

    fn node_id(&self, stable_key: &str) -> Option<NodeId> {
        self.node_by_document_id.get(stable_key).copied()
    }
}

/// Graph-derived lexical index with no query-time edge access.
#[derive(Debug, Clone)]
pub struct GraphIndexOnlyRanker {
    graph: MemoryGraph,
    index: GraphLexicalIndex,
    build_telemetry: IndexBuildTelemetry,
}

impl GraphIndexOnlyRanker {
    pub fn new(graph: MemoryGraph) -> Result<Self> {
        let index = GraphLexicalIndex::build(&graph)?;
        let build_telemetry = IndexBuildTelemetry {
            graph_build_nanoseconds: graph.build_telemetry.graph_build_nanoseconds,
            index_build_nanoseconds: index.index_build_nanoseconds,
            index_size_bytes: index.index_size_bytes,
        };
        Ok(Self {
            graph,
            index,
            build_telemetry,
        })
    }

    pub const fn graph(&self) -> &MemoryGraph {
        &self.graph
    }

    pub const fn build_telemetry(&self) -> IndexBuildTelemetry {
        self.build_telemetry
    }

    pub fn rank_with_telemetry(
        &self,
        request: &RankRequest<'_>,
        k: usize,
    ) -> Result<(Ranking, QueryTelemetry)> {
        let started = Instant::now();
        ensure_request_corpus(&self.graph, request)?;
        let node_ranking = self
            .index
            .rank_nodes(request, self.graph.facts.nodes.len())?;
        let mut seen = BTreeSet::new();
        let mut hits = Vec::new();

        for node_hit in node_ranking.hits {
            if hits.len() == k {
                break;
            }
            let Some(node_id) = self.index.node_id(&node_hit.occurrence_id) else {
                continue;
            };
            let Some(provenance) = self.graph.provenance(node_id) else {
                continue;
            };
            if provenance.granularity != request.granularity
                || !seen.insert(provenance.occurrence_id.clone())
            {
                continue;
            }
            hits.push(RankedHit {
                occurrence_id: provenance.occurrence_id.clone(),
                provenance_id: Some(node_hit.occurrence_id),
                score: node_hit.score,
            });
        }

        let ranking = finish_ranking(
            self.index.ranking_metadata(request)?,
            Variant::GraphIndexOnly,
            k,
            hits,
        );
        Ok((
            ranking,
            QueryTelemetry {
                query_nanoseconds: started.elapsed().as_nanos(),
                scored_nodes: self.graph.facts.nodes.len(),
                visited_nodes: 0,
                traversed_edges: 0,
            },
        ))
    }
}

impl Ranker for GraphIndexOnlyRanker {
    fn variant(&self) -> Variant {
        Variant::GraphIndexOnly
    }

    fn rank(&self, request: &RankRequest<'_>, k: usize) -> Result<Ranking> {
        self.rank_with_telemetry(request, k)
            .map(|(ranking, _)| ranking)
    }
}

/// BM25-seeded graph traversal under an explicit recorded relation policy.
#[derive(Debug, Clone)]
pub struct GraphTraversalRanker {
    graph: MemoryGraph,
    index: GraphLexicalIndex,
    config: TraversalConfig,
    build_telemetry: IndexBuildTelemetry,
}

impl GraphTraversalRanker {
    pub fn new(graph: MemoryGraph, config: TraversalConfig) -> Result<Self> {
        let index = GraphLexicalIndex::build(&graph)?;
        let build_telemetry = IndexBuildTelemetry {
            graph_build_nanoseconds: graph.build_telemetry.graph_build_nanoseconds,
            index_build_nanoseconds: index.index_build_nanoseconds,
            index_size_bytes: index.index_size_bytes,
        };
        Ok(Self {
            graph,
            index,
            config,
            build_telemetry,
        })
    }

    pub const fn graph(&self) -> &MemoryGraph {
        &self.graph
    }

    pub const fn config(&self) -> &TraversalConfig {
        &self.config
    }

    pub const fn build_telemetry(&self) -> IndexBuildTelemetry {
        self.build_telemetry
    }

    pub fn rank_with_telemetry(
        &self,
        request: &RankRequest<'_>,
        k: usize,
    ) -> Result<(Ranking, QueryTelemetry)> {
        let started = Instant::now();
        ensure_request_corpus(&self.graph, request)?;
        let seeds = self.index.rank_nodes(request, self.config.seed_k)?;
        let mut best = BTreeMap::<String, TraversalCandidate>::new();
        let mut visited_nodes = BTreeSet::new();
        let mut traversed_edges = 0usize;

        for seed in seeds.hits {
            let Some(seed_node) = self.index.node_id(&seed.occurrence_id) else {
                continue;
            };
            let mut queue = VecDeque::from([(seed_node, 0usize)]);
            let mut seed_visited = BTreeSet::from([seed_node]);

            while let Some((node_id, distance)) = queue.pop_front() {
                visited_nodes.insert(node_id);
                if let (Some(provenance), Some(stable_key)) = (
                    self.graph.provenance(node_id),
                    self.graph.stable_key(node_id),
                ) {
                    if provenance.granularity == request.granularity {
                        let candidate = TraversalCandidate {
                            occurrence_id: provenance.occurrence_id.clone(),
                            graph_node_id: stable_key.to_owned(),
                            seed_score: seed.score,
                            distance,
                        };
                        match best.get(&candidate.occurrence_id) {
                            Some(current) if !candidate.is_better_than(current) => {}
                            _ => {
                                best.insert(candidate.occurrence_id.clone(), candidate);
                            }
                        }
                    }
                }

                if distance == self.config.max_depth {
                    continue;
                }
                for relation in &self.config.relations {
                    let neighbors = self.graph.neighbors(node_id, *relation);
                    traversed_edges += neighbors.len();
                    for neighbor in neighbors {
                        if seed_visited.insert(*neighbor) {
                            queue.push_back((*neighbor, distance + 1));
                        }
                    }
                }
            }
        }

        let mut candidates = best.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .seed_score
                .total_cmp(&left.seed_score)
                .then_with(|| left.distance.cmp(&right.distance))
                .then_with(|| left.occurrence_id.cmp(&right.occurrence_id))
                .then_with(|| left.graph_node_id.cmp(&right.graph_node_id))
        });
        let hits = candidates
            .into_iter()
            .take(k)
            .map(|candidate| RankedHit {
                occurrence_id: candidate.occurrence_id,
                provenance_id: Some(candidate.graph_node_id),
                score: candidate.seed_score,
            })
            .collect();
        let ranking = finish_ranking(
            self.index.ranking_metadata(request)?,
            Variant::GraphTraversal,
            k,
            hits,
        );

        Ok((
            ranking,
            QueryTelemetry {
                query_nanoseconds: started.elapsed().as_nanos(),
                scored_nodes: self.graph.facts.nodes.len(),
                visited_nodes: visited_nodes.len(),
                traversed_edges,
            },
        ))
    }
}

impl Ranker for GraphTraversalRanker {
    fn variant(&self) -> Variant {
        Variant::GraphTraversal
    }

    fn rank(&self, request: &RankRequest<'_>, k: usize) -> Result<Ranking> {
        self.rank_with_telemetry(request, k)
            .map(|(ranking, _)| ranking)
    }
}

#[derive(Debug)]
struct TraversalCandidate {
    occurrence_id: String,
    graph_node_id: String,
    seed_score: f64,
    distance: usize,
}

impl TraversalCandidate {
    fn is_better_than(&self, other: &Self) -> bool {
        self.seed_score > other.seed_score
            || (self.seed_score == other.seed_score
                && (self.distance < other.distance
                    || (self.distance == other.distance
                        && self.graph_node_id < other.graph_node_id)))
    }
}

fn ensure_request_corpus(graph: &MemoryGraph, request: &RankRequest<'_>) -> Result<()> {
    ensure!(
        request.corpus == graph.corpus(request.granularity),
        "rank request corpus does not match the memory graph corpus"
    );
    Ok(())
}

fn finish_ranking(
    mut ranking: Ranking,
    variant: Variant,
    k: usize,
    hits: Vec<RankedHit>,
) -> Ranking {
    ranking.variant = variant;
    ranking.k = k;
    ranking.hits = hits;
    ranking
}
