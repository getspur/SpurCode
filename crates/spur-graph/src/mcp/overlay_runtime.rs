use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use anyhow::{anyhow, bail};
use arc_swap::ArcSwap;
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio::task::JoinHandle;

use crate::overlay_watch::{
    ChangeBatch, ChangeProviderKind, ChangeSourceSet, ChangeSubscription, CompositeCursor,
    SubscriptionFactory, TrustLoss,
};
use crate::{OverlayGeneration, OverlayGenerationIdentity};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct OverlayRuntimeKey {
    canonical_worktree: PathBuf,
    base_graph_identity: String,
}

impl OverlayRuntimeKey {
    pub(super) fn new(canonical_worktree: PathBuf, base_graph_identity: String) -> Self {
        Self {
            canonical_worktree,
            base_graph_identity,
        }
    }

    pub(super) fn canonical_worktree(&self) -> &Path {
        &self.canonical_worktree
    }

    pub(super) fn base_graph_identity(&self) -> &str {
        &self.base_graph_identity
    }
}

#[derive(Clone)]
pub(super) struct BuiltOverlayGeneration {
    snapshot_identity: OverlayGenerationIdentity,
    generation: Arc<OverlayGeneration>,
}

impl BuiltOverlayGeneration {
    pub(super) fn new(
        key: &OverlayRuntimeKey,
        snapshot_identity: OverlayGenerationIdentity,
        generation: Arc<OverlayGeneration>,
    ) -> anyhow::Result<Self> {
        if snapshot_identity.canonical_worktree != key.canonical_worktree {
            bail!(
                "overlay runtime worktree identity mismatch: snapshot={}, registry={}",
                snapshot_identity.canonical_worktree.display(),
                key.canonical_worktree.display()
            );
        }
        if snapshot_identity.indexed_graph_content_hash != key.base_graph_identity {
            bail!(
                "overlay runtime snapshot base identity mismatch: snapshot={}, registry={}",
                snapshot_identity.indexed_graph_content_hash,
                key.base_graph_identity
            );
        }
        if generation.base_artifact().graph_content_hash != key.base_graph_identity {
            bail!(
                "overlay runtime generation base identity mismatch: generation={}, registry={}",
                generation.base_artifact().graph_content_hash,
                key.base_graph_identity
            );
        }
        if let Some(generation_identity) = generation.identity() {
            if generation_identity != &snapshot_identity {
                bail!("overlay runtime generation and snapshot identities disagree");
            }
        }
        Ok(Self {
            snapshot_identity,
            generation,
        })
    }

    fn from_published(state: &PublishedState) -> Self {
        Self {
            snapshot_identity: state.snapshot_identity.clone(),
            generation: Arc::clone(&state.generation),
        }
    }
}

impl fmt::Debug for BuiltOverlayGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltOverlayGeneration")
            .field("snapshot_identity", &self.snapshot_identity)
            .field(
                "base_graph_identity",
                &self.generation.base_artifact().graph_content_hash,
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PublishedUntrustedReason {
    Provider(TrustLoss),
    GitMetadata(BTreeSet<PathBuf>),
    EmptyChangeBatch,
    BuildFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PublishedTrust {
    Trusted,
    Rebuilding,
    Untrusted(PublishedUntrustedReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeStartupCause {
    SourceResolution,
    SubscriptionConstruction,
    SubscriptionArm,
    ProviderUnavailable,
    ExactSeed,
    ReplayTrustLost,
    ReplayRebuild,
}

impl RuntimeStartupCause {
    pub(super) fn diagnostic(self) -> &'static str {
        match self {
            Self::SourceResolution => "source_resolution",
            Self::SubscriptionConstruction => "subscription_construction",
            Self::SubscriptionArm => "subscription_arm",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ExactSeed => "exact_seed",
            Self::ReplayTrustLost => "replay_trust_lost",
            Self::ReplayRebuild => "replay_rebuild",
        }
    }
}

#[derive(Debug)]
pub(super) struct RuntimeStartupError {
    cause: RuntimeStartupCause,
    source: anyhow::Error,
}

impl RuntimeStartupError {
    pub(super) fn new(cause: RuntimeStartupCause, source: impl Into<anyhow::Error>) -> Self {
        Self {
            cause,
            source: source.into(),
        }
    }

    pub(super) fn cause(&self) -> RuntimeStartupCause {
        self.cause
    }
}

impl fmt::Display for RuntimeStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:#}", self.source)
    }
}

impl std::error::Error for RuntimeStartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(super) struct PublishedState {
    key: OverlayRuntimeKey,
    provider: ChangeProviderKind,
    armed_cursor: CompositeCursor,
    current_cursor: CompositeCursor,
    epoch: u64,
    trust: PublishedTrust,
    snapshot_identity: OverlayGenerationIdentity,
    generation: Arc<OverlayGeneration>,
}

impl PublishedState {
    fn initial(
        key: OverlayRuntimeKey,
        provider: ChangeProviderKind,
        armed_cursor: CompositeCursor,
        current_cursor: CompositeCursor,
        built: BuiltOverlayGeneration,
    ) -> Self {
        Self {
            key,
            provider,
            armed_cursor,
            current_cursor,
            epoch: 0,
            trust: PublishedTrust::Trusted,
            snapshot_identity: built.snapshot_identity,
            generation: built.generation,
        }
    }

    fn retaining_generation(
        previous: &Self,
        provider: ChangeProviderKind,
        armed_cursor: CompositeCursor,
        current_cursor: CompositeCursor,
        epoch: u64,
        trust: PublishedTrust,
    ) -> Self {
        Self {
            key: previous.key.clone(),
            provider,
            armed_cursor,
            current_cursor,
            epoch,
            trust,
            snapshot_identity: previous.snapshot_identity.clone(),
            generation: Arc::clone(&previous.generation),
        }
    }

    fn replacing_generation(
        previous: &Self,
        provider: ChangeProviderKind,
        armed_cursor: CompositeCursor,
        current_cursor: CompositeCursor,
        epoch: u64,
        built: BuiltOverlayGeneration,
    ) -> Self {
        Self {
            key: previous.key.clone(),
            provider,
            armed_cursor,
            current_cursor,
            epoch,
            trust: PublishedTrust::Trusted,
            snapshot_identity: built.snapshot_identity,
            generation: built.generation,
        }
    }

    pub(super) fn key(&self) -> &OverlayRuntimeKey {
        &self.key
    }

    pub(super) fn provider(&self) -> ChangeProviderKind {
        self.provider
    }

    pub(super) fn armed_cursor(&self) -> &CompositeCursor {
        &self.armed_cursor
    }

    pub(super) fn current_cursor(&self) -> &CompositeCursor {
        &self.current_cursor
    }

    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(super) fn trust(&self) -> &PublishedTrust {
        &self.trust
    }

    pub(super) fn snapshot_identity(&self) -> &OverlayGenerationIdentity {
        &self.snapshot_identity
    }

    pub(super) fn generation(&self) -> &Arc<OverlayGeneration> {
        &self.generation
    }
}

impl fmt::Debug for PublishedState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublishedState")
            .field("key", &self.key)
            .field("provider", &self.provider)
            .field("armed_cursor", &self.armed_cursor)
            .field("current_cursor", &self.current_cursor)
            .field("epoch", &self.epoch)
            .field("trust", &self.trust)
            .field("snapshot_identity", &self.snapshot_identity)
            .finish_non_exhaustive()
    }
}

#[async_trait]
pub(super) trait OverlayGenerationBuilder: Send + Sync {
    async fn exact_scan(&self, key: &OverlayRuntimeKey) -> anyhow::Result<BuiltOverlayGeneration>;

    async fn rebuild_incremental(
        &self,
        key: &OverlayRuntimeKey,
        previous: BuiltOverlayGeneration,
        changed_paths: BTreeSet<PathBuf>,
    ) -> anyhow::Result<BuiltOverlayGeneration>;
}

#[async_trait]
pub(super) trait RuntimeChangeStream: Send {
    fn provider(&self) -> ChangeProviderKind;
    fn initial_cursor(&self) -> &CompositeCursor;
    async fn next_batch(&mut self) -> Option<ChangeBatch>;
}

#[async_trait]
pub(super) trait RuntimeSubscriptionFactory: Send + Sync {
    async fn arm(&self, key: &OverlayRuntimeKey) -> anyhow::Result<Box<dyn RuntimeChangeStream>>;
}

pub(super) struct CompositeSubscriptionFactory {
    sources: ChangeSourceSet,
    factory: SubscriptionFactory,
}

impl CompositeSubscriptionFactory {
    pub(super) fn new(key: &OverlayRuntimeKey, sources: ChangeSourceSet) -> anyhow::Result<Self> {
        if sources.worktree() != key.canonical_worktree() {
            bail!(
                "overlay runtime source worktree mismatch: sources={}, registry={}",
                sources.worktree().display(),
                key.canonical_worktree().display()
            );
        }
        Ok(Self {
            sources,
            factory: SubscriptionFactory::new(),
        })
    }
}

#[async_trait]
impl RuntimeSubscriptionFactory for CompositeSubscriptionFactory {
    async fn arm(&self, key: &OverlayRuntimeKey) -> anyhow::Result<Box<dyn RuntimeChangeStream>> {
        if self.sources.worktree() != key.canonical_worktree() {
            bail!("overlay runtime registry key changed after subscription construction");
        }
        Ok(Box::new(TaskOneChangeStream {
            subscription: self.factory.subscribe(&self.sources).await,
        }))
    }
}

struct TaskOneChangeStream {
    subscription: ChangeSubscription,
}

#[async_trait]
impl RuntimeChangeStream for TaskOneChangeStream {
    fn provider(&self) -> ChangeProviderKind {
        self.subscription.provider()
    }

    fn initial_cursor(&self) -> &CompositeCursor {
        self.subscription.initial_cursor()
    }

    async fn next_batch(&mut self) -> Option<ChangeBatch> {
        Some(self.subscription.next_batch().await)
    }
}

#[derive(Clone, Default)]
pub(super) struct OverlayRuntimeRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    entries: Mutex<HashMap<OverlayRuntimeKey, Weak<RuntimeEntry>>>,
    start_gates: Mutex<HashMap<OverlayRuntimeKey, Weak<AsyncMutex<()>>>>,
}

impl OverlayRuntimeRegistry {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) async fn get_or_start(
        &self,
        key: OverlayRuntimeKey,
        subscriptions: Arc<dyn RuntimeSubscriptionFactory>,
        builder: Arc<dyn OverlayGenerationBuilder>,
    ) -> Result<OverlayRuntimeHandle, RuntimeStartupError> {
        if let Some(entry) = self.existing(&key) {
            return Ok(OverlayRuntimeHandle { entry });
        }

        let gate = self.start_gate(&key);
        let _start = gate.lock().await;
        if let Some(entry) = self.existing(&key) {
            return Ok(OverlayRuntimeHandle { entry });
        }

        let initialized =
            initialize_runtime(&key, subscriptions.as_ref(), builder.as_ref()).await?;
        let entry = Arc::new(RuntimeEntry {
            key: key.clone(),
            published: ArcSwap::from_pointee(PublishedState::initial(
                key.clone(),
                initialized.active.provider,
                initialized.active.armed_cursor.clone(),
                initialized.current_cursor,
                initialized.built,
            )),
            actor: Mutex::new(None),
        });
        let actor = tokio::spawn(actor_loop(
            Arc::downgrade(&entry),
            key.clone(),
            initialized.active,
            subscriptions,
            builder,
        ));
        *entry
            .actor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(actor);
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, Arc::downgrade(&entry));
        Ok(OverlayRuntimeHandle { entry })
    }

    fn existing(&self, key: &OverlayRuntimeKey) -> Option<Arc<RuntimeEntry>> {
        let mut entries = self
            .inner
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let existing = entries.get(key).and_then(Weak::upgrade);
        if existing.is_none() {
            entries.remove(key);
        }
        existing
    }

    fn start_gate(&self, key: &OverlayRuntimeKey) -> Arc<AsyncMutex<()>> {
        let mut gates = self
            .inner
            .start_gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(gate) = gates.get(key).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(AsyncMutex::new(()));
        gates.insert(key.clone(), Arc::downgrade(&gate));
        gate
    }
}

struct RuntimeEntry {
    key: OverlayRuntimeKey,
    published: ArcSwap<PublishedState>,
    actor: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for RuntimeEntry {
    fn drop(&mut self) {
        if let Some(actor) = self
            .actor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            actor.abort();
        }
    }
}

#[derive(Clone)]
pub(super) struct OverlayRuntimeHandle {
    entry: Arc<RuntimeEntry>,
}

impl OverlayRuntimeHandle {
    /// Performs one coherent atomic load. This path never resolves Git state,
    /// reads the filesystem, or invokes a generation builder.
    pub(super) fn acquire_published(&self) -> Arc<PublishedState> {
        let published = self.entry.published.load_full();
        assert_eq!(
            published.key, self.entry.key,
            "overlay runtime registry returned a mismatched published key"
        );
        published
    }
}

struct InitializedRuntime {
    active: ActiveSubscription,
    current_cursor: CompositeCursor,
    built: BuiltOverlayGeneration,
}

struct ActiveSubscription {
    provider: ChangeProviderKind,
    armed_cursor: CompositeCursor,
    receiver: mpsc::UnboundedReceiver<ChangeBatch>,
    barriers: mpsc::UnboundedSender<oneshot::Sender<()>>,
    pump: JoinHandle<()>,
}

impl ActiveSubscription {
    async fn arm(
        key: &OverlayRuntimeKey,
        subscriptions: &dyn RuntimeSubscriptionFactory,
    ) -> Result<Self, RuntimeStartupError> {
        let mut stream = subscriptions.arm(key).await.map_err(|error| {
            RuntimeStartupError::new(RuntimeStartupCause::SubscriptionArm, error)
        })?;
        let provider = stream.provider();
        if provider == ChangeProviderKind::ExactOnly {
            return Err(RuntimeStartupError::new(
                RuntimeStartupCause::ProviderUnavailable,
                anyhow!(
                    "change providers unavailable; exact-only runtime cannot publish trusted state"
                ),
            ));
        }
        let armed_cursor = stream.initial_cursor().clone();
        let pump_cursor = armed_cursor.clone();
        let (sender, receiver) = mpsc::unbounded_channel();
        let (barriers, mut barrier_receiver) = mpsc::unbounded_channel::<oneshot::Sender<()>>();
        let pump = tokio::spawn(async move {
            let mut current_cursor = pump_cursor;
            loop {
                tokio::select! {
                    biased;
                    batch = stream.next_batch() => {
                        match batch {
                            Some(batch) => {
                                current_cursor = batch.cursor().clone();
                                let trust_lost = matches!(batch, ChangeBatch::TrustLost { .. });
                                if sender.send(batch).is_err() || trust_lost {
                                    break;
                                }
                            }
                            None => {
                                let _ = sender.send(ChangeBatch::TrustLost {
                                    cursor: current_cursor,
                                    reason: TrustLoss::ChannelDisconnected { provider },
                                });
                                break;
                            }
                        }
                    }
                    barrier = barrier_receiver.recv() => {
                        let Some(barrier) = barrier else {
                            break;
                        };
                        let _ = barrier.send(());
                    }
                }
            }
        });
        Ok(Self {
            provider,
            armed_cursor,
            receiver,
            barriers,
            pump,
        })
    }

    async fn next_batch(&mut self) -> Option<ChangeBatch> {
        self.receiver.recv().await
    }

    async fn flush_immediately_available(&self) {
        let (sender, receiver) = oneshot::channel();
        if self.barriers.send(sender).is_ok() {
            let _ = receiver.await;
        }
    }
}

impl Drop for ActiveSubscription {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

async fn initialize_runtime(
    key: &OverlayRuntimeKey,
    subscriptions: &dyn RuntimeSubscriptionFactory,
    builder: &dyn OverlayGenerationBuilder,
) -> Result<InitializedRuntime, RuntimeStartupError> {
    let mut active = ActiveSubscription::arm(key, subscriptions).await?;
    let mut built = builder
        .exact_scan(key)
        .await
        .map_err(|error| RuntimeStartupError::new(RuntimeStartupCause::ExactSeed, error))?;
    let mut current_cursor = active.armed_cursor.clone();

    loop {
        active.flush_immediately_available().await;
        let mut changed_paths = BTreeSet::new();
        let requires_exact_replay =
            drain_startup_available(&mut active, &mut current_cursor, &mut changed_paths).map_err(
                |reason| {
                    RuntimeStartupError::new(
                        RuntimeStartupCause::ReplayTrustLost,
                        anyhow!("trust lost while replaying after exact scan: {reason:?}"),
                    )
                },
            )?;
        if requires_exact_replay {
            built = builder.exact_scan(key).await.map_err(|error| {
                RuntimeStartupError::new(RuntimeStartupCause::ReplayRebuild, error)
            })?;
            continue;
        }
        if changed_paths.is_empty() {
            return Ok(InitializedRuntime {
                active,
                current_cursor,
                built,
            });
        }
        built = builder
            .rebuild_incremental(key, built, changed_paths)
            .await
            .map_err(|error| RuntimeStartupError::new(RuntimeStartupCause::ReplayRebuild, error))?;
    }
}

async fn actor_loop(
    entry: Weak<RuntimeEntry>,
    key: OverlayRuntimeKey,
    mut active: ActiveSubscription,
    subscriptions: Arc<dyn RuntimeSubscriptionFactory>,
    builder: Arc<dyn OverlayGenerationBuilder>,
) {
    loop {
        let Some(batch) = active.next_batch().await else {
            return;
        };
        match process_batch(&entry, &key, &mut active, builder.as_ref(), batch).await {
            ProcessOutcome::Continue => {}
            ProcessOutcome::Recover => {
                let Some(recovered) =
                    recover_runtime(&entry, &key, subscriptions.as_ref(), builder.as_ref()).await
                else {
                    return;
                };
                active = recovered;
            }
            ProcessOutcome::Stop => return,
        }
    }
}

enum ProcessOutcome {
    Continue,
    Recover,
    Stop,
}

async fn process_batch(
    entry: &Weak<RuntimeEntry>,
    key: &OverlayRuntimeKey,
    active: &mut ActiveSubscription,
    builder: &dyn OverlayGenerationBuilder,
    first_batch: ChangeBatch,
) -> ProcessOutcome {
    if matches!(first_batch, ChangeBatch::Ignored { .. }) {
        return ProcessOutcome::Continue;
    }
    let mut current_cursor = first_batch.cursor().clone();
    let mut changed_paths = BTreeSet::new();
    if let Err(reason) = absorb_batch(first_batch, &mut current_cursor, &mut changed_paths) {
        return if publish_untrusted(entry, active, current_cursor, reason) {
            ProcessOutcome::Recover
        } else {
            ProcessOutcome::Stop
        };
    }
    if !publish_rebuilding(entry, active, current_cursor.clone()) {
        return ProcessOutcome::Stop;
    }
    active.flush_immediately_available().await;
    if let Err(reason) = drain_available(active, &mut current_cursor, &mut changed_paths) {
        return if publish_untrusted(entry, active, current_cursor, reason) {
            ProcessOutcome::Recover
        } else {
            ProcessOutcome::Stop
        };
    }

    let Some(runtime) = entry.upgrade() else {
        return ProcessOutcome::Stop;
    };
    let mut built = BuiltOverlayGeneration::from_published(&runtime.published.load_full());
    drop(runtime);

    loop {
        match builder.rebuild_incremental(key, built, changed_paths).await {
            Ok(next) => built = next,
            Err(error) => {
                let reason = PublishedUntrustedReason::BuildFailed(format!(
                    "incremental overlay rebuild failed: {error:#}"
                ));
                return if publish_untrusted(entry, active, current_cursor, reason) {
                    ProcessOutcome::Recover
                } else {
                    ProcessOutcome::Stop
                };
            }
        }

        active.flush_immediately_available().await;
        changed_paths = BTreeSet::new();
        if let Err(reason) = drain_available(active, &mut current_cursor, &mut changed_paths) {
            return if publish_untrusted(entry, active, current_cursor, reason) {
                ProcessOutcome::Recover
            } else {
                ProcessOutcome::Stop
            };
        }
        if changed_paths.is_empty() {
            return if publish_trusted(entry, active, current_cursor, built) {
                ProcessOutcome::Continue
            } else {
                ProcessOutcome::Stop
            };
        }
    }
}

async fn recover_runtime(
    entry: &Weak<RuntimeEntry>,
    key: &OverlayRuntimeKey,
    subscriptions: &dyn RuntimeSubscriptionFactory,
    builder: &dyn OverlayGenerationBuilder,
) -> Option<ActiveSubscription> {
    match initialize_runtime(key, subscriptions, builder).await {
        Ok(initialized) => {
            let runtime = entry.upgrade()?;
            let previous = runtime.published.load_full();
            let epoch = next_epoch(previous.epoch)?;
            runtime
                .published
                .store(Arc::new(PublishedState::replacing_generation(
                    &previous,
                    initialized.active.provider,
                    initialized.active.armed_cursor.clone(),
                    initialized.current_cursor,
                    epoch,
                    initialized.built,
                )));
            Some(initialized.active)
        }
        Err(error) => {
            let runtime = entry.upgrade()?;
            let previous = runtime.published.load_full();
            let epoch = next_epoch(previous.epoch)?;
            runtime
                .published
                .store(Arc::new(PublishedState::retaining_generation(
                    &previous,
                    previous.provider,
                    previous.armed_cursor.clone(),
                    previous.current_cursor.clone(),
                    epoch,
                    PublishedTrust::Untrusted(PublishedUntrustedReason::BuildFailed(format!(
                        "exact overlay recovery failed: {error:#}"
                    ))),
                )));
            None
        }
    }
}

fn publish_rebuilding(
    entry: &Weak<RuntimeEntry>,
    active: &ActiveSubscription,
    current_cursor: CompositeCursor,
) -> bool {
    let Some(runtime) = entry.upgrade() else {
        return false;
    };
    let previous = runtime.published.load_full();
    let Some(epoch) = next_epoch(previous.epoch) else {
        return false;
    };
    runtime
        .published
        .store(Arc::new(PublishedState::retaining_generation(
            &previous,
            active.provider,
            active.armed_cursor.clone(),
            current_cursor,
            epoch,
            PublishedTrust::Rebuilding,
        )));
    true
}

fn publish_untrusted(
    entry: &Weak<RuntimeEntry>,
    active: &ActiveSubscription,
    current_cursor: CompositeCursor,
    reason: PublishedUntrustedReason,
) -> bool {
    let Some(runtime) = entry.upgrade() else {
        return false;
    };
    let previous = runtime.published.load_full();
    let Some(epoch) = next_epoch(previous.epoch) else {
        return false;
    };
    runtime
        .published
        .store(Arc::new(PublishedState::retaining_generation(
            &previous,
            active.provider,
            active.armed_cursor.clone(),
            current_cursor,
            epoch,
            PublishedTrust::Untrusted(reason),
        )));
    true
}

fn publish_trusted(
    entry: &Weak<RuntimeEntry>,
    active: &ActiveSubscription,
    current_cursor: CompositeCursor,
    built: BuiltOverlayGeneration,
) -> bool {
    let Some(runtime) = entry.upgrade() else {
        return false;
    };
    let previous = runtime.published.load_full();
    let Some(epoch) = next_epoch(previous.epoch) else {
        return false;
    };
    runtime
        .published
        .store(Arc::new(PublishedState::replacing_generation(
            &previous,
            active.provider,
            active.armed_cursor.clone(),
            current_cursor,
            epoch,
            built,
        )));
    true
}

fn next_epoch(epoch: u64) -> Option<u64> {
    epoch.checked_add(1)
}

fn drain_available(
    active: &mut ActiveSubscription,
    current_cursor: &mut CompositeCursor,
    changed_paths: &mut BTreeSet<PathBuf>,
) -> Result<(), PublishedUntrustedReason> {
    loop {
        match active.receiver.try_recv() {
            Ok(batch) => absorb_batch(batch, current_cursor, changed_paths)?,
            Err(mpsc::error::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return Err(PublishedUntrustedReason::Provider(
                    TrustLoss::ChannelDisconnected {
                        provider: active.provider,
                    },
                ));
            }
        }
    }
}

fn drain_startup_available(
    active: &mut ActiveSubscription,
    current_cursor: &mut CompositeCursor,
    changed_paths: &mut BTreeSet<PathBuf>,
) -> Result<bool, PublishedUntrustedReason> {
    let mut requires_exact_replay = false;
    loop {
        match active.receiver.try_recv() {
            Ok(batch) => absorb_startup_batch(
                batch,
                current_cursor,
                changed_paths,
                &mut requires_exact_replay,
            )?,
            Err(mpsc::error::TryRecvError::Empty) => return Ok(requires_exact_replay),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return Err(PublishedUntrustedReason::Provider(
                    TrustLoss::ChannelDisconnected {
                        provider: active.provider,
                    },
                ));
            }
        }
    }
}

fn absorb_startup_batch(
    batch: ChangeBatch,
    current_cursor: &mut CompositeCursor,
    changed_paths: &mut BTreeSet<PathBuf>,
    requires_exact_replay: &mut bool,
) -> Result<(), PublishedUntrustedReason> {
    *current_cursor = batch.cursor().clone();
    match batch {
        ChangeBatch::Ignored { .. } => Ok(()),
        ChangeBatch::TrustLost { reason, .. } => Err(PublishedUntrustedReason::Provider(reason)),
        ChangeBatch::Changes {
            added,
            modified,
            deleted,
            renamed,
            git_metadata,
            ..
        } => {
            *requires_exact_replay |= !git_metadata.is_empty();
            let mut batch_paths = BTreeSet::new();
            batch_paths.extend(added);
            batch_paths.extend(modified);
            batch_paths.extend(deleted);
            for (from, to) in renamed {
                batch_paths.extend([from, to]);
            }
            if batch_paths.is_empty() && git_metadata.is_empty() {
                return Err(PublishedUntrustedReason::EmptyChangeBatch);
            }
            changed_paths.extend(batch_paths);
            Ok(())
        }
    }
}

fn absorb_batch(
    batch: ChangeBatch,
    current_cursor: &mut CompositeCursor,
    changed_paths: &mut BTreeSet<PathBuf>,
) -> Result<(), PublishedUntrustedReason> {
    *current_cursor = batch.cursor().clone();
    match batch {
        ChangeBatch::Ignored { .. } => Ok(()),
        ChangeBatch::TrustLost { reason, .. } => Err(PublishedUntrustedReason::Provider(reason)),
        ChangeBatch::Changes {
            added,
            modified,
            deleted,
            renamed,
            git_metadata,
            ..
        } => {
            if !git_metadata.is_empty() {
                return Err(PublishedUntrustedReason::GitMetadata(git_metadata));
            }
            let mut batch_paths = BTreeSet::new();
            batch_paths.extend(added);
            batch_paths.extend(modified);
            batch_paths.extend(deleted);
            for (from, to) in renamed {
                batch_paths.extend([from, to]);
            }
            if batch_paths.is_empty() {
                return Err(PublishedUntrustedReason::EmptyChangeBatch);
            }
            changed_paths.extend(batch_paths);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use anyhow::{anyhow, bail};
    use async_trait::async_trait;
    use tokio::sync::{mpsc, Semaphore};

    use super::*;
    use crate::overlay_watch::{ChangeBatch, ChangeProviderKind, CompositeCursor, TrustLoss};
    use crate::schema::{GraphIndexHeader, GRAPH_INDEX_VERSION_TEMPORAL};
    use crate::{GraphIndexArtifact, OverlayGeneration, OverlayGenerationIdentity};

    #[derive(Clone, Default)]
    struct FakeSubscriber {
        inner: Arc<FakeSubscriberInner>,
    }

    #[derive(Default)]
    struct FakeSubscriberInner {
        arms: AtomicUsize,
        delivered: AtomicUsize,
        stream_drops: AtomicUsize,
        senders: Mutex<Vec<mpsc::UnboundedSender<ChangeBatch>>>,
        trace: Arc<Mutex<Vec<&'static str>>>,
    }

    impl FakeSubscriber {
        fn with_trace(trace: Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self {
                inner: Arc::new(FakeSubscriberInner {
                    trace,
                    ..FakeSubscriberInner::default()
                }),
            }
        }

        fn arm_count(&self) -> usize {
            self.inner.arms.load(Ordering::SeqCst)
        }

        fn delivered_count(&self) -> usize {
            self.inner.delivered.load(Ordering::SeqCst)
        }

        fn stream_drop_count(&self) -> usize {
            self.inner.stream_drops.load(Ordering::SeqCst)
        }

        fn send(&self, arm: usize, batch: ChangeBatch) {
            self.inner
                .senders
                .lock()
                .expect("fake sender lock")
                .get(arm)
                .expect("armed fake stream")
                .send(batch)
                .expect("live fake stream");
        }
    }

    #[async_trait]
    impl RuntimeSubscriptionFactory for FakeSubscriber {
        async fn arm(
            &self,
            _key: &OverlayRuntimeKey,
        ) -> anyhow::Result<Box<dyn RuntimeChangeStream>> {
            self.inner.trace.lock().expect("trace lock").push("arm");
            let arm = self.inner.arms.fetch_add(1, Ordering::SeqCst);
            let cursor = cursor(arm as u64);
            let (sender, receiver) = mpsc::unbounded_channel();
            self.inner
                .senders
                .lock()
                .expect("fake sender lock")
                .push(sender);
            Ok(Box::new(FakeStream {
                cursor,
                receiver,
                inner: Arc::clone(&self.inner),
            }))
        }
    }

    struct FakeStream {
        cursor: CompositeCursor,
        receiver: mpsc::UnboundedReceiver<ChangeBatch>,
        inner: Arc<FakeSubscriberInner>,
    }

    #[async_trait]
    impl RuntimeChangeStream for FakeStream {
        fn provider(&self) -> ChangeProviderKind {
            ChangeProviderKind::Watchman
        }

        fn initial_cursor(&self) -> &CompositeCursor {
            &self.cursor
        }

        async fn next_batch(&mut self) -> Option<ChangeBatch> {
            let batch = self.receiver.recv().await;
            if batch.is_some() {
                self.inner.delivered.fetch_add(1, Ordering::SeqCst);
            }
            batch
        }
    }

    impl Drop for FakeStream {
        fn drop(&mut self) {
            self.inner.stream_drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Clone)]
    struct FakeBuilder {
        inner: Arc<FakeBuilderInner>,
    }

    struct FakeBuilderInner {
        exact_calls: AtomicUsize,
        incremental_calls: AtomicUsize,
        build_ids: AtomicUsize,
        block_exact: AtomicBool,
        block_incremental: AtomicBool,
        fail_next_exact: AtomicBool,
        fail_next_incremental: AtomicBool,
        exact_permits: Semaphore,
        incremental_permits: Semaphore,
        incremental_paths: Mutex<Vec<BTreeSet<PathBuf>>>,
        trace: Arc<Mutex<Vec<&'static str>>>,
    }

    impl FakeBuilder {
        fn new(trace: Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self {
                inner: Arc::new(FakeBuilderInner {
                    exact_calls: AtomicUsize::new(0),
                    incremental_calls: AtomicUsize::new(0),
                    build_ids: AtomicUsize::new(0),
                    block_exact: AtomicBool::new(false),
                    block_incremental: AtomicBool::new(false),
                    fail_next_exact: AtomicBool::new(false),
                    fail_next_incremental: AtomicBool::new(false),
                    exact_permits: Semaphore::new(0),
                    incremental_permits: Semaphore::new(0),
                    incremental_paths: Mutex::new(Vec::new()),
                    trace,
                }),
            }
        }

        fn standalone() -> Self {
            Self::new(Arc::new(Mutex::new(Vec::new())))
        }

        fn block_exact(&self) {
            self.inner.block_exact.store(true, Ordering::SeqCst);
        }

        fn release_exact(&self) {
            self.inner.exact_permits.add_permits(1);
        }

        fn block_incremental(&self) {
            self.inner.block_incremental.store(true, Ordering::SeqCst);
        }

        fn release_incremental(&self) {
            self.inner.incremental_permits.add_permits(1);
        }

        fn fail_next_exact(&self) {
            self.inner.fail_next_exact.store(true, Ordering::SeqCst);
        }

        fn fail_next_incremental(&self) {
            self.inner
                .fail_next_incremental
                .store(true, Ordering::SeqCst);
        }

        fn exact_calls(&self) -> usize {
            self.inner.exact_calls.load(Ordering::SeqCst)
        }

        fn incremental_calls(&self) -> usize {
            self.inner.incremental_calls.load(Ordering::SeqCst)
        }

        fn incremental_paths(&self) -> Vec<BTreeSet<PathBuf>> {
            self.inner
                .incremental_paths
                .lock()
                .expect("incremental paths lock")
                .clone()
        }

        async fn wait_if_blocked(blocked: &AtomicBool, permits: &Semaphore) {
            if blocked.load(Ordering::SeqCst) {
                permits
                    .acquire()
                    .await
                    .expect("fake builder permit")
                    .forget();
            }
        }

        fn built(&self, key: &OverlayRuntimeKey) -> anyhow::Result<BuiltOverlayGeneration> {
            let build_id = self.inner.build_ids.fetch_add(1, Ordering::SeqCst) + 1;
            let generation = Arc::new(OverlayGeneration::seed(Arc::new(empty_artifact(
                key.base_graph_identity(),
            )))?);
            let mut fingerprint = [0_u8; 32];
            fingerprint[0] = u8::try_from(build_id).expect("bounded test build id");
            BuiltOverlayGeneration::new(
                key,
                OverlayGenerationIdentity {
                    canonical_worktree: key.canonical_worktree().to_path_buf(),
                    indexed_graph_content_hash: key.base_graph_identity().to_owned(),
                    indexed_head_oid: Some("indexed".to_owned()),
                    current_head_oid: format!("head-{build_id}"),
                    index_identity: format!("index-{build_id}"),
                    normalized_changed_set_fingerprint: fingerprint,
                },
                generation,
            )
        }
    }

    #[async_trait]
    impl OverlayGenerationBuilder for FakeBuilder {
        async fn exact_scan(
            &self,
            key: &OverlayRuntimeKey,
        ) -> anyhow::Result<BuiltOverlayGeneration> {
            self.inner.exact_calls.fetch_add(1, Ordering::SeqCst);
            self.inner
                .trace
                .lock()
                .expect("trace lock")
                .push("exact_start");
            Self::wait_if_blocked(&self.inner.block_exact, &self.inner.exact_permits).await;
            if self.inner.fail_next_exact.swap(false, Ordering::SeqCst) {
                bail!("scripted exact failure");
            }
            self.inner
                .trace
                .lock()
                .expect("trace lock")
                .push("exact_finish");
            self.built(key)
        }

        async fn rebuild_incremental(
            &self,
            key: &OverlayRuntimeKey,
            _previous: BuiltOverlayGeneration,
            changed_paths: BTreeSet<PathBuf>,
        ) -> anyhow::Result<BuiltOverlayGeneration> {
            self.inner.incremental_calls.fetch_add(1, Ordering::SeqCst);
            self.inner
                .incremental_paths
                .lock()
                .expect("incremental paths lock")
                .push(changed_paths);
            self.inner
                .trace
                .lock()
                .expect("trace lock")
                .push("incremental_start");
            Self::wait_if_blocked(
                &self.inner.block_incremental,
                &self.inner.incremental_permits,
            )
            .await;
            if self
                .inner
                .fail_next_incremental
                .swap(false, Ordering::SeqCst)
            {
                bail!("scripted incremental failure");
            }
            self.inner
                .trace
                .lock()
                .expect("trace lock")
                .push("incremental_finish");
            self.built(key)
        }
    }

    fn key(worktree: &str, base: &str) -> OverlayRuntimeKey {
        OverlayRuntimeKey::new(PathBuf::from(worktree), base.to_owned())
    }

    fn cursor(sequence: u64) -> CompositeCursor {
        CompositeCursor::from_entries([(PathBuf::from("/watched"), sequence.to_string())])
    }

    fn changes(sequence: u64, paths: &[&str]) -> ChangeBatch {
        ChangeBatch::Changes {
            cursor: cursor(sequence),
            added: paths.iter().map(PathBuf::from).collect(),
            modified: BTreeSet::new(),
            deleted: BTreeSet::new(),
            renamed: BTreeSet::new(),
            git_metadata: BTreeSet::new(),
        }
    }

    fn ignored(sequence: u64) -> ChangeBatch {
        ChangeBatch::Ignored {
            cursor: cursor(sequence),
        }
    }

    fn loss(sequence: u64) -> ChangeBatch {
        ChangeBatch::TrustLost {
            cursor: cursor(sequence),
            reason: TrustLoss::Overflow {
                provider: ChangeProviderKind::Watchman,
            },
        }
    }

    fn empty_artifact(hash: &str) -> GraphIndexArtifact {
        GraphIndexArtifact {
            header: GraphIndexHeader {
                graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.to_owned(),
                content_hash_blake3: None,
            },
            manifest_version: "overlay-runtime-test".to_owned(),
            graph_content_hash: hash.to_owned(),
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

    async fn wait_for(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("deterministic fake made progress");
    }

    async fn start(
        registry: &OverlayRuntimeRegistry,
        key: OverlayRuntimeKey,
        subscriber: Arc<FakeSubscriber>,
        builder: Arc<FakeBuilder>,
    ) -> anyhow::Result<OverlayRuntimeHandle> {
        Ok(registry.get_or_start(key, subscriber, builder).await?)
    }

    #[tokio::test]
    async fn overlay_runtime_orders_arm_exact_scan_replay_before_first_publish() {
        let registry = OverlayRuntimeRegistry::new();
        let trace = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Arc::new(FakeSubscriber::with_trace(Arc::clone(&trace)));
        let builder = Arc::new(FakeBuilder::new(Arc::clone(&trace)));

        let handle = start(
            &registry,
            key("/worktree", "base"),
            Arc::clone(&subscriber),
            builder,
        )
        .await
        .expect("runtime starts");
        let state = handle.acquire_published();

        assert_eq!(state.epoch(), 0);
        assert_eq!(state.trust(), &PublishedTrust::Trusted);
        assert_eq!(state.provider(), ChangeProviderKind::Watchman);
        assert_eq!(state.armed_cursor(), &cursor(0));
        assert_eq!(state.current_cursor(), &cursor(0));
        assert_eq!(
            trace.lock().expect("trace lock").as_slice(),
            ["arm", "exact_start", "exact_finish"]
        );
    }

    #[tokio::test]
    async fn overlay_runtime_replays_scan_events_into_generation_zero_and_coalesces_paths() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber = Arc::new(FakeSubscriber::default());
        let builder = Arc::new(FakeBuilder::standalone());
        builder.block_exact();
        let task = tokio::spawn({
            let registry = registry.clone();
            let subscriber = Arc::clone(&subscriber);
            let builder = Arc::clone(&builder);
            async move { start(&registry, key("/worktree", "base"), subscriber, builder).await }
        });

        wait_for(|| subscriber.arm_count() == 1 && builder.exact_calls() == 1).await;
        subscriber.send(0, changes(1, &["/worktree/a.rs", "/worktree/b.rs"]));
        subscriber.send(0, changes(2, &["/worktree/b.rs", "/worktree/c.rs"]));
        subscriber.send(0, changes(3, &["/worktree/b.rs"]));
        builder.release_exact();
        let handle = task.await.expect("start task").expect("runtime starts");
        let state = handle.acquire_published();

        assert_eq!(subscriber.delivered_count(), 3);
        assert_eq!(state.epoch(), 0, "replay belongs to generation zero");
        assert_eq!(state.current_cursor(), &cursor(3));
        assert_eq!(builder.incremental_calls(), 1);
        assert_eq!(
            builder.incremental_paths(),
            vec![BTreeSet::from([
                PathBuf::from("/worktree/a.rs"),
                PathBuf::from("/worktree/b.rs"),
                PathBuf::from("/worktree/c.rs"),
            ])]
        );
        assert_eq!(
            state.snapshot_identity().normalized_changed_set_fingerprint[0],
            2
        );
    }

    #[tokio::test]
    async fn overlay_runtime_ignored_activity_does_not_trigger_an_empty_rebuild() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber = Arc::new(FakeSubscriber::default());
        let builder = Arc::new(FakeBuilder::standalone());
        let handle = start(
            &registry,
            key("/worktree", "base"),
            Arc::clone(&subscriber),
            Arc::clone(&builder),
        )
        .await
        .expect("runtime starts");

        subscriber.send(0, ignored(1));
        subscriber.send(0, changes(2, &["/worktree/real.rs"]));
        wait_for(|| handle.acquire_published().current_cursor() == &cursor(2)).await;

        assert_eq!(builder.incremental_calls(), 1);
        assert_eq!(
            builder.incremental_paths(),
            vec![BTreeSet::from([PathBuf::from("/worktree/real.rs")])]
        );
        assert_eq!(handle.acquire_published().trust(), &PublishedTrust::Trusted);
    }

    #[tokio::test]
    async fn overlay_runtime_runs_another_pass_for_event_arriving_during_rebuild() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber = Arc::new(FakeSubscriber::default());
        let builder = Arc::new(FakeBuilder::standalone());
        let handle = start(
            &registry,
            key("/worktree", "base"),
            Arc::clone(&subscriber),
            Arc::clone(&builder),
        )
        .await
        .expect("runtime starts");
        let initial = handle.acquire_published();
        builder.block_incremental();

        subscriber.send(0, changes(1, &["/worktree/first.rs"]));
        wait_for(|| builder.incremental_calls() == 1).await;
        subscriber.send(0, changes(2, &["/worktree/second.rs"]));
        wait_for(|| subscriber.delivered_count() == 2).await;
        builder.release_incremental();
        wait_for(|| builder.incremental_calls() == 2).await;

        let rebuilding = handle.acquire_published();
        assert_eq!(rebuilding.trust(), &PublishedTrust::Rebuilding);
        assert!(Arc::ptr_eq(rebuilding.generation(), initial.generation()));
        builder.release_incremental();
        wait_for(|| handle.acquire_published().trust() == &PublishedTrust::Trusted).await;

        let published = handle.acquire_published();
        assert_eq!(published.epoch(), 2);
        assert_eq!(published.current_cursor(), &cursor(2));
        assert_eq!(
            builder.incremental_paths(),
            vec![
                BTreeSet::from([PathBuf::from("/worktree/first.rs")]),
                BTreeSet::from([PathBuf::from("/worktree/second.rs")]),
            ]
        );
        assert_eq!(
            published
                .snapshot_identity()
                .normalized_changed_set_fingerprint[0],
            3
        );
    }

    #[tokio::test]
    async fn overlay_runtime_revokes_trust_before_exact_loss_recovery() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber = Arc::new(FakeSubscriber::default());
        let builder = Arc::new(FakeBuilder::standalone());
        let handle = start(
            &registry,
            key("/worktree", "base"),
            Arc::clone(&subscriber),
            Arc::clone(&builder),
        )
        .await
        .expect("runtime starts");
        let initial = handle.acquire_published();
        builder.block_exact();

        subscriber.send(0, loss(1));
        wait_for(|| builder.exact_calls() == 2).await;
        let revoked = handle.acquire_published();
        assert!(matches!(
            revoked.trust(),
            PublishedTrust::Untrusted(PublishedUntrustedReason::Provider(
                TrustLoss::Overflow { .. }
            ))
        ));
        assert_eq!(revoked.epoch(), 1);
        assert!(Arc::ptr_eq(revoked.generation(), initial.generation()));

        builder.release_exact();
        wait_for(|| handle.acquire_published().trust() == &PublishedTrust::Trusted).await;
        let recovered = handle.acquire_published();
        assert_eq!(subscriber.arm_count(), 2);
        assert_eq!(recovered.epoch(), 2);
        assert!(!Arc::ptr_eq(recovered.generation(), initial.generation()));
    }

    #[tokio::test]
    async fn overlay_runtime_failed_build_never_publishes_partial_generation() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber = Arc::new(FakeSubscriber::default());
        let builder = Arc::new(FakeBuilder::standalone());
        let handle = start(
            &registry,
            key("/worktree", "base"),
            Arc::clone(&subscriber),
            Arc::clone(&builder),
        )
        .await
        .expect("runtime starts");
        let initial = handle.acquire_published();
        builder.fail_next_incremental();
        builder.fail_next_exact();

        subscriber.send(0, changes(1, &["/worktree/fail.rs"]));
        wait_for(|| builder.exact_calls() == 2).await;
        let failed = handle.acquire_published();

        assert!(matches!(failed.trust(), PublishedTrust::Untrusted(_)));
        assert!(Arc::ptr_eq(failed.generation(), initial.generation()));
        assert_eq!(
            failed.snapshot_identity(),
            initial.snapshot_identity(),
            "neither failed incremental output nor failed exact output may publish"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn overlay_runtime_readers_pin_one_coherent_state_during_publication() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber = Arc::new(FakeSubscriber::default());
        let builder = Arc::new(FakeBuilder::standalone());
        let handle = start(
            &registry,
            key("/worktree", "base"),
            Arc::clone(&subscriber),
            Arc::clone(&builder),
        )
        .await
        .expect("runtime starts");
        let initial = handle.acquire_published();
        let initial_generation = Arc::clone(initial.generation());
        builder.block_incremental();
        subscriber.send(0, changes(1, &["/worktree/change.rs"]));
        wait_for(|| builder.incremental_calls() == 1).await;

        let mut readers = VecDeque::new();
        for _ in 0..16 {
            let handle = handle.clone();
            readers.push_back(tokio::spawn(async move {
                let mut observations = Vec::new();
                for _ in 0..64 {
                    let state = handle.acquire_published();
                    observations.push((
                        state.epoch(),
                        state.trust().clone(),
                        Arc::clone(state.generation()),
                        state.snapshot_identity().normalized_changed_set_fingerprint[0],
                    ));
                    tokio::task::yield_now().await;
                }
                observations
            }));
        }
        builder.release_incremental();
        wait_for(|| handle.acquire_published().trust() == &PublishedTrust::Trusted).await;
        let final_state = handle.acquire_published();
        let final_generation = Arc::clone(final_state.generation());

        while let Some(reader) = readers.pop_front() {
            for (epoch, trust, generation, fingerprint) in
                reader.await.expect("coherent reader task")
            {
                match epoch {
                    0 => {
                        assert_eq!(trust, PublishedTrust::Trusted);
                        assert!(Arc::ptr_eq(&generation, &initial_generation));
                        assert_eq!(fingerprint, 1);
                    }
                    1 => {
                        assert_eq!(trust, PublishedTrust::Rebuilding);
                        assert!(Arc::ptr_eq(&generation, &initial_generation));
                        assert_eq!(fingerprint, 1);
                    }
                    2 => {
                        assert_eq!(trust, PublishedTrust::Trusted);
                        assert!(Arc::ptr_eq(&generation, &final_generation));
                        assert_eq!(fingerprint, 2);
                    }
                    other => panic!("reader observed impossible epoch {other}"),
                }
            }
        }
        assert_eq!(initial.epoch(), 0, "the reader's old Arc stays pinned");
        assert!(Arc::ptr_eq(initial.generation(), &initial_generation));
    }

    #[tokio::test]
    async fn overlay_runtime_registry_isolates_worktree_and_base_identity_keys() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber_a = Arc::new(FakeSubscriber::default());
        let subscriber_duplicate = Arc::new(FakeSubscriber::default());
        let subscriber_b = Arc::new(FakeSubscriber::default());
        let subscriber_c = Arc::new(FakeSubscriber::default());
        let handle_a = start(
            &registry,
            key("/worktree-a", "base-1"),
            Arc::clone(&subscriber_a),
            Arc::new(FakeBuilder::standalone()),
        )
        .await
        .expect("first runtime");
        let handle_a_again = start(
            &registry,
            key("/worktree-a", "base-1"),
            Arc::clone(&subscriber_duplicate),
            Arc::new(FakeBuilder::standalone()),
        )
        .await
        .expect("same runtime");
        let handle_b = start(
            &registry,
            key("/worktree-a", "base-2"),
            Arc::clone(&subscriber_b),
            Arc::new(FakeBuilder::standalone()),
        )
        .await
        .expect("new base runtime");
        let handle_c = start(
            &registry,
            key("/worktree-b", "base-1"),
            Arc::clone(&subscriber_c),
            Arc::new(FakeBuilder::standalone()),
        )
        .await
        .expect("new worktree runtime");

        assert!(Arc::ptr_eq(
            &handle_a.acquire_published(),
            &handle_a_again.acquire_published()
        ));
        assert!(!Arc::ptr_eq(
            &handle_a.acquire_published(),
            &handle_b.acquire_published()
        ));
        assert!(!Arc::ptr_eq(
            &handle_a.acquire_published(),
            &handle_c.acquire_published()
        ));
        assert_eq!(subscriber_a.arm_count(), 1);
        assert_eq!(subscriber_duplicate.arm_count(), 0);
        assert_eq!(subscriber_b.arm_count(), 1);
        assert_eq!(subscriber_c.arm_count(), 1);
    }

    #[tokio::test]
    async fn overlay_runtime_stops_actor_when_final_handle_drops() {
        let registry = OverlayRuntimeRegistry::new();
        let subscriber = Arc::new(FakeSubscriber::default());
        let runtime_key = key("/worktree", "base");
        let builder = Arc::new(FakeBuilder::standalone());
        let handle = start(
            &registry,
            runtime_key.clone(),
            Arc::clone(&subscriber),
            Arc::clone(&builder),
        )
        .await
        .expect("runtime starts");
        let pinned = handle.acquire_published();
        let clone = handle.clone();
        builder.block_incremental();
        subscriber.send(0, changes(1, &["/worktree/cancelled.rs"]));
        wait_for(|| builder.incremental_calls() == 1).await;

        drop(handle);
        tokio::task::yield_now().await;
        assert_eq!(subscriber.stream_drop_count(), 0);
        drop(clone);
        wait_for(|| subscriber.stream_drop_count() == 1).await;
        assert_eq!(pinned.epoch(), 0);
        assert_eq!(pinned.trust(), &PublishedTrust::Trusted);

        let replacement = start(
            &registry,
            runtime_key,
            Arc::clone(&subscriber),
            Arc::new(FakeBuilder::standalone()),
        )
        .await
        .map_err(|error| anyhow!("replacement actor failed: {error:#}"))
        .expect("replacement runtime");
        assert_eq!(subscriber.arm_count(), 2);
        drop(replacement);
    }

    #[test]
    fn overlay_runtime_key_validation_is_pure_and_rejects_mismatched_builds() {
        let key = key("/worktree", "base");
        let generation = Arc::new(
            OverlayGeneration::seed(Arc::new(empty_artifact("other")))
                .expect("seed mismatched test generation"),
        );
        let error = BuiltOverlayGeneration::new(
            &key,
            OverlayGenerationIdentity {
                canonical_worktree: Path::new("/worktree").to_path_buf(),
                indexed_graph_content_hash: "base".to_owned(),
                indexed_head_oid: None,
                current_head_oid: "head".to_owned(),
                index_identity: "index".to_owned(),
                normalized_changed_set_fingerprint: [0; 32],
            },
            generation,
        )
        .expect_err("mismatched base generation must fail closed");
        assert!(error.to_string().contains("base identity"));
    }
}
