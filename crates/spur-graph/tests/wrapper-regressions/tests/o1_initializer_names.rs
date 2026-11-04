#![cfg(all(target_os = "macos", feature = "embed"))]

#[test]
fn overridable_initializer_names_are_freed_and_inference_remains_usable() {
    let cache = tempfile::tempdir().unwrap();
    spur_graph::store::ensure_bundled_ort_runtime_at(cache.path()).unwrap();
    let mut session = ort::session::Session::builder()
        .unwrap()
        .commit_from_memory(include_bytes!("fixtures/overridable_initializer.onnx"))
        .unwrap();

    ort::owner_test_hooks::reset_allocator_free_count();
    for _ in 0..3 {
        let initializers = session.overridable_initializers();
        assert_eq!(initializers.len(), 1);
        assert_eq!(initializers[0].name(), "weight");
        assert!(matches!(
            initializers[0].dtype(),
            ort::value::ValueType::Tensor { .. }
        ));
    }
    let observed_frees = ort::owner_test_hooks::allocator_free_count();

    let input = ort::value::Tensor::from_array(([1usize], vec![3.0_f32])).unwrap();
    let outputs = session.run(ort::inputs![input]).unwrap();
    assert_eq!(outputs[0].try_extract_tensor::<f32>().unwrap().1, &[6.0]);
    drop(outputs);
    drop(session);
    ort::environment::try_shutdown().unwrap();
    assert_eq!(
        observed_frees, 3,
        "each returned name needs its matching native allocator free"
    );
}
