#[cfg(all(target_os = "macos", feature = "embed"))]
pub mod store {
    mod ort_runtime;
    pub use ort_runtime::ensure_ort_runtime_at as ensure_bundled_ort_runtime_at;
}
