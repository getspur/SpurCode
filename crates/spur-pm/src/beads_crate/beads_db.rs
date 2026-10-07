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

enum WriteMsg {
    Job(WriteJob),
    Checkpoint,
    Shutdown,
}

enum ReadMsg {
    Job(ReadJob),
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
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let job = Box::new(move |storage: &SqliteStorage| {
            let _ = reply_tx.send(f(storage));
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

            let _ = ready_tx.send(Ok(()));
            reader_loop(&storage, &read_rx);
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

fn reader_loop(storage: &SqliteStorage, read_rx: &Mutex<Receiver<ReadMsg>>) {
    loop {
        let msg = {
            let receiver = read_rx.lock().unwrap_or_else(|err| err.into_inner());
            receiver.recv()
        };

        match msg {
            Ok(ReadMsg::Job(job)) => job(storage),
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
