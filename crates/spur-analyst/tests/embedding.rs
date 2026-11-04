#[path = "embedding/dependency_boundary.rs"]
mod dependency_boundary;
#[path = "embedding/embed_service.rs"]
mod embed_service;
#[path = "embedding/embedding_sidecar_layout.rs"]
mod embedding_sidecar_layout;
#[cfg(all(unix, feature = "embed"))]
#[path = "embedding/warm_helper.rs"]
mod warm_helper;

#[cfg(all(unix, feature = "embed"))]
#[path = "embedding/warm_inference.rs"]
mod warm_inference;

#[cfg(all(unix, feature = "embed"))]
#[path = "embedding/owned_runtime.rs"]
mod owned_runtime;
