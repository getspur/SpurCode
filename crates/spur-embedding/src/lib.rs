//! Native-free types and framing for the supervised embedding helper.
mod protocol;
pub use protocol::*;
mod client;
pub use client::{Cancellation, CleanupOwner, ClientError, ClientPolicy, EmbeddingClient};
mod policy;
pub use policy::{HelperConfig, LocalPolicy, PolicyError};
mod discovery;
pub use discovery::{DiscoveryError, HelperCommand, LocalHelper};
