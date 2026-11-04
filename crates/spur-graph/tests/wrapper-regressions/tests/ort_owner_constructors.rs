#![cfg(all(target_os = "macos", feature = "embed"))]

use std::process::Command;

#[test]
fn compiler_options_keep_environment_alive_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let compiler = ort::compiler::ModelCompiler::new(ort::session::Session::builder().unwrap())
        .unwrap()
        .with_model_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    if ort::environment::try_shutdown().is_ok() {
        // The unfixed implementation has already closed the environment. Avoid
        // dropping native options whose environment is now gone on the RED path.
        std::mem::forget(compiler);
        panic!("live compiler options must make shutdown busy");
    }
    let compiler = compiler.with_embed_ep_context(false).unwrap();
    drop(compiler);
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn compiler_options_keep_environment_alive() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "compiler_options_keep_environment_alive_child",
            "--nocapture",
        ])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
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
fn allocator_info_keeps_backing_alive_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let internal = std::env::var_os("SPUR_OWNER_INTERNAL").is_some();
    let builder = ort::session::Session::builder().unwrap();
    let builder = if internal {
        builder
            .with_allocator(ort::memory::MemoryInfo::default())
            .unwrap()
    } else {
        builder
    };
    let session = builder
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    let info = if internal {
        session.allocator().memory_info()
    } else {
        let allocator =
            ort::memory::Allocator::new(&session, ort::memory::MemoryInfo::default()).unwrap();
        allocator.memory_info()
    };
    drop(session);
    assert!(
        ort::environment::try_shutdown().is_err(),
        "borrowed allocator info must retain its backing"
    );
    assert_eq!(info.device_type(), ort::memory::DeviceType::CPU);
    assert_eq!(info.device_id(), 0);
    drop(info);
    ort::environment::try_shutdown().unwrap();
}

fn check_allocator_info(internal: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "allocator_info_keeps_backing_alive_child",
            "--nocapture",
        ])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK");
    if internal {
        command.env("SPUR_OWNER_INTERNAL", "1");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn external_allocator_info_keeps_backing_alive() {
    check_allocator_info(false);
}

#[test]
fn internal_allocator_info_keeps_backing_alive() {
    check_allocator_info(true);
}

struct PathMustNotBeConverted;
impl AsRef<std::path::Path> for PathMustNotBeConverted {
    fn as_ref(&self) -> &std::path::Path {
        panic!("adapter path was converted");
    }
}

#[test]
fn device_adapter_rejected_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let environment = ort::environment::get_environment().unwrap();
    let allocator = ort::memory::Allocator::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if std::env::var_os("SPUR_OWNER_FILE").is_some() {
            ort::adapter::Adapter::from_file(PathMustNotBeConverted, Some(&allocator))
        } else {
            ort::adapter::Adapter::from_memory(b"invalid adapter", Some(&allocator))
        }
        .map_err(|error| error.to_string())
    }));
    drop(allocator);
    drop(environment);
    ort::environment::try_shutdown().unwrap();
    let error = result.expect("adapter path was converted").unwrap_err();
    assert!(
        error.contains("managed-environment") && error.contains("allocator"),
        "{error}"
    );
}

fn check_device_adapter(file: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "device_adapter_rejected_child", "--nocapture"])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK");
    if file {
        command.env("SPUR_OWNER_FILE", "1");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn device_adapter_file_rejected_before_path_conversion() {
    check_device_adapter(true);
}
#[test]
fn device_adapter_memory_rejected_before_native_creation() {
    check_device_adapter(false);
}

#[test]
fn graph_initializer_rejected_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let environment = ort::environment::get_environment().unwrap();
    let mut graph = ort::editor::Graph::new().unwrap();
    let tensor =
        ort::value::Tensor::<f32>::new(&ort::memory::Allocator::default(), [1usize]).unwrap();
    let result = graph
        .add_initializer("weight", tensor, false)
        .map_err(|error| error.to_string());
    drop(graph);
    drop(environment);
    ort::environment::try_shutdown().unwrap();
    let error = result.expect_err("managed initializer transfer must be rejected");
    assert!(error.contains("managed-environment"), "{error}");
}

#[test]
fn graph_initializer_rejected_before_transfer() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "graph_initializer_rejected_child", "--nocapture"])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
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
fn tensor_construction_failure_releases_value_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let environment = ort::environment::get_environment().unwrap();
    let string = std::env::var_os("SPUR_OWNER_STRING").is_some();
    ort::owner_test_hooks::fail_next_tensor_step(if string { 2 } else { 1 });
    let error = if string {
        ort::value::Tensor::from_string_array(([1usize], &["hello"][..])).err()
    } else {
        ort::value::Tensor::<f32>::new(&ort::memory::Allocator::default(), [1usize]).err()
    }
    .expect("the native value was created before the injected failure");
    let releases = ort::owner_test_hooks::native_release_count();
    if string {
        let value = ort::value::Tensor::from_string_array(([1usize], &["hello"][..])).unwrap();
        assert_eq!(value.try_extract_strings().unwrap().1, &["hello"]);
        let invalid = ort::value::Tensor::from_string_array(([1usize], &["bad\0string"][..]));
        assert!(invalid.is_err());
    } else {
        let value =
            ort::value::Tensor::<f32>::new(&ort::memory::Allocator::default(), [1usize]).unwrap();
        assert_eq!(value.extract_tensor().1, &[0.0]);
    }
    drop(environment);
    ort::environment::try_shutdown().unwrap();
    assert!(error
        .to_string()
        .contains("injected tensor construction failure"));
    assert_eq!(
        releases, 1,
        "the created native value must be released once on failure"
    );
}

fn check_tensor_construction_failure(string: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "tensor_construction_failure_releases_value_child",
            "--nocapture",
        ])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK");
    if string {
        command.env("SPUR_OWNER_STRING", "1");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn numeric_tensor_failure_releases_native_value() {
    check_tensor_construction_failure(false);
}
#[test]
fn string_tensor_failure_releases_native_value() {
    check_tensor_construction_failure(true);
}

#[test]
fn sequence_extracted_value_keeps_allocator_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let session = ort::session::Session::builder()
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    let allocator =
        ort::memory::Allocator::new(&session, ort::memory::MemoryInfo::default()).unwrap();
    let tensor = ort::value::Tensor::from_array(([1usize], vec![42.0_f32])).unwrap();
    let sequence = ort::value::Sequence::new([tensor]).unwrap();
    ort::owner_test_hooks::reset_allocator_release_count();
    let values = sequence.extract_sequence(&allocator);
    let retained = allocator.owner_test_backing_count();
    if retained < 2 {
        // Drop the extracted values while their allocator is still live on RED.
        drop(values);
        drop(allocator);
        drop(session);
        drop(sequence);
        ort::environment::try_shutdown().unwrap();
        panic!("extracted tensor must retain its allocator backing");
    }
    drop(allocator);
    drop(session);
    let premature = ort::owner_test_hooks::allocator_release_count();
    assert_eq!(premature, 0, "extracted tensor must retain its allocator");
    assert_eq!(values[0].extract_tensor().1, &[42.0]);
    drop(values);
    drop(sequence);
    ort::environment::try_shutdown().unwrap();
    assert_eq!(ort::owner_test_hooks::allocator_release_count(), 1);
}

#[test]
fn sequence_extracted_value_keeps_allocator() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sequence_extracted_value_keeps_allocator_child",
            "--nocapture",
        ])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
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
fn editor_node_attributes_rejected_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let environment = ort::environment::get_environment().unwrap();
    let attribute = ort::operator::attribute::Attribute::new("alpha", 1.0_f32).unwrap();
    ort::owner_test_hooks::fail_next_editor_node();
    let result =
        ort::editor::Node::new("LeakyRelu", "", "node", ["input"], ["output"], &[attribute]);
    let error = match result {
        Ok(node) => {
            drop(node);
            None
        }
        Err(error) => Some(error.to_string()),
    };
    ort::owner_test_hooks::clear_failpoint();
    let empty: &[ort::operator::attribute::Attribute] = &[];
    let node =
        ort::editor::Node::new("Identity", "", "node", ["input"], ["output"], empty).unwrap();
    drop(node);
    drop(environment);
    ort::environment::try_shutdown().unwrap();
    assert!(
        error.unwrap_or_default().contains("managed-environment"),
        "node attributes must be rejected before native transfer"
    );
}

#[test]
fn editor_node_attributes_rejected_before_transfer() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "editor_node_attributes_rejected_child",
            "--nocapture",
        ])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
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
fn bundled_native_provider_inventory_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    use ort::execution_providers::{
        CANNExecutionProvider, CUDAExecutionProvider, ExecutionProvider, OneDNNExecutionProvider,
        ROCmExecutionProvider, TensorRTExecutionProvider,
    };
    let environment = ort::environment::get_environment().unwrap();
    for provider in [
        &CUDAExecutionProvider::default() as &dyn ExecutionProvider,
        &ROCmExecutionProvider::default(),
        &TensorRTExecutionProvider::default(),
        &CANNExecutionProvider::default(),
        &OneDNNExecutionProvider::default(),
    ] {
        assert!(
            !provider.is_available().unwrap(),
            "{} unexpectedly available",
            provider.name()
        );
        let mut options = ort::session::Session::builder().unwrap();
        let error = provider
            .register(&mut options)
            .expect_err("provider unexpectedly registered");
        println!(
            "{}: unavailable; registration error: {error:?}",
            provider.name()
        );
    }
    drop(environment);
    ort::environment::try_shutdown().unwrap();
}

#[test]
fn bundled_native_provider_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bundled_native_provider_inventory_child",
            "--nocapture",
        ])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
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
fn compiler_failed_output_is_freed_child() {
    let Some(cache) = std::env::var_os("SPUR_OWNER_TEST_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let compiler = ort::compiler::ModelCompiler::new(ort::session::Session::builder().unwrap())
        .unwrap()
        .with_model_from_memory(include_bytes!("fixtures/mul_1.onnx"))
        .unwrap();
    ort::owner_test_hooks::fail_next_compiler_after_output();
    let result = compiler.compile_to_buffer().err().map(|e| e.to_string());
    let frees = ort::owner_test_hooks::compiler_buffer_free_count();
    let compiled = compiler.compile_to_buffer().unwrap();
    assert!(!compiled.as_slice().is_empty());
    drop(compiled);
    drop(compiler);
    ort::environment::try_shutdown().unwrap();
    assert!(
        result
            .unwrap_or_default()
            .contains("injected tensor construction failure"),
        "expected injected failure after native output"
    );
    assert_eq!(frees, 1, "published output must be freed on error");
}

#[test]
fn compiler_failed_output_is_freed() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "compiler_failed_output_is_freed_child",
            "--nocapture",
        ])
        .env("SPUR_OWNER_TEST_CACHE", dir.path())
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
