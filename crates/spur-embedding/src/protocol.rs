use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
};
use thiserror::Error;
use uuid::Uuid;

/// Internal helper wire version; distinct from the external analyst socket.
pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelId {
    NomicEmbedTextV15,
    CodeRankEmbed,
    JinaEmbeddingsV2BaseCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelIdentity {
    pub model: ModelId,
    pub dimensions: usize,
    pub model_fingerprint: String,
    pub tokenizer_fingerprint: String,
    pub pooling: String,
    pub normalization: String,
    pub prefix: String,
}

/// Explicit caller policy, without product defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolLimits {
    pub max_frame_bytes: u32,
    pub max_texts: usize,
    pub max_text_bytes: usize,
    pub max_total_text_bytes: usize,
    pub max_dimensions: usize,
    pub max_identity_bytes: usize,
    pub max_pending: usize,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame length {length} exceeds limit {limit}")]
    FrameTooLarge { length: u64, limit: u32 },
    #[error("invalid protocol: {0}")]
    Invalid(&'static str),
}

impl ProtocolLimits {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.max_frame_bytes == 0
            || self.max_texts == 0
            || self.max_text_bytes == 0
            || self.max_total_text_bytes == 0
            || self.max_dimensions == 0
            || self.max_identity_bytes == 0
            || self.max_pending == 0
        {
            return Err(ProtocolError::Invalid("zero limit"));
        }
        self.max_texts
            .checked_mul(self.max_dimensions)
            .and_then(|v| v.checked_mul(std::mem::size_of::<f32>()))
            .ok_or(ProtocolError::Invalid("row size overflow"))?;
        Ok(())
    }
}
impl ModelIdentity {
    pub fn validate(&self, limits: &ProtocolLimits) -> Result<(), ProtocolError> {
        limits.validate()?;
        if self.dimensions == 0 || self.dimensions > limits.max_dimensions {
            return Err(ProtocolError::Invalid("dimensions"));
        }
        for field in [
            &self.model_fingerprint,
            &self.tokenizer_fingerprint,
            &self.pooling,
            &self.normalization,
            &self.prefix,
        ] {
            if field.is_empty() || field.len() > limits.max_identity_bytes {
                return Err(ProtocolError::Invalid("identity field"));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum Request {
    Hello {
        identity: ModelIdentity,
    },
    Embed {
        identity: ModelIdentity,
        texts: Vec<String>,
    },
    Shutdown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    InvalidRequest,
    Internal,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum Reply {
    Ready {
        identity: ModelIdentity,
    },
    Embeddings {
        identity: ModelIdentity,
        rows: Vec<Vec<f32>>,
    },
    Closed,
    Error {
        code: ErrorCode,
        message: String,
    },
}
// Struct variants make Serde enforce the empty field set for Shutdown/Closed.
// The public unit variants and their wire representation remain unchanged.
#[derive(Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
#[expect(
    clippy::empty_enum_variants_with_brackets,
    reason = "empty struct variants reject unknown fields"
)]
enum WireRequest {
    Hello {
        identity: ModelIdentity,
    },
    Embed {
        identity: ModelIdentity,
        texts: Vec<String>,
    },
    Shutdown {},
}
impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match WireRequest::deserialize(deserializer)? {
            WireRequest::Hello { identity } => Self::Hello { identity },
            WireRequest::Embed { identity, texts } => Self::Embed { identity, texts },
            WireRequest::Shutdown {} => Self::Shutdown,
        })
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
#[expect(
    clippy::empty_enum_variants_with_brackets,
    reason = "empty struct variants reject unknown fields"
)]
enum WireReply {
    Ready {
        identity: ModelIdentity,
    },
    Embeddings {
        identity: ModelIdentity,
        rows: Vec<Vec<f32>>,
    },
    Closed {},
    Error {
        code: ErrorCode,
        message: String,
    },
}
impl<'de> Deserialize<'de> for Reply {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match WireReply::deserialize(deserializer)? {
            WireReply::Ready { identity } => Self::Ready { identity },
            WireReply::Embeddings { identity, rows } => Self::Embeddings { identity, rows },
            WireReply::Closed {} => Self::Closed,
            WireReply::Error { code, message } => Self::Error { code, message },
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEnvelope {
    pub version: u32,
    pub generation: Uuid,
    pub request_id: u64,
    pub request: Request,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyEnvelope {
    pub version: u32,
    pub generation: Uuid,
    pub request_id: u64,
    pub reply: Reply,
}

fn envelope(version: u32, generation: Uuid, id: u64) -> Result<(), ProtocolError> {
    if version != PROTOCOL_VERSION || generation.is_nil() || id == 0 {
        return Err(ProtocolError::Invalid("version, generation, or request id"));
    }
    Ok(())
}
pub trait WireEnvelope: Serialize + DeserializeOwned {
    fn validate(&self, limits: &ProtocolLimits) -> Result<(), ProtocolError>;
}
impl RequestEnvelope {
    pub fn validate(&self, limits: &ProtocolLimits) -> Result<(), ProtocolError> {
        limits.validate()?;
        envelope(self.version, self.generation, self.request_id)?;
        match &self.request {
            Request::Hello { identity } => identity.validate(limits),
            Request::Embed { identity, texts } => {
                identity.validate(limits)?;
                if texts.is_empty() || texts.len() > limits.max_texts {
                    return Err(ProtocolError::Invalid("text count"));
                }
                let mut total = 0usize;
                for text in texts {
                    if text.is_empty() || text.len() > limits.max_text_bytes {
                        return Err(ProtocolError::Invalid("text size"));
                    }
                    total = total
                        .checked_add(text.len())
                        .ok_or(ProtocolError::Invalid("text size overflow"))?;
                }
                if total > limits.max_total_text_bytes {
                    return Err(ProtocolError::Invalid("total text size"));
                }
                Ok(())
            }
            Request::Shutdown => Ok(()),
        }
    }
}
impl WireEnvelope for RequestEnvelope {
    fn validate(&self, l: &ProtocolLimits) -> Result<(), ProtocolError> {
        self.validate(l)
    }
}
impl ReplyEnvelope {
    pub fn validate(&self, limits: &ProtocolLimits) -> Result<(), ProtocolError> {
        limits.validate()?;
        envelope(self.version, self.generation, self.request_id)?;
        match &self.reply {
            Reply::Ready { identity } => identity.validate(limits),
            Reply::Embeddings { identity, rows } => {
                identity.validate(limits)?;
                if rows.is_empty() || rows.len() > limits.max_texts {
                    return Err(ProtocolError::Invalid("row count"));
                }
                for row in rows {
                    if row.len() != identity.dimensions || row.iter().any(|x| !x.is_finite()) {
                        return Err(ProtocolError::Invalid("row dimensions or nonfinite value"));
                    }
                }
                Ok(())
            }
            Reply::Error { message, .. }
                if message.is_empty() || message.len() > limits.max_identity_bytes =>
            {
                Err(ProtocolError::Invalid("error message"))
            }
            Reply::Error { .. } | Reply::Closed => Ok(()),
        }
    }
}
impl WireEnvelope for ReplyEnvelope {
    fn validate(&self, l: &ProtocolLimits) -> Result<(), ProtocolError> {
        self.validate(l)
    }
}

/// Read exactly one length-prefixed JSON envelope. Reject length before body allocation.
pub fn read_frame<R: Read, T: WireEnvelope>(
    reader: &mut R,
    limits: &ProtocolLimits,
) -> Result<T, ProtocolError> {
    limits.validate()?;
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header);
    if length > limits.max_frame_bytes {
        return Err(ProtocolError::FrameTooLarge {
            length: length.into(),
            limit: limits.max_frame_bytes,
        });
    }
    let mut body = vec![0; length as usize];
    reader.read_exact(&mut body)?;
    let value: T = serde_json::from_slice(&body)?;
    value.validate(limits)?;
    Ok(value)
}
struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for LimitedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// Validate and write one bounded frame; no partial frame is emitted on serialization failure.
pub fn write_frame<W: Write, T: WireEnvelope>(
    writer: &mut W,
    value: &T,
    limits: &ProtocolLimits,
) -> Result<(), ProtocolError> {
    value.validate(limits)?;
    let mut buffer = LimitedBuffer {
        bytes: Vec::new(),
        limit: limits.max_frame_bytes as usize,
    };
    serde_json::to_writer(&mut buffer, value)?;
    let len = u32::try_from(buffer.bytes.len())
        .map_err(|_error| ProtocolError::Invalid("frame length"))?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&buffer.bytes)?;
    Ok(())
}

/// Tracks monotonic request IDs and the exact requests eligible for reply validation.
#[derive(Debug)]
enum Phase {
    Initial,
    HelloPending,
    Ready,
    ShutdownPending,
    Closed,
}

#[derive(Debug)]
pub struct Session {
    generation: Uuid,
    phase: Phase,
    identity: ModelIdentity,
    limits: ProtocolLimits,
    last_id: u64,
    requests: HashMap<u64, RequestEnvelope>,
}
impl Session {
    pub fn new(
        generation: Uuid,
        identity: ModelIdentity,
        limits: ProtocolLimits,
    ) -> Result<Self, ProtocolError> {
        if generation.is_nil() {
            return Err(ProtocolError::Invalid("generation"));
        }
        identity.validate(&limits)?;
        Ok(Self {
            generation,
            phase: Phase::Initial,
            identity,
            limits,
            last_id: 0,
            requests: HashMap::new(),
        })
    }
    pub fn validate_request(&mut self, request: &RequestEnvelope) -> Result<(), ProtocolError> {
        request.validate(&self.limits)?;
        if request.generation != self.generation || request.request_id <= self.last_id {
            return Err(ProtocolError::Invalid("stale generation or request"));
        }
        match &request.request {
            Request::Hello { identity } | Request::Embed { identity, .. }
                if identity != &self.identity =>
            {
                return Err(ProtocolError::Invalid("model identity"))
            }
            _ => {}
        }
        if self.requests.len() >= self.limits.max_pending {
            return Err(ProtocolError::Invalid("pending request limit"));
        }
        self.phase = match (&self.phase, &request.request) {
            (Phase::Initial, Request::Hello { .. }) => Phase::HelloPending,
            (Phase::Ready, Request::Embed { .. }) => Phase::Ready,
            (Phase::Ready, Request::Shutdown) if self.requests.is_empty() => Phase::ShutdownPending,
            _ => return Err(ProtocolError::Invalid("session phase")),
        };
        self.last_id = request.request_id;
        self.requests.insert(request.request_id, request.clone());
        Ok(())
    }
    pub fn validate_reply(
        &self,
        request: &RequestEnvelope,
        reply: &ReplyEnvelope,
    ) -> Result<(), ProtocolError> {
        reply.validate(&self.limits)?;
        if self.requests.get(&request.request_id) != Some(request)
            || reply.generation != self.generation
            || reply.request_id != request.request_id
        {
            return Err(ProtocolError::Invalid("reply correlation"));
        }
        match (&request.request, &reply.reply) {
            (Request::Hello { .. }, Reply::Ready { identity }) if identity == &self.identity => {
                Ok(())
            }
            (Request::Embed { texts, .. }, Reply::Embeddings { identity, rows })
                if identity == &self.identity && rows.len() == texts.len() =>
            {
                Ok(())
            }
            (Request::Shutdown, Reply::Closed) | (_, Reply::Error { .. }) => Ok(()),
            _ => Err(ProtocolError::Invalid("reply type, identity, or row count")),
        }
    }
    /// Validate and consume a pending reply. Invalid replies leave the request pending.
    pub fn complete_reply(
        &mut self,
        request: &RequestEnvelope,
        reply: &ReplyEnvelope,
    ) -> Result<(), ProtocolError> {
        self.validate_reply(request, reply)?;
        self.requests.remove(&request.request_id);
        self.phase = match (&request.request, &reply.reply) {
            (Request::Hello { .. }, Reply::Ready { .. })
            | (Request::Shutdown, Reply::Error { .. }) => Phase::Ready,
            (Request::Hello { .. }, Reply::Error { .. }) => Phase::Initial,
            (Request::Shutdown, Reply::Closed) => Phase::Closed,
            _ => std::mem::replace(&mut self.phase, Phase::Closed),
        };
        Ok(())
    }
}
