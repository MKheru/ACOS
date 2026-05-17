use serde_json::{json, Value};

const SCHEMA: &str = include_str!("../../../api/schema/ws13_messages.schema.json");

fn required<'a>(schema: &'a Value, section: &[&str]) -> Vec<&'a str> {
    let mut node = schema;
    for key in section {
        node = node.get(*key).expect("schema section exists");
    }
    node.as_array()
        .expect("required is an array")
        .iter()
        .map(|entry| entry.as_str().expect("required entries are strings"))
        .collect()
}

fn reject_missing_required(value: &Value, required_fields: &[&str]) -> Result<(), String> {
    for field in required_fields {
        if value.get(*field).is_none() {
            return Err(format!("missing required field {field}"));
        }
    }
    Ok(())
}

fn validate_ws13_envelope(schema: &Value, envelope: &Value) -> Result<(), String> {
    reject_missing_required(envelope, &required(schema, &["required"]))?;
    let signature = envelope
        .get("signature")
        .ok_or_else(|| "missing required field signature".to_string())?;
    reject_missing_required(
        signature,
        &required(schema, &["$defs", "signature", "required"]),
    )?;
    let capability_target = envelope
        .get("capability_target")
        .ok_or_else(|| "missing required field capability_target".to_string())?;
    reject_missing_required(
        capability_target,
        &required(schema, &["$defs", "capability_target", "required"]),
    )?;

    let version = envelope
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| "version must be a string".to_string())?;
    if version != "ws13.v1" {
        return Err("version must equal ws13.v1".to_string());
    }

    let payload_hash = envelope
        .get("payload_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| "payload_hash must be a string".to_string())?;
    if !(payload_hash.starts_with("sha256:") && payload_hash.len() == "sha256:".len() + 64) {
        return Err("payload_hash must be sha256:<64 lowercase hex>".to_string());
    }
    if !payload_hash["sha256:".len()..]
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("payload_hash must use lowercase hex".to_string());
    }

    let signature_value = signature
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| "signature.value must be a string".to_string())?;
    if signature_value.len() < 43 {
        return Err("signature.value too short for HMAC-SHA256/Ed25519 output".to_string());
    }

    Ok(())
}

fn valid_envelope() -> Value {
    json!({
        "version": "ws13.v1",
        "kind": "event.subscribe",
        "session_id": "session-0123456789abcdef",
        "nonce": "nonce-0123456789abcdef",
        "ts": 1779027000000_u64,
        "capability_target": {
            "scope": "ObservabilityRead",
            "operation": "observability.recent",
            "resource": "/api/observability/recent"
        },
        "payload_hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "payload": {
            "limit": 100
        },
        "signature": {
            "alg": "HMAC-SHA256",
            "key_id": "session-key-1",
            "value": "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ"
        }
    })
}

#[test]
fn ws13_schema_lints_as_json_schema_document() {
    let schema: Value = serde_json::from_str(SCHEMA).expect("schema parses as JSON");

    assert_eq!(
        schema.get("$schema").and_then(Value::as_str),
        Some("https://json-schema.org/draft/2020-12/schema")
    );
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
    assert!(required(&schema, &["required"]).contains(&"signature"));
    assert!(required(&schema, &["required"]).contains(&"payload_hash"));
    assert!(required(&schema, &["required"]).contains(&"capability_target"));
}

#[test]
fn ws13_schema_validates_signed_envelope_example() {
    let schema: Value = serde_json::from_str(SCHEMA).expect("schema parses as JSON");
    let envelope = valid_envelope();

    validate_ws13_envelope(&schema, &envelope).expect("valid signed envelope passes schema");
}

#[test]
fn ws13_schema_rejects_missing_signature() {
    let schema: Value = serde_json::from_str(SCHEMA).expect("schema parses as JSON");
    let mut envelope = valid_envelope();
    envelope
        .as_object_mut()
        .expect("test envelope is an object")
        .remove("signature");

    let error = validate_ws13_envelope(&schema, &envelope)
        .expect_err("missing signature must fail validation");
    assert!(error.contains("signature"), "unexpected error: {error}");
}
