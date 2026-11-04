use std::path::Path;
use std::process::Command;

#[test]
fn default_product_embedding_closure_has_no_native_inference_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("analyst crate lives under crates");
    for package in ["spur-graph", "spur-analyst", "spur-core", "spur-cli"] {
        let output = Command::new(root.join("scripts/spur-cargo"))
            .current_dir(root)
            .args([
                "tree",
                "-p",
                package,
                "--edges",
                "normal,build",
                "--prefix",
                "none",
            ])
            .output()
            .expect("run dependency tree");
        assert!(
            output.status.success(),
            "dependency tree for {package} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let tree = String::from_utf8(output.stdout).expect("UTF-8 dependency tree");
        for line in tree.lines() {
            let name = line.split_whitespace().next().unwrap_or_default();
            assert!(
                !["ort", "ort-sys", "fastembed"].contains(&name),
                "{package} default product closure contains {line}"
            );
        }
    }
}
