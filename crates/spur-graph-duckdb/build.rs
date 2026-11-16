//! Source provenance, not a binary fingerprint or a claim about Git HEAD.
//!
//! v1 hashes length-framed, sorted repository-relative names and exact bytes:
//! adapter + locked local dependencies' source/query/build trees and manifests,
//! the standalone lockfile, toolchain/config, and only inherited workspace
//! settings used by those manifests. This deliberately conservatively includes
//! each local dependency's entire source tree. Tests/docs/output directories and
//! unrelated workspace members are excluded. Ambient compiler flags and binary
//! bytes are outside this source identity; packaging owns the artifact SHA.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};
use toml::Value;

fn read_toml(path: &Path) -> Value {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .parse()
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn add_file(root: &Path, path: &Path, inputs: &mut BTreeMap<String, Vec<u8>>) {
    let relative = path.strip_prefix(root).expect("source input outside root");
    let label = relative
        .to_str()
        .expect("source path must be UTF-8")
        .replace('\\', "/");
    inputs.insert(label, fs::read(path).expect("read source input"));
    println!("cargo:rerun-if-changed={}", path.display());
}

fn add_tree(root: &Path, path: &Path, inputs: &mut BTreeMap<String, Vec<u8>>) {
    // Watching directories also invalidates provenance for additions/removals.
    println!("cargo:rerun-if-changed={}", path.display());
    for entry in fs::read_dir(path).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let kind = entry.file_type().expect("read source file type");
        assert!(
            !kind.is_symlink(),
            "source identity requires ordinary source files"
        );
        if kind.is_dir() {
            add_tree(root, &entry.path(), inputs);
        } else if kind.is_file() {
            add_file(root, &entry.path(), inputs);
        }
    }
}

fn inherited_dependencies(value: &Value, names: &mut BTreeSet<String>) {
    if let Some(table) = value.as_table() {
        for (key, value) in table {
            if matches!(
                key.as_str(),
                "dependencies" | "dev-dependencies" | "build-dependencies"
            ) {
                for (name, dependency) in value.as_table().expect("dependency table") {
                    if dependency.get("workspace").and_then(Value::as_bool) == Some(true) {
                        names.insert(name.clone());
                    }
                }
            } else {
                inherited_dependencies(value, names);
            }
        }
    }
}

fn main() {
    let adapter =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let root = adapter
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let lock = read_toml(&adapter.join("Cargo.lock"));
    let workspace = read_toml(&root.join("Cargo.toml"));
    let mut inputs = BTreeMap::new();
    let mut inherited = BTreeMap::new();
    let mut dependency_names = BTreeSet::new();

    // Cargo.lock is the standalone resolved dependency inventory. Registry/Git
    // packages are bound by its versions, sources and checksums; local packages
    // have no source/checksum, so bind their actual working-tree inputs as well.
    for package in lock["package"].as_array().expect("locked packages") {
        if package.get("source").is_some() {
            continue;
        }
        let name = package["name"].as_str().expect("locked package name");
        let directory = root.join("crates").join(name);
        let manifest_path = directory.join("Cargo.toml");
        let manifest = read_toml(&manifest_path);
        assert_eq!(manifest["package"]["name"].as_str(), Some(name));
        add_file(root, &manifest_path, &mut inputs);
        inherited_dependencies(&manifest, &mut dependency_names);
        for (key, value) in manifest["package"].as_table().expect("package table") {
            if value.get("workspace").and_then(Value::as_bool) == Some(true) {
                inherited.insert(
                    format!("package.{key}"),
                    workspace["workspace"]["package"][key].clone(),
                );
            }
        }
        for name in ["src", "queries", "bindings", "build_support"] {
            let path = directory.join(name);
            if path.is_dir() {
                add_tree(root, &path, &mut inputs);
            }
        }
        let build = directory.join("build.rs");
        if build.is_file() {
            add_file(root, &build, &mut inputs);
        }
    }
    for name in dependency_names {
        inherited.insert(
            format!("dependencies.{name}"),
            workspace["workspace"]["dependencies"][&name].clone(),
        );
    }
    // Hash the relevant projection, not workspace membership, unrelated
    // dependency declarations, Git status, commit IDs or absolute locations.
    let inherited = serde_json::to_string(&inherited).expect("serialize inherited build settings");
    inputs.insert(
        "Cargo.toml#inherited".to_owned(),
        inherited.as_bytes().to_vec(),
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );
    add_file(root, &adapter.join("Cargo.lock"), &mut inputs);
    for name in [
        "rust-toolchain.toml",
        "rust-toolchain",
        ".cargo/config.toml",
        ".cargo/config",
    ] {
        let path = root.join(name);
        if path.is_file() {
            add_file(root, &path, &mut inputs);
        }
    }

    let mut digest = Sha256::new();
    digest.update(b"spur-graph-extractor-source-v1\0");
    let mut records = Vec::new();
    for (label, bytes) in inputs {
        digest.update((label.len() as u64).to_le_bytes());
        digest.update(label.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
        let mut record = json!({"path": label, "sha256": format!("{:x}", Sha256::digest(&bytes))});
        if label == "Cargo.toml#inherited" {
            record["content"] = json!(inherited);
        }
        records.push(record);
    }
    let extractor_id = format!("source-sha256-v1:{:x}", digest.finalize());
    let receipt = json!({"extractor_id": extractor_id, "inputs": records});
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"))
            .join("spur_graph_source_identity.json"),
        serde_json::to_vec_pretty(&receipt).expect("serialize source receipt"),
    )
    .expect("write source receipt");
    println!("cargo:rustc-env=SPUR_GRAPH_EXTRACTOR_ID={extractor_id}");
}
