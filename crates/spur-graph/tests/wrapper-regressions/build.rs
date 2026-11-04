#[path = "build_support/ort_runtime.rs"]
mod ort_runtime;

fn main() {
    println!("cargo:rerun-if-env-changed=SPUR_ORT_ARCHIVE");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("CARGO_FEATURE_EMBED").is_some()
    {
        ort_runtime::bundle().expect("bundle pinned ONNX Runtime for macOS");
    }
}
