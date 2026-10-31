use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::schema::{
    ChangeKind, CommitIndexArtifact, EdgeEndpoint, GraphIndexArtifact, RelationKind, RenamePrev,
    SnapshotKey, StableSymbolId, TemporalEdgeArtifact,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<T> {
    Found { value: T, chain: Vec<SnapshotKey> },
    Deleted { last_seen: SnapshotKey },
    Ambiguous { candidates: Vec<T> },
    Unknown { reason: ResolutionFailure },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionFailure {
    AnchorCommitNotIndexed(String),
    SymbolNotPresentAtAnchor,
    IndexCorrupt(String),
}

pub type GitSha = String;

const MAX_ANCESTOR_CACHE_ENTRIES: usize = 256;

#[derive(Debug, Default)]
struct AncestorCache {
    sets: HashMap<String, Arc<HashSet<String>>>,
    lru: VecDeque<String>,
}

impl AncestorCache {
    fn get(&mut self, commit: &str) -> Option<Arc<HashSet<String>>> {
        let ancestors = Arc::clone(self.sets.get(commit)?);
        self.touch(commit);
        Some(ancestors)
    }

    fn insert(&mut self, commit: String, ancestors: Arc<HashSet<String>>) -> Arc<HashSet<String>> {
        if self.sets.contains_key(&commit) {
            self.sets.insert(commit.clone(), Arc::clone(&ancestors));
            self.touch(&commit);
            return ancestors;
        }

        self.sets.insert(commit.clone(), Arc::clone(&ancestors));
        self.lru.push_back(commit);
        while self.sets.len() > MAX_ANCESTOR_CACHE_ENTRIES {
            if let Some(expired) = self.lru.pop_front() {
                self.sets.remove(&expired);
            }
        }
        ancestors
    }

    fn touch(&mut self, commit: &str) {
        self.lru.retain(|cached| cached != commit);
        self.lru.push_back(commit.to_owned());
    }

    fn size_bytes(&self) -> usize {
        use std::mem::size_of;

        let map_bytes =
            self.sets.capacity() * (size_of::<String>() + size_of::<Arc<HashSet<String>>>());
        let lru_bytes = self.lru.capacity() * size_of::<String>();
        let set_bytes = self
            .sets
            .values()
            .map(|ancestors| ancestors.capacity() * size_of::<String>())
            .sum::<usize>();
        map_bytes + lru_bytes + set_bytes
    }
}

#[derive(Debug)]
pub struct TemporalIndex {
    code: Arc<GraphIndexArtifact>,
    edges_by_stable_symbol_id: HashMap<String, Vec<usize>>,
    edges_by_commit_sha: HashMap<String, Vec<usize>>,
    commit_positions: HashMap<String, usize>,
    commit_parents: HashMap<String, Vec<String>>,
    snapshot_keys_by_stable_symbol_id: HashMap<String, Vec<SnapshotKey>>,
    deleted_snapshot_keys: HashSet<SnapshotKey>,
    rename_edges: Vec<(SnapshotKey, SnapshotKey)>,
    rename_neighbors_by_snapshot: HashMap<SnapshotKey, Vec<SnapshotKey>>,
    referenced_snapshot_count: usize,
    ancestor_cache: Mutex<AncestorCache>,
}

impl TemporalIndex {
    pub fn new(code: Arc<GraphIndexArtifact>) -> Self {
        let mut edges_by_stable_symbol_id: HashMap<String, Vec<usize>> = HashMap::new();
        let mut edges_by_commit_sha: HashMap<String, Vec<usize>> = HashMap::new();
        let commit_positions = code
            .commits
            .iter()
            .enumerate()
            .map(|(index, commit)| (commit.sha.clone(), index))
            .collect();
        let commit_parents = code
            .commits
            .iter()
            .map(|commit| (commit.sha.clone(), commit.parents.clone()))
            .collect();
        let mut snapshot_key_sets: HashMap<String, HashSet<SnapshotKey>> = HashMap::new();
        let mut deleted_snapshot_keys = HashSet::new();
        let mut referenced_snapshot_count = 0;
        let mut rename_edges = Vec::new();

        for snapshot in &code.symbol_snapshots {
            insert_snapshot_key(
                &mut snapshot_key_sets,
                snapshot.key.stable_symbol_id.as_str(),
                snapshot.key.clone(),
                &mut referenced_snapshot_count,
            );
        }

        for (edge_index, edge) in code.temporal_edges.iter().enumerate() {
            let mut stable_symbol_ids = Vec::new();
            index_endpoint(
                &edge.source,
                &mut stable_symbol_ids,
                &mut snapshot_key_sets,
                &mut referenced_snapshot_count,
            );
            index_endpoint(
                &edge.target,
                &mut stable_symbol_ids,
                &mut snapshot_key_sets,
                &mut referenced_snapshot_count,
            );

            if let Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(prev))) = &edge.change_kind {
                push_unique_stable_symbol_id(&mut stable_symbol_ids, &prev.stable_symbol_id);
                insert_snapshot_key(
                    &mut snapshot_key_sets,
                    prev.stable_symbol_id.as_str(),
                    prev.clone(),
                    &mut referenced_snapshot_count,
                );
                if let Some(next) = rename_target(edge, prev) {
                    rename_edges.push((prev.clone(), next));
                }
            }

            for stable_symbol_id in stable_symbol_ids {
                edges_by_stable_symbol_id
                    .entry(stable_symbol_id.to_owned())
                    .or_default()
                    .push(edge_index);
            }
            if let EdgeEndpoint::Commit { sha } = &edge.source {
                edges_by_commit_sha
                    .entry(sha.clone())
                    .or_default()
                    .push(edge_index);
            }
            if let EdgeEndpoint::Commit { sha } = &edge.target {
                edges_by_commit_sha
                    .entry(sha.clone())
                    .or_default()
                    .push(edge_index);
            }
            if let (
                EdgeEndpoint::Commit { .. },
                EdgeEndpoint::Snapshot { key },
                Some(ChangeKind::Deleted),
            ) = (&edge.source, &edge.target, &edge.change_kind)
            {
                deleted_snapshot_keys.insert(key.clone());
            }
        }

        let mut snapshot_keys_by_stable_symbol_id = HashMap::new();
        for (stable_symbol_id, keys) in snapshot_key_sets {
            let mut keys: Vec<_> = keys.into_iter().collect();
            keys.sort_by(|left, right| {
                left.commit
                    .cmp(&right.commit)
                    .then_with(|| left.stable_symbol_id.cmp(&right.stable_symbol_id))
            });
            snapshot_keys_by_stable_symbol_id.insert(stable_symbol_id, keys);
        }

        let mut rename_neighbors_by_snapshot: HashMap<SnapshotKey, Vec<SnapshotKey>> =
            HashMap::new();
        for (prev, next) in &rename_edges {
            rename_neighbors_by_snapshot
                .entry(prev.clone())
                .or_default()
                .push(next.clone());
            rename_neighbors_by_snapshot
                .entry(next.clone())
                .or_default()
                .push(prev.clone());
        }

        Self {
            code,
            edges_by_stable_symbol_id,
            edges_by_commit_sha,
            commit_positions,
            commit_parents,
            snapshot_keys_by_stable_symbol_id,
            deleted_snapshot_keys,
            rename_edges,
            rename_neighbors_by_snapshot,
            referenced_snapshot_count,
            // Bounded LRU avoids the worst-case O(commits^2) memory growth of all ancestor sets.
            ancestor_cache: Mutex::new(AncestorCache::default()),
        }
    }

    pub fn artifact(&self) -> &GraphIndexArtifact {
        self.code.as_ref()
    }

    pub fn commit_graph(&self) -> CommitGraph<'_> {
        CommitGraph::from_borrowed(&self.commit_positions, &self.commit_parents)
    }

    pub fn ancestors_of(&self, commit: &str) -> Option<Arc<HashSet<String>>> {
        let graph = self.commit_graph();
        self.ancestors_of_with_graph(&graph, commit)
    }

    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let graph = self.commit_graph();
        self.is_ancestor_with_graph(&graph, ancestor, descendant)
    }

    pub fn edges_for_stable_symbol_id(
        &self,
        stable_symbol_id: &str,
    ) -> impl Iterator<Item = &TemporalEdgeArtifact> + '_ {
        self.edges_by_stable_symbol_id
            .get(stable_symbol_id)
            .into_iter()
            .flat_map(|edge_indices| {
                edge_indices
                    .iter()
                    .map(|&edge_index| &self.code.temporal_edges[edge_index])
            })
    }

    pub fn edges_for_commit_sha(
        &self,
        commit_sha: &str,
    ) -> impl Iterator<Item = &TemporalEdgeArtifact> + '_ {
        self.edges_by_commit_sha
            .get(commit_sha)
            .into_iter()
            .flat_map(|edge_indices| {
                edge_indices
                    .iter()
                    .map(|&edge_index| &self.code.temporal_edges[edge_index])
            })
    }

    fn has_commit_positions(&self) -> bool {
        !self.commit_positions.is_empty()
    }

    pub fn index_size_bytes(&self) -> usize {
        use std::mem::size_of;

        let edges_by_id = self.edges_by_stable_symbol_id.capacity()
            * (size_of::<String>() + size_of::<Vec<usize>>());
        let edges_by_sha =
            self.edges_by_commit_sha.capacity() * (size_of::<String>() + size_of::<Vec<usize>>());
        let positions =
            self.commit_positions.capacity() * (size_of::<String>() + size_of::<usize>());
        let snapshot_keys = self.snapshot_keys_by_stable_symbol_id.capacity()
            * (size_of::<String>() + size_of::<Vec<SnapshotKey>>());
        let deleted_snapshot_keys =
            self.deleted_snapshot_keys.capacity() * size_of::<SnapshotKey>();
        let rename_neighbors = self.rename_neighbors_by_snapshot.capacity()
            * (size_of::<SnapshotKey>() + size_of::<Vec<SnapshotKey>>());
        let rename_edges_bytes =
            self.rename_edges.capacity() * size_of::<(SnapshotKey, SnapshotKey)>();
        let ancestor_cache_bytes =
            size_of::<Mutex<AncestorCache>>() + self.lock_ancestor_cache().size_bytes();

        edges_by_id
            + edges_by_sha
            + positions
            + snapshot_keys
            + deleted_snapshot_keys
            + rename_neighbors
            + rename_edges_bytes
            + ancestor_cache_bytes
    }

    pub fn is_deleted_snapshot(&self, snapshot: &SnapshotKey) -> bool {
        self.deleted_snapshot_keys.contains(snapshot)
    }

    fn commit_position(&self, commit: &str) -> usize {
        self.commit_positions
            .get(commit)
            .copied()
            .unwrap_or(usize::MAX)
    }

    fn ancestors_of_with_graph(
        &self,
        graph: &CommitGraph<'_>,
        commit: &str,
    ) -> Option<Arc<HashSet<String>>> {
        if let Some(ancestors) = self.lock_ancestor_cache().get(commit) {
            return Some(ancestors);
        }

        let ancestors = Arc::new(graph.ancestors_of(commit)?);
        let mut cache = self.lock_ancestor_cache();
        Some(cache.insert(commit.to_owned(), ancestors))
    }

    fn is_ancestor_with_graph(
        &self,
        graph: &CommitGraph<'_>,
        ancestor: &str,
        descendant: &str,
    ) -> bool {
        if ancestor == descendant {
            return graph.contains(ancestor);
        }
        if !graph.contains(ancestor) || !graph.contains(descendant) {
            return false;
        }

        // Commits are indexed oldest-first via `git rev-list --topo-order --reverse`,
        // so an ancestor with a higher position than its descendant cannot precede it.
        // Keep this fast-reject before cache lookup to avoid storing impossible pairs.
        if graph.position(ancestor) > graph.position(descendant) {
            return false;
        }

        self.ancestors_of_with_graph(graph, descendant)
            .is_some_and(|ancestors| ancestors.contains(ancestor))
    }

    fn lock_ancestor_cache(&self) -> MutexGuard<'_, AncestorCache> {
        self.ancestor_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub enum TemporalHistorySource<'a> {
    Artifact(&'a GraphIndexArtifact),
    Index(&'a TemporalIndex),
}

impl<'a> From<&'a GraphIndexArtifact> for TemporalHistorySource<'a> {
    fn from(value: &'a GraphIndexArtifact) -> Self {
        Self::Artifact(value)
    }
}

impl<'a> From<&'a TemporalIndex> for TemporalHistorySource<'a> {
    fn from(value: &'a TemporalIndex) -> Self {
        Self::Index(value)
    }
}

pub fn symbol_history<'a, S>(
    source: S,
    commits: &CommitIndexArtifact,
    symbol: &str,
) -> Vec<(GitSha, ChangeKind, SnapshotKey)>
where
    S: Into<TemporalHistorySource<'a>>,
{
    match source.into() {
        TemporalHistorySource::Artifact(code) => {
            let index = TemporalIndex::new(Arc::new(code.clone()));
            symbol_history_indexed(&index, commits, symbol)
        }
        TemporalHistorySource::Index(index) => symbol_history_indexed(index, commits, symbol),
    }
}

pub(crate) fn symbol_history_indexed(
    index: &TemporalIndex,
    commits: &CommitIndexArtifact,
    symbol: &str,
) -> Vec<(GitSha, ChangeKind, SnapshotKey)> {
    let mut chain_keys = seed_symbol_history_keys(index, symbol);
    if close_symbol_history_chain(index, &mut chain_keys).is_err() {
        return Vec::new();
    }

    let stable_symbol_ids: HashSet<_> = chain_keys
        .iter()
        .map(|key| key.stable_symbol_id.as_str())
        .collect();
    let mut events = Vec::new();
    for stable_symbol_id in stable_symbol_ids {
        for edge in index.edges_for_stable_symbol_id(stable_symbol_id) {
            match (&edge.source, &edge.target) {
                (EdgeEndpoint::Commit { sha }, EdgeEndpoint::Snapshot { key })
                    if chain_keys.contains(key) =>
                {
                    if let Some(change_kind) = &edge.change_kind {
                        events.push((sha.clone(), change_kind.clone(), key.clone()));
                    }
                }
                _ => {}
            }
        }
    }

    if index.has_commit_positions() {
        sort_history_events(&mut events, |commit| index.commit_position(commit));
    } else {
        let graph = CommitGraph::new(commits);
        sort_history_events(&mut events, |commit| graph.position(commit));
    }
    events.dedup();
    events
}

fn sort_history_events(
    events: &mut [(GitSha, ChangeKind, SnapshotKey)],
    position: impl Fn(&str) -> usize,
) {
    events.sort_by(|left, right| {
        position(&left.0)
            .cmp(&position(&right.0))
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.2.stable_symbol_id.cmp(&right.2.stable_symbol_id))
            .then_with(|| left.2.commit.cmp(&right.2.commit))
    });
}

fn insert_snapshot_key(
    snapshot_key_sets: &mut HashMap<String, HashSet<SnapshotKey>>,
    stable_symbol_id: &str,
    key: SnapshotKey,
    referenced_snapshot_count: &mut usize,
) {
    if snapshot_key_sets
        .entry(stable_symbol_id.to_owned())
        .or_default()
        .insert(key)
    {
        *referenced_snapshot_count += 1;
    }
}

fn index_endpoint<'a>(
    endpoint: &'a EdgeEndpoint,
    stable_symbol_ids: &mut Vec<&'a str>,
    snapshot_key_sets: &mut HashMap<String, HashSet<SnapshotKey>>,
    referenced_snapshot_count: &mut usize,
) {
    match endpoint {
        EdgeEndpoint::Symbol { stable_symbol_id } => {
            push_unique_stable_symbol_id(stable_symbol_ids, stable_symbol_id);
        }
        EdgeEndpoint::Snapshot { key } => {
            push_unique_stable_symbol_id(stable_symbol_ids, &key.stable_symbol_id);
            insert_snapshot_key(
                snapshot_key_sets,
                key.stable_symbol_id.as_str(),
                key.clone(),
                referenced_snapshot_count,
            );
        }
        EdgeEndpoint::File { .. } | EdgeEndpoint::Commit { .. } => {}
    }
}

fn push_unique_stable_symbol_id<'a>(
    stable_symbol_ids: &mut Vec<&'a str>,
    stable_symbol_id: &'a str,
) {
    if !stable_symbol_ids.contains(&stable_symbol_id) {
        stable_symbol_ids.push(stable_symbol_id);
    }
}

pub fn resolve_symbol_at(
    code: &GraphIndexArtifact,
    commits: &CommitIndexArtifact,
    symbol: &str,
    anchor: &str,
    target: &str,
) -> Resolution<StableSymbolId> {
    let index = TemporalIndex::new(Arc::new(code.clone()));
    resolve_symbol_at_indexed(&index, commits, symbol, anchor, target)
}

pub fn resolve_symbol_at_indexed(
    index: &TemporalIndex,
    commits: &CommitIndexArtifact,
    symbol: &str,
    anchor: &str,
    target: &str,
) -> Resolution<StableSymbolId> {
    let graph = if index.has_commit_positions() {
        index.commit_graph()
    } else {
        CommitGraph::new(commits)
    };

    let Some(target_ancestors) = index.ancestors_of_with_graph(&graph, target) else {
        return Resolution::Unknown {
            reason: ResolutionFailure::IndexCorrupt(format!(
                "target commit `{target}` is not indexed"
            )),
        };
    };
    if !target_ancestors.contains(anchor) {
        if !graph.contains(anchor) {
            return Resolution::Unknown {
                reason: ResolutionFailure::AnchorCommitNotIndexed(anchor.to_owned()),
            };
        }
        return Resolution::Unknown {
            reason: ResolutionFailure::IndexCorrupt(format!(
                "anchor commit `{anchor}` is not reachable from target `{target}`"
            )),
        };
    }

    let mut anchor_candidates = match latest_anchor_snapshots_indexed(index, &graph, symbol, anchor)
    {
        Ok(candidates) => candidates,
        Err(reason) => return Resolution::Unknown { reason },
    };
    if anchor_candidates.is_empty() {
        return Resolution::Unknown {
            reason: ResolutionFailure::SymbolNotPresentAtAnchor,
        };
    }
    if anchor_candidates.len() > 1 {
        return Resolution::Ambiguous {
            candidates: stable_symbol_ids(anchor_candidates),
        };
    }

    let anchor_key = anchor_candidates.remove(0);
    let mut reachable_chain = [anchor_key.clone()].into_iter().collect();
    if let Err(reason) = close_rename_chain_indexed(index, &mut reachable_chain) {
        return Resolution::Unknown { reason };
    }

    resolve_from_anchor_key_indexed(index, &graph, &target_ancestors, anchor_key)
}

fn seed_symbol_history_keys(index: &TemporalIndex, symbol: &str) -> HashSet<SnapshotKey> {
    snapshot_keys_for_symbol_indexed(index, symbol)
        .into_iter()
        .collect()
}

fn close_symbol_history_chain(
    index: &TemporalIndex,
    chain_keys: &mut HashSet<SnapshotKey>,
) -> Result<(), ResolutionFailure> {
    loop {
        let previous_len = chain_keys.len();
        expand_stable_symbol_snapshots(index, chain_keys);
        close_rename_chain_indexed(index, chain_keys)?;
        if chain_keys.len() == previous_len {
            break;
        }
    }
    if !index.rename_edges.is_empty() {
        detect_reachable_rename_cycle(&index.rename_edges, chain_keys)?;
    }
    Ok(())
}

fn expand_stable_symbol_snapshots(index: &TemporalIndex, chain_keys: &mut HashSet<SnapshotKey>) {
    let stable_symbol_ids: HashSet<_> = chain_keys
        .iter()
        .map(|key| key.stable_symbol_id.clone())
        .collect();

    for stable_symbol_id in stable_symbol_ids {
        chain_keys.extend(snapshot_keys_for_symbol_indexed(index, &stable_symbol_id));
    }
}

fn close_rename_chain_indexed(
    index: &TemporalIndex,
    chain_keys: &mut HashSet<SnapshotKey>,
) -> Result<(), ResolutionFailure> {
    if index.rename_edges.is_empty() {
        return Ok(());
    }

    let snapshot_count = index.referenced_snapshot_count;
    let mut component = HashSet::new();
    let mut stack: Vec<_> = chain_keys.iter().cloned().collect();

    while let Some(current) = stack.pop() {
        if !component.insert(current.clone()) {
            continue;
        }
        chain_keys.insert(current.clone());
        guard_rename_chain_bound(chain_keys.len(), snapshot_count, &current)?;

        if let Some(neighbors) = index.rename_neighbors_by_snapshot.get(&current) {
            for neighbor in neighbors {
                if !component.contains(neighbor) {
                    stack.push(neighbor.clone());
                }
            }
        }
    }

    debug_assert!(chain_keys.len() <= snapshot_count);
    Ok(())
}

fn guard_rename_chain_bound(
    chain_len: usize,
    snapshot_count: usize,
    key: &SnapshotKey,
) -> Result<(), ResolutionFailure> {
    if chain_len <= snapshot_count {
        return Ok(());
    }

    Err(ResolutionFailure::IndexCorrupt(format!(
        "rename chain exceeds snapshot count at `{}`@`{}`",
        key.stable_symbol_id, key.commit
    )))
}

fn detect_reachable_rename_cycle(
    rename_edges: &[(SnapshotKey, SnapshotKey)],
    component: &HashSet<SnapshotKey>,
) -> Result<(), ResolutionFailure> {
    let mut forward: HashMap<SnapshotKey, Vec<SnapshotKey>> = HashMap::new();
    for (prev, next) in rename_edges {
        if component.contains(prev) && component.contains(next) {
            forward.entry(prev.clone()).or_default().push(next.clone());
        }
    }

    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();
    for key in component {
        visit_rename_chain(key, &forward, &mut visited, &mut visiting)?;
    }
    Ok(())
}

fn visit_rename_chain(
    key: &SnapshotKey,
    forward: &HashMap<SnapshotKey, Vec<SnapshotKey>>,
    visited: &mut HashSet<SnapshotKey>,
    visiting: &mut HashSet<SnapshotKey>,
) -> Result<(), ResolutionFailure> {
    if visiting.contains(key) {
        return Err(ResolutionFailure::IndexCorrupt(format!(
            "cycle in rename chain at `{}`@`{}`",
            key.stable_symbol_id, key.commit
        )));
    }
    if !visited.insert(key.clone()) {
        return Ok(());
    }

    visiting.insert(key.clone());
    if let Some(next_keys) = forward.get(key) {
        for next in next_keys {
            visit_rename_chain(next, forward, visited, visiting)?;
        }
    }
    visiting.remove(key);

    Ok(())
}

fn resolve_from_anchor_key_indexed(
    index: &TemporalIndex,
    graph: &CommitGraph<'_>,
    target_ancestors: &HashSet<String>,
    anchor_key: SnapshotKey,
) -> Resolution<StableSymbolId> {
    let mut current = anchor_key;
    let anchor = current.clone();
    let mut chain = Vec::new();
    let mut visited = HashSet::new();

    loop {
        if !visited.insert(current.clone()) {
            let path = std::iter::once(&anchor)
                .chain(chain.iter())
                .map(|key| format!("`{}`@`{}`", key.stable_symbol_id, key.commit))
                .collect::<Vec<_>>()
                .join(" -> ");
            return Resolution::Unknown {
                reason: ResolutionFailure::IndexCorrupt(format!(
                    "cycle in rename chain at `{}`@`{}` (cycle: {})",
                    current.stable_symbol_id, current.commit, path
                )),
            };
        }

        let mut candidates = match forward_rename_candidates_indexed(
            index,
            graph,
            target_ancestors,
            &current.stable_symbol_id,
            &current.commit,
        ) {
            Ok(candidates) => candidates,
            Err(reason) => return Resolution::Unknown { reason },
        };

        if candidates.is_empty() {
            match latest_reachable_snapshots_indexed(
                index,
                graph,
                target_ancestors,
                &current.stable_symbol_id,
                &current.commit,
            ) {
                Ok(latest) => {
                    if latest
                        .iter()
                        .any(|snapshot| index.is_deleted_snapshot(snapshot))
                    {
                        if let [last_seen] = latest.as_slice() {
                            return Resolution::Deleted {
                                last_seen: last_seen.clone(),
                            };
                        }
                        return Resolution::Ambiguous {
                            candidates: vec![current.stable_symbol_id],
                        };
                    }
                    return Resolution::Found {
                        value: current.stable_symbol_id,
                        chain,
                    };
                }
                Err(reason) => return Resolution::Unknown { reason },
            }
        }

        sort_snapshot_keys(&mut candidates, graph);
        candidates.dedup();
        if candidates.len() > 1 {
            return Resolution::Ambiguous {
                candidates: stable_symbol_ids(candidates),
            };
        }

        current = candidates.remove(0);
        chain.push(current.clone());
    }
}

fn latest_anchor_snapshots_indexed(
    index: &TemporalIndex,
    graph: &CommitGraph<'_>,
    stable_symbol_id: &str,
    anchor: &str,
) -> Result<Vec<SnapshotKey>, ResolutionFailure> {
    let Some(anchor_ancestors) = index.ancestors_of_with_graph(graph, anchor) else {
        return Err(ResolutionFailure::AnchorCommitNotIndexed(anchor.to_owned()));
    };

    let mut candidates = snapshot_keys_for_symbol_indexed(index, stable_symbol_id);
    candidates.retain(|key| anchor_ancestors.contains(&key.commit));
    sort_snapshot_keys(&mut candidates, graph);
    candidates.dedup();

    latest_from_candidates(index, candidates, graph)
}

fn latest_reachable_snapshots_indexed(
    index: &TemporalIndex,
    graph: &CommitGraph<'_>,
    target_ancestors: &HashSet<String>,
    stable_symbol_id: &str,
    after_commit: &str,
) -> Result<Vec<SnapshotKey>, ResolutionFailure> {
    let mut candidates = snapshot_keys_for_symbol_indexed(index, stable_symbol_id);
    candidates.retain(|key| {
        target_ancestors.contains(&key.commit)
            && index.is_ancestor_with_graph(graph, after_commit, &key.commit)
    });
    sort_snapshot_keys(&mut candidates, graph);
    candidates.dedup();

    latest_from_candidates(index, candidates, graph)
}

fn latest_from_candidates(
    index: &TemporalIndex,
    candidates: Vec<SnapshotKey>,
    graph: &CommitGraph<'_>,
) -> Result<Vec<SnapshotKey>, ResolutionFailure> {
    let mut latest = Vec::new();
    'candidate: for candidate in &candidates {
        if !graph.contains(&candidate.commit) {
            return Err(ResolutionFailure::IndexCorrupt(format!(
                "snapshot `{}` references unindexed commit `{}`",
                candidate.stable_symbol_id, candidate.commit
            )));
        }

        for other in &candidates {
            if candidate == other {
                continue;
            }
            if index.is_ancestor_with_graph(graph, &candidate.commit, &other.commit) {
                continue 'candidate;
            }
        }
        latest.push(candidate.clone());
    }

    Ok(latest)
}

fn forward_rename_candidates_indexed(
    index: &TemporalIndex,
    graph: &CommitGraph<'_>,
    target_ancestors: &HashSet<String>,
    stable_symbol_id: &str,
    after_commit: &str,
) -> Result<Vec<SnapshotKey>, ResolutionFailure> {
    forward_rename_candidates_from_edges(
        index,
        index.edges_for_stable_symbol_id(stable_symbol_id),
        graph,
        target_ancestors,
        stable_symbol_id,
        after_commit,
    )
}

fn forward_rename_candidates_from_edges<'a>(
    index: &TemporalIndex,
    edges: impl IntoIterator<Item = &'a TemporalEdgeArtifact>,
    graph: &CommitGraph<'_>,
    target_ancestors: &HashSet<String>,
    stable_symbol_id: &str,
    after_commit: &str,
) -> Result<Vec<SnapshotKey>, ResolutionFailure> {
    let mut candidates = Vec::new();
    for edge in edges {
        if edge.relation != RelationKind::Touches {
            continue;
        }

        let Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(prev))) = &edge.change_kind else {
            continue;
        };
        if prev.stable_symbol_id != stable_symbol_id {
            continue;
        }
        if !target_ancestors.contains(&prev.commit) {
            continue;
        }
        if !graph.contains(&prev.commit) {
            return Err(ResolutionFailure::IndexCorrupt(format!(
                "rename predecessor `{}` references unindexed commit `{}`",
                prev.stable_symbol_id, prev.commit
            )));
        }
        if !index.is_ancestor_with_graph(graph, after_commit, &prev.commit) {
            continue;
        }

        let Some(next) = rename_target(edge, prev) else {
            continue;
        };
        if !target_ancestors.contains(&next.commit) {
            continue;
        }
        if !graph.contains(&next.commit) {
            return Err(ResolutionFailure::IndexCorrupt(format!(
                "rename target `{}` references unindexed commit `{}`",
                next.stable_symbol_id, next.commit
            )));
        }
        if !index.is_ancestor_with_graph(graph, &prev.commit, &next.commit) {
            return Err(ResolutionFailure::IndexCorrupt(format!(
                "rename target `{}`@`{}` predates predecessor `{}`@`{}`",
                next.stable_symbol_id, next.commit, prev.stable_symbol_id, prev.commit
            )));
        }

        candidates.push(next);
    }
    Ok(candidates)
}

fn rename_target(
    edge: &crate::schema::TemporalEdgeArtifact,
    prev: &SnapshotKey,
) -> Option<SnapshotKey> {
    match (&edge.source, &edge.target) {
        // T7 made SymbolSnapshot -> SymbolSnapshot RenamedFrom edges the
        // authoritative traversal surface. Commit -> Snapshot change_kind
        // remains event metadata and is intentionally ignored here.
        (EdgeEndpoint::Snapshot { key: from }, EdgeEndpoint::Snapshot { key: to })
            if from == prev && to != prev =>
        {
            Some(to.clone())
        }
        _ => None,
    }
}

fn snapshot_keys_for_symbol_indexed(
    index: &TemporalIndex,
    stable_symbol_id: &str,
) -> Vec<SnapshotKey> {
    index
        .snapshot_keys_by_stable_symbol_id
        .get(stable_symbol_id)
        .cloned()
        .unwrap_or_default()
}

fn stable_symbol_ids(mut keys: Vec<SnapshotKey>) -> Vec<StableSymbolId> {
    keys.sort_by(|left, right| left.stable_symbol_id.cmp(&right.stable_symbol_id));
    let mut ids: Vec<_> = keys.into_iter().map(|key| key.stable_symbol_id).collect();
    ids.dedup();
    ids
}

fn sort_snapshot_keys(keys: &mut [SnapshotKey], graph: &CommitGraph<'_>) {
    keys.sort_by(|left, right| {
        graph
            .position(&left.commit)
            .cmp(&graph.position(&right.commit))
            .then_with(|| left.commit.cmp(&right.commit))
            .then_with(|| left.stable_symbol_id.cmp(&right.stable_symbol_id))
    });
}

pub struct CommitGraph<'a> {
    parents: CommitGraphParents<'a>,
    positions: CommitGraphPositions<'a>,
}

enum CommitGraphParents<'a> {
    Built(HashMap<&'a str, &'a [String]>),
    Borrowed(&'a HashMap<String, Vec<String>>),
}

enum CommitGraphPositions<'a> {
    Built(HashMap<&'a str, usize>),
    Borrowed(&'a HashMap<String, usize>),
}

impl<'a> CommitGraph<'a> {
    fn new(commits: &'a CommitIndexArtifact) -> Self {
        Self {
            parents: CommitGraphParents::Built(
                commits
                    .commits
                    .iter()
                    .map(|commit| (commit.sha.as_str(), commit.parents.as_slice()))
                    .collect(),
            ),
            positions: CommitGraphPositions::Built(
                commits
                    .commits
                    .iter()
                    .enumerate()
                    .map(|(index, commit)| (commit.sha.as_str(), index))
                    .collect(),
            ),
        }
    }

    fn from_borrowed(
        commit_positions: &'a HashMap<String, usize>,
        commit_parents: &'a HashMap<String, Vec<String>>,
    ) -> Self {
        Self {
            parents: CommitGraphParents::Borrowed(commit_parents),
            positions: CommitGraphPositions::Borrowed(commit_positions),
        }
    }

    fn contains(&self, commit: &str) -> bool {
        match &self.positions {
            CommitGraphPositions::Built(positions) => positions.contains_key(commit),
            CommitGraphPositions::Borrowed(positions) => positions.contains_key(commit),
        }
    }

    fn position(&self, commit: &str) -> usize {
        match &self.positions {
            CommitGraphPositions::Built(positions) => {
                positions.get(commit).copied().unwrap_or(usize::MAX)
            }
            CommitGraphPositions::Borrowed(positions) => {
                positions.get(commit).copied().unwrap_or(usize::MAX)
            }
        }
    }

    fn parent_shas(&self, commit: &str) -> Option<&[String]> {
        match &self.parents {
            CommitGraphParents::Built(parents) => parents.get(commit).copied(),
            CommitGraphParents::Borrowed(parents) => parents.get(commit).map(Vec::as_slice),
        }
    }

    fn ancestors_of(&self, commit: &str) -> Option<HashSet<String>> {
        if !self.contains(commit) {
            return None;
        }

        let mut seen = HashSet::new();
        let mut stack = vec![commit.to_owned()];
        while let Some(current) = stack.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            if let Some(parents) = self.parent_shas(current.as_str()) {
                stack.extend(parents.iter().cloned());
            }
        }
        Some(seen)
    }

    #[cfg(test)]
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        if ancestor == descendant {
            return self.contains(ancestor);
        }
        if !self.contains(ancestor) || !self.contains(descendant) {
            return false;
        }

        // Commits are indexed oldest-first via `git rev-list --topo-order --reverse`,
        // so an ancestor with a higher position than its descendant cannot precede it.
        // Both calls are safe to compare directly because the `contains` guards above
        // ensure neither lookup falls through to the `usize::MAX` sentinel.
        if self.position(ancestor) > self.position(descendant) {
            return false;
        }

        let mut seen = HashSet::new();
        let mut stack = vec![descendant];
        while let Some(current) = stack.pop() {
            if current == ancestor {
                return true;
            }
            if !seen.insert(current) {
                continue;
            }
            if let Some(parents) = self.parent_shas(current) {
                stack.extend(parents.iter().map(String::as_str));
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::*;
    use std::sync::Arc;

    #[test]
    fn rename_target_uses_t7_snapshot_edge_not_commit_edge() {
        let prev = SnapshotKey {
            stable_symbol_id: "old".into(),
            commit: "c1".into(),
        };
        let next = SnapshotKey {
            stable_symbol_id: "new".into(),
            commit: "c2".into(),
        };
        let commit_edge = TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: "c2".into() },
            target: EdgeEndpoint::Snapshot { key: next.clone() },
            relation: RelationKind::Touches,
            parent: Some("c1".into()),
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(prev.clone()))),
        };
        let snapshot_edge = TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot { key: prev.clone() },
            target: EdgeEndpoint::Snapshot { key: next.clone() },
            relation: RelationKind::Touches,
            parent: Some("c1".into()),
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(prev.clone()))),
        };

        assert_eq!(rename_target(&commit_edge, &prev), None);
        assert_eq!(rename_target(&snapshot_edge, &prev), Some(next));
    }

    fn fixture() -> (GraphIndexArtifact, CommitIndexArtifact) {
        let mut graph = GraphIndexArtifact {
            macro_invocations: None,
            header: GraphIndexHeader {
                graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.into(),
                content_hash_blake3: None,
            },
            manifest_version: String::new(),
            graph_content_hash: String::new(),
            file_manifests: vec![],
            files: vec![],
            file_node_ids: vec![],
            symbols: vec![],
            symbol_node_ids: vec![],
            edges: vec![],
            tombstones: vec![],
            diagnostics: vec![],
            commits: vec![],
            symbol_snapshots: vec![],
            temporal_edges: vec![],
        };

        let c1 = CommitArtifact {
            sha: "c1".into(),
            parents: vec![],
            author_time: 0,
            author_name: String::new(),
            author_email: String::new(),
            summary: "init".into(),
        };
        let c2 = CommitArtifact {
            sha: "c2".into(),
            parents: vec!["c1".into()],
            author_time: 1,
            author_name: String::new(),
            author_email: String::new(),
            summary: "rename".into(),
        };
        let c3 = CommitArtifact {
            sha: "c3".into(),
            parents: vec!["c2".into()],
            author_time: 2,
            author_name: String::new(),
            author_email: String::new(),
            summary: "later".into(),
        };

        let snap_old = SymbolSnapshotArtifact {
            key: SnapshotKey {
                stable_symbol_id: "old".into(),
                commit: "c1".into(),
            },
            file_path: "lib.rs".into(),
            entity_name: "old".into(),
            symbol_kind: "function".into(),
            enclosing_scope: None,
            byte_range: [0, 10],
            line_range: [1, 1],
            anchor_hash: "h1".into(),
            tokens: vec![],
        };
        let snap_new = SymbolSnapshotArtifact {
            key: SnapshotKey {
                stable_symbol_id: "new".into(),
                commit: "c2".into(),
            },
            file_path: "lib.rs".into(),
            entity_name: "new".into(),
            symbol_kind: "function".into(),
            enclosing_scope: None,
            byte_range: [0, 10],
            line_range: [1, 1],
            anchor_hash: "h1".into(),
            tokens: vec![],
        };

        graph.symbol_snapshots.push(snap_old.clone());
        graph.symbol_snapshots.push(snap_new.clone());
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: "c1".into() },
            target: EdgeEndpoint::Snapshot {
                key: snap_old.key.clone(),
            },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::Added),
        });
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: "c2".into() },
            target: EdgeEndpoint::Snapshot {
                key: snap_new.key.clone(),
            },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(
                snap_old.key.clone(),
            ))),
        });
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot {
                key: snap_old.key.clone(),
            },
            target: EdgeEndpoint::Snapshot {
                key: snap_new.key.clone(),
            },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(
                snap_old.key.clone(),
            ))),
        });

        let commits = CommitIndexArtifact {
            schema_version: 1,
            commits: vec![c1, c2, c3],
            refs: [("main".into(), "c3".into())].into(),
            indexed_at: "2026-05-20T12:00:00Z".into(),
            walk_strategy: WalkStrategy::Reachable,
        };

        (graph, commits)
    }

    #[test]
    fn is_ancestor_topo_fast_reject_returns_false_without_dfs() {
        let (_, mut commits) = fixture();
        commits.commits.swap(0, 1);
        let graph = CommitGraph::new(&commits);

        assert_eq!(graph.position("c1"), 1);
        assert_eq!(graph.position("c2"), 0);
        assert!(!graph.is_ancestor("c1", "c2"));
    }

    #[test]
    fn is_ancestor_matches_dfs_for_fixture_pairs() {
        let (_, commits) = fixture();
        let graph = CommitGraph::new(&commits);

        for ancestor in &commits.commits {
            for descendant in &commits.commits {
                let dfs_result = graph
                    .ancestors_of(&descendant.sha)
                    .is_some_and(|ancestors| ancestors.contains(&ancestor.sha));
                assert_eq!(
                    graph.is_ancestor(&ancestor.sha, &descendant.sha),
                    dfs_result,
                    "ancestor={}, descendant={}",
                    ancestor.sha,
                    descendant.sha
                );
            }
        }
    }

    #[test]
    fn temporal_index_cached_is_ancestor_matches_dfs_with_merge_and_fast_reject() {
        let (mut graph, _) = fixture();
        graph.commits = vec![
            CommitArtifact {
                sha: "root".into(),
                parents: vec![],
                author_time: 0,
                author_name: String::new(),
                author_email: String::new(),
                summary: "root".into(),
            },
            CommitArtifact {
                sha: "left".into(),
                parents: vec!["root".into()],
                author_time: 1,
                author_name: String::new(),
                author_email: String::new(),
                summary: "left".into(),
            },
            CommitArtifact {
                sha: "right".into(),
                parents: vec!["root".into()],
                author_time: 2,
                author_name: String::new(),
                author_email: String::new(),
                summary: "right".into(),
            },
            CommitArtifact {
                sha: "merge".into(),
                parents: vec!["left".into(), "right".into()],
                author_time: 3,
                author_name: String::new(),
                author_email: String::new(),
                summary: "merge".into(),
            },
        ];
        let index = TemporalIndex::new(Arc::new(graph));
        let graph = index.commit_graph();

        for ancestor in ["root", "left", "right", "merge"] {
            for descendant in ["root", "left", "right", "merge"] {
                let dfs_result = graph
                    .ancestors_of(descendant)
                    .is_some_and(|ancestors| ancestors.contains(ancestor));
                assert_eq!(
                    index.is_ancestor(ancestor, descendant),
                    dfs_result,
                    "ancestor={ancestor}, descendant={descendant}",
                );
            }
        }

        assert!(!index.is_ancestor("merge", "left"));
    }

    #[test]
    fn cycle_error_includes_chain_path() {
        // Construct the cycle within a single commit so the `is_ancestor`
        // topo-position fast-reject (which assumes commits are stored in
        // `git rev-list --topo-order --reverse` order) stays satisfied and
        // the visited-set cycle detector in `resolve_from_anchor_key_indexed`
        // is what actually fires.
        let (mut graph, commits) = fixture();
        let foo = SnapshotKey {
            stable_symbol_id: "foo".into(),
            commit: "c1".into(),
        };
        let bar = SnapshotKey {
            stable_symbol_id: "bar".into(),
            commit: "c1".into(),
        };
        let snap = |key: &SnapshotKey, anchor: &str| SymbolSnapshotArtifact {
            key: key.clone(),
            file_path: "lib.rs".into(),
            entity_name: key.stable_symbol_id.clone(),
            symbol_kind: "function".into(),
            enclosing_scope: None,
            byte_range: [0, 10],
            line_range: [1, 1],
            anchor_hash: anchor.into(),
            tokens: vec![],
        };
        graph.symbol_snapshots.push(snap(&foo, "hf"));
        graph.symbol_snapshots.push(snap(&bar, "hb"));
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot { key: foo.clone() },
            target: EdgeEndpoint::Snapshot { key: bar.clone() },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(foo.clone()))),
        });
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Snapshot { key: bar.clone() },
            target: EdgeEndpoint::Snapshot { key: foo.clone() },
            relation: RelationKind::Touches,
            parent: None,
            change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(bar))),
        });

        let commit_graph = CommitGraph::new(&commits);
        let target_ancestors = commit_graph.ancestors_of("c3").unwrap();
        let index = TemporalIndex::new(Arc::new(graph));

        let resolution =
            resolve_from_anchor_key_indexed(&index, &commit_graph, &target_ancestors, foo);

        match resolution {
            Resolution::Unknown {
                reason: ResolutionFailure::IndexCorrupt(message),
            } => {
                assert!(
                    message.contains("cycle: `foo`@`c1` -> `bar`@`c1` -> `foo`@`c1`"),
                    "expected path in {message}"
                );
            }
            other => panic!("expected cycle error, got {other:?}"),
        }
    }

    #[test]
    fn resolves_renamed_symbol_to_target() {
        let (graph, commits) = fixture();
        let resolution = resolve_symbol_at(&graph, &commits, "old", "c1", "c3");

        match resolution {
            Resolution::Found { value, chain } => {
                assert_eq!(value, "new");
                assert_eq!(chain.len(), 1);
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn resolve_symbol_at_indexed_matches_raw() {
        let (graph, commits) = fixture();
        let index = TemporalIndex::new(Arc::new(graph.clone()));
        let symbols = ["old", "new", "missing"];
        let commits_to_check = ["c1", "c2", "c3", "nonexistent"];

        for symbol in symbols {
            for anchor in commits_to_check {
                for target in commits_to_check {
                    assert_eq!(
                        resolve_symbol_at(&graph, &commits, symbol, anchor, target),
                        resolve_symbol_at_indexed(&index, &commits, symbol, anchor, target),
                        "symbol={symbol} anchor={anchor} target={target}"
                    );
                }
            }
        }
    }

    #[test]
    fn latest_anchor_snapshots_indexed_uses_snapshot_key_index() {
        let (mut graph, commits) = fixture();
        let key = SnapshotKey {
            stable_symbol_id: "edge-only".into(),
            commit: "c3".into(),
        };
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: "c3".into() },
            target: EdgeEndpoint::Snapshot { key: key.clone() },
            relation: RelationKind::Touches,
            parent: Some("c2".into()),
            change_kind: Some(ChangeKind::Added),
        });
        let index = TemporalIndex::new(Arc::new(graph));
        let commit_graph = CommitGraph::new(&commits);

        assert_eq!(
            latest_anchor_snapshots_indexed(&index, &commit_graph, "edge-only", "c3")
                .expect("anchor is indexed"),
            vec![key]
        );
    }

    #[test]
    fn temporal_index_tracks_deleted_snapshot_keys_by_identity() {
        let (mut graph, _) = fixture();
        let deleted = SnapshotKey {
            stable_symbol_id: "old".into(),
            commit: "c3".into(),
        };
        let same_symbol_other_commit = SnapshotKey {
            stable_symbol_id: "old".into(),
            commit: "c2".into(),
        };
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: "c3".into() },
            target: EdgeEndpoint::Snapshot {
                key: deleted.clone(),
            },
            relation: RelationKind::Touches,
            parent: Some("c2".into()),
            change_kind: Some(ChangeKind::Deleted),
        });

        let index = TemporalIndex::new(Arc::new(graph));

        assert!(index.is_deleted_snapshot(&deleted));
        assert!(!index.is_deleted_snapshot(&same_symbol_other_commit));
    }

    #[test]
    fn resolves_unknown_anchor() {
        let (graph, commits) = fixture();
        let resolution = resolve_symbol_at(&graph, &commits, "old", "nonexistent", "c3");

        assert!(matches!(resolution, Resolution::Unknown { .. }));
    }

    #[test]
    fn symbol_history_returns_chronological_chain() {
        let (g, c) = fixture();
        let hist = symbol_history(&g, &c, "old");

        // old (c1, Added) -> new (c2, RenamedFrom(old))
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].0, "c1");
        assert!(matches!(hist[0].1, ChangeKind::Added));
        assert_eq!(hist[0].2.stable_symbol_id, "old");
        assert_eq!(hist[1].0, "c2");
        assert!(matches!(hist[1].1, ChangeKind::RenamedFrom(_)));
        assert_eq!(hist[1].2.stable_symbol_id, "new");
    }

    #[test]
    fn symbol_history_walks_backward_across_rename_predecessors() {
        let (g, c) = fixture();
        let hist = symbol_history(&g, &c, "new");

        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].2.stable_symbol_id, "old");
        assert_eq!(hist[1].2.stable_symbol_id, "new");
    }

    #[test]
    fn index_size_bytes_is_nonzero_for_populated_index() {
        let (graph, _) = fixture();
        let index = TemporalIndex::new(Arc::new(graph));

        assert!(index.index_size_bytes() > 0);
    }

    #[test]
    fn temporal_index_is_arc_owned_and_preserves_symbol_history_api() {
        fn assert_send_sync<T: Send + Sync>() {}

        let (graph, commits) = fixture();
        let artifact = Arc::new(graph);
        let expected = symbol_history(artifact.as_ref(), &commits, "old");
        let index = TemporalIndex::new(Arc::clone(&artifact));

        drop(artifact);

        assert_send_sync::<TemporalIndex>();
        assert_eq!(symbol_history(&index, &commits, "old"), expected);
    }
}
