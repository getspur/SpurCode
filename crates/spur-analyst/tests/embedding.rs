#[path = "embedding/embed_service.rs"]
mod embed_service;
#[path = "embedding/embedding_sidecar_layout.rs"]
mod embedding_sidecar_layout;
#[path = "embedding/dependency_boundary.rs"]
mod dependency_boundary;
#[cfg(all(unix, feature = "embed"))]
#[path = "embedding/warm_helper.rs"]
mod warm_helper;
