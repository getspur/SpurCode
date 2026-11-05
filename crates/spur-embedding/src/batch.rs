//! Admission of a complete logical input before any helper work is launched.
use crate::{
    ModelIdentity, ProtocolError, ProtocolLimits, Request, RequestEnvelope, PROTOCOL_VERSION,
};
use serde::Serialize;
use std::{io, ops::Range};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum BatchError {
    #[error("batch size must be positive")]
    InvalidBatchSize,
    #[error("input {index} is empty or exceeds the UTF-8 text limit")]
    InvalidText { index: usize },
    #[error("input {index} cannot fit an individual request within text/frame limits")]
    IndividualTooLarge { index: usize },
    #[error("batch size arithmetic overflow")]
    Overflow,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("batch JSON sizing: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Default)]
struct Size(usize);
impl io::Write for Size {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "JSON size overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn json_size(value: &impl Serialize) -> Result<usize, BatchError> {
    let mut size = Size::default();
    serde_json::to_writer(&mut size, value)?;
    Ok(size.0)
}

/// Ordered ranges covering every input exactly once, or an error for the whole input.
/// The body cap excludes the four-byte wire prefix. UUIDs have a fixed serialized
/// length; reserving the maximum u64 request ID makes every correlated request fit.
/// Count serialized strings with serde itself, including UTF-8 and JSON escaping.
pub fn plan_batches(
    texts: &[String],
    identity: &ModelIdentity,
    limits: &ProtocolLimits,
    batch_size: Option<usize>,
) -> Result<Vec<Range<usize>>, BatchError> {
    identity.validate(limits)?;
    if batch_size == Some(0) {
        return Err(BatchError::InvalidBatchSize);
    }
    let max_rows = batch_size.unwrap_or(limits.max_texts).min(limits.max_texts);
    let frame_limit = usize::try_from(limits.max_frame_bytes).map_err(|_| BatchError::Overflow)?;
    let overhead = json_size(&RequestEnvelope {
        version: PROTOCOL_VERSION,
        generation: Uuid::from_u128(u128::MAX),
        request_id: u64::MAX,
        request: Request::Embed {
            identity: identity.clone(),
            texts: Vec::new(),
        },
    })?;
    let mut ranges = Vec::new();
    let (mut start, mut raw, mut body) = (0, 0usize, overhead);
    for (index, text) in texts.iter().enumerate() {
        if text.is_empty() || text.len() > limits.max_text_bytes {
            return Err(BatchError::InvalidText { index });
        }
        let encoded = json_size(text)?;
        let single = overhead.checked_add(encoded).ok_or(BatchError::Overflow)?;
        if text.len() > limits.max_total_text_bytes || single > frame_limit {
            return Err(BatchError::IndividualTooLarge { index });
        }
        let next_raw = raw.checked_add(text.len());
        let next_body = body
            .checked_add(usize::from(index != start))
            .and_then(|n| n.checked_add(encoded));
        if index - start >= max_rows
            || !next_raw.is_some_and(|n| n <= limits.max_total_text_bytes)
            || !next_body.is_some_and(|n| n <= frame_limit)
        {
            ranges.push(start..index);
            start = index;
            raw = text.len();
            body = single;
        } else {
            raw = next_raw.ok_or(BatchError::Overflow)?;
            body = next_body.ok_or(BatchError::Overflow)?;
        }
    }
    if start < texts.len() {
        ranges.push(start..texts.len());
    }
    Ok(ranges)
}
