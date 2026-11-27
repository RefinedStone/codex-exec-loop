//! A full read from the *same* authenticated app-server that will receive the turn. No cache,
//! notification, token count or reset timestamp is a substitute for backend permission.
use crate::domain::delegation::EndReason;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    ordinary_usage_allowed: Option<bool>,
    rate_limits: Option<Bucket>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bucket {
    limit_id: Option<String>,
    primary: Option<Window>,
    secondary: Option<Window>,
    spend_control_reached: Option<bool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    used_percent: u16,
}

pub(super) fn admission(reply: Value) -> Result<(), EndReason> {
    let limits: Limits = serde_json::from_value(reply).map_err(|_| EndReason::Unavailable)?;
    match limits.ordinary_usage_allowed {
        Some(false) => return Err(EndReason::Quota),
        None => return Err(EndReason::Unavailable),
        Some(true) => {}
    }
    let bucket = limits.rate_limits.ok_or(EndReason::Unavailable)?;
    if bucket.limit_id.as_deref() != Some("codex") {
        return Err(EndReason::Unavailable);
    }
    if bucket.spend_control_reached == Some(true) {
        return Err(EndReason::Quota);
    }
    match (bucket.primary, bucket.secondary) {
        (None, None) => Err(EndReason::Unavailable),
        (primary, secondary)
            if primary
                .iter()
                .chain(secondary.iter())
                .any(|window| window.used_percent >= 100) =>
        {
            Err(EndReason::Quota)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_a_fresh_positive_reply_with_remaining_codex_capacity_permits_a_call() {
        let snapshot = |allowed: Value, bucket: &str, used: Value| json!({"ordinaryUsageAllowed":allowed,"rateLimits":{"limitId":bucket,"primary":{"usedPercent":12},"secondary":{"usedPercent":used,"resetsAt":1}}});
        assert_eq!(admission(snapshot(json!(true), "codex", json!(45))), Ok(()));
        assert_eq!(
            admission(snapshot(json!(true), "codex", json!(100))),
            Err(EndReason::Quota)
        );
        assert_eq!(
            admission(snapshot(json!(false), "codex", json!(0))),
            Err(EndReason::Quota)
        );
        assert_eq!(
            admission(snapshot(Value::Null, "codex", json!(0))),
            Err(EndReason::Unavailable)
        );
        assert_eq!(
            admission(snapshot(json!(true), "other", json!(0))),
            Err(EndReason::Unavailable)
        );
        assert_eq!(
            admission(json!({"ordinaryUsageAllowed":true,"rateLimits":{"limitId":"codex"}})),
            Err(EndReason::Unavailable)
        );
    }
}
