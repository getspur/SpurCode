use spur_embedding::{read_frame, ProtocolLimits, ReplyEnvelope, RequestEnvelope};
use std::io::Cursor;

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 4096,
        max_texts: 2,
        max_text_bytes: 4,
        max_total_text_bytes: 6,
        max_dimensions: 3,
        max_identity_bytes: 32,
        max_pending: 2,
    }
}

fn frame(body: &str) -> Cursor<Vec<u8>> {
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(body.as_bytes());
    Cursor::new(bytes)
}

#[test]
fn shutdown_rejects_unknown_variant_fields() {
    let body = r#"{"version":2,"generation":"00000000-0000-0000-0000-000000000001","request_id":1,"request":{"type":"Shutdown","extra":0}}"#;
    let result = read_frame::<_, RequestEnvelope>(&mut frame(body), &limits());
    println!("ROOT_OBSERVATION shutdown_unknown_field {result:?}");
    assert!(
        result.is_err(),
        "Shutdown must reject unknown variant fields"
    );
}

#[test]
fn closed_rejects_unknown_variant_fields() {
    let body = r#"{"version":2,"generation":"00000000-0000-0000-0000-000000000001","request_id":1,"reply":{"type":"Closed","extra":0}}"#;
    let result = read_frame::<_, ReplyEnvelope>(&mut frame(body), &limits());
    println!("ROOT_OBSERVATION closed_unknown_field {result:?}");
    assert!(result.is_err(), "Closed must reject unknown variant fields");
}
