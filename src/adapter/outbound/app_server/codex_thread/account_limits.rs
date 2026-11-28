//! A full read from the *same* authenticated app-server that will receive the turn. No cache,
//! notification, token count or reset timestamp is a substitute for backend permission.
use crate::application::port::outbound::issue_account_quota_port::IssueAccountQuota;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    ordinary_usage_allowed: Option<bool>,
    rate_limits: Option<Bucket>,
    rate_limits_by_limit_id: Option<CodexBuckets>,
}

#[derive(Deserialize)]
struct CodexBuckets {
    codex: Option<Bucket>,
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

pub(in super::super) fn admission(reply: Value) -> IssueAccountQuota {
    match evaluate(reply) {
        Ok(()) => IssueAccountQuota::Allowed,
        Err(reason) => reason,
    }
}

fn evaluate(reply: Value) -> Result<(), IssueAccountQuota> {
    use IssueAccountQuota::{Exhausted, Unknown};
    let limits: Limits = serde_json::from_value(reply).map_err(|_| Unknown)?;
    match limits.ordinary_usage_allowed {
        Some(false) => return Err(Exhausted),
        None => return Err(Unknown),
        Some(true) => {}
    }
    let bucket = limits
        .rate_limits_by_limit_id
        .and_then(|buckets| buckets.codex)
        .map_or_else(
            || {
                limits
                    .rate_limits
                    .filter(|bucket| bucket.limit_id.as_deref() == Some("codex"))
            },
            |bucket| match bucket.limit_id.as_deref() {
                None | Some("codex") => Some(bucket),
                Some(_) => None,
            },
        )
        .ok_or(Unknown)?;
    if bucket.spend_control_reached == Some(true) {
        return Err(Exhausted);
    }
    match (bucket.primary, bucket.secondary) {
        (None, None) => Err(Unknown),
        (primary, secondary)
            if primary
                .iter()
                .chain(secondary.iter())
                .any(|window| window.used_percent >= 100) =>
        {
            Err(Exhausted)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use IssueAccountQuota::{Allowed, Exhausted, Unknown};
    use serde_json::json;
    #[test]
    fn only_a_fresh_positive_reply_with_remaining_codex_capacity_permits_a_call() {
        let snapshot = |allowed: Value, bucket: &str, used: Value| json!({"ordinaryUsageAllowed":allowed,"rateLimits":{"limitId":bucket,"primary":{"usedPercent":12},"secondary":{"usedPercent":used,"resetsAt":1}}});
        assert_eq!(
            admission(snapshot(json!(true), "codex", json!(45))),
            Allowed
        );
        assert_eq!(
            admission(snapshot(json!(true), "codex", json!(100))),
            Exhausted
        );
        assert_eq!(
            admission(snapshot(json!(false), "codex", json!(0))),
            Exhausted
        );
        assert_eq!(admission(snapshot(Value::Null, "codex", json!(0))), Unknown);
        assert_eq!(admission(snapshot(json!(true), "other", json!(0))), Unknown);
        assert_eq!(
            admission(json!({"ordinaryUsageAllowed":true,"rateLimits":{"limitId":"codex"}})),
            Unknown
        );
        for (used, result) in [(99, Allowed), (100, Exhausted)] {
            assert_eq!(
                admission(json!({
                    "ordinaryUsageAllowed": true,
                    "rateLimits": {"limitId": "other", "primary": {"usedPercent": 0}},
                    "rateLimitsByLimitId": {"codex": {"primary": {"usedPercent": used}}}
                })),
                result
            );
        }
    }
}
