use serde_json::Value;

pub(crate) fn json_payload_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value.contains(needle),
        Value::Array(values) => values
            .iter()
            .any(|value| json_payload_contains(value, needle)),
        Value::Object(values) => values
            .values()
            .any(|value| json_payload_contains(value, needle)),
        _ => false,
    }
}
