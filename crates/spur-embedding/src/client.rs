//! Native-free process supervision for the embedding wire protocol.
use crate::{
    read_frame, write_frame, ErrorCode, ModelId, ModelIdentity, ProtocolLimits, Reply,
    ReplyEnvelope, Request, RequestEnvelope, Session, PROTOCOL_VERSION,
};
use std::{
    ffi::OsString,
    io::{Read as _, Write as _},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex, Weak,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ClientPolicy {
    pub queue_capacity: usize,
    pub max_restarts: usize,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
    pub shutdown_timeout: Duration,
    /// Caller-selected maximum delay between child exit-status observations.
    pub shutdown_poll_interval: Duration,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("invalid policy: {0}")]
    InvalidPolicy(String),
    #[error("deadline exceeded")]
    Timeout,
    #[error("cancelled")]
    Cancelled,
    #[error("request queue full")]
    QueueFull,
    #[error("client closed")]
    Closed,
    #[error("restart budget exhausted")]
    RestartExhausted,
    #[error("process or protocol failure: {0}")]
    Transport(String),
    #[error("remote error {code:?}: {message}")]
    Remote { code: ErrorCode, message: String },
}

#[derive(Default)]
struct Gate {
    active: bool,
    queued: usize,
    closed: bool,
    force_close: bool,
}
#[derive(Default)]
struct Wake {
    gate: Mutex<Gate>,
    cv: Condvar,
}
#[derive(Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    listeners: Mutex<Vec<Weak<Wake>>>,
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<CancellationInner>);
impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        let mut listeners = self.0.listeners.lock().unwrap();
        listeners.retain(|w| {
            if let Some(wake) = w.upgrade() {
                let _guard = wake.gate.lock().unwrap();
                wake.cv.notify_all();
                true
            } else {
                false
            }
        });
    }
    fn cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    fn register(&self, wake: &Arc<Wake>) {
        let mut listeners = self.0.listeners.lock().unwrap();
        listeners.retain(|w| w.strong_count() > 0);
        if !listeners.iter().any(|w| w.ptr_eq(&Arc::downgrade(wake))) {
            listeners.push(Arc::downgrade(wake));
        }
    }
}
struct Active(Arc<Wake>);
impl Drop for Active {
    fn drop(&mut self) {
        let mut gate = self.0.gate.lock().unwrap();
        gate.active = false;
        self.0.cv.notify_all();
    }
}
struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    session: Session,
    generation: Uuid,
    next_id: u64,
}
struct State {
    worker: Option<Worker>,
    attempted: bool,
    restarts_left: usize,
}
#[derive(Default)]
struct Launch {
    abandoned: bool,
    result: Option<Result<Child, String>>,
}
pub struct EmbeddingClient {
    program: OsString,
    args: Vec<OsString>,
    identity: ModelIdentity,
    limits: ProtocolLimits,
    policy: ClientPolicy,
    wake: Arc<Wake>,
    state: Arc<Mutex<State>>,
    trace: Arc<Trace>,
}

type IoResult = (Result<ReplyEnvelope, String>, ChildStdin, ChildStdout);

#[cfg(feature = "test-support")]
#[derive(Clone, serde::Serialize)]
pub struct TestEvent {
    pub kind: &'static str,
    pub elapsed_nanos: u128,
    pub pid: Option<u32>,
    pub generation: Option<String>,
    pub request_id: Option<u64>,
    pub status: Option<String>,
}
#[cfg(feature = "test-support")]
struct Trace {
    started: Instant,
    events: Mutex<Vec<TestEvent>>,
    delivery_hold: Mutex<Option<Duration>>,
    spawn_hold: Mutex<Option<Duration>>,
    validation_hold: Mutex<Option<Duration>>,
}
#[cfg(feature = "test-support")]
impl Trace {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            events: Mutex::new(Vec::new()),
            delivery_hold: Mutex::new(None),
            spawn_hold: Mutex::new(None),
            validation_hold: Mutex::new(None),
        }
    }
    fn hold_delivery(&self) {
        if let Some(duration) = self.delivery_hold.lock().unwrap().take() {
            thread::sleep(duration);
        }
    }
    fn hold_spawn(&self) {
        if let Some(duration) = self.spawn_hold.lock().unwrap().take() {
            thread::sleep(duration);
        }
    }
    fn hold_validation(&self) {
        if let Some(duration) = self.validation_hold.lock().unwrap().take() {
            thread::sleep(duration);
        }
    }
    fn record(
        &self,
        kind: &'static str,
        pid: Option<u32>,
        generation: Option<Uuid>,
        request_id: Option<u64>,
        status: Option<String>,
    ) {
        self.events.lock().unwrap().push(TestEvent {
            kind,
            elapsed_nanos: self.started.elapsed().as_nanos(),
            pid,
            generation: generation.map(|g| g.to_string()),
            request_id,
            status,
        });
    }
}
#[cfg(not(feature = "test-support"))]
struct Trace;
#[cfg(not(feature = "test-support"))]
impl Trace {
    fn new() -> Self {
        Self
    }
    fn hold_delivery(&self) {}
    fn hold_spawn(&self) {}
    fn hold_validation(&self) {}
    fn record(
        &self,
        _: &'static str,
        _: Option<u32>,
        _: Option<Uuid>,
        _: Option<u64>,
        _: Option<String>,
    ) {
    }
}
impl EmbeddingClient {
    /// Discover a helper's actual artifact-bound identity before opening a session.
    /// All bounds and the deadline are supplied by the caller.
    pub fn describe_model(
        program: impl Into<OsString>,
        args: Vec<OsString>,
        model: ModelId,
        artifact_budget: u64,
        limits: ProtocolLimits,
        policy: ClientPolicy,
        caller_deadline: Instant,
    ) -> Result<ModelIdentity, ClientError> {
        Self::describe_model_cancellable(
            program,
            args,
            model,
            artifact_budget,
            limits,
            policy,
            caller_deadline,
            Cancellation::new(),
        )
    }

    /// Discover the artifact identity while observing caller cancellation.
    pub fn describe_model_cancellable(
        program: impl Into<OsString>,
        mut args: Vec<OsString>,
        model: ModelId,
        artifact_budget: u64,
        limits: ProtocolLimits,
        policy: ClientPolicy,
        caller_deadline: Instant,
        cancel: Cancellation,
    ) -> Result<ModelIdentity, ClientError> {
        let generation = Uuid::new_v4();
        let model_name = match model {
            ModelId::NomicEmbedTextV15 => "NomicEmbedTextV15",
            ModelId::CodeRankEmbed => "CodeRankEmbed",
            ModelId::JinaEmbeddingsV2BaseCode => "JinaEmbeddingsV2BaseCode",
        };
        args.extend([
            OsString::from("--describe-model"),
            OsString::from(model_name),
            OsString::from(artifact_budget.to_string()),
            OsString::from(generation.to_string()),
            OsString::from("1"),
            OsString::from(limits.max_frame_bytes.to_string()),
            OsString::from(limits.max_texts.to_string()),
            OsString::from(limits.max_text_bytes.to_string()),
            OsString::from(limits.max_total_text_bytes.to_string()),
            OsString::from(limits.max_dimensions.to_string()),
            OsString::from(limits.max_identity_bytes.to_string()),
            OsString::from(limits.max_pending.to_string()),
        ]);
        let placeholder = ModelIdentity {
            model,
            dimensions: 1,
            model_fingerprint: "-".into(),
            tokenizer_fingerprint: "-".into(),
            pooling: "-".into(),
            normalization: "-".into(),
            prefix: "-".into(),
        };
        let client = Self::new(program, args, placeholder, limits.clone(), policy.clone())?;
        let deadline = Self::effective(caller_deadline, policy.startup_timeout)?;
        cancel.register(&client.wake);
        let mut child = client.launch(generation, deadline, &cancel)?;
        drop(child.stdin.take());
        let mut stdout = child.stdout.take().expect("descriptor stdout");
        let (tx, rx) = mpsc::sync_channel(1);
        let wake = Arc::clone(&client.wake);
        let job = thread::spawn(move || {
            let result = (|| -> Result<ReplyEnvelope, String> {
                let reply = read_frame(&mut stdout, &limits).map_err(|e| e.to_string())?;
                let mut trailing = [0u8; 1];
                if stdout.read(&mut trailing).map_err(|e| e.to_string())? != 0 {
                    return Err("trailing descriptor bytes".into());
                }
                Ok(reply)
            })();
            let _guard = wake.gate.lock().unwrap();
            let _ = tx.send(result);
            wake.cv.notify_all();
        });
        let mut gate = client.wake.gate.lock().unwrap();
        let received = loop {
            if cancel.cancelled() {
                break Err(ClientError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err(ClientError::Timeout);
            }
            match rx.try_recv() {
                Ok(result) => break Ok(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(ClientError::Transport("descriptor reader stopped".into()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            gate = client.wake.cv.wait_timeout(gate, remaining).unwrap().0;
        };
        drop(gate);
        let reply = match received {
            Ok(result) => {
                let _ = job.join();
                match result {
                    Ok(reply) => reply,
                    Err(error) => {
                        Self::dispose_child(child, generation, Arc::clone(&client.trace), None);
                        return Err(ClientError::Transport(error));
                    }
                }
            }
            Err(error) => {
                Self::dispose_child(child, generation, Arc::clone(&client.trace), Some(job));
                return Err(error);
            }
        };
        loop {
            if cancel.cancelled() {
                Self::dispose_child(child, generation, Arc::clone(&client.trace), None);
                return Err(ClientError::Cancelled);
            }
            if Instant::now() >= deadline {
                Self::dispose_child(child, generation, Arc::clone(&client.trace), None);
                return Err(ClientError::Timeout);
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(status)) => {
                    return Err(ClientError::Transport(format!("descriptor exit: {status}")));
                }
                Ok(None) => {
                    let gate = client.wake.gate.lock().unwrap();
                    if !cancel.cancelled() {
                        drop(
                            client
                                .wake
                                .cv
                                .wait_timeout(
                                    gate,
                                    policy
                                        .shutdown_poll_interval
                                        .min(deadline.saturating_duration_since(Instant::now())),
                                )
                                .unwrap(),
                        );
                    }
                }
                Err(error) => {
                    Self::dispose_child(child, generation, Arc::clone(&client.trace), None);
                    return Err(ClientError::Transport(error.to_string()));
                }
            }
        }
        if cancel.cancelled() {
            return Err(ClientError::Cancelled);
        }
        if reply.version != PROTOCOL_VERSION
            || reply.generation != generation
            || reply.request_id != 1
        {
            return Err(ClientError::Transport(
                "descriptor correlation mismatch".into(),
            ));
        }
        match reply.reply {
            Reply::Ready { identity } if identity.model == model => Ok(identity),
            Reply::Error { code, message } => Err(ClientError::Remote { code, message }),
            _ => Err(ClientError::Transport("unexpected descriptor reply".into())),
        }
    }
    pub fn new(
        program: impl Into<OsString>,
        args: Vec<OsString>,
        identity: ModelIdentity,
        limits: ProtocolLimits,
        policy: ClientPolicy,
    ) -> Result<Self, ClientError> {
        identity
            .validate(&limits)
            .map_err(|e| ClientError::InvalidPolicy(e.to_string()))?;
        if policy.queue_capacity == 0
            || [
                policy.startup_timeout,
                policy.request_timeout,
                policy.shutdown_timeout,
                policy.shutdown_poll_interval,
            ]
            .iter()
            .any(|d| d.is_zero() || Instant::now().checked_add(*d).is_none())
        {
            return Err(ClientError::InvalidPolicy(
                "zero capacity/timeout or timeout overflow".into(),
            ));
        }
        let restarts_left = policy.max_restarts;
        Ok(Self {
            program: program.into(),
            args,
            identity,
            limits,
            policy,
            wake: Arc::new(Wake::default()),
            state: Arc::new(Mutex::new(State {
                worker: None,
                attempted: false,
                restarts_left,
            })),
            trace: Arc::new(Trace::new()),
        })
    }
    #[cfg(feature = "test-support")]
    pub fn test_events(&self) -> Vec<TestEvent> {
        self.trace.events.lock().unwrap().clone()
    }
    #[cfg(feature = "test-support")]
    pub fn test_queue_depth(&self) -> usize {
        self.wake.gate.lock().unwrap().queued
    }
    #[cfg(feature = "test-support")]
    pub fn test_hold_delivery(&self, delay: Duration) {
        *self.trace.delivery_hold.lock().unwrap() = Some(delay);
    }
    #[cfg(feature = "test-support")]
    pub fn test_hold_spawn(&self, delay: Duration) {
        *self.trace.spawn_hold.lock().unwrap() = Some(delay);
    }
    #[cfg(feature = "test-support")]
    pub fn test_hold_validation(&self, delay: Duration) {
        *self.trace.validation_hold.lock().unwrap() = Some(delay);
    }
    fn effective(caller: Instant, timeout: Duration) -> Result<Instant, ClientError> {
        Instant::now()
            .checked_add(timeout)
            .map(|d| d.min(caller))
            .ok_or_else(|| ClientError::InvalidPolicy("deadline overflow".into()))
    }
    fn acquire(
        &self,
        deadline: Instant,
        cancel: &Cancellation,
        shutdown: bool,
    ) -> Result<Active, ClientError> {
        cancel.register(&self.wake);
        let mut gate = self.wake.gate.lock().unwrap();
        if shutdown {
            if gate.closed {
                return Err(ClientError::Closed);
            }
            // An already-expired close must leave admission and a later close usable.
            if Instant::now() >= deadline {
                return Err(ClientError::Timeout);
            }
            gate.closed = true;
            self.wake.cv.notify_all();
        }
        if cancel.cancelled() {
            return Err(ClientError::Cancelled);
        }
        if Instant::now() >= deadline {
            if shutdown {
                gate.force_close = true;
                self.wake.cv.notify_all();
                self.cleanup_after_active();
            }
            return Err(ClientError::Timeout);
        }
        if !shutdown && gate.closed {
            return Err(ClientError::Closed);
        }
        if !gate.active {
            gate.active = true;
            return Ok(Active(Arc::clone(&self.wake)));
        }
        if !shutdown && gate.queued >= self.policy.queue_capacity {
            return Err(ClientError::QueueFull);
        }
        gate.queued = gate
            .queued
            .checked_add(1)
            .ok_or_else(|| ClientError::InvalidPolicy("queue counter overflow".into()))?;
        loop {
            if cancel.cancelled() {
                gate.queued -= 1;
                return Err(ClientError::Cancelled);
            }
            if !shutdown && gate.closed {
                gate.queued -= 1;
                return Err(ClientError::Closed);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                gate.queued -= 1;
                if shutdown {
                    // The active owner may be in launch, pipe IO, or validation.
                    // Wake it and retain independent ownership of eventual cleanup.
                    gate.force_close = true;
                    self.wake.cv.notify_all();
                    self.cleanup_after_active();
                }
                return Err(ClientError::Timeout);
            }
            if !gate.active {
                gate.queued -= 1;
                gate.active = true;
                return Ok(Active(Arc::clone(&self.wake)));
            }
            gate = self.wake.cv.wait_timeout(gate, remaining).unwrap().0;
        }
    }
    fn cleanup_after_active(&self) {
        let wake = Arc::clone(&self.wake);
        let state = Arc::clone(&self.state);
        let trace = Arc::clone(&self.trace);
        thread::spawn(move || {
            let mut gate = wake.gate.lock().unwrap();
            while gate.active {
                gate = wake.cv.wait(gate).unwrap();
            }
            drop(gate);
            if let Some(worker) = state.lock().unwrap().worker.take() {
                Self::dispose_child(worker.child, worker.generation, trace, None);
            }
        });
    }
    pub fn embed(
        &self,
        texts: Vec<String>,
        deadline: Instant,
    ) -> Result<Vec<Vec<f32>>, ClientError> {
        self.embed_cancellable(texts, deadline, Cancellation::new())
    }
    pub fn embed_cancellable(
        &self,
        texts: Vec<String>,
        caller_deadline: Instant,
        cancel: Cancellation,
    ) -> Result<Vec<Vec<f32>>, ClientError> {
        let deadline = Self::effective(caller_deadline, self.policy.request_timeout)?;
        // Validate before launching a process or consuming restart budget.
        RequestEnvelope {
            version: PROTOCOL_VERSION,
            generation: Uuid::new_v4(),
            request_id: 1,
            request: Request::Embed {
                identity: self.identity.clone(),
                texts: texts.clone(),
            },
        }
        .validate(&self.limits)
        .map_err(|e| ClientError::Transport(e.to_string()))?;
        let _active = self.acquire(deadline, &cancel, false)?;
        let mut state = self.state.lock().unwrap();
        if state.worker.is_none() {
            self.start(&mut state, deadline, &cancel)?;
        }
        match self.exchange(
            &mut state.worker,
            Request::Embed {
                identity: self.identity.clone(),
                texts,
            },
            deadline,
            &cancel,
        )? {
            Reply::Embeddings { rows, .. } => Ok(rows),
            Reply::Error { code, message } => Err(ClientError::Remote { code, message }),
            _ => Err(ClientError::Transport("unexpected reply".into())),
        }
    }
    fn start(
        &self,
        state: &mut State,
        deadline: Instant,
        cancel: &Cancellation,
    ) -> Result<(), ClientError> {
        if cancel.cancelled() {
            return Err(ClientError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(ClientError::Timeout);
        }
        let start_deadline = Self::effective(deadline, self.policy.startup_timeout)?;
        if state.attempted {
            if state.restarts_left == 0 {
                return Err(ClientError::RestartExhausted);
            }
            state.restarts_left -= 1;
        }
        state.attempted = true;
        let generation = Uuid::new_v4();
        let mut child = self.launch(generation, start_deadline, cancel)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let session = Session::new(generation, self.identity.clone(), self.limits.clone())
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        state.worker = Some(Worker {
            child,
            stdin,
            stdout,
            session,
            generation,
            next_id: 1,
        });
        if cancel.cancelled() || Instant::now() >= start_deadline {
            self.dispose(&mut state.worker, None);
            return Err(if cancel.cancelled() {
                ClientError::Cancelled
            } else {
                ClientError::Timeout
            });
        }
        match self.exchange(
            &mut state.worker,
            Request::Hello {
                identity: self.identity.clone(),
            },
            start_deadline,
            cancel,
        )? {
            Reply::Ready { .. } => Ok(()),
            Reply::Error { code, message } => {
                self.dispose(&mut state.worker, None);
                Err(ClientError::Remote { code, message })
            }
            _ => {
                self.dispose(&mut state.worker, None);
                Err(ClientError::Transport("unexpected hello reply".into()))
            }
        }
    }
    fn launch(
        &self,
        generation: Uuid,
        deadline: Instant,
        cancel: &Cancellation,
    ) -> Result<Child, ClientError> {
        let shared = Arc::new(Mutex::new(Launch::default()));
        let launch = Arc::clone(&shared);
        let wake = Arc::clone(&self.wake);
        let trace = Arc::clone(&self.trace);
        let program = self.program.clone();
        let args = self.args.clone();
        let job = thread::spawn(move || {
            trace.hold_spawn();
            {
                let _gate = wake.gate.lock().unwrap();
                if launch.lock().unwrap().abandoned {
                    return;
                }
            }
            let result = Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string());
            if let Ok(child) = &result {
                trace.record("spawned", Some(child.id()), Some(generation), None, None);
            }
            let _gate = wake.gate.lock().unwrap();
            let mut launch = launch.lock().unwrap();
            if launch.abandoned {
                if let Ok(child) = result {
                    Self::dispose_child(child, generation, Arc::clone(&trace), None);
                }
            } else {
                launch.result = Some(result);
                wake.cv.notify_all();
            }
        });
        let mut gate = self.wake.gate.lock().unwrap();
        loop {
            let error = if gate.force_close {
                Some(ClientError::Closed)
            } else if cancel.cancelled() {
                Some(ClientError::Cancelled)
            } else if Instant::now() >= deadline {
                Some(ClientError::Timeout)
            } else {
                None
            };
            let mut launch = shared.lock().unwrap();
            if let Some(error) = error {
                launch.abandoned = true;
                let result = launch.result.take();
                drop(launch);
                drop(gate);
                if let Some(Ok(child)) = result {
                    Self::dispose_child(child, generation, Arc::clone(&self.trace), None);
                }
                return Err(error);
            }
            if let Some(result) = launch.result.take() {
                drop(launch);
                drop(gate);
                let _ = job.join();
                return result.map_err(ClientError::Transport);
            }
            drop(launch);
            gate = self
                .wake
                .cv
                .wait_timeout(gate, deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .0;
        }
    }
    fn exchange(
        &self,
        worker: &mut Option<Worker>,
        request: Request,
        deadline: Instant,
        cancel: &Cancellation,
    ) -> Result<Reply, ClientError> {
        let w = worker.as_mut().expect("active worker");
        let id = w.next_id;
        w.next_id = id
            .checked_add(1)
            .ok_or_else(|| ClientError::Transport("request id exhausted".into()))?;
        let envelope = RequestEnvelope {
            version: PROTOCOL_VERSION,
            generation: w.generation,
            request_id: id,
            request,
        };
        w.session
            .validate_request(&envelope)
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        let pid = w.child.id();
        self.trace.record(
            "request",
            Some(pid),
            Some(w.generation),
            Some(id),
            Some(
                match &envelope.request {
                    Request::Hello { .. } => "hello",
                    Request::Embed { .. } => "embed",
                    Request::Shutdown => "shutdown",
                }
                .into(),
            ),
        );
        let mut input = w.stdin.take().expect("worker stdin");
        let mut output = w.stdout.take().expect("worker stdout");
        let limits = self.limits.clone();
        let sent = envelope.clone();
        let wake = Arc::clone(&self.wake);
        let (tx, rx) = mpsc::channel::<IoResult>();
        let trace = Arc::clone(&self.trace);
        let job = thread::spawn(move || {
            trace.record(
                "write_started",
                Some(pid),
                Some(sent.generation),
                Some(id),
                None,
            );
            let result = write_frame(&mut input, &sent, &limits)
                .and_then(|()| input.flush().map_err(Into::into))
                .and_then(|()| {
                    trace.record(
                        "write_completed",
                        Some(pid),
                        Some(sent.generation),
                        Some(id),
                        None,
                    );
                    read_frame(&mut output, &limits)
                })
                .map_err(|e| e.to_string());
            let _guard = wake.gate.lock().unwrap();
            if let Err(error) = tx.send((result, input, output)) {
                drop(error);
                trace.record(
                    "io_pipes_closed",
                    Some(pid),
                    Some(sent.generation),
                    Some(id),
                    None,
                );
            }
            trace.hold_delivery();
            wake.cv.notify_all();
        });
        let result = self.wait_io(&rx, deadline, cancel);
        match result {
            Ok((reply, input, output)) => {
                let _ = job.join();
                self.trace.record(
                    "io_joined",
                    Some(pid),
                    Some(envelope.generation),
                    Some(id),
                    None,
                );
                let w = worker.as_mut().unwrap();
                w.stdin = Some(input);
                w.stdout = Some(output);
                self.trace.hold_validation();
                match reply {
                    Ok(reply) => match w.session.complete_reply(&envelope, &reply) {
                        Ok(()) => {
                            let failure = if self.wake.gate.lock().unwrap().force_close {
                                Some(ClientError::Closed)
                            } else if cancel.cancelled() {
                                Some(ClientError::Cancelled)
                            } else if Instant::now() >= deadline {
                                Some(ClientError::Timeout)
                            } else {
                                None
                            };
                            if let Some(failure) = failure {
                                self.trace.record(
                                    "request_failed",
                                    Some(pid),
                                    Some(envelope.generation),
                                    Some(id),
                                    Some(failure.to_string()),
                                );
                                self.dispose(worker, None);
                                return Err(failure);
                            }
                            self.trace.record(
                                "reply_validated",
                                Some(pid),
                                Some(envelope.generation),
                                Some(id),
                                Some(
                                    match &reply.reply {
                                        Reply::Ready { .. } => "ready",
                                        Reply::Embeddings { .. } => "embeddings",
                                        Reply::Closed => "closed",
                                        Reply::Error { .. } => "error",
                                    }
                                    .into(),
                                ),
                            );
                            Ok(reply.reply)
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            self.trace.record(
                                "request_failed",
                                Some(pid),
                                Some(envelope.generation),
                                Some(id),
                                Some(msg.clone()),
                            );
                            self.dispose(worker, None);
                            Err(ClientError::Transport(msg))
                        }
                    },
                    Err(e) => {
                        self.trace.record(
                            "request_failed",
                            Some(pid),
                            Some(envelope.generation),
                            Some(id),
                            Some(e.clone()),
                        );
                        self.dispose(worker, None);
                        Err(ClientError::Transport(e))
                    }
                }
            }
            Err(e) => {
                self.trace.record(
                    "request_failed",
                    Some(pid),
                    Some(envelope.generation),
                    Some(id),
                    Some(e.to_string()),
                );
                drop(rx);
                self.dispose(worker, Some(job));
                Err(e)
            }
        }
    }
    fn wait_io(
        &self,
        rx: &mpsc::Receiver<IoResult>,
        deadline: Instant,
        cancel: &Cancellation,
    ) -> Result<IoResult, ClientError> {
        let mut gate = self.wake.gate.lock().unwrap();
        loop {
            if gate.force_close {
                return Err(ClientError::Closed);
            }
            if cancel.cancelled() {
                return Err(ClientError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ClientError::Timeout);
            }
            if let Ok(result) = rx.try_recv() {
                return Ok(result);
            }
            gate = self.wake.cv.wait_timeout(gate, remaining).unwrap().0;
        }
    }
    fn dispose(&self, worker: &mut Option<Worker>, job: Option<JoinHandle<()>>) {
        if let Some(w) = worker.take() {
            Self::dispose_child(w.child, w.generation, Arc::clone(&self.trace), job);
        }
    }
    fn dispose_child(
        mut child: Child,
        generation: Uuid,
        trace: Arc<Trace>,
        job: Option<JoinHandle<()>>,
    ) {
        let pid = child.id();
        let killed = child.kill();
        trace.record(
            "kill",
            Some(pid),
            Some(generation),
            None,
            Some(format!("{killed:?}")),
        );
        // Reap and join off the caller's critical path after terminating the process.
        thread::spawn(move || {
            let status = child.wait();
            trace.record(
                if status.is_ok() {
                    "reaped"
                } else {
                    "reap_failed"
                },
                Some(pid),
                Some(generation),
                None,
                Some(format!("{status:?}")),
            );
            if let Some(job) = job {
                let _ = job.join();
                trace.record("io_joined", Some(pid), Some(generation), None, None);
            }
        });
    }
    pub fn shutdown(&self, caller_deadline: Instant) -> Result<(), ClientError> {
        let deadline = Self::effective(caller_deadline, self.policy.shutdown_timeout)?;
        let cancel = Cancellation::new();
        let _active = self.acquire(deadline, &cancel, true)?;
        let mut state = self.state.lock().unwrap();
        if state.worker.is_none() {
            return if state.attempted {
                Err(ClientError::Transport("worker unavailable".into()))
            } else {
                Ok(())
            };
        }
        match self.exchange(&mut state.worker, Request::Shutdown, deadline, &cancel)? {
            Reply::Closed => {}
            Reply::Error { code, message } => {
                self.dispose(&mut state.worker, None);
                return Err(ClientError::Remote { code, message });
            }
            _ => {
                self.dispose(&mut state.worker, None);
                return Err(ClientError::Transport("unexpected shutdown reply".into()));
            }
        }
        let mut worker = state.worker.take().unwrap();
        drop(worker.stdin.take());
        drop(worker.stdout.take());
        loop {
            match worker.child.try_wait() {
                Ok(Some(status)) => {
                    self.trace.record(
                        "reaped",
                        Some(worker.child.id()),
                        Some(worker.generation),
                        None,
                        Some(format!("{status:?}")),
                    );
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(ClientError::Transport(format!("shutdown exit: {status}")))
                    };
                }
                Ok(None) if Instant::now() < deadline => thread::park_timeout(
                    self.policy
                        .shutdown_poll_interval
                        .min(deadline.saturating_duration_since(Instant::now())),
                ),
                Ok(None) => {
                    state.worker = Some(worker);
                    self.dispose(&mut state.worker, None);
                    return Err(ClientError::Timeout);
                }
                Err(e) => {
                    state.worker = Some(worker);
                    self.dispose(&mut state.worker, None);
                    return Err(ClientError::Transport(e.to_string()));
                }
            }
        }
    }
}
impl Drop for EmbeddingClient {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            self.dispose(&mut state.worker, None);
        }
    }
}
