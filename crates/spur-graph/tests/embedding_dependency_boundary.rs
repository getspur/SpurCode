//! The graph process must delegate inference to the native-free client.
#[test]
fn graph_normal_and_build_dependencies_do_not_include_native_inference() {
    let manifest: toml::Value = include_str!("../Cargo.toml").parse().unwrap();
    for table in ["dependencies", "build-dependencies"] {
        if let Some(dependencies) = manifest.get(table).and_then(toml::Value::as_table) {
            for forbidden in ["fastembed", "ort", "hf-hub"] {
                assert!(
                    !dependencies.contains_key(forbidden),
                    "{table} still contains {forbidden}"
                );
            }
        }
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, tables) in targets {
            for table in ["dependencies", "build-dependencies"] {
                if let Some(dependencies) = tables.get(table).and_then(toml::Value::as_table) {
                    for forbidden in ["fastembed", "ort", "hf-hub"] {
                        assert!(
                            !dependencies.contains_key(forbidden),
                            "{target} {table} still contains {forbidden}"
                        );
                    }
                }
            }
        }
    }
    assert!(
        !include_str!("../build.rs").contains("ort_runtime"),
        "graph still bundles the native runtime"
    );
}
