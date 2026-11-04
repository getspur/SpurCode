#![cfg(all(target_os = "macos", feature = "embed"))]

use ort::{
    memory::{MemoryInfo, MemoryType},
    operator::{
        io::{OperatorInput, OperatorOutput},
        kernel::{Kernel, KernelAttributes, KernelContext},
        Operator, OperatorDomain,
    },
    tensor::TensorElementType,
};
use std::{
    process::Command,
    sync::{Arc, Mutex},
};

struct Probe(Arc<Mutex<Vec<(bool, String)>>>);

impl Operator for Probe {
    fn name(&self) -> &str {
        "Probe"
    }
    fn inputs(&self) -> Vec<OperatorInput> {
        vec![
            OperatorInput::required(TensorElementType::Float32),
            OperatorInput::required(TensorElementType::Float32),
        ]
    }
    fn outputs(&self) -> Vec<OperatorOutput> {
        vec![OperatorOutput::required(TensorElementType::Float32)]
    }
    fn create_kernel(&self, attributes: &KernelAttributes) -> ort::Result<Box<dyn Kernel>> {
        let result = attributes.allocator(MemoryType::Default);
        self.0.lock().unwrap().push((
            result.is_err(),
            result.err().map(|e| e.to_string()).unwrap_or_default(),
        ));
        let observations = Arc::clone(&self.0);
        Ok(Box::new(move |ctx: &KernelContext| {
            let result = ctx.allocator(&MemoryInfo::default());
            observations.lock().unwrap().push((
                result.is_err(),
                result.err().map(|e| e.to_string()).unwrap_or_default(),
            ));
            let x = ctx.input(0)?.unwrap();
            let (shape, values) = x.try_extract_tensor::<f32>()?;
            let mut y = ctx.output(0, shape.to_vec())?.unwrap();
            y.try_extract_tensor_mut::<f32>()?.1.copy_from_slice(values);
            Ok(())
        }))
    }
}

#[test]
fn managed_custom_allocator_child() {
    let Some(cache) = std::env::var_os("SPUR_CUSTOM_ALLOCATOR_CACHE") else {
        return;
    };
    spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
    let observations = Arc::new(Mutex::new(Vec::new()));
    let mut session = ort::session::Session::builder()
        .unwrap()
        .with_operators(
            OperatorDomain::new("test.custom")
                .unwrap()
                .add(Probe(Arc::clone(&observations)))
                .unwrap(),
        )
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/custom_allocator_probe.onnx"))
        .unwrap();
    let input =
        ort::value::Tensor::from_array(([3usize, 2], vec![1.0_f32, 2., 3., 4., 5., 6.])).unwrap();
    let output = session.run(ort::inputs![input]).unwrap();
    assert_eq!(
        output[0].try_extract_tensor::<f32>().unwrap().1,
        &[1., 2., 3., 4., 5., 6.]
    );
    let seen = observations.lock().unwrap().clone();
    drop(output);
    drop(session);
    ort::environment::try_shutdown().unwrap();
    assert_eq!(seen.len(), 2, "expected both actual custom-op callbacks");
    assert!(
        seen.iter()
            .all(|(rejected, message)| *rejected && message.contains("managed-environment")),
        "both custom operator allocators must be rejected: {seen:?}"
    );
}

#[test]
fn managed_custom_allocators_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "managed_custom_allocator_child", "--nocapture"])
        .env("SPUR_CUSTOM_ALLOCATOR_CACHE", dir.path())
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
