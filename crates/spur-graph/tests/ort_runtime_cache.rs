#![cfg(all(target_os = "macos", feature = "embed"))]

#[path = "../src/store/ort_runtime.rs"]
mod ort_runtime;

#[test]
fn runtime_cache_reuses_valid_payload_and_repairs_corruption() {
    let cache = tempfile::tempdir().unwrap();
    let first = ort_runtime::materialize_ort_runtime(cache.path()).unwrap();
    let original = std::fs::read(&first).unwrap();
    assert!(!original.is_empty());
    let second = ort_runtime::materialize_ort_runtime(cache.path()).unwrap();
    assert_eq!(first, second);
    assert_eq!(std::fs::read(&second).unwrap(), original);
    std::fs::write(&first, b"corrupt").unwrap();
    let repaired = ort_runtime::materialize_ort_runtime(cache.path()).unwrap();
    assert_eq!(repaired, first);
    assert_eq!(std::fs::read(repaired).unwrap(), original);
}

#[test]
fn runtime_cache_reports_unwritable_location() {
    let cache = tempfile::tempdir().unwrap();
    let file = cache.path().join("not-a-directory");
    std::fs::write(&file, b"file").unwrap();
    assert!(ort_runtime::materialize_ort_runtime(&file).is_err());
}

#[test]
fn relocated_process_runs_ort_inference_then_pgq() {
    use std::process::Command;
    if let Some(cache) = std::env::var_os("SPUR_ORT_TEST_CACHE") {
        ort_runtime::ensure_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
        let model = include_bytes!("fixtures/mul_1.onnx");
        use sha2::{Digest, Sha256};
        assert_eq!(
            format!("{:x}", Sha256::digest(model)),
            "71f431c4e9321ec6fbeb158d02ed240459a7dcc98673fa79a4f439ce42efaf10"
        );
        let mut session = ort::session::Session::builder()
            .unwrap()
            .commit_from_memory(model)
            .unwrap();
        let input =
            ort::value::Tensor::from_array(([3usize, 2], vec![1.0_f32, 2., 3., 4., 5., 6.]))
                .unwrap();
        let outputs = session.run(ort::inputs![input]).unwrap();
        assert_eq!(
            outputs[0].try_extract_tensor::<f32>().unwrap().1,
            &[1., 4., 9., 16., 25., 36.]
        );
        println!("ORT output: 1,4,9,16,25,36");
        let db = duckdb::Connection::open_in_memory().unwrap();
        db.execute_batch("LOAD duckpgq;").unwrap();
        db.execute_batch("CREATE TABLE nodes (id INTEGER PRIMARY KEY, name VARCHAR); INSERT INTO nodes VALUES (1, 'a'), (2, 'b'); CREATE TABLE edges (src INTEGER, dst INTEGER); INSERT INTO edges VALUES (1, 2); CREATE PROPERTY GRAPH g VERTEX TABLES (nodes) EDGE TABLES (edges SOURCE KEY (src) REFERENCES nodes (id) DESTINATION KEY (dst) REFERENCES nodes (id));").unwrap();
        let name: String = db.query_row("FROM GRAPH_TABLE (g MATCH (a:nodes)-[e:edges]->(b:nodes) COLUMNS (b.name AS name)) SELECT name", [], |row| row.get(0)).unwrap();
        assert_eq!(name, "b");
        println!("PGQ result: {name}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("relocated-test");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let output = Command::new(executable)
        .args([
            "--exact",
            "relocated_process_runs_ort_inference_then_pgq",
            "--nocapture",
        ])
        .env("SPUR_ORT_TEST_CACHE", dir.path().join("cache"))
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    println!(
        "relocated stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "relocated stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
