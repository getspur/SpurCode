use std::process::Command;

#[test]
fn compatibility_metadata_is_available_without_native_initialization() {
    let output = Command::new(env!("CARGO_BIN_EXE_spur-embedding-worker"))
        .arg("--compatibility")
        .env("SPUR_ORT_CACHE", "/dev/null/unusable")
        .env("SPUR_MODEL_CACHE", "/dev/null/unusable")
        .output()
        .expect("launch helper");
    assert!(
        output.status.success(),
        "metadata command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["protocol"], spur_embedding::PROTOCOL_VERSION);
    assert_eq!(value["os"], std::env::consts::OS);
    assert_eq!(value["arch"], std::env::consts::ARCH);
}
