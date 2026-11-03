//! Native-free types and framing for the supervised embedding helper.
mod protocol;
pub use protocol::*;
mod client;
pub use client::{Cancellation, ClientError, ClientPolicy, EmbeddingClient};
