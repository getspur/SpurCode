use serde_json::{json, Value};

pub fn execute(_request: Value) -> anyhow::Result<Value> {
    Ok(json!({}))
}

fn main() {}
