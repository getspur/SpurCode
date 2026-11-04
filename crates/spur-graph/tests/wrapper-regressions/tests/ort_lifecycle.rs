#![cfg(all(target_os = "macos", feature = "embed"))]

use std::process::Command;

// The child calls the library's runtime initializer, sharing the same INIT
// used by production model loading. It does not perform test-only cleanup.
#[test]
fn production_child() {
    let Some(mode) = std::env::var_os("SPUR_LIFECYCLE_CHILD") else {
        return;
    };
    let mode = mode.to_str().unwrap();
    if mode != "pgq" {
        let cache = std::env::var_os("SPUR_LIFECYCLE_CACHE").unwrap();
        spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
        if mode == "repeat" {
            spur_graph::store::ensure_bundled_ort_runtime_at(std::path::Path::new(&cache)).unwrap();
        }
        if mode == "failed_session" {
            assert!(ort::session::Session::builder()
                .unwrap()
                .commit_from_memory(b"not an ONNX model")
                .is_err());
        }
        if mode == "multiple" {
            let first = ort::session::Session::builder()
                .unwrap()
                .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
                .unwrap();
            let other = std::thread::spawn(|| {
                ort::session::Session::builder()
                    .unwrap()
                    .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
                    .unwrap()
            });
            let mut second = other.join().unwrap();
            drop(first);
            let input =
                ort::value::Tensor::from_array(([3usize, 2], vec![1.0_f32, 2., 3., 4., 5., 6.]))
                    .unwrap();
            let outputs = second.run(ort::inputs![input]).unwrap();
            assert_eq!(
                outputs[0].try_extract_tensor::<f32>().unwrap().1,
                &[1., 4., 9., 16., 25., 36.]
            );
            drop(outputs);
            drop(second);
        }
        let mut session = ort::session::Session::builder()
            .unwrap()
            .commit_from_memory(include_bytes!("fixtures/mul_1.onnx"))
            .unwrap();
        let input =
            ort::value::Tensor::from_array(([3usize, 2], vec![1.0_f32, 2., 3., 4., 5., 6.]))
                .unwrap();
        let outputs = session.run(ort::inputs![input]).unwrap();
        if mode == "outstanding_value" {
            let value = outputs.into_iter().map(|(_, value)| value).next().unwrap();
            drop(session);
            assert_eq!(
                value.try_extract_tensor::<f32>().unwrap().1,
                &[1., 4., 9., 16., 25., 36.]
            );
        } else {
            assert_eq!(
                outputs[0].try_extract_tensor::<f32>().unwrap().1,
                &[1., 4., 9., 16., 25., 36.]
            );
        }
        println!("ORT output: 1,4,9,16,25,36");
    }
    if mode == "pgq" || mode == "combined" {
        let db = duckdb::Connection::open_in_memory().unwrap();
        db.execute_batch("LOAD duckpgq;").unwrap();
        db.execute_batch("CREATE TABLE nodes (id INTEGER PRIMARY KEY, name VARCHAR); INSERT INTO nodes VALUES (1, 'a'), (2, 'b'); CREATE TABLE edges (src INTEGER, dst INTEGER); INSERT INTO edges VALUES (1, 2); CREATE PROPERTY GRAPH g VERTEX TABLES (nodes) EDGE TABLES (edges SOURCE KEY (src) REFERENCES nodes (id) DESTINATION KEY (dst) REFERENCES nodes (id));").unwrap();
        let name: String = db.query_row("FROM GRAPH_TABLE (g MATCH (a:nodes)-[e:edges]->(b:nodes) COLUMNS (b.name AS name)) SELECT name", [], |row| row.get(0)).unwrap();
        assert_eq!(name, "b");
        println!("PGQ result: {name}");
    }
}

fn child(mode: &str, relocate: bool) {
    let dir = tempfile::tempdir().unwrap();
    let current = std::env::current_exe().unwrap();
    let executable = if relocate {
        let path = dir.path().join("relocated-lifecycle");
        std::fs::copy(current, &path).unwrap();
        path
    } else {
        current
    };
    let output = Command::new(executable)
        .args(["--exact", "production_child", "--nocapture"])
        .env("SPUR_LIFECYCLE_CHILD", mode)
        .env("SPUR_LIFECYCLE_CACHE", dir.path().join("cache"))
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("ORT_LIB_LOCATION")
        .env_remove("ORT_PREFER_DYNAMIC_LINK")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mode={mode} status={}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn production_ort_process_exits_normally() {
    child("ort", false);
}

#[test]
fn production_pgq_process_exits_normally() {
    child("pgq", false);
}

#[test]
fn production_combined_process_exits_normally() {
    child("combined", false);
}

#[test]
fn relocated_production_combined_process_exits_normally() {
    child("combined", true);
}

#[test]
fn repeated_production_initialization_exits_normally() {
    child("repeat", false);
}

#[test]
fn failed_session_then_valid_session_exits_normally() {
    child("failed_session", false);
}

#[test]
fn session_owners_on_distinct_threads_drop_before_normal_exit() {
    child("multiple", false);
}

#[test]
fn output_value_survives_session_drop_before_normal_exit() {
    child("outstanding_value", false);
}
