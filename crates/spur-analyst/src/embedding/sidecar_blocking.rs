//! Ownership of blocking callbacks supplied to the public socket service.
use std::sync::{Arc, Condvar, Mutex};

use futures::FutureExt as _;
use tokio::task::JoinHandle;

type Embedder = dyn Fn(Vec<String>) -> Result<Vec<Vec<f32>>, String> + Send + Sync;

pub(super) struct BlockingBackend {
    embedder: Arc<Embedder>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    jobs: Vec<Job>,
}

struct Job {
    handle: JoinHandle<()>,
    completion: Arc<Completion>,
}

struct Completion {
    done: Mutex<bool>,
    condvar: Condvar,
    watch: tokio::sync::watch::Sender<bool>,
}

struct CompleteOnDrop(Arc<Completion>);

impl Drop for CompleteOnDrop {
    fn drop(&mut self) {
        let mut done = self
            .0
            .done
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *done = true;
        self.0.watch.send_replace(true);
        self.0.condvar.notify_all();
    }
}

impl BlockingBackend {
    pub(super) fn new<E>(embedder: E) -> Self
    where
        E: Fn(Vec<String>) -> Result<Vec<Vec<f32>>, String> + Send + Sync + 'static,
    {
        Self {
            embedder: Arc::new(embedder),
            state: Mutex::new(State::default()),
        }
    }

    pub(super) async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return Err("embed service is shut down".into());
            }
            let mut index = 0;
            while index < state.jobs.len() {
                if state.jobs[index].handle.is_finished() {
                    let job = state.jobs.swap_remove(index);
                    if let Some(Err(error)) = job.handle.now_or_never() {
                        tracing::error!(%error, "blocking embed task failed");
                    }
                } else {
                    index += 1;
                }
            }
            let completion = Arc::new(Completion {
                done: Mutex::new(false),
                condvar: Condvar::new(),
                watch: tokio::sync::watch::channel(false).0,
            });
            let guard = CompleteOnDrop(Arc::clone(&completion));
            let embedder = Arc::clone(&self.embedder);
            let handle = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| embedder(texts)))
                        .unwrap_or_else(|_| Err("blocking embed task panicked".into()));
                let _ = tx.send(result);
            });
            state.jobs.push(Job { handle, completion });
        }
        rx.await
            .map_err(|error| format!("embed task failed: {error}"))?
    }

    pub(super) async fn close(&self) -> Result<(), String> {
        let completions = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            state
                .jobs
                .iter()
                .map(|job| Arc::clone(&job.completion))
                .collect::<Vec<_>>()
        };
        for completion in completions {
            let mut receiver = completion.watch.subscribe();
            while !*receiver.borrow_and_update() {
                receiver
                    .changed()
                    .await
                    .map_err(|error| format!("embed task tracker closed: {error}"))?;
            }
        }
        let jobs = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut state.jobs)
        };
        let mut failure = None;
        for job in jobs {
            if let Err(error) = job.handle.await {
                failure.get_or_insert_with(|| format!("embed task failed: {error}"));
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for BlockingBackend {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        for job in std::mem::take(&mut state.jobs) {
            let mut done = job
                .completion
                .done
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while !*done {
                done = job
                    .completion
                    .condvar
                    .wait(done)
                    .unwrap_or_else(|error| error.into_inner());
            }
            drop(done);
            while !job.handle.is_finished() {
                std::thread::yield_now();
            }
            if let Some(Err(error)) = job.handle.now_or_never() {
                tracing::error!(%error, "blocking embed task failed during drop");
            }
        }
    }
}
