// kill_on_drop audit (bd-arch.WTA Phase 0a, 2026-04-26):
// - pre-audit line 204 is the killpg helper. The helper itself does not wait; its
//   graceful-shutdown escalation caller pairs it with child.wait()/child.kill(),
//   but the Drop safety-net caller cannot reap. Phase 0a follow-up: keep this
//   distinction explicit when adding kill_on_drop(true).
// - Terminal kill/release/shutdown controls go to terminal_reader, which owns
//   Child and pairs termination with child.wait(), independently of pipe EOF.
// Second SIGKILL races after kill_on_drop are benign on POSIX (ESRCH/no-op).
//! `NativeAcpConnection` — drives an ACP agent subprocess over stdio using the
//! official SDK's builder/handler API (`Client.builder()…connect_with`).
//!
//! # Architecture
//!
//! Spur's high-level orchestrator runs on a multi-threaded Tokio runtime, so its
//! channels and tasks are required to be `Send`. The ACP SDK builder, on the
//! other hand, registers handler callbacks that themselves must be `Send`, but
//! the `connect_with` "command-loop" closure is allowed to be `!Send`. We keep
//! the dedicated-OS-thread + `LocalSet` shape from the previous SDK version —
//! it gives us a single-threaded execution surface for the loop's bookkeeping
//! (e.g. small `Rc<RefCell<…>>` reply slots) without needing `Send` everywhere.
//!
//! Send-safe state (cwd, terminal map) is held in `Arc<Mutex<…>>` so handlers
//! can clone it cheaply.
//!
//! # Lifecycle mapping
//!
//! | `AgentConnection` method | Behaviour |
//! |---|---|
//! | `initialize()` | Spawn the agent subprocess, build the SDK connection, send `initialize` |
//! | `new_session()` | Send `NewSessionRequest` with cwd + MCP servers to the agent |
//! | `prompt()` | Send `PromptRequest`; `SessionNotification`s flow out via the connection-scoped broadcast |
//! | `wait_for_prompt_response()` | Await the terminal `PromptResponse` or RPC error for that prompt |
//! | `cancel()` | Send `CancelNotification` via the connection |
//! | `shutdown()` | Close stdin (drop the SDK connection), SIGTERM the process group, then SIGKILL if needed |
//! | `health()` | Return cached `AgentHealth` |

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};

use async_trait::async_trait;
use futures::future::{BoxFuture, Shared};
use futures::stream::unfold;
use futures::{FutureExt, Stream};
use tokio::sync::{mpsc, oneshot};

use agent_client_protocol::schema::v1::{
    AgentNotification, AuthenticateRequest, AuthenticateResponse, CancelNotification,
    ClientCapabilities, ClientRequest, CloseSessionRequest, CloseSessionResponse, ContentBlock,
    ContentChunk, CreateElicitationRequest, CreateElicitationResponse, CreateTerminalRequest,
    CreateTerminalResponse, DeleteSessionRequest, DeleteSessionResponse, ElicitationAcceptAction,
    ElicitationAction, ElicitationCapabilities, ElicitationMode, ElicitationUrlCapabilities,
    ExtRequest, ExtResponse, FileSystemCapabilities, InitializeRequest, InitializeResponse,
    KillTerminalRequest, KillTerminalResponse, ListSessionsRequest, ListSessionsResponse,
    LoadSessionRequest, LoadSessionResponse, McpServer, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, ReadTextFileRequest, ReadTextFileResponse,
    ReleaseTerminalRequest, ReleaseTerminalResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest,
    ResumeSessionResponse, SelectedPermissionOutcome, SessionCapabilities, SessionConfigOption,
    SessionConfigValueId, SessionId, SessionModeId, SessionModeState, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    SetSessionModeRequest, SetSessionModeResponse, TerminalExitStatus, TerminalId,
    TerminalOutputRequest, TerminalOutputResponse, ToolCall, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind, Usage, UsageUpdate, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectionTo, Handled, JsonRpcMessage, UntypedMessage,
};

use crate::capability_evidence::{
    CapabilityKey, CapabilityKind, CliIdentity, EvidenceEpoch, EvidenceEpochId, EvidenceRecord,
    EvidenceSessionScope,
};
use crate::config::LogConfig;
use crate::connection::child_stderr_bridge::ChildStderrBridge;
use crate::connection::{
    AcpSessionModeSnapshot, AgentClientRequestKind, AgentClientRequestPayload, AgentConnection,
    ExtNotificationPayload,
};
use crate::error::AcpError;
use crate::spur_agent_caps::{
    build_capability_cli_identity, inject_capability_evidence_epoch,
    native_dispatch_failure_record, normalize_raw_acp_observation, operational_failure_record,
    project_semantic_probe, update_identity_from_initialize_frame, CapabilityEvidenceCompleteness,
    SemanticProbeProjection, SpurAgentCaps,
};
use crate::types::{AgentHealth, AgentKind};

const NATIVE_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const NATIVE_SHUTDOWN_POLL: std::time::Duration = std::time::Duration::from_millis(10);
const SESSION_NOTIFICATION_CAPACITY: usize = 4096;
const PENDING_NEW_SESSION_MAX_NOTIFICATIONS: usize = SESSION_NOTIFICATION_CAPACITY;
const PENDING_NEW_SESSION_MAX_BYTES: usize = 8 * 1024 * 1024;

fn forwardable_ext_notification(
    method: String,
    params: serde_json::Value,
) -> Option<ExtNotificationPayload> {
    (method != "_auth/status_update").then_some(ExtNotificationPayload { method, params })
}

type TelemetryOutcome = spur_telemetry::tier1_events::Outcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcpRequestFailure {
    Timeout,
    Error,
}

fn acp_request_outcome<T>(result: Result<T, AcpRequestFailure>) -> TelemetryOutcome {
    match result {
        Ok(_) => TelemetryOutcome::Ok,
        Err(AcpRequestFailure::Timeout) => TelemetryOutcome::Timeout,
        Err(AcpRequestFailure::Error) => TelemetryOutcome::Error,
    }
}

fn acp_request_failure_from_error(error: &impl std::fmt::Display) -> AcpRequestFailure {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("timed out") || message.contains("timeout") {
        AcpRequestFailure::Timeout
    } else {
        AcpRequestFailure::Error
    }
}

fn acp_request_duration_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u64::MAX as u128) as u64
}

fn emit_acp_request_result<T, E: std::fmt::Display>(started_at: Instant, result: &Result<T, E>) {
    let outcome = acp_request_outcome(
        result
            .as_ref()
            .map(|_| ())
            .map_err(acp_request_failure_from_error),
    );
    spur_telemetry::emit!(spur_telemetry::tier1_events::AcpRequestDuration {
        duration_ms: acp_request_duration_ms(started_at),
        outcome,
    });
}

/// Spur's canonical `ClientCapabilities` literal advertised at every
/// `initialize` call. Spec §6.2.
///
/// Declares:
/// - `fs.{read_text_file, write_text_file}` — spur honors `fs/*` requests.
/// - `terminal = true` — spur honors all `terminal/*` RPCs.
/// - `_meta.terminal_output = true` — vendor extension that unlocks
///   codex's tool-call meta tunneling (consumed in M9).
pub fn spur_client_capabilities() -> ClientCapabilities {
    let mut meta = serde_json::Map::new();
    meta.insert("terminal_output".to_string(), serde_json::Value::Bool(true));
    meta.insert(
        "jetbrains".to_string(),
        serde_json::json!({
            "air": {
                "version": 1,
                "capabilities": ["nativeSubagentSessions", "agentFileChangeReport"]
            }
        }),
    );

    ClientCapabilities::new()
        .fs(FileSystemCapabilities::new()
            .read_text_file(true)
            .write_text_file(true))
        .terminal(true)
        .elicitation(ElicitationCapabilities::new().url(ElicitationUrlCapabilities::new()))
        .meta(meta)
}

#[cfg(any(test, feature = "test-support"))]
pub fn spawn_native_worker_for_test(
    command: &str,
    args: &[&str],
) -> std::io::Result<tokio::process::Child> {
    tokio::process::Command::new(command)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
}

#[derive(Clone, Copy)]
enum RawFrameDirection {
    Sent,
    Received,
}

struct PendingAcpRequest {
    method: String,
    scope: EvidenceSessionScope,
    probe_projection: Option<SemanticProbeProjection>,
    retained_bytes: usize,
}

impl PendingAcpRequest {
    fn new(
        request_id: &str,
        method: String,
        scope: EvidenceSessionScope,
        probe_projection: Option<SemanticProbeProjection>,
    ) -> Self {
        let scope_bytes = match &scope {
            EvidenceSessionScope::Global | EvidenceSessionScope::IsolatedProbe => 0,
            EvidenceSessionScope::Session(session_id) => session_id.len(),
        };
        let retained_bytes = request_id
            .len()
            .saturating_add(method.len())
            .saturating_add(scope_bytes)
            .saturating_add(
                probe_projection
                    .as_ref()
                    .map_or(0, SemanticProbeProjection::retained_bytes),
            );
        Self {
            method,
            scope,
            probe_projection,
            retained_bytes,
        }
    }
}

struct RawAcpFrameCapture {
    identity: CliIdentity,
    records: Vec<EvidenceRecord>,
    retained_bytes: usize,
    epoch_id: u64,
    pending_requests: HashMap<String, PendingAcpRequest>,
    pending_request_bytes: usize,
    sent_buffer: Vec<u8>,
    received_buffer: Vec<u8>,
    overflowed: bool,
    initialize_succeeded: bool,
    session_succeeded: bool,
    invalidated: bool,
}

impl RawAcpFrameCapture {
    fn new(identity: CliIdentity) -> Self {
        Self {
            identity,
            records: Vec::new(),
            retained_bytes: 0,
            epoch_id: 0,
            pending_requests: HashMap::new(),
            pending_request_bytes: 0,
            sent_buffer: Vec::new(),
            received_buffer: Vec::new(),
            overflowed: false,
            initialize_succeeded: false,
            session_succeeded: false,
            invalidated: false,
        }
    }

    fn feed(&mut self, direction: RawFrameDirection, bytes: &[u8]) {
        let buffer = match direction {
            RawFrameDirection::Sent => &mut self.sent_buffer,
            RawFrameDirection::Received => &mut self.received_buffer,
        };
        buffer.extend_from_slice(bytes);
        if buffer.len() > PENDING_NEW_SESSION_MAX_BYTES {
            buffer.clear();
            self.overflowed = true;
            let identity = self.identity.clone();
            self.append_records(vec![operational_failure_record(
                &identity,
                "wire",
                "malformed ACP frame exceeded the bounded capture size",
                EvidenceSessionScope::Global,
            )]);
            return;
        }

        let mut lines = Vec::new();
        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = buffer.drain(..=newline).collect::<Vec<_>>();
            while line
                .last()
                .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
            {
                line.pop();
            }
            if !line.is_empty() {
                lines.push(line);
            }
        }
        for line in lines {
            self.process_line(direction, &line);
        }
    }

    fn process_line(&mut self, direction: RawFrameDirection, line: &[u8]) {
        let message = match serde_json::from_slice::<serde_json::Value>(line) {
            Ok(message) => message,
            Err(error) => {
                self.invalidated = true;
                let identity = self.identity.clone();
                self.append_records(vec![operational_failure_record(
                    &identity,
                    "wire",
                    &format!("malformed ACP JSON: {error}"),
                    EvidenceSessionScope::Global,
                )]);
                return;
            }
        };
        match direction {
            RawFrameDirection::Sent => self.observe_sent(&message),
            RawFrameDirection::Received => self.observe_received(&message),
        }
    }

    fn observe_sent(&mut self, message: &serde_json::Value) {
        let Some(method) = message.get("method").and_then(serde_json::Value::as_str) else {
            return;
        };
        let Some(id) = message.get("id").and_then(json_rpc_id) else {
            return;
        };
        let scope = evidence_scope(message.get("params"));
        let pending = PendingAcpRequest::new(
            &id,
            method.to_owned(),
            scope,
            project_semantic_probe(method, message.get("params")),
        );
        if let Some(previous) = self.pending_requests.remove(&id) {
            self.pending_request_bytes = self
                .pending_request_bytes
                .saturating_sub(previous.retained_bytes);
        }
        if self.pending_requests.len() == SESSION_NOTIFICATION_CAPACITY
            || self
                .pending_request_bytes
                .saturating_add(pending.retained_bytes)
                > PENDING_NEW_SESSION_MAX_BYTES
        {
            self.overflowed = true;
            return;
        }
        self.pending_request_bytes = self
            .pending_request_bytes
            .saturating_add(pending.retained_bytes);
        self.pending_requests.insert(id, pending);
    }

    fn observe_received(&mut self, message: &serde_json::Value) {
        if let Some(method) = message.get("method").and_then(serde_json::Value::as_str) {
            let scope = evidence_scope(message.get("params"));
            let identity = self.identity.clone();
            let records = normalize_raw_acp_observation(&identity, method, message, scope, None);
            self.append_records(records);
            return;
        }

        let Some(id) = message.get("id").and_then(json_rpc_id) else {
            return;
        };
        let Some(pending) = self.pending_requests.remove(&id) else {
            return;
        };
        self.pending_request_bytes = self
            .pending_request_bytes
            .saturating_sub(pending.retained_bytes);
        let PendingAcpRequest {
            method,
            scope: pending_scope,
            probe_projection,
            ..
        } = pending;
        let has_error = message.get("error").is_some_and(|error| !error.is_null());
        let has_result = message.get("result").is_some();
        let succeeded = has_result && !has_error;
        if !has_result && !has_error {
            self.invalidated = true;
        }
        if has_error
            && matches!(
                method.as_str(),
                "initialize" | "session/new" | "session/load" | "authenticate"
            )
        {
            self.invalidated = true;
        }
        if method == "initialize" && succeeded {
            update_identity_from_initialize_frame(&mut self.identity, message);
            for record in &mut self.records {
                record.identity.clone_from(&self.identity);
            }
        }
        let scope = evidence_scope(message.get("result")).or_session(pending_scope);
        let identity = self.identity.clone();
        let records = normalize_raw_acp_observation(
            &identity,
            &method,
            message,
            scope,
            probe_projection.as_ref(),
        );
        self.append_records(records);
        if succeeded {
            match method.as_str() {
                "initialize" => self.initialize_succeeded = true,
                "session/new" | "session/load" if self.initialize_succeeded => {
                    self.session_succeeded = true;
                }
                _ => {}
            }
        }
    }

    fn append_records(&mut self, records: Vec<EvidenceRecord>) {
        let before = self.records.len();
        for record in records {
            if self.records.len() == SESSION_NOTIFICATION_CAPACITY {
                self.overflowed = true;
                break;
            }
            let retained_bytes = evidence_record_retained_bytes(&record);
            if self.retained_bytes.saturating_add(retained_bytes) > PENDING_NEW_SESSION_MAX_BYTES {
                self.overflowed = true;
                continue;
            }
            self.retained_bytes = self.retained_bytes.saturating_add(retained_bytes);
            self.records.push(record);
        }
        if self.records.len() != before {
            self.epoch_id = self.epoch_id.saturating_add(1);
        }
    }

    fn epoch(&self) -> Option<EvidenceEpoch> {
        EvidenceEpoch::new(
            EvidenceEpochId(self.epoch_id),
            self.identity.clone(),
            self.records.clone(),
        )
        .ok()
    }

    fn completeness(&self) -> CapabilityEvidenceCompleteness {
        if self.initialize_succeeded
            && self.session_succeeded
            && !self.invalidated
            && !self.overflowed
            && self.pending_requests.is_empty()
            && self.pending_request_bytes == 0
            && self.sent_buffer.is_empty()
            && self.received_buffer.is_empty()
        {
            CapabilityEvidenceCompleteness::Complete
        } else {
            CapabilityEvidenceCompleteness::Incomplete
        }
    }
}

#[cfg(test)]
mod raw_acp_frame_capture_tests {
    use super::{CapabilityEvidenceCompleteness, RawAcpFrameCapture, RawFrameDirection};
    use crate::capability_evidence::{
        reduce_capability, CapabilityKey, CapabilityKind, CliIdentity, DispatchRoute,
        EvidenceClaim, EvidenceProvenance, EvidenceSessionScope,
    };

    fn feed(
        capture: &mut RawAcpFrameCapture,
        direction: RawFrameDirection,
        value: serde_json::Value,
    ) {
        let mut bytes = serde_json::to_vec(&value).expect("test frame must serialize");
        bytes.push(b'\n');
        capture.feed(direction, &bytes);
    }

    fn initialized_capture() -> RawAcpFrameCapture {
        let mut capture = RawAcpFrameCapture::new(CliIdentity {
            resolved_executable: "/usr/bin/test-acp".into(),
            upstream_version: None,
            argv_fingerprint: "sha256:argv".to_owned(),
            environment_fingerprint: "sha256:env".to_owned(),
        });
        feed(
            &mut capture,
            RawFrameDirection::Sent,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "initialize",
                "method": "initialize",
                "params": {"protocolVersion": 1}
            }),
        );
        feed(
            &mut capture,
            RawFrameDirection::Received,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "initialize",
                "result": {"agentInfo": {"version": "1.0.13"}}
            }),
        );
        feed(
            &mut capture,
            RawFrameDirection::Sent,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "session-new",
                "method": "session/new",
                "params": {}
            }),
        );
        feed(
            &mut capture,
            RawFrameDirection::Received,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "session-new",
                "result": {
                    "sessionId": "session-1",
                    "models": {
                        "availableModels": [{"modelId": "grok-4.5"}],
                        "currentModelId": "grok-4.5"
                    }
                }
            }),
        );
        capture
    }

    fn feed_request_response(
        capture: &mut RawAcpFrameCapture,
        id: &str,
        method: &str,
        params: serde_json::Value,
        response: serde_json::Value,
    ) {
        feed(
            capture,
            RawFrameDirection::Sent,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }),
        );
        let mut response = response;
        response["jsonrpc"] = serde_json::Value::String("2.0".to_owned());
        response["id"] = serde_json::Value::String(id.to_owned());
        feed(capture, RawFrameDirection::Received, response);
    }

    #[test]
    fn paired_set_model_success_promotes_advertisement_to_native_preferred() {
        let mut capture = initialized_capture();
        let key = CapabilityKey {
            kind: CapabilityKind::Model,
            upstream_id: "model".to_owned(),
        };
        let advertised_epoch = capture.epoch().expect("advertisement epoch");
        assert_eq!(
            reduce_capability(&advertised_epoch, advertised_epoch.identity(), &key).route,
            DispatchRoute::PromptOnly,
            "vendor advertisement alone must stay prompt-only"
        );

        feed_request_response(
            &mut capture,
            "set-model",
            "session/set_model",
            serde_json::json!({
                "sessionId": "session-1",
                "modelId": "grok-4.5"
            }),
            serde_json::json!({"result": {}}),
        );

        let epoch = capture.epoch().expect("accepted-probe epoch");
        let reduced = reduce_capability(&epoch, epoch.identity(), &key);
        assert_eq!(reduced.route, DispatchRoute::NativePreferred);
        assert_eq!(
            reduced
                .choices
                .iter()
                .map(|choice| choice.id.as_str())
                .collect::<Vec<_>>(),
            ["grok-4.5"]
        );
        let verified = epoch
            .records()
            .iter()
            .find(|record| {
                record.key == key
                    && record.claim == EvidenceClaim::NativeVerified
                    && record.provenance == EvidenceProvenance::AcceptedActiveProbe
            })
            .expect("paired success must emit semantic evidence");
        assert_eq!(
            verified.session_scope,
            EvidenceSessionScope::Session("session-1".to_owned())
        );
    }

    #[test]
    fn paired_set_mode_and_known_config_success_bind_selected_choices() {
        let cases = [
            (
                "session/set_mode",
                serde_json::json!({"session_id": "session-1", "mode_id": "architect"}),
                CapabilityKind::Mode,
                "mode",
                "architect",
            ),
            (
                "session/set_config_option",
                serde_json::json!({
                    "sessionId": "session-1",
                    "configId": "reasoning_effort",
                    "value": "high"
                }),
                CapabilityKind::Effort,
                "reasoning_effort",
                "high",
            ),
        ];

        for (index, (method, params, kind, upstream_id, choice_id)) in cases.into_iter().enumerate()
        {
            let mut capture = initialized_capture();
            feed_request_response(
                &mut capture,
                &format!("set-{index}"),
                method,
                params,
                serde_json::json!({"result": null}),
            );
            let epoch = capture.epoch().expect("accepted-probe epoch");
            let record = epoch
                .records()
                .iter()
                .find(|record| {
                    record.key.kind == kind
                        && record.key.upstream_id == upstream_id
                        && record.claim == EvidenceClaim::NativeVerified
                        && record.provenance == EvidenceProvenance::AcceptedActiveProbe
                })
                .expect("paired success must bind the semantic capability");
            assert_eq!(
                record
                    .choices
                    .iter()
                    .map(|choice| choice.id.as_str())
                    .collect::<Vec<_>>(),
                [choice_id]
            );
        }
    }

    #[test]
    fn rejected_set_model_is_semantic_but_options_method_rejection_is_not() {
        let mut capture = initialized_capture();
        feed_request_response(
            &mut capture,
            "set-model",
            "session/set_model",
            serde_json::json!({"sessionId": "session-1", "modelId": "grok-4.5"}),
            serde_json::json!({
                "error": {"code": -32602, "message": "Invalid params"}
            }),
        );
        feed_request_response(
            &mut capture,
            "options",
            "vendor/optionsMethod",
            serde_json::json!({"modelId": "grok-4.5"}),
            serde_json::json!({
                "error": {"code": -32601, "message": "Method not found"}
            }),
        );

        let epoch = capture.epoch().expect("rejected-probe epoch");
        let model_rejections = epoch
            .records()
            .iter()
            .filter(|record| {
                record.key.kind == CapabilityKind::Model
                    && record.claim == EvidenceClaim::Rejected
                    && record.provenance == EvidenceProvenance::RejectedActiveProbe
            })
            .collect::<Vec<_>>();
        assert_eq!(model_rejections.len(), 1);
        assert_eq!(model_rejections[0].key.upstream_id, "model");
    }

    #[test]
    fn paired_response_without_result_invalidates_capture_without_native_evidence() {
        let mut capture = initialized_capture();
        feed_request_response(
            &mut capture,
            "set-model",
            "session/set_model",
            serde_json::json!({"sessionId": "session-1", "modelId": "grok-4.5"}),
            serde_json::json!({}),
        );

        assert_eq!(
            capture.completeness(),
            CapabilityEvidenceCompleteness::Incomplete
        );
        assert!(!capture.records.iter().any(|record| {
            record.key.kind == CapabilityKind::Model
                && record.claim == EvidenceClaim::NativeVerified
        }));
    }
}

fn evidence_record_retained_bytes(record: &EvidenceRecord) -> usize {
    record
        .key
        .upstream_id
        .len()
        .saturating_add(record.raw_digest.0.len())
        .saturating_add(
            record
                .choices
                .iter()
                .map(|choice| {
                    choice
                        .id
                        .len()
                        .saturating_add(choice.label.len())
                        .saturating_add(
                            choice
                                .description
                                .as_ref()
                                .map_or(0, std::string::String::len),
                        )
                        .saturating_add(64)
                })
                .fold(0_usize, usize::saturating_add),
        )
        .saturating_add(256)
}

trait EvidenceScopeExt {
    fn or_session(self, fallback: EvidenceSessionScope) -> EvidenceSessionScope;
}

impl EvidenceScopeExt for EvidenceSessionScope {
    fn or_session(self, fallback: EvidenceSessionScope) -> EvidenceSessionScope {
        match self {
            EvidenceSessionScope::Global => fallback,
            session => session,
        }
    }
}

fn evidence_scope(payload: Option<&serde_json::Value>) -> EvidenceSessionScope {
    payload
        .and_then(|payload| {
            payload
                .get("sessionId")
                .or_else(|| payload.get("session_id"))
        })
        .and_then(serde_json::Value::as_str)
        .filter(|session_id| !session_id.is_empty())
        .map_or(EvidenceSessionScope::Global, |session_id| {
            EvidenceSessionScope::Session(session_id.to_owned())
        })
}

fn json_rpc_id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(id) => Some(format!("s:{id}")),
        serde_json::Value::Number(id) => Some(format!("n:{id}")),
        _ => None,
    }
}

fn inject_latest_capability_evidence(
    meta: &mut Option<agent_client_protocol::schema::v1::Meta>,
    capture: &Arc<Mutex<RawAcpFrameCapture>>,
) {
    let snapshot = capture
        .lock()
        .ok()
        .and_then(|capture| capture.epoch().map(|epoch| (epoch, capture.completeness())));
    if let Some((epoch, completeness)) = snapshot {
        inject_capability_evidence_epoch(meta, &epoch, completeness);
    }
}

fn record_operational_capability_failure(
    capture: &Arc<Mutex<RawAcpFrameCapture>>,
    method: &str,
    error: &impl std::fmt::Display,
    scope: EvidenceSessionScope,
) {
    let Ok(mut capture) = capture.lock() else {
        return;
    };
    capture.invalidated = true;
    let identity = capture.identity.clone();
    capture.append_records(vec![operational_failure_record(
        &identity,
        method,
        &error.to_string(),
        scope,
    )]);
}

fn record_native_dispatch_failure(
    capture: &Arc<Mutex<RawAcpFrameCapture>>,
    key: CapabilityKey,
    method: &str,
    error: &impl std::fmt::Display,
    scope: EvidenceSessionScope,
) {
    let Ok(mut capture) = capture.lock() else {
        return;
    };
    let identity = capture.identity.clone();
    capture.append_records(vec![native_dispatch_failure_record(
        &identity,
        key,
        method,
        &error.to_string(),
        scope,
    )]);
}

pin_project_lite::pin_project! {
    struct EvidenceCaptureReader<R> {
        #[pin]
        inner: R,
        capture: Arc<Mutex<RawAcpFrameCapture>>,
    }
}

impl<R: AsyncRead> AsyncRead for EvidenceCaptureReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.project();
        let before = buffer.filled().len();
        match this.inner.poll_read(context, buffer) {
            Poll::Ready(Ok(())) => {
                if let Ok(mut capture) = this.capture.lock() {
                    capture.feed(RawFrameDirection::Received, &buffer.filled()[before..]);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

pin_project_lite::pin_project! {
    struct EvidenceCaptureWriter<W> {
        #[pin]
        inner: W,
        capture: Arc<Mutex<RawAcpFrameCapture>>,
    }
}

impl<W: AsyncWrite> AsyncWrite for EvidenceCaptureWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        match this.inner.poll_write(context, buffer) {
            Poll::Ready(Ok(written)) => {
                if let Ok(mut capture) = this.capture.lock() {
                    capture.feed(RawFrameDirection::Sent, &buffer[..written]);
                }
                Poll::Ready(Ok(written))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().inner.poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.project().inner.poll_shutdown(context)
    }
}

// ─── Commands sent to the dedicated ACP thread ──────────────────────────────

/// Commands sent from the main (Send) world to the dedicated !Send ACP thread.
#[expect(
    clippy::large_enum_variant,
    reason = "ACP command payloads are sent across a private channel and boxed variants make call sites noisier"
)]
enum AcpCommand {
    Initialize {
        request: InitializeRequest,
        reply: oneshot::Sender<anyhow::Result<InitializeResponse>>,
    },
    NewSession {
        request: NewSessionRequest,
        reply: oneshot::Sender<anyhow::Result<NewSessionResponse>>,
    },
    Prompt {
        request: PromptRequest,
        /// Setup reply carrying the compatibility notification stream.
        reply: oneshot::Sender<anyhow::Result<mpsc::UnboundedReceiver<SessionNotification>>>,
        /// Terminal result of the underlying `session/prompt` RPC.
        terminal_reply: oneshot::Sender<anyhow::Result<PromptResponse>>,
    },
    Cancel {
        session_id: String,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Shutdown {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    LoadSession {
        request: LoadSessionRequest,
        reply: oneshot::Sender<
            anyhow::Result<(
                LoadSessionResponse,
                mpsc::UnboundedReceiver<SessionNotification>,
            )>,
        >,
    },
    ResumeSession {
        request: ResumeSessionRequest,
        reply: oneshot::Sender<anyhow::Result<ResumeSessionResponse>>,
    },
    ListSessions {
        request: ListSessionsRequest,
        reply: oneshot::Sender<anyhow::Result<ListSessionsResponse>>,
    },
    DeleteSession {
        request: DeleteSessionRequest,
        reply: oneshot::Sender<anyhow::Result<DeleteSessionResponse>>,
    },
    CloseSession {
        request: CloseSessionRequest,
        reply: oneshot::Sender<anyhow::Result<CloseSessionResponse>>,
    },
    SetSessionMode {
        request: SetSessionModeRequest,
        reply: oneshot::Sender<anyhow::Result<SetSessionModeResponse>>,
    },
    SetSessionConfigOption {
        request: SetSessionConfigOptionRequest,
        reply: oneshot::Sender<anyhow::Result<SetSessionConfigOptionResponse>>,
    },
    Authenticate {
        request: AuthenticateRequest,
        reply: oneshot::Sender<anyhow::Result<AuthenticateResponse>>,
    },
    ExtMethod {
        request: ExtRequest,
        reply: oneshot::Sender<anyhow::Result<ExtResponse>>,
    },
}

// ─── NativeAcpConnection ────────────────────────────────────────────────────

/// A native ACP connection that wraps the official SDK's `ClientSideConnection`.
///
/// This is the "real" ACP implementation that spawns an agent subprocess and
/// communicates via the Agent Client Protocol over stdio.
///
/// Because the SDK requires `!Send` futures, the actual SDK connection lives on
/// a dedicated thread.  This struct is `Send + Sync` and communicates with that
/// thread via channels.
pub struct NativeAcpConnection {
    /// Human-readable agent name.
    agent_name: String,
    /// Wire-level adapter kind used to standardize incoming ACP events.
    agent_kind: AgentKind,
    /// Binary to invoke.
    command: String,
    /// Extra arguments passed to the binary on startup.
    extra_args: Vec<String>,
    /// Environment entries applied only to this subprocess launch.
    launch_env: BTreeMap<String, String>,
    /// Bounded, redacted evidence ledger populated by the stdio tee before
    /// the SDK projects raw ACP frames into typed responses.
    capability_evidence: Arc<Mutex<RawAcpFrameCapture>>,
    /// Additional absolute workspace roots sent with ACP `session/new`.
    additional_directories: Vec<PathBuf>,
    /// Channel to send commands to the dedicated ACP thread.
    cmd_tx: Option<mpsc::UnboundedSender<AcpCommand>>,
    /// Join handle for the dedicated thread.
    thread_handle: Option<std::thread::JoinHandle<()>>,
    /// Cached health status.
    health_status: AgentHealth,
    /// Optional sender for interactive permission requests (forwarded to the TUI).
    permission_tx: Option<mpsc::UnboundedSender<crate::types::PermissionRequest>>,
    /// Optional shared lease stamp snapshotted at permission-handler entry.
    permission_stamp: Option<std::sync::Arc<crate::types::PermissionLeaseStamp>>,
    /// Receiver for vendor-extension notifications. Filled at construction,
    /// taken once by the orchestrator via `take_ext_notification_rx`.
    ext_notification_rx: Option<mpsc::UnboundedReceiver<ExtNotificationPayload>>,
    /// Paired sender for `ext_notification_rx`, cloned into the ACP thread.
    ext_notification_tx: mpsc::UnboundedSender<ExtNotificationPayload>,
    /// Receiver for defensive agent-originated client requests.
    agent_client_request_rx: Option<mpsc::UnboundedReceiver<AgentClientRequestPayload>>,
    /// Paired sender for `agent_client_request_rx`, cloned into the ACP thread.
    agent_client_request_tx: mpsc::UnboundedSender<AgentClientRequestPayload>,
    /// Connection-scoped broadcast of session notifications. Cloned into
    /// `SpurAcpClientDynamic` (via `acp_thread_main`); subscribers obtained
    /// via `subscribe_session_notifications` live for the connection's
    /// whole lifetime — no per-turn channel swap, no grace window, no
    /// dead_tx. Capacity 1024 absorbs bursty history replay from
    /// `load_session`. Task 4 rewires `session_notification` onto this;
    /// today it's only plumbed.
    session_notif_tx: tokio::sync::broadcast::Sender<SessionNotification>,
    /// ACP session IDs whose `session/close` request completed successfully.
    closed_sessions: Arc<Mutex<HashSet<String>>>,
    /// Last ACP-visible session mode state keyed by ACP session id. Populated
    /// from `NewSessionResponse` / `LoadSessionResponse` and
    /// `SessionUpdate::CurrentModeUpdate` so policy code can gate
    /// `session/set_mode` without probing unsupported modes and diagnostics
    /// can report the current ACP mode when known.
    session_modes: Arc<Mutex<HashMap<String, AcpSessionModeSnapshot>>>,
    /// Last Grok model confirmed by a successful set call or a
    /// `model_changed` extension notification.
    /// Session LLM model ids (ACP config_option / Grok model_changed / set_model).
    /// Keyed by ACP session id. Not the coding-agent entry name.
    /// Authority: sol_0225528da3534508 P2 (generalized beyond Grok-only).
    grok_session_models: Arc<Mutex<HashMap<String, String>>>,
    /// Session effort / thought-level value ids from ACP config options.
    session_efforts: Arc<Mutex<HashMap<String, String>>>,
    /// Session lifecycle capabilities from this connection's
    /// `InitializeResponse`. `None` means initialize has not completed yet;
    /// `Some(default)` means the agent initialized but did not advertise
    /// optional lifecycle methods.
    session_capabilities: Arc<Mutex<Option<SessionCapabilities>>>,
    /// Usage from the most recently completed `session/prompt` response.
    /// Cleared when a new prompt starts and consumed by the orchestrator after
    /// `drive_prompt_notifications` observes turn completion.
    last_prompt_usage: Arc<Mutex<Option<Usage>>>,
    /// Observed `numTurns` from Grok `turn_completed.usage` for the last prompt.
    last_num_turns: Arc<Mutex<Option<u64>>>,
    /// Terminal response receiver for the most recently started prompt.
    /// Kept separate from the compatibility notification stream so native
    /// callers can distinguish RPC failure from successful stream closure.
    prompt_response_rx: Option<oneshot::Receiver<anyhow::Result<PromptResponse>>>,
    /// Process-group id of the spawned child (equal to its pid because we spawn
    /// with `setsid` / session detach, which also creates a new process group).
    /// Populated by the ACP thread after spawn, read by the graceful shutdown
    /// path and the `Drop` safety net to kill the entire descendant tree via
    /// `killpg`.
    child_pgid: Arc<Mutex<Option<i32>>>,
    /// Repo root used to resolve `.spur/pgids/<pgid>.toml` for the orphan-
    /// reaping registry. Defaults to `PathBuf::from(".")` so production
    /// callers (which run with cwd at the repo root) need no extra wiring.
    repo_root: PathBuf,
    /// Per-connection log configuration. Defaults to `LogConfig::default()`,
    /// which has `child_stderr_pipe: true` (the new file-rotate-backed
    /// stderr bridge is on by default). Tests and orchestrator wiring may
    /// override via [`Self::set_log_config`].
    log_config: LogConfig,
}

/// Compute the path where the ACP subprocess's stderr should be written.
/// Uses `.spur/logs/<agent>-<unix_ts>-<pid>-acp.log` relative to CWD.
///
/// The file is truncated when opened and the child process appends to it
/// for its lifetime — so one file per child-process spawn. Including PID
/// avoids collisions when multiple agents start in the same second.
fn build_acp_log_path(repo_root: &std::path::Path, agent_name: &str) -> std::path::PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let pid = std::process::id();
    repo_root
        .join(".spur")
        .join("logs")
        .join(format!("{agent_name}-{ts}-{pid}-acp.log"))
}

/// State-gated dispatch decision for `set_session_model`. Spec §6.3.
///
/// ACP 1.0 removed the dedicated `session/set_model` request from the typed
/// schema, but Grok and Kiro still implement the wire method. Proprietary
/// catalogs route to DirectSetModel; standard config-option agents use the
/// fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetSessionModelDispatch {
    /// Send the proven `session/set_model` request directly (Grok / Kiro).
    DirectSetModel,
    /// Fall back to `set_session_config_option` with the selected model option's ID.
    FallbackConfigOption,
    /// No model surface is advertised → `AcpError::CapabilityMissing`.
    Unsupported,
}

pub(crate) fn decide_set_session_model_dispatch(caps: &SpurAgentCaps) -> SetSessionModelDispatch {
    if caps.supports_direct_set_model() {
        SetSessionModelDispatch::DirectSetModel
    } else if caps.supports_set_model() {
        SetSessionModelDispatch::FallbackConfigOption
    } else {
        SetSessionModelDispatch::Unsupported
    }
}

fn direct_set_model_params(
    session_id: &SessionId,
    model_id: &str,
    effort_id: Option<&str>,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "sessionId": session_id.to_string(),
        "modelId": model_id,
    });
    if let Some(effort_id) = effort_id {
        params["_meta"] = serde_json::json!({"reasoningEffort": effort_id});
    }
    params
}

/// Recover the top-level `models` plane dropped by ACP schema 1.1 typed
/// deserialize, and stash it under session meta for Kiro freeze-time extract.
fn inject_recovered_models_into_meta(
    meta: &mut Option<agent_client_protocol::schema::v1::Meta>,
    raw: &serde_json::Value,
) {
    let Some(models) = raw.get("models").cloned() else {
        return;
    };
    let mut map = meta.take().unwrap_or_default();
    crate::adapter::kiro_session_display::inject_recovered_models_meta(&mut map, models);
    *meta = Some(map);
}

fn new_session_from_raw_value(raw: serde_json::Value) -> anyhow::Result<NewSessionResponse> {
    let mut typed: NewSessionResponse = serde_json::from_value(raw.clone())
        .map_err(|e| anyhow::anyhow!("failed to deserialize session/new response: {e}"))?;
    inject_recovered_models_into_meta(&mut typed.meta, &raw);
    Ok(typed)
}

fn load_session_from_raw_value(raw: serde_json::Value) -> anyhow::Result<LoadSessionResponse> {
    let mut typed: LoadSessionResponse = serde_json::from_value(raw.clone())
        .map_err(|e| anyhow::anyhow!("failed to deserialize session/load response: {e}"))?;
    inject_recovered_models_into_meta(&mut typed.meta, &raw);
    Ok(typed)
}

fn cache_grok_model_changed(
    cache: &Arc<Mutex<HashMap<String, String>>>,
    params: &serde_json::Value,
) {
    let Some(session_id) = params.get("sessionId").and_then(serde_json::Value::as_str) else {
        return;
    };
    let Some(update) = params.get("update") else {
        return;
    };
    if update
        .get("sessionUpdate")
        .and_then(serde_json::Value::as_str)
        != Some("model_changed")
    {
        return;
    }
    let Some(model_id) = update.get("model_id").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Ok(mut guard) = cache.lock() {
        guard.insert(session_id.to_owned(), model_id.to_owned());
    }
}

/// sol_55e2f7194a224bba phase B / sol_5d3dc964920d420f:
/// Fill `last_prompt_usage` from `UsageUpdate` **only** when:
/// - slot is still empty (do not clobber PromptResponse.usage), and
/// - meta carries *observed* billable token fields (not context-window `used`/`size`).
fn maybe_fill_usage_from_session_update(
    last_prompt_usage: &Arc<Mutex<Option<Usage>>>,
    update: &SessionUpdate,
) {
    let SessionUpdate::UsageUpdate(usage_update) = update else {
        return;
    };
    let Ok(mut slot) = last_prompt_usage.lock() else {
        return;
    };
    if slot.is_some() {
        // PromptResponse.usage (or a prior meta fill) wins.
        return;
    }
    let Some(usage) = usage_from_usage_update_meta(usage_update) else {
        tracing::trace!(
            used = usage_update.used,
            size = usage_update.size,
            "UsageUpdate without billable token meta; not inventing input_tokens from used/size"
        );
        return;
    };
    *slot = Some(usage);
}

/// Extract ACP [`Usage`] from UsageUpdate.meta when agents embed token counts.
///
/// Recognized keys (snake or camel): input_tokens, output_tokens,
/// cached_read_tokens / cache_read_input_tokens, cached_write_tokens /
/// cache_creation_input_tokens, total_tokens.
fn usage_from_usage_update_meta(update: &UsageUpdate) -> Option<Usage> {
    let meta = update.meta.as_ref()?;
    let input = meta_u64(meta, &["input_tokens", "inputTokens"])?;
    let output = meta_u64(meta, &["output_tokens", "outputTokens"]).unwrap_or(0);
    let cached_read = meta_u64(
        meta,
        &[
            "cached_read_tokens",
            "cachedReadTokens",
            "cache_read_input_tokens",
            "cacheReadInputTokens",
        ],
    );
    let cached_write = meta_u64(
        meta,
        &[
            "cached_write_tokens",
            "cachedWriteTokens",
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
        ],
    );
    let total = meta_u64(meta, &["total_tokens", "totalTokens"])
        .unwrap_or_else(|| input.saturating_add(output));
    let mut usage = Usage::new(total, input, output);
    if let Some(r) = cached_read {
        usage = usage.cached_read_tokens(r);
    }
    if let Some(w) = cached_write {
        usage = usage.cached_write_tokens(w);
    }
    Some(usage)
}

fn meta_u64(meta: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(v) = meta.get(*key) {
            if let Some(n) = v.as_u64() {
                return Some(n);
            }
            if let Some(n) = v.as_i64() {
                return Some(n.max(0) as u64);
            }
            if let Some(s) = v.as_str() {
                if let Ok(n) = s.parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Cache model + effort ids from ACP `config_options` (NewSession / ConfigOptionUpdate).
/// Identity: does not store empty strings. sol_0225528da3534508 P2.
fn cache_config_options_model_effort(
    models: &Arc<Mutex<HashMap<String, String>>>,
    efforts: &Arc<Mutex<HashMap<String, String>>>,
    session_id: &str,
    options: &[SessionConfigOption],
) {
    if session_id.is_empty() {
        return;
    }
    if let Some(model_id) = crate::spur_agent_caps::model_id_from_config_options(options) {
        if let Ok(mut guard) = models.lock() {
            guard.insert(session_id.to_string(), model_id);
        }
    }
    if let Some(effort_id) = crate::spur_agent_caps::effort_id_from_config_options(options) {
        if let Ok(mut guard) = efforts.lock() {
            guard.insert(session_id.to_string(), effort_id);
        }
    }
}

fn cache_session_notification_config_options(
    models: &Arc<Mutex<HashMap<String, String>>>,
    efforts: &Arc<Mutex<HashMap<String, String>>>,
    notification: &SessionNotification,
) {
    let SessionUpdate::ConfigOptionUpdate(update) = &notification.update else {
        return;
    };
    // ConfigOptionUpdate holds the full options snapshot (or deltas) as Vec.
    cache_config_options_model_effort(
        models,
        efforts,
        notification.session_id.0.as_ref(),
        &update.config_options,
    );
}

fn cache_session_modes(
    session_modes: &Arc<Mutex<HashMap<String, AcpSessionModeSnapshot>>>,
    session_id: &SessionId,
    modes: Option<&SessionModeState>,
) {
    let Some(modes) = modes else {
        return;
    };
    if let Ok(mut guard) = session_modes.lock() {
        guard.insert(
            session_id.0.to_string(),
            AcpSessionModeSnapshot::from_mode_state(modes),
        );
    }
}

fn cache_current_session_mode(
    session_modes: &Arc<Mutex<HashMap<String, AcpSessionModeSnapshot>>>,
    session_id: &SessionId,
    current_mode_id: SessionModeId,
) {
    if let Ok(mut guard) = session_modes.lock() {
        guard
            .entry(session_id.0.to_string())
            .and_modify(|snapshot| {
                snapshot.current_mode_id = Some(current_mode_id.clone());
            })
            .or_insert_with(|| AcpSessionModeSnapshot::new(Some(current_mode_id), Vec::new()));
    }
}

fn cache_session_notification_mode_update(
    session_modes: &Arc<Mutex<HashMap<String, AcpSessionModeSnapshot>>>,
    notification: &SessionNotification,
) {
    if let SessionUpdate::CurrentModeUpdate(update) = &notification.update {
        cache_current_session_mode(
            session_modes,
            &notification.session_id,
            update.current_mode_id.clone(),
        );
    }
}

fn should_forward_session_notification(
    closed_sessions: &Arc<Mutex<HashSet<String>>>,
    notification: &SessionNotification,
) -> bool {
    closed_sessions
        .lock()
        .is_ok_and(|closed| !closed.contains(notification.session_id.0.as_ref()))
}

#[derive(Debug, Clone, Copy)]
struct SessionOpenRollback {
    was_root: bool,
    was_closed: bool,
}

fn begin_session_open(
    root_sessions: &Arc<Mutex<HashSet<String>>>,
    closed_sessions: &Arc<Mutex<HashSet<String>>>,
    session_id: &str,
) -> SessionOpenRollback {
    let was_root = root_sessions
        .lock()
        .map(|mut roots| !roots.insert(session_id.to_owned()))
        .unwrap_or(false);
    let was_closed = closed_sessions
        .lock()
        .map(|mut closed| closed.remove(session_id))
        .unwrap_or(false);
    SessionOpenRollback {
        was_root,
        was_closed,
    }
}

fn rollback_session_open(
    root_sessions: &Arc<Mutex<HashSet<String>>>,
    closed_sessions: &Arc<Mutex<HashSet<String>>>,
    session_id: &str,
    rollback: SessionOpenRollback,
) {
    if !rollback.was_root {
        if let Ok(mut roots) = root_sessions.lock() {
            roots.remove(session_id);
        }
    }
    if rollback.was_closed {
        if let Ok(mut closed) = closed_sessions.lock() {
            closed.insert(session_id.to_owned());
        }
    }
}

fn is_known_session_id(
    session_id: &str,
    root_sessions: &Arc<Mutex<HashSet<String>>>,
    active_subagents: &Arc<Mutex<HashMap<String, String>>>,
) -> bool {
    root_sessions
        .lock()
        .is_ok_and(|roots| roots.contains(session_id))
        || active_subagents
            .lock()
            .is_ok_and(|active| active.contains_key(session_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionNotificationRoute {
    Forward,
    BufferUntilNewSession,
    Drop,
}

fn route_session_notification(
    notification: &SessionNotification,
    root_sessions: &Arc<Mutex<HashSet<String>>>,
    active_subagents: &Arc<Mutex<HashMap<String, String>>>,
    new_session_in_flight: bool,
) -> SessionNotificationRoute {
    if is_known_session_id(
        notification.session_id.0.as_ref(),
        root_sessions,
        active_subagents,
    ) {
        SessionNotificationRoute::Forward
    } else if new_session_in_flight {
        SessionNotificationRoute::BufferUntilNewSession
    } else {
        SessionNotificationRoute::Drop
    }
}

#[derive(Default)]
struct PendingNewSessionUpdates {
    notifications: Vec<SessionNotification>,
    bytes: usize,
    overflowed: bool,
}

type PendingNewSessionUpdateBuffer = Arc<Mutex<PendingNewSessionUpdates>>;

fn buffer_new_session_update(
    pending: &PendingNewSessionUpdateBuffer,
    notification: SessionNotification,
    encoded_bytes: usize,
) -> bool {
    let Ok(mut pending) = pending.lock() else {
        return false;
    };
    if pending.overflowed
        || pending.notifications.len() >= PENDING_NEW_SESSION_MAX_NOTIFICATIONS
        || pending
            .bytes
            .checked_add(encoded_bytes)
            .is_none_or(|bytes| bytes > PENDING_NEW_SESSION_MAX_BYTES)
    {
        pending.notifications.clear();
        pending.bytes = 0;
        pending.overflowed = true;
        return false;
    }
    pending.notifications.push(notification);
    pending.bytes += encoded_bytes;
    true
}

fn take_matching_new_session_updates(
    pending: &PendingNewSessionUpdateBuffer,
    session_id: &str,
) -> Vec<SessionNotification> {
    let Ok(mut pending) = pending.lock() else {
        return Vec::new();
    };
    if pending.overflowed {
        *pending = PendingNewSessionUpdates::default();
        return Vec::new();
    }
    let matching = pending
        .notifications
        .drain(..)
        .filter(|notification| notification.session_id.0.as_ref() == session_id)
        .collect();
    pending.bytes = 0;
    matching
}

fn clear_pending_new_session_updates(pending: &PendingNewSessionUpdateBuffer) {
    if let Ok(mut pending) = pending.lock() {
        *pending = PendingNewSessionUpdates::default();
    }
}

fn remove_subagent_descendants(
    active_subagents: &Arc<Mutex<HashMap<String, String>>>,
    root_session_id: &str,
) {
    let Ok(mut active) = active_subagents.lock() else {
        return;
    };
    let mut removed_parents = HashSet::from([root_session_id.to_owned()]);
    loop {
        let descendants = active
            .iter()
            .filter(|(_, parent)| removed_parents.contains(parent.as_str()))
            .map(|(child, _)| child.clone())
            .collect::<Vec<_>>();
        if descendants.is_empty() {
            break;
        }
        for child in descendants {
            active.remove(&child);
            removed_parents.insert(child);
        }
    }
}

/// Normalize codex-acp v1.7's temporary native-subagent update variants into
/// ordinary ACP tool lifecycle events on the immediate parent session.
///
/// The Rust ACP SDK does not yet contain the draft `subagent_spawned` and
/// `subagent_state_update` variants. Keeping the compatibility shim here lets
/// every downstream consumer continue handling provider-neutral
/// `SessionNotification`s. A child is removed on its first terminal update, so
/// duplicate or out-of-order terminal notifications cannot reopen the trace.
fn normalize_session_notification(
    raw: serde_json::Value,
    active_subagents: &Arc<Mutex<HashMap<String, String>>>,
    root_sessions: &Arc<Mutex<HashSet<String>>>,
    closed_sessions: &Arc<Mutex<HashSet<String>>>,
) -> anyhow::Result<Option<SessionNotification>> {
    let update_kind = raw
        .get("update")
        .and_then(|update| update.get("sessionUpdate"))
        .and_then(serde_json::Value::as_str);

    match update_kind {
        Some("subagent_spawned") => {
            let parent_session_id = required_string(&raw, &["sessionId"])?;
            let parent_is_open = closed_sessions
                .lock()
                .is_ok_and(|closed| !closed.contains(parent_session_id));
            if !parent_is_open
                || !is_known_session_id(parent_session_id, root_sessions, active_subagents)
            {
                return Ok(None);
            }
            let update = raw
                .get("update")
                .ok_or_else(|| anyhow::anyhow!("subagent spawn is missing update"))?;
            let child_session_id = required_string(update, &["subagentSessionId"])?;
            let name = required_string(update, &["name"])?;
            let task = required_string(update, &["task"])?;
            let capabilities = update
                .get("capabilities")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));

            let inserted = {
                let mut active = active_subagents
                    .lock()
                    .map_err(|_| anyhow::anyhow!("native subagent state lock poisoned"))?;
                if active.contains_key(child_session_id) {
                    false
                } else {
                    active.insert(child_session_id.to_owned(), parent_session_id.to_owned());
                    true
                }
            };
            if !inserted {
                return Ok(None);
            }

            let tool_call = ToolCall::new(
                subagent_tool_call_id(child_session_id),
                format!("Subagent: {name}"),
            )
            .kind(ToolKind::Other)
            .status(ToolCallStatus::InProgress)
            .raw_input(serde_json::json!({
                "task": task,
                "sessionId": child_session_id,
                "capabilities": capabilities,
            }));
            Ok(Some(SessionNotification::new(
                parent_session_id.to_owned(),
                SessionUpdate::ToolCall(tool_call),
            )))
        }
        Some("subagent_state_update") => {
            let reported_parent = required_string(&raw, &["sessionId"])?;
            let update = raw
                .get("update")
                .ok_or_else(|| anyhow::anyhow!("subagent state is missing update"))?;
            let child_session_id = required_string(update, &["subagentSessionId"])?;
            let state = required_string(update, &["state"])?;

            let parent_session_id = {
                let mut active = active_subagents
                    .lock()
                    .map_err(|_| anyhow::anyhow!("native subagent state lock poisoned"))?;
                match active.get(child_session_id) {
                    Some(parent) if parent == reported_parent => {
                        active.remove(child_session_id).expect("entry was present")
                    }
                    _ => return Ok(None),
                }
            };
            let status = match state {
                "completed" => ToolCallStatus::Completed,
                "failed" | "cancelled" | "disconnected" => ToolCallStatus::Failed,
                other => {
                    tracing::warn!(state = other, "ignoring unknown native subagent state");
                    return Ok(None);
                }
            };
            let fields = ToolCallUpdateFields::new()
                .status(status)
                .raw_output(serde_json::json!({"state": state}));
            Ok(Some(SessionNotification::new(
                parent_session_id,
                SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    subagent_tool_call_id(child_session_id),
                    fields,
                )),
            )))
        }
        _ => {
            let notification: SessionNotification =
                serde_json::from_value(raw).map_err(|error| {
                    anyhow::anyhow!("failed to deserialize session/update: {error}")
                })?;
            Ok(Some(notification))
        }
    }
}

fn required_string<'a>(value: &'a serde_json::Value, path: &[&str]) -> anyhow::Result<&'a str> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| anyhow::anyhow!("missing required field {key}"))?;
    }
    current
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "required field {} must be a non-empty string",
                path.join(".")
            )
        })
}

fn subagent_tool_call_id(session_id: &str) -> String {
    format!("subagent:{session_id}")
}

/// Add a versioned AIR file-change report request without disturbing other
/// prompt metadata. Codex returns the terminal report in a
/// `session_info_update`; correlation is enforced by
/// [`correlated_air_file_change_report`].
fn attach_air_file_change_report_request(request: &mut PromptRequest, request_id: &str) {
    let meta = request.meta.get_or_insert_default();
    let jetbrains = meta
        .entry("jetbrains".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !jetbrains.is_object() {
        *jetbrains = serde_json::json!({});
    }
    let jetbrains = jetbrains.as_object_mut().expect("object assigned above");
    let air = jetbrains
        .entry("air".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !air.is_object() {
        *air = serde_json::json!({});
    }
    air.as_object_mut().expect("object assigned above").insert(
        "agentFileChangeReportRequest".to_string(),
        serde_json::json!({"version": 1, "requestId": request_id}),
    );
}

type PendingAirReportRequests = Arc<Mutex<HashMap<String, String>>>;

fn register_pending_air_report(
    pending: &PendingAirReportRequests,
    session_id: &str,
    request_id: &str,
) {
    if let Ok(mut pending) = pending.lock() {
        pending.insert(session_id.to_owned(), request_id.to_owned());
    }
}

fn clear_pending_air_report(pending: &PendingAirReportRequests, session_id: &str) {
    if let Ok(mut pending) = pending.lock() {
        pending.remove(session_id);
    }
}

/// Return a valid AIR report only when its request id is currently pending.
/// Removing the id on success provides both request/response correlation and
/// duplicate-terminal suppression.
fn correlated_air_file_change_report(
    notification: &SessionNotification,
    pending_requests: &PendingAirReportRequests,
) -> Option<serde_json::Value> {
    let SessionUpdate::SessionInfoUpdate(info) = &notification.update else {
        return None;
    };
    let report = info
        .meta
        .as_ref()?
        .get("jetbrains")?
        .get("air")?
        .get("agentFileChangeReport")?;
    let report_object = report.as_object()?;
    if report_object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return None;
    }
    let request_id = report_object
        .get("requestId")
        .and_then(serde_json::Value::as_str)?;
    match report_object
        .get("status")
        .and_then(serde_json::Value::as_str)
    {
        Some("reported" | "unavailable") => {}
        _ => return None,
    }
    let session_id = notification.session_id.0.as_ref();
    let mut pending = pending_requests.lock().ok()?;
    let matches = pending
        .get(session_id)
        .is_some_and(|pending_id| pending_id == request_id);
    if matches {
        pending.remove(session_id);
        Some(report.clone())
    } else {
        None
    }
}

#[derive(Clone)]
struct SessionNotificationSink {
    closed_sessions: Arc<Mutex<HashSet<String>>>,
    standardizer: Arc<Mutex<crate::adapter::SessionEventStandardizer>>,
    pending_air_report_requests: PendingAirReportRequests,
    ext_notification_tx: mpsc::UnboundedSender<ExtNotificationPayload>,
    session_modes: Arc<Mutex<HashMap<String, AcpSessionModeSnapshot>>>,
    grok_session_models: Arc<Mutex<HashMap<String, String>>>,
    session_efforts: Arc<Mutex<HashMap<String, String>>>,
    last_prompt_usage: Arc<Mutex<Option<Usage>>>,
    session_notif_tx: tokio::sync::broadcast::Sender<SessionNotification>,
}

impl SessionNotificationSink {
    fn forward(&self, args: SessionNotification) {
        if !should_forward_session_notification(&self.closed_sessions, &args) {
            tracing::debug!(
                session = %args.session_id,
                "dropping ACP notification for closed session"
            );
            return;
        }
        let args = self.standardizer.lock().unwrap().standardize(args);
        if let Some(report) =
            correlated_air_file_change_report(&args, &self.pending_air_report_requests)
        {
            let _ = self.ext_notification_tx.send(ExtNotificationPayload {
                method: "_jetbrains.air/agent_file_change_report".to_string(),
                params: serde_json::json!({
                    "sessionId": args.session_id.to_string(),
                    "report": report,
                }),
            });
        }
        cache_session_notification_mode_update(&self.session_modes, &args);
        cache_session_notification_config_options(
            &self.grok_session_models,
            &self.session_efforts,
            &args,
        );
        // sol_55e2f7194a224bba phase B: when PromptResponse.usage is absent,
        // accept observed token fields from UsageUpdate.meta only.
        maybe_fill_usage_from_session_update(&self.last_prompt_usage, &args.update);
        let variant = session_update_variant_name(&args.update);
        let text_len = match &args.update {
            SessionUpdate::AgentMessageChunk(c)
            | SessionUpdate::AgentThoughtChunk(c)
            | SessionUpdate::UserMessageChunk(c) => content_chunk_text_len(c),
            _ => 0,
        };
        let session = args.session_id.to_string();
        // The orchestrator pre-subscribes before creating/loading a session.
        // An error therefore means the connection is tearing down.
        let send_result = self.session_notif_tx.send(args);
        let send_result_str = if send_result.is_ok() { "ok" } else { "err" };
        tracing::debug!(
            streaming_probe = true,
            site = "A_session_notification",
            variant,
            text_len,
            session = %session,
            send_result = send_result_str,
            "ACP session_notification (broadcast)"
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "session/new completion atomically orders all shared lifecycle caches"
)]
fn complete_new_session(
    response: &NewSessionResponse,
    root_sessions: &Arc<Mutex<HashSet<String>>>,
    closed_sessions: &Arc<Mutex<HashSet<String>>>,
    new_session_in_flight: &Arc<AtomicBool>,
    pending_updates: &PendingNewSessionUpdateBuffer,
    notification_sink: &SessionNotificationSink,
    session_modes: &Arc<Mutex<HashMap<String, AcpSessionModeSnapshot>>>,
    grok_session_models: &Arc<Mutex<HashMap<String, String>>>,
    session_efforts: &Arc<Mutex<HashMap<String, String>>>,
) {
    let session_id = response.session_id.0.as_ref();
    begin_session_open(root_sessions, closed_sessions, session_id);

    // The response is the baseline. Notifications emitted before the response
    // are temporally newer and must be replayed after these snapshots.
    cache_session_modes(session_modes, &response.session_id, response.modes.as_ref());
    if let Some(options) = &response.config_options {
        cache_config_options_model_effort(
            grok_session_models,
            session_efforts,
            session_id,
            options,
        );
    }

    new_session_in_flight.store(false, Ordering::Release);
    for notification in take_matching_new_session_updates(pending_updates, session_id) {
        notification_sink.forward(notification);
    }
}

fn session_capability_advertised(
    session_capabilities: &Arc<Mutex<Option<SessionCapabilities>>>,
    supports: impl FnOnce(&SessionCapabilities) -> bool,
) -> bool {
    let Ok(guard) = session_capabilities.lock() else {
        return true;
    };
    guard.as_ref().is_none_or(supports)
}

fn busy_in_flight_error(agent_name: &str, in_flight: &str) -> anyhow::Error {
    anyhow::anyhow!("NativeAcpConnection '{agent_name}': busy ({in_flight} in flight)")
}

/// Reply to every non-Cancel/non-Shutdown command variant with a busy error.
/// Used inside the in-flight `Prompt` and `LoadSession` select! loops to
/// reject commands the orchestrator should not be issuing while a request is
/// pending. The match is exhaustive over `AcpCommand` so a future variant
/// cannot silently sneak past the busy guard.
fn reject_busy_command(cmd: AcpCommand, agent_name: &str, in_flight: &str) {
    let err = || busy_in_flight_error(agent_name, in_flight);
    match cmd {
        AcpCommand::Initialize { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::NewSession { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::Prompt { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::Cancel { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::Shutdown { reply } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::LoadSession { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::ResumeSession { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::ListSessions { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::DeleteSession { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::CloseSession { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::SetSessionMode { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::SetSessionConfigOption { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::Authenticate { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
        AcpCommand::ExtMethod { reply, .. } => {
            let _ = reply.send(Err(err()));
        }
    }
}

/// Send `session/cancel` as a JSON-RPC notification (one-way) and reply to
/// the originating `AcpCommand::Cancel`. `cx.send_notification` is synchronous
/// and does not wait for an agent response, so this is safe to call while a
/// `Prompt` or `LoadSession` request future is in flight on the same `cx`.
fn dispatch_cancel(
    cx: &ConnectionTo<Agent>,
    session_id: String,
    reply: oneshot::Sender<anyhow::Result<()>>,
    agent_name: &str,
) {
    let cancel = CancelNotification::new(session_id);
    let result = cx.send_notification(cancel);
    let _ = reply
        .send(result.map_err(|e| {
            anyhow::anyhow!("NativeAcpConnection '{agent_name}': cancel failed: {e}")
        }));
}

fn handle_agent_client_request(
    request: UntypedMessage,
    responder: agent_client_protocol::Responder<serde_json::Value>,
    agent_client_request_tx: &mpsc::UnboundedSender<AgentClientRequestPayload>,
    agent_name: &str,
) -> agent_client_protocol::Result<
    Handled<(
        UntypedMessage,
        agent_client_protocol::Responder<serde_json::Value>,
    )>,
> {
    match request.method() {
        "logout" => {
            tracing::info!(
                agent = %agent_name,
                "NativeAcpConnection: agent requested logout"
            );
            let _ = agent_client_request_tx.send(AgentClientRequestPayload {
                kind: AgentClientRequestKind::Logout,
            });
            responder.respond(serde_json::json!({}))?;
            Ok(Handled::Yes)
        }
        "authenticate" => {
            let method_id = serde_json::from_value::<AuthenticateRequest>(request.params().clone())
                .map(|request| request.method_id.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string());
            tracing::info!(
                agent = %agent_name,
                method_id = %method_id,
                "NativeAcpConnection: agent requested authenticate, but credential forwarding is not configured"
            );
            let _ = agent_client_request_tx.send(AgentClientRequestPayload {
                kind: AgentClientRequestKind::Authenticate { method_id },
            });
            responder.respond_with_result(Err(
                agent_client_protocol::Error::internal_error().data(serde_json::json!({
                    "reason": "authenticate requested by agent, but Spur credential forwarding is not configured"
                })),
            ))?;
            Ok(Handled::Yes)
        }
        _ => Ok(Handled::No {
            message: (request, responder),
            retry: false,
        }),
    }
}

impl NativeAcpConnection {
    /// Create a new native ACP connection.
    ///
    /// `command` is the agent binary (e.g. "claude", "codex").
    /// `extra_args` are passed to the binary at spawn time.
    pub fn new(
        agent_name: impl Into<String>,
        command: impl Into<String>,
        extra_args: Vec<String>,
        permission_tx: Option<mpsc::UnboundedSender<crate::types::PermissionRequest>>,
    ) -> Self {
        let agent_name = agent_name.into();
        let agent_kind = AgentKind::from_name(&agent_name);
        Self::new_with_kind(agent_name, command, extra_args, agent_kind, permission_tx)
    }

    pub fn new_with_kind(
        agent_name: impl Into<String>,
        command: impl Into<String>,
        extra_args: Vec<String>,
        agent_kind: AgentKind,
        permission_tx: Option<mpsc::UnboundedSender<crate::types::PermissionRequest>>,
    ) -> Self {
        let command = command.into();
        let launch_env = BTreeMap::new();
        let capability_evidence = Arc::new(Mutex::new(RawAcpFrameCapture::new(
            build_capability_cli_identity(&command, &extra_args, &launch_env),
        )));
        let (ext_tx, ext_rx) = mpsc::unbounded_channel::<ExtNotificationPayload>();
        let (agent_client_request_tx, agent_client_request_rx) =
            mpsc::unbounded_channel::<AgentClientRequestPayload>();
        // Capacity 4096 per the broadcast-sizing invariant (anchor 3ff4e86):
        // bursty history replay from `load_session` can produce O(hundreds)
        // of notifications in rapid succession, and the floor was established
        // empirically under 20 workers × 80 evt/s load.
        let (session_notif_tx, _) = tokio::sync::broadcast::channel(SESSION_NOTIFICATION_CAPACITY);
        Self {
            agent_name: agent_name.into(),
            agent_kind,
            command,
            extra_args,
            launch_env,
            capability_evidence,
            additional_directories: Vec::new(),
            cmd_tx: None,
            thread_handle: None,
            health_status: AgentHealth::Unknown,
            permission_tx,
            permission_stamp: None,
            ext_notification_rx: Some(ext_rx),
            ext_notification_tx: ext_tx,
            agent_client_request_rx: Some(agent_client_request_rx),
            agent_client_request_tx,
            session_notif_tx,
            closed_sessions: Arc::new(Mutex::new(HashSet::new())),
            session_modes: Arc::new(Mutex::new(HashMap::new())),
            grok_session_models: Arc::new(Mutex::new(HashMap::new())),
            session_efforts: Arc::new(Mutex::new(HashMap::new())),
            session_capabilities: Arc::new(Mutex::new(None)),
            last_prompt_usage: Arc::new(Mutex::new(None)),
            last_num_turns: Arc::new(Mutex::new(None)),
            prompt_response_rx: None,
            child_pgid: Arc::new(Mutex::new(None)),
            repo_root: PathBuf::from("."),
            log_config: LogConfig::default(),
        }
    }

    /// Override the directory used to resolve `.spur/pgids/`. Production
    /// callers run with cwd at the repo root so the default is correct;
    /// tests use this to redirect the registry into a tempdir.
    pub fn set_repo_root(&mut self, root: PathBuf) {
        self.repo_root = root;
    }

    /// Bind the shared lease stamp snapshotted onto interactive permission requests.
    pub fn set_permission_stamp(
        &mut self,
        stamp: std::sync::Arc<crate::types::PermissionLeaseStamp>,
    ) {
        self.permission_stamp = Some(stamp);
    }

    /// Configure additional workspace roots sent on every new ACP session.
    pub fn set_additional_directories(&mut self, additional_directories: Vec<PathBuf>) {
        self.additional_directories = additional_directories;
    }

    /// Configure environment entries applied only to the spawned ACP child.
    pub fn set_launch_env(&mut self, launch_env: BTreeMap<String, String>) {
        self.launch_env = launch_env;
        if let Ok(mut capture) = self.capability_evidence.lock() {
            let upstream_version = capture.identity.upstream_version.clone();
            capture.identity =
                build_capability_cli_identity(&self.command, &self.extra_args, &self.launch_env);
            capture.identity.upstream_version = upstream_version;
            let identity = capture.identity.clone();
            for record in &mut capture.records {
                record.identity.clone_from(&identity);
            }
        }
    }

    /// Latest immutable evidence epoch, including notifications observed after
    /// session creation. Legacy facade routing remains authoritative.
    #[must_use]
    pub fn capability_evidence_epoch(&self) -> Option<EvidenceEpoch> {
        self.capability_evidence.lock().ok()?.epoch()
    }

    /// Whether the bounded raw-frame ledger dropped data after reaching a
    /// capture limit. This is diagnostic only and never changes ACP routing.
    #[must_use]
    pub fn capability_evidence_overflowed(&self) -> bool {
        self.capability_evidence
            .lock()
            .is_ok_and(|capture| capture.overflowed)
    }

    /// Override the log configuration used by the spawn site (controls the
    /// child-stderr capture mode + per-child rotation caps). Default is
    /// `LogConfig::default()`, which enables the file-rotate-backed bridge.
    pub fn set_log_config(&mut self, log_config: LogConfig) {
        self.log_config = log_config;
    }

    fn kill_subprocess_now(&self) {
        let pgid = self
            .child_pgid
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(pgid) = pgid {
            killpg(pgid, "KILL");
            let registry = crate::orphan_registry::PgidRegistry::new(
                self.repo_root.join(".spur").join("pgids"),
            );
            let _ = registry.delete(pgid);
        }
    }

    async fn wait_for_thread_until(
        handle: std::thread::JoinHandle<()>,
        deadline: tokio::time::Instant,
    ) -> bool {
        let mut handle = Some(handle);
        while let Some(h) = handle {
            if h.is_finished() {
                let _ = h.join();
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                drop(h);
                return false;
            }
            handle = Some(h);
            let sleep_for = std::cmp::min(
                NATIVE_SHUTDOWN_POLL,
                deadline.saturating_duration_since(now),
            );
            tokio::time::sleep(sleep_for).await;
        }
        true
    }
}

/// Send `signal` (e.g. `"TERM"`, `"KILL"`) to the process group `pgid`.
///
/// Benign races (ESRCH on an already-reaped group, EPERM on a recycled pgid)
/// are intentionally ignored; shutdown and Drop paths are best-effort cleanup.
#[cfg_attr(
    unix,
    expect(
        unsafe_code,
        reason = "libc::kill FFI is required for Unix process-group signal delivery"
    )
)]
fn killpg(pgid: i32, signal: &str) {
    #[cfg(unix)]
    {
        if pgid <= 0 {
            return;
        }
        let signal = match signal {
            "TERM" => libc::SIGTERM,
            "KILL" => libc::SIGKILL,
            _ => return,
        };
        // SAFETY: Negative pid targets the process group whose id is `pgid`;
        // invalid or already-reaped groups are intentionally ignored.
        let _ = unsafe { libc::kill(-pgid, signal) };
    }

    #[cfg(not(unix))]
    {
        let _ = (pgid, signal);
    }
}

impl Drop for NativeAcpConnection {
    fn drop(&mut self) {
        // Safety net for panics, runtime teardown, or `tokio::task::abort()`
        // paths that skipped `shutdown()`. Synchronous SIGKILL of the process
        // group catches the agent subprocess AND its descendants (claude-
        // agent-acp spawns `node`, which can spawn further children).
        //
        // Skip when `shutdown()` has already run — it takes `cmd_tx`, so a
        // `None` here means graceful teardown succeeded and the pgid is
        // already gone. Re-killing a reaped (possibly recycled) pgid is how
        // we leaked `kill: -NNNN: Operation not permitted` to the terminal.
        if self.cmd_tx.is_none() {
            return;
        }
        self.kill_subprocess_now();
    }
}

#[async_trait]
impl AgentConnection for NativeAcpConnection {
    // ─── initialize ─────────────────────────────────────────────────────

    async fn initialize(
        &mut self,
        mut request: InitializeRequest,
    ) -> anyhow::Result<InitializeResponse> {
        // Override the caller-supplied client_capabilities with spur's
        // canonical literal so every InitializeRequest carries the
        // explicit fs / terminal / meta.terminal_output advertisement.
        // Callers today all pass InitializeRequest::new(ProtocolVersion::LATEST)
        // (which yields ClientCapabilities::default()) — spec §6.2 requires
        // we replace those defaults with the explicit gate.
        request.client_capabilities = spur_client_capabilities();

        let agent_name = self.agent_name.clone();
        let agent_kind = self.agent_kind;
        let command = self.command.clone();
        let extra_args = self.extra_args.clone();
        let launch_env = self.launch_env.clone();

        // Create the command channel.
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<AcpCommand>();

        tracing::info!(
            agent = %agent_name,
            command = %command,
            "NativeAcpConnection: spawning agent subprocess and ACP thread"
        );

        // Spawn the dedicated thread that will own the !Send SDK connection.
        let thread_agent_name = agent_name.clone();
        let permission_tx = self.permission_tx.clone();
        let permission_stamp = self.permission_stamp.clone();
        let ext_tx = self.ext_notification_tx.clone();
        let agent_client_request_tx = self.agent_client_request_tx.clone();
        let session_notif_tx_for_thread = self.session_notif_tx.clone();
        let closed_sessions = self.closed_sessions.clone();
        let session_modes = self.session_modes.clone();
        let grok_session_models = self.grok_session_models.clone();
        let session_efforts = self.session_efforts.clone();
        let last_prompt_usage = self.last_prompt_usage.clone();
        let last_num_turns = self.last_num_turns.clone();
        let capability_evidence = self.capability_evidence.clone();
        let child_pgid = self.child_pgid.clone();
        let repo_root = self.repo_root.clone();
        let log_config = self.log_config.clone();
        let handle = std::thread::Builder::new()
            .name(format!("acp-{}", agent_name))
            .spawn(move || {
                acp_thread_main(
                    thread_agent_name,
                    agent_kind,
                    command,
                    extra_args,
                    launch_env,
                    cmd_rx,
                    permission_tx,
                    permission_stamp,
                    ext_tx,
                    agent_client_request_tx,
                    session_notif_tx_for_thread,
                    closed_sessions,
                    session_modes,
                    grok_session_models,
                    session_efforts,
                    last_prompt_usage,
                    last_num_turns,
                    capability_evidence,
                    child_pgid,
                    repo_root,
                    log_config,
                );
            })
            .map_err(|e| {
                anyhow::anyhow!(
                    "NativeAcpConnection '{}': failed to spawn ACP thread: {e}",
                    agent_name
                )
            })?;

        self.thread_handle = Some(handle);
        self.cmd_tx = Some(cmd_tx.clone());

        // Send the initialize command and wait for the response.
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::Initialize {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!(
                    "NativeAcpConnection '{}': ACP thread died before initialize",
                    self.agent_name
                )
            })?;

        let result = reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during initialize",
                self.agent_name
            )
        })??;

        // This connection uses schema::v1; generic SDK response decoding does
        // not validate that the peer negotiated a version we implement.
        if result.protocol_version != ProtocolVersion::V1 {
            let version = result.protocol_version;
            self.shutdown().await?;
            anyhow::bail!(
                "NativeAcpConnection '{}': unsupported ACP protocol version {version}; supported version is {}",
                self.agent_name,
                ProtocolVersion::V1,
            );
        }

        if let Ok(mut guard) = self.session_capabilities.lock() {
            *guard = Some(result.agent_capabilities.session_capabilities.clone());
        }

        self.health_status = AgentHealth::Ready;
        tracing::info!(
            agent = %self.agent_name,
            "NativeAcpConnection: initialized successfully"
        );

        Ok(result)
    }

    // ─── new_session ────────────────────────────────────────────────────

    async fn new_session(
        &mut self,
        cwd: PathBuf,
        mcp_servers: Vec<McpServer>,
    ) -> anyhow::Result<NewSessionResponse> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let mut request = NewSessionRequest::new(&cwd);
        request.mcp_servers = mcp_servers;
        request.additional_directories = self.additional_directories.clone();

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::NewSession {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        let result = reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during new_session",
                self.agent_name
            )
        })??;

        tracing::debug!(
            agent = %self.agent_name,
            session = %result.session_id,
            "NativeAcpConnection: session created"
        );

        Ok(result)
    }

    // ─── prompt ─────────────────────────────────────────────────────────

    async fn prompt(
        &mut self,
        request: PromptRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = SessionNotification> + Send>>> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;
        if let Ok(mut usage) = self.last_prompt_usage.lock() {
            *usage = None;
        }
        if let Ok(mut turns) = self.last_num_turns.lock() {
            *turns = None;
        }

        let session_id = request.session_id.clone();

        tracing::debug!(
            agent = %self.agent_name,
            session = %session_id,
            "NativeAcpConnection: sending prompt"
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        let (terminal_tx, terminal_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::Prompt {
                request,
                reply: reply_tx,
                terminal_reply: terminal_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        let notification_rx = reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during prompt setup",
                self.agent_name
            )
        })??;
        self.prompt_response_rx = Some(terminal_rx);

        // Wrap the unbounded receiver as a Stream.
        let stream = unfold(notification_rx, |mut rx| async move {
            rx.recv().await.map(|notif| (notif, rx))
        });

        Ok(Box::pin(stream))
    }

    async fn wait_for_prompt_response(&mut self) -> anyhow::Result<Option<PromptResponse>> {
        let response_rx = self.prompt_response_rx.take().ok_or_else(|| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': no prompt response is pending",
                self.agent_name
            )
        })?;
        let response = response_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died before prompt completed",
                self.agent_name
            )
        })??;
        // Belt-and-suspenders: also record usage here (ACP thread path does the same).
        // sol_55e2f7194a224bba primary path — PromptResponse.usage → last_prompt_usage
        if let Some(usage) = &response.usage {
            if let Ok(mut slot) = self.last_prompt_usage.lock() {
                *slot = Some(usage.clone());
            }
        }
        Ok(Some(response))
    }

    fn take_last_prompt_usage(&mut self) -> Option<Usage> {
        self.last_prompt_usage
            .lock()
            .ok()
            .and_then(|mut u| u.take())
    }

    fn take_last_num_turns(&mut self) -> Option<u64> {
        self.last_num_turns.lock().ok().and_then(|mut n| n.take())
    }

    fn session_llm_model(&self, acp_session_id: &str) -> Option<String> {
        // ACP config_option model / Grok model_changed / set_session_model.
        // Authority: sol_0225528da3534508 P2 (not Grok-only).
        self.grok_session_models
            .lock()
            .ok()
            .and_then(|guard| guard.get(acp_session_id).cloned())
    }

    fn session_effort(&self, acp_session_id: &str) -> Option<String> {
        self.session_efforts
            .lock()
            .ok()
            .and_then(|guard| guard.get(acp_session_id).cloned())
    }

    // ─── cancel ─────────────────────────────────────────────────────────

    async fn cancel(&mut self, session_id: &str) -> anyhow::Result<()> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        tracing::debug!(
            agent = %self.agent_name,
            session = %session_id,
            "NativeAcpConnection: cancelling session"
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::Cancel {
                session_id: session_id.to_string(),
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during cancel",
                self.agent_name
            )
        })?
    }

    // ─── shutdown ───────────────────────────────────────────────────────

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        tracing::info!(agent = %self.agent_name, "NativeAcpConnection: shutting down");
        let deadline = tokio::time::Instant::now() + NATIVE_SHUTDOWN_TIMEOUT;
        let mut force_kill = false;

        if let Some(cmd_tx) = self.cmd_tx.as_ref() {
            let (reply_tx, reply_rx) = oneshot::channel();
            // If the thread is already dead, that's fine — we'll just drop.
            let _ = cmd_tx.send(AcpCommand::Shutdown { reply: reply_tx });
            // Keep `cmd_tx` populated until shutdown completes so Drop still
            // has kill authority if we time out before the ACP thread exits.
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if tokio::time::timeout(remaining, reply_rx).await.is_err() {
                force_kill = true;
                tracing::warn!(
                    agent = %self.agent_name,
                    "NativeAcpConnection: shutdown ack timed out; forcing subprocess kill",
                );
                self.kill_subprocess_now();
            }
        }

        // Wait for the thread to finish within the same bounded window.
        if let Some(handle) = self.thread_handle.take() {
            let joined = Self::wait_for_thread_until(handle, deadline).await;
            if !joined {
                force_kill = true;
                tracing::warn!(
                    agent = %self.agent_name,
                    "NativeAcpConnection: ACP thread join timed out; forcing subprocess kill",
                );
                self.kill_subprocess_now();
            }
        }

        self.cmd_tx = None;
        self.health_status = AgentHealth::Unknown;
        if force_kill {
            tracing::warn!(
                agent = %self.agent_name,
                "NativeAcpConnection: shutdown completed after forced-kill fallback",
            );
        }
        tracing::info!(agent = %self.agent_name, "NativeAcpConnection: shutdown complete");
        Ok(())
    }

    // ─── health ─────────────────────────────────────────────────────────

    fn health(&self) -> AgentHealth {
        self.health_status.clone()
    }

    fn advertised_session_modes(&self, session_id: &SessionId) -> Option<Vec<SessionModeId>> {
        self.session_mode_snapshot(session_id)
            .map(|snapshot| snapshot.available_modes)
    }

    fn session_mode_snapshot(&self, session_id: &SessionId) -> Option<AcpSessionModeSnapshot> {
        self.session_modes
            .lock()
            .ok()
            .and_then(|modes| modes.get(session_id.0.as_ref()).cloned())
    }

    // ─── load_session ────────────────────────────────────────────────────

    async fn load_session(
        &mut self,
        request: LoadSessionRequest,
    ) -> anyhow::Result<(
        LoadSessionResponse,
        Pin<Box<dyn Stream<Item = SessionNotification> + Send>>,
    )> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::LoadSession {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        let (response, notification_rx) = reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during load_session setup",
                self.agent_name
            )
        })??;

        let stream = unfold(notification_rx, |mut rx| async move {
            rx.recv().await.map(|notif| (notif, rx))
        });
        Ok((response, Box::pin(stream)))
    }

    // ─── resume_session ─────────────────────────────────────────────────

    async fn resume_session(
        &mut self,
        request: ResumeSessionRequest,
    ) -> Result<ResumeSessionResponse, AcpError> {
        if !session_capability_advertised(&self.session_capabilities, |caps| caps.resume.is_some())
        {
            return Err(AcpError::CapabilityMissing("session/resume"));
        }

        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::ResumeSession {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "NativeAcpConnection '{}': ACP thread died during resume_session",
                    self.agent_name
                )
            })?
            .map_err(AcpError::Transport)
    }

    // ─── list_sessions ───────────────────────────────────────────────────

    async fn list_sessions(
        &mut self,
        request: ListSessionsRequest,
    ) -> anyhow::Result<ListSessionsResponse> {
        if !session_capability_advertised(&self.session_capabilities, |caps| caps.list.is_some()) {
            return Err(AcpError::CapabilityMissing("session/list").into());
        }

        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::ListSessions {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during list_sessions",
                self.agent_name
            )
        })?
    }

    // ─── delete_session ─────────────────────────────────────────────────

    async fn delete_session(
        &mut self,
        request: DeleteSessionRequest,
    ) -> Result<DeleteSessionResponse, AcpError> {
        if !session_capability_advertised(&self.session_capabilities, |caps| caps.delete.is_some())
        {
            return Err(AcpError::CapabilityMissing("session/delete"));
        }

        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::DeleteSession {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "NativeAcpConnection '{}': ACP thread died during delete_session",
                    self.agent_name
                )
            })?
            .map_err(AcpError::Transport)
    }

    // ─── close_session ──────────────────────────────────────────────────

    async fn close_session(
        &mut self,
        request: CloseSessionRequest,
    ) -> Result<CloseSessionResponse, AcpError> {
        if !session_capability_advertised(&self.session_capabilities, |caps| caps.close.is_some()) {
            return Err(AcpError::CapabilityMissing("session/close"));
        }

        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::CloseSession {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "NativeAcpConnection '{}': ACP thread died during close_session",
                    self.agent_name
                )
            })?
            .map_err(AcpError::Transport)
    }

    // ─── set_session_mode ────────────────────────────────────────────────

    async fn set_session_mode(
        &mut self,
        request: SetSessionModeRequest,
    ) -> anyhow::Result<SetSessionModeResponse> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::SetSessionMode {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during set_session_mode",
                self.agent_name
            )
        })?
    }

    // ─── set_session_config_option ───────────────────────────────────────

    async fn set_session_config_option(
        &mut self,
        request: SetSessionConfigOptionRequest,
    ) -> anyhow::Result<SetSessionConfigOptionResponse> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::SetSessionConfigOption {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during set_session_config_option",
                self.agent_name
            )
        })?
    }

    // ─── set_session_model ───────────────────────────────────────────────

    /// Issue standard model selection through `session/set_config_option`, or
    /// proprietary model selection through the proven `session/set_model`
    /// method (Grok / Kiro).
    async fn set_session_model(
        &mut self,
        sid: SessionId,
        model_id: String,
        caps: &SpurAgentCaps,
    ) -> Result<Vec<SessionConfigOption>, AcpError> {
        match decide_set_session_model_dispatch(caps) {
            SetSessionModelDispatch::DirectSetModel => {
                let is_known_model = caps.grok_display.as_ref().is_some_and(|display| {
                    display.models().iter().any(|model| model.id == model_id)
                }) || caps.kiro_display.as_ref().is_some_and(|display| {
                    display.models().iter().any(|model| model.id == model_id)
                });
                if !is_known_model {
                    return Err(AcpError::Transport(anyhow::anyhow!(
                        "model id is not present in the advertised catalog: {model_id}"
                    )));
                }
                let result = self
                    .call_ext(
                        "session/set_model",
                        direct_set_model_params(&sid, &model_id, None),
                    )
                    .await;
                if let Err(error) = &result {
                    record_native_dispatch_failure(
                        &self.capability_evidence,
                        CapabilityKey {
                            kind: CapabilityKind::Model,
                            upstream_id: "model".to_owned(),
                        },
                        "session/set_model",
                        error,
                        EvidenceSessionScope::Session(sid.to_string()),
                    );
                }
                result.map_err(AcpError::Transport)?;
                if let Ok(mut guard) = self.grok_session_models.lock() {
                    guard.insert(sid.to_string(), model_id);
                }
                Ok(Vec::new())
            }
            SetSessionModelDispatch::FallbackConfigOption => {
                let config_id = caps
                    .model_option()
                    .ok_or(AcpError::CapabilityMissing("set_model"))?
                    .id
                    .clone();
                let request = SetSessionConfigOptionRequest::new(
                    sid,
                    config_id,
                    SessionConfigValueId::new(model_id),
                );
                self.set_session_config_option(request)
                    .await
                    .map(|resp| resp.config_options)
                    .map_err(AcpError::Transport)
            }
            SetSessionModelDispatch::Unsupported => Err(AcpError::CapabilityMissing("set_model")),
        }
    }

    async fn set_session_effort(
        &mut self,
        sid: SessionId,
        effort_id: String,
        caps: &SpurAgentCaps,
    ) -> Result<(), AcpError> {
        if !caps.supports_grok_set_model() {
            return Err(AcpError::CapabilityMissing("set_effort"));
        }
        let model_id = self
            .grok_session_models
            .lock()
            .ok()
            .and_then(|guard| guard.get(&sid.to_string()).cloned())
            .or_else(|| {
                caps.grok_display
                    .as_ref()
                    .and_then(|display| display.model_id.clone())
            })
            .ok_or(AcpError::CapabilityMissing("set_effort"))?;
        let effort_is_supported = caps.grok_display.as_ref().is_some_and(|display| {
            display
                .efforts_for_model(&model_id)
                .iter()
                .any(|effort| effort.id == effort_id)
        });
        if !effort_is_supported {
            return Err(AcpError::Transport(anyhow::anyhow!(
                "Grok effort id is not available for model {model_id}: {effort_id}"
            )));
        }
        let result = self
            .call_ext(
                "session/set_model",
                direct_set_model_params(&sid, &model_id, Some(&effort_id)),
            )
            .await;
        if let Err(error) = &result {
            record_native_dispatch_failure(
                &self.capability_evidence,
                CapabilityKey {
                    kind: CapabilityKind::Effort,
                    upstream_id: "reasoning_effort".to_owned(),
                },
                "session/set_model",
                error,
                EvidenceSessionScope::Session(sid.to_string()),
            );
        }
        result.map(|_| ()).map_err(AcpError::Transport)
    }

    // ─── authenticate ────────────────────────────────────────────────────

    async fn authenticate(
        &mut self,
        request: AuthenticateRequest,
    ) -> anyhow::Result<AuthenticateResponse> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::Authenticate {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during authenticate",
                self.agent_name
            )
        })?
    }

    // ─── call_ext ────────────────────────────────────────────────────────

    async fn call_ext(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let cmd_tx = self.cmd_tx.as_ref().ok_or_else(|| {
            anyhow::anyhow!("NativeAcpConnection '{}': not initialized", self.agent_name)
        })?;

        // The ACP SDK re-prepends `_` when serializing `ExtRequest::method`
        // to the wire, so strip a single leading `_` here if present.
        let sdk_method = method.strip_prefix('_').unwrap_or(method).to_string();

        let raw: Box<serde_json::value::RawValue> = serde_json::value::to_raw_value(&params)?;
        let raw_arc: std::sync::Arc<serde_json::value::RawValue> = raw.into();
        let request = ExtRequest::new(sdk_method, raw_arc);

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(AcpCommand::ExtMethod {
                request,
                reply: reply_tx,
            })
            .map_err(|_| {
                anyhow::anyhow!("NativeAcpConnection '{}': ACP thread died", self.agent_name)
            })?;

        let response: ExtResponse = reply_rx.await.map_err(|_| {
            anyhow::anyhow!(
                "NativeAcpConnection '{}': ACP thread died during ext_method",
                self.agent_name
            )
        })??;

        let value: serde_json::Value = serde_json::from_str(response.0.get())?;
        Ok(value)
    }

    fn take_ext_notification_rx(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<ExtNotificationPayload>> {
        self.ext_notification_rx.take()
    }

    fn take_agent_client_request_rx(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<AgentClientRequestPayload>> {
        self.agent_client_request_rx.take()
    }

    fn subscribe_session_notifications(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<SessionNotification>> {
        Some(self.session_notif_tx.subscribe())
    }
}

// ─── Dedicated ACP thread ───────────────────────────────────────────────────

/// Entry point for the dedicated thread that owns the `!Send` ACP connection.
///
/// This function creates its own single-threaded Tokio runtime + `LocalSet`
/// and runs the SDK's I/O loop alongside a command handler that processes
/// requests from the main thread.
#[expect(
    clippy::too_many_arguments,
    reason = "thread entry point wires explicit channel ownership into the !Send ACP runtime"
)]
fn acp_thread_main(
    agent_name: String,
    agent_kind: AgentKind,
    command: String,
    extra_args: Vec<String>,
    launch_env: BTreeMap<String, String>,
    mut cmd_rx: mpsc::UnboundedReceiver<AcpCommand>,
    permission_tx: Option<mpsc::UnboundedSender<crate::types::PermissionRequest>>,
    permission_stamp: Option<std::sync::Arc<crate::types::PermissionLeaseStamp>>,
    ext_notification_tx: mpsc::UnboundedSender<ExtNotificationPayload>,
    agent_client_request_tx: mpsc::UnboundedSender<AgentClientRequestPayload>,
    session_notif_tx: tokio::sync::broadcast::Sender<SessionNotification>,
    closed_sessions: Arc<Mutex<HashSet<String>>>,
    session_modes: Arc<Mutex<HashMap<String, AcpSessionModeSnapshot>>>,
    grok_session_models: Arc<Mutex<HashMap<String, String>>>,
    session_efforts: Arc<Mutex<HashMap<String, String>>>,
    last_prompt_usage: Arc<Mutex<Option<Usage>>>,
    last_num_turns: Arc<Mutex<Option<u64>>>,
    capability_evidence: Arc<Mutex<RawAcpFrameCapture>>,
    child_pgid: Arc<Mutex<Option<i32>>>,
    repo_root: PathBuf,
    log_config: LogConfig,
) {
    // Build a single-threaded runtime for this thread.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(
                agent = %agent_name,
                "NativeAcpConnection: failed to create tokio runtime: {e}"
            );
            return;
        }
    };

    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async move {
        // Wait for the Initialize command — it tells us when to spawn the process.
        let Some(first_cmd) = cmd_rx.recv().await else {
            tracing::warn!(agent = %agent_name, "NativeAcpConnection: command channel closed before initialize");
            return;
        };

        let (init_request, init_reply) = match first_cmd {
            AcpCommand::Initialize { request, reply } => (request, reply),
            _ => {
                tracing::error!(agent = %agent_name, "NativeAcpConnection: first command must be Initialize");
                return;
            }
        };

        // Spawn the agent subprocess.
        let log_path = build_acp_log_path(&repo_root, &agent_name);
        if let Some(parent) = log_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    agent = %agent_name,
                    path = %parent.display(),
                    error = %e,
                    "NativeAcpConnection: failed to create log directory; falling back to inherit",
                );
            }
        }
        tracing::info!(
            agent = %agent_name,
            log_path = %log_path.display(),
            "NativeAcpConnection: capturing child stderr to log file"
        );
        let stderr_cfg = if log_config.child_stderr_pipe {
            // New default: spur owns the writer, child stderr flows through a
            // bounded byte-chunk reader into a per-child file-rotate writer.
            // See `connection/child_stderr_bridge.rs`.
            std::process::Stdio::piped()
        } else {
            // Legacy fall-back: child holds the FD directly. No rotation.
            match std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&log_path)
            {
                Ok(f) => std::process::Stdio::from(f),
                Err(e) => {
                    tracing::warn!(
                        agent = %agent_name,
                        path = %log_path.display(),
                        error = %e,
                        "NativeAcpConnection: child_stderr_pipe disabled but log open failed; using inherit",
                    );
                    std::process::Stdio::inherit()
                }
            }
        };

        let mut cmd = tokio::process::Command::new(&command);
        cmd.args(&extra_args)
            .envs(&launch_env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(stderr_cfg)
            .kill_on_drop(true);
        // Detach into a new session (and process group). `process_group(0)` alone
        // keeps the child in Spur's controlling session so background `/dev/tty`
        // reads deliver SIGTTIN and can freeze the TUI (codex/npx under
        // `spur tui --brain codex`). `setsid` keeps pgid == child pid for killpg
        // while removing the controlling TTY.
        #[cfg(unix)]
        detach_acp_child_session(&mut cmd);
        let child_result = cmd.spawn();

        let mut child = match child_result {
            Ok(c) => c,
            Err(e) => {
                let _ = init_reply.send(Err(anyhow::anyhow!(
                    "NativeAcpConnection '{}': failed to spawn '{}': {e}",
                    agent_name,
                    command
                )));
                return;
            }
        };

        // Record the pgid (= child pid under session detach / setsid) so the
        // `Drop` safety net and the graceful shutdown arm can reach the
        // entire process group.
        if let Some(pid) = child.id() {
            if let Ok(mut guard) = child_pgid.lock() {
                *guard = Some(pid as i32);
            }

            // Persist a registry record so the next-boot sweep can
            // reconcile this pgid even if spur dies before reaping it.
            let registry = crate::orphan_registry::PgidRegistry::new(
                repo_root.join(".spur").join("pgids"),
            );
            let rec = crate::orphan_registry::PgidRecord {
                spur_pid: std::process::id() as i32,
                spur_pid_start_time: crate::process_inspector::starttime_of_self(),
                agent_name: agent_name.clone(),
                cmd: format!("{} {}", command, extra_args.join(" ")),
                pgid: pid as i32,
                pgid_leader_start_time: crate::process_inspector::starttime_of(pid as i32)
                    .unwrap_or(0),
                spawned_at: chrono::Utc::now().timestamp(),
            };
            if let Err(e) = registry.write(&rec) {
                tracing::warn!(
                    error = %e,
                    "orphan_registry write failed; sweep cannot reclaim this child"
                );
            }
        }

        // Start the per-child stderr bridge when piping is enabled. The
        // bridge owns the read side of the child's stderr pipe and writes
        // through `file-rotate` so per-child disk usage stays bounded.
        // The handle is kept in scope until after `child.wait()` returns
        // so we can drain its `non_blocking` worker on shutdown.
        let stderr_bridge: Option<ChildStderrBridge> = if log_config.child_stderr_pipe {
            match child.stderr.take() {
                Some(stderr) => {
                    let log_dir = log_path
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."));
                    let pid = child.id().unwrap_or(0);
                    match ChildStderrBridge::start(
                        stderr,
                        &log_dir,
                        &agent_name,
                        pid,
                        log_config.child_stderr_max_bytes,
                        log_config.child_stderr_max_files,
                        log_config.child_stderr_buffered_lines_limit,
                    ) {
                        Ok(bridge) => Some(bridge),
                        Err(e) => {
                            tracing::warn!(
                                agent = %agent_name,
                                error = %e,
                                "NativeAcpConnection: failed to start child stderr bridge; \
                                 child stderr will be discarded for this run"
                            );
                            None
                        }
                    }
                }
                None => {
                    tracing::warn!(
                        agent = %agent_name,
                        "NativeAcpConnection: child_stderr_pipe enabled but child.stderr was None"
                    );
                    None
                }
            }
        } else {
            None
        };

        let child_stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = init_reply.send(Err(anyhow::anyhow!(
                    "NativeAcpConnection '{}': failed to capture stdin",
                    agent_name
                )));
                return;
            }
        };
        let child_stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = init_reply.send(Err(anyhow::anyhow!(
                    "NativeAcpConnection '{}': failed to capture stdout",
                    agent_name
                )));
                return;
            }
        };

        // Tee complete newline-delimited ACP frames before the SDK performs
        // typed projection, then adapt Tokio I/O for the SDK transport.
        let stdout_capture = EvidenceCaptureReader {
            inner: child_stdout,
            capture: capability_evidence.clone(),
        };
        let stdin_capture = EvidenceCaptureWriter {
            inner: child_stdin,
            capture: capability_evidence.clone(),
        };
        let stdout_compat = tokio_util::compat::TokioAsyncReadCompatExt::compat(stdout_capture);
        let stdin_compat =
            tokio_util::compat::TokioAsyncWriteCompatExt::compat_write(stdin_capture);
        let transport = ByteStreams::new(stdin_compat, stdout_compat);

        // Send-safe state shared between handler closures (which carry a
        // `+ Send` bound in the 0.11.1 API). Builder handler closures clone
        // these Arcs into their own captures.
        let cwd: Arc<Mutex<PathBuf>> = Arc::new(Mutex::new(PathBuf::from(".")));
        let terminals: Arc<Mutex<HashMap<String, TerminalState>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let session_event_standardizer: Arc<Mutex<crate::adapter::SessionEventStandardizer>> =
            Arc::new(Mutex::new(
                crate::adapter::SessionEventStandardizer::for_agent(agent_kind),
            ));
        let active_subagents: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let root_sessions: Arc<Mutex<HashSet<String>>> =
            Arc::new(Mutex::new(HashSet::new()));
        let new_session_in_flight = Arc::new(AtomicBool::new(false));
        let pending_new_session_updates: PendingNewSessionUpdateBuffer =
            Arc::new(Mutex::new(PendingNewSessionUpdates::default()));
        let pending_air_report_requests: PendingAirReportRequests =
            Arc::new(Mutex::new(HashMap::new()));
        let session_notification_sink = SessionNotificationSink {
            closed_sessions: closed_sessions.clone(),
            standardizer: session_event_standardizer.clone(),
            pending_air_report_requests: pending_air_report_requests.clone(),
            ext_notification_tx: ext_notification_tx.clone(),
            session_modes: session_modes.clone(),
            grok_session_models: grok_session_models.clone(),
            session_efforts: session_efforts.clone(),
            last_prompt_usage: last_prompt_usage.clone(),
            session_notif_tx: session_notif_tx.clone(),
        };

        // !Send slots used to ferry oneshot replies between the connect_with
        // closure (which is allowed to be !Send) and the post-connection
        // cleanup phase. `init_reply_slot` lets us surface a fatal error to
        // the caller even if `connect_with` ends before initialize completes;
        // `shutdown_reply_slot` lets us ack `AcpCommand::Shutdown` only AFTER
        // the child has been reaped.
        let init_reply_slot: std::rc::Rc<
            std::cell::RefCell<Option<oneshot::Sender<anyhow::Result<InitializeResponse>>>>,
        > = std::rc::Rc::new(std::cell::RefCell::new(Some(init_reply)));
        let shutdown_reply_slot: std::rc::Rc<
            std::cell::RefCell<Option<oneshot::Sender<anyhow::Result<()>>>>,
        > = std::rc::Rc::new(std::cell::RefCell::new(None));

        let connect_result: Result<(), agent_client_protocol::Error> = {
            // Per-handler clones. Each handler closure is `async move`, so
            // it owns its captures; we hand it a fresh clone of every Arc /
            // sender it needs.
            let perm_tx_h = permission_tx.clone();
            let perm_stamp_h = permission_stamp.clone();
            let ext_notification_tx_h = ext_notification_tx.clone();
            let agent_client_request_tx_h = agent_client_request_tx.clone();
            let agent_name_request_h = agent_name.clone();
            let active_subagents_h = active_subagents.clone();
            let root_sessions_h = root_sessions.clone();
            let closed_sessions_h = closed_sessions.clone();
            let new_session_in_flight_h = new_session_in_flight.clone();
            let pending_new_session_updates_h = pending_new_session_updates.clone();
            let session_notification_sink_h = session_notification_sink.clone();
            let grok_session_models_h = grok_session_models.clone();
            // For UsageUpdate → last_prompt_usage fill (phase B) during notify.
            let last_prompt_usage_h = last_prompt_usage.clone();
            let last_num_turns_h = last_num_turns.clone();

            let cwd_read = cwd.clone();
            let cwd_write = cwd.clone();
            let cwd_create_term = cwd.clone();
            let terminal_agent_kind = agent_kind;

            let terminals_create = terminals.clone();
            let terminals_output = terminals.clone();
            let terminals_wait = terminals.clone();
            let terminals_kill = terminals.clone();
            let terminals_release = terminals.clone();

            // Captures for the connect_with main_fn (the command loop).
            let cwd_loop = cwd.clone();
            let agent_name_loop = agent_name.clone();
            let init_reply_slot_loop = init_reply_slot.clone();
            let shutdown_reply_slot_loop = shutdown_reply_slot.clone();
            let last_prompt_usage_loop = last_prompt_usage.clone();
            let pending_air_report_requests_loop = pending_air_report_requests.clone();
            let session_notification_sink_loop = session_notification_sink.clone();
            let capability_evidence_loop = capability_evidence.clone();

            Client
                .builder()
                .name(format!("spur-acp-{}", agent_name))
                // ── defensive agent-originated auth/logout requests ───────
                .on_receive_request(
                    async move |req: UntypedMessage, responder, _cx| {
                        handle_agent_client_request(
                            req,
                            responder,
                            &agent_client_request_tx_h,
                            &agent_name_request_h,
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── elicitation/create (URL only) ────────────────────────
                .on_receive_request(
                    async move |req: CreateElicitationRequest, responder, _cx| {
                        responder.respond(handle_create_elicitation(req))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── session/request_permission ────────────────────────────
                .on_receive_request(
                    async move |req: RequestPermissionRequest, responder, cx| {
                        cx.spawn({
                            let permission_tx = perm_tx_h.clone();
                            let permission_stamp = perm_stamp_h.clone();
                            async move {
                                let outcome = handle_request_permission(
                                    req,
                                    permission_tx,
                                    permission_stamp,
                                )
                                .await;
                                responder.respond_with_result(outcome)
                            }
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── fs/read_text_file ─────────────────────────────────────
                .on_receive_request(
                    async move |req: ReadTextFileRequest, responder, _cx| {
                        let cwd_now = cwd_read.lock().unwrap().clone();
                        let path = if req.path.is_absolute() {
                            req.path.clone()
                        } else {
                            cwd_now.join(&req.path)
                        };
                        tracing::debug!(
                            path = %path.display(),
                            "NativeAcpConnection: reading text file"
                        );
                        let outcome = std::fs::read_to_string(&path)
                            .map_err(|e| {
                                agent_client_protocol::Error::internal_error()
                                    .data(format!("Failed to read {}: {e}", path.display()))
                            })
                            .map(|content| {
                                let trimmed = match (req.line, req.limit) {
                                    (Some(s), Some(l)) => content
                                        .lines()
                                        .skip((s.saturating_sub(1)) as usize)
                                        .take(l as usize)
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                    (Some(s), None) => content
                                        .lines()
                                        .skip((s.saturating_sub(1)) as usize)
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                    (None, Some(l)) => content
                                        .lines()
                                        .take(l as usize)
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                    (None, None) => content,
                                };
                                ReadTextFileResponse::new(trimmed)
                            });
                        responder.respond_with_result(outcome)
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── fs/write_text_file ────────────────────────────────────
                .on_receive_request(
                    async move |req: WriteTextFileRequest, responder, _cx| {
                        let cwd_now = cwd_write.lock().unwrap().clone();
                        let path = if req.path.is_absolute() {
                            req.path.clone()
                        } else {
                            cwd_now.join(&req.path)
                        };
                        tracing::debug!(
                            path = %path.display(),
                            content_len = req.content.len(),
                            "NativeAcpConnection: writing text file"
                        );
                        let outcome: agent_client_protocol::Result<WriteTextFileResponse> = (|| {
                            if let Some(parent) = path.parent() {
                                std::fs::create_dir_all(parent).map_err(|e| {
                                    agent_client_protocol::Error::internal_error().data(format!(
                                        "Failed to create directories for {}: {e}",
                                        path.display()
                                    ))
                                })?;
                            }
                            std::fs::write(&path, &req.content).map_err(|e| {
                                agent_client_protocol::Error::internal_error()
                                    .data(format!("Failed to write {}: {e}", path.display()))
                            })?;
                            Ok(WriteTextFileResponse::new())
                        })();
                        responder.respond_with_result(outcome)
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── terminal/create ───────────────────────────────────────
                .on_receive_request(
                    async move |req: CreateTerminalRequest, responder, _cx| {
                        let cwd_now = req
                            .cwd
                            .clone()
                            .unwrap_or_else(|| cwd_create_term.lock().unwrap().clone());
                        let byte_limit = req.output_byte_limit.or(Some(10 * 1024 * 1024));
                        let normalized = crate::adapter::normalize_terminal_command(
                            terminal_agent_kind,
                            &req.command,
                            &req.args,
                        );
                        let (program, command_args): (&str, &[String]) = match &normalized {
                            Some(normalized) => (normalized.program, &normalized.args),
                            None => (&req.command, &req.args),
                        };
                        let mut cmd = tokio::process::Command::new(program);
                        cmd.args(command_args)
                            .current_dir(&cwd_now)
                            .kill_on_drop(true);
                        configure_terminal_create_stdio(&mut cmd);
                        #[cfg(unix)]
                        detach_acp_child_session(&mut cmd);
                        for env_var in &req.env {
                            cmd.env(&env_var.name, &env_var.value);
                        }
                        let outcome: agent_client_protocol::Result<CreateTerminalResponse> =
                            (|| -> agent_client_protocol::Result<CreateTerminalResponse> {
                                let mut child = cmd.spawn().map_err(|e| {
                                    agent_client_protocol::Error::internal_error().data(
                                        format!("Failed to spawn '{program}': {e}"),
                                    )
                                })?;
                                let pid = child.id().ok_or_else(|| {
                                    agent_client_protocol::Error::internal_error()
                                        .data("Failed to get process ID")
                                })?;
                                let child_stdout = child.stdout.take().ok_or_else(|| {
                                    agent_client_protocol::Error::internal_error()
                                        .data("Failed to capture stdout")
                                })?;
                                let child_stderr = child.stderr.take().ok_or_else(|| {
                                    agent_client_protocol::Error::internal_error()
                                        .data("Failed to capture stderr")
                                })?;

                                let output = Arc::new(Mutex::new(String::new()));
                                let truncated = Arc::new(AtomicBool::new(false));
                                let (exit_tx, exit_rx) = tokio::sync::watch::channel(None);
                                let (control_tx, control_rx) = mpsc::unbounded_channel();

                                // Reader runs on the LocalSet task — its captured
                                // state is all `Send` so it would also satisfy
                                // `tokio::spawn`, but we don't have a multi-threaded
                                // runtime here.
                                let reader = tokio::task::spawn_local(terminal_reader(
                                    child_stdout,
                                    child_stderr,
                                    child,
                                    output.clone(),
                                    truncated.clone(),
                                    byte_limit,
                                    TerminalChannels { exit_tx, control_rx },
                                ));
                                let reader = async move {
                                    let _ = reader.await;
                                }.boxed().shared();

                                let terminal_id =
                                    TerminalId::new(uuid::Uuid::new_v4().to_string());
                                let id_string = terminal_id.to_string();
                                tracing::debug!(
                                    terminal = %id_string,
                                    command = %req.command,
                                    pid = pid,
                                    "Terminal created"
                                );
                                terminals_create.lock().unwrap().insert(
                                    id_string,
                                    TerminalState {
                                        output,
                                        truncated,
                                        exit_rx,
                                        control_tx,
                                        reader,
                                    },
                                );
                                Ok(CreateTerminalResponse::new(terminal_id))
                            })();
                        responder.respond_with_result(outcome)
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── terminal/output ───────────────────────────────────────
                .on_receive_request(
                    async move |req: TerminalOutputRequest, responder, _cx| {
                        let key = req.terminal_id.to_string();
                        let outcome: agent_client_protocol::Result<TerminalOutputResponse> = {
                            let map = terminals_output.lock().unwrap();
                            match map.get(&key) {
                                Some(terminal) => {
                                    let output = terminal.output.lock().unwrap().clone();
                                    let truncated = terminal.truncated.load(Ordering::Relaxed);
                                    let exit_status = terminal.exit_rx.borrow().clone();
                                    Ok(TerminalOutputResponse::new(output, truncated)
                                        .exit_status(exit_status))
                                }
                                None => Err(agent_client_protocol::Error::invalid_params()
                                    .data(format!("Terminal '{}' not found", key))),
                            }
                        };
                        responder.respond_with_result(outcome)
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── terminal/wait_for_exit ────────────────────────────────
                .on_receive_request(
                    async move |req: WaitForTerminalExitRequest, responder, cx| {
                        let key = req.terminal_id.to_string();
                        let mut exit_rx = {
                            let map = terminals_wait.lock().unwrap();
                            match map.get(&key) {
                                Some(terminal) => terminal.exit_rx.clone(),
                                None => {
                                    return responder.respond_with_result(Err(
                                        agent_client_protocol::Error::invalid_params().data(
                                            format!("Terminal '{}' not found", key),
                                        ),
                                    ));
                                }
                            }
                        };
                        cx.spawn(async move {
                            if let Some(status) = exit_rx.borrow().clone() {
                                return responder
                                    .respond(WaitForTerminalExitResponse::new(status));
                            }
                            loop {
                                match exit_rx.changed().await {
                                    Ok(()) => {
                                        if let Some(status) = exit_rx.borrow().clone() {
                                            return responder.respond(
                                                WaitForTerminalExitResponse::new(status),
                                            );
                                        }
                                    }
                                    Err(_) => {
                                        let status = exit_rx
                                            .borrow()
                                            .clone()
                                            .unwrap_or_else(TerminalExitStatus::new);
                                        return responder
                                            .respond(WaitForTerminalExitResponse::new(status));
                                    }
                                }
                            }
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── terminal/kill ─────────────────────────────────────────
                .on_receive_request(
                    async move |req: KillTerminalRequest, responder, _cx| {
                        let key = req.terminal_id.to_string();
                        let control = {
                            let map = terminals_kill.lock().unwrap();
                            map.get(&key).map(|t| t.control_tx.clone())
                        };
                        match control {
                            None => responder.respond_with_result(Err(
                                agent_client_protocol::Error::invalid_params()
                                    .data(format!("Terminal '{}' not found", key)),
                            )),
                            Some(control) => {
                                // Descendants can still hold the pipes after
                                // the command's exit status has been published.
                                let _ = control.send(TerminalControl::Kill);
                                responder.respond(KillTerminalResponse::new())
                            }
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── terminal/release ──────────────────────────────────────
                .on_receive_request(
                    async move |req: ReleaseTerminalRequest, responder, _cx| {
                        let key = req.terminal_id.to_string();
                        let terminal = terminals_release.lock().unwrap().get(&key)
                            .map(|terminal| (terminal.control_tx.clone(), terminal.reader.clone()));
                        if let Some((control, reader)) = terminal {
                            // Retain completion in the map while awaiting it:
                            // disconnect can cancel this callback, and shutdown
                            // must still be able to join the owning reader.
                            let _ = control.send(TerminalControl::Release);
                            reader.await;
                            terminals_release.lock().unwrap().remove(&key);
                        }
                        responder.respond(ReleaseTerminalResponse::new())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                // ── notifications: session/update + extension ────────────
                .on_receive_notification(
                    async move |notif: UntypedMessage, _cx| {
                        let (notif, encoded_update_bytes) = if notif.method == "session/update" {
                            let encoded_bytes = serde_json::to_vec(&notif.params)
                                .map(|bytes| bytes.len())
                                .unwrap_or(PENDING_NEW_SESSION_MAX_BYTES.saturating_add(1));
                            match normalize_session_notification(
                                notif.params,
                                &active_subagents_h,
                                &root_sessions_h,
                                &closed_sessions_h,
                            ) {
                                Ok(Some(notification)) => {
                                    (
                                        AgentNotification::SessionNotification(notification),
                                        Some(encoded_bytes),
                                    )
                                }
                                Ok(None) => return Ok(()),
                                Err(error) => {
                                    tracing::warn!(
                                        %error,
                                        "dropping malformed ACP session/update notification"
                                    );
                                    return Ok(());
                                }
                            }
                        } else {
                            match AgentNotification::parse_message(&notif.method, &notif.params) {
                                Ok(notification) => (notification, None),
                                Err(error) => {
                                    tracing::warn!(
                                        method = %notif.method,
                                        %error,
                                        "dropping malformed ACP agent notification"
                                    );
                                    return Ok(());
                                }
                            }
                        };
                        match notif {
                            AgentNotification::SessionNotification(args) => {
                                match route_session_notification(
                                    &args,
                                    &root_sessions_h,
                                    &active_subagents_h,
                                    new_session_in_flight_h.load(Ordering::Acquire),
                                ) {
                                    SessionNotificationRoute::Forward => {
                                        session_notification_sink_h.forward(args);
                                    }
                                    SessionNotificationRoute::BufferUntilNewSession => {
                                        if !buffer_new_session_update(
                                            &pending_new_session_updates_h,
                                            args,
                                            encoded_update_bytes.unwrap_or(0),
                                        ) {
                                            tracing::warn!(
                                                max_notifications = PENDING_NEW_SESSION_MAX_NOTIFICATIONS,
                                                max_bytes = PENDING_NEW_SESSION_MAX_BYTES,
                                                "dropping all buffered session/new updates after capacity overflow"
                                            );
                                        }
                                    }
                                    SessionNotificationRoute::Drop => {
                                        tracing::debug!(
                                            session = %args.session_id,
                                            "dropping ACP notification for unknown session"
                                        );
                                    }
                                }
                            }
                            AgentNotification::ExtNotification(args) => {
                                // The SDK already stripped the leading `_` from
                                // the wire method, so reattach it when reporting
                                // upward so consumers see the full
                                // `_foo.dev/...` form.
                                let method = format!("_{}", args.method);
                                let params: serde_json::Value =
                                    serde_json::from_str(args.params.get())
                                        .unwrap_or(serde_json::Value::Null);
                                if method == "_x.ai/session_notification" {
                                    cache_grok_model_changed(&grok_session_models_h, &params);
                                }
                                // R1 / sol_10e37ea151524119: Grok turn_completed.usage
                                // arrives on `_x.ai/session/update` (not events.jsonl,
                                // not chunk `_meta.totalTokens`).
                                if method == "_x.ai/session/update"
                                    || method == "_x.ai/session_notification"
                                {
                                    crate::usage_ingest::maybe_apply_turn_completed(
                                        last_prompt_usage_h.as_ref(),
                                        last_num_turns_h.as_ref(),
                                        grok_session_models_h.as_ref(),
                                        &params,
                                    );
                                }
                                tracing::debug!(
                                    method = %method,
                                    "NativeAcpConnection: ext_notification"
                                );
                                if let Some(payload) =
                                    forwardable_ext_notification(method, params)
                                {
                                    let _ = ext_notification_tx_h.send(payload);
                                }
                            }
                            AgentNotification::CompleteElicitationNotification(args) => {
                                tracing::debug!(
                                    elicitation_id = %args.elicitation_id,
                                    "ACP URL elicitation completed"
                                );
                            }
                            _ => {
                                // `AgentNotification` is `#[non_exhaustive]`;
                                // future variants under unstable features land
                                // here. Drop them silently.
                            }
                        }
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                // ── connect_with: drives the command loop ────────────────
                .connect_with(transport, async move |cx: ConnectionTo<Agent>| -> agent_client_protocol::Result<()> {
                    // 1. Run the ACP initialize handshake and forward the
                    //    response to the caller blocked in `initialize()`.
                    let init_started_at = Instant::now();
                    let init_outcome = cx.send_request(init_request).block_task().await;
                    emit_acp_request_result(init_started_at, &init_outcome);
                    match init_outcome {
                        Ok(mut response) => {
                            inject_latest_capability_evidence(
                                &mut response.meta,
                                &capability_evidence_loop,
                            );
                            if let Some(reply) =
                                init_reply_slot_loop.borrow_mut().take()
                            {
                                let _ = reply.send(Ok(response));
                            }
                        }
                        Err(e) => {
                            record_operational_capability_failure(
                                &capability_evidence_loop,
                                "initialize",
                                &e,
                                EvidenceSessionScope::Global,
                            );
                            if let Some(reply) =
                                init_reply_slot_loop.borrow_mut().take()
                            {
                                let _ = reply.send(Err(anyhow::anyhow!(
                                    "NativeAcpConnection '{}': initialize failed: {e}",
                                    agent_name_loop
                                )));
                            }
                            return Err(e);
                        }
                    }

                    // 2. Process commands sequentially. Each `block_task().await`
                    //    suspends here while handler callbacks continue to run
                    //    on the dispatch loop.
                    while let Some(cmd) = cmd_rx.recv().await {
                        match cmd {
                            AcpCommand::Initialize { reply, .. } => {
                                let _ = reply.send(Err(anyhow::anyhow!(
                                    "NativeAcpConnection '{}': already initialized",
                                    agent_name_loop
                                )));
                            }
                            AcpCommand::NewSession { request, reply } => {
                                *cwd_loop.lock().unwrap() = request.cwd.clone();
                                clear_pending_new_session_updates(&pending_new_session_updates);
                                new_session_in_flight.store(true, Ordering::Release);
                                let request_started_at = Instant::now();
                                // Kiro (and Grok) still emit a top-level `models`
                                // plane on session/new, but ACP schema 1.1 drops
                                // it on typed deserialize. Re-issue as ExtMethod
                                // for Kiro so we can recover the catalog into meta.
                                let mut result = if agent_kind == AgentKind::Kiro {
                                    match serde_json::to_value(&request) {
                                        Ok(params) => {
                                            match serde_json::value::to_raw_value(&params) {
                                                Ok(raw) => {
                                                    let client_req = ClientRequest::ExtMethodRequest(
                                                        ExtRequest::new(
                                                            "session/new",
                                                            std::sync::Arc::from(raw),
                                                        ),
                                                    );
                                                    match cx
                                                        .send_request(client_req)
                                                        .block_task()
                                                        .await
                                                    {
                                                        Ok(json) => new_session_from_raw_value(json)
                                                            .map_err(|e| {
                                                                agent_client_protocol::Error::internal_error()
                                                                    .data(e.to_string())
                                                            }),
                                                        Err(e) => Err(e),
                                                    }
                                                }
                                                Err(e) => Err(agent_client_protocol::Error::internal_error()
                                                    .data(format!(
                                                        "session/new params not serializable: {e}"
                                                    ))),
                                            }
                                        }
                                        Err(e) => Err(agent_client_protocol::Error::internal_error()
                                            .data(format!(
                                                "session/new params not serializable: {e}"
                                            ))),
                                    }
                                } else {
                                    cx.send_request(request).block_task().await
                                };
                                emit_acp_request_result(request_started_at, &result);
                                if let Ok(response) = &mut result {
                                    inject_latest_capability_evidence(
                                        &mut response.meta,
                                        &capability_evidence_loop,
                                    );
                                    complete_new_session(
                                        response,
                                        &root_sessions,
                                        &closed_sessions,
                                        &new_session_in_flight,
                                        &pending_new_session_updates,
                                        &session_notification_sink_loop,
                                        &session_modes,
                                        &grok_session_models,
                                        &session_efforts,
                                    );
                                } else {
                                    if let Err(error) = &result {
                                        record_operational_capability_failure(
                                            &capability_evidence_loop,
                                            "session/new",
                                            error,
                                            EvidenceSessionScope::Global,
                                        );
                                    }
                                    new_session_in_flight.store(false, Ordering::Release);
                                    clear_pending_new_session_updates(&pending_new_session_updates);
                                }
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': new_session failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::Prompt {
                                mut request,
                                reply,
                                terminal_reply,
                            } => {
                                // Notifications flow out-of-band via the
                                // `session_notif_tx` broadcast. The `Stream`
                                // returned by `prompt()` is a live but-empty
                                // `UnboundedReceiver`. Its close remains a
                                // compatibility completion signal, while the
                                // terminal oneshot preserves RPC success/error.
                                let (tx_empty, rx_empty) =
                                    mpsc::unbounded_channel::<SessionNotification>();
                                let _ = reply.send(Ok(rx_empty));
                                let mut terminal_reply = Some(terminal_reply);
                                if agent_kind == AgentKind::CodexAcp {
                                    let request_id =
                                        format!("spur-{}", uuid::Uuid::new_v4());
                                    attach_air_file_change_report_request(
                                        &mut request,
                                        &request_id,
                                    );
                                    register_pending_air_report(
                                        &pending_air_report_requests_loop,
                                        request.session_id.0.as_ref(),
                                        &request_id,
                                    );
                                }
                                let session_id_for_probe = request.session_id.clone();
                                // Multiplex command intake against the in-flight prompt
                                // future so `Cancel` and `Shutdown` can be serviced while
                                // the agent is still streaming. `biased;` polls cmd_rx
                                // first so a queued cancel cannot starve behind heavy
                                // notification flow.
                                let request_started_at = Instant::now();
                                let prompt_fut = cx.send_request(request).block_task();
                                tokio::pin!(prompt_fut);
                                let mut cmd_rx_closed = false;
                                loop {
                                    tokio::select! {
                                        biased;
                                        maybe_cmd = cmd_rx.recv(), if !cmd_rx_closed => {
                                            match maybe_cmd {
                                                Some(AcpCommand::Cancel { session_id, reply }) => {
                                                    dispatch_cancel(&cx, session_id, reply, &agent_name_loop);
                                                }
                                                Some(AcpCommand::Shutdown { reply }) => {
                                                    tracing::debug!(
                                                        agent = %agent_name_loop,
                                                        "NativeAcpConnection: ACP thread received shutdown during in-flight prompt"
                                                    );
                                                    *shutdown_reply_slot_loop.borrow_mut() = Some(reply);
                                                    if let Some(terminal_reply) = terminal_reply.take() {
                                                        let _ = terminal_reply.send(Err(anyhow::anyhow!(
                                                            "NativeAcpConnection '{}': shutdown before prompt completed",
                                                            agent_name_loop
                                                        )));
                                                    }
                                                    drop(tx_empty);
                                                    return Ok(());
                                                }
                                                Some(other) => {
                                                    reject_busy_command(other, &agent_name_loop, "prompt");
                                                }
                                                None => {
                                                    cmd_rx_closed = true;
                                                }
                                            }
                                        }
                                        prompt_result = &mut prompt_fut => {
                                            emit_acp_request_result(request_started_at, &prompt_result);
                                            clear_pending_air_report(
                                                &pending_air_report_requests_loop,
                                                session_id_for_probe.0.as_ref(),
                                            );
                                            let terminal_result = match prompt_result {
                                                Ok(response) => {
                                                    // H6: PromptResponse.usage wins when
                                                    // present. Do not clobber a prior
                                                    // turn_completed / UsageUpdate fill
                                                    // with None.
                                                    if let Some(u) = &response.usage {
                                                        if let Ok(mut usage) =
                                                            last_prompt_usage_loop.lock()
                                                        {
                                                            *usage = Some(u.clone());
                                                        }
                                                    }
                                                    tracing::debug!(
                                                        agent = %agent_name_loop,
                                                        session = %session_id_for_probe,
                                                        "NativeAcpConnection: prompt completed"
                                                    );
                                                    Ok(response)
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        agent = %agent_name_loop,
                                                        session = %session_id_for_probe,
                                                        "NativeAcpConnection: prompt failed: {e}"
                                                    );
                                                    Err(anyhow::anyhow!(
                                                        "NativeAcpConnection '{}': prompt failed: {e}",
                                                        agent_name_loop
                                                    ))
                                                }
                                            };
                                            let terminal_reply = terminal_reply
                                                .take()
                                                .expect("Prompt terminal reply consumed only once");
                                            let _ = terminal_reply.send(terminal_result);
                                            drop(tx_empty);
                                            break;
                                        }
                                    }
                                }
                            }
                            AcpCommand::Cancel { session_id, reply } => {
                                dispatch_cancel(&cx, session_id, reply, &agent_name_loop);
                            }
                            AcpCommand::Shutdown { reply } => {
                                tracing::debug!(
                                    agent = %agent_name_loop,
                                    "NativeAcpConnection: ACP thread received shutdown"
                                );
                                // Stash the reply for the post-connection
                                // cleanup phase. Returning here closes the
                                // SDK's writer half — which is the protocol's
                                // graceful-exit contract: the agent sees EOF
                                // on stdin and exits cleanly.
                                *shutdown_reply_slot_loop.borrow_mut() = Some(reply);
                                return Ok(());
                            }
                            AcpCommand::LoadSession { request, reply } => {
                                *cwd_loop.lock().unwrap() = request.cwd.clone();
                                let (tx_empty, rx_empty) =
                                    mpsc::unbounded_channel::<SessionNotification>();
                                let session_id_for_probe = request.session_id.clone();
                                let open_rollback = begin_session_open(
                                    &root_sessions,
                                    &closed_sessions,
                                    session_id_for_probe.0.as_ref(),
                                );
                                // Multiplex command intake against the in-flight
                                // `session/load` future so cancel/shutdown can be
                                // serviced while history replay is in progress.
                                // Reply is sent AFTER the future resolves (load_session's
                                // contract is reply-with-result, unlike Prompt which
                                // hands the empty stream out up front).
                                let request_started_at = Instant::now();
                                // See NewSession: recover Kiro models plane via ExtMethod.
                                let load_session_fut = async {
                                    if agent_kind == AgentKind::Kiro {
                                        match serde_json::to_value(&request) {
                                            Ok(params) => {
                                                match serde_json::value::to_raw_value(&params) {
                                                    Ok(raw) => {
                                                        let client_req =
                                                            ClientRequest::ExtMethodRequest(
                                                                ExtRequest::new(
                                                                    "session/load",
                                                                    std::sync::Arc::from(raw),
                                                                ),
                                                            );
                                                        match cx
                                                            .send_request(client_req)
                                                            .block_task()
                                                            .await
                                                        {
                                                            Ok(json) => {
                                                                load_session_from_raw_value(json)
                                                                    .map_err(|e| {
                                                                        agent_client_protocol::Error::internal_error()
                                                                            .data(e.to_string())
                                                                    })
                                                            }
                                                            Err(e) => Err(e),
                                                        }
                                                    }
                                                    Err(e) => Err(
                                                        agent_client_protocol::Error::internal_error()
                                                            .data(format!(
                                                                "session/load params not serializable: {e}"
                                                            )),
                                                    ),
                                                }
                                            }
                                            Err(e) => Err(
                                                agent_client_protocol::Error::internal_error()
                                                    .data(format!(
                                                        "session/load params not serializable: {e}"
                                                    )),
                                            ),
                                        }
                                    } else {
                                        cx.send_request(request).block_task().await
                                    }
                                };
                                tokio::pin!(load_session_fut);
                                let mut reply_holder = Some(reply);
                                let mut cmd_rx_closed = false;
                                loop {
                                    tokio::select! {
                                        biased;
                                        maybe_cmd = cmd_rx.recv(), if !cmd_rx_closed => {
                                            match maybe_cmd {
                                                Some(AcpCommand::Cancel { session_id, reply }) => {
                                                    dispatch_cancel(&cx, session_id, reply, &agent_name_loop);
                                                }
                                                Some(AcpCommand::Shutdown { reply }) => {
                                                    tracing::debug!(
                                                        agent = %agent_name_loop,
                                                        "NativeAcpConnection: ACP thread received shutdown during in-flight load_session"
                                                    );
                                                    *shutdown_reply_slot_loop.borrow_mut() = Some(reply);
                                                    drop(tx_empty);
                                                    return Ok(());
                                                }
                                                Some(other) => {
                                                    reject_busy_command(other, &agent_name_loop, "load_session");
                                                }
                                                None => {
                                                    cmd_rx_closed = true;
                                                }
                                            }
                                        }
                                        load_result = &mut load_session_fut => {
                                            emit_acp_request_result(request_started_at, &load_result);
                                            let reply = reply_holder
                                                .take()
                                                .expect("LoadSession reply consumed only once");
                                            match load_result {
                                                Ok(mut response) => {
                                                    inject_latest_capability_evidence(
                                                        &mut response.meta,
                                                        &capability_evidence_loop,
                                                    );
                                                    cache_session_modes(
                                                        &session_modes,
                                                        &session_id_for_probe,
                                                        response.modes.as_ref(),
                                                    );
                                    if let Some(opts) = &response.config_options {
                                        cache_config_options_model_effort(
                                            &grok_session_models,
                                            &session_efforts,
                                            session_id_for_probe.0.as_ref(),
                                            opts,
                                        );
                                    }
                                                    tracing::debug!(
                                                        agent = %agent_name_loop,
                                                        session = %session_id_for_probe,
                                                        "NativeAcpConnection: load_session completed"
                                                    );
                                                    let _ = reply.send(Ok((response, rx_empty)));
                                                }
                                                Err(e) => {
                                                    record_operational_capability_failure(
                                                        &capability_evidence_loop,
                                                        "session/load",
                                                        &e,
                                                        EvidenceSessionScope::Session(
                                                            session_id_for_probe.to_string(),
                                                        ),
                                                    );
                                                    rollback_session_open(
                                                        &root_sessions,
                                                        &closed_sessions,
                                                        session_id_for_probe.0.as_ref(),
                                                        open_rollback,
                                                    );
                                                    tracing::warn!(
                                                        agent = %agent_name_loop,
                                                        session = %session_id_for_probe,
                                                        "NativeAcpConnection: load_session failed: {e}"
                                                    );
                                                    let _ = reply.send(Err(anyhow::anyhow!(
                                                        "NativeAcpConnection '{}': load_session failed: {e}",
                                                        agent_name_loop
                                                    )));
                                                }
                                            }
                                            drop(tx_empty);
                                            break;
                                        }
                                    }
                                }
                            }
                            AcpCommand::ResumeSession { request, reply } => {
                                *cwd_loop.lock().unwrap() = request.cwd.clone();
                                let session_id = request.session_id.clone();
                                let open_rollback = begin_session_open(
                                    &root_sessions,
                                    &closed_sessions,
                                    session_id.0.as_ref(),
                                );
                                let request_started_at = Instant::now();
                                let result = cx.send_request(request).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                if let Ok(response) = &result {
                                    cache_session_modes(
                                        &session_modes,
                                        &session_id,
                                        response.modes.as_ref(),
                                    );
                                    if let Some(opts) = &response.config_options {
                                        cache_config_options_model_effort(
                                            &grok_session_models,
                                            &session_efforts,
                                            session_id.0.as_ref(),
                                            opts,
                                        );
                                    }
                                } else {
                                    rollback_session_open(
                                        &root_sessions,
                                        &closed_sessions,
                                        session_id.0.as_ref(),
                                        open_rollback,
                                    );
                                }
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': resume_session failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::ListSessions { request, reply } => {
                                let request_started_at = Instant::now();
                                let result = cx.send_request(request).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': list_sessions failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::DeleteSession { request, reply } => {
                                let request_started_at = Instant::now();
                                let result = cx.send_request(request).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': delete_session failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::CloseSession { request, reply } => {
                                let session_id = request.session_id.0.to_string();
                                if closed_sessions
                                    .lock()
                                    .is_ok_and(|closed| closed.contains(&session_id))
                                {
                                    let _ = reply.send(Ok(CloseSessionResponse::new()));
                                    continue;
                                }
                                let request_started_at = Instant::now();
                                let result = cx.send_request(request).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                if result.is_ok() {
                                    if let Ok(mut closed) = closed_sessions.lock() {
                                        closed.insert(session_id.clone());
                                    }
                                    remove_subagent_descendants(&active_subagents, &session_id);
                                    clear_pending_air_report(
                                        &pending_air_report_requests_loop,
                                        &session_id,
                                    );
                                }
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': close_session failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::SetSessionMode { request, reply } => {
                                let request_started_at = Instant::now();
                                let result = cx.send_request(request).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': set_session_mode failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::SetSessionConfigOption { request, reply } => {
                                let request_started_at = Instant::now();
                                let result = cx.send_request(request).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': set_session_config_option failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::Authenticate { request, reply } => {
                                let request_started_at = Instant::now();
                                let result = cx.send_request(request).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                let _ = reply.send(result.map_err(|e| {
                                    anyhow::anyhow!(
                                        "NativeAcpConnection '{}': authenticate failed: {e}",
                                        agent_name_loop
                                    )
                                }));
                            }
                            AcpCommand::ExtMethod { request, reply } => {
                                // The 0.11.1 SDK exposes extension calls only
                                // via the wrapping `ClientRequest` enum (whose
                                // `Response` is `serde_json::Value`). We
                                // re-wrap the response payload back into an
                                // `ExtResponse` so the caller-side translation
                                // in `call_ext` is unchanged.
                                let client_req =
                                    ClientRequest::ExtMethodRequest(
                                        request,
                                    );
                                let request_started_at = Instant::now();
                                let result = cx.send_request(client_req).block_task().await;
                                emit_acp_request_result(request_started_at, &result);
                                let mapped: anyhow::Result<ExtResponse> = match result {
                                    Ok(json) => match serde_json::value::to_raw_value(&json)
                                    {
                                        Ok(raw) => Ok(ExtResponse::new(std::sync::Arc::from(
                                            raw,
                                        ))),
                                        Err(e) => Err(anyhow::anyhow!(
                                            "NativeAcpConnection '{}': ext_method response not serializable: {e}",
                                            agent_name_loop
                                        )),
                                    },
                                    Err(e) => Err(anyhow::anyhow!(
                                        "NativeAcpConnection '{}': ext_method failed: {e}",
                                        agent_name_loop
                                    )),
                                };
                                let _ = reply.send(mapped);
                            }
                        }
                    }
                    Ok(())
                })
                .await
        };

        if let Err(e) = &connect_result {
            tracing::warn!(
                agent = %agent_name,
                "NativeAcpConnection: connection ended with error: {e}"
            );
        }

        // If init never produced a response (transport died during the
        // handshake), make sure the caller blocked in `initialize()` sees an
        // error instead of waiting forever on the oneshot.
        if let Some(reply) = init_reply_slot.borrow_mut().take() {
            let err = match &connect_result {
                Err(e) => anyhow::anyhow!(
                    "NativeAcpConnection '{}': connection ended before initialize: {e}",
                    agent_name
                ),
                Ok(()) => anyhow::anyhow!(
                    "NativeAcpConnection '{}': connection closed before initialize",
                    agent_name
                ),
            };
            let _ = reply.send(Err(err));
        }

        // Signal every reader before joining any of them. Exited commands can
        // still own descendants and open pipes, so exit_rx is not a cleanup gate.
        let remaining_terminals: Vec<_> = terminals.lock().unwrap().drain().collect();
        for (id, terminal) in &remaining_terminals {
            tracing::debug!(terminal = %id, "Releasing terminal on shutdown");
            let _ = terminal.control_tx.send(TerminalControl::Release);
        }
        for (_, terminal) in remaining_terminals {
            terminal.reader.await;
        }

        // Stdin has already closed (the SDK writer was dropped when the
        // connection tore down), but ACP+rmcp agents can remain alive on
        // non-stdin event loops. Send SIGTERM immediately via the process
        // group, which catches grandchildren (e.g. the `node` tree under
        // `claude-agent-acp`) that don't watch stdin themselves.
        // Take (not copy) so the Drop safety net at :349-373 sees `None`
        // and does not later signal a recycled PID.
        let pgid = child_pgid.lock().ok().and_then(|mut g| g.take());
        if let Some(pgid) = pgid {
            killpg(pgid, "TERM");
        }
        let graceful = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            child.wait(),
        )
        .await;
        match graceful {
            Ok(Ok(status)) => {
                tracing::debug!(
                    agent = %agent_name,
                    ?status,
                    "NativeAcpConnection: agent exited gracefully after SIGTERM"
                );
            }
            _ => {
                tracing::warn!(
                    agent = %agent_name,
                    "NativeAcpConnection: agent did not exit within 1s of SIGTERM; escalating"
                );
                if let Some(pgid) = pgid {
                    killpg(pgid, "KILL");
                }
                let _ = child.kill().await;
            }
        }
        // The pgid (if any) is now reaped via either branch above; clear
        // its on-disk registry record so the next-boot sweep doesn't trip
        // over a recycled pid.
        if let Some(pgid) = pgid {
            let registry = crate::orphan_registry::PgidRegistry::new(
                repo_root.join(".spur").join("pgids"),
            );
            let _ = registry.delete(pgid);
        }

        // Drain the per-child stderr bridge: child exit closed the pipe so
        // the reader task is at EOF; awaiting the join handle then dropping
        // the WorkerGuard lets `non_blocking` flush remaining chunks.
        if let Some(bridge) = stderr_bridge {
            bridge.shutdown().await;
        }
        // Mark pgid consumed so `Drop` won't re-kill a reaped or recycled
        // group id.
        if let Ok(mut guard) = child_pgid.lock() {
            *guard = None;
        }

        // Send shutdown ack only after the child has been reaped — so the
        // caller sees `Ok(())` truly mean "everything is gone".
        if let Some(reply) = shutdown_reply_slot.borrow_mut().take() {
            let _ = reply.send(Ok(()));
        }

        tracing::debug!(agent = %agent_name, "NativeAcpConnection: ACP thread exiting");
    });
}

/// Permission request handler factored out so the handler closure stays
/// small. Keeps the original 60s timeout + auto-fallback behaviour.
///
/// Fail-closed on a closed interactive channel (deny). `permission_tx = None`
/// remains the product skip-permissions auto-approve path.
///
/// Interactive replies wait for the user without a wall-clock cap
/// (`PERMISSION_REPLY_TIMEOUT`); a dropped reply channel still denies.
const PERMISSION_REPLY_TIMEOUT: Option<std::time::Duration> = None;

async fn handle_request_permission(
    args: RequestPermissionRequest,
    permission_tx: Option<mpsc::UnboundedSender<crate::types::PermissionRequest>>,
    permission_stamp: Option<std::sync::Arc<crate::types::PermissionLeaseStamp>>,
) -> agent_client_protocol::Result<RequestPermissionResponse> {
    let Some(perm_tx) = permission_tx else {
        return auto_approve(&args);
    };

    // Capture lease identity at handler entry (not at send time) so a late
    // request still carries the fence observed when the handler began.
    let session_id = args.session_id.0.as_ref();
    let (generation, operation_fence) = permission_stamp
        .as_ref()
        .map(|stamp| stamp.snapshot_for_session(session_id))
        .unwrap_or((0, 0));

    let (reply_tx, reply_rx) = oneshot::channel();
    let request = crate::types::PermissionRequest {
        args: args.clone(),
        reply_tx,
        generation,
        operation_fence,
    };

    if perm_tx.send(request).is_err() {
        tracing::warn!("NativeAcpConnection: permission channel closed, denying");
        return auto_deny(&args);
    }

    tracing::debug!(
        session = %args.session_id,
        generation,
        operation_fence,
        "NativeAcpConnection: awaiting interactive permission response"
    );

    let awaited = match PERMISSION_REPLY_TIMEOUT {
        Some(limit) => match tokio::time::timeout(limit, reply_rx).await {
            Ok(resolved) => resolved,
            Err(_) => {
                tracing::warn!("NativeAcpConnection: permission timed out (safety cap)");
                return auto_deny(&args);
            }
        },
        None => reply_rx.await,
    };

    match awaited {
        Ok(response) => {
            let Some(option) = args
                .options
                .iter()
                .find(|option| option.option_id.0.as_ref() == response.option_id)
            else {
                tracing::warn!(
                    option = %response.option_id,
                    "NativeAcpConnection: permission response was not advertised; cancelling"
                );
                return Ok(cancelled_permission_response());
            };
            let option_id = option.option_id.clone();
            tracing::debug!(option = %option_id, "NativeAcpConnection: permission responded");
            Ok(RequestPermissionResponse::new(
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
            ))
        }
        Err(_) => {
            tracing::debug!("NativeAcpConnection: permission denied (channel dropped)");
            auto_deny(&args)
        }
    }
}

fn handle_create_elicitation(request: CreateElicitationRequest) -> CreateElicitationResponse {
    let ElicitationMode::Url(url_mode) = request.mode else {
        return CreateElicitationResponse::new(ElicitationAction::Cancel);
    };
    let action = if open_elicitation_url(&url_mode.url) {
        ElicitationAction::Accept(ElicitationAcceptAction::new())
    } else {
        ElicitationAction::Cancel
    };
    CreateElicitationResponse::new(action)
}

fn open_elicitation_url(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        tracing::warn!(url, "refusing non-HTTP elicitation URL");
        return false;
    }

    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");

    command.arg(url).spawn().is_ok()
}

// ─── Terminal state ─────────────────────────────────────────────────────────

/// Per-terminal handle stored in the connection-scoped `terminals` map.
///
/// The fields are all `Send` so the entire map can sit behind an
/// `Arc<Mutex<…>>` shared by Send-bounded handler closures.
struct TerminalState {
    output: Arc<Mutex<String>>,
    truncated: Arc<AtomicBool>,
    exit_rx: tokio::sync::watch::Receiver<Option<TerminalExitStatus>>,
    control_tx: mpsc::UnboundedSender<TerminalControl>,
    reader: Shared<BoxFuture<'static, ()>>,
}

enum TerminalControl {
    Kill,
    Release,
}

struct TerminalChannels {
    exit_tx: tokio::sync::watch::Sender<Option<TerminalExitStatus>>,
    control_rx: mpsc::UnboundedReceiver<TerminalControl>,
}

// ─── Diagnostic helpers (streaming probes) ──────────────────────────────────

/// Short static name for each SessionUpdate discriminant.
/// Used by diagnostic logging only; keep lowercase snake_case.
fn session_update_variant_name(u: &SessionUpdate) -> &'static str {
    use agent_client_protocol::schema::v1::SessionUpdate::*;
    match u {
        AgentThoughtChunk(_) => "agent_thought_chunk",
        AgentMessageChunk(_) => "agent_message_chunk",
        UserMessageChunk(_) => "user_message_chunk",
        ToolCall(_) => "tool_call",
        ToolCallUpdate(_) => "tool_call_update",
        Plan(_) => "plan",
        AvailableCommandsUpdate(_) => "available_commands_update",
        ConfigOptionUpdate(_) => "config_option_update",
        CurrentModeUpdate(_) => "current_mode_update",
        SessionInfoUpdate(_) => "session_info_update",
        UsageUpdate(_) => "usage_update",
        _ => "other",
    }
}

/// Return the text length of a content chunk, or 0 if non-text.
fn content_chunk_text_len(chunk: &ContentChunk) -> usize {
    match &chunk.content {
        ContentBlock::Text(tc) => tc.text.len(),
        _ => 0,
    }
}

// ─── Permission helpers ─────────────────────────────────────────────────────

fn auto_approve(
    _args: &RequestPermissionRequest,
) -> agent_client_protocol::Result<RequestPermissionResponse> {
    // Without a user-selected opaque option ID there is no protocol-safe way
    // to infer authorization from presentation order, label, kind, or metadata.
    Ok(cancelled_permission_response())
}

/// Test-only re-export of the private `auto_approve` helper so
/// integration tests under `tests/` can exercise its selection logic
/// without spawning an agent. Hidden from rustdoc; not a stability
/// surface.
#[doc(hidden)]
pub fn __test_auto_approve(
    args: &RequestPermissionRequest,
) -> agent_client_protocol::Result<RequestPermissionResponse> {
    auto_approve(args)
}

fn auto_deny(
    _args: &RequestPermissionRequest,
) -> agent_client_protocol::Result<RequestPermissionResponse> {
    Ok(cancelled_permission_response())
}

fn cancelled_permission_response() -> RequestPermissionResponse {
    RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
}

// ─── Terminal helpers ────────────────────────────────────────────────────────

/// Prevents ACP terminal tools from inheriting Spur's terminal through fd 0.
///
/// This closes the inherited-stdin route only; pair with
/// [`detach_acp_child_session`] so the child also cannot open `/dev/tty` on
/// Spur's controlling session.
fn configure_terminal_create_stdio(command: &mut tokio::process::Command) {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
}

/// Detach an ACP child from Spur's controlling terminal session.
///
/// Creates a new session via `setsid` so the child is no longer in the TUI's
/// session and has no controlling TTY. Also makes the child its own process
/// group leader (`pgid == pid`), preserving `killpg` reaping used by
/// shutdown / orphan sweep.
///
/// Prefer this over `process_group(0)` alone: a new process group in the same
/// session still inherits the controlling TTY, and background reads of
/// `/dev/tty` then receive SIGTTIN (reproduced under `spur tui --brain codex`).
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "pre_exec is required to call setsid after fork before exec"
)]
fn detach_acp_child_session(command: &mut tokio::process::Command) {
    // SAFETY: the child callback only invokes the async-signal-safe `setsid`
    // syscall and reports its errno. No allocation or lock is used after fork.
    // `tokio::process::Command::pre_exec` is the Unix-only hook for this.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn append_terminal_output(
    output: &Arc<Mutex<String>>,
    truncated: &Arc<AtomicBool>,
    byte_limit: Option<u64>,
    data: &[u8],
) {
    let text = String::from_utf8_lossy(data);
    let mut buf = output.lock().unwrap();
    buf.push_str(&text);
    if let Some(limit) = byte_limit {
        let limit = limit as usize;
        if buf.len() > limit {
            let mut start = buf.len() - limit;
            while !buf.is_char_boundary(start) {
                start += 1;
            }
            *buf = buf[start..].to_string();
            truncated.store(true, Ordering::Relaxed);
        }
    }
}

async fn terminal_reader(
    mut stdout: tokio::process::ChildStdout,
    mut stderr: tokio::process::ChildStderr,
    mut child: tokio::process::Child,
    output: Arc<Mutex<String>>,
    truncated: Arc<AtomicBool>,
    byte_limit: Option<u64>,
    channels: TerminalChannels,
) {
    // Final output normally reaches EOF immediately. A descendant may retain
    // either pipe indefinitely, so bound only the wait to publish status;
    // keep collecting late output until EOF or explicit release.
    const EXIT_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_millis(50);
    let TerminalChannels {
        exit_tx,
        mut control_rx,
    } = channels;
    #[cfg(unix)]
    let pid = child.id();
    let mut stdout_buf = [0u8; 4096];
    let mut stderr_buf = [0u8; 4096];
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut exit_status = None;
    let mut exit_published = false;
    let drain_grace = tokio::time::sleep(EXIT_DRAIN_GRACE);
    tokio::pin!(drain_grace);

    loop {
        if stdout_done && stderr_done && exit_status.is_some() {
            if !exit_published {
                let _ = exit_tx.send(exit_status.clone());
            }
            break;
        }
        tokio::select! {
            control = control_rx.recv() => {
                // This task owns process cleanup. Once it completes, the
                // channel closes and subsequent controls become no-ops.
                #[cfg(unix)]
                if let Some(pid) = pid {
                    killpg(pid as i32, "KILL");
                }
                if exit_status.is_none() {
                    let _ = child.start_kill();
                }
                if !matches!(control, Some(TerminalControl::Kill)) {
                    // Release/channel closure discards subsequent output and
                    // reaps directly, without waiting for inherited pipe EOF.
                    if exit_status.is_none() {
                        exit_status = Some(terminal_exit_status(child.wait().await));
                    }
                    if !exit_published {
                        let _ = exit_tx.send(exit_status);
                    }
                    return;
                }
            }
            result = child.wait(), if exit_status.is_none() => {
                exit_status = Some(terminal_exit_status(result));
                drain_grace.as_mut().reset(tokio::time::Instant::now() + EXIT_DRAIN_GRACE);
            }
            () = &mut drain_grace, if exit_status.is_some() && !exit_published => {
                let _ = exit_tx.send(exit_status.clone());
                exit_published = true;
            }
            result = AsyncReadExt::read(&mut stdout, &mut stdout_buf), if !stdout_done => {
                match result {
                    Ok(0) | Err(_) => stdout_done = true,
                    Ok(n) => append_terminal_output(&output, &truncated, byte_limit, &stdout_buf[..n]),
                }
            }
            result = AsyncReadExt::read(&mut stderr, &mut stderr_buf), if !stderr_done => {
                match result {
                    Ok(0) | Err(_) => stderr_done = true,
                    Ok(n) => append_terminal_output(&output, &truncated, byte_limit, &stderr_buf[..n]),
                }
            }
        }
    }
}

fn terminal_exit_status(result: std::io::Result<std::process::ExitStatus>) -> TerminalExitStatus {
    match result {
        Ok(status) => {
            let mut es = TerminalExitStatus::new();
            if let Some(code) = status.code() {
                es = es.exit_code(code as u32);
            }
            es
        }
        Err(_) => TerminalExitStatus::new(),
    }
}

#[cfg(test)]
mod client_capabilities_tests {
    use super::*;
    use agent_client_protocol::schema::ProtocolVersion;

    /// Spur must announce the explicit, non-default `ClientCapabilities`
    /// literal at initialize: fs.read/write, terminal=true, and the
    /// `_meta.terminal_output` extension that unlocks codex's tool-call
    /// meta tunnelling. See design spec §6.2.
    #[test]
    fn spur_client_capabilities_advertises_terminal_fs_and_terminal_output_meta() {
        let caps = spur_client_capabilities();

        assert!(caps.terminal, "spur supports terminal/* methods");
        assert!(
            caps.fs.read_text_file,
            "spur supports fs/read_text_file requests"
        );
        assert!(
            caps.fs.write_text_file,
            "spur supports fs/write_text_file requests"
        );

        let meta = caps
            .meta
            .as_ref()
            .expect("client meta must include terminal_output gate");
        let terminal_output = meta
            .get("terminal_output")
            .and_then(serde_json::Value::as_bool)
            .expect("meta.terminal_output must be a bool");
        assert!(
            terminal_output,
            "meta.terminal_output must be true to unlock codex tool-call meta tunneling"
        );
    }

    #[test]
    fn spur_client_capabilities_advertises_only_supported_url_elicitation() {
        let json = serde_json::to_value(spur_client_capabilities()).expect("serialize caps");
        assert_eq!(json["elicitation"]["url"], serde_json::json!({}));
        assert!(
            json["elicitation"].get("form").is_none(),
            "Spur must not advertise form elicitation before it has a form renderer"
        );
    }

    #[test]
    fn spur_client_capabilities_advertises_native_subagent_sessions() {
        let json = serde_json::to_value(spur_client_capabilities()).expect("serialize caps");
        assert_eq!(json["_meta"]["jetbrains"]["air"]["version"], 1);
        assert!(json["_meta"]["jetbrains"]["air"]["capabilities"]
            .as_array()
            .expect("AIR capabilities array")
            .contains(&serde_json::json!("nativeSubagentSessions")),);
    }

    #[test]
    fn spur_client_capabilities_advertises_agent_file_change_reports() {
        let json = serde_json::to_value(spur_client_capabilities()).expect("serialize caps");
        assert!(json["_meta"]["jetbrains"]["air"]["capabilities"]
            .as_array()
            .expect("AIR capabilities array")
            .contains(&serde_json::json!("agentFileChangeReport")),);
    }

    /// The constructed `InitializeRequest` is what spur actually sends on
    /// the wire. Serialize the full thing and confirm the negotiated
    /// `clientCapabilities` shape includes the gate codex looks for.
    #[test]
    fn initialize_request_payload_contains_explicit_client_capabilities() {
        let caps = spur_client_capabilities();
        let req = InitializeRequest::new(ProtocolVersion::LATEST).client_capabilities(caps);
        let json = serde_json::to_value(&req).expect("InitializeRequest must serialize");

        let cc = json
            .get("clientCapabilities")
            .expect("clientCapabilities must serialize");
        assert_eq!(cc.get("terminal"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(
            cc.get("fs").and_then(|v| v.get("readTextFile")),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            cc.get("fs").and_then(|v| v.get("writeTextFile")),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            cc.get("_meta").and_then(|v| v.get("terminal_output")),
            Some(&serde_json::Value::Bool(true))
        );
    }
}

#[cfg(test)]
mod native_helper_tests {
    use super::*;
    use agent_client_protocol::role::UntypedRole;
    use agent_client_protocol::schema::v1::{
        AuthMethodId, CurrentModeUpdate, PermissionOption, PermissionOptionId,
        PermissionOptionKind, SessionConfigId, SessionMode, TextContent, ToolCallUpdate,
        ToolCallUpdateFields,
    };
    use agent_client_protocol::schema::ProtocolVersion;
    use agent_client_protocol::Channel;
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tokio::sync::mpsc::error::TryRecvError;

    fn expect_busy<T>(mut rx: oneshot::Receiver<anyhow::Result<T>>) {
        match rx.try_recv() {
            Ok(Err(err)) => {
                let msg = err.to_string();
                assert!(msg.contains("NativeAcpConnection 'busy-agent': busy"));
                assert!(msg.contains("prompt in flight"));
            }
            Ok(Ok(_)) => panic!("expected busy command to reply with an error"),
            Err(err) => panic!("expected busy command to send a reply: {err:?}"),
        }
    }

    fn raw_ext_params(value: serde_json::Value) -> Arc<serde_json::value::RawValue> {
        let raw: Box<serde_json::value::RawValue> =
            serde_json::value::to_raw_value(&value).expect("test JSON should serialize");
        raw.into()
    }

    #[test]
    fn auth_status_update_is_not_forwarded() {
        let payload = forwardable_ext_notification(
            "_auth/status_update".to_owned(),
            serde_json::json!({"authStatus": {"account": {"email": "private@example.invalid"}}}),
        );

        assert!(payload.is_none());
    }

    #[test]
    fn unrelated_extension_notification_is_forwarded_unchanged() {
        let params = serde_json::json!({"sessionId": "s1", "value": 7});
        let payload = forwardable_ext_notification("_vendor/update".to_owned(), params.clone())
            .expect("unrelated extension should be forwarded");

        assert_eq!(payload.method, "_vendor/update");
        assert_eq!(payload.params, params);
    }

    fn assert_no_more_agent_client_requests(
        rx: &mut mpsc::UnboundedReceiver<AgentClientRequestPayload>,
    ) {
        match rx.try_recv() {
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            Ok(payload) => panic!("unexpected extra agent-client request: {payload:?}"),
        }
    }

    #[test]
    fn acp_request_outcome_maps_ok_error_and_timeout() {
        assert_eq!(acp_request_outcome::<()>(Ok(())), TelemetryOutcome::Ok);
        assert_eq!(
            acp_request_outcome::<()>(Err(AcpRequestFailure::Error)),
            TelemetryOutcome::Error
        );
        assert_eq!(
            acp_request_outcome::<()>(Err(AcpRequestFailure::Timeout)),
            TelemetryOutcome::Timeout
        );
    }

    #[test]
    fn reject_busy_command_replies_busy_to_all_command_variants() {
        macro_rules! assert_busy {
            ($cmd:expr, $rx:expr) => {{
                reject_busy_command($cmd, "busy-agent", "prompt");
                expect_busy($rx);
            }};
        }

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::Initialize {
                request: InitializeRequest::new(ProtocolVersion::LATEST),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::NewSession {
                request: NewSessionRequest::new(PathBuf::from("/tmp/spur-new")),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        let (terminal_reply, _terminal_rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::Prompt {
                request: PromptRequest::new(
                    SessionId::new("sid"),
                    vec![ContentBlock::Text(TextContent::new("hello"))],
                ),
                reply,
                terminal_reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::Cancel {
                session_id: "sid".to_string(),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(AcpCommand::Shutdown { reply }, rx);

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::LoadSession {
                request: LoadSessionRequest::new(
                    "sid".to_string(),
                    PathBuf::from("/tmp/spur-load")
                ),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::ResumeSession {
                request: ResumeSessionRequest::new(
                    SessionId::new("sid"),
                    PathBuf::from("/tmp/spur-resume"),
                ),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::ListSessions {
                request: ListSessionsRequest::new(),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::DeleteSession {
                request: DeleteSessionRequest::new(SessionId::new("sid")),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::CloseSession {
                request: CloseSessionRequest::new(SessionId::new("sid")),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::SetSessionMode {
                request: SetSessionModeRequest::new(SessionId::new("sid"), "plan"),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::SetSessionConfigOption {
                request: SetSessionConfigOptionRequest::new(
                    SessionId::new("sid"),
                    SessionConfigId::new("model"),
                    SessionConfigValueId::new("gpt-5-codex"),
                ),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::Authenticate {
                request: AuthenticateRequest::new(AuthMethodId::new("github")),
                reply,
            },
            rx
        );

        let (reply, rx) = oneshot::channel();
        assert_busy!(
            AcpCommand::ExtMethod {
                request: ExtRequest::new("test/method", raw_ext_params(serde_json::json!({}))),
                reply,
            },
            rx
        );
    }

    async fn run_agent_client_request(
        method: &str,
        params: serde_json::Value,
    ) -> (
        agent_client_protocol::Result<serde_json::Value>,
        mpsc::UnboundedReceiver<AgentClientRequestPayload>,
    ) {
        let method = method.to_string();
        let (signal_tx, signal_rx) = mpsc::unbounded_channel();
        let local = tokio::task::LocalSet::new();

        let result = local
            .run_until(async move {
                let (server_channel, client_channel) = Channel::duplex();
                let signal_tx_for_handler = signal_tx.clone();
                let server = UntypedRole
                    .builder()
                    .on_receive_request(
                        async move |req: UntypedMessage, responder, _cx| {
                            handle_agent_client_request(
                                req,
                                responder,
                                &signal_tx_for_handler,
                                "test-agent",
                            )
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_request(
                        async |req: UntypedMessage, responder, _cx| {
                            responder.respond(serde_json::json!({ "fallback": req.method() }))
                        },
                        agent_client_protocol::on_receive_request!(),
                    );

                let server_task =
                    tokio::task::spawn_local(
                        async move { server.connect_to(server_channel).await },
                    );
                let client = UntypedRole.builder();
                let result = tokio::time::timeout(
                    Duration::from_secs(2),
                    client.connect_with(client_channel, async move |cx| {
                        let request = UntypedMessage::new(&method, params)?;
                        cx.send_request(request).block_task().await
                    }),
                )
                .await
                .expect("agent-client request should complete without timing out");
                server_task.abort();
                let _ = server_task.await;
                result
            })
            .await;

        (result, signal_rx)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handle_agent_client_request_logout_signals_and_acknowledges() {
        let (result, mut signal_rx) =
            run_agent_client_request("logout", serde_json::json!({})).await;

        assert_eq!(
            result.expect("logout should be acknowledged"),
            serde_json::json!({})
        );
        let payload = signal_rx
            .recv()
            .await
            .expect("logout should be forwarded to the bridge channel");
        assert_eq!(payload.kind, AgentClientRequestKind::Logout);
        assert_no_more_agent_client_requests(&mut signal_rx);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handle_agent_client_request_authenticate_signals_and_rejects() {
        let params = serde_json::to_value(AuthenticateRequest::new(AuthMethodId::new("github")))
            .expect("auth request should serialize");
        let (result, mut signal_rx) = run_agent_client_request("authenticate", params).await;

        let err = result.expect_err("authenticate should be rejected without forwarding config");
        let reason = err
            .data
            .as_ref()
            .and_then(|data| data.get("reason"))
            .and_then(serde_json::Value::as_str)
            .expect("auth rejection should explain why it was rejected");
        assert!(reason.contains("credential forwarding is not configured"));

        let payload = signal_rx
            .recv()
            .await
            .expect("authenticate should be forwarded to the bridge channel");
        assert_eq!(
            payload.kind,
            AgentClientRequestKind::Authenticate {
                method_id: "github".to_string(),
            }
        );
        assert_no_more_agent_client_requests(&mut signal_rx);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handle_agent_client_request_authenticate_uses_unknown_for_bad_params() {
        let (result, mut signal_rx) =
            run_agent_client_request("authenticate", serde_json::json!({ "bad": true })).await;

        result.expect_err("authenticate should still be rejected");
        let payload = signal_rx
            .recv()
            .await
            .expect("malformed authenticate should still be signaled");
        assert_eq!(
            payload.kind,
            AgentClientRequestKind::Authenticate {
                method_id: "<unknown>".to_string(),
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handle_agent_client_request_unknown_method_falls_through() {
        let (result, mut signal_rx) =
            run_agent_client_request("unknown/method", serde_json::json!({ "x": 1 })).await;

        assert_eq!(
            result.expect("fallback handler should answer unknown methods"),
            serde_json::json!({ "fallback": "unknown/method" })
        );
        assert_no_more_agent_client_requests(&mut signal_rx);
    }

    fn permission_option(id: &str, kind: PermissionOptionKind) -> PermissionOption {
        PermissionOption::new(PermissionOptionId::new(id), id, kind)
    }

    fn permission_request(options: Vec<PermissionOption>) -> RequestPermissionRequest {
        let tool_call = ToolCallUpdate::new("tool-call", ToolCallUpdateFields::new());
        RequestPermissionRequest::new("session", tool_call, options)
    }

    #[test]
    fn session_notification_gate_drops_updates_after_close() {
        let closed_sessions = Arc::new(Mutex::new(HashSet::new()));
        let notification = SessionNotification::new(
            SessionId::new("closed-session"),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("too late"),
            ))),
        );

        assert!(should_forward_session_notification(
            &closed_sessions,
            &notification
        ));
        closed_sessions
            .lock()
            .expect("closed-session set")
            .insert("closed-session".to_string());
        assert!(!should_forward_session_notification(
            &closed_sessions,
            &notification
        ));
    }

    #[test]
    fn reopening_session_clears_closed_marker_and_restores_updates() {
        let closed_sessions = Arc::new(Mutex::new(HashSet::from(["closed-session".to_string()])));
        let root_sessions = Arc::new(Mutex::new(HashSet::new()));
        let notification = SessionNotification::new(
            SessionId::new("closed-session"),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("visible again"),
            ))),
        );

        begin_session_open(&root_sessions, &closed_sessions, "closed-session");

        assert!(should_forward_session_notification(
            &closed_sessions,
            &notification
        ));
        assert!(root_sessions
            .lock()
            .expect("root-session set")
            .contains("closed-session"));
    }

    #[test]
    fn child_output_is_gated_until_spawn_is_observed() {
        let roots = Arc::new(Mutex::new(HashSet::from(["parent".to_string()])));
        let active = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(Mutex::new(HashSet::new()));
        let child_output = serde_json::json!({
            "sessionId": "child",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "child output"}
            }
        });

        let before_spawn =
            normalize_session_notification(child_output.clone(), &active, &roots, &closed)
                .expect("ordinary child output should parse")
                .expect("ordinary output should reach the session router");
        assert_eq!(
            route_session_notification(&before_spawn, &roots, &active, false),
            SessionNotificationRoute::Drop
        );

        normalize_session_notification(
            serde_json::json!({
                "sessionId": "parent",
                "update": {
                    "sessionUpdate": "subagent_spawned",
                    "subagentSessionId": "child",
                    "name": "researcher",
                    "task": "inspect Cargo.toml",
                    "capabilities": {}
                }
            }),
            &active,
            &roots,
            &closed,
        )
        .expect("spawn update should parse")
        .expect("spawn update should be emitted");

        let after_spawn = normalize_session_notification(child_output, &active, &roots, &closed)
            .expect("spawned child output should parse")
            .expect("spawned child output should reach the session router");
        assert_eq!(
            route_session_notification(&after_spawn, &roots, &active, false),
            SessionNotificationRoute::Forward
        );
    }

    #[test]
    fn early_new_session_updates_are_buffered_and_replayed_only_for_returned_root() {
        let roots = Arc::new(Mutex::new(HashSet::new()));
        let active = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(Mutex::new(HashSet::new()));
        let early_root = normalize_session_notification(
            serde_json::json!({
                "sessionId": "returned-root",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "ready"}
                }
            }),
            &active,
            &roots,
            &closed,
        )
        .expect("early root update should parse")
        .expect("ordinary updates must survive until routing");
        let unrelated = SessionNotification::new(
            SessionId::new("unrelated"),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("not this session"),
            ))),
        );

        assert_eq!(
            route_session_notification(&early_root, &roots, &active, true),
            SessionNotificationRoute::BufferUntilNewSession
        );
        let pending = Arc::new(Mutex::new(PendingNewSessionUpdates::default()));
        assert!(buffer_new_session_update(&pending, early_root, 128));
        assert!(buffer_new_session_update(&pending, unrelated, 128));
        begin_session_open(
            &roots,
            &Arc::new(Mutex::new(HashSet::new())),
            "returned-root",
        );
        let replay = take_matching_new_session_updates(&pending, "returned-root");

        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].session_id.0.as_ref(), "returned-root");
        assert_eq!(
            route_session_notification(&replay[0], &roots, &active, false),
            SessionNotificationRoute::Forward
        );
        assert!(pending
            .lock()
            .expect("pending updates")
            .notifications
            .is_empty());
    }

    #[test]
    fn early_new_session_buffer_fails_closed_at_count_or_byte_limit() {
        let notification = || {
            SessionNotification::new(
                SessionId::new("returned-root"),
                SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                    TextContent::new("ready"),
                ))),
            )
        };
        let pending = Arc::new(Mutex::new(PendingNewSessionUpdates::default()));

        for _ in 0..PENDING_NEW_SESSION_MAX_NOTIFICATIONS {
            assert!(buffer_new_session_update(&pending, notification(), 1));
        }
        assert!(!buffer_new_session_update(&pending, notification(), 1));
        {
            let pending = pending.lock().expect("pending updates");
            assert!(pending.overflowed);
            assert!(pending.notifications.is_empty());
            assert_eq!(pending.bytes, 0);
        }
        assert!(take_matching_new_session_updates(&pending, "returned-root").is_empty());

        assert!(!buffer_new_session_update(
            &pending,
            notification(),
            PENDING_NEW_SESSION_MAX_BYTES + 1,
        ));
        assert!(pending.lock().expect("pending updates").overflowed);
    }

    #[test]
    fn response_mode_baseline_precedes_replayed_early_mode_update() {
        let roots = Arc::new(Mutex::new(HashSet::new()));
        let closed = Arc::new(Mutex::new(HashSet::new()));
        let in_flight = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(Mutex::new(PendingNewSessionUpdates::default()));
        assert!(buffer_new_session_update(
            &pending,
            SessionNotification::new(
                SessionId::new("returned-root"),
                SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("early-update")),
            ),
            128,
        ));
        let session_modes = Arc::new(Mutex::new(HashMap::new()));
        let models = Arc::new(Mutex::new(HashMap::new()));
        let efforts = Arc::new(Mutex::new(HashMap::new()));
        let (ext_tx, _ext_rx) = mpsc::unbounded_channel();
        let (session_tx, _session_rx) = tokio::sync::broadcast::channel(8);
        let sink = SessionNotificationSink {
            closed_sessions: closed.clone(),
            standardizer: Arc::new(Mutex::new(
                crate::adapter::SessionEventStandardizer::for_agent(AgentKind::CodexAcp),
            )),
            pending_air_report_requests: Arc::new(Mutex::new(HashMap::new())),
            ext_notification_tx: ext_tx,
            session_modes: session_modes.clone(),
            grok_session_models: models.clone(),
            session_efforts: efforts.clone(),
            last_prompt_usage: Arc::new(Mutex::new(None)),
            session_notif_tx: session_tx,
        };
        let response =
            NewSessionResponse::new(SessionId::new("returned-root")).modes(SessionModeState::new(
                SessionModeId::new("response-baseline"),
                vec![SessionMode::new(
                    SessionModeId::new("early-update"),
                    "Early update",
                )],
            ));

        complete_new_session(
            &response,
            &roots,
            &closed,
            &in_flight,
            &pending,
            &sink,
            &session_modes,
            &models,
            &efforts,
        );

        assert_eq!(
            session_modes
                .lock()
                .expect("session modes")
                .get("returned-root")
                .and_then(|snapshot| snapshot.current_mode_id.as_ref())
                .map(|id| id.0.as_ref()),
            Some("early-update"),
        );
    }

    #[test]
    fn closing_root_retires_all_subagent_descendants() {
        let roots = Arc::new(Mutex::new(HashSet::from(["root".to_string()])));
        let closed = Arc::new(Mutex::new(HashSet::from(["root".to_string()])));
        let active = Arc::new(Mutex::new(HashMap::from([
            ("child".to_string(), "root".to_string()),
            ("grandchild".to_string(), "child".to_string()),
            ("other-child".to_string(), "other-root".to_string()),
        ])));

        remove_subagent_descendants(&active, "root");

        let active_guard = active.lock().expect("active subagents");
        assert!(!active_guard.contains_key("child"));
        assert!(!active_guard.contains_key("grandchild"));
        assert!(active_guard.contains_key("other-child"));
        drop(active_guard);

        let late_child = SessionNotification::new(
            SessionId::new("child"),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("too late"),
            ))),
        );
        assert_eq!(
            route_session_notification(&late_child, &roots, &active, false),
            SessionNotificationRoute::Drop
        );
        assert!(normalize_session_notification(
            serde_json::json!({
                "sessionId": "root",
                "update": {
                    "sessionUpdate": "subagent_spawned",
                    "subagentSessionId": "late-child",
                    "name": "late",
                    "task": "must stay closed"
                }
            }),
            &active,
            &roots,
            &closed,
        )
        .expect("late spawn should be structurally valid")
        .is_none());
        assert!(!active
            .lock()
            .expect("active subagents")
            .contains_key("late-child"));
    }

    #[test]
    fn v1_7_subagent_updates_become_one_parent_tool_lifecycle() {
        let active = Arc::new(Mutex::new(HashMap::new()));
        let roots = Arc::new(Mutex::new(HashSet::from(["parent".to_string()])));
        let closed = Arc::new(Mutex::new(HashSet::new()));
        let spawned = normalize_session_notification(
            serde_json::json!({
                "sessionId": "parent",
                "update": {
                    "sessionUpdate": "subagent_spawned",
                    "subagentSessionId": "child",
                    "name": "researcher",
                    "task": "inspect Cargo.toml",
                    "capabilities": {"cancel": true, "close": true}
                }
            }),
            &active,
            &roots,
            &closed,
        )
        .expect("spawn update should parse")
        .expect("first spawn should be emitted");

        assert_eq!(spawned.session_id.0.as_ref(), "parent");
        let SessionUpdate::ToolCall(tool_call) = spawned.update else {
            panic!("spawn must normalize to ToolCall")
        };
        assert_eq!(tool_call.tool_call_id.0.as_ref(), "subagent:child");
        assert_eq!(tool_call.title, "Subagent: researcher");
        assert_eq!(tool_call.status, ToolCallStatus::InProgress);

        let terminal_json = serde_json::json!({
            "sessionId": "parent",
            "update": {
                "sessionUpdate": "subagent_state_update",
                "subagentSessionId": "child",
                "state": "completed"
            }
        });
        let terminal =
            normalize_session_notification(terminal_json.clone(), &active, &roots, &closed)
                .expect("state update should parse")
                .expect("first terminal update should be emitted");
        let SessionUpdate::ToolCallUpdate(update) = terminal.update else {
            panic!("terminal state must normalize to ToolCallUpdate")
        };
        assert_eq!(update.tool_call_id.0.as_ref(), "subagent:child");
        assert_eq!(update.fields.status, Some(ToolCallStatus::Completed));

        assert!(
            normalize_session_notification(terminal_json, &active, &roots, &closed)
                .expect("duplicate state update should be valid but ignored")
                .is_none(),
            "a child session must emit at most one terminal parent update"
        );
    }

    #[test]
    fn air_file_change_report_requires_matching_request_id_once() {
        let pending = Arc::new(Mutex::new(HashMap::from([(
            "parent".to_string(),
            "report-17".to_string(),
        )])));
        let notification: SessionNotification = serde_json::from_value(serde_json::json!({
            "sessionId": "parent",
            "update": {
                "sessionUpdate": "session_info_update",
                "_meta": {
                    "jetbrains": {
                        "air": {
                            "version": 1,
                            "agentFileChangeReport": {
                                "version": 1,
                                "requestId": "report-17",
                                "status": "reported",
                                "paths": ["src/lib.rs"],
                                "declaredComplete": true,
                                "truncated": false
                            }
                        }
                    }
                }
            }
        }))
        .expect("AIR report fixture should be a valid SessionNotification");

        let report = correlated_air_file_change_report(&notification, &pending)
            .expect("matching report should be forwarded");
        assert_eq!(report["requestId"], "report-17");
        assert_eq!(report["paths"], serde_json::json!(["src/lib.rs"]));
        assert!(
            correlated_air_file_change_report(&notification, &pending).is_none(),
            "duplicate terminal reports must be suppressed"
        );
    }

    #[test]
    fn pending_air_report_is_bounded_to_one_request_per_session() {
        let pending = Arc::new(Mutex::new(HashMap::new()));

        register_pending_air_report(&pending, "session-1", "report-1");
        register_pending_air_report(&pending, "session-1", "report-2");
        register_pending_air_report(&pending, "session-2", "report-3");

        assert_eq!(
            pending.lock().expect("pending AIR map").clone(),
            HashMap::from([
                ("session-1".to_string(), "report-2".to_string()),
                ("session-2".to_string(), "report-3".to_string()),
            ])
        );
        clear_pending_air_report(&pending, "session-1");
        assert!(!pending
            .lock()
            .expect("pending AIR map")
            .contains_key("session-1"));
    }

    fn selected_option_id(response: RequestPermissionResponse) -> String {
        match response.outcome {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome { option_id, .. }) => {
                option_id.0.to_string()
            }
            other => panic!("expected selected permission outcome, got {other:?}"),
        }
    }

    fn assert_cancelled(response: RequestPermissionResponse) {
        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Cancelled
        ));
    }

    #[tokio::test]
    async fn handle_request_permission_without_channel_cancels_without_user_choice() {
        let args = permission_request(vec![
            permission_option("reject_once", PermissionOptionKind::RejectOnce),
            permission_option("allow_once", PermissionOptionKind::AllowOnce),
        ]);

        let response = handle_request_permission(args, None, None)
            .await
            .expect("auto approve should succeed");

        assert_cancelled(response);
    }

    #[tokio::test]
    async fn handle_request_permission_forwards_to_channel_and_uses_reply() {
        let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
        let stamp = crate::types::PermissionLeaseStamp::new();
        stamp.set_generation(7);
        stamp.set_session_fence("session", 3);
        let args = permission_request(vec![permission_option(
            "allow_once",
            PermissionOptionKind::AllowOnce,
        )]);
        let task = tokio::spawn(handle_request_permission(
            args.clone(),
            Some(permission_tx),
            Some(stamp),
        ));

        let request = permission_rx
            .recv()
            .await
            .expect("permission request should be forwarded");
        assert_eq!(request.args, args);
        assert_eq!(request.generation, 7);
        assert_eq!(request.operation_fence, 3);
        request
            .reply_tx
            .send(crate::types::PermissionResponse {
                option_id: "allow_once".to_string(),
            })
            .expect("permission handler should still be awaiting the reply");

        let response = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("permission handler should finish")
            .expect("permission task should not panic")
            .expect("permission handler should return a response");
        assert_eq!(selected_option_id(response), "allow_once");
    }

    #[tokio::test]
    async fn handle_request_permission_auto_denies_when_channel_closed() {
        let (permission_tx, permission_rx) = mpsc::unbounded_channel();
        drop(permission_rx);
        let args = permission_request(vec![
            permission_option("allow_once", PermissionOptionKind::AllowOnce),
            permission_option("reject_once", PermissionOptionKind::RejectOnce),
        ]);

        let response = handle_request_permission(args, Some(permission_tx), None)
            .await
            .expect("closed channel fallback should succeed");

        assert_cancelled(response);
    }

    #[tokio::test]
    async fn handle_request_permission_stamps_fence_at_handler_entry() {
        let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
        let stamp = crate::types::PermissionLeaseStamp::new();
        stamp.set_generation(1);
        stamp.set_session_fence("session", 1);
        let args = permission_request(vec![
            permission_option("allow_once", PermissionOptionKind::AllowOnce),
            permission_option("reject_once", PermissionOptionKind::RejectOnce),
        ]);
        let stamp_for_task = stamp.clone();
        let task = tokio::spawn(handle_request_permission(
            args,
            Some(permission_tx),
            Some(stamp_for_task),
        ));

        // Rotate the live lease after the handler has already started and
        // stamped fence=1 — the forwarded request must keep the entry fence.
        let request = permission_rx
            .recv()
            .await
            .expect("permission request should be forwarded");
        stamp.set_generation(2);
        stamp.set_session_fence("session", 9);
        assert_eq!(request.generation, 1);
        assert_eq!(request.operation_fence, 1);
        drop(request.reply_tx);

        let response = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("permission handler should finish")
            .expect("permission task should not panic")
            .expect("permission handler should return auto-deny response");
        assert_cancelled(response);
    }

    #[tokio::test]
    async fn handle_request_permission_auto_denies_when_reply_dropped() {
        let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
        let args = permission_request(vec![
            permission_option("allow_once", PermissionOptionKind::AllowOnce),
            permission_option("reject_once", PermissionOptionKind::RejectOnce),
        ]);
        let task = tokio::spawn(handle_request_permission(args, Some(permission_tx), None));

        let request = permission_rx
            .recv()
            .await
            .expect("permission request should be forwarded");
        drop(request.reply_tx);

        let response = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("permission handler should finish after reply drop")
            .expect("permission task should not panic")
            .expect("permission handler should return auto-deny response");
        assert_cancelled(response);
    }

    #[test]
    fn permission_reply_timeout_is_unbounded_for_interactive_users() {
        // Users step away from the terminal; an unanswered prompt must wait
        // for the user's reply rather than silently deny after a fixed cap.
        assert!(
            PERMISSION_REPLY_TIMEOUT.is_none(),
            "interactive permission prompts must wait for the user's reply"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn handle_request_permission_waits_beyond_legacy_deadline_for_reply() {
        let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
        let args = permission_request(vec![permission_option(
            "allow_once",
            PermissionOptionKind::AllowOnce,
        )]);
        let task = tokio::spawn(handle_request_permission(args, Some(permission_tx), None));

        let request = permission_rx
            .recv()
            .await
            .expect("permission request should be forwarded");

        // Simulate the user answering 61s in — past the legacy 60s cap.
        tokio::time::advance(Duration::from_secs(61)).await;
        request
            .reply_tx
            .send(crate::types::PermissionResponse {
                option_id: "allow_once".to_string(),
            })
            .expect("permission handler should still be awaiting the reply");

        let response = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("permission handler should finish")
            .expect("permission task should not panic")
            .expect("permission handler should return a response");
        assert_eq!(selected_option_id(response), "allow_once");
    }

    #[test]
    fn append_terminal_output_appends_without_limit() {
        let output = Arc::new(Mutex::new(String::new()));
        let truncated = Arc::new(AtomicBool::new(false));

        append_terminal_output(&output, &truncated, None, b"hello ");
        append_terminal_output(&output, &truncated, None, b"world");

        assert_eq!(&*output.lock().unwrap(), "hello world");
        assert!(!truncated.load(Ordering::Relaxed));
    }

    #[test]
    fn append_terminal_output_truncates_on_utf8_boundary() {
        let output = Arc::new(Mutex::new(String::new()));
        let truncated = Arc::new(AtomicBool::new(false));

        append_terminal_output(&output, &truncated, Some(4), "abcé456".as_bytes());

        assert_eq!(&*output.lock().unwrap(), "456");
        assert!(truncated.load(Ordering::Relaxed));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_create_stdio_nulls_stdin_and_read_reaches_eof() {
        let tty_stdin = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/ptmx")
            .expect("PTY master should provide a TTY stdin sentinel");
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("if test -t 0; then exit 97; fi; if read x; then exit 98; fi")
            .stdin(std::process::Stdio::from(tty_stdin))
            .kill_on_drop(true);
        configure_terminal_create_stdio(&mut command);
        detach_acp_child_session(&mut command);

        let mut child = command.spawn().expect("test child should spawn");
        let status = match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(status) => status.expect("test child should be waitable"),
            Err(_) => {
                let _ = child.kill().await;
                panic!("terminal/create child did not exit after stdin should have reached EOF");
            }
        };

        assert_ne!(status.code(), Some(97), "child fd0 remained a TTY");
        assert_ne!(
            status.code(),
            Some(98),
            "read unexpectedly consumed stdin instead of observing EOF"
        );
        assert!(
            status.success(),
            "child should exit cleanly after read observes EOF: {status}"
        );
    }

    /// Regression for SIGTTIN under `spur tui --brain codex`: process_group(0)
    /// alone keeps the child in the TUI session; setsid must create a new one.
    #[cfg(unix)]
    #[tokio::test]
    #[expect(
        unsafe_code,
        reason = "test inspects session/pgid via getsid/getpgid after setsid detach"
    )]
    async fn acp_child_session_detach_uses_new_session_and_pgid_equals_pid() {
        use std::{
            fs,
            time::{Duration, Instant},
        };

        let temp = tempfile::tempdir().expect("tempdir");
        let hold_path = temp.path().join("hold");
        let pid_path = temp.path().join("child.pid");
        fs::write(&hold_path, b"").expect("hold file");

        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!(
                "printf '%s\\n' \"$$\" > '{}'; while [ -e '{}' ]; do sleep 0.05; done",
                pid_path.display(),
                hold_path.display()
            ))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        detach_acp_child_session(&mut command);

        let mut child = command.spawn().expect("spawn detached child");
        let deadline = Instant::now() + Duration::from_secs(5);
        let child_pid = loop {
            match fs::read_to_string(&pid_path) {
                Ok(text) => {
                    break text
                        .trim()
                        .parse::<i32>()
                        .expect("numeric child pid from shell $$")
                }
                Err(err)
                    if err.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(err) => panic!("read child pid: {err}"),
            }
        };

        // SAFETY: getsid/getpgid only inspect kernel process metadata.
        let parent_sid = unsafe { libc::getsid(0) };
        let child_sid = unsafe { libc::getsid(child_pid) };
        let child_pgid = unsafe { libc::getpgid(child_pid) };
        assert_ne!(parent_sid, -1, "parent getsid failed");
        assert_ne!(child_sid, -1, "child getsid failed");
        assert_ne!(child_pgid, -1, "child getpgid failed");
        assert_ne!(
            child_sid, parent_sid,
            "ACP child must not share Spur's controlling session (SIGTTIN risk)"
        );
        assert_eq!(
            child_pgid, child_pid,
            "setsid child must remain killpg-reachable via pid-as-pgid"
        );

        // Without a controlling TTY, /dev/tty open fails (ENXIO) instead of
        // stopping the process with SIGTTIN in a shared session.
        let mut tty_probe = tokio::process::Command::new("/bin/sh");
        tty_probe
            .arg("-c")
            .arg("exec 3<>/dev/tty 2>/dev/null; echo $?")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        detach_acp_child_session(&mut tty_probe);
        let output = tty_probe.output().await.expect("tty probe should run");
        let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_ne!(
            code, "0",
            "detached child must not successfully open /dev/tty; got exit echo {code:?}"
        );

        let _ = fs::remove_file(&hold_path);
        let _ = child.kill().await;
        let _ = child.wait().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_reader_captures_stdout_stderr_and_exit_code() {
        let mut child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("printf stdout; printf stderr >&2; exit 7")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("test child should spawn");
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");
        let output = Arc::new(Mutex::new(String::new()));
        let truncated = Arc::new(AtomicBool::new(false));
        let (exit_tx, exit_rx) = tokio::sync::watch::channel(None);
        let (_control_tx, control_rx) = mpsc::unbounded_channel();

        tokio::time::timeout(
            Duration::from_secs(2),
            terminal_reader(
                stdout,
                stderr,
                child,
                output.clone(),
                truncated.clone(),
                None,
                TerminalChannels {
                    exit_tx,
                    control_rx,
                },
            ),
        )
        .await
        .expect("terminal reader should finish");

        let captured = output.lock().unwrap().clone();
        assert!(captured.contains("stdout"), "captured output: {captured:?}");
        assert!(captured.contains("stderr"), "captured output: {captured:?}");
        assert!(!truncated.load(Ordering::Relaxed));
        let status = exit_rx
            .borrow()
            .clone()
            .expect("terminal reader should publish exit status");
        assert_eq!(status.exit_code, Some(7));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn initialize_sets_ready_and_caches_session_capabilities() {
        let stub = format!(
            "{}/tests/fixtures/load_error_stub.py",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut conn = NativeAcpConnection::new("init-stub", "python3", vec![stub], None);

        let response = conn
            .initialize(InitializeRequest::new(ProtocolVersion::LATEST))
            .await
            .expect("initialize should succeed against the existing stub");

        assert_eq!(conn.health(), AgentHealth::Ready);
        assert!(response.agent_capabilities.load_session);
        assert_eq!(
            *conn.session_capabilities.lock().unwrap(),
            Some(response.agent_capabilities.session_capabilities.clone())
        );

        conn.shutdown()
            .await
            .expect("initialized stub should shut down cleanly");
    }

    #[cfg(unix)]
    async fn initialize_env_probe(
        shell_probe: &str,
        launch_env: std::collections::BTreeMap<String, String>,
    ) -> String {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let observed_path = tempdir.path().join("observed-env");
        let stub = format!(
            "{}/tests/fixtures/load_error_stub.py",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut conn = NativeAcpConnection::new(
            "env-probe",
            "/bin/sh",
            vec![
                "-c".to_string(),
                shell_probe.to_string(),
                "spur-native-env-test".to_string(),
                observed_path.display().to_string(),
                stub,
            ],
            None,
        );
        conn.set_repo_root(tempdir.path().to_path_buf());
        conn.set_launch_env(launch_env);

        conn.initialize(InitializeRequest::new(ProtocolVersion::LATEST))
            .await
            .expect("environment-probe child should initialize");
        let observed = std::fs::read_to_string(&observed_path)
            .expect("environment-probe child should write its observation");

        conn.shutdown()
            .await
            .expect("environment-probe child should shut down cleanly");
        observed
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn native_child_receives_injected_launch_env() {
        let launch_env = std::collections::BTreeMap::from([(
            "SPUR_NATIVE_ENV_PROBE".to_string(),
            "per-attempt-value".to_string(),
        )]);

        let observed = initialize_env_probe(
            "printf '%s' \"${SPUR_NATIVE_ENV_PROBE-<unset>}\" > \"$1\"; exec python3 \"$2\"",
            launch_env,
        )
        .await;

        assert_eq!(observed, "per-attempt-value");
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn empty_launch_env_preserves_parent_inheritance() {
        let expected_home = std::env::var("HOME").expect("test process should define HOME");

        let observed = initialize_env_probe(
            "printf '%s' \"${HOME-<unset>}\" > \"$1\"; exec python3 \"$2\"",
            std::collections::BTreeMap::new(),
        )
        .await;

        assert_eq!(observed, expected_home);
    }
}

#[cfg(test)]
mod session_mode_cache_tests {
    use super::*;
    use crate::connection::AgentConnection;
    use agent_client_protocol::schema::v1::{CurrentModeUpdate, SessionMode};
    use tokio::sync::mpsc;

    fn mode_state(current: &str, available: &[&str]) -> SessionModeState {
        SessionModeState::new(
            SessionModeId::new(current),
            available
                .iter()
                .map(|id| SessionMode::new(SessionModeId::new(*id), *id))
                .collect(),
        )
    }

    fn ids(modes: &[SessionModeId]) -> Vec<&str> {
        modes.iter().map(|id| id.0.as_ref()).collect()
    }

    #[test]
    fn mode_snapshot_from_session_state_retains_current_and_available_ids() {
        let conn = NativeAcpConnection::new("test-agent", "/bin/false", vec![], None);
        let sid = SessionId::new("codex-sid");

        cache_session_modes(
            &conn.session_modes,
            &sid,
            Some(&mode_state(
                "agent-full-access",
                &["read-only", "agent", "agent-full-access"],
            )),
        );

        let snapshot = conn
            .session_mode_snapshot(&sid)
            .expect("mode snapshot should be cached");
        assert_eq!(
            snapshot.current_mode_id.as_ref().map(|id| id.0.as_ref()),
            Some("agent-full-access"),
        );
        assert_eq!(
            ids(&snapshot.available_modes),
            vec!["read-only", "agent", "agent-full-access"],
        );
        assert_eq!(
            ids(&conn.advertised_session_modes(&sid).unwrap()),
            vec!["read-only", "agent", "agent-full-access"],
            "legacy advertised-mode getter should preserve validation behavior",
        );
    }

    #[test]
    fn current_mode_update_replaces_current_mode_without_rewriting_available_modes() {
        let conn = NativeAcpConnection::new("test-agent", "/bin/false", vec![], None);
        let sid = SessionId::new("codex-sid");
        cache_session_modes(
            &conn.session_modes,
            &sid,
            Some(&mode_state("agent-full-access", &["read-only", "agent"])),
        );

        let notification = SessionNotification::new(
            sid.clone(),
            SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("read-only")),
        );
        cache_session_notification_mode_update(&conn.session_modes, &notification);

        let snapshot = conn
            .session_mode_snapshot(&sid)
            .expect("mode snapshot should remain cached");
        assert_eq!(
            snapshot.current_mode_id.as_ref().map(|id| id.0.as_ref()),
            Some("read-only"),
        );
        assert_eq!(
            ids(&snapshot.available_modes),
            vec!["read-only", "agent"],
            "mode updates should not discard advertised modes",
        );
    }

    #[tokio::test]
    async fn successful_set_session_mode_leaves_cached_current_until_update_arrives() {
        let mut conn = NativeAcpConnection::new("test-agent", "/bin/false", vec![], None);
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        conn.cmd_tx = Some(cmd_tx);
        let sid = SessionId::new("codex-sid");
        cache_session_modes(
            &conn.session_modes,
            &sid,
            Some(&mode_state(
                "agent-full-access",
                &["read-only", "agent-full-access"],
            )),
        );

        let request = SetSessionModeRequest::new(sid.clone(), "read-only");
        let handle = tokio::spawn(async move {
            let result = conn.set_session_mode(request).await;
            (conn, result)
        });

        match cmd_rx
            .recv()
            .await
            .expect("set_mode command should be sent")
        {
            AcpCommand::SetSessionMode { request, reply } => {
                assert_eq!(request.session_id, sid);
                assert_eq!(request.mode_id, SessionModeId::new("read-only"));
                reply
                    .send(Ok(SetSessionModeResponse::new()))
                    .expect("test receiver must still be waiting");
            }
            _ => panic!("expected SetSessionMode command"),
        }

        let (conn, response) = handle.await.expect("set_mode task must not panic");
        response.expect("set_mode should succeed");
        let snapshot = conn
            .session_mode_snapshot(&SessionId::new("codex-sid"))
            .expect("mode snapshot should remain cached");
        assert_eq!(
            snapshot.current_mode_id.as_ref().map(|id| id.0.as_ref()),
            Some("agent-full-access"),
            "set_mode response alone is not ACP-visible mode state",
        );
    }
}

#[cfg(test)]
mod set_session_model_dispatch_tests {
    use super::{
        cache_grok_model_changed, decide_set_session_model_dispatch, direct_set_model_params,
        ClientRequest, ExtRequest, SetSessionModelDispatch,
    };
    use crate::connection::AgentConnection;
    use crate::SpurAgentCaps;
    use agent_client_protocol::schema::v1::{
        InitializeResponse, NewSessionResponse, SessionConfigId, SessionConfigKind,
        SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
        SessionConfigSelectOption, SessionConfigSelectOptions, SessionConfigValueId, SessionId,
    };
    use agent_client_protocol::schema::ProtocolVersion;

    fn caps_from(modify: impl FnOnce(&mut NewSessionResponse)) -> SpurAgentCaps {
        let init = InitializeResponse::new(ProtocolVersion::LATEST);
        let mut new = NewSessionResponse::new(SessionId::new("test"));
        modify(&mut new);
        SpurAgentCaps::new(&init, &new, crate::AgentKind::CodexAcp)
    }

    fn model_option(current: &str, choices: &[(&str, &str)]) -> SessionConfigOption {
        SessionConfigOption::select(
            SessionConfigId::new("model"),
            "Model",
            SessionConfigValueId::new(current),
            choices
                .iter()
                .map(|(value, name)| {
                    SessionConfigSelectOption::new(SessionConfigValueId::new(*value), *name)
                })
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn caps_with_model_config_option_routes_fallback() {
        let caps = caps_from(|n| {
            n.config_options = Some(vec![model_option(
                "gpt-5-codex",
                &[("gpt-5-codex", "GPT-5 Codex")],
            )]);
        });
        assert!(caps.supports_set_model());
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::FallbackConfigOption
        ));
    }

    #[test]
    fn caps_with_category_model_option_routes_fallback() {
        let caps = caps_from(|n| {
            n.config_options = Some(vec![model_option(
                "gpt-5-codex",
                &[("gpt-5-codex", "GPT-5 Codex")],
            )
            .category(SessionConfigOptionCategory::Model)]);
        });
        assert!(caps.supports_set_model());
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::FallbackConfigOption
        ));
    }

    #[test]
    fn caps_with_unusable_model_option_routes_unsupported() {
        let caps = caps_from(|n| {
            n.config_options = Some(vec![SessionConfigOption::new(
                SessionConfigId::new("model"),
                "Model",
                SessionConfigKind::Select(SessionConfigSelect::new(
                    SessionConfigValueId::new("default"),
                    SessionConfigSelectOptions::Ungrouped(vec![]),
                )),
            )]);
        });
        assert!(!caps.supports_set_model());
        assert!(caps.supports_set_config_option());
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::Unsupported
        ));
    }

    #[test]
    fn caps_with_unrelated_config_options_routes_unsupported() {
        let caps = caps_from(|n| {
            n.config_options = Some(vec![SessionConfigOption::select(
                SessionConfigId::new("reasoning_effort"),
                "Reasoning effort",
                SessionConfigValueId::new("medium"),
                vec![SessionConfigSelectOption::new("medium", "Medium")],
            )]);
        });
        assert!(!caps.supports_set_model());
        assert!(caps.supports_set_config_option());
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::Unsupported
        ));
    }

    #[test]
    fn caps_with_neither_routes_unsupported() {
        let caps = caps_from(|_| {});
        assert!(!caps.supports_set_model());
        assert!(!caps.supports_set_config_option());
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::Unsupported
        ));
    }

    #[test]
    fn model_config_option_routes_fallback_with_additional_options() {
        let caps = caps_from(|n| {
            n.config_options = Some(vec![
                model_option("gpt-5-codex", &[("gpt-5-codex", "GPT-5 Codex")]),
                SessionConfigOption::new(
                    SessionConfigId::new("reasoning_effort"),
                    "Reasoning effort",
                    SessionConfigKind::Select(SessionConfigSelect::new(
                        SessionConfigValueId::new("medium"),
                        SessionConfigSelectOptions::Ungrouped(vec![]),
                    )),
                ),
            ]);
        });
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::FallbackConfigOption
        ));
    }

    #[test]
    fn grok_meta_catalog_routes_direct_set_model_without_config_options() {
        let mut init = InitializeResponse::new(ProtocolVersion::LATEST);
        init.meta = Some(
            serde_json::json!({
                "modelState": {
                    "currentModelId": "grok-4.5",
                    "availableModels": [{
                        "modelId": "grok-4.5",
                        "name": "Grok 4.5",
                        "_meta": {
                            "reasoningEfforts": [{"id": "low", "label": "Low Effort"}]
                        }
                    }]
                }
            })
            .as_object()
            .expect("meta fixture must be an object")
            .clone(),
        );
        let new = NewSessionResponse::new(SessionId::new("test"));
        let caps = SpurAgentCaps::new(&init, &new, crate::AgentKind::Grok);

        assert!(!caps.supports_set_model());
        assert!(!caps.supports_set_config_option());
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::DirectSetModel
        ));
    }

    #[test]
    fn kiro_recovered_models_catalog_routes_direct_set_model() {
        let init = InitializeResponse::new(ProtocolVersion::LATEST);
        let mut new = NewSessionResponse::new(SessionId::new("test"));
        new.meta = Some(
            serde_json::json!({
                "spur.recoveredModels": {
                    "availableModels": [
                        {"modelId": "auto", "name": "auto"},
                        {"modelId": "claude-sonnet-4.5", "name": "claude-sonnet-4.5"}
                    ],
                    "currentModelId": "claude-sonnet-4.5"
                }
            })
            .as_object()
            .expect("meta fixture must be an object")
            .clone(),
        );
        let caps = SpurAgentCaps::new(&init, &new, crate::AgentKind::Kiro);

        assert!(!caps.supports_set_model());
        assert!(!caps.supports_set_config_option());
        assert!(caps.supports_kiro_set_model());
        assert!(caps.supports_direct_set_model());
        assert_eq!(
            caps.current_model_label().as_deref(),
            Some("claude-sonnet-4.5")
        );
        assert!(matches!(
            decide_set_session_model_dispatch(&caps),
            SetSessionModelDispatch::DirectSetModel
        ));
    }

    #[test]
    fn new_session_from_raw_value_recovers_models_plane() {
        let raw = serde_json::json!({
            "sessionId": "sid-kiro",
            "modes": {
                "currentModeId": "kiro_default",
                "availableModes": [{"id": "kiro_default", "name": "kiro_default"}]
            },
            "models": {
                "availableModels": [
                    {"modelId": "auto", "name": "auto", "description": "task-picked"}
                ],
                "currentModelId": "auto"
            }
        });
        let response = super::new_session_from_raw_value(raw).expect("typed response");
        assert_eq!(response.session_id.0.as_ref(), "sid-kiro");
        let models = response
            .meta
            .as_ref()
            .and_then(|m| m.get("spur.recoveredModels"))
            .expect("recovered models meta");
        assert_eq!(
            models.get("currentModelId").and_then(|v| v.as_str()),
            Some("auto")
        );
    }

    #[test]
    fn grok_effort_params_use_only_meta_reasoning_effort() {
        let params = direct_set_model_params(&SessionId::new("sid"), "grok-4.5", Some("low"));

        assert_eq!(
            params,
            serde_json::json!({
                "sessionId": "sid",
                "modelId": "grok-4.5",
                "_meta": {"reasoningEffort": "low"}
            })
        );
        assert!(params.get("reasoningEffort").is_none());
        assert!(params.get("effort").is_none());
    }

    #[test]
    fn grok_direct_request_keeps_exact_standard_method_name() {
        let raw = serde_json::value::to_raw_value(&serde_json::json!({
            "sessionId": "sid",
            "modelId": "grok-4.5"
        }))
        .expect("params must serialize");
        let request = ClientRequest::ExtMethodRequest(ExtRequest::new(
            "session/set_model",
            std::sync::Arc::from(raw),
        ));

        assert_eq!(
            agent_client_protocol::JsonRpcMessage::method(&request),
            "session/set_model"
        );
    }

    #[test]
    fn model_changed_notification_updates_native_effort_model_cache() {
        let cache = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        cache_grok_model_changed(
            &cache,
            &serde_json::json!({
                "sessionId": "sid",
                "update": {
                    "sessionUpdate": "model_changed",
                    "model_id": "grok-composer-2.5-fast"
                }
            }),
        );

        assert_eq!(
            cache
                .lock()
                .expect("cache lock must remain healthy")
                .get("sid")
                .map(String::as_str),
            Some("grok-composer-2.5-fast")
        );
    }

    #[tokio::test]
    async fn set_session_model_returns_capability_missing_when_unsupported() {
        let caps = caps_from(|_| {});
        let mut conn = super::NativeAcpConnection::new(
            "test-agent".to_string(),
            "/bin/false".to_string(),
            vec![],
            None,
        );
        let res = conn
            .set_session_model(SessionId::new("sid"), "m".to_string(), &caps)
            .await;
        match res {
            Err(crate::AcpError::CapabilityMissing(name)) => assert_eq!(name, "set_model"),
            other => panic!("expected CapabilityMissing(\"set_model\"), got {other:?}"),
        }
    }
}

#[cfg(test)]
mod resume_delete_dispatch_tests {
    use std::path::PathBuf;

    use super::{AcpCommand, NativeAcpConnection};
    use crate::connection::AgentConnection;
    use crate::AcpError;
    use agent_client_protocol::schema::v1::{
        CloseSessionRequest, CloseSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
        ListSessionsRequest, ListSessionsResponse, NewSessionResponse, ResumeSessionRequest,
        ResumeSessionResponse, SessionCapabilities, SessionCloseCapabilities,
        SessionDeleteCapabilities, SessionId, SessionListCapabilities, SessionResumeCapabilities,
    };
    use tokio::sync::mpsc::{self, error::TryRecvError};

    fn connection_with_command_channel(
    ) -> (NativeAcpConnection, mpsc::UnboundedReceiver<AcpCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut conn = NativeAcpConnection::new(
            "test-agent".to_string(),
            "/bin/false".to_string(),
            vec![],
            None,
        );
        conn.cmd_tx = Some(tx);
        (conn, rx)
    }

    fn connection_with_session_capabilities(
        capabilities: SessionCapabilities,
    ) -> (NativeAcpConnection, mpsc::UnboundedReceiver<AcpCommand>) {
        let (conn, rx) = connection_with_command_channel();
        conn.session_capabilities
            .lock()
            .expect("test mutex must not be poisoned")
            .replace(capabilities);
        (conn, rx)
    }

    #[tokio::test]
    async fn new_session_dispatches_configured_additional_directories() {
        let (mut conn, mut rx) = connection_with_command_channel();
        let cwd = PathBuf::from("/tmp/spur-main");
        let additional = vec![
            PathBuf::from("/tmp/spur-extra"),
            PathBuf::from("/Volumes/Projects/other-root"),
        ];
        conn.set_additional_directories(additional.clone());

        let expected_cwd = cwd.clone();
        let expected_additional = additional.clone();
        let handle = tokio::spawn(async move { conn.new_session(cwd, vec![]).await });

        match rx.recv().await.expect("new session command must be sent") {
            AcpCommand::NewSession { request, reply } => {
                assert_eq!(request.cwd, expected_cwd);
                assert_eq!(request.additional_directories, expected_additional);
                assert!(request
                    .additional_directories
                    .iter()
                    .all(|path| path.is_absolute()));
                reply
                    .send(Ok(NewSessionResponse::new(SessionId::new("sid"))))
                    .expect("test receiver must still be waiting");
            }
            _ => panic!("expected NewSession command"),
        }

        let response = handle
            .await
            .expect("new_session task must not panic")
            .unwrap();
        assert_eq!(response.session_id, SessionId::new("sid"));
    }

    #[tokio::test]
    async fn resume_session_dispatches_command_and_returns_response() {
        let (mut conn, mut rx) = connection_with_session_capabilities(
            SessionCapabilities::new().resume(SessionResumeCapabilities::new()),
        );
        let cwd = PathBuf::from("/tmp/spur-resume");
        let request = ResumeSessionRequest::new(SessionId::new("sid"), cwd.clone());

        let handle = tokio::spawn(async move { conn.resume_session(request).await });

        match rx.recv().await.expect("resume command must be sent") {
            AcpCommand::ResumeSession { request, reply } => {
                assert_eq!(request.session_id, SessionId::new("sid"));
                assert_eq!(request.cwd, cwd);
                reply
                    .send(Ok(ResumeSessionResponse::new()))
                    .expect("test receiver must still be waiting");
            }
            _ => panic!("expected ResumeSession command"),
        }

        let response = handle.await.expect("resume task must not panic").unwrap();
        assert_eq!(response, ResumeSessionResponse::new());
    }

    #[tokio::test]
    async fn resume_session_without_capability_returns_missing_without_dispatch() {
        let (mut conn, mut rx) = connection_with_session_capabilities(SessionCapabilities::new());
        let request =
            ResumeSessionRequest::new(SessionId::new("sid"), PathBuf::from("/tmp/spur-resume"));

        let result = conn.resume_session(request).await;

        match result {
            Err(AcpError::CapabilityMissing(name)) => assert_eq!(name, "session/resume"),
            other => panic!("expected CapabilityMissing(\"session/resume\"), got {other:?}"),
        }
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn list_sessions_dispatches_command_and_returns_response() {
        let (mut conn, mut rx) = connection_with_session_capabilities(
            SessionCapabilities::new().list(SessionListCapabilities::new()),
        );
        let request = ListSessionsRequest::new().cursor("next-page".to_string());

        let handle = tokio::spawn(async move { conn.list_sessions(request).await });

        match rx.recv().await.expect("list command must be sent") {
            AcpCommand::ListSessions { request, reply } => {
                assert_eq!(request.cursor.as_deref(), Some("next-page"));
                reply
                    .send(Ok(ListSessionsResponse::new(vec![])))
                    .expect("test receiver must still be waiting");
            }
            _ => panic!("expected ListSessions command"),
        }

        let response = handle.await.expect("list task must not panic").unwrap();
        assert_eq!(response, ListSessionsResponse::new(vec![]));
    }

    #[tokio::test]
    async fn list_sessions_without_capability_returns_missing_without_dispatch() {
        let (mut conn, mut rx) = connection_with_session_capabilities(SessionCapabilities::new());

        let err = conn
            .list_sessions(ListSessionsRequest::new())
            .await
            .expect_err("missing session/list capability must fail");

        match err.downcast_ref::<AcpError>() {
            Some(AcpError::CapabilityMissing(name)) => assert_eq!(*name, "session/list"),
            other => panic!("expected CapabilityMissing(\"session/list\"), got {other:?}"),
        }
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn delete_session_dispatches_command_and_returns_response() {
        let (mut conn, mut rx) = connection_with_session_capabilities(
            SessionCapabilities::new().delete(SessionDeleteCapabilities::new()),
        );
        let request = DeleteSessionRequest::new(SessionId::new("sid"));

        let handle = tokio::spawn(async move { conn.delete_session(request).await });

        match rx.recv().await.expect("delete command must be sent") {
            AcpCommand::DeleteSession { request, reply } => {
                assert_eq!(request.session_id, SessionId::new("sid"));
                reply
                    .send(Ok(DeleteSessionResponse::new()))
                    .expect("test receiver must still be waiting");
            }
            _ => panic!("expected DeleteSession command"),
        }

        let response = handle.await.expect("delete task must not panic").unwrap();
        assert_eq!(response, DeleteSessionResponse::new());
    }

    #[tokio::test]
    async fn delete_session_without_capability_returns_missing_without_dispatch() {
        let (mut conn, mut rx) = connection_with_session_capabilities(SessionCapabilities::new());
        let request = DeleteSessionRequest::new(SessionId::new("sid"));

        let result = conn.delete_session(request).await;

        match result {
            Err(AcpError::CapabilityMissing(name)) => assert_eq!(name, "session/delete"),
            other => panic!("expected CapabilityMissing(\"session/delete\"), got {other:?}"),
        }
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn close_session_dispatches_command_and_returns_response() {
        let (mut conn, mut rx) = connection_with_session_capabilities(
            SessionCapabilities::new().close(SessionCloseCapabilities::new()),
        );
        let request = CloseSessionRequest::new(SessionId::new("sid"));

        let handle = tokio::spawn(async move { conn.close_session(request).await });

        match rx.recv().await.expect("close command must be sent") {
            AcpCommand::CloseSession { request, reply } => {
                assert_eq!(request.session_id, SessionId::new("sid"));
                reply
                    .send(Ok(CloseSessionResponse::new()))
                    .expect("test receiver must still be waiting");
            }
            _ => panic!("expected CloseSession command"),
        }

        let response = handle.await.expect("close task must not panic").unwrap();
        assert_eq!(response, CloseSessionResponse::new());
    }

    #[tokio::test]
    async fn close_session_without_capability_returns_missing_without_dispatch() {
        let (mut conn, mut rx) = connection_with_session_capabilities(SessionCapabilities::new());
        let request = CloseSessionRequest::new(SessionId::new("sid"));

        let result = conn.close_session(request).await;

        match result {
            Err(AcpError::CapabilityMissing(name)) => assert_eq!(name, "session/close"),
            other => panic!("expected CapabilityMissing(\"session/close\"), got {other:?}"),
        }
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
}

#[cfg(all(test, unix))]
mod shutdown_timeout_tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::connection::AgentConnection;
    use tokio::sync::mpsc;

    async fn pid_alive(pid: u32) -> bool {
        tokio::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn shutdown_times_out_and_kills_subprocess_without_leak() {
        let mut conn = NativeAcpConnection::new("test-agent", "/bin/false", vec![], None);
        let mut sleeper = tokio::process::Command::new("/bin/sh");
        sleeper
            .arg("-c")
            .arg("sleep 60")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        let mut child = sleeper.spawn().expect("spawn sleeper");
        let pid = child.id().expect("pid should exist");
        assert!(
            pid_alive(pid).await,
            "sleeper should be alive before shutdown"
        );

        *conn.child_pgid.lock().unwrap() = Some(pid as i32);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        conn.cmd_tx = Some(cmd_tx);
        drop(cmd_rx);
        conn.thread_handle = Some(std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(30));
        }));

        let started = Instant::now();
        conn.shutdown().await.expect("shutdown should succeed");
        assert!(
            started.elapsed() <= Duration::from_millis(1200),
            "shutdown should return within the bounded timeout",
        );
        assert!(conn.cmd_tx.is_none(), "shutdown should consume cmd_tx");
        assert!(
            conn.thread_handle.is_none(),
            "shutdown should clear the thread handle"
        );
        tokio::time::timeout(Duration::from_secs(1), child.wait())
            .await
            .expect("sleeper should exit after shutdown fallback")
            .expect("waiting on sleeper should succeed");
    }
}

// Note: behavioral verification of the SIGTERM-before-wait shutdown ladder
// belongs in integration tests with a real subprocess (e.g. `sleep 30` +
// signal-status capture: SIGTERM yields exit code 143, SIGKILL yields 137).
// A unit-level "regression test" that greps this file's source for symbol
// ordering passes for the wrong reasons (textual reorder, not runtime
// ordering) and silently breaks on refactor — explicitly omitted.

#[cfg(test)]
mod usage_fill_tests {
    use super::*;
    use agent_client_protocol::schema::v1::UsageUpdate;

    #[test]
    fn usage_from_meta_reads_billable_token_fields() {
        let mut update = UsageUpdate::new(100, 200_000);
        update.meta = Some(
            serde_json::json!({
                "input_tokens": 1200,
                "output_tokens": 340,
                "cached_read_tokens": 50,
                "cached_write_tokens": 10,
                "total_tokens": 1600
            })
            .as_object()
            .expect("object")
            .clone(),
        );
        let usage = usage_from_usage_update_meta(&update).expect("usage from meta");
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 340);
        assert_eq!(usage.cached_read_tokens, Some(50));
        assert_eq!(usage.cached_write_tokens, Some(10));
        assert_eq!(usage.total_tokens, 1600);
    }

    #[test]
    fn usage_from_meta_ignores_context_window_only_update() {
        // used/size alone must not invent input_tokens (sol_5d3dc964920d420f).
        let update = UsageUpdate::new(42_000, 200_000);
        assert!(usage_from_usage_update_meta(&update).is_none());
    }

    #[test]
    fn maybe_fill_does_not_clobber_existing_prompt_usage() {
        let slot = Arc::new(Mutex::new(Some(Usage::new(100, 80, 20))));
        let mut update = UsageUpdate::new(1, 2);
        update.meta = Some(
            serde_json::json!({"input_tokens": 9999, "output_tokens": 1})
                .as_object()
                .expect("object")
                .clone(),
        );
        maybe_fill_usage_from_session_update(&slot, &SessionUpdate::UsageUpdate(update));
        let guarded = slot.lock().unwrap();
        let u = guarded.as_ref().unwrap();
        assert_eq!(u.input_tokens, 80); // not clobbered
    }

    #[test]
    fn maybe_fill_writes_when_slot_empty_and_meta_present() {
        let slot = Arc::new(Mutex::new(None));
        let mut update = UsageUpdate::new(1, 2);
        update.meta = Some(
            serde_json::json!({"inputTokens": 500, "outputTokens": 25})
                .as_object()
                .expect("object")
                .clone(),
        );
        maybe_fill_usage_from_session_update(&slot, &SessionUpdate::UsageUpdate(update));
        let guarded = slot.lock().unwrap();
        let u = guarded.as_ref().unwrap();
        assert_eq!(u.input_tokens, 500);
        assert_eq!(u.output_tokens, 25);
    }
}

#[cfg(test)]
mod stderr_capture_tests {
    use super::*;

    #[test]
    fn log_path_uses_spur_logs_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let path = build_acp_log_path(tmp.path(), "claude-code-acp");
        let s = path.to_string_lossy();
        assert!(
            s.contains(".spur/logs/"),
            "expected log under .spur/logs/, got {}",
            path.display()
        );
        assert!(
            s.ends_with("-acp.log"),
            "expected -acp.log suffix, got {}",
            path.display()
        );
        assert!(
            s.contains("claude-code-acp"),
            "expected agent name in path, got {}",
            path.display()
        );
        // PID must be embedded so concurrent spawns don't collide.
        let pid = std::process::id().to_string();
        assert!(
            s.contains(&pid),
            "expected process id {pid} in path, got {}",
            path.display()
        );
    }
}
