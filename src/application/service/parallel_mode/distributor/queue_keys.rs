use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn distributor_claim_owner_token(queue_item_id: &str) -> String {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "distributor-queue-head-{}-{}-{unique_suffix}",
        std::process::id(),
        sanitize_runtime_record_key(queue_item_id)
    )
}

pub(super) fn sanitize_runtime_record_key(value: &str) -> String {
    let mut key = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            key.push(ch);
        } else {
            key.push('_');
        }
    }
    key
}
