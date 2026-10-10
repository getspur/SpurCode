use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use beads_rust::storage::sqlite::SqliteStorage;

use crate::beads_crate::metrics::ContentionMetrics;

pub(crate) const DEFAULT_READER_THREADS: usize = 4;

const CHANNEL_CAPACITY: usize = 1024;
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(30);
const JOIN_TIMEOUT: Duration = Duration::from_secs(1);
const MIN_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

type WriteJob = Box<dyn FnOnce(&mut SqliteStorage) + Send + 'static>;
type ReadJob = Box<dyn FnOnce(&SqliteStorage) + Send + 'static>;
type SummaryReadJob = Box<dyn FnOnce(&rusqlite::Connection) + Send + 'static>;

enum WriteMsg {
    Job(WriteJob),
    Checkpoint,
    Shutdown,
}

enum ReadMsg {
    Job(ReadJob),
    Summary(SummaryReadJob),
}

pub(crate) struct BeadsDb {
    write_tx: Option<SyncSender<WriteMsg>>,
    read_tx: Option<SyncSender<ReadMsg>>,
    threads: Mutex<Vec<DbThread>>,
}

struct DbThread {
    name: String,
    join: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
}

impl BeadsDb {
    #[expect(
        dead_code,
        reason = "connection actor design exposes spawn without metrics for module users"
    )]
    pub(crate) fn spawn(
        beads_dir: PathBuf,
        lock_timeout_ms: u64,
        reader_threads: usize,
    ) -> anyhow::Result<Self> {
        Self::spawn_with_metrics(
            beads_dir,
            lock_timeout_ms,
            reader_threads,
            Arc::new(ContentionMetrics::default()),
        )
    }

    pub(crate) fn spawn_with_metrics(
        beads_dir: PathBuf,
        lock_timeout_ms: u64,
        reader_threads: usize,
        metrics: Arc<ContentionMetrics>,
    ) -> anyhow::Result<Self> {
        let reader_threads = reader_threads.max(1);
        let db_path = beads_dir.join("beads.db");
        let startup_timeout = startup_timeout(lock_timeout_ms);

        let (write_tx, write_rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (read_tx, read_rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let shared_read_rx = Arc::new(Mutex::new(read_rx));

        let mut threads = Vec::with_capacity(reader_threads + 1);
        match spawn_writer(
            db_path.clone(),
            lock_timeout_ms,
            Arc::clone(&metrics),
            write_rx,
            startup_timeout,
        ) {
            Ok(thread) => threads.push(thread),
            Err(err) => {
                drop(write_tx);
                drop(read_tx);
                return Err(err);
            }
        }

        // The writer has initialized the schema. Ensure this database-wide
        // index once before starting readers, outside actor readiness deadlines
        // so an optional index's busy wait cannot make actor startup fail.
        if let Err(err) = crate::beads_crate::sort_index::ensure_composite_sort_index(&db_path) {
            tracing::warn!(
                ?err,
                ?db_path,
                "composite sort index ensure failed (best-effort)"
            );
        }

        for index in 0..reader_threads {
            match spawn_reader(
                index,
                db_path.clone(),
                lock_timeout_ms,
                Arc::clone(&metrics),
                Arc::clone(&shared_read_rx),
                startup_timeout,
            ) {
                Ok(thread) => threads.push(thread),
                Err(err) => {
                    drop(write_tx);
                    drop(read_tx);
                    join_threads_bounded(&mut threads);
                    return Err(err);
                }
            }
        }

        Ok(Self {
            write_tx: Some(write_tx),
            read_tx: Some(read_tx),
            threads: Mutex::new(threads),
        })
    }

    pub(crate) async fn submit_write<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut SqliteStorage) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let job = Box::new(move |storage: &mut SqliteStorage| {
            let _ = reply_tx.send(f(storage));
        });

        self.write_tx
            .as_ref()
            .context("beads db writer is not running")?
            .try_send(WriteMsg::Job(job))
            .map_err(write_send_error)?;

        reply_rx
            .await
            .context("beads db writer stopped before replying")?
    }

    pub(crate) async fn submit_read<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&SqliteStorage) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let span = tracing::Span::current();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let job = Box::new(move |storage: &SqliteStorage| {
            // Reader threads outlive requests. Bind both the subscriber and
            // span only while executing this job, including error replies and
            // dropped receivers; never hold a tracing guard across an await.
            tracing::dispatcher::with_default(&dispatch, || {
                span.in_scope(|| {
                    let _ = reply_tx.send(f(storage));
                });
            });
        });

        self.read_tx
            .as_ref()
            .context("beads db readers are not running")?
            .try_send(ReadMsg::Job(job))
            .map_err(read_send_error)?;

        reply_rx
            .await
            .context("beads db reader stopped before replying")?
    }

    /// Run narrow SQL on a persistent read-only companion, using the same
    /// bounded queue, reader threads, tracing context and reply semantics.
    pub(crate) async fn submit_summary_read<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&rusqlite::Connection) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let span = tracing::Span::current();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let job = Box::new(move |storage: &rusqlite::Connection| {
            // Reader threads outlive requests. Bind both the subscriber and
            // span only while executing this job, including error replies and
            // dropped receivers; never hold a tracing guard across an await.
            tracing::dispatcher::with_default(&dispatch, || {
                span.in_scope(|| {
                    let _ = reply_tx.send(f(storage));
                });
            });
        });

        self.read_tx
            .as_ref()
            .context("beads db readers are not running")?
            .try_send(ReadMsg::Summary(job))
            .map_err(read_send_error)?;

        reply_rx
            .await
            .context("beads db reader stopped before replying")?
    }

    pub(crate) fn request_checkpoint(&self) {
        let Some(write_tx) = &self.write_tx else {
            return;
        };
        match write_tx.try_send(WriteMsg::Checkpoint) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tracing::debug!("beads db writer queue full; checkpoint request coalesced");
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::debug!("beads db writer stopped before checkpoint request");
            }
        }
    }
}

impl Drop for BeadsDb {
    fn drop(&mut self) {
        if let Some(write_tx) = self.write_tx.take() {
            let _ = write_tx.try_send(WriteMsg::Shutdown);
        }
        drop(self.read_tx.take());

        let mut threads = self.threads.lock().unwrap_or_else(|err| err.into_inner());
        join_threads_bounded(&mut threads);
    }
}

fn spawn_writer(
    db_path: PathBuf,
    lock_timeout_ms: u64,
    metrics: Arc<ContentionMetrics>,
    write_rx: Receiver<WriteMsg>,
    startup_timeout: Duration,
) -> anyhow::Result<DbThread> {
    let name = "beads-db-writer".to_owned();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let thread_name = name.clone();
    let join = thread::Builder::new()
        .name(thread_name.clone())
        .spawn(move || {
            let mut storage =
                match open_storage_connection(&db_path, lock_timeout_ms, metrics.as_ref()) {
                    Ok(storage) => storage,
                    Err(err) => {
                        let _ = ready_tx.send(Err(format!("{err:#}")));
                        let _ = done_tx.send(());
                        return;
                    }
                };

            let checkpoint_conn = match open_checkpoint_connection(&db_path, metrics.as_ref()) {
                Ok(conn) => conn,
                Err(err) => {
                    let _ = ready_tx.send(Err(format!("{err:#}")));
                    let _ = done_tx.send(());
                    return;
                }
            };

            let _ = ready_tx.send(Ok(()));
            writer_loop(&mut storage, &checkpoint_conn, &write_rx, metrics.as_ref());
            drop(storage);
            drop(checkpoint_conn);
            let _ = done_tx.send(());
        })
        .with_context(|| format!("failed to spawn {thread_name}"))?;

    wait_until_ready(&name, &ready_rx, startup_timeout, join, done_rx)
}

fn spawn_reader(
    index: usize,
    db_path: PathBuf,
    lock_timeout_ms: u64,
    metrics: Arc<ContentionMetrics>,
    read_rx: Arc<Mutex<Receiver<ReadMsg>>>,
    startup_timeout: Duration,
) -> anyhow::Result<DbThread> {
    let name = format!("beads-db-reader-{index}");
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let thread_name = name.clone();
    let join = thread::Builder::new()
        .name(thread_name.clone())
        .spawn(move || {
            let storage = match open_storage_connection(&db_path, lock_timeout_ms, metrics.as_ref())
            {
                Ok(storage) => storage,
                Err(err) => {
                    let _ = ready_tx.send(Err(format!("{err:#}")));
                    let _ = done_tx.send(());
                    return;
                }
            };

            // SqliteStorage keeps its connection private. Open exactly one
            // raw companion per existing reader, never per request.
            let summary_conn = match open_summary_connection(&db_path, lock_timeout_ms, &metrics) {
                Ok(conn) => conn,
                Err(err) => {
                    let _ = ready_tx.send(Err(format!("{err:#}")));
                    let _ = done_tx.send(());
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            reader_loop(&storage, &summary_conn, &read_rx);
            drop(summary_conn);
            drop(storage);
            let _ = done_tx.send(());
        })
        .with_context(|| format!("failed to spawn {thread_name}"))?;

    wait_until_ready(&name, &ready_rx, startup_timeout, join, done_rx)
}

fn wait_until_ready(
    name: &str,
    ready_rx: &Receiver<Result<(), String>>,
    startup_timeout: Duration,
    join: JoinHandle<()>,
    done_rx: Receiver<()>,
) -> anyhow::Result<DbThread> {
    match ready_rx.recv_timeout(startup_timeout) {
        Ok(Ok(())) => Ok(DbThread {
            name: name.to_owned(),
            join: Some(join),
            done_rx,
        }),
        Ok(Err(err)) => {
            let mut thread = DbThread {
                name: name.to_owned(),
                join: Some(join),
                done_rx,
            };
            thread.join_bounded();
            Err(anyhow!("{name} failed to open SQLite connection: {err}"))
        }
        Err(RecvTimeoutError::Timeout) => Err(anyhow!(
            "{name} did not finish SQLite connection warmup within {startup_timeout:?}"
        )),
        Err(RecvTimeoutError::Disconnected) => Err(anyhow!(
            "{name} exited before reporting SQLite connection warmup"
        )),
    }
}

fn writer_loop(
    storage: &mut SqliteStorage,
    checkpoint_conn: &rusqlite::Connection,
    write_rx: &Receiver<WriteMsg>,
    metrics: &ContentionMetrics,
) {
    loop {
        match write_rx.recv_timeout(CHECKPOINT_INTERVAL) {
            Ok(WriteMsg::Job(job)) => job(storage),
            Ok(WriteMsg::Checkpoint) | Err(RecvTimeoutError::Timeout) => {
                checkpoint_passive(checkpoint_conn, metrics);
            }
            Ok(WriteMsg::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn reader_loop(
    storage: &SqliteStorage,
    summary_conn: &rusqlite::Connection,
    read_rx: &Mutex<Receiver<ReadMsg>>,
) {
    loop {
        let msg = {
            let receiver = read_rx.lock().unwrap_or_else(|err| err.into_inner());
            receiver.recv()
        };

        match msg {
            Ok(ReadMsg::Job(job)) => job(storage),
            Ok(ReadMsg::Summary(job)) => job(summary_conn),
            Err(_) => break,
        }
    }
}

fn open_storage_connection(
    db_path: &Path,
    lock_timeout_ms: u64,
    metrics: &ContentionMetrics,
) -> anyhow::Result<SqliteStorage> {
    let storage = SqliteStorage::open_with_timeout(db_path, Some(lock_timeout_ms))?;
    metrics.incr_sqlite_open();
    // Spec §6: beads_rust beff256 exposes no raw-pragma method on SqliteStorage.
    // persist_wal and wal_autocheckpoint are per-connection settings, so setting
    // them through a throwaway rusqlite connection would be inert for this
    // long-lived handle. Leave them unset until upstream exposes a real hook.
    Ok(storage)
}

fn open_summary_connection(
    db_path: &Path,
    lock_timeout_ms: u64,
    metrics: &ContentionMetrics,
) -> anyhow::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    metrics.incr_sqlite_open();
    conn.busy_timeout(Duration::from_millis(lock_timeout_ms))?;
    Ok(conn)
}

fn open_checkpoint_connection(
    db_path: &Path,
    metrics: &ContentionMetrics,
) -> anyhow::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(db_path)?;
    metrics.incr_sqlite_open();
    conn.busy_timeout(Duration::ZERO)?;
    Ok(conn)
}

fn checkpoint_passive(conn: &rusqlite::Connection, metrics: &ContentionMetrics) {
    metrics.incr_checkpoint();
    let result = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });

    match result {
        Ok((busy, log_frames, checkpointed_frames)) => {
            tracing::debug!(
                busy,
                log_frames,
                checkpointed_frames,
                "beads db passive WAL checkpoint complete"
            );
        }
        Err(err) => {
            tracing::debug!(error = %err, "beads db passive WAL checkpoint failed");
        }
    }
}

fn startup_timeout(lock_timeout_ms: u64) -> Duration {
    Duration::from_millis(lock_timeout_ms)
        .saturating_add(Duration::from_secs(5))
        .max(MIN_STARTUP_TIMEOUT)
}

fn write_send_error(err: TrySendError<WriteMsg>) -> anyhow::Error {
    match err {
        TrySendError::Full(_) => anyhow!("beads db writer queue is full"),
        TrySendError::Disconnected(_) => anyhow!("beads db writer is not running"),
    }
}

fn read_send_error(err: TrySendError<ReadMsg>) -> anyhow::Error {
    match err {
        TrySendError::Full(_) => anyhow!("beads db reader queue is full"),
        TrySendError::Disconnected(_) => anyhow!("beads db readers are not running"),
    }
}

fn join_threads_bounded(threads: &mut [DbThread]) {
    for thread in threads {
        thread.join_bounded();
    }
}

impl DbThread {
    fn join_bounded(&mut self) {
        let Some(join) = self.join.take() else {
            return;
        };

        match self.done_rx.recv_timeout(JOIN_TIMEOUT) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                if join.join().is_err() {
                    tracing::debug!(thread = %self.name, "beads db thread panicked during shutdown");
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                tracing::warn!(
                    thread = %self.name,
                    timeout_ms = JOIN_TIMEOUT.as_millis() as u64,
                    "timed out joining beads db thread; detaching"
                );
                drop(join);
            }
        }
    }
}

#[cfg(test)]
pub(super) mod trace_tests {
    use std::collections::BTreeMap;
    use std::sync::Barrier;

    use tracing::instrument::WithSubscriber;
    use tracing::Instrument;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    use super::*;

    #[derive(Clone, Debug, Default)]
    pub(crate) struct Fields(pub BTreeMap<String, String>);

    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    #[derive(Clone, Debug)]
    pub(crate) struct TraceEvent {
        pub fields: Fields,
        pub scope: Vec<(String, Fields)>,
    }

    #[derive(Clone, Default)]
    pub(crate) struct Capture(pub Arc<Mutex<Vec<TraceEvent>>>);

    impl Capture {
        pub fn dispatch(&self) -> tracing::Dispatch {
            tracing::Dispatch::new(Registry::default().with(self.clone()))
        }

        pub fn events(&self) -> Vec<TraceEvent> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Layer<Registry> for Capture {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            id: &tracing::span::Id,
            ctx: Context<'_, Registry>,
        ) {
            let mut fields = Fields::default();
            attrs.record(&mut fields);
            ctx.span(id).unwrap().extensions_mut().insert(fields);
        }

        fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, Registry>) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            let scope = ctx
                .event_scope(event)
                .into_iter()
                .flat_map(|scope| scope.from_root())
                .map(|span| {
                    (
                        span.name().to_owned(),
                        span.extensions().get::<Fields>().unwrap().clone(),
                    )
                })
                .collect();
            self.0.lock().unwrap().push(TraceEvent { fields, scope });
        }
    }

    #[tokio::test]
    async fn submit_read_preserves_parent_span_and_dispatcher() {
        let dir = tempfile::TempDir::new().unwrap();
        let db =
            BeadsDb::spawn_with_metrics(dir.path().to_owned(), 5_000, 1, Arc::default()).unwrap();
        let capture = Capture::default();
        let dispatch = capture.dispatch();
        let caller = tracing::dispatcher::with_default(&dispatch, || {
            tracing::info_span!("ui_refresh", caller = "ui")
        });
        let expected = caller.id();
        let actual = db
            .submit_read(|_| {
                tracing::info!(site = "reader_body");
                Ok(tracing::Span::current().id())
            })
            .instrument(caller)
            .with_subscriber(dispatch)
            .await
            .unwrap();
        assert_eq!(
            actual, expected,
            "submit_read lost the submitting parent span"
        );
        let events = capture.events();
        let event = events
            .iter()
            .find(|e| e.fields.0.contains_key("site"))
            .unwrap();
        assert_eq!(event.scope[0].0, "ui_refresh");
        assert_eq!(event.scope[0].1 .0["caller"], "\"ui\"");
    }

    #[tokio::test]
    async fn submit_read_concurrent_readers_do_not_leak_spans() {
        let dir = tempfile::TempDir::new().unwrap();
        let db =
            BeadsDb::spawn_with_metrics(dir.path().to_owned(), 5_000, 2, Arc::default()).unwrap();
        let dispatch = Capture::default().dispatch();
        let (left, right) = tracing::dispatcher::with_default(&dispatch, || {
            (tracing::info_span!("left"), tracing::info_span!("right"))
        });
        let expected = (left.id(), right.id());
        let barrier = Arc::new(Barrier::new(2));
        let read = |barrier: Arc<Barrier>| {
            move |_: &SqliteStorage| {
                let before = tracing::Span::current().id();
                barrier.wait();
                Ok((before, tracing::Span::current().id()))
            }
        };
        let (a, b) = tokio::join!(
            db.submit_read(read(Arc::clone(&barrier)))
                .instrument(left)
                .with_subscriber(dispatch.clone()),
            db.submit_read(read(barrier))
                .instrument(right)
                .with_subscriber(dispatch.clone()),
        );
        assert_eq!(a.unwrap(), (expected.0.clone(), expected.0));
        assert_eq!(b.unwrap(), (expected.1.clone(), expected.1));
        assert_eq!(
            db.submit_read(|_| Ok(tracing::Span::current().id()))
                .with_subscriber(dispatch)
                .await
                .unwrap(),
            None,
        );
    }

    #[tokio::test]
    async fn submit_read_error_and_cancelled_reply_release_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(
            BeadsDb::spawn_with_metrics(dir.path().to_owned(), 5_000, 1, Arc::default()).unwrap(),
        );
        let capture = Capture::default();
        let dispatch = capture.dispatch();
        let caller = tracing::dispatcher::with_default(&dispatch, || tracing::info_span!("caller"));
        let error = db
            .submit_read::<(), _>(|_| {
                tracing::info!(site = "error_body");
                anyhow::bail!("original read error")
            })
            .instrument(caller.clone())
            .with_subscriber(dispatch.clone())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "original read error");

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_db = Arc::clone(&db);
        let task = tokio::spawn(
            async move {
                worker_db
                    .submit_read(move |_| {
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        tracing::info!(site = "cancelled_body");
                        Ok(())
                    })
                    .await
            }
            .instrument(caller)
            .with_subscriber(dispatch.clone()),
        );
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        let current = db
            .submit_read(|_| Ok(tracing::Span::current().id()))
            .with_subscriber(dispatch)
            .await
            .unwrap();
        assert_eq!(current, None, "reader retained the previous job's span");
        for site in ["error_body", "cancelled_body"] {
            let events = capture.events();
            let event = events
                .iter()
                .find(|e| e.fields.0.get("site") == Some(&format!("{site:?}")))
                .expect("reader event must use the submitting dispatcher");
            assert_eq!(event.scope[0].0, "caller");
        }
    }

    #[tokio::test]
    async fn submit_read_preserves_backpressure_and_reply_errors() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.try_send(ReadMsg::Job(Box::new(|_| {}))).unwrap();
        let mut db = BeadsDb {
            write_tx: None,
            read_tx: Some(tx),
            threads: Mutex::new(Vec::new()),
        };
        let error = db.submit_read(|_| Ok(())).await.unwrap_err();
        assert_eq!(error.to_string(), "beads db reader queue is full");
        drop(rx);
        let error = db.submit_read(|_| Ok(())).await.unwrap_err();
        assert_eq!(error.to_string(), "beads db readers are not running");
        db.read_tx = None;
        let error = db.submit_read(|_| Ok(())).await.unwrap_err();
        assert_eq!(error.to_string(), "beads db readers are not running");

        let (tx, rx) = mpsc::sync_channel(1);
        db.read_tx = Some(tx);
        let read = db.submit_read(|_| Ok(()));
        tokio::pin!(read);
        tokio::select! {
            biased;
            result = &mut read => panic!("read should wait for reply: {result:?}"),
            () = std::future::ready(()) => {},
        }
        drop(rx.recv().unwrap());
        let error = read.await.unwrap_err();
        assert_eq!(error.to_string(), "beads db reader stopped before replying");
    }
}
