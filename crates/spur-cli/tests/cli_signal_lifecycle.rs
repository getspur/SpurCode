//! Exercises the built CLI and a protocol-only helper as separate processes.
#[cfg(all(unix, feature = "embed"))]
#[test]
fn command_signals_join_embedding_children() {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cli_signal_lifecycle.py");
    let status = std::process::Command::new("python3")
        .arg(script)
        .env("SPUR_SIGNAL_TEST_BINARY", env!("CARGO_BIN_EXE_spur"))
        .status()
        .expect("run CLI signal lifecycle fixture");
    assert!(
        status.success(),
        "CLI signal lifecycle fixture failed: {status}"
    );
}
