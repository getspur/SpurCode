//! Regression coverage for the locked upstream v1 initialization/configuration contracts.

#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use spur_acp::connection::{native::NativeAcpConnection, AgentConnection};
use spur_acp::types::AgentHealth;
use spur_acp::{AgentKind, InitializeRequest, ProtocolVersion, SpurAgentCaps};

// A synthetic protocol peer isolates the wire boundary without billing a vendor agent.
const AGENT: &str = r#"
import json, os, sys
version, config_id, legacy, root = int(sys.argv[1]), sys.argv[2], sys.argv[3] == "true", sys.argv[4]
with open(root + "/pid", "w") as f:
    f.write(str(os.getpid()))
model = {"id": config_id, "name": "Model", "category": "model", "type": "select",
         "currentValue": "old", "options": [{"value": "old", "name": "Old"},
                                            {"value": "next", "name": "Next"}]}
effort = {"id": "reasoning_effort", "name": "Effort", "category": "thought_level", "type": "select",
          "currentValue": "low", "options": [{"value": "low", "name": "Low"}]}
options = [model, effort]
if legacy:
    options.insert(0, {"id": "model", "name": "Legacy Model", "type": "select",
                       "currentValue": "legacy", "options": [{"value": "legacy", "name": "Legacy"}]})
for line in sys.stdin:
    msg = json.loads(line)
    with open(root + "/wire.jsonl", "a") as log:
        log.write(json.dumps(msg) + "\n")
    if "id" not in msg:
        continue
    response = {"jsonrpc": "2.0", "id": msg["id"]}
    method = msg.get("method")
    if method == "initialize":
        response["result"] = {"protocolVersion": version, "agentCapabilities": {}, "authMethods": []}
    elif method == "session/new":
        response["result"] = {"sessionId": "contract-session", "configOptions": options}
    elif method == "session/set_config_option":
        if msg["params"]["configId"] != config_id:
            response["error"] = {"code": -32602, "message": "unadvertised model configId: " + msg["params"]["configId"]}
        else:
            model["currentValue"] = msg["params"]["value"]
            effort["currentValue"] = "high"
            effort["options"] = [{"value": "high", "name": "High"}]
            response["result"] = {"configOptions": options}
    else:
        response["error"] = {"code": -32601, "message": "unsupported mock method"}
    print(json.dumps(response), flush=True)
"#;

fn connection(version: u16, config_id: &str, legacy: bool, root: &Path) -> NativeAcpConnection {
    let mut conn = NativeAcpConnection::new_with_kind(
        "upstream-contract-test",
        "python3",
        vec![
            "-u".into(),
            "-c".into(),
            AGENT.into(),
            version.to_string(),
            config_id.into(),
            legacy.to_string(),
            root.display().to_string(),
        ],
        AgentKind::CodexAcp,
        None,
    );
    conn.set_repo_root(root.to_path_buf());
    conn
}

fn wire_messages(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("wire.jsonl"))
        .expect("wire capture")
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON-RPC frame"))
        .collect()
}

async fn set_model_case(config_id: &str, legacy: bool) {
    let root = tempfile::tempdir().unwrap();
    let mut conn = connection(1, config_id, legacy, root.path());
    let init = conn
        .initialize(InitializeRequest::new(ProtocolVersion::V1))
        .await
        .expect("v1 initialization");
    let session = conn
        .new_session(root.path().to_path_buf(), Vec::new())
        .await
        .expect("session/new");
    let caps = SpurAgentCaps::new(&init, &session, AgentKind::CodexAcp);
    assert_eq!(caps.model_option().unwrap().id.0.as_ref(), config_id);
    let result = conn
        .set_session_model(session.session_id, "next".into(), &caps)
        .await;
    conn.shutdown()
        .await
        .expect("cleanup even when assertion fails");

    let messages = wire_messages(root.path());
    let requests: Vec<_> = messages
        .iter()
        .filter(|msg| msg["method"] == "session/set_config_option")
        .collect();
    assert_eq!(requests.len(), 1, "send one configuration update");
    assert_eq!(requests[0]["params"]["configId"], config_id, "{result:?}");
    assert_eq!(requests[0]["params"]["value"], "next");

    // Keep the whole new snapshot, including dependent option changes.
    let options = result.expect("advertised model change must succeed");
    let snapshot = serde_json::to_value(options).unwrap();
    let options = snapshot.as_array().unwrap();
    assert_eq!(options.len(), if legacy { 3 } else { 2 });
    let selected = options
        .iter()
        .find(|option| option["id"] == config_id)
        .unwrap();
    assert_eq!(selected["currentValue"], "next");
    let effort = options
        .iter()
        .find(|option| option["id"] == "reasoning_effort")
        .unwrap();
    assert_eq!(effort["currentValue"], "high");
    assert_eq!(effort["options"][0]["value"], "high");
}

#[tokio::test]
async fn standard_model_config_id_and_updated_snapshot_are_preserved() {
    tokio::time::timeout(Duration::from_secs(10), set_model_case("model", false))
        .await
        .expect("model update watchdog");
}

#[tokio::test]
async fn advertised_vendor_model_config_id_is_preserved() {
    tokio::time::timeout(
        Duration::from_secs(10),
        set_model_case("vendor_model", false),
    )
    .await
    .expect("model update watchdog");
}

#[tokio::test]
async fn model_category_preference_is_preserved_on_the_wire() {
    tokio::time::timeout(
        Duration::from_secs(10),
        set_model_case("vendor_model", true),
    )
    .await
    .expect("model update watchdog");
}

async fn rejected_version_case(version: u16) {
    let root = tempfile::tempdir().unwrap();
    let mut conn = connection(version, "model", false, root.path());
    let result = conn
        .initialize(InitializeRequest::new(ProtocolVersion::V1))
        .await;
    let health = conn.health();
    let pid: i32 = std::fs::read_to_string(root.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    // Signal 0 only checks the fixture process; it sends no signal.
    let child_reaped = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
        == Err(nix::errno::Errno::ESRCH);
    let session = conn
        .new_session(root.path().to_path_buf(), Vec::new())
        .await;
    conn.shutdown().await.expect("cleanup even on RED");

    let error = result.expect_err("unsupported version must fail initialization");
    assert!(error.to_string().contains(&version.to_string()), "{error}");
    assert!(
        !matches!(health, AgentHealth::Ready),
        "unsupported peer became ready"
    );
    assert!(
        child_reaped,
        "initialize returned before the peer was reaped"
    );
    assert!(
        session.is_err(),
        "rejected connection must not create sessions"
    );
    assert!(wire_messages(root.path())
        .iter()
        .all(|msg| msg["method"] != "session/new"));
}

#[tokio::test]
async fn version_zero_is_rejected_and_cleaned_up() {
    tokio::time::timeout(Duration::from_secs(10), rejected_version_case(0))
        .await
        .expect("rejection cleanup watchdog");
}

#[tokio::test]
async fn unsupported_v2_is_rejected_and_cleaned_up() {
    tokio::time::timeout(Duration::from_secs(10), rejected_version_case(2))
        .await
        .expect("rejection cleanup watchdog");
}

#[tokio::test]
async fn maximum_version_is_rejected_and_cleaned_up() {
    tokio::time::timeout(Duration::from_secs(10), rejected_version_case(u16::MAX))
        .await
        .expect("rejection cleanup watchdog");
}

#[tokio::test]
async fn supported_v1_becomes_ready_and_creates_sessions() {
    let root = tempfile::tempdir().unwrap();
    let mut conn = connection(1, "model", false, root.path());
    tokio::time::timeout(Duration::from_secs(10), async {
        let response = conn
            .initialize(InitializeRequest::new(ProtocolVersion::V1))
            .await
            .unwrap();
        assert_eq!(response.protocol_version, ProtocolVersion::V1);
        assert!(matches!(conn.health(), AgentHealth::Ready));
        conn.new_session(root.path().to_path_buf(), Vec::new())
            .await
            .unwrap();
        conn.shutdown().await.unwrap();
    })
    .await
    .expect("supported initialization watchdog");
}
