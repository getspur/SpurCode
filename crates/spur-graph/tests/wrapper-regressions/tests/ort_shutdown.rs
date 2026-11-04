#![cfg(all(target_os = "macos", feature = "embed"))]

use std::process::Command;

// Conversion is deliberately observable: managed async must reject before it
// touches inputs or creates any native callback state.
struct ConversionMustNotRun;

#[test]
fn allocator_finalization_failure_allows_checked_shutdown_child() {
    let Some(cache) = std::env::var_os("SPUR_SHUTDOWN_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let info = ort::memory::MemoryInfo::new(
        ort::memory::AllocationDevice::CUDA,
        0,
        ort::memory::AllocatorType::Device,
        ort::memory::MemoryType::Default,
    )
    .unwrap();
    let result = ort::session::Session::builder()
        .unwrap()
        .with_allocator(info)
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"));
    let error = result.err().expect("expected allocator setup to fail");
    println!("finalization error: {error}");
    assert!(error
        .to_string()
        .contains("No requested allocator available"));
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn allocator_finalization_failure_allows_checked_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "allocator_finalization_failure_allows_checked_shutdown_child",
            "--nocapture",
        ])
        .env("SPUR_SHUTDOWN_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    print!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_internal_allocator_outlives_session_child() {
    let Some(cache) = std::env::var_os("SPUR_SHUTDOWN_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let session = ort::session::Session::builder()
        .unwrap()
        .with_allocator(ort::memory::MemoryInfo::default())
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    let value = ort::value::Tensor::<f32>::new(session.allocator(), [2usize]).unwrap();
    drop(session);
    assert!(ort::environment::try_shutdown().is_err());
    drop(value);
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn session_internal_allocator_outlives_session() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session_internal_allocator_outlives_session_child",
            "--nocapture",
        ])
        .env("SPUR_SHUTDOWN_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_allocator_outlives_its_handle_child() {
    let Some(cache) = std::env::var_os("SPUR_SHUTDOWN_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let session = ort::session::Session::builder()
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    let allocator =
        ort::memory::Allocator::new(&session, ort::memory::MemoryInfo::default()).unwrap();
    let value = ort::value::Tensor::<f32>::new(&allocator, [2usize]).unwrap();
    drop(allocator);
    drop(session);
    assert!(ort::environment::try_shutdown().is_err());
    drop(value);
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn session_allocator_outlives_its_handle() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session_allocator_outlives_its_handle_child",
            "--nocapture",
        ])
        .env("SPUR_SHUTDOWN_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

impl<'i, 'v> From<ConversionMustNotRun> for ort::session::SessionInputs<'i, 'v, 1> {
    fn from(_: ConversionMustNotRun) -> Self {
        panic!("managed async converted inputs");
    }
}

#[test]
fn managed_async_rejected_before_input_conversion_child() {
    let Some(cache) = std::env::var_os("SPUR_SHUTDOWN_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let mut session = ort::session::Session::builder()
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    let options = ort::session::run_options::RunOptions::new().unwrap();
    {
        let result = session.run_async(ConversionMustNotRun, &options);
        match result {
            Err(error) => assert!(error.to_string().contains("managed-environment")),
            Ok(_) => panic!("managed async unexpectedly dispatched"),
        }
    }
    drop(options);
    drop(session);
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn managed_async_rejected_before_input_conversion() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "managed_async_rejected_before_input_conversion_child",
            "--nocapture",
        ])
        .env("SPUR_SHUTDOWN_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn busy_shutdown_child() {
    let Some(cache) = std::env::var_os("SPUR_SHUTDOWN_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let mut session = ort::session::Session::builder()
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();

    assert!(ort::environment::try_shutdown().is_err());
    let input =
        ort::value::Tensor::from_array(([3usize, 2], vec![1.0_f32, 2., 3., 4., 5., 6.])).unwrap();
    let outputs = session.run(ort::inputs![input]).unwrap();
    let value = outputs.into_iter().map(|(_, value)| value).next().unwrap();
    drop(session);

    // The output's backing session must also prevent environment release.
    assert!(ort::environment::try_shutdown().is_err());
    assert_eq!(
        value.try_extract_tensor::<f32>().unwrap().1,
        &[1., 4., 9., 16., 25., 36.]
    );
    drop(value);

    // A safe environment owner must also keep the native environment alive,
    // even after every session has gone away.
    let environment = ort::environment::get_environment().unwrap();
    assert!(ort::environment::try_shutdown().is_err());
    environment.set_log_level(ort::logging::LogLevel::Warning);
    drop(environment);
    ort::environment::try_shutdown().unwrap();
    ort::environment::try_shutdown().unwrap();
    assert!(ort::environment::get_environment().is_err());
}

#[test]
fn standalone_and_concurrent_owners_child() {
    let Some(cache) = std::env::var_os("SPUR_SHUTDOWN_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let guard = ort::environment::get_environment().unwrap();
    let other = guard.clone();
    std::thread::spawn(move || {
        assert!(ort::environment::try_shutdown().is_err());
        drop(other);
    })
    .join()
    .unwrap();
    drop(guard);
    let value = ort::value::Tensor::from_array(([1usize], vec![42.0_f32])).unwrap();
    assert!(ort::environment::try_shutdown().is_err());
    drop(value);
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn standalone_and_concurrent_owners() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "standalone_and_concurrent_owners_child",
            "--nocapture",
        ])
        .env("SPUR_SHUTDOWN_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explicit_shutdown_with_pgq_child() {
    let Some(cache) = std::env::var_os("SPUR_SHUTDOWN_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    assert!(ort::session::Session::builder()
        .unwrap()
        .commit_from_memory(b"invalid model")
        .is_err());
    let mut session = ort::session::Session::builder()
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    let db = duckdb::Connection::open_in_memory().unwrap();
    db.execute_batch("LOAD duckpgq;").unwrap();
    db.execute_batch("CREATE TABLE nodes (id INTEGER PRIMARY KEY, name VARCHAR); INSERT INTO nodes VALUES (1, 'a'), (2, 'b'); CREATE TABLE edges (src INTEGER, dst INTEGER); INSERT INTO edges VALUES (1, 2); CREATE PROPERTY GRAPH g VERTEX TABLES (nodes) EDGE TABLES (edges SOURCE KEY (src) REFERENCES nodes (id) DESTINATION KEY (dst) REFERENCES nodes (id));").unwrap();
    let input =
        ort::value::Tensor::from_array(([3usize, 2], vec![1.0_f32, 2., 3., 4., 5., 6.])).unwrap();
    let outputs = session.run(ort::inputs![input]).unwrap();
    assert_eq!(
        outputs[0].try_extract_tensor::<f32>().unwrap().1,
        &[1., 4., 9., 16., 25., 36.]
    );
    let name: String = db.query_row("FROM GRAPH_TABLE (g MATCH (a:nodes)-[e:edges]->(b:nodes) COLUMNS (b.name AS name)) SELECT name", [], |row| row.get(0)).unwrap();
    assert_eq!(name, "b");
    println!("ORT output: 1,4,9,16,25,36; PGQ result: {name}");
    drop(outputs);
    drop(session);
    drop(db);
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn explicit_shutdown_after_inference_and_pgq() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "explicit_shutdown_with_pgq_child", "--nocapture"])
        .env("SPUR_SHUTDOWN_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    print!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn shutdown_refuses_live_session_and_environment_owners() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "busy_shutdown_child", "--nocapture"])
        .env("SPUR_SHUTDOWN_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
