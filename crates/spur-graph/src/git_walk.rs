//! Git history walker for code graph artifacts.
//!
//! `try_rename_match` is deliberately crate-private; rename-corpus coverage
//! lives in this crate's unit tests so the heuristic is not exported as public
//! API.
//!
//! ```compile_fail
//! use spur_graph::git_walk::try_rename_match;
//!
//! let _ = try_rename_match;
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _, Result};
use gix::object::tree::diff::{Action, Change};
use gix::objs::tree::EntryMode;
use indicatif::ProgressBar;

use crate::content_hash::compute_graph_content_hash;
use crate::extract::languages::Language;
use crate::extract::tree_sitter::{BytesExtractor, ExtractError, ExtractedSymbol};
use crate::schema::{
    ChangeKind, CommitArtifact, CommitIndexArtifact, EdgeEndpoint, GitPath, GraphIndexArtifact,
    GraphIndexHeader, RelationKind, RenamePrev, SnapshotKey, SymbolSnapshotArtifact,
    TemporalEdgeArtifact, WalkStrategy, GRAPH_INDEX_VERSION_TEMPORAL,
};
use crate::store::parquet::{
    load_temporal_artifact_metadata_parquet, stream_temporal_artifact_parquet_into_sink,
};
use crate::store::{
    commit_index, current_manifest_version, load_temporal_artifact_parquet,
    resolve_artifact_location, TemporalShardSink,
};

const TEMPORAL_PARSE_CACHE_BUDGET_ENV: &str = "SPUR_GRAPH_TEMPORAL_PARSE_CACHE_BYTES";
const DEFAULT_TEMPORAL_PARSE_CACHE_BUDGET_BYTES: u64 = 0;

fn parse_temporal_parse_cache_budget_bytes(value: Option<OsString>) -> Result<u64> {
    let Some(value) = value else {
        return Ok(DEFAULT_TEMPORAL_PARSE_CACHE_BUDGET_BYTES);
    };
    let value = value.into_string().map_err(|_| {
        anyhow!("{TEMPORAL_PARSE_CACHE_BUDGET_ENV} must be a valid Unicode decimal u64 byte count")
    })?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("{TEMPORAL_PARSE_CACHE_BUDGET_ENV} must be a decimal u64 byte count, got `{value}`");
    }
    value.parse::<u64>().with_context(|| {
        format!("{TEMPORAL_PARSE_CACHE_BUDGET_ENV} exceeds the decimal u64 byte range: `{value}`")
    })
}

fn configured_temporal_parse_cache_budget_bytes() -> Result<u64> {
    parse_temporal_parse_cache_budget_bytes(std::env::var_os(TEMPORAL_PARSE_CACHE_BUDGET_ENV))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitWalkConfig {
    pub target_refs: Vec<String>,
    pub walk_strategy: WalkStrategy,
    pub allow_replace_refs: bool,
    pub use_gix_diff: bool,
    pub temporal_jobs: NonZeroUsize,
}

impl Default for GitWalkConfig {
    fn default() -> Self {
        Self {
            target_refs: vec!["main".to_owned()],
            walk_strategy: WalkStrategy::Reachable,
            allow_replace_refs: false,
            use_gix_diff: true,
            temporal_jobs: NonZeroUsize::MIN,
        }
    }
}

pub fn snapshot_refs(worktree: &Path, refs: &[&str]) -> Result<BTreeMap<String, String>> {
    ensure_not_shallow(worktree)?;
    let mut snapshot = BTreeMap::new();

    for target_ref in refs {
        let ref_name;
        let rev = if *target_ref == "HEAD" {
            "HEAD^{commit}"
        } else {
            ref_name = format!("refs/heads/{target_ref}");
            &ref_name
        };
        let stdout = run_git(worktree, &["rev-parse", "--verify", rev]).with_context(|| {
            format!("target ref `{target_ref}` does not exist; refusing to fall back")
        })?;
        snapshot.insert((*target_ref).to_owned(), stdout.trim().to_owned());
    }

    Ok(snapshot)
}

pub fn ensure_not_shallow(worktree: &Path) -> Result<()> {
    let stdout = run_git(worktree, &["rev-parse", "--is-shallow-repository"]).with_context(
        || {
            format!(
                "spur-graph: could not determine whether `{}` is a shallow repository; refusing to walk",
                worktree.display()
            )
        },
    )?;

    if stdout.trim() == "true" {
        bail!(
            "spur-graph: refusing to index shallow clone at `{}`; symbol history would be silently truncated. Run `git fetch --unshallow` first.",
            worktree.display()
        );
    }

    Ok(())
}

pub fn check_replace_refs(worktree: &Path, allow: bool) -> Result<()> {
    if allow {
        return Ok(());
    }

    let replace_refs = run_git(
        worktree,
        &["for-each-ref", "--format=%(refname)", "refs/replace"],
    )
    .with_context(|| {
        format!(
            "spur-graph: could not inspect git replace refs at `{}`; refusing to walk",
            worktree.display()
        )
    })?;
    let grafts_path = git_dir(worktree)?.join("info/grafts");

    if !replace_refs.trim().is_empty() || grafts_path.exists() {
        bail!(
            "spur-graph: git replace refs or grafts detected at `{}`; refusing to walk. Set GitWalkConfig.allow_replace_refs = true to override.",
            worktree.display()
        );
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncrementalPlan {
    ColdWalk {
        from_root: bool,
    },
    FastForward {
        from: String,
        to: String,
    },
    ForcePushRecover {
        merge_base: Option<String>,
        to: String,
    },
}

#[derive(Debug, Clone)]
struct CommitWalkResult {
    ordinal: usize,
    commit: CommitArtifact,
    temporal_edges: Vec<TemporalEdgeArtifact>,
    symbol_snapshots: Vec<SymbolSnapshotArtifact>,
    diagnostics: Vec<String>,
}

struct CommitResultReducer<'a> {
    graph: &'a mut GraphIndexArtifact,
    commits: &'a mut CommitIndexArtifact,
    progress: Option<ProgressBar>,
    sink: Option<&'a mut TemporalShardSink>,
    next_ordinal: usize,
    pending: BTreeMap<usize, CommitWalkResult>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WorkerPoolStats {
    max_in_flight: usize,
    max_queued_work: usize,
    max_active_work: usize,
    max_result_occupancy: usize,
    max_reducer_pending: usize,
    pool_elapsed_nanos: u64,
    active_worker_nanos: u64,
    average_active_workers_milli: u64,
    completed_out_of_order: u64,
    admission_window_full_receive_wait_nanos: u64,
    next_ordinal_blocked_wait_nanos: u64,
    coordinator_send_blocked_nanos: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReductionProgress {
    next_ordinal: usize,
    pending_results: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TemporalWalkStats {
    pool: WorkerPoolStats,
    shared_parse_cache_current_entries: usize,
    shared_parse_cache_peak_entries: usize,
    shared_parse_cache: ParseCacheTelemetryStats,
}

fn duration_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn atomic_saturating_add(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

#[derive(Default)]
struct OccupancyCounter {
    current: AtomicUsize,
    maximum: AtomicUsize,
}

impl OccupancyCounter {
    fn increment(&self) {
        let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(current, Ordering::SeqCst);
    }

    fn decrement(&self, count: usize) {
        let previous = self.current.fetch_sub(count, Ordering::SeqCst);
        debug_assert!(previous >= count, "worker-pool occupancy underflow");
    }

    fn maximum(&self) -> usize {
        self.maximum.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct ConcurrentOccupancy {
    queued_work: OccupancyCounter,
    active_work: OccupancyCounter,
    /// Completed results in worker hands, the result channel, or reducer pending storage.
    results: OccupancyCounter,
}

#[derive(Default)]
struct WorkerTiming {
    active_worker_nanos: AtomicU64,
}

/// Bounds admitted ordinals independently of the channel capacities.
///
/// An ordinal keeps its slot until the reducer advances past it, so a stalled
/// low ordinal bounds queued work, active work, and every form of buffered result.
struct OrdinalAdmissionWindow {
    capacity: usize,
    next_dispatch: usize,
    next_reduced: usize,
    max_in_flight: usize,
}

impl OrdinalAdmissionWindow {
    fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity: capacity.get(),
            next_dispatch: 0,
            next_reduced: 0,
            max_in_flight: 0,
        }
    }

    fn in_flight(&self) -> usize {
        self.next_dispatch - self.next_reduced
    }

    fn has_capacity(&self) -> bool {
        self.in_flight() < self.capacity
    }

    fn admit(&mut self, ordinal: usize) -> Result<()> {
        if ordinal != self.next_dispatch {
            bail!(
                "worker-pool admission expected ordinal {} but received {ordinal}",
                self.next_dispatch
            );
        }
        if !self.has_capacity() {
            bail!(
                "worker-pool admission window exceeded {} in-flight ordinals",
                self.capacity
            );
        }
        self.next_dispatch = self
            .next_dispatch
            .checked_add(1)
            .context("worker-pool dispatch ordinal overflow")?;
        self.max_in_flight = self.max_in_flight.max(self.in_flight());
        Ok(())
    }

    fn note_reduced(&mut self, next_ordinal: usize) -> Result<usize> {
        if !(self.next_reduced..=self.next_dispatch).contains(&next_ordinal) {
            bail!(
                "worker-pool reducer advanced from {} to invalid ordinal {next_ordinal} with {} dispatched",
                self.next_reduced,
                self.next_dispatch
            );
        }
        let reduced = next_ordinal - self.next_reduced;
        self.next_reduced = next_ordinal;
        Ok(reduced)
    }
}

enum WorkerEvent<Output> {
    Ready {
        worker_id: usize,
    },
    Completed {
        ordinal: usize,
        output: Output,
    },
    Failed {
        worker_id: usize,
        ordinal: Option<usize>,
        error: anyhow::Error,
    },
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

fn join_worker_threads(handles: Vec<thread::ScopedJoinHandle<'_, ()>>) -> Result<()> {
    let mut panicked = Vec::new();
    for (worker_id, handle) in handles.into_iter().enumerate() {
        if let Err(payload) = handle.join() {
            panicked.push(format!("worker {worker_id}: {}", panic_message(payload)));
        }
    }
    if panicked.is_empty() {
        Ok(())
    } else {
        bail!("failed to join temporal workers: {}", panicked.join("; "))
    }
}

struct CancelOnFailure<'a> {
    cancellation: &'a AtomicBool,
    armed: bool,
}

impl<'a> CancelOnFailure<'a> {
    fn new(cancellation: &'a AtomicBool) -> Self {
        Self {
            cancellation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelOnFailure<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.store(true, Ordering::Release);
        }
    }
}

fn drain_worker_events<Output>(
    receiver: &Receiver<WorkerEvent<Output>>,
    occupancy: &ConcurrentOccupancy,
) {
    while let Ok(event) = receiver.recv() {
        if matches!(event, WorkerEvent::Completed { .. }) {
            occupancy.results.decrement(1);
        }
    }
}

fn run_bounded_worker_pool<State, Work, Output, Initialize, Compute, Reduce>(
    jobs: NonZeroUsize,
    work_items: Vec<Work>,
    initialize: Initialize,
    compute: Compute,
    reduce: Reduce,
) -> Result<WorkerPoolStats>
where
    Work: Send + 'static,
    Output: Send + 'static,
    Initialize: Fn(usize) -> Result<State> + Send + Sync + 'static,
    Compute: Fn(&mut State, usize, Work) -> Result<Output> + Send + Sync + 'static,
    Reduce: FnMut(Output) -> Result<ReductionProgress>,
{
    let total_work = work_items.len();
    if total_work == 0 {
        return Ok(WorkerPoolStats::default());
    }
    let pool_started = Instant::now();
    thread::scope(move |scope| {
        let worker_count = jobs.get().min(total_work);
        let initialize = Arc::new(initialize);
        let compute = Arc::new(compute);
        let cancellation = Arc::new(AtomicBool::new(false));
        let occupancy = Arc::new(ConcurrentOccupancy::default());
        let timing = Arc::new(WorkerTiming::default());
        let (event_sender, event_receiver) = sync_channel(worker_count);
        let mut work_senders: Vec<SyncSender<(usize, Work)>> = Vec::with_capacity(worker_count);
        let mut handles = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let (work_sender, work_receiver) = sync_channel(1);
            let worker_events = event_sender.clone();
            let worker_initialize = Arc::clone(&initialize);
            let worker_compute = Arc::clone(&compute);
            let worker_cancellation = Arc::clone(&cancellation);
            let worker_occupancy = Arc::clone(&occupancy);
            let worker_timing = Arc::clone(&timing);
            let spawn = thread::Builder::new()
                .name(format!("spur-temporal-{worker_id}"))
                .spawn_scoped(scope, move || {
                    let mut current_ordinal = None;
                    let worker_run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                        || -> Result<()> {
                            let mut state = worker_initialize(worker_id).with_context(|| {
                                format!("initialize temporal worker {worker_id}")
                            })?;
                            worker_events
                                .send(WorkerEvent::Ready { worker_id })
                                .map_err(|_| {
                                    anyhow!("temporal result channel closed during startup")
                                })?;

                            while let Ok((ordinal, work)) = work_receiver.recv() {
                                current_ordinal = Some(ordinal);
                                worker_occupancy.queued_work.decrement(1);
                                if worker_cancellation.load(Ordering::Acquire) {
                                    current_ordinal = None;
                                    drop(work);
                                    continue;
                                }

                                worker_occupancy.active_work.increment();
                                let compute_started = Instant::now();
                                let computed = std::panic::catch_unwind(
                                    std::panic::AssertUnwindSafe(|| {
                                        worker_compute(&mut state, ordinal, work)
                                    }),
                                );
                                atomic_saturating_add(
                                    &worker_timing.active_worker_nanos,
                                    duration_nanos(compute_started.elapsed()),
                                );
                                worker_occupancy.active_work.decrement(1);
                                let output = match computed {
                                    Ok(result) => result,
                                    Err(payload) => bail!(
                                        "temporal worker {worker_id} panicked at ordinal {ordinal}: {}",
                                        panic_message(payload)
                                    ),
                                }?;
                                worker_occupancy.results.increment();
                                if worker_events
                                    .send(WorkerEvent::Completed { ordinal, output })
                                    .is_err()
                                {
                                    worker_occupancy.results.decrement(1);
                                    return Ok(());
                                }
                                current_ordinal = None;
                            }
                            Ok(())
                        },
                    ));

                    let failure = match worker_run {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => Some(error),
                        Err(payload) => Some(anyhow!(
                            "temporal worker {worker_id} panicked: {}",
                            panic_message(payload)
                        )),
                    };
                    if let Some(error) = failure {
                        worker_cancellation.store(true, Ordering::Release);
                        let _ = worker_events.send(WorkerEvent::Failed {
                            worker_id,
                            ordinal: current_ordinal,
                            error,
                        });
                    }
                });

            match spawn {
                Ok(handle) => {
                    work_senders.push(work_sender);
                    handles.push(handle);
                }
                Err(error) => {
                    cancellation.store(true, Ordering::Release);
                    drop(work_sender);
                    drop(work_senders);
                    drop(event_sender);
                    drain_worker_events(&event_receiver, &occupancy);
                    let join_result = join_worker_threads(handles);
                    let error =
                        anyhow!(error).context(format!("spawn temporal worker {worker_id}"));
                    return match join_result {
                        Ok(()) => Err(error),
                        Err(join_error) => Err(error.context(join_error.to_string())),
                    };
                }
            }
        }
        drop(event_sender);

        let mut reduce = reduce;
        let coordinator = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<WorkerPoolStats> {
                let mut cancel_on_failure = CancelOnFailure::new(&cancellation);
                let mut ready = vec![false; worker_count];
                let mut ready_count = 0;
                while ready_count < worker_count {
                    match event_receiver.recv() {
                        Ok(WorkerEvent::Ready { worker_id }) => {
                            let was_ready = ready
                                .get_mut(worker_id)
                                .with_context(|| format!("unknown temporal worker {worker_id}"))?;
                            if std::mem::replace(was_ready, true) {
                                bail!("temporal worker {worker_id} reported ready twice");
                            }
                            ready_count += 1;
                        }
                        Ok(WorkerEvent::Failed {
                            worker_id,
                            ordinal,
                            error,
                        }) => {
                            let location = ordinal
                                .map(|ordinal| format!(" at ordinal {ordinal}"))
                                .unwrap_or_default();
                            return Err(error).with_context(|| {
                                format!("temporal worker {worker_id} failed{location}")
                            });
                        }
                        Ok(WorkerEvent::Completed { .. }) => {
                            bail!("temporal worker completed work before startup finished")
                        }
                        Err(_) => bail!("temporal worker result channel closed during startup"),
                    }
                }

                let mut work = work_items.into_iter().enumerate();
                let mut window = OrdinalAdmissionWindow::new(jobs);
                let mut max_reducer_pending = 0;
                let mut reducer_pending = 0;
                let mut completed_out_of_order = 0_u64;
                let mut admission_window_full_receive_wait_nanos = 0_u64;
                let mut next_ordinal_blocked_wait_nanos = 0_u64;
                let mut coordinator_send_blocked_nanos = 0_u64;

                while window.next_reduced < total_work {
                    while window.next_dispatch < total_work
                        && window.has_capacity()
                        && !cancellation.load(Ordering::Acquire)
                    {
                        let (ordinal, item) = work
                            .next()
                            .context("temporal worker input ended before its declared length")?;
                        let worker_id = ordinal % worker_count;
                        occupancy.queued_work.increment();
                        let send_started = Instant::now();
                        let send_result = work_senders[worker_id].send((ordinal, item));
                        coordinator_send_blocked_nanos = coordinator_send_blocked_nanos
                            .saturating_add(duration_nanos(send_started.elapsed()));
                        if send_result.is_err() {
                            occupancy.queued_work.decrement(1);
                            bail!(
                                "temporal worker {worker_id} stopped before accepting ordinal {ordinal}"
                            );
                        }
                        window.admit(ordinal)?;
                    }

                    let admission_window_was_full = !window.has_capacity();
                    let next_ordinal_was_blocked = reducer_pending > 0;
                    let receive_started = Instant::now();
                    let event = event_receiver.recv();
                    let receive_wait_nanos = duration_nanos(receive_started.elapsed());
                    if admission_window_was_full {
                        admission_window_full_receive_wait_nanos =
                            admission_window_full_receive_wait_nanos
                                .saturating_add(receive_wait_nanos);
                    }
                    if next_ordinal_was_blocked {
                        next_ordinal_blocked_wait_nanos =
                            next_ordinal_blocked_wait_nanos.saturating_add(receive_wait_nanos);
                    }

                    match event {
                        Ok(WorkerEvent::Completed { ordinal, output }) => {
                            if !(window.next_reduced..window.next_dispatch).contains(&ordinal) {
                                bail!(
                                    "temporal worker returned ordinal {ordinal} outside admitted window {}..{}",
                                    window.next_reduced,
                                    window.next_dispatch
                                );
                            }
                            if ordinal != window.next_reduced {
                                completed_out_of_order = completed_out_of_order.saturating_add(1);
                            }
                            let progress = reduce(output).with_context(|| {
                                format!("reduce temporal commit ordinal {ordinal}")
                            })?;
                            let reduced = window.note_reduced(progress.next_ordinal)?;
                            occupancy.results.decrement(reduced);
                            reducer_pending = progress.pending_results;
                            max_reducer_pending = max_reducer_pending.max(progress.pending_results);
                        }
                        Ok(WorkerEvent::Failed {
                            worker_id,
                            ordinal,
                            error,
                        }) => {
                            let location = ordinal
                                .map(|ordinal| format!(" at ordinal {ordinal}"))
                                .unwrap_or_default();
                            return Err(error).with_context(|| {
                                format!("temporal worker {worker_id} failed{location}")
                            });
                        }
                        Ok(WorkerEvent::Ready { worker_id }) => {
                            bail!("temporal worker {worker_id} reported ready twice")
                        }
                        Err(_) => bail!(
                            "temporal worker result channel closed while waiting for ordinal {}",
                            window.next_reduced
                        ),
                    }
                }

                let pool_elapsed_nanos = duration_nanos(pool_started.elapsed());
                let active_worker_nanos = timing.active_worker_nanos.load(Ordering::Relaxed);
                let average_active_workers_milli = if pool_elapsed_nanos == 0 {
                    0
                } else {
                    ((u128::from(active_worker_nanos) * 1_000) / u128::from(pool_elapsed_nanos))
                        .min(u128::from(u64::MAX)) as u64
                };
                let stats = WorkerPoolStats {
                    max_in_flight: window.max_in_flight,
                    max_queued_work: occupancy.queued_work.maximum(),
                    max_active_work: occupancy.active_work.maximum(),
                    max_result_occupancy: occupancy.results.maximum(),
                    max_reducer_pending,
                    pool_elapsed_nanos,
                    active_worker_nanos,
                    average_active_workers_milli,
                    completed_out_of_order,
                    admission_window_full_receive_wait_nanos,
                    next_ordinal_blocked_wait_nanos,
                    coordinator_send_blocked_nanos,
                };
                cancel_on_failure.disarm();
                Ok(stats)
            },
        ));

        if !matches!(&coordinator, Ok(Ok(_))) {
            cancellation.store(true, Ordering::Release);
        }
        drop(work_senders);
        if !matches!(&coordinator, Ok(Ok(_))) {
            drain_worker_events(&event_receiver, &occupancy);
        }
        let join_result = join_worker_threads(handles);

        match coordinator {
            Err(payload) => {
                drop(join_result);
                std::panic::resume_unwind(payload)
            }
            Ok(coordinator) => match (coordinator, join_result) {
                (Ok(stats), Ok(())) => Ok(stats),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(join_error)) => Err(join_error),
                (Err(error), Err(join_error)) => Err(error.context(join_error.to_string())),
            },
        }
    })
}

impl<'a> CommitResultReducer<'a> {
    fn new(
        graph: &'a mut GraphIndexArtifact,
        commits: &'a mut CommitIndexArtifact,
        progress: Option<ProgressBar>,
        sink: Option<&'a mut TemporalShardSink>,
    ) -> Self {
        Self {
            graph,
            commits,
            progress,
            sink,
            next_ordinal: 0,
            pending: BTreeMap::new(),
        }
    }

    fn push(&mut self, result: CommitWalkResult) -> Result<()> {
        if result.ordinal < self.next_ordinal || self.pending.contains_key(&result.ordinal) {
            bail!(
                "commit walk reducer received duplicate or stale ordinal {} while waiting for {}",
                result.ordinal,
                self.next_ordinal
            );
        }
        let ordinal = result.ordinal;
        self.pending.insert(ordinal, result);

        while let Some(result) = self.pending.remove(&self.next_ordinal) {
            self.apply(result)?;
            self.next_ordinal = self
                .next_ordinal
                .checked_add(1)
                .context("commit walk ordinal overflow")?;
        }
        Ok(())
    }

    fn apply(&mut self, mut result: CommitWalkResult) -> Result<()> {
        self.graph.commits.push(result.commit.clone());
        self.commits.commits.push(result.commit.clone());
        self.graph.temporal_edges.append(&mut result.temporal_edges);
        self.graph
            .symbol_snapshots
            .append(&mut result.symbol_snapshots);
        self.graph.diagnostics.append(&mut result.diagnostics);
        if let Some(progress) = self.progress.as_ref() {
            progress.inc(1);
        }
        if let Some(sink) = self.sink.as_deref_mut() {
            sink.append_commit(
                &result.commit,
                &mut self.graph.temporal_edges,
                &mut self.graph.symbol_snapshots,
            )?;
        }
        Ok(())
    }

    fn finish(self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        bail!(
            "commit walk reducer is missing ordinal {} before buffered ordinals {:?}",
            self.next_ordinal,
            self.pending.keys().collect::<Vec<_>>()
        )
    }
}

fn compute_commit_walk_result(
    ordinal: usize,
    worktree: &Path,
    sha: &str,
    gix_repo: Option<&gix::Repository>,
    ctx: &mut SymbolDiffCtx,
) -> Result<CommitWalkResult> {
    let diagnostics_start = ctx.diagnostics().len();
    let commit = match gix_repo {
        Some(repo) => read_commit_gix(repo, sha)?,
        None => read_commit(worktree, sha)?,
    };
    let file_changes = match gix_repo {
        Some(repo) => file_changes_for_commit_gix(repo, sha)?,
        None => file_changes_for_commit(worktree, sha)?,
    };
    let symbol_changes = symbol_changes_for_commit(worktree, sha, &file_changes, ctx)?;
    let mut temporal_edges = Vec::with_capacity(file_changes.len() + symbol_changes.len() * 2);
    let mut symbol_snapshots = Vec::with_capacity(symbol_changes.len());

    temporal_edges.extend(
        file_changes
            .iter()
            .map(|file_change| file_change_to_temporal_edge(sha, file_change)),
    );
    for symbol_change in symbol_changes {
        let snapshot_key = symbol_change.snapshot.key.clone();
        let parent_sha = symbol_change.parent_sha;
        let change_kind = symbol_change.change_kind;

        symbol_snapshots.push(symbol_change.snapshot);
        temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit {
                sha: sha.to_owned(),
            },
            target: EdgeEndpoint::Snapshot {
                key: snapshot_key.clone(),
            },
            relation: RelationKind::Touches,
            parent: parent_sha.clone(),
            change_kind: Some(change_kind.clone()),
        });

        if let ChangeKind::RenamedFrom(RenamePrev::Symbol(previous_key)) = change_kind {
            temporal_edges.push(TemporalEdgeArtifact {
                source: EdgeEndpoint::Snapshot {
                    key: previous_key.clone(),
                },
                target: EdgeEndpoint::Snapshot { key: snapshot_key },
                relation: RelationKind::Touches,
                parent: parent_sha,
                change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(previous_key))),
            });
        }
    }

    Ok(CommitWalkResult {
        ordinal,
        commit,
        temporal_edges,
        symbol_snapshots,
        diagnostics: ctx.diagnostics()[diagnostics_start..].to_vec(),
    })
}

struct CommitWorkerState {
    repo: gix::Repository,
    symbol_diff: SymbolDiffCtx,
}

pub fn plan_incremental_walk(
    worktree: &Path,
    stored_tip: Option<&str>,
    new_tip: &str,
) -> Result<IncrementalPlan> {
    let Some(stored) = stored_tip else {
        return Ok(IncrementalPlan::ColdWalk { from_root: true });
    };

    let ancestor = Command::new("git")
        .current_dir(worktree)
        .args(["merge-base", "--is-ancestor", stored, new_tip])
        .status()
        .with_context(|| {
            format!(
                "spawn git merge-base --is-ancestor in `{}`",
                worktree.display()
            )
        })?;

    if ancestor.success() {
        return Ok(IncrementalPlan::FastForward {
            from: stored.to_owned(),
            to: new_tip.to_owned(),
        });
    }

    tracing::warn!(
        stored_tip = stored,
        new_tip,
        status = ?ancestor.code(),
        "spur-graph: stored commit is not an ancestor of new tip; force-push recovery will re-walk the diverged range"
    );
    let merge_base = run_git(worktree, &["merge-base", stored, new_tip])
        .map(|stdout| stdout.trim().to_owned())
        .inspect_err(|error| {
            tracing::warn!(
                stored_tip = stored,
                new_tip,
                error = %error,
                "spur-graph: force-push recovery could not find a merge base; falling back to cold recovery for this ref"
            );
        })
        .ok()
        .filter(|sha| !sha.is_empty());

    Ok(IncrementalPlan::ForcePushRecover {
        merge_base,
        to: new_tip.to_owned(),
    })
}

pub fn run_full_walk_into(
    worktree: &Path,
    config: &GitWalkConfig,
    progress: Option<ProgressBar>,
    sink: Option<&mut TemporalShardSink>,
) -> Result<(GraphIndexArtifact, CommitIndexArtifact)> {
    let (graph, commits, _) = run_full_walk_into_with_stats(worktree, config, progress, sink)?;
    Ok((graph, commits))
}

fn run_full_walk_into_with_stats(
    worktree: &Path,
    config: &GitWalkConfig,
    progress: Option<ProgressBar>,
    mut sink: Option<&mut TemporalShardSink>,
) -> Result<(GraphIndexArtifact, CommitIndexArtifact, TemporalWalkStats)> {
    ensure_not_shallow(worktree)?;
    check_replace_refs(worktree, config.allow_replace_refs)?;

    let first_ref = config
        .target_refs
        .first()
        .context("GitWalkConfig.target_refs must contain at least one ref")?;
    let target_refs: Vec<_> = config.target_refs.iter().map(String::as_str).collect();
    let refs = snapshot_refs(worktree, &target_refs)?;
    let tip = refs
        .get(first_ref)
        .with_context(|| format!("target ref `{first_ref}` was not present after ref snapshot"))?;
    let base = load_incremental_base(
        worktree,
        first_ref,
        config.walk_strategy,
        sink.as_deref_mut(),
    )?;
    let plan = plan_incremental_walk(
        worktree,
        base.as_ref().map(|base| base.stored_tip.as_str()),
        tip,
    )?;
    let use_incremental_base =
        matches!(plan, IncrementalPlan::FastForward { .. }) && base.is_some();
    let commit_shas = if use_incremental_base {
        planned_commits(worktree, &plan, tip, config.walk_strategy)?
    } else {
        walk_commits(worktree, tip, config.walk_strategy)?
    };

    let (mut graph, mut commits) = if use_incremental_base {
        let base = base.expect("incremental base checked above");
        let mut commits = base.commits;
        commits.refs = refs.clone();
        commits.indexed_at = chrono::Utc::now().to_rfc3339();
        (base.graph, commits)
    } else {
        (
            empty_graph_artifact(),
            CommitIndexArtifact {
                schema_version: current_temporal_schema_version()?,
                commits: Vec::with_capacity(commit_shas.len()),
                refs: refs.clone(),
                indexed_at: chrono::Utc::now().to_rfc3339(),
                walk_strategy: config.walk_strategy,
            },
        )
    };
    let telemetry_enabled = tracing::enabled!(tracing::Level::DEBUG);
    let retained_budget_bytes = configured_temporal_parse_cache_budget_bytes()?;
    let shared_parse_cache = Arc::new(SharedParseCache::new(
        telemetry_enabled,
        retained_budget_bytes,
    ));
    let worker_root = Arc::new(worktree.to_path_buf());
    let initialize_root = Arc::clone(&worker_root);
    let compute_root = Arc::clone(&worker_root);
    let worker_parse_cache = Arc::clone(&shared_parse_cache);
    let reducer_parse_cache = Arc::clone(&shared_parse_cache);
    let use_gix_diff = config.use_gix_diff;
    let mut reducer = CommitResultReducer::new(&mut graph, &mut commits, progress, sink);
    let pool = run_bounded_worker_pool(
        config.temporal_jobs,
        commit_shas,
        move |_| {
            let repo = open_gix_repo_without_replace_refs(&initialize_root)?;
            let mut symbol_diff = SymbolDiffCtx::with_parse_cache(Arc::clone(&worker_parse_cache));
            symbol_diff.cat_file_batch(&initialize_root)?;
            Ok(CommitWorkerState { repo, symbol_diff })
        },
        move |worker, ordinal, sha| {
            worker.symbol_diff.set_parse_cache_ordinal(ordinal);
            let repo = use_gix_diff.then_some(&worker.repo);
            compute_commit_walk_result(ordinal, &compute_root, &sha, repo, &mut worker.symbol_diff)
        },
        |result| {
            reducer.push(result)?;
            reducer_parse_cache.evict_before(reducer.next_ordinal);
            Ok(ReductionProgress {
                next_ordinal: reducer.next_ordinal,
                pending_results: reducer.pending.len(),
            })
        },
    )?;
    reducer.finish()?;
    let stats = TemporalWalkStats {
        pool,
        shared_parse_cache_current_entries: shared_parse_cache.len(),
        shared_parse_cache_peak_entries: shared_parse_cache.peak_len(),
        shared_parse_cache: shared_parse_cache.telemetry_snapshot(),
    };
    tracing::debug!(
        temporal_jobs = config.temporal_jobs.get(),
        max_in_flight = stats.pool.max_in_flight,
        max_queued_work = stats.pool.max_queued_work,
        max_active_work = stats.pool.max_active_work,
        max_result_occupancy = stats.pool.max_result_occupancy,
        max_reducer_pending = stats.pool.max_reducer_pending,
        pool_elapsed_nanos = stats.pool.pool_elapsed_nanos,
        active_worker_nanos = stats.pool.active_worker_nanos,
        average_active_workers_milli = stats.pool.average_active_workers_milli,
        completed_out_of_order = stats.pool.completed_out_of_order,
        admission_window_full_receive_wait_nanos =
            stats.pool.admission_window_full_receive_wait_nanos,
        next_ordinal_blocked_wait_nanos = stats.pool.next_ordinal_blocked_wait_nanos,
        coordinator_send_blocked_nanos = stats.pool.coordinator_send_blocked_nanos,
        shared_parse_cache_current_entries = stats.shared_parse_cache_current_entries,
        shared_parse_cache_peak_entries = stats.shared_parse_cache_peak_entries,
        telemetry_enabled = stats.shared_parse_cache.telemetry_enabled,
        cache_hits = stats.shared_parse_cache.cache_hits,
        cold_initializations = stats.shared_parse_cache.cold_initializations,
        reparse_initializations = stats.shared_parse_cache.reparse_initializations,
        successful_initializations = stats.shared_parse_cache.successful_initializations,
        failed_initializations = stats.shared_parse_cache.failed_initializations,
        initialization_nanos = stats.shared_parse_cache.initialization_nanos,
        cold_initialization_nanos = stats.shared_parse_cache.cold_initialization_nanos,
        reparse_initialization_nanos = stats.shared_parse_cache.reparse_initialization_nanos,
        cache_hit_nanos = stats.shared_parse_cache.cache_hit_nanos,
        cache_lock_wait_nanos = stats.shared_parse_cache.lock_wait_nanos,
        evicted_entries = stats.shared_parse_cache.evicted_entries,
        retained_budget_bytes = stats.shared_parse_cache.retained_budget_bytes,
        retained_tier_hits = stats.shared_parse_cache.retained_tier_hits,
        budget_evictions = stats.shared_parse_cache.budget_evictions,
        current_retained_payload_bytes = stats.shared_parse_cache.current_retained_payload_bytes,
        peak_retained_payload_bytes = stats.shared_parse_cache.peak_retained_payload_bytes,
        current_retained_tier_payload_bytes =
            stats.shared_parse_cache.current_retained_tier_payload_bytes,
        peak_retained_tier_payload_bytes =
            stats.shared_parse_cache.peak_retained_tier_payload_bytes,
        reuse_distance_count = stats.shared_parse_cache.reuse_distance_count,
        reuse_distance_sum = stats.shared_parse_cache.reuse_distance_sum,
        reuse_distance_max = stats.shared_parse_cache.reuse_distance_max,
        current_exact_ghost_keys = stats.shared_parse_cache.current_exact_ghost_keys,
        peak_exact_ghost_keys = stats.shared_parse_cache.peak_exact_ghost_keys,
        "spur-graph: temporal worker-pool occupancy"
    );

    Ok((graph, commits, stats))
}

struct IncrementalBase {
    graph: GraphIndexArtifact,
    commits: CommitIndexArtifact,
    stored_tip: String,
}

fn load_incremental_base(
    worktree: &Path,
    first_ref: &str,
    walk_strategy: WalkStrategy,
    sink: Option<&mut TemporalShardSink>,
) -> Result<Option<IncrementalBase>> {
    let Some(pointer) = commit_index::load_pointer(worktree)? else {
        return Ok(None);
    };
    let commits = match commit_index::load_artifact(worktree, &pointer) {
        Ok(commits) => commits,
        Err(error) => {
            tracing::info!(
                error = %error,
                "spur-graph: commit-index pointer exists but prior commit-index artifact is missing or unreadable; selecting cold temporal walk"
            );
            return Ok(None);
        }
    };
    if commits.walk_strategy != walk_strategy {
        tracing::info!(
            stored = ?commits.walk_strategy,
            requested = ?walk_strategy,
            "spur-graph: commit-index walk strategy changed; selecting cold temporal walk"
        );
        return Ok(None);
    }
    let Some(stored_tip) = pointer
        .refs
        .get(first_ref)
        .or_else(|| commits.refs.get(first_ref))
        .or_else(|| commits.commits.last().map(|commit| &commit.sha))
        .cloned()
    else {
        return Ok(None);
    };

    let graph_location = match resolve_artifact_location(worktree, None) {
        Ok(location) => location,
        Err(error) => {
            tracing::info!(
                error = %error,
                "spur-graph: commit-index pointer exists but prior graph artifact is missing; selecting cold temporal walk"
            );
            return Ok(None);
        }
    };

    tracing::debug!(
        path = %graph_location.path.display(),
        "spur-graph: loading temporal-only Parquet graph artifact for incremental walk"
    );
    let mut graph = if sink.is_some() {
        load_temporal_artifact_metadata_parquet(&graph_location.path)?
    } else {
        load_temporal_artifact_parquet(&graph_location.path)?
    };
    graph.header.content_hash_blake3 = None;
    // The temporal store reuses persisted snapshot shards verbatim; only the
    // structural artifact is rebuilt when the extractor/queries change. If the
    // manifest version (which hashes the tree-sitter query bytes + schema +
    // extractor versions) has moved since this artifact was written, the stored
    // snapshots were produced by a different extractor and would silently miss
    // any newly-emitted symbol kinds. Force a cold re-walk so history is
    // re-extracted with the current extractor.
    let expected_manifest_version = crate::store::build::current_manifest_version();
    if graph.manifest_version != expected_manifest_version {
        tracing::info!(
            stored = graph.manifest_version,
            expected = expected_manifest_version,
            path = %graph_location.path.display(),
            "spur-graph: extractor/query manifest version changed; selecting cold temporal walk"
        );
        return Ok(None);
    }
    if !graph.commits.iter().any(|commit| commit.sha == stored_tip) {
        tracing::info!(
            stored_tip,
            path = %graph_location.path.display(),
            "spur-graph: prior graph artifact lacks stored tip; selecting cold temporal walk"
        );
        return Ok(None);
    }

    if let Some(sink) = sink {
        stream_temporal_artifact_parquet_into_sink(&graph_location.path, sink)?;
    }

    Ok(Some(IncrementalBase {
        graph,
        commits,
        stored_tip,
    }))
}

fn planned_commits(
    worktree: &Path,
    plan: &IncrementalPlan,
    tip: &str,
    strategy: WalkStrategy,
) -> Result<Vec<String>> {
    match plan {
        IncrementalPlan::ColdWalk { .. } => walk_commits(worktree, tip, strategy),
        IncrementalPlan::FastForward { from, to } => {
            walk_commit_range(worktree, Some(from), to, strategy)
        }
        IncrementalPlan::ForcePushRecover {
            merge_base: Some(from),
            to,
        } => walk_commit_range(worktree, Some(from), to, strategy),
        IncrementalPlan::ForcePushRecover {
            merge_base: None,
            to,
        } => walk_commits(worktree, to, strategy),
    }
}

fn walk_commits(worktree: &Path, tip: &str, strategy: WalkStrategy) -> Result<Vec<String>> {
    let mut args = vec!["rev-list", "--topo-order", "--reverse"];
    if matches!(strategy, WalkStrategy::FirstParent) {
        args.push("--first-parent");
    }
    args.push(tip);

    let stdout = run_git(worktree, &args)?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn walk_commit_range(
    worktree: &Path,
    from_exclusive: Option<&str>,
    tip: &str,
    strategy: WalkStrategy,
) -> Result<Vec<String>> {
    let Some(from_exclusive) = from_exclusive else {
        return walk_commits(worktree, tip, strategy);
    };

    let range = format!("{from_exclusive}..{tip}");
    let mut args = vec!["rev-list", "--topo-order", "--reverse"];
    if matches!(strategy, WalkStrategy::FirstParent) {
        args.push("--first-parent");
    }
    args.push(&range);

    let stdout = run_git(worktree, &args)?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn read_commit(worktree: &Path, sha: &str) -> Result<CommitArtifact> {
    let stdout = run_git(
        worktree,
        &[
            "show",
            "-s",
            "--format=%H%x00%P%x00%ct%x00%an%x00%ae%x00%s",
            sha,
        ],
    )?;
    let mut fields = stdout.trim_end_matches('\n').splitn(6, '\0');
    let actual_sha = fields
        .next()
        .filter(|field| !field.is_empty())
        .with_context(|| format!("git show emitted malformed metadata for commit `{sha}`"))?;
    let parents = fields
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let author_time = fields
        .next()
        .with_context(|| format!("git show omitted author time for commit `{sha}`"))?
        .parse::<i64>()
        .with_context(|| format!("git show emitted invalid author time for commit `{sha}`"))?;
    let author_name = fields.next().unwrap_or_default().to_owned();
    let author_email = fields.next().unwrap_or_default().to_owned();
    let summary = fields.next().unwrap_or_default().to_owned();

    Ok(CommitArtifact {
        sha: actual_sha.to_owned(),
        parents,
        author_time,
        author_name,
        author_email,
        summary,
    })
}

fn open_gix_repo_without_replace_refs(worktree: &Path) -> Result<gix::Repository> {
    // Disable git's default replace-ref honoring inside gix. We rely on the
    // pre-walk `check_replace_refs` guard to refuse repos with `refs/replace`
    // entries unless `allow_replace_refs: true` is explicitly set; if a caller
    // opts in to allow them, the CLI path uses raw OIDs, and we want gix to do
    // the same so the two paths stay equivalent.
    // gix 0.77 treats `core.useReplaceRefs=false` as permission to scan the
    // replacement namespace at open time, so route that scan to an empty
    // SPUR-private prefix while keeping the git-compatible config override.
    let mut repo = gix::open_opts(
        worktree,
        gix::open::Options::default().cli_overrides([
            "core.useReplaceRefs=false",
            "gitoxide.objects.replaceRefBase=refs/spur-disabled-replace/",
        ]),
    )
    .with_context(|| format!("open gix repository `{}`", worktree.display()))?;
    repo.object_cache_size_if_unset(128 * 1024 * 1024);
    Ok(repo)
}

fn read_commit_gix(repo: &gix::Repository, sha: &str) -> Result<CommitArtifact> {
    let oid = gix::ObjectId::from_hex(sha.as_bytes())?;
    let commit = repo.find_commit(oid)?;
    let parents = commit
        .parent_ids()
        .map(|parent| parent.to_hex().to_string())
        .collect();
    let author = commit.author()?.trim();
    let author_time = author.seconds();
    let author_name = String::from_utf8_lossy(author.name.as_ref()).into_owned();
    let author_email = String::from_utf8_lossy(author.email.as_ref()).into_owned();
    let summary = commit_subject(&commit);

    Ok(CommitArtifact {
        sha: commit.id.to_hex().to_string(),
        parents,
        author_time,
        author_name,
        author_email,
        summary,
    })
}

fn commit_subject(commit: &gix::Commit<'_>) -> String {
    let message = commit.message_raw_sloppy();
    let first_line = message
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(message, |index| &message[..index]);
    String::from_utf8_lossy(first_line).into_owned()
}

fn file_change_to_temporal_edge(commit_sha: &str, change: &FileChange) -> TemporalEdgeArtifact {
    TemporalEdgeArtifact {
        source: EdgeEndpoint::Commit {
            sha: commit_sha.to_owned(),
        },
        target: EdgeEndpoint::File {
            path: change.path.clone(),
        },
        relation: RelationKind::Touches,
        parent: change.parent_sha.clone(),
        change_kind: Some(match &change.kind {
            FileChangeKind::Added => ChangeKind::Added,
            FileChangeKind::Modified | FileChangeKind::Gitlink { .. } => ChangeKind::Modified,
            FileChangeKind::Deleted => ChangeKind::Deleted,
            FileChangeKind::Renamed { from } => {
                ChangeKind::RenamedFrom(RenamePrev::File(from.clone()))
            }
        }),
    }
}

fn empty_graph_artifact() -> GraphIndexArtifact {
    GraphIndexArtifact {
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
            content_hash_blake3: None,
        },
        manifest_version: current_manifest_version(),
        graph_content_hash: compute_graph_content_hash(std::iter::empty::<(&str, &str)>()),
        file_manifests: Vec::new(),
        files: Vec::new(),
        file_node_ids: Vec::new(),
        symbols: Vec::new(),
        symbol_node_ids: Vec::new(),
        edges: Vec::new(),
        tombstones: Vec::new(),
        diagnostics: Vec::new(),
        commits: Vec::new(),
        symbol_snapshots: Vec::new(),
        temporal_edges: Vec::new(),
        ..GraphIndexArtifact::empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed {
        from: GitPath,
    },
    Gitlink {
        old_oid: Option<String>,
        new_oid: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: GitPath,
    pub kind: FileChangeKind,
    pub parent_sha: Option<String>,
}

pub fn file_changes_for_commit(worktree: &Path, sha: &str) -> Result<Vec<FileChange>> {
    let parents = commit_parents(worktree, sha)?;
    if parents.is_empty() {
        return root_commit_changes(worktree, sha);
    }

    let mut changes = Vec::new();
    for parent in parents {
        let stdout = run_git_bytes(
            worktree,
            &[
                "-c",
                "core.quotepath=false",
                "diff-tree",
                "-r",
                "-z",
                "--raw",
                "--find-renames",
                &parent,
                sha,
            ],
        )?;
        parse_raw_diff(&stdout, Some(parent), &mut changes)?;
    }

    Ok(changes)
}

fn file_changes_for_commit_gix(repo: &gix::Repository, sha: &str) -> Result<Vec<FileChange>> {
    let oid = gix::ObjectId::from_hex(sha.as_bytes())?;
    let commit = repo.find_commit(oid)?;
    let parent_ids = commit.parent_ids().map(gix::Id::detach).collect::<Vec<_>>();

    if parent_ids.is_empty() {
        return root_commit_changes_gix(&commit);
    }

    let commit_tree = commit.tree()?;
    let mut resource_cache = repo.diff_resource_cache_for_tree_diff()?;
    let mut changes = Vec::new();

    for parent_id in parent_ids {
        let parent = repo.find_commit(parent_id)?;
        let parent_tree = parent.tree()?;
        let parent_sha = parent_id.to_hex().to_string();
        let mut platform = parent_tree.changes()?;
        platform.options(|opts| {
            opts.track_path()
                .track_rewrites(Some(gix::diff::Rewrites::default()));
        });
        platform.for_each_to_obtain_tree_with_cache(
            &commit_tree,
            &mut resource_cache,
            |change| -> std::result::Result<Action, std::convert::Infallible> {
                if let Some(change) = gix_change_to_file_change(change, &parent_sha) {
                    changes.push(change);
                }
                Ok(Action::Continue)
            },
        )?;
        resource_cache.clear_resource_cache_keep_allocation();
    }

    Ok(changes)
}

fn root_commit_changes_gix(commit: &gix::Commit<'_>) -> Result<Vec<FileChange>> {
    let tree = commit.tree()?;
    let entries = tree.traverse().breadthfirst.files()?;
    Ok(entries
        .into_iter()
        .filter(|entry| !entry.mode.is_tree())
        .map(|entry| {
            let kind = if is_gitlink(entry.mode) {
                FileChangeKind::Gitlink {
                    old_oid: None,
                    new_oid: Some(entry.oid.to_hex().to_string()),
                }
            } else {
                FileChangeKind::Added
            };
            FileChange {
                path: GitPath::from_bytes(entry.filepath.to_vec()),
                kind,
                parent_sha: None,
            }
        })
        .collect())
}

fn gix_change_to_file_change(change: Change<'_, '_, '_>, parent_sha: &str) -> Option<FileChange> {
    let parent_sha = Some(parent_sha.to_owned());
    match change {
        Change::Addition {
            location,
            entry_mode,
            id,
            ..
        } => {
            if entry_mode.is_tree() {
                return None;
            }
            let kind = if is_gitlink(entry_mode) {
                FileChangeKind::Gitlink {
                    old_oid: None,
                    new_oid: Some(id.to_hex().to_string()),
                }
            } else {
                FileChangeKind::Added
            };
            Some(FileChange {
                path: GitPath::from_bytes(location.to_vec()),
                kind,
                parent_sha,
            })
        }
        Change::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => {
            if entry_mode.is_tree() {
                return None;
            }
            let kind = if is_gitlink(entry_mode) {
                FileChangeKind::Gitlink {
                    old_oid: Some(id.to_hex().to_string()),
                    new_oid: None,
                }
            } else {
                FileChangeKind::Deleted
            };
            Some(FileChange {
                path: GitPath::from_bytes(location.to_vec()),
                kind,
                parent_sha,
            })
        }
        Change::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => {
            if previous_entry_mode.is_tree() || entry_mode.is_tree() {
                return None;
            }
            let kind = if is_gitlink(previous_entry_mode) || is_gitlink(entry_mode) {
                FileChangeKind::Gitlink {
                    old_oid: Some(previous_id.to_hex().to_string()),
                    new_oid: Some(id.to_hex().to_string()),
                }
            } else {
                FileChangeKind::Modified
            };
            Some(FileChange {
                path: GitPath::from_bytes(location.to_vec()),
                kind,
                parent_sha,
            })
        }
        Change::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            ..
        } => {
            if source_entry_mode.is_tree() || entry_mode.is_tree() {
                return None;
            }
            let kind = if is_gitlink(source_entry_mode) || is_gitlink(entry_mode) {
                FileChangeKind::Gitlink {
                    old_oid: Some(source_id.to_hex().to_string()),
                    new_oid: Some(id.to_hex().to_string()),
                }
            } else {
                FileChangeKind::Renamed {
                    from: GitPath::from_bytes(source_location.to_vec()),
                }
            };
            Some(FileChange {
                path: GitPath::from_bytes(location.to_vec()),
                kind,
                parent_sha,
            })
        }
    }
}

fn is_gitlink(mode: EntryMode) -> bool {
    mode.value() == 0o160000
}

type BlobOid = String;
type ParseCacheKey = (Language, BlobOid);
type SharedExtractedSymbols = Arc<[ExtractedSymbol]>;

/// A stable lower-bound estimate: inline symbol storage plus owned string bytes.
fn retained_payload_estimate(symbols: &[ExtractedSymbol]) -> u64 {
    symbols.iter().fold(
        u64::try_from(std::mem::size_of_val(symbols)).unwrap_or(u64::MAX),
        |total, symbol| {
            let owned_string_bytes = symbol
                .entity_name
                .len()
                .saturating_add(symbol.symbol_kind.len())
                .saturating_add(symbol.enclosing_scope.as_ref().map_or(0, String::len))
                .saturating_add(symbol.anchor_hash.len())
                .saturating_add(
                    symbol
                        .tokens
                        .iter()
                        .map(String::len)
                        .fold(0_usize, usize::saturating_add),
                );
            total.saturating_add(u64::try_from(owned_string_bytes).unwrap_or(u64::MAX))
        },
    )
}

fn stable_language_rank(language: Language) -> u8 {
    match language {
        Language::Rust => 0,
        Language::Python => 1,
        Language::TypeScript => 2,
        Language::Tsx => 3,
        Language::Javascript => 4,
        Language::Markdown => 5,
        Language::JupyterNotebook => 6,
        Language::C => 7,
        Language::Cpp => 8,
        Language::Go => 9,
        Language::Hcl => 10,
        Language::Terraform => 11,
        Language::Lua => 12,
        Language::Shell => 13,
        Language::Sql => 14,
        Language::Json => 15,
        Language::Toml => 16,
        Language::Yaml => 17,
        Language::Mermaid => 18,
        Language::Html => 19,
        Language::Css => 20,
    }
}

#[derive(Clone, Copy)]
enum ParseCacheInitializationClass {
    Cold,
    Reparse { evicted_at: usize },
}

struct ParseCacheInitializationState {
    symbols: Option<SharedExtractedSymbols>,
    class: ParseCacheInitializationClass,
}

struct SharedParseCacheEntry {
    initialization: Arc<Mutex<ParseCacheInitializationState>>,
    /// `None` marks persistent access from a standalone `SymbolDiffCtx`.
    greatest_using_ordinal: Option<usize>,
    retained_payload_bytes: u64,
    initialized: bool,
    retained_since_ordinal: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ParseCacheTelemetryStats {
    telemetry_enabled: bool,
    cache_hits: u64,
    cold_initializations: u64,
    reparse_initializations: u64,
    successful_initializations: u64,
    failed_initializations: u64,
    initialization_nanos: u64,
    cold_initialization_nanos: u64,
    reparse_initialization_nanos: u64,
    cache_hit_nanos: u64,
    lock_wait_nanos: u64,
    evicted_entries: u64,
    retained_budget_bytes: u64,
    retained_tier_hits: u64,
    budget_evictions: u64,
    current_retained_payload_bytes: u64,
    peak_retained_payload_bytes: u64,
    current_retained_tier_payload_bytes: u64,
    peak_retained_tier_payload_bytes: u64,
    reuse_distance_count: u64,
    reuse_distance_sum: u64,
    reuse_distance_max: u64,
    current_exact_ghost_keys: usize,
    peak_exact_ghost_keys: usize,
}

struct ParseCacheTelemetry {
    enabled: bool,
    cache_hits: AtomicU64,
    cold_initializations: AtomicU64,
    reparse_initializations: AtomicU64,
    successful_initializations: AtomicU64,
    failed_initializations: AtomicU64,
    initialization_nanos: AtomicU64,
    cold_initialization_nanos: AtomicU64,
    reparse_initialization_nanos: AtomicU64,
    cache_hit_nanos: AtomicU64,
    lock_wait_nanos: AtomicU64,
    evicted_entries: AtomicU64,
    retained_tier_hits: AtomicU64,
    budget_evictions: AtomicU64,
    peak_retained_payload_bytes: AtomicU64,
    peak_retained_tier_payload_bytes: AtomicU64,
    reuse_distance_count: AtomicU64,
    reuse_distance_sum: AtomicU64,
    reuse_distance_max: AtomicU64,
    peak_exact_ghost_keys: AtomicUsize,
}

impl ParseCacheTelemetry {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            cache_hits: AtomicU64::new(0),
            cold_initializations: AtomicU64::new(0),
            reparse_initializations: AtomicU64::new(0),
            successful_initializations: AtomicU64::new(0),
            failed_initializations: AtomicU64::new(0),
            initialization_nanos: AtomicU64::new(0),
            cold_initialization_nanos: AtomicU64::new(0),
            reparse_initialization_nanos: AtomicU64::new(0),
            cache_hit_nanos: AtomicU64::new(0),
            lock_wait_nanos: AtomicU64::new(0),
            evicted_entries: AtomicU64::new(0),
            retained_tier_hits: AtomicU64::new(0),
            budget_evictions: AtomicU64::new(0),
            peak_retained_payload_bytes: AtomicU64::new(0),
            peak_retained_tier_payload_bytes: AtomicU64::new(0),
            reuse_distance_count: AtomicU64::new(0),
            reuse_distance_sum: AtomicU64::new(0),
            reuse_distance_max: AtomicU64::new(0),
            peak_exact_ghost_keys: AtomicUsize::new(0),
        }
    }
}

#[derive(Default)]
struct SharedParseCacheState {
    entries: HashMap<ParseCacheKey, SharedParseCacheEntry>,
    /// Present only when exact temporal reuse telemetry is enabled.
    exact_ghost_ordinals: Option<HashMap<ParseCacheKey, usize>>,
    current_payload_bytes: u64,
    retained_tier_payload_bytes: u64,
}

/// Successful entries are shared by every extraction state while their users are active.
/// Failed initializations are removed once no same-key caller is waiting to retry them.
struct SharedParseCache {
    state: Mutex<SharedParseCacheState>,
    peak_entries: AtomicUsize,
    retained_budget_bytes: u64,
    telemetry: ParseCacheTelemetry,
}

impl Default for SharedParseCache {
    fn default() -> Self {
        Self::new(false, DEFAULT_TEMPORAL_PARSE_CACHE_BUDGET_BYTES)
    }
}

impl SharedParseCache {
    fn new(telemetry_enabled: bool, retained_budget_bytes: u64) -> Self {
        Self {
            state: Mutex::new(SharedParseCacheState {
                entries: HashMap::new(),
                exact_ghost_ordinals: telemetry_enabled.then(HashMap::new),
                current_payload_bytes: 0,
                retained_tier_payload_bytes: 0,
            }),
            peak_entries: AtomicUsize::new(0),
            retained_budget_bytes,
            telemetry: ParseCacheTelemetry::new(telemetry_enabled),
        }
    }

    #[cfg(test)]
    fn with_telemetry() -> Self {
        Self::new(true, DEFAULT_TEMPORAL_PARSE_CACHE_BUDGET_BYTES)
    }

    #[cfg(test)]
    fn with_budget(retained_budget_bytes: u64) -> Self {
        Self::new(false, retained_budget_bytes)
    }

    #[cfg(test)]
    fn with_budget_and_telemetry(retained_budget_bytes: u64) -> Self {
        Self::new(true, retained_budget_bytes)
    }

    fn lock_state(&self) -> MutexGuard<'_, SharedParseCacheState> {
        if !self.telemetry.enabled {
            return self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let started = Instant::now();
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        atomic_saturating_add(
            &self.telemetry.lock_wait_nanos,
            duration_nanos(started.elapsed()),
        );
        state
    }

    fn lock_initialization<'a>(
        &self,
        initialization: &'a Mutex<ParseCacheInitializationState>,
    ) -> MutexGuard<'a, ParseCacheInitializationState> {
        if !self.telemetry.enabled {
            return initialization
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let started = Instant::now();
        let initialized = initialization
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        atomic_saturating_add(
            &self.telemetry.lock_wait_nanos,
            duration_nanos(started.elapsed()),
        );
        initialized
    }

    fn get_or_init<F>(
        &self,
        key: ParseCacheKey,
        initialize: F,
    ) -> Result<std::result::Result<SharedExtractedSymbols, ExtractError>>
    where
        F: FnOnce() -> Result<std::result::Result<Vec<ExtractedSymbol>, ExtractError>>,
    {
        self.get_or_init_with_ordinal(None, key, initialize)
    }

    fn get_or_init_at<F>(
        &self,
        ordinal: usize,
        key: ParseCacheKey,
        initialize: F,
    ) -> Result<std::result::Result<SharedExtractedSymbols, ExtractError>>
    where
        F: FnOnce() -> Result<std::result::Result<Vec<ExtractedSymbol>, ExtractError>>,
    {
        self.get_or_init_with_ordinal(Some(ordinal), key, initialize)
    }

    fn get_or_init_with_ordinal<F>(
        &self,
        ordinal: Option<usize>,
        key: ParseCacheKey,
        initialize: F,
    ) -> Result<std::result::Result<SharedExtractedSymbols, ExtractError>>
    where
        F: FnOnce() -> Result<std::result::Result<Vec<ExtractedSymbol>, ExtractError>>,
    {
        let lookup_started = self.telemetry.enabled.then(Instant::now);
        let (entry, retained_tier_hit) = {
            let mut state = self.lock_state();
            let SharedParseCacheState {
                entries,
                exact_ghost_ordinals,
                retained_tier_payload_bytes,
                ..
            } = &mut *state;
            let (entry, retained_tier_hit) = match entries.entry(key.clone()) {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    let entry = occupied.get_mut();
                    let retained_tier_hit = entry.retained_since_ordinal.take().is_some();
                    if retained_tier_hit {
                        *retained_tier_payload_bytes = retained_tier_payload_bytes
                            .saturating_sub(entry.retained_payload_bytes);
                    }
                    entry.greatest_using_ordinal = match (entry.greatest_using_ordinal, ordinal) {
                        (Some(previous), Some(current)) => Some(previous.max(current)),
                        _ => None,
                    };
                    (Arc::clone(&entry.initialization), retained_tier_hit)
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    let evicted_at = exact_ghost_ordinals
                        .as_mut()
                        .and_then(|ghosts| ghosts.remove(&key));
                    let class = evicted_at
                        .map_or(ParseCacheInitializationClass::Cold, |evicted_at| {
                            ParseCacheInitializationClass::Reparse { evicted_at }
                        });
                    let initialization = Arc::new(Mutex::new(ParseCacheInitializationState {
                        symbols: None,
                        class,
                    }));
                    vacant.insert(SharedParseCacheEntry {
                        initialization: Arc::clone(&initialization),
                        greatest_using_ordinal: ordinal,
                        retained_payload_bytes: 0,
                        initialized: false,
                        retained_since_ordinal: None,
                    });
                    (initialization, false)
                }
            };
            self.peak_entries
                .fetch_max(entries.len(), Ordering::Relaxed);
            (entry, retained_tier_hit)
        };

        let mut cached = self.lock_initialization(&entry);
        if let Some(symbols) = cached.symbols.as_ref() {
            if self.telemetry.enabled {
                atomic_saturating_add(&self.telemetry.cache_hits, 1);
                if retained_tier_hit {
                    atomic_saturating_add(&self.telemetry.retained_tier_hits, 1);
                }
                if let Some(started) = lookup_started {
                    atomic_saturating_add(
                        &self.telemetry.cache_hit_nanos,
                        duration_nanos(started.elapsed()),
                    );
                }
            }
            return Ok(Ok(Arc::clone(symbols)));
        }

        let initialization_class = cached.class;
        if self.telemetry.enabled {
            match initialization_class {
                ParseCacheInitializationClass::Cold => {
                    atomic_saturating_add(&self.telemetry.cold_initializations, 1);
                }
                ParseCacheInitializationClass::Reparse { evicted_at } => {
                    atomic_saturating_add(&self.telemetry.reparse_initializations, 1);
                    if let Some(ordinal) = ordinal {
                        let distance = ordinal.saturating_sub(evicted_at) as u64;
                        atomic_saturating_add(&self.telemetry.reuse_distance_count, 1);
                        atomic_saturating_add(&self.telemetry.reuse_distance_sum, distance);
                        self.telemetry
                            .reuse_distance_max
                            .fetch_max(distance, Ordering::Relaxed);
                    }
                }
            }
        }
        let initialization_started = self.telemetry.enabled.then(Instant::now);
        let initialized = initialize();
        if let Some(started) = initialization_started {
            let elapsed = duration_nanos(started.elapsed());
            atomic_saturating_add(&self.telemetry.initialization_nanos, elapsed);
            match initialization_class {
                ParseCacheInitializationClass::Cold => {
                    atomic_saturating_add(&self.telemetry.cold_initialization_nanos, elapsed)
                }
                ParseCacheInitializationClass::Reparse { .. } => {
                    atomic_saturating_add(&self.telemetry.reparse_initialization_nanos, elapsed)
                }
            }
        }

        match initialized {
            Ok(Ok(symbols)) => {
                let retained_payload_bytes = retained_payload_estimate(&symbols);
                let symbols = Arc::<[ExtractedSymbol]>::from(symbols);
                cached.symbols = Some(Arc::clone(&symbols));
                drop(cached);
                let current_payload_bytes = {
                    let mut state = self.lock_state();
                    let newly_registered_payload_bytes = state
                        .entries
                        .get_mut(&key)
                        .filter(|registered| Arc::ptr_eq(&registered.initialization, &entry))
                        .and_then(|registered| {
                            if registered.initialized {
                                return None;
                            }
                            registered.retained_payload_bytes = retained_payload_bytes;
                            registered.initialized = true;
                            Some(retained_payload_bytes)
                        });
                    if let Some(newly_registered_payload_bytes) = newly_registered_payload_bytes {
                        state.current_payload_bytes = state
                            .current_payload_bytes
                            .saturating_add(newly_registered_payload_bytes);
                    }
                    state.current_payload_bytes
                };
                self.telemetry
                    .peak_retained_payload_bytes
                    .fetch_max(current_payload_bytes, Ordering::Relaxed);
                if self.telemetry.enabled {
                    atomic_saturating_add(&self.telemetry.successful_initializations, 1);
                }
                Ok(Ok(symbols))
            }
            Ok(Err(error)) => {
                if self.telemetry.enabled {
                    atomic_saturating_add(&self.telemetry.failed_initializations, 1);
                }
                drop(cached);
                self.remove_uninitialized_entry(&key, &entry);
                Ok(Err(error))
            }
            Err(error) => {
                if self.telemetry.enabled {
                    atomic_saturating_add(&self.telemetry.failed_initializations, 1);
                }
                drop(cached);
                self.remove_uninitialized_entry(&key, &entry);
                Err(error)
            }
        }
    }

    fn remove_uninitialized_entry(
        &self,
        key: &ParseCacheKey,
        entry: &Arc<Mutex<ParseCacheInitializationState>>,
    ) {
        let cached = self.lock_initialization(entry);
        if cached.symbols.is_some() {
            return;
        }
        let initialization_class = cached.class;
        let mut state = self.lock_state();
        if Arc::strong_count(entry) == 2
            && state
                .entries
                .get(key)
                .is_some_and(|registered| Arc::ptr_eq(&registered.initialization, entry))
        {
            state.entries.remove(key);
            if let ParseCacheInitializationClass::Reparse { evicted_at } = initialization_class {
                let current_exact_ghost_keys = if let Some(ghosts) = &mut state.exact_ghost_ordinals
                {
                    ghosts.insert(key.clone(), evicted_at);
                    ghosts.len()
                } else {
                    0
                };
                self.telemetry
                    .peak_exact_ghost_keys
                    .fetch_max(current_exact_ghost_keys, Ordering::Relaxed);
            }
        }
    }

    fn evict_before(&self, next_unreduced_ordinal: usize) {
        let mut state = self.lock_state();
        let mut newly_retained_payload_bytes = 0_u64;
        let mut candidates = Vec::new();
        for (key, entry) in &mut state.entries {
            let Some(greatest_using_ordinal) = entry.greatest_using_ordinal else {
                continue;
            };
            if greatest_using_ordinal >= next_unreduced_ordinal || !entry.initialized {
                continue;
            }
            if entry.retained_since_ordinal.is_none() {
                entry.retained_since_ordinal = Some(next_unreduced_ordinal);
                newly_retained_payload_bytes =
                    newly_retained_payload_bytes.saturating_add(entry.retained_payload_bytes);
            }
            candidates.push((
                greatest_using_ordinal,
                key.1.clone(),
                stable_language_rank(key.0),
                key.clone(),
            ));
        }
        state.retained_tier_payload_bytes = state
            .retained_tier_payload_bytes
            .saturating_add(newly_retained_payload_bytes);
        candidates.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });

        let mut evicted = Vec::new();
        for (_, _, _, key) in candidates {
            if self.retained_budget_bytes != 0
                && state.retained_tier_payload_bytes <= self.retained_budget_bytes
            {
                break;
            }
            let Some(entry) = state.entries.remove(&key) else {
                continue;
            };
            debug_assert!(entry.initialized);
            debug_assert!(entry.retained_since_ordinal.is_some());
            state.retained_tier_payload_bytes = state
                .retained_tier_payload_bytes
                .saturating_sub(entry.retained_payload_bytes);
            state.current_payload_bytes = state
                .current_payload_bytes
                .saturating_sub(entry.retained_payload_bytes);
            evicted.push(key);
        }
        self.telemetry
            .peak_retained_tier_payload_bytes
            .fetch_max(state.retained_tier_payload_bytes, Ordering::Relaxed);

        let evicted_entries = evicted.len();
        let current_exact_ghost_keys = if let Some(ghosts) = &mut state.exact_ghost_ordinals {
            for key in evicted {
                ghosts.insert(key, next_unreduced_ordinal);
            }
            ghosts.len()
        } else {
            0
        };
        self.telemetry
            .peak_exact_ghost_keys
            .fetch_max(current_exact_ghost_keys, Ordering::Relaxed);
        if self.telemetry.enabled {
            let evicted_entries = u64::try_from(evicted_entries).unwrap_or(u64::MAX);
            atomic_saturating_add(&self.telemetry.evicted_entries, evicted_entries);
            atomic_saturating_add(&self.telemetry.budget_evictions, evicted_entries);
        }
    }

    fn len(&self) -> usize {
        self.lock_state().entries.len()
    }

    fn peak_len(&self) -> usize {
        self.peak_entries.load(Ordering::Relaxed)
    }

    fn telemetry_snapshot(&self) -> ParseCacheTelemetryStats {
        let state = self.lock_state();
        let current_exact_ghost_keys = state.exact_ghost_ordinals.as_ref().map_or(0, HashMap::len);
        ParseCacheTelemetryStats {
            telemetry_enabled: self.telemetry.enabled,
            cache_hits: self.telemetry.cache_hits.load(Ordering::Relaxed),
            cold_initializations: self.telemetry.cold_initializations.load(Ordering::Relaxed),
            reparse_initializations: self
                .telemetry
                .reparse_initializations
                .load(Ordering::Relaxed),
            successful_initializations: self
                .telemetry
                .successful_initializations
                .load(Ordering::Relaxed),
            failed_initializations: self
                .telemetry
                .failed_initializations
                .load(Ordering::Relaxed),
            initialization_nanos: self.telemetry.initialization_nanos.load(Ordering::Relaxed),
            cold_initialization_nanos: self
                .telemetry
                .cold_initialization_nanos
                .load(Ordering::Relaxed),
            reparse_initialization_nanos: self
                .telemetry
                .reparse_initialization_nanos
                .load(Ordering::Relaxed),
            cache_hit_nanos: self.telemetry.cache_hit_nanos.load(Ordering::Relaxed),
            lock_wait_nanos: self.telemetry.lock_wait_nanos.load(Ordering::Relaxed),
            evicted_entries: self.telemetry.evicted_entries.load(Ordering::Relaxed),
            retained_budget_bytes: self.retained_budget_bytes,
            retained_tier_hits: self.telemetry.retained_tier_hits.load(Ordering::Relaxed),
            budget_evictions: self.telemetry.budget_evictions.load(Ordering::Relaxed),
            current_retained_payload_bytes: state.current_payload_bytes,
            peak_retained_payload_bytes: self
                .telemetry
                .peak_retained_payload_bytes
                .load(Ordering::Relaxed),
            current_retained_tier_payload_bytes: state.retained_tier_payload_bytes,
            peak_retained_tier_payload_bytes: self
                .telemetry
                .peak_retained_tier_payload_bytes
                .load(Ordering::Relaxed),
            reuse_distance_count: self.telemetry.reuse_distance_count.load(Ordering::Relaxed),
            reuse_distance_sum: self.telemetry.reuse_distance_sum.load(Ordering::Relaxed),
            reuse_distance_max: self.telemetry.reuse_distance_max.load(Ordering::Relaxed),
            current_exact_ghost_keys,
            peak_exact_ghost_keys: self.telemetry.peak_exact_ghost_keys.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    fn contains(&self, key: &ParseCacheKey) -> bool {
        self.lock_state().entries.contains_key(key)
    }

    #[cfg(test)]
    fn greatest_using_ordinal(&self, key: &ParseCacheKey) -> Option<Option<usize>> {
        self.lock_state()
            .entries
            .get(key)
            .map(|entry| entry.greatest_using_ordinal)
    }
}

#[derive(Default)]
struct SymbolExtractionState {
    extractors: HashMap<Language, BytesExtractor>,
    diagnostics: Vec<String>,
    cat_file_batches: HashMap<PathBuf, CatFileBatch>,
}

pub struct SymbolDiffCtx {
    local: SymbolExtractionState,
    parse_cache: Arc<SharedParseCache>,
    parse_cache_ordinal: Option<usize>,
}

impl SymbolDiffCtx {
    pub fn new() -> Self {
        Self::with_parse_cache(Arc::new(SharedParseCache::default()))
    }

    fn with_parse_cache(parse_cache: Arc<SharedParseCache>) -> Self {
        Self {
            local: SymbolExtractionState::default(),
            parse_cache,
            parse_cache_ordinal: None,
        }
    }

    fn set_parse_cache_ordinal(&mut self, ordinal: usize) {
        self.parse_cache_ordinal = Some(ordinal);
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.local.diagnostics
    }

    #[cfg(test)]
    pub(crate) fn parse_cache_len(&self) -> usize {
        self.parse_cache.len()
    }

    fn for_language(&mut self, language: Language) -> Result<&mut BytesExtractor> {
        match self.local.extractors.entry(language) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                Ok(entry.insert(BytesExtractor::for_language(language)?))
            }
        }
    }

    fn record_diagnostics(&mut self, diagnostics: Vec<String>) {
        self.local.diagnostics.extend(diagnostics);
    }

    fn cat_file_batch(&mut self, worktree: &Path) -> Result<&mut CatFileBatch> {
        match self.local.cat_file_batches.entry(worktree.to_path_buf()) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                Ok(entry.insert(CatFileBatch::new(worktree)?))
            }
        }
    }
}

impl Default for SymbolDiffCtx {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolChange {
    pub snapshot: SymbolSnapshotArtifact,
    pub change_kind: ChangeKind,
    pub parent_sha: Option<String>,
}

pub fn symbol_changes_for_commit(
    worktree: &Path,
    sha: &str,
    file_changes: &[FileChange],
    ctx: &mut SymbolDiffCtx,
) -> Result<Vec<SymbolChange>> {
    let mut out = Vec::new();
    let mut by_snapshot_key = HashMap::new();

    for file_change in file_changes {
        if matches!(file_change.kind, FileChangeKind::Gitlink { .. }) {
            let diagnostic = format!(
                "gitlink: file={} commit={} skipped submodule recursion; file-level touch retained",
                file_change.path.display(),
                sha
            );
            tracing::warn!(diagnostic = %diagnostic, "spur-graph: gitlink encountered during symbol walk");
            ctx.record_diagnostics(vec![diagnostic]);
            continue;
        }

        let current_path = file_change.path.to_path_buf();
        let Some(language) = Language::from_path(&current_path) else {
            continue;
        };

        let blobs = {
            let cat_file_batch = ctx.cat_file_batch(worktree)?;
            blobs_for_change(worktree, cat_file_batch, sha, file_change)?
        };
        if blobs
            .left
            .as_ref()
            .is_some_and(|(_, bytes)| is_binary(bytes))
            || blobs
                .right
                .as_ref()
                .is_some_and(|(_, bytes)| is_binary(bytes))
        {
            let diagnostic = format!(
                "binary_blob: file={} commit={} skipped symbol diff; file-level touch retained",
                file_change.path.display(),
                sha
            );
            tracing::warn!(diagnostic = %diagnostic, "spur-graph: binary blob encountered during symbol walk");
            ctx.record_diagnostics(vec![diagnostic]);
            continue;
        }

        let deleted_path = blobs.left_path.as_ref().unwrap_or(&file_change.path);
        let left_path_buf = blobs.left_path.as_ref().map(GitPath::to_path_buf);
        let left_result = match (left_path_buf.as_deref(), blobs.left.as_ref()) {
            (Some(path), Some((oid, bytes))) => cached_extract(ctx, language, oid, path, bytes)?,
            _ => Ok(Arc::from(Vec::<ExtractedSymbol>::new())),
        };
        let right_result = match blobs.right.as_ref() {
            Some((oid, bytes)) => cached_extract(ctx, language, oid, &current_path, bytes)?,
            None => Ok(Arc::from(Vec::<ExtractedSymbol>::new())),
        };
        let mut parse_failed = false;
        let left_symbols = match left_result {
            Ok(symbols) => symbols,
            Err(error) => {
                ctx.record_diagnostics(vec![parse_failed_diagnostic(
                    sha,
                    deleted_path,
                    "left",
                    &error,
                )]);
                parse_failed = true;
                Arc::from(Vec::<ExtractedSymbol>::new())
            }
        };
        let right_symbols = match right_result {
            Ok(symbols) => symbols,
            Err(error) => {
                ctx.record_diagnostics(vec![parse_failed_diagnostic(
                    sha,
                    &file_change.path,
                    "right",
                    &error,
                )]);
                parse_failed = true;
                Arc::from(Vec::<ExtractedSymbol>::new())
            }
        };
        if parse_failed {
            continue;
        }

        let (changes, diagnostics) =
            symbol_changes_from_extracted(sha, file_change, &left_symbols, &right_symbols);
        ctx.record_diagnostics(diagnostics);

        for change in changes {
            push_symbol_change(&mut out, &mut by_snapshot_key, change);
        }
    }

    Ok(out)
}

/// Match symbols already extracted from the two sides of a file change.
///
/// Both sides must be extracted with the language of `file_change.path`, including
/// file renames. Blob loading, binary checks, and parse diagnostics belong to the
/// caller. Unsupported paths and gitlinks produce no changes or diagnostics.
/// Candidate order and duplicate identities retain the commit walker's matching
/// semantics; use [`merge_symbol_changes`] to fold changes across a commit.
pub fn symbol_changes_from_extracted(
    sha: &str,
    file_change: &FileChange,
    left_symbols: &[ExtractedSymbol],
    right_symbols: &[ExtractedSymbol],
) -> (Vec<SymbolChange>, Vec<String>) {
    if matches!(file_change.kind, FileChangeKind::Gitlink { .. }) {
        return (Vec::new(), Vec::new());
    }
    let Some(language) = Language::from_path(&file_change.path.to_path_buf()) else {
        return (Vec::new(), Vec::new());
    };
    symbol_changes_from_extracted_with_language(
        sha,
        file_change,
        language,
        left_symbols,
        right_symbols,
    )
}

/// Notebook callers supply the resolved cell language and validate uniqueness
/// before entering this matcher. No path-extension inference or source rewriting.
pub(crate) fn symbol_changes_from_extracted_with_language(
    sha: &str,
    file_change: &FileChange,
    language: Language,
    left_symbols: &[ExtractedSymbol],
    right_symbols: &[ExtractedSymbol],
) -> (Vec<SymbolChange>, Vec<String>) {
    if matches!(file_change.kind, FileChangeKind::Gitlink { .. }) {
        return (Vec::new(), Vec::new());
    }
    let deleted_path = match &file_change.kind {
        FileChangeKind::Renamed { from } => from,
        _ => &file_change.path,
    };

    let mut direct_changes = Vec::new();
    let mut deleted_candidates = Vec::new();
    let mut added_candidates = Vec::new();

    if matches!(file_change.kind, FileChangeKind::Renamed { .. }) {
        deleted_candidates.extend(left_symbols.iter().map(|left| SymbolChange {
            snapshot: snapshot_from(sha, deleted_path, left),
            change_kind: ChangeKind::Deleted,
            parent_sha: file_change.parent_sha.clone(),
        }));
        added_candidates.extend(right_symbols.iter().map(|right| SymbolChange {
            snapshot: snapshot_from(sha, &file_change.path, right),
            change_kind: ChangeKind::Added,
            parent_sha: file_change.parent_sha.clone(),
        }));
    } else {
        let mut left_by_identity: HashMap<(String, String, Option<String>), &ExtractedSymbol> =
            left_symbols
                .iter()
                .map(|symbol| {
                    (
                        (
                            symbol.entity_name.clone(),
                            symbol.symbol_kind.clone(),
                            symbol.enclosing_scope.clone(),
                        ),
                        symbol,
                    )
                })
                .collect();

        for right in right_symbols.iter() {
            let identity = (
                right.entity_name.clone(),
                right.symbol_kind.clone(),
                right.enclosing_scope.clone(),
            );
            let right_snapshot = snapshot_from(sha, &file_change.path, right);
            match left_by_identity.remove(&identity) {
                Some(left) => {
                    let left_snapshot = snapshot_from(sha, deleted_path, left);
                    if left.anchor_hash == right.anchor_hash
                        && left_snapshot.key.stable_symbol_id == right_snapshot.key.stable_symbol_id
                    {
                        continue;
                    }
                    direct_changes.push(SymbolChange {
                        snapshot: right_snapshot,
                        change_kind: ChangeKind::Modified,
                        parent_sha: file_change.parent_sha.clone(),
                    });
                }
                None => added_candidates.push(SymbolChange {
                    snapshot: right_snapshot,
                    change_kind: ChangeKind::Added,
                    parent_sha: file_change.parent_sha.clone(),
                }),
            }
        }

        for (_, left) in left_by_identity {
            deleted_candidates.push(SymbolChange {
                snapshot: snapshot_from(sha, deleted_path, left),
                change_kind: ChangeKind::Deleted,
                parent_sha: file_change.parent_sha.clone(),
            });
        }
    }

    let (rename_changes, diagnostics) =
        detect_renames(deleted_candidates, added_candidates, file_change, language);
    direct_changes.extend(rename_changes);
    (direct_changes, diagnostics)
}

/// Fold commit changes by parent and snapshot key, retaining the first payload.
/// Conflicting change kinds for the same key are represented as `Modified`.
pub fn merge_symbol_changes(changes: impl IntoIterator<Item = SymbolChange>) -> Vec<SymbolChange> {
    let mut out = Vec::new();
    let mut by_snapshot_key = HashMap::new();
    for change in changes {
        push_symbol_change(&mut out, &mut by_snapshot_key, change);
    }
    out
}

fn push_symbol_change(
    out: &mut Vec<SymbolChange>,
    by_snapshot_key: &mut HashMap<(Option<String>, SnapshotKey), usize>,
    change: SymbolChange,
) {
    let key = (change.parent_sha.clone(), change.snapshot.key.clone());
    if let Some(existing) = by_snapshot_key.get(&key).copied() {
        let merged = merge_change_kind(&out[existing].change_kind, change.change_kind);
        out[existing].change_kind = merged;
        return;
    }

    by_snapshot_key.insert(key, out.len());
    out.push(change);
}

fn merge_change_kind(existing: &ChangeKind, incoming: ChangeKind) -> ChangeKind {
    if existing == &incoming {
        return incoming;
    }
    ChangeKind::Modified
}

#[derive(Debug, Clone)]
struct RenameMatch {
    from: SymbolChange,
    to: SymbolChange,
    score: f64,
}

#[cfg(test)]
pub(crate) fn try_rename_match(
    deleted_candidates: Vec<SymbolChange>,
    added_candidates: Vec<SymbolChange>,
    file_change: &FileChange,
    language: Language,
) -> (Vec<SymbolChange>, Vec<String>) {
    detect_renames(deleted_candidates, added_candidates, file_change, language)
}

fn detect_renames(
    deleted_candidates: Vec<SymbolChange>,
    added_candidates: Vec<SymbolChange>,
    file_change: &FileChange,
    language: Language,
) -> (Vec<SymbolChange>, Vec<String>) {
    if deleted_candidates.is_empty() || added_candidates.is_empty() {
        let mut changes = deleted_candidates;
        changes.extend(added_candidates);
        return (changes, Vec::new());
    }

    let mut diagnostics = Vec::new();
    let mut matches = Vec::new();
    let mut tier2_deleted = deleted_candidates;
    let mut tier2_added = added_candidates;

    if matches!(file_change.kind, FileChangeKind::Renamed { .. }) {
        let (tier1_matches, remaining_deleted, remaining_added) =
            tier1_file_rename_matches(tier2_deleted, tier2_added);
        matches.extend(tier1_matches);
        tier2_deleted = remaining_deleted;
        tier2_added = remaining_added;
    }

    let tier2_matches = tier2_jaccard_matches(
        &tier2_deleted,
        &tier2_added,
        file_change,
        language,
        &mut diagnostics,
    );
    matches.extend(tier2_matches);

    let matched_from: HashSet<_> = matches
        .iter()
        .map(|rename_match| rename_match.from.snapshot.key.clone())
        .collect();
    let matched_to: HashSet<_> = matches
        .iter()
        .map(|rename_match| rename_match.to.snapshot.key.clone())
        .collect();

    let mut changes = Vec::new();
    for rename_match in matches {
        let mut to = rename_match.to;
        to.change_kind =
            ChangeKind::RenamedFrom(RenamePrev::Symbol(rename_match.from.snapshot.key.clone()));
        changes.push(to);
    }

    changes.extend(
        tier2_deleted
            .into_iter()
            .filter(|change| !matched_from.contains(&change.snapshot.key)),
    );
    changes.extend(
        tier2_added
            .into_iter()
            .filter(|change| !matched_to.contains(&change.snapshot.key)),
    );

    (changes, diagnostics)
}

fn tier1_file_rename_matches(
    deleted_candidates: Vec<SymbolChange>,
    added_candidates: Vec<SymbolChange>,
) -> (Vec<RenameMatch>, Vec<SymbolChange>, Vec<SymbolChange>) {
    let mut matches = Vec::new();
    let mut used_deleted = HashSet::new();
    let mut used_added = HashSet::new();

    for (added_index, added) in added_candidates.iter().enumerate() {
        if let Some((deleted_index, deleted)) =
            deleted_candidates
                .iter()
                .enumerate()
                .find(|(deleted_index, deleted)| {
                    !used_deleted.contains(deleted_index)
                        && deleted.snapshot.entity_name == added.snapshot.entity_name
                        && deleted.snapshot.symbol_kind == added.snapshot.symbol_kind
                        && deleted.snapshot.enclosing_scope == added.snapshot.enclosing_scope
                })
        {
            used_deleted.insert(deleted_index);
            used_added.insert(added_index);
            matches.push(RenameMatch {
                from: deleted.clone(),
                to: added.clone(),
                score: 1.0,
            });
        }
    }

    let remaining_deleted = deleted_candidates
        .into_iter()
        .enumerate()
        .filter_map(|(index, change)| (!used_deleted.contains(&index)).then_some(change))
        .collect();
    let remaining_added = added_candidates
        .into_iter()
        .enumerate()
        .filter_map(|(index, change)| (!used_added.contains(&index)).then_some(change))
        .collect();

    (matches, remaining_deleted, remaining_added)
}

fn tier2_jaccard_matches(
    deleted_candidates: &[SymbolChange],
    added_candidates: &[SymbolChange],
    file_change: &FileChange,
    language: Language,
    diagnostics: &mut Vec<String>,
) -> Vec<RenameMatch> {
    let Some(threshold) = jaccard_threshold_for(language) else {
        return Vec::new();
    };

    let deleted_token_sets = token_sets_for_changes(deleted_candidates);
    let added_token_sets = token_sets_for_changes(added_candidates);
    let mut matches = Vec::new();
    for added in &added_token_sets {
        let (Some((best_deleted, best_score)), second) =
            best_two_jaccard_matches(added, &deleted_token_sets)
        else {
            continue;
        };
        if best_score < threshold {
            record_ambiguous_rename_pair(diagnostics, file_change, added.change, best_deleted);
            continue;
        }
        if let Some((second_deleted, second_score)) = second {
            if second_score >= threshold {
                record_ambiguous_rename_pair(diagnostics, file_change, added.change, best_deleted);
                record_ambiguous_rename_pair(
                    diagnostics,
                    file_change,
                    added.change,
                    second_deleted,
                );
                diagnostics.push(format!(
                    "merge_collision: file={} candidate={}",
                    file_change.path.display(),
                    added.change.snapshot.entity_name
                ));
                continue;
            }
            if best_score - second_score < RENAME_AMBIGUITY_EPSILON {
                record_ambiguous_rename_pair(diagnostics, file_change, added.change, best_deleted);
                record_ambiguous_rename_pair(
                    diagnostics,
                    file_change,
                    added.change,
                    second_deleted,
                );
                continue;
            }
        }

        matches.push(RenameMatch {
            from: best_deleted.clone(),
            to: added.change.clone(),
            score: best_score,
        });
    }

    reject_ambiguous_splits(matches, file_change, diagnostics)
}

struct ChangeTokenSet<'a> {
    change: &'a SymbolChange,
    tokens: HashSet<&'a str>,
}

fn token_sets_for_changes(changes: &[SymbolChange]) -> Vec<ChangeTokenSet<'_>> {
    changes
        .iter()
        .map(|change| ChangeTokenSet {
            change,
            tokens: change.snapshot.tokens.iter().map(String::as_str).collect(),
        })
        .collect()
}

type ScoredChange<'a> = (&'a SymbolChange, f64);
const RENAME_AMBIGUITY_EPSILON: f64 = 0.05;

fn best_two_jaccard_matches<'a>(
    added: &ChangeTokenSet<'_>,
    deleted_candidates: &'a [ChangeTokenSet<'a>],
) -> (Option<ScoredChange<'a>>, Option<ScoredChange<'a>>) {
    let mut best = None;
    let mut second = None;
    for deleted in deleted_candidates {
        let scored = (
            deleted.change,
            jaccard_token_sets(&added.tokens, &deleted.tokens),
        );
        if best.is_none_or(|(_, best_score)| score_outranks(scored.1, best_score)) {
            second = best;
            best = Some(scored);
        } else if second.is_none_or(|(_, second_score)| score_outranks(scored.1, second_score)) {
            second = Some(scored);
        }
    }
    (best, second)
}

fn score_outranks(candidate: f64, current: f64) -> bool {
    current.partial_cmp(&candidate) == Some(std::cmp::Ordering::Less)
}

fn reject_ambiguous_splits(
    matches: Vec<RenameMatch>,
    file_change: &FileChange,
    diagnostics: &mut Vec<String>,
) -> Vec<RenameMatch> {
    let mut by_deleted: HashMap<SnapshotKey, Vec<usize>> = HashMap::new();
    for (index, rename_match) in matches.iter().enumerate() {
        by_deleted
            .entry(rename_match.from.snapshot.key.clone())
            .or_default()
            .push(index);
    }

    let mut rejected = HashSet::new();
    for indexes in by_deleted.values() {
        if indexes.len() < 2 {
            continue;
        }
        let mut scores: Vec<_> = indexes
            .iter()
            .map(|index| (*index, matches[*index].score))
            .collect();
        scores.sort_by(|(_, left), (_, right)| {
            right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal)
        });
        if scores[0].1 - scores[1].1 < RENAME_AMBIGUITY_EPSILON {
            if let Some(winner) = split_tiebreak_winner(&matches, &scores) {
                for (index, _) in scores {
                    if index != winner {
                        rejected.insert(index);
                    }
                }
                continue;
            }

            for (index, _) in scores {
                rejected.insert(index);
                record_ambiguous_rename_pair(
                    diagnostics,
                    file_change,
                    &matches[index].to,
                    &matches[index].from,
                );
            }
        }
    }

    matches
        .into_iter()
        .enumerate()
        .filter_map(|(index, rename_match)| (!rejected.contains(&index)).then_some(rename_match))
        .collect()
}

fn split_tiebreak_winner(matches: &[RenameMatch], scores: &[(usize, f64)]) -> Option<usize> {
    let top_score = scores.first()?.1;
    let mut winner = None;
    let mut tied = false;

    for (index, score) in scores {
        if top_score - score >= RENAME_AMBIGUITY_EPSILON {
            continue;
        }

        let candidate = split_tiebreak_signal(&matches[*index]);
        match winner {
            None => {
                winner = Some((*index, candidate));
                tied = false;
            }
            Some((_, current)) => match candidate.cmp(&current) {
                std::cmp::Ordering::Greater => {
                    winner = Some((*index, candidate));
                    tied = false;
                }
                std::cmp::Ordering::Equal => tied = true,
                std::cmp::Ordering::Less => {}
            },
        }
    }

    if tied {
        None
    } else {
        winner.and_then(|(index, signal)| (signal.0 || signal.1).then_some(index))
    }
}

fn split_tiebreak_signal(rename_match: &RenameMatch) -> (bool, bool, std::cmp::Reverse<usize>) {
    (
        rename_match.from.snapshot.entity_name == rename_match.to.snapshot.entity_name,
        entity_names_share_significant_token(
            &rename_match.from.snapshot.entity_name,
            &rename_match.to.snapshot.entity_name,
        ),
        std::cmp::Reverse(line_start_distance(
            &rename_match.from.snapshot,
            &rename_match.to.snapshot,
        )),
    )
}

fn entity_names_share_significant_token(left: &str, right: &str) -> bool {
    significant_name_tokens(left).any(|left_token| {
        significant_name_tokens(right).any(|right_token| right_token == left_token)
    })
}

fn significant_name_tokens(name: &str) -> impl Iterator<Item = &str> {
    name.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 3)
        .filter(|token| !matches!(*token, "old" | "new"))
}

fn line_start_distance(from: &SymbolSnapshotArtifact, to: &SymbolSnapshotArtifact) -> usize {
    from.line_range[0].abs_diff(to.line_range[0])
}

fn record_ambiguous_rename_pair(
    diagnostics: &mut Vec<String>,
    file_change: &FileChange,
    left: &SymbolChange,
    right: &SymbolChange,
) {
    diagnostics.push(ambiguous_rename_diagnostic(file_change, left, right));
    diagnostics.push(ambiguous_rename_diagnostic(file_change, right, left));
}

fn ambiguous_rename_diagnostic(
    file_change: &FileChange,
    change: &SymbolChange,
    other: &SymbolChange,
) -> String {
    format!(
        "ambiguous_rename: file={} stable_symbol_id={} candidate={} other_stable_symbol_id={} other_candidate={}",
        file_change.path.display(),
        change.snapshot.key.stable_symbol_id,
        change.snapshot.entity_name,
        other.snapshot.key.stable_symbol_id,
        other.snapshot.entity_name
    )
}

fn jaccard_token_sets(a_tokens: &HashSet<&str>, b_tokens: &HashSet<&str>) -> f64 {
    let union = a_tokens.union(b_tokens).count();
    if union == 0 {
        return 0.0;
    }
    let intersection = a_tokens.intersection(b_tokens).count();
    intersection as f64 / union as f64
}

fn jaccard_threshold_for(language: Language) -> Option<f64> {
    match language {
        Language::Rust | Language::TypeScript => Some(0.7),
        Language::Python => Some(0.65),
        _ => None,
    }
}

struct ChangeBlobs {
    left_path: Option<GitPath>,
    left: Option<(String, Vec<u8>)>,
    right: Option<(String, Vec<u8>)>,
}

fn blobs_for_change(
    worktree: &Path,
    cat_file_batch: &mut CatFileBatch,
    sha: &str,
    file_change: &FileChange,
) -> Result<ChangeBlobs> {
    let right = match &file_change.kind {
        FileChangeKind::Deleted => None,
        FileChangeKind::Added | FileChangeKind::Modified | FileChangeKind::Renamed { .. } => {
            Some(cat_file_blob(
                worktree,
                cat_file_batch,
                sha,
                &file_change.path.to_path_buf(),
            )?)
        }
        FileChangeKind::Gitlink { .. } => None,
    };

    let left_path = match &file_change.kind {
        FileChangeKind::Added | FileChangeKind::Gitlink { .. } => None,
        FileChangeKind::Modified | FileChangeKind::Deleted => Some(file_change.path.clone()),
        FileChangeKind::Renamed { from } => Some(from.clone()),
    };
    let left = match (file_change.parent_sha.as_deref(), left_path.as_ref()) {
        (Some(parent), Some(path)) => Some(cat_file_blob(
            worktree,
            cat_file_batch,
            parent,
            &path.to_path_buf(),
        )?),
        _ => None,
    };

    Ok(ChangeBlobs {
        left_path,
        left,
        right,
    })
}

struct CatFileBatch {
    worktree: PathBuf,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr_drain: Option<JoinHandle<()>>,
}

impl CatFileBatch {
    fn new(worktree: &Path) -> Result<Self> {
        let mut child = Command::new("git")
            .current_dir(worktree)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawn git cat-file --batch in `{}`", worktree.display()))?;
        let stdin = child
            .stdin
            .take()
            .context("git cat-file --batch missing stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("git cat-file --batch missing stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("git cat-file --batch missing stderr")?;
        let stderr_drain = std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut sink = io::sink();
            let _ = io::copy(&mut stderr, &mut sink);
        });

        Ok(Self {
            worktree: worktree.to_path_buf(),
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr_drain: Some(stderr_drain),
        })
    }

    fn read(&mut self, sha: &str, path: &Path) -> Result<Option<(String, Vec<u8>)>> {
        let spec = blob_spec(sha, path);
        let spec_display = blob_spec_display(sha, path);
        let stdin = self
            .stdin
            .as_mut()
            .context("git cat-file --batch stdin already closed")?;
        write_blob_query(stdin, &spec).with_context(|| {
            format!(
                "write git cat-file --batch query `{spec_display}` in `{}`",
                self.worktree.display()
            )
        })?;

        let mut header = Vec::new();
        let bytes_read = self
            .stdout
            .read_until(b'\n', &mut header)
            .with_context(|| {
                format!(
                    "read git cat-file --batch header for `{spec_display}` in `{}`",
                    self.worktree.display()
                )
            })?;
        if bytes_read == 0 {
            bail!(
                "git cat-file --batch closed before header for `{spec_display}` in `{}`",
                self.worktree.display()
            );
        }
        if header.ends_with(b" missing\n") {
            return Ok(None);
        }
        if header.ends_with(b"\n") {
            header.pop();
        }

        let header = std::str::from_utf8(&header).with_context(|| {
            format!("git cat-file --batch header for `{spec_display}` emitted non-UTF-8")
        })?;
        let mut parts = header.split(' ');
        let oid = parts
            .next()
            .filter(|part| !part.is_empty())
            .with_context(|| format!("git cat-file --batch header `{header}` missing oid"))?;
        let oid = oid.to_owned();
        let object_type = parts
            .next()
            .with_context(|| format!("git cat-file --batch header `{header}` missing type"))?;
        let size = parts
            .next()
            .with_context(|| format!("git cat-file --batch header `{header}` missing size"))?;
        if parts.next().is_some() {
            bail!("unexpected git cat-file --batch header `{header}` for `{spec_display}`");
        }
        if object_type != "blob" {
            bail!(
                "git cat-file --batch `{spec_display}` resolved {oid} as `{object_type}`, expected blob"
            );
        }
        let size = size.parse::<usize>().with_context(|| {
            format!("parse git cat-file --batch size `{size}` for `{spec_display}`")
        })?;
        let mut bytes = vec![0; size];
        self.stdout.read_exact(&mut bytes).with_context(|| {
            format!(
                "read git cat-file --batch body for `{spec_display}` in `{}`",
                self.worktree.display()
            )
        })?;
        let mut trailing = [0; 1];
        self.stdout.read_exact(&mut trailing).with_context(|| {
            format!(
                "read git cat-file --batch trailing newline for `{spec_display}` in `{}`",
                self.worktree.display()
            )
        })?;
        if trailing != [b'\n'] {
            bail!("git cat-file --batch body for `{spec_display}` missing trailing newline");
        }

        Ok(Some((oid, bytes)))
    }
}

impl Drop for CatFileBatch {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
        if let Some(stderr_drain) = self.stderr_drain.take() {
            let _ = stderr_drain.join();
        }
    }
}

fn cat_file_blob(
    worktree: &Path,
    cat_file_batch: &mut CatFileBatch,
    sha: &str,
    path: &Path,
) -> Result<(String, Vec<u8>)> {
    let spec_display = blob_spec_display(sha, path);
    let missing_blob_name = || {
        blob_oid_for_path(worktree, sha, path)
            .map(|oid| format!("{oid} (`{spec_display}`)"))
            .unwrap_or_else(|_| spec_display.clone())
    };

    match cat_file_batch.read(sha, path) {
        Ok(Some(blob)) => Ok(blob),
        Ok(None) if has_promisor_remote(worktree) => {
            let missing = missing_blob_name();
            let first_error = missing_batch_error(worktree, &spec_display);
            tracing::warn!(
                blob = %missing,
                error = %first_error,
                "spur-graph: missing blob during git walk; retrying once to trigger promisor fetch"
            );
            cat_file_blob_legacy(worktree, sha, path).with_context(|| {
                format!("missing blob `{missing}` not recovered by promisor remote; fail-closed for this commit")
            })?;
            match cat_file_batch.read(sha, path) {
                Ok(Some(blob)) => Ok(blob),
                Ok(None) => Err(missing_batch_error(worktree, &spec_display).context(format!(
                    "missing blob `{missing}` not recovered by promisor remote; fail-closed for this commit"
                ))),
                Err(error) => Err(error.context(format!(
                    "missing blob `{missing}` not recovered by promisor remote; fail-closed for this commit"
                ))),
            }
        }
        Ok(None) => {
            let missing = missing_blob_name();
            Err(
                missing_batch_error(worktree, &spec_display).context(format!(
                    "missing blob `{missing}`; partial clone? fail-closed"
                )),
            )
        }
        Err(error) => {
            let missing = missing_blob_name();
            Err(error.context(format!(
                "missing blob `{missing}`; partial clone? fail-closed"
            )))
        }
    }
}

fn cat_file_blob_legacy(worktree: &Path, sha: &str, path: &Path) -> Result<(String, Vec<u8>)> {
    let spec = blob_spec(sha, path);
    let spec_display = blob_spec_display(sha, path);
    let output = Command::new("git")
        .current_dir(worktree)
        .args(["cat-file", "blob"])
        .arg(&spec)
        .output()
        .with_context(|| {
            format!(
                "spawn git cat-file blob `{spec_display}` in `{}`",
                worktree.display()
            )
        })?;

    if output.status.success() {
        Ok((blob_oid_for_path(worktree, sha, path)?, output.stdout))
    } else {
        Err(anyhow!(
            "git cat-file blob `{spec_display}` failed in `{}`: {}",
            worktree.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn missing_batch_error(worktree: &Path, spec_display: &str) -> anyhow::Error {
    anyhow!(
        "git cat-file --batch reported `{spec_display}` missing in `{}`",
        worktree.display()
    )
}

fn blob_oid_for_path(worktree: &Path, sha: &str, path: &Path) -> Result<String> {
    let spec = blob_spec(sha, path);
    let spec_display = blob_spec_display(sha, path);
    let output = Command::new("git")
        .current_dir(worktree)
        .arg("rev-parse")
        .arg(&spec)
        .output()
        .with_context(|| {
            format!(
                "spawn git rev-parse `{spec_display}` in `{}`",
                worktree.display()
            )
        })?;

    if !output.status.success() {
        bail!(
            "git rev-parse `{spec_display}` failed in `{}`: {}",
            worktree.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let oid = String::from_utf8(output.stdout)
        .with_context(|| format!("git rev-parse `{spec_display}` emitted non-UTF-8 stdout"))?
        .trim()
        .to_owned();
    if oid.is_empty() {
        bail!("git rev-parse `{spec_display}` returned an empty oid");
    }

    Ok(oid)
}

#[cfg(unix)]
fn blob_spec(sha: &str, path: &Path) -> OsString {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let mut bytes = Vec::with_capacity(sha.len() + 1 + path.as_os_str().as_bytes().len());
    bytes.extend_from_slice(sha.as_bytes());
    bytes.push(b':');
    bytes.extend_from_slice(path.as_os_str().as_bytes());
    OsString::from_vec(bytes)
}

#[cfg(not(unix))]
fn blob_spec(sha: &str, path: &Path) -> OsString {
    OsString::from(format!("{sha}:{}", path.to_string_lossy()))
}

fn write_blob_query(stdin: &mut ChildStdin, spec: &OsString) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        stdin.write_all(spec.as_os_str().as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        stdin.write_all(spec.to_string_lossy().as_bytes())?;
    }
    stdin.write_all(b"\n")?;
    stdin.flush()
}

fn blob_spec_display(sha: &str, path: &Path) -> String {
    format!("{sha}:{}", path.to_string_lossy())
}

fn has_promisor_remote(worktree: &Path) -> bool {
    run_git(
        worktree,
        &["config", "--get-regexp", r"^remote\..*\.promisor$"],
    )
    .map(|stdout| !stdout.trim().is_empty())
    .unwrap_or(false)
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

fn cached_extract(
    ctx: &mut SymbolDiffCtx,
    language: Language,
    oid: &str,
    path: &Path,
    bytes: &[u8],
) -> Result<std::result::Result<SharedExtractedSymbols, ExtractError>> {
    let key = (language, oid.to_owned());
    let parse_cache = Arc::clone(&ctx.parse_cache);
    let ordinal = ctx.parse_cache_ordinal;
    let initialize = || {
        let extractor = ctx.for_language(language)?;
        Ok(extractor.extract(path, bytes))
    };
    match ordinal {
        Some(ordinal) => parse_cache.get_or_init_at(ordinal, key, initialize),
        None => parse_cache.get_or_init(key, initialize),
    }
}

fn parse_failed_diagnostic(
    commit: &str,
    path: &GitPath,
    side: &str,
    error: &ExtractError,
) -> String {
    format!(
        "parse_failed: file={} sha={} side={} error={}; skipped symbol diff, file-level touch retained",
        path.display(),
        commit,
        side,
        error
    )
}

pub(crate) fn snapshot_from(
    commit: &str,
    path: &GitPath,
    symbol: &ExtractedSymbol,
) -> SymbolSnapshotArtifact {
    let relative_path = path.to_string_lossy();
    let fqn = match symbol.enclosing_scope.as_deref() {
        Some(scope) => format!("{scope}::{}", symbol.entity_name),
        None => symbol.entity_name.clone(),
    };
    SymbolSnapshotArtifact {
        key: SnapshotKey {
            stable_symbol_id: crate::identity::stable_symbol_id_for_discriminator(
                relative_path.as_ref(),
                &fqn,
                &symbol.symbol_kind,
                symbol.byte_range[0] as u64,
            ),
            commit: commit.to_owned(),
        },
        file_path: path.clone(),
        entity_name: symbol.entity_name.clone(),
        symbol_kind: symbol.symbol_kind.clone(),
        enclosing_scope: symbol.enclosing_scope.clone(),
        byte_range: symbol.byte_range,
        line_range: symbol.line_range,
        anchor_hash: symbol.anchor_hash.clone(),
        tokens: symbol.tokens.clone(),
    }
}

fn current_temporal_schema_version() -> Result<u32> {
    GRAPH_INDEX_VERSION_TEMPORAL.parse().with_context(|| {
        format!("parse temporal graph index version `{GRAPH_INDEX_VERSION_TEMPORAL}`")
    })
}

fn commit_parents(worktree: &Path, sha: &str) -> Result<Vec<String>> {
    let stdout = run_git(worktree, &["rev-list", "--parents", "-n", "1", sha])?;
    let mut fields = stdout.split_whitespace();
    fields.next();
    Ok(fields.map(str::to_string).collect())
}

fn root_commit_changes(worktree: &Path, sha: &str) -> Result<Vec<FileChange>> {
    let stdout = run_git_bytes(
        worktree,
        &["-c", "core.quotepath=false", "ls-tree", "-r", "-z", sha],
    )?;

    parse_ls_tree_root(&stdout)
}

fn parse_ls_tree_root(stdout: &[u8]) -> Result<Vec<FileChange>> {
    nul_fields(stdout)
        .map(|entry| {
            let (header, path) = split_once(entry, b'\t').with_context(|| {
                format!(
                    "git ls-tree emitted entry without path: `{}`",
                    String::from_utf8_lossy(entry)
                )
            })?;
            let header = std::str::from_utf8(header)
                .with_context(|| format!("git ls-tree emitted non-UTF-8 metadata: {header:?}"))?;
            let mut fields = header.split_whitespace();
            let mode = fields
                .next()
                .with_context(|| format!("git ls-tree emitted malformed entry `{header}`"))?;
            let object_type = fields
                .next()
                .with_context(|| format!("git ls-tree emitted malformed entry `{header}`"))?;
            let oid = fields
                .next()
                .with_context(|| format!("git ls-tree emitted malformed entry `{header}`"))?;
            let kind = if mode == "160000" || object_type == "commit" {
                FileChangeKind::Gitlink {
                    old_oid: None,
                    new_oid: oid_option(oid),
                }
            } else {
                FileChangeKind::Added
            };

            Ok(FileChange {
                path: GitPath::from_bytes(path.to_vec()),
                kind,
                parent_sha: None,
            })
        })
        .collect()
}

fn parse_raw_diff(
    stdout: &[u8],
    parent_sha: Option<String>,
    changes: &mut Vec<FileChange>,
) -> Result<()> {
    let mut fields = nul_fields(stdout);

    while let Some(header) = fields.next() {
        let header = std::str::from_utf8(header)
            .with_context(|| format!("git diff-tree emitted non-UTF-8 raw header {header:?}"))?;
        let mut parts = header.split_whitespace();
        let old_mode = parts
            .next()
            .and_then(|mode| mode.strip_prefix(':'))
            .with_context(|| format!("git diff-tree emitted malformed raw header `{header}`"))?;
        let new_mode = parts
            .next()
            .with_context(|| format!("git diff-tree emitted malformed raw header `{header}`"))?;
        let old_oid = parts
            .next()
            .with_context(|| format!("git diff-tree emitted malformed raw header `{header}`"))?;
        let new_oid = parts
            .next()
            .with_context(|| format!("git diff-tree emitted malformed raw header `{header}`"))?;
        let status = parts
            .next()
            .with_context(|| format!("git diff-tree emitted malformed raw header `{header}`"))?;
        let status_kind = status.as_bytes().first().copied().unwrap_or_default();

        let path1 = fields.next().with_context(|| {
            format!(
                "git diff-tree emitted raw status `{}` without a path",
                String::from_utf8_lossy(status.as_bytes())
            )
        })?;

        let kind = if old_mode == "160000" || new_mode == "160000" {
            FileChangeKind::Gitlink {
                old_oid: oid_option(old_oid),
                new_oid: oid_option(new_oid),
            }
        } else {
            match status_kind {
                b'A' => FileChangeKind::Added,
                b'M' | b'T' => FileChangeKind::Modified,
                b'D' => FileChangeKind::Deleted,
                b'R' | b'C' => FileChangeKind::Renamed {
                    from: GitPath::from_bytes(path1.to_vec()),
                },
                other => bail!(
                    "unexpected diff status `{}` in `{status}`",
                    char::from(other)
                ),
            }
        };

        let path = match &kind {
            FileChangeKind::Renamed { .. } | FileChangeKind::Gitlink { .. }
                if status_kind == b'R' || status_kind == b'C' =>
            {
                let path2 = fields.next().with_context(|| {
                    format!(
                        "git diff-tree emitted rename/copy `{status}` without a destination path"
                    )
                })?;
                GitPath::from_bytes(path2.to_vec())
            }
            FileChangeKind::Added
            | FileChangeKind::Modified
            | FileChangeKind::Deleted
            | FileChangeKind::Gitlink { .. } => GitPath::from_bytes(path1.to_vec()),
            FileChangeKind::Renamed { .. } => {
                unreachable!("kind=Renamed implies status_kind in [R,C]; see lines 1339-1340")
            }
        };

        changes.push(FileChange {
            path,
            kind,
            parent_sha: parent_sha.clone(),
        });
    }

    Ok(())
}

fn oid_option(oid: &str) -> Option<String> {
    (!oid.as_bytes().iter().all(|byte| *byte == b'0')).then(|| oid.to_owned())
}

fn split_once(bytes: &[u8], needle: u8) -> Option<(&[u8], &[u8])> {
    bytes
        .iter()
        .position(|byte| *byte == needle)
        .map(|index| (&bytes[..index], &bytes[index + 1..]))
}

fn nul_fields(stdout: &[u8]) -> impl Iterator<Item = &[u8]> {
    stdout.split(|b| *b == 0).filter(|field| !field.is_empty())
}

fn git_dir(worktree: &Path) -> Result<std::path::PathBuf> {
    let stdout = run_git(
        worktree,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    )?;
    Ok(std::path::PathBuf::from(stdout.trim()))
}

pub(crate) fn run_git_bytes(worktree: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(worktree)
        .args(args)
        .output()
        .with_context(|| format!("spawn git {args:?} in `{}`", worktree.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "git {:?} failed in `{}`: {}",
            args,
            worktree.display(),
            stderr.trim()
        ));
    }

    Ok(output.stdout)
}

pub(crate) fn run_git(worktree: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(worktree)
        .args(args)
        .output()
        .with_context(|| format!("spawn git {args:?} in `{}`", worktree.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "git {:?} failed in `{}`: {}",
            args,
            worktree.display(),
            stderr.trim()
        ));
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("git {args:?} emitted non-UTF-8 stdout"))
}

#[cfg(test)]
#[path = "rename_corpus_tests.rs"]
mod rename_corpus_tests;

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::schema::ChangeKind;

    use super::*;

    fn init_repo(dir: &std::path::Path) {
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
        ] {
            run_git(dir, &args).unwrap();
        }
    }

    fn commit(dir: &std::path::Path, msg: &str) -> String {
        run_git(dir, &["add", "-A"]).unwrap();
        run_git(dir, &["commit", "-q", "--allow-empty", "-m", msg]).unwrap();
        run_git(dir, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned()
    }

    fn commit_walk_result(ordinal: usize) -> CommitWalkResult {
        let sha = format!("commit-{ordinal}");
        let parent_sha = ordinal
            .checked_sub(1)
            .map(|parent| format!("commit-{parent}"));
        let snapshot_key = SnapshotKey {
            stable_symbol_id: format!("symbol-{ordinal}"),
            commit: sha.clone(),
        };
        let change_kind = match ordinal.checked_sub(1) {
            Some(parent) => ChangeKind::RenamedFrom(RenamePrev::Symbol(SnapshotKey {
                stable_symbol_id: format!("symbol-{parent}"),
                commit: format!("commit-{parent}"),
            })),
            None => ChangeKind::Added,
        };
        let snapshot = SymbolSnapshotArtifact {
            key: snapshot_key.clone(),
            file_path: GitPath::from("lib.rs"),
            entity_name: format!("symbol_{ordinal}"),
            symbol_kind: "function".into(),
            enclosing_scope: None,
            byte_range: [ordinal, ordinal + 1],
            line_range: [ordinal + 1, ordinal + 1],
            anchor_hash: format!("anchor-{ordinal}"),
            tokens: vec![format!("token-{ordinal}")],
        };
        let mut temporal_edges = vec![TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: sha.clone() },
            target: EdgeEndpoint::Snapshot {
                key: snapshot_key.clone(),
            },
            relation: RelationKind::Touches,
            parent: parent_sha.clone(),
            change_kind: Some(change_kind.clone()),
        }];
        if let ChangeKind::RenamedFrom(RenamePrev::Symbol(previous_key)) = &change_kind {
            temporal_edges.push(TemporalEdgeArtifact {
                source: EdgeEndpoint::Snapshot {
                    key: previous_key.clone(),
                },
                target: EdgeEndpoint::Snapshot { key: snapshot_key },
                relation: RelationKind::Touches,
                parent: parent_sha.clone(),
                change_kind: Some(change_kind),
            });
        }

        CommitWalkResult {
            ordinal,
            commit: CommitArtifact {
                sha,
                parents: parent_sha.iter().cloned().collect(),
                author_time: ordinal as i64,
                author_name: "Test Author".into(),
                author_email: "test@example.com".into(),
                summary: format!("commit {ordinal}"),
            },
            temporal_edges,
            symbol_snapshots: vec![snapshot],
            diagnostics: vec![format!("diagnostic-{ordinal}")],
        }
    }

    fn commit_index(indexed_at: &str) -> CommitIndexArtifact {
        CommitIndexArtifact {
            schema_version: GRAPH_INDEX_VERSION_TEMPORAL.parse().unwrap(),
            commits: Vec::new(),
            refs: BTreeMap::new(),
            indexed_at: indexed_at.into(),
            walk_strategy: WalkStrategy::Reachable,
        }
    }

    fn apply_serial_reference(
        results: Vec<CommitWalkResult>,
    ) -> (GraphIndexArtifact, CommitIndexArtifact) {
        let mut graph = empty_graph_artifact();
        let mut commits = commit_index("serial-indexed-at");
        for mut result in results {
            graph.commits.push(result.commit.clone());
            commits.commits.push(result.commit);
            graph.temporal_edges.append(&mut result.temporal_edges);
            graph.symbol_snapshots.append(&mut result.symbol_snapshots);
            graph.diagnostics.append(&mut result.diagnostics);
        }
        (graph, commits)
    }

    fn normalize_walk_output(
        mut output: (GraphIndexArtifact, CommitIndexArtifact),
    ) -> (GraphIndexArtifact, CommitIndexArtifact) {
        output.1.indexed_at.clear();
        output
    }

    #[test]
    fn ordinal_reducer_buffers_out_of_order_results_until_gap_closes() {
        let mut graph = empty_graph_artifact();
        let mut commits = commit_index("reducer-indexed-at");
        let progress = ProgressBar::hidden();
        let progress_observer = progress.clone();
        let mut reducer = CommitResultReducer::new(&mut graph, &mut commits, Some(progress), None);

        reducer.push(commit_walk_result(1)).unwrap();

        assert_eq!(reducer.next_ordinal, 0);
        assert_eq!(reducer.pending.keys().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(progress_observer.position(), 0);

        reducer.push(commit_walk_result(0)).unwrap();
        reducer.finish().unwrap();

        assert_eq!(
            graph
                .commits
                .iter()
                .map(|commit| commit.sha.as_str())
                .collect::<Vec<_>>(),
            ["commit-0", "commit-1"]
        );
        assert_eq!(graph.commits, commits.commits);
        assert_eq!(graph.diagnostics, ["diagnostic-0", "diagnostic-1"]);
        assert_eq!(progress_observer.position(), 2);
    }

    #[test]
    fn ordinal_reducer_matches_normalized_serial_application() {
        let serial_results = (0..3).map(commit_walk_result).collect::<Vec<_>>();
        let expected = apply_serial_reference(serial_results.clone());
        let mut graph = empty_graph_artifact();
        let mut commits = commit_index("reducer-indexed-at");
        let mut reducer = CommitResultReducer::new(&mut graph, &mut commits, None, None);

        for ordinal in [2, 0, 1] {
            reducer.push(serial_results[ordinal].clone()).unwrap();
        }
        reducer.finish().unwrap();

        assert_eq!(
            normalize_walk_output((graph, commits)),
            normalize_walk_output(expected)
        );
    }

    #[test]
    fn worker_pool_forces_out_of_order_completion_but_reduces_deterministically() {
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let worker_gate = Arc::clone(&gate);
        let reducer_gate = Arc::clone(&gate);
        let progress = ProgressBar::hidden();
        let progress_observer = progress.clone();
        let mut graph = empty_graph_artifact();
        let mut commits = commit_index("worker-pool-indexed-at");
        let mut reducer = CommitResultReducer::new(&mut graph, &mut commits, Some(progress), None);

        let stats = run_bounded_worker_pool(
            std::num::NonZeroUsize::new(2).unwrap(),
            vec![(), ()],
            |_| Ok(()),
            move |_, ordinal, ()| {
                if ordinal == 0 {
                    let (released, condvar) = &*worker_gate;
                    let mut released = released.lock().unwrap();
                    while !*released {
                        released = condvar.wait(released).unwrap();
                    }
                }
                Ok(commit_walk_result(ordinal))
            },
            |result| {
                let ordinal = result.ordinal;
                reducer.push(result)?;
                if ordinal == 1 {
                    assert_eq!(reducer.next_ordinal, 0);
                    assert_eq!(reducer.pending.keys().copied().collect::<Vec<_>>(), [1]);
                    let (released, condvar) = &*reducer_gate;
                    *released.lock().unwrap() = true;
                    condvar.notify_one();
                }
                Ok(ReductionProgress {
                    next_ordinal: reducer.next_ordinal,
                    pending_results: reducer.pending.len(),
                })
            },
        )
        .unwrap();
        reducer.finish().unwrap();

        assert_eq!(
            graph
                .commits
                .iter()
                .map(|commit| commit.sha.as_str())
                .collect::<Vec<_>>(),
            ["commit-0", "commit-1"]
        );
        assert_eq!(graph.commits, commits.commits);
        assert_eq!(graph.diagnostics, ["diagnostic-0", "diagnostic-1"]);
        assert_eq!(progress_observer.position(), 2);
        assert_eq!(stats.max_in_flight, 2);
        assert_eq!(stats.max_reducer_pending, 1);
    }

    #[test]
    fn worker_pool_joins_workers_before_reducer_panic_escapes() {
        struct WorkerLifetime {
            live: Option<Arc<std::sync::atomic::AtomicUsize>>,
        }

        impl Drop for WorkerLifetime {
            fn drop(&mut self) {
                if let Some(live) = &self.live {
                    live.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }

        let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let initializer_live = Arc::clone(&live);
        let compute_started = Arc::clone(&started);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = run_bounded_worker_pool(
                std::num::NonZeroUsize::new(2).unwrap(),
                vec![(), ()],
                move |worker_id| {
                    let live = (worker_id == 1).then(|| {
                        initializer_live.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Arc::clone(&initializer_live)
                    });
                    Ok(WorkerLifetime { live })
                },
                move |_, ordinal, ()| {
                    compute_started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if ordinal == 1 {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    Ok(ordinal)
                },
                |ordinal| {
                    if ordinal == 0 {
                        while started.load(std::sync::atomic::Ordering::SeqCst) < 2 {
                            std::thread::yield_now();
                        }
                        panic!("forced reducer panic");
                    }
                    Ok(ReductionProgress {
                        next_ordinal: 0,
                        pending_results: 0,
                    })
                },
            );
        }));

        assert!(unwind.is_err());
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn worker_failure_cancels_queued_work_and_joins_every_worker() {
        struct WorkerLifetime {
            live: Arc<std::sync::atomic::AtomicUsize>,
            dropped: Arc<std::sync::atomic::AtomicUsize>,
        }

        impl Drop for WorkerLifetime {
            fn drop(&mut self) {
                self.live.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                self.dropped
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let jobs = std::num::NonZeroUsize::new(3).unwrap();
        let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let started = Arc::new(Mutex::new(Vec::new()));
        let ordinal_zero_gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let initializer_live = Arc::clone(&live);
        let initializer_dropped = Arc::clone(&dropped);
        let compute_started = Arc::clone(&started);
        let compute_gate = Arc::clone(&ordinal_zero_gate);
        let mut graph = empty_graph_artifact();
        let mut commits = commit_index("worker-failure-indexed-at");
        let mut reducer = CommitResultReducer::new(&mut graph, &mut commits, None, None);

        let error = run_bounded_worker_pool(
            jobs,
            vec![(); 4],
            move |_| {
                initializer_live.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(WorkerLifetime {
                    live: Arc::clone(&initializer_live),
                    dropped: Arc::clone(&initializer_dropped),
                })
            },
            move |_, ordinal, ()| {
                compute_started.lock().unwrap().push(ordinal);
                match ordinal {
                    0 => {
                        let (released, condvar) = &*compute_gate;
                        let mut released = released.lock().unwrap();
                        while !*released {
                            released = condvar.wait(released).unwrap();
                        }
                        drop(released);
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                    1 => {
                        let (released, condvar) = &*compute_gate;
                        let mut released = released.lock().unwrap();
                        while !*released {
                            released = condvar.wait(released).unwrap();
                        }
                        drop(released);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        bail!("forced late worker failure");
                    }
                    _ => {}
                }
                Ok(commit_walk_result(ordinal))
            },
            |result| {
                assert_eq!(result.ordinal, 2);
                reducer.push(result)?;
                let (released, condvar) = &*ordinal_zero_gate;
                *released.lock().unwrap() = true;
                condvar.notify_all();
                // Release one slot so ordinal 3 is buffered behind the still-active ordinal 0.
                Ok(ReductionProgress {
                    next_ordinal: 1,
                    pending_results: reducer.pending.len(),
                })
            },
        )
        .unwrap_err();
        drop(reducer);

        let message = format!("{error:#}");
        assert!(message.contains("ordinal 1"), "{message}");
        assert!(message.contains("forced late worker failure"), "{message}");
        let mut started = started.lock().unwrap().clone();
        started.sort_unstable();
        assert_eq!(started, [0, 1, 2]);
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            jobs.get()
        );
        assert!(graph.commits.is_empty());
        assert!(commits.commits.is_empty());
        assert!(graph.diagnostics.is_empty());
    }

    #[test]
    fn sliding_window_bounds_every_occupancy_class_when_ordinal_zero_stalls() {
        for jobs in [1, 2, 4, 8].map(|jobs| std::num::NonZeroUsize::new(jobs).unwrap()) {
            let gate = Arc::new((Mutex::new(jobs.get() == 1), std::sync::Condvar::new()));
            let worker_gate = Arc::clone(&gate);
            let reducer_gate = Arc::clone(&gate);
            let mut graph = empty_graph_artifact();
            let mut commits = commit_index("occupancy-indexed-at");
            let mut reducer = CommitResultReducer::new(&mut graph, &mut commits, None, None);

            let stats = run_bounded_worker_pool(
                jobs,
                vec![(); jobs.get() * 3],
                |_| Ok(()),
                move |_, ordinal, ()| {
                    if ordinal == 0 {
                        let (released, condvar) = &*worker_gate;
                        let mut released = released.lock().unwrap();
                        while !*released {
                            released = condvar.wait(released).unwrap();
                        }
                    }
                    Ok(commit_walk_result(ordinal))
                },
                |result| {
                    reducer.push(result)?;
                    if reducer.next_ordinal == 0
                        && reducer.pending.len() == jobs.get().saturating_sub(1)
                    {
                        let (released, condvar) = &*reducer_gate;
                        *released.lock().unwrap() = true;
                        condvar.notify_one();
                    }
                    Ok(ReductionProgress {
                        next_ordinal: reducer.next_ordinal,
                        pending_results: reducer.pending.len(),
                    })
                },
            )
            .unwrap();
            reducer.finish().unwrap();

            eprintln!(
                "temporal worker occupancy jobs={}: in_flight={} queued={} active={} results={} reducer_pending={}",
                jobs,
                stats.max_in_flight,
                stats.max_queued_work,
                stats.max_active_work,
                stats.max_result_occupancy,
                stats.max_reducer_pending,
            );
            assert_eq!(stats.max_in_flight, jobs.get());
            assert!(stats.max_queued_work <= jobs.get());
            assert!(stats.max_active_work <= jobs.get());
            assert_eq!(stats.max_result_occupancy, jobs.get());
            assert_eq!(stats.max_reducer_pending, jobs.get().saturating_sub(1));
            assert!(stats.pool_elapsed_nanos > 0);
            assert!(stats.active_worker_nanos > 0);
            assert!(stats.average_active_workers_milli <= jobs.get() as u64 * 1_000);
            assert!(stats.coordinator_send_blocked_nanos <= stats.pool_elapsed_nanos);
            if jobs.get() == 1 {
                assert_eq!(stats.completed_out_of_order, 0);
                assert_eq!(stats.next_ordinal_blocked_wait_nanos, 0);
            } else {
                assert!(stats.completed_out_of_order > 0);
                assert!(stats.admission_window_full_receive_wait_nanos > 0);
                assert!(stats.next_ordinal_blocked_wait_nanos > 0);
            }
            assert_eq!(graph.commits.len(), jobs.get() * 3);
            assert_eq!(graph.commits, commits.commits);
        }
    }

    #[test]
    fn temporal_jobs_1_2_4_8_match_reference_for_every_walk_strategy() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("lib.rs"), b"pub fn root() -> u8 { 1 }\n").unwrap();
        commit(dir.path(), "root");
        run_git(dir.path(), &["checkout", "-q", "-b", "side"]).unwrap();
        std::fs::write(dir.path().join("side.py"), b"def side():\n    return 2\n").unwrap();
        commit(dir.path(), "side");
        run_git(dir.path(), &["checkout", "-q", "main"]).unwrap();
        std::fs::write(dir.path().join("lib.rs"), b"pub fn root() -> u8 { 3 }\n").unwrap();
        commit(dir.path(), "main");
        run_git(
            dir.path(),
            &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
        )
        .unwrap();
        std::fs::rename(dir.path().join("lib.rs"), dir.path().join("core.rs")).unwrap();
        commit(dir.path(), "rename");

        for strategy in [WalkStrategy::Reachable, WalkStrategy::FirstParent] {
            let (reference_graph, reference_commits, reference_stats) =
                run_full_walk_into_with_stats(
                    dir.path(),
                    &GitWalkConfig {
                        walk_strategy: strategy,
                        temporal_jobs: std::num::NonZeroUsize::new(1).unwrap(),
                        ..GitWalkConfig::default()
                    },
                    None,
                    None,
                )
                .unwrap();
            let expected = normalize_walk_output((reference_graph, reference_commits));
            assert_eq!(reference_stats.shared_parse_cache_current_entries, 0);
            assert!(reference_stats.shared_parse_cache_peak_entries > 0);

            for jobs in [2, 4, 8].map(|jobs| std::num::NonZeroUsize::new(jobs).unwrap()) {
                let (graph, commits, stats) = run_full_walk_into_with_stats(
                    dir.path(),
                    &GitWalkConfig {
                        walk_strategy: strategy,
                        temporal_jobs: jobs,
                        ..GitWalkConfig::default()
                    },
                    None,
                    None,
                )
                .unwrap();

                assert_eq!(normalize_walk_output((graph, commits)), expected);
                assert_eq!(stats.shared_parse_cache_current_entries, 0);
                assert!(stats.shared_parse_cache_peak_entries > 0);
                assert!(stats.pool.max_in_flight <= jobs.get());
                assert!(stats.pool.max_result_occupancy <= jobs.get());
            }
        }
    }

    #[test]
    fn full_walk_parse_cache_peak_is_bounded_by_admitted_active_work() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let commit_count = 24;
        for ordinal in 0..commit_count {
            std::fs::write(
                dir.path().join("lib.rs"),
                format!("pub fn version_{ordinal}() -> usize {{ {ordinal} }}\n"),
            )
            .unwrap();
            commit(dir.path(), &format!("version {ordinal}"));
        }
        let jobs = std::num::NonZeroUsize::new(4).unwrap();

        let (_, commits, stats) = run_full_walk_into_with_stats(
            dir.path(),
            &GitWalkConfig {
                temporal_jobs: jobs,
                ..GitWalkConfig::default()
            },
            None,
            None,
        )
        .unwrap();

        assert_eq!(commits.commits.len(), commit_count);
        assert_eq!(stats.shared_parse_cache_current_entries, 0);
        assert!(stats.shared_parse_cache_peak_entries > 0);
        eprintln!(
            "parse cache telemetry: current={} peak={} jobs={} commits={commit_count}",
            stats.shared_parse_cache_current_entries,
            stats.shared_parse_cache_peak_entries,
            jobs.get(),
        );
        assert!(
            stats.shared_parse_cache_peak_entries <= jobs.get() * 2,
            "parse cache telemetry was not bounded by admitted work: {stats:?}"
        );
    }

    #[test]
    fn gix_open_ignores_replace_refs() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.rs"), b"fn a() -> u8 { 1 }\n").unwrap();
        let a = commit(dir.path(), "A");
        std::fs::write(dir.path().join("a.rs"), b"fn a() -> u8 { 2 }\n").unwrap();
        let b = commit(dir.path(), "B");
        std::fs::write(dir.path().join("a.rs"), b"fn a() -> u8 { 3 }\n").unwrap();
        let c = commit(dir.path(), "C");
        run_git(dir.path(), &["replace", &c, &a]).unwrap();

        let repo = open_gix_repo_without_replace_refs(dir.path()).unwrap();
        assert_eq!(
            repo.config_snapshot().boolean("core.useReplaceRefs"),
            Some(false),
            "opener must expose git-compatible replace-ref config"
        );
        let commit = repo
            .find_commit(gix::ObjectId::from_hex(c.as_bytes()).unwrap())
            .unwrap();
        let parents: Vec<_> = commit
            .parent_ids()
            .map(|parent| parent.to_hex().to_string())
            .collect();

        assert_eq!(parents, vec![b]);
    }

    #[test]
    fn snapshot_refs_returns_main_tip() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let sha = commit(dir.path(), "init");

        let snap = snapshot_refs(dir.path(), &["main"]).unwrap();

        assert_eq!(snap.get("main"), Some(&sha));
    }

    #[test]
    fn snapshot_refs_returns_head_tip_when_detached() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let sha = commit(dir.path(), "init");
        run_git(dir.path(), &["checkout", "--detach", "-q", &sha]).unwrap();

        let snap = snapshot_refs(dir.path(), &["HEAD"]).unwrap();

        assert_eq!(snap.get("HEAD"), Some(&sha));
    }

    #[test]
    fn fail_closed_on_shallow_clone() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let sha = commit(dir.path(), "init");
        std::fs::write(dir.path().join(".git/shallow"), format!("{sha}\n")).unwrap();

        let err = ensure_not_shallow(dir.path()).unwrap_err();

        assert!(
            err.to_string().contains("refusing to index shallow clone"),
            "{err:#}"
        );
    }

    #[test]
    fn fail_closed_on_missing_target_ref() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());

        let err = snapshot_refs(dir.path(), &["main"]).unwrap_err();

        assert!(
            err.to_string().contains("target ref `main` does not exist"),
            "{err:#}"
        );
    }

    #[test]
    fn file_diff_initial_commit_marks_all_added() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.rs"), b"fn a() {}").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"hi").unwrap();
        let sha = commit(dir.path(), "init");

        let changes = file_changes_for_commit(dir.path(), &sha).unwrap();

        let mut paths: Vec<_> = changes.iter().map(|c| (&c.path, &c.kind)).collect();
        paths.sort_by_key(|(p, _)| p.to_string_lossy().to_string());
        assert_eq!(paths.len(), 2);
        assert!(matches!(paths[0].1, FileChangeKind::Added));
    }

    #[test]
    fn file_diff_rename_detected() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("old.rs"), b"fn x() {}").unwrap();
        commit(dir.path(), "init");
        std::fs::rename(dir.path().join("old.rs"), dir.path().join("new.rs")).unwrap();
        let sha = commit(dir.path(), "rename");

        let changes = file_changes_for_commit(dir.path(), &sha).unwrap();

        let r = changes.iter().find(|c| c.path.ends_with("new.rs")).unwrap();
        assert!(matches!(&r.kind, FileChangeKind::Renamed { from } if from.ends_with("old.rs")));
    }

    #[test]
    fn parse_raw_diff_handles_copy_status_like_rename() {
        let mut stdout = Vec::new();
        stdout.extend_from_slice(b":100644 100644 0000000000000000000000000000000000000001 0000000000000000000000000000000000000002 C100");
        stdout.push(0);
        stdout.extend_from_slice(b"src/original.rs");
        stdout.push(0);
        stdout.extend_from_slice(b"src/copied.rs");
        stdout.push(0);

        let mut changes = Vec::new();
        parse_raw_diff(&stdout, Some("deadbeef".into()), &mut changes)
            .expect("copy status must parse");

        assert_eq!(changes.len(), 1);
        let change = &changes[0];
        assert_eq!(change.path.display().to_string(), "src/copied.rs");
        match &change.kind {
            FileChangeKind::Renamed { from } => {
                assert_eq!(from.display().to_string(), "src/original.rs");
            }
            other => panic!("expected Renamed (copy treated as rename), got {other:?}"),
        }
    }

    #[test]
    fn symbol_diff_classifies_added_modified_deleted() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("lib.rs"), b"fn a() {}\nfn b() {}\n").unwrap();
        commit(dir.path(), "c1");
        std::fs::write(dir.path().join("lib.rs"), b"fn a() { 42; }\nfn c() {}\n").unwrap();
        let sha2 = commit(dir.path(), "c2");

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha2).unwrap();
        let changes =
            symbol_changes_for_commit(dir.path(), &sha2, &file_changes, &mut ctx).unwrap();
        let by_name: std::collections::HashMap<_, _> = changes
            .iter()
            .map(|c| (c.snapshot.entity_name.clone(), &c.change_kind))
            .collect();

        assert!(matches!(by_name.get("a"), Some(ChangeKind::Modified)));
        assert!(matches!(by_name.get("c"), Some(ChangeKind::Added)));
        assert!(matches!(by_name.get("b"), Some(ChangeKind::Deleted)));
    }

    #[test]
    fn symbol_diff_marks_byte_start_shift_as_modified() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("lib.rs"), b"fn target() { 1 }\n").unwrap();
        commit(dir.path(), "c1");
        std::fs::write(
            dir.path().join("lib.rs"),
            b"// inserted before target\nfn target() { 1 }\n",
        )
        .unwrap();
        let sha2 = commit(dir.path(), "c2");

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha2).unwrap();
        let changes =
            symbol_changes_for_commit(dir.path(), &sha2, &file_changes, &mut ctx).unwrap();
        let target = changes
            .iter()
            .find(|change| change.snapshot.entity_name == "target")
            .expect("target should be emitted when stable id changes");

        assert!(matches!(target.change_kind, ChangeKind::Modified));
        assert_eq!(target.snapshot.byte_range[0], 26);
    }

    #[test]
    fn tier1_file_rename_inheritance_matches_same_name_kind() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("old.rs"), b"pub fn helper() { 1; 2; 3; }\n").unwrap();
        commit(dir.path(), "c1");
        std::fs::rename(dir.path().join("old.rs"), dir.path().join("new.rs")).unwrap();
        let sha = commit(dir.path(), "rename");

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha).unwrap();
        let changes = symbol_changes_for_commit(dir.path(), &sha, &file_changes, &mut ctx).unwrap();
        let helper = changes
            .iter()
            .find(|c| c.snapshot.entity_name == "helper")
            .unwrap();

        assert!(matches!(&helper.change_kind, ChangeKind::RenamedFrom(_)));
    }

    #[test]
    fn tier2_jaccard_matches_renamed_body_similar() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(
            dir.path().join("lib.rs"),
            b"pub fn old_name(a: u32, b: u32) -> u32 { a + b * 2 }\n",
        )
        .unwrap();
        commit(dir.path(), "c1");
        std::fs::write(
            dir.path().join("lib.rs"),
            b"pub fn new_name(a: u32, b: u32) -> u32 { a + b * 2 }\n",
        )
        .unwrap();
        let sha = commit(dir.path(), "c2");

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha).unwrap();
        let changes = symbol_changes_for_commit(dir.path(), &sha, &file_changes, &mut ctx).unwrap();
        let renamed = changes
            .iter()
            .find(|c| c.snapshot.entity_name == "new_name")
            .unwrap();

        assert!(matches!(&renamed.change_kind, ChangeKind::RenamedFrom(_)));
    }

    #[test]
    fn tier3_ambiguous_falls_back_to_added_deleted() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("lib.rs"), b"pub fn old() { 1 }\n").unwrap();
        commit(dir.path(), "c1");
        std::fs::write(
            dir.path().join("lib.rs"),
            b"pub fn a() { 1 }\npub fn b() { 1 }\n",
        )
        .unwrap();
        let sha = commit(dir.path(), "c2");

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha).unwrap();
        let changes = symbol_changes_for_commit(dir.path(), &sha, &file_changes, &mut ctx).unwrap();
        let kinds: Vec<_> = changes.iter().map(|c| &c.change_kind).collect();

        assert!(kinds.iter().any(|k| matches!(k, ChangeKind::Deleted)));
        assert_eq!(
            kinds
                .iter()
                .filter(|k| matches!(k, ChangeKind::Added))
                .count(),
            2
        );
        assert!(!kinds
            .iter()
            .any(|k| matches!(k, ChangeKind::RenamedFrom(_))));
        assert!(ctx
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.contains("ambiguous_rename")));
    }

    #[test]
    fn merge_collision_emits_added_and_keeps_deleted() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(
            dir.path().join("lib.rs"),
            b"pub fn old_a(x: u32) -> u32 { x + 1 }\npub fn old_b(x: u32) -> u32 { x + 1 }\n",
        )
        .unwrap();
        commit(dir.path(), "c1");
        std::fs::write(
            dir.path().join("lib.rs"),
            b"pub fn merged_target(x: u32) -> u32 { x + 1 }\n",
        )
        .unwrap();
        let sha = commit(dir.path(), "c2");

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha).unwrap();
        let changes = symbol_changes_for_commit(dir.path(), &sha, &file_changes, &mut ctx).unwrap();
        let added: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c.change_kind, ChangeKind::Added))
            .collect();
        let deleted: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c.change_kind, ChangeKind::Deleted))
            .collect();
        let renamed: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c.change_kind, ChangeKind::RenamedFrom(_)))
            .collect();

        assert_eq!(added.len(), 1, "merged_target should be Added");
        assert_eq!(deleted.len(), 2, "both olds should remain Deleted");
        assert!(
            renamed.is_empty(),
            "no RenamedFrom may be emitted in merge collision"
        );
        assert!(ctx
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.contains("merge_collision")));
    }

    #[test]
    fn tier2_jaccard_matches_preserves_matches_and_ambiguity_diagnostics() {
        let file_change = FileChange {
            path: GitPath::from("lib.rs"),
            kind: FileChangeKind::Modified,
            parent_sha: Some("parent".into()),
        };
        let deleted = vec![
            symbol_change("deleted_clean", "old_clean", &["a", "b", "c", "d"]),
            symbol_change("deleted_tie_a", "old_tie_a", &["t1", "t2", "t3", "t4"]),
            symbol_change("deleted_tie_b", "old_tie_b", &["t1", "t2", "t3", "t4"]),
            symbol_change("deleted_low", "old_low", &["x", "y", "z"]),
        ];
        let added = vec![
            symbol_change("added_clean", "new_clean", &["a", "b", "c", "d", "e"]),
            symbol_change("added_tie", "new_tie", &["t1", "t2", "t3", "t4"]),
            symbol_change("added_low", "new_low", &["x", "unrelated"]),
        ];
        let mut diagnostics = Vec::new();

        let matches = tier2_jaccard_matches(
            &deleted,
            &added,
            &file_change,
            Language::Rust,
            &mut diagnostics,
        );

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].from.snapshot.key.stable_symbol_id,
            "deleted_clean"
        );
        assert_eq!(matches[0].to.snapshot.key.stable_symbol_id, "added_clean");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("merge_collision") && diagnostic.contains("candidate=new_tie")
        }));
        assert!(has_ambiguous_rename_diagnostic(
            &diagnostics,
            "added_tie",
            "deleted_tie_a"
        ));
        assert!(has_ambiguous_rename_diagnostic(
            &diagnostics,
            "added_tie",
            "deleted_tie_b"
        ));
        assert!(has_ambiguous_rename_diagnostic(
            &diagnostics,
            "added_low",
            "deleted_low"
        ));
    }

    fn symbol_change(stable_symbol_id: &str, entity_name: &str, tokens: &[&str]) -> SymbolChange {
        SymbolChange {
            snapshot: SymbolSnapshotArtifact {
                key: SnapshotKey {
                    stable_symbol_id: stable_symbol_id.into(),
                    commit: "commit".into(),
                },
                file_path: "lib.rs".into(),
                entity_name: entity_name.into(),
                symbol_kind: "function".into(),
                enclosing_scope: None,
                byte_range: [0, 1],
                line_range: [1, 1],
                anchor_hash: format!("hash-{stable_symbol_id}"),
                tokens: tokens.iter().map(|token| (*token).into()).collect(),
            },
            change_kind: ChangeKind::Added,
            parent_sha: Some("parent".into()),
        }
    }

    fn has_ambiguous_rename_diagnostic(
        diagnostics: &[String],
        stable_id: &str,
        other_stable_id: &str,
    ) -> bool {
        diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("ambiguous_rename")
                && diagnostic.contains(&format!("stable_symbol_id={stable_id}"))
                && diagnostic.contains(&format!("other_stable_symbol_id={other_stable_id}"))
        })
    }

    #[test]
    fn force_push_invalidates_and_rewalks_diverged_range() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("a.rs"), b"fn a() {}\n").unwrap();
        let sha1 = commit(dir.path(), "c1");
        std::fs::write(dir.path().join("a.rs"), b"fn a() { 1 }\n").unwrap();
        let sha2 = commit(dir.path(), "c2");

        run_git(dir.path(), &["reset", "--hard", &sha1]).unwrap();
        std::fs::write(dir.path().join("a.rs"), b"fn a() { 999 }\n").unwrap();
        let sha2b = commit(dir.path(), "c2b");

        let plan = plan_incremental_walk(dir.path(), Some(&sha1), &sha2b).unwrap();
        assert!(matches!(
            plan,
            IncrementalPlan::FastForward { from, to } if from == sha1 && to == sha2b
        ));

        let plan = plan_incremental_walk(dir.path(), Some(&sha2), &sha2b).unwrap();
        assert!(matches!(
            plan,
            IncrementalPlan::ForcePushRecover { merge_base: Some(base), to }
                if base == sha1 && to == sha2b
        ));
    }

    #[test]
    fn gitlink_emits_file_change_no_recurse() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        commit(dir.path(), "c1");
        let oid = "0123456789012345678901234567890123456789";
        run_git(
            dir.path(),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000",
                oid,
                "vendor/submodule",
            ],
        )
        .unwrap();
        run_git(dir.path(), &["commit", "-q", "-m", "add gitlink"]).unwrap();
        let sha = run_git(dir.path(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned();

        let changes = file_changes_for_commit(dir.path(), &sha).unwrap();

        let gitlink = changes
            .iter()
            .find(|change| change.path.ends_with("vendor/submodule"))
            .expect("gitlink change");
        assert!(matches!(
            &gitlink.kind,
            FileChangeKind::Gitlink {
                old_oid: None,
                new_oid: Some(new_oid),
            } if new_oid == oid
        ));
    }

    #[test]
    fn binary_blob_downgrades_to_file_level_and_logs() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("lib.rs"), b"\0not rust\n").unwrap();
        let sha = commit(dir.path(), "binary rust extension");

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha).unwrap();
        let changes = symbol_changes_for_commit(dir.path(), &sha, &file_changes, &mut ctx).unwrap();

        assert!(changes.is_empty());
        assert!(ctx
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.contains("binary_blob")));
    }

    #[test]
    fn parse_cache_dedups_repeated_blob_oids() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("lib.rs"), b"pub fn repeated_blob() {}\n").unwrap();
        let sha1 = commit(dir.path(), "add rust file");
        run_git(dir.path(), &["update-index", "--chmod=+x", "lib.rs"]).unwrap();
        run_git(dir.path(), &["commit", "-q", "-m", "mode only"]).unwrap();
        let sha2 = run_git(dir.path(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned();

        let mut ctx = SymbolDiffCtx::new();
        let file_changes1 = file_changes_for_commit(dir.path(), &sha1).unwrap();
        symbol_changes_for_commit(dir.path(), &sha1, &file_changes1, &mut ctx).unwrap();
        let file_changes2 = file_changes_for_commit(dir.path(), &sha2).unwrap();

        assert!(file_changes2
            .iter()
            .any(|change| matches!(change.kind, FileChangeKind::Modified)));
        symbol_changes_for_commit(dir.path(), &sha2, &file_changes2, &mut ctx).unwrap();

        assert_eq!(ctx.parse_cache_len(), 1);
    }

    #[test]
    fn cache_telemetry_distinguishes_cold_hit_and_post_eviction_reparse() {
        let cache = SharedParseCache::with_telemetry();
        let key = (Language::Rust, "reused".to_owned());

        cache
            .get_or_init_at(1, key.clone(), || Ok(Ok(vec![cache_test_symbol("first")])))
            .unwrap()
            .unwrap();
        cache
            .get_or_init_at(2, key.clone(), || unreachable!("cache hit"))
            .unwrap()
            .unwrap();
        cache.evict_before(3);
        cache
            .get_or_init_at(7, key, || Ok(Ok(vec![cache_test_symbol("second")])))
            .unwrap()
            .unwrap();

        let stats = cache.telemetry_snapshot();
        assert_eq!(stats.cold_initializations, 1);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.reparse_initializations, 1);
        assert_eq!(stats.successful_initializations, 2);
        assert!(stats.evicted_entries >= 1);
        assert!(stats.cache_hit_nanos > 0);
        assert!(stats.cold_initialization_nanos > 0);
        assert!(stats.reparse_initialization_nanos > 0);
        assert_eq!(
            stats.initialization_nanos,
            stats
                .cold_initialization_nanos
                .saturating_add(stats.reparse_initialization_nanos)
        );
        assert_eq!(stats.reuse_distance_count, 1);
        assert_eq!(stats.reuse_distance_sum, 4);
        assert_eq!(stats.reuse_distance_max, 4);
    }

    #[test]
    fn cache_telemetry_tracks_payload_bytes_and_eviction() {
        let cache = SharedParseCache::with_telemetry();
        let key = (Language::Rust, "payload".to_owned());

        cache
            .get_or_init_at(1, key, || {
                Ok(Ok(vec![cache_test_symbol("retained-payload")]))
            })
            .unwrap()
            .unwrap();

        let retained = cache.telemetry_snapshot();
        assert!(retained.telemetry_enabled);
        assert!(retained.initialization_nanos > 0);
        assert!(retained.current_retained_payload_bytes > 0);
        assert!(retained.peak_retained_payload_bytes >= retained.current_retained_payload_bytes);

        cache.evict_before(2);

        let evicted = cache.telemetry_snapshot();
        assert_eq!(evicted.current_retained_payload_bytes, 0);
        assert!(evicted.peak_retained_payload_bytes > 0);
        assert_eq!(evicted.evicted_entries, 1);
        assert_eq!(evicted.current_exact_ghost_keys, 1);
        assert_eq!(evicted.peak_exact_ghost_keys, 1);
    }

    #[test]
    fn cache_telemetry_counts_failures_without_retaining_payload() {
        let cache = SharedParseCache::with_telemetry();
        let key = (Language::Rust, "invalid-payload".to_owned());

        let failed = cache
            .get_or_init_at(1, key.clone(), || Ok(Err(ExtractError::NoTree)))
            .unwrap();

        assert!(matches!(failed, Err(ExtractError::NoTree)));
        let after_failure = cache.telemetry_snapshot();
        assert_eq!(after_failure.failed_initializations, 1);
        assert_eq!(after_failure.successful_initializations, 0);
        assert_eq!(after_failure.current_retained_payload_bytes, 0);
        assert_eq!(cache.len(), 0);

        cache
            .get_or_init_at(2, key, || Ok(Ok(vec![cache_test_symbol("retried")])))
            .unwrap()
            .unwrap();

        let after_retry = cache.telemetry_snapshot();
        assert_eq!(after_retry.cold_initializations, 2);
        assert_eq!(after_retry.failed_initializations, 1);
        assert_eq!(after_retry.successful_initializations, 1);
        assert!(after_retry.current_retained_payload_bytes > 0);
    }

    #[test]
    fn cache_telemetry_records_same_key_lock_wait_and_initialization_time() {
        use std::sync::Barrier;

        let cache = Arc::new(SharedParseCache::with_telemetry());
        let ready = Arc::new(Barrier::new(2));
        let initializations = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..2)
            .map(|ordinal| {
                let cache = Arc::clone(&cache);
                let ready = Arc::clone(&ready);
                let initializations = Arc::clone(&initializations);
                std::thread::spawn(move || {
                    ready.wait();
                    cache
                        .get_or_init_at(ordinal, (Language::Rust, "contended".to_owned()), || {
                            initializations.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            Ok(Ok(vec![cache_test_symbol("contended")]))
                        })
                        .unwrap()
                        .unwrap()
                })
            })
            .collect();
        let symbols: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();

        assert_eq!(initializations.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&symbols[0], &symbols[1]));
        let stats = cache.telemetry_snapshot();
        assert_eq!(stats.cold_initializations, 1);
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.successful_initializations, 1);
        assert_eq!(stats.failed_initializations, 0);
        assert!(stats.initialization_nanos >= 20_000_000);
        assert_eq!(stats.initialization_nanos, stats.cold_initialization_nanos);
        assert_eq!(stats.reparse_initialization_nanos, 0);
        assert!(stats.cache_hit_nanos > 0);
        assert!(stats.lock_wait_nanos > 0);
    }

    #[test]
    fn cache_telemetry_is_disabled_by_default_and_retains_no_ghost_keys() {
        let cache = SharedParseCache::default();
        let key = (Language::Rust, "disabled".to_owned());

        cache
            .get_or_init_at(1, key, || Ok(Ok(vec![cache_test_symbol("disabled")])))
            .unwrap()
            .unwrap();
        cache.evict_before(2);

        let stats = cache.telemetry_snapshot();
        assert!(!stats.telemetry_enabled);
        assert_eq!(stats.current_exact_ghost_keys, 0);
        assert_eq!(stats.peak_exact_ghost_keys, 0);
        assert_eq!(stats.cold_initializations, 0);
        assert_eq!(stats.evicted_entries, 0);
    }

    #[test]
    fn retained_tier_reuses_an_eligible_entry_without_reinitializing() {
        let cache = SharedParseCache::with_budget_and_telemetry(1 << 20);
        let key = (Language::Rust, "retained".to_owned());
        let first = cache
            .get_or_init_at(1, key.clone(), || {
                Ok(Ok(vec![cache_test_symbol("retained")]))
            })
            .unwrap()
            .unwrap();

        cache.evict_before(2);

        let second = cache
            .get_or_init_at(8, key, || panic!("retained hit must not reparse"))
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.telemetry_snapshot().retained_tier_hits, 1);
    }

    #[test]
    fn retained_tier_zero_budget_evicts_successful_empty_entries() {
        let cache = SharedParseCache::with_budget(0);
        let key = (Language::Rust, "empty".to_owned());
        let symbols = cache
            .get_or_init_at(1, key.clone(), || Ok(Ok(Vec::new())))
            .unwrap()
            .unwrap();

        assert!(symbols.is_empty());
        assert_eq!(retained_payload_estimate(&symbols), 0);
        assert!(cache.contains(&key));

        cache.evict_before(2);

        assert!(!cache.contains(&key));
    }

    #[test]
    fn retained_tier_evicts_oversized_entries_but_keeps_unreduced_entries() {
        let symbol = cache_test_symbol("oversized");
        let budget = retained_payload_estimate(std::slice::from_ref(&symbol)) - 1;
        let cache = SharedParseCache::with_budget(budget);
        let old_key = (Language::Rust, "old".to_owned());
        let live_key = (Language::Rust, "live".to_owned());
        let standalone_key = (Language::Rust, "standalone".to_owned());
        cache
            .get_or_init_at(1, old_key.clone(), || Ok(Ok(vec![symbol])))
            .unwrap()
            .unwrap();
        cache
            .get_or_init_at(9, live_key.clone(), || {
                Ok(Ok(vec![cache_test_symbol("live")]))
            })
            .unwrap()
            .unwrap();
        cache
            .get_or_init(standalone_key.clone(), || {
                Ok(Ok(vec![cache_test_symbol("standalone")]))
            })
            .unwrap()
            .unwrap();

        cache.evict_before(2);

        assert!(!cache.contains(&old_key));
        assert!(cache.contains(&live_key));
        assert!(cache.contains(&standalone_key));
        let stats = cache.telemetry_snapshot();
        assert_eq!(stats.retained_budget_bytes, budget);
        assert_eq!(stats.current_retained_tier_payload_bytes, 0);
    }

    #[test]
    fn retained_tier_ties_are_independent_of_hashmap_and_insertion_order() {
        let entry_bytes = retained_payload_estimate(&[cache_test_symbol("equal")]);
        let budget = entry_bytes.saturating_mul(2);
        let orders = [
            [
                (Language::Rust, "a"),
                (Language::Python, "a"),
                (Language::Rust, "z"),
            ],
            [
                (Language::Rust, "z"),
                (Language::Python, "a"),
                (Language::Rust, "a"),
            ],
        ];

        for order in orders {
            let cache = SharedParseCache::with_budget_and_telemetry(budget);
            for (language, blob_oid) in order {
                cache
                    .get_or_init_at(1, (language, blob_oid.to_owned()), || {
                        Ok(Ok(vec![cache_test_symbol("equal")]))
                    })
                    .unwrap()
                    .unwrap();
            }

            cache.evict_before(2);

            assert!(!cache.contains(&(Language::Rust, "a".to_owned())));
            assert!(cache.contains(&(Language::Python, "a".to_owned())));
            assert!(cache.contains(&(Language::Rust, "z".to_owned())));
            let stats = cache.telemetry_snapshot();
            assert_eq!(stats.budget_evictions, 1);
            assert_eq!(stats.current_retained_tier_payload_bytes, budget);
        }
    }

    #[test]
    fn retained_tier_tracks_exact_bytes_with_telemetry_off_and_on() {
        let expected = retained_payload_estimate(&[cache_test_symbol("accounted")]);

        for telemetry_enabled in [false, true] {
            let cache = if telemetry_enabled {
                SharedParseCache::with_budget_and_telemetry(expected)
            } else {
                SharedParseCache::with_budget(expected)
            };
            let key = (Language::Rust, "accounted".to_owned());
            let first = cache
                .get_or_init_at(1, key.clone(), || {
                    Ok(Ok(vec![cache_test_symbol("accounted")]))
                })
                .unwrap()
                .unwrap();
            let initialized = cache.telemetry_snapshot();
            assert_eq!(initialized.retained_budget_bytes, expected);
            assert_eq!(initialized.current_retained_payload_bytes, expected);
            assert_eq!(initialized.current_retained_tier_payload_bytes, 0);

            cache.evict_before(2);

            let retained = cache.telemetry_snapshot();
            assert_eq!(retained.current_retained_payload_bytes, expected);
            assert_eq!(retained.current_retained_tier_payload_bytes, expected);
            assert!(retained.peak_retained_tier_payload_bytes >= expected);

            let second = cache
                .get_or_init_at(8, key, || panic!("retained hit must not reparse"))
                .unwrap()
                .unwrap();
            assert!(Arc::ptr_eq(&first, &second));
            let promoted = cache.telemetry_snapshot();
            assert_eq!(promoted.current_retained_payload_bytes, expected);
            assert_eq!(promoted.current_retained_tier_payload_bytes, 0);
        }
    }

    #[test]
    fn retained_tier_failed_initialization_releases_every_accounted_byte() {
        let cache = SharedParseCache::with_budget_and_telemetry(1 << 20);
        let key = (Language::Rust, "failed".to_owned());

        let failed = cache
            .get_or_init_at(1, key.clone(), || Ok(Err(ExtractError::NoTree)))
            .unwrap();

        assert!(matches!(failed, Err(ExtractError::NoTree)));
        cache.evict_before(2);
        assert!(!cache.contains(&key));
        let stats = cache.telemetry_snapshot();
        assert_eq!(stats.current_retained_payload_bytes, 0);
        assert_eq!(stats.current_retained_tier_payload_bytes, 0);
        assert_eq!(stats.budget_evictions, 0);
    }

    #[test]
    fn retained_tier_same_key_initialization_remains_single_flight() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        let cache = Arc::new(SharedParseCache::with_budget(1 << 20));
        let ready = Arc::new(Barrier::new(8));
        let initializations = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..8)
            .map(|ordinal| {
                let cache = Arc::clone(&cache);
                let ready = Arc::clone(&ready);
                let initializations = Arc::clone(&initializations);
                std::thread::spawn(move || {
                    ready.wait();
                    cache
                        .get_or_init_at(ordinal, (Language::Rust, "same-blob".to_owned()), || {
                            initializations.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            Ok(Ok(vec![cache_test_symbol("shared")]))
                        })
                        .unwrap()
                        .unwrap()
                })
            })
            .collect();
        let symbols: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();

        assert_eq!(initializations.load(Ordering::SeqCst), 1);
        assert_eq!(cache.len(), 1);
        let first = &symbols[0];
        assert!(symbols
            .iter()
            .skip(1)
            .all(|symbols| Arc::ptr_eq(first, symbols)));
    }

    #[test]
    fn retained_tier_initializer_can_reenter_cache_state_without_deadlock() {
        let cache = SharedParseCache::with_budget(1 << 20);
        let key = (Language::Rust, "reentrant".to_owned());

        cache
            .get_or_init_at(1, key.clone(), || {
                assert!(cache.contains(&key));
                assert_eq!(cache.len(), 1);
                Ok(Ok(vec![cache_test_symbol("reentrant")]))
            })
            .unwrap()
            .unwrap();
    }

    #[test]
    fn parse_cache_budget_accepts_zero_and_u64_values_and_rejects_invalid_input() {
        assert_eq!(parse_temporal_parse_cache_budget_bytes(None).unwrap(), 0);
        assert_eq!(
            parse_temporal_parse_cache_budget_bytes(Some(OsString::from("0"))).unwrap(),
            0
        );
        assert_eq!(
            parse_temporal_parse_cache_budget_bytes(Some(OsString::from(u64::MAX.to_string())))
                .unwrap(),
            u64::MAX
        );

        for invalid in ["", "-1", "+1", "1MiB", "18446744073709551616"] {
            let error =
                parse_temporal_parse_cache_budget_bytes(Some(OsString::from(invalid))).unwrap_err();
            assert!(
                error.to_string().contains(TEMPORAL_PARSE_CACHE_BUDGET_ENV),
                "{error:#}"
            );
        }

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt as _;

            let error =
                parse_temporal_parse_cache_budget_bytes(Some(OsString::from_vec(vec![0xff])))
                    .unwrap_err();
            assert!(error.to_string().contains("valid Unicode"), "{error:#}");
        }
    }

    #[test]
    fn shared_parse_cache_evicts_only_fully_reduced_ordinals() {
        let cache = SharedParseCache::default();
        let old_key = (Language::Rust, "old".to_owned());
        let active_key = (Language::Rust, "active".to_owned());
        cache
            .get_or_init_at(2, old_key.clone(), || {
                Ok(Ok(vec![cache_test_symbol("old")]))
            })
            .unwrap()
            .unwrap();
        cache
            .get_or_init_at(5, active_key.clone(), || {
                Ok(Ok(vec![cache_test_symbol("active")]))
            })
            .unwrap()
            .unwrap();

        cache.evict_before(4);

        assert!(!cache.contains(&old_key));
        assert!(cache.contains(&active_key));
    }

    #[test]
    fn shared_parse_cache_keeps_unreduced_concurrent_access() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let cache = Arc::new(SharedParseCache::default());
        let initializations = Arc::new(AtomicUsize::new(0));
        let key = (Language::Rust, "same-blob".to_owned());
        let (initialization_started_tx, initialization_started_rx) =
            std::sync::mpsc::sync_channel(0);
        let (release_initialization_tx, release_initialization_rx) =
            std::sync::mpsc::sync_channel(0);
        let old_cache = Arc::clone(&cache);
        let old_key = key.clone();
        let old_initializations = Arc::clone(&initializations);
        let old = std::thread::spawn(move || {
            old_cache
                .get_or_init_at(2, old_key, || {
                    old_initializations.fetch_add(1, Ordering::SeqCst);
                    initialization_started_tx.send(()).unwrap();
                    release_initialization_rx.recv().unwrap();
                    Ok(Ok(vec![cache_test_symbol("shared")]))
                })
                .unwrap()
                .unwrap()
        });
        initialization_started_rx.recv().unwrap();

        let active_cache = Arc::clone(&cache);
        let active_key = key.clone();
        let active_initializations = Arc::clone(&initializations);
        let active = std::thread::spawn(move || {
            active_cache
                .get_or_init_at(5, active_key, || {
                    active_initializations.fetch_add(1, Ordering::SeqCst);
                    Ok(Ok(vec![cache_test_symbol("unexpected")]))
                })
                .unwrap()
                .unwrap()
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let recorded_before_eviction = loop {
            if cache.greatest_using_ordinal(&key) == Some(Some(5)) {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::yield_now();
        };

        cache.evict_before(4);
        let retained_before_release = cache.contains(&key);
        release_initialization_tx.send(()).unwrap();
        let old_symbols = old.join().unwrap();
        let active_symbols = active.join().unwrap();

        assert!(recorded_before_eviction);
        assert!(retained_before_release);
        assert_eq!(initializations.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&old_symbols, &active_symbols));
        assert!(cache.contains(&key));
    }

    #[test]
    fn shared_parse_cache_separates_languages_for_same_blob_oid() {
        let cache = SharedParseCache::default();

        let rust = cache
            .get_or_init((Language::Rust, "same-blob".to_owned()), || {
                Ok(Ok(vec![cache_test_symbol("rust")]))
            })
            .unwrap()
            .unwrap();
        let python = cache
            .get_or_init((Language::Python, "same-blob".to_owned()), || {
                Ok(Ok(vec![cache_test_symbol("python")]))
            })
            .unwrap()
            .unwrap();

        assert_eq!(cache.len(), 2);
        assert_eq!(rust[0].entity_name, "rust");
        assert_eq!(python[0].entity_name, "python");
    }

    #[test]
    fn shared_parse_cache_does_not_retain_parse_failures() {
        let cache = SharedParseCache::default();
        let key = (Language::Rust, "invalid-blob".to_owned());

        let failed = cache
            .get_or_init(key.clone(), || Ok(Err(ExtractError::NoTree)))
            .unwrap();

        assert!(matches!(failed, Err(ExtractError::NoTree)));
        assert_eq!(cache.len(), 0);

        let retried = cache
            .get_or_init(key, || Ok(Ok(vec![cache_test_symbol("retried")])))
            .unwrap()
            .unwrap();

        assert_eq!(cache.len(), 1);
        assert_eq!(retried[0].entity_name, "retried");
    }

    fn cache_test_symbol(entity_name: &str) -> ExtractedSymbol {
        ExtractedSymbol {
            entity_name: entity_name.to_owned(),
            symbol_kind: "function".to_owned(),
            enclosing_scope: None,
            byte_range: [0, 1],
            line_range: [1, 1],
            anchor_hash: format!("hash-{entity_name}"),
            tokens: vec![entity_name.to_owned()],
        }
    }

    #[test]
    fn cat_file_batch_reads_multiple_blobs_and_reports_missing() {
        let dir = TempDir::new().unwrap();
        let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sample_corpus");
        for path in ["src/lib.rs", "src/utils.rs", "expected_graph_index.json"] {
            let source = fixture_root.join(path);
            let destination = dir.path().join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(source, destination).unwrap();
        }
        init_repo(dir.path());
        let sha = commit(dir.path(), "fixture");

        let valid_paths = [
            std::path::Path::new("src/lib.rs"),
            std::path::Path::new("src/utils.rs"),
            std::path::Path::new("expected_graph_index.json"),
        ];
        let mut batch = CatFileBatch::new(dir.path()).unwrap();
        for path in valid_paths {
            let expected = std::fs::read(dir.path().join(path)).unwrap();
            let actual = batch.read(&sha, path).unwrap().map(|(_oid, bytes)| bytes);

            assert_eq!(actual, Some(expected));
        }

        let missing = batch
            .read(&sha, std::path::Path::new("src/missing.rs"))
            .unwrap();

        assert_eq!(missing, None);
    }

    #[test]
    fn missing_blob_fails_closed_with_named_oid() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("lib.rs"), b"fn missing_blob() {}\n").unwrap();
        let sha = commit(dir.path(), "blob to remove");
        let blob_oid = run_git(dir.path(), &["rev-parse", &format!("{sha}:lib.rs")])
            .unwrap()
            .trim()
            .to_owned();
        let object_path = git_dir(dir.path())
            .unwrap()
            .join("objects")
            .join(&blob_oid[..2])
            .join(&blob_oid[2..]);
        std::fs::remove_file(object_path).unwrap();

        let mut ctx = SymbolDiffCtx::new();
        let file_changes = file_changes_for_commit(dir.path(), &sha).unwrap();
        let error =
            symbol_changes_for_commit(dir.path(), &sha, &file_changes, &mut ctx).unwrap_err();
        let message = format!("{error:#}");

        assert!(message.contains("missing blob"));
        assert!(message.contains(&blob_oid), "{message}");
    }

    #[test]
    #[cfg(unix)]
    fn non_utf8_path_does_not_panic() {
        use std::io::Write as _;
        use std::os::unix::ffi::OsStrExt as _;

        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"bad-\xff.rs"));
        let mut blob = Command::new("git")
            .current_dir(dir.path())
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        blob.stdin
            .as_mut()
            .unwrap()
            .write_all(b"fn non_utf8_path() {}\n")
            .unwrap();
        let blob_output = blob.wait_with_output().unwrap();
        assert!(blob_output.status.success());
        let blob_oid = String::from_utf8(blob_output.stdout)
            .unwrap()
            .trim()
            .to_owned();

        let mut tree_entry = Vec::new();
        tree_entry.extend_from_slice(b"100644 blob ");
        tree_entry.extend_from_slice(blob_oid.as_bytes());
        tree_entry.push(b'\t');
        tree_entry.extend_from_slice(path.as_os_str().as_bytes());
        tree_entry.push(0);
        let mut tree = Command::new("git")
            .current_dir(dir.path())
            .args(["mktree", "-z"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        tree.stdin.as_mut().unwrap().write_all(&tree_entry).unwrap();
        let tree_output = tree.wait_with_output().unwrap();
        assert!(tree_output.status.success());
        let tree_oid = String::from_utf8(tree_output.stdout)
            .unwrap()
            .trim()
            .to_owned();
        let sha = run_git(
            dir.path(),
            &["commit-tree", &tree_oid, "-m", "non-utf8 path"],
        )
        .unwrap()
        .trim()
        .to_owned();

        let changes = file_changes_for_commit(dir.path(), &sha).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path.as_bytes(), path.as_os_str().as_bytes());

        let mut ctx = SymbolDiffCtx::new();
        let symbol_changes =
            symbol_changes_for_commit(dir.path(), &sha, &changes, &mut ctx).unwrap();
        assert!(symbol_changes
            .iter()
            .any(|change| change.snapshot.entity_name == "non_utf8_path"));
    }
}
