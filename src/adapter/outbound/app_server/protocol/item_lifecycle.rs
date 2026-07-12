use serde_json::Value;

use crate::domain::conversation_item_lifecycle::{
    ConversationItemKind, ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
    ConversationItemLifecycleRejection, ConversationItemLifecycleSource, ConversationItemOutcome,
    MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES, MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES,
    MAX_CONVERSATION_ITEM_SUMMARY_BYTES,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ItemProjectionManifestRow {
    pub wire_type: &'static str,
    pub required_fields: &'static [&'static str],
    pub preserved_fields: &'static [&'static str],
    pub bounded_redacted_fields: &'static [&'static str],
    pub ignored_fields: &'static [&'static str],
    pub outcome_statuses: &'static [&'static str],
    pub open_outcome_status: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnknownItemProjectionDecision {
    pub preserved_fields: &'static [&'static str],
    pub ignore_all_other_fields: bool,
}

pub(crate) const UNKNOWN_ITEM_PROJECTION_DECISION: UnknownItemProjectionDecision =
    UnknownItemProjectionDecision {
        preserved_fields: &["id", "type"],
        ignore_all_other_fields: true,
    };

#[cfg(test)]
pub(crate) const SUBAGENT_ACTIVITY_OUTCOME_KINDS: &[&str] =
    &["started", "interacted", "interrupted"];

pub(crate) const ITEM_PROJECTION_MANIFEST: &[ItemProjectionManifestRow] = &[
    ItemProjectionManifestRow {
        wire_type: "userMessage",
        required_fields: &["content", "id", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["content"],
        ignored_fields: &["clientId"],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "hookPrompt",
        required_fields: &["fragments", "id", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["fragments"],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "agentMessage",
        required_fields: &["id", "text", "type"],
        preserved_fields: &["id", "phase", "type"],
        bounded_redacted_fields: &["text"],
        ignored_fields: &["memoryCitation"],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "plan",
        required_fields: &["id", "text", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["text"],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "reasoning",
        required_fields: &["id", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["content", "summary"],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "commandExecution",
        required_fields: &["command", "commandActions", "cwd", "id", "status", "type"],
        preserved_fields: &["id", "status", "type"],
        bounded_redacted_fields: &["command", "commandActions"],
        ignored_fields: &[
            "aggregatedOutput",
            "cwd",
            "durationMs",
            "exitCode",
            "processId",
            "source",
        ],
        outcome_statuses: &["inProgress", "completed", "failed", "declined"],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "fileChange",
        required_fields: &["changes", "id", "status", "type"],
        preserved_fields: &["id", "status", "type"],
        bounded_redacted_fields: &["changes"],
        ignored_fields: &[],
        outcome_statuses: &["inProgress", "completed", "failed", "declined"],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "mcpToolCall",
        required_fields: &["arguments", "id", "server", "status", "tool", "type"],
        preserved_fields: &["id", "server", "status", "tool", "type"],
        bounded_redacted_fields: &[],
        ignored_fields: &[
            "appContext",
            "arguments",
            "durationMs",
            "error",
            "mcpAppResourceUri",
            "pluginId",
            "result",
        ],
        outcome_statuses: &["inProgress", "completed", "failed"],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "dynamicToolCall",
        required_fields: &["arguments", "id", "status", "tool", "type"],
        preserved_fields: &["id", "namespace", "status", "success", "tool", "type"],
        bounded_redacted_fields: &[],
        ignored_fields: &["arguments", "contentItems", "durationMs"],
        outcome_statuses: &["inProgress", "completed", "failed"],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "collabAgentToolCall",
        required_fields: &[
            "agentsStates",
            "id",
            "receiverThreadIds",
            "senderThreadId",
            "status",
            "tool",
            "type",
        ],
        preserved_fields: &["id", "status", "tool", "type"],
        bounded_redacted_fields: &["receiverThreadIds"],
        ignored_fields: &[
            "agentsStates",
            "model",
            "prompt",
            "reasoningEffort",
            "senderThreadId",
        ],
        outcome_statuses: &["inProgress", "completed", "failed"],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "subAgentActivity",
        required_fields: &["agentPath", "agentThreadId", "id", "kind", "type"],
        preserved_fields: &["id", "kind", "type"],
        bounded_redacted_fields: &[],
        ignored_fields: &["agentPath", "agentThreadId"],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "webSearch",
        required_fields: &["id", "query", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["query"],
        ignored_fields: &["action"],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "imageView",
        required_fields: &["id", "path", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["path"],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "sleep",
        required_fields: &["durationMs", "id", "type"],
        preserved_fields: &["durationMs", "id", "type"],
        bounded_redacted_fields: &[],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "imageGeneration",
        required_fields: &["id", "result", "status", "type"],
        preserved_fields: &["id", "status", "type"],
        bounded_redacted_fields: &["result"],
        ignored_fields: &["revisedPrompt", "savedPath"],
        outcome_statuses: &[],
        open_outcome_status: true,
    },
    ItemProjectionManifestRow {
        wire_type: "enteredReviewMode",
        required_fields: &["id", "review", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["review"],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "exitedReviewMode",
        required_fields: &["id", "review", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &["review"],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
    ItemProjectionManifestRow {
        wire_type: "contextCompaction",
        required_fields: &["id", "type"],
        preserved_fields: &["id", "type"],
        bounded_redacted_fields: &[],
        ignored_fields: &[],
        outcome_statuses: &[],
        open_outcome_status: false,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ItemLifecycleParseError {
    MissingField(&'static str),
    InvalidField(&'static str),
    InvalidObservation(ConversationItemLifecycleRejection),
}

impl ItemLifecycleParseError {
    pub(super) const fn notice_label(&self) -> &'static str {
        match self {
            Self::MissingField(_) => "missing required item lifecycle field",
            Self::InvalidField(_) => "invalid item lifecycle field",
            Self::InvalidObservation(rejection) => rejection.notice_label(),
        }
    }
}

pub(super) fn parse_live_item_lifecycle(
    params: &Value,
    phase: ConversationItemLifecyclePhase,
) -> Result<ConversationItemLifecycleObservation, ItemLifecycleParseError> {
    let timestamp_field = match phase {
        ConversationItemLifecyclePhase::Started => "startedAtMs",
        ConversationItemLifecyclePhase::Completed => "completedAtMs",
        ConversationItemLifecyclePhase::SnapshotObserved => {
            return Err(ItemLifecycleParseError::InvalidField("lifecyclePhase"));
        }
    };
    let observed_at_ms = required_i64(params, timestamp_field)?;
    let thread_id = required_identifier(params, "threadId")?;
    let turn_id = required_identifier(params, "turnId")?;
    let item = params
        .get("item")
        .ok_or(ItemLifecycleParseError::MissingField("item"))?;

    parse_item_lifecycle(
        thread_id,
        turn_id,
        item,
        phase,
        ConversationItemLifecycleSource::Live,
        Some(observed_at_ms),
    )
}

pub(super) fn parse_snapshot_item_lifecycle(
    thread_id: &str,
    turn_id: &str,
    item: &Value,
) -> Result<ConversationItemLifecycleObservation, ItemLifecycleParseError> {
    parse_item_lifecycle(
        thread_id,
        turn_id,
        item,
        ConversationItemLifecyclePhase::SnapshotObserved,
        ConversationItemLifecycleSource::Snapshot,
        None,
    )
}

fn parse_item_lifecycle(
    thread_id: &str,
    turn_id: &str,
    item: &Value,
    phase: ConversationItemLifecyclePhase,
    source: ConversationItemLifecycleSource,
    observed_at_ms: Option<i64>,
) -> Result<ConversationItemLifecycleObservation, ItemLifecycleParseError> {
    validate_identifier(thread_id, "threadId")?;
    validate_identifier(turn_id, "turnId")?;
    let item_id = required_identifier(item, "id")?;
    let wire_type = required_discriminator(item, "type")?;
    let (kind, outcome, summary) = classify_item(item, wire_type)?;
    let observation = ConversationItemLifecycleObservation {
        thread_id: thread_id.to_string(),
        turn_id: turn_id.to_string(),
        item_id: item_id.to_string(),
        kind,
        phase,
        source,
        observed_at_ms,
        outcome,
        summary: bounded_summary(summary),
    };
    observation
        .validate()
        .map_err(ItemLifecycleParseError::InvalidObservation)?;
    Ok(observation)
}

fn classify_item(
    item: &Value,
    wire_type: &str,
) -> Result<(ConversationItemKind, ConversationItemOutcome, String), ItemLifecycleParseError> {
    let Some(decision) = ITEM_PROJECTION_MANIFEST
        .iter()
        .find(|decision| decision.wire_type == wire_type)
    else {
        return Ok(unknown_item_classification(wire_type));
    };
    let _declared_field_count = decision
        .preserved_fields
        .len()
        .saturating_add(decision.bounded_redacted_fields.len())
        .saturating_add(decision.ignored_fields.len());
    let _declared_outcome_count = decision.outcome_statuses.len();
    let _required_field_count = decision.required_fields.len();
    let _open_outcome_status = decision.open_outcome_status;

    match wire_type {
        "userMessage" => {
            let count = required_array(item, "content")?.len();
            Ok((
                ConversationItemKind::UserMessage,
                ConversationItemOutcome::NotReported,
                format!("user input fragments={count}"),
            ))
        }
        "hookPrompt" => {
            let count = required_array(item, "fragments")?.len();
            Ok((
                ConversationItemKind::HookPrompt,
                ConversationItemOutcome::NotReported,
                format!("hook prompt fragments={count}"),
            ))
        }
        "agentMessage" => {
            let text = required_string(item, "text")?;
            let phase =
                bounded_kind_label(optional_string(item, "phase")?.unwrap_or("unspecified"));
            Ok((
                ConversationItemKind::AgentMessage,
                ConversationItemOutcome::NotReported,
                format!("agent message bytes={}; phase={phase}", text.len()),
            ))
        }
        "plan" => {
            let text = required_string(item, "text")?;
            Ok((
                ConversationItemKind::Plan,
                ConversationItemOutcome::NotReported,
                format!("plan bytes={}", text.len()),
            ))
        }
        "reasoning" => {
            let content_count = optional_array(item, "content")?.map_or(0, <[Value]>::len);
            let summary_count = optional_array(item, "summary")?.map_or(0, <[Value]>::len);
            Ok((
                ConversationItemKind::Reasoning,
                ConversationItemOutcome::NotReported,
                format!("reasoning summary_parts={summary_count}; content_parts={content_count}"),
            ))
        }
        "commandExecution" => {
            let command = required_string(item, "command")?;
            let _ = required_array(item, "commandActions")?;
            let _ = required_string(item, "cwd")?;
            let status = required_string(item, "status")?;
            let status_label = bounded_kind_label(status);
            Ok((
                ConversationItemKind::CommandExecution,
                outcome_from_status(status),
                format!("command bytes={}; status={status_label}", command.len()),
            ))
        }
        "fileChange" => {
            let change_count = required_array(item, "changes")?.len();
            let status = required_string(item, "status")?;
            let status_label = bounded_kind_label(status);
            Ok((
                ConversationItemKind::FileChange,
                outcome_from_status(status),
                format!("file changes={change_count}; status={status_label}"),
            ))
        }
        "mcpToolCall" => {
            require_present(item, "arguments")?;
            let server = required_string(item, "server")?;
            let tool = required_string(item, "tool")?;
            let status = required_string(item, "status")?;
            let server = bounded_kind_label(server);
            let tool = bounded_kind_label(tool);
            let status_label = bounded_kind_label(status);
            Ok((
                ConversationItemKind::McpToolCall,
                outcome_from_status(status),
                format!("mcp server={server}; tool={tool}; status={status_label}"),
            ))
        }
        "dynamicToolCall" => {
            require_present(item, "arguments")?;
            let namespace =
                bounded_kind_label(optional_string(item, "namespace")?.unwrap_or("none"));
            let tool = bounded_kind_label(required_string(item, "tool")?);
            let status = required_string(item, "status")?;
            let status_label = bounded_kind_label(status);
            let success = optional_bool(item, "success")?;
            let outcome = if status == "completed" && success == Some(false) {
                ConversationItemOutcome::Unknown("completed-with-success-false".to_string())
            } else {
                outcome_from_status(status)
            };
            Ok((
                ConversationItemKind::DynamicToolCall,
                outcome,
                format!("dynamic tool namespace={namespace}; tool={tool}; status={status_label}"),
            ))
        }
        "collabAgentToolCall" => {
            let _ = required_object(item, "agentsStates")?;
            let receiver_count = required_array(item, "receiverThreadIds")?.len();
            let _ = required_string(item, "senderThreadId")?;
            let tool = bounded_kind_label(required_string(item, "tool")?);
            let status = required_string(item, "status")?;
            let status_label = bounded_kind_label(status);
            Ok((
                ConversationItemKind::CollaborationAgentToolCall,
                outcome_from_status(status),
                format!(
                    "collaboration tool={tool}; receivers={receiver_count}; status={status_label}"
                ),
            ))
        }
        "subAgentActivity" => {
            let _ = required_string(item, "agentPath")?;
            let _ = required_string(item, "agentThreadId")?;
            let kind = required_string(item, "kind")?;
            let kind_label = bounded_kind_label(kind);
            let outcome = outcome_from_subagent_activity(kind);
            Ok((
                ConversationItemKind::SubAgentActivity,
                outcome,
                format!("subagent activity={kind_label}"),
            ))
        }
        "webSearch" => {
            let query = required_string(item, "query")?;
            Ok((
                ConversationItemKind::WebSearch,
                ConversationItemOutcome::NotReported,
                format!("web search query_bytes={}", query.len()),
            ))
        }
        "imageView" => {
            let path = required_string(item, "path")?;
            Ok((
                ConversationItemKind::ImageView,
                ConversationItemOutcome::NotReported,
                format!("image view path_bytes={}", path.len()),
            ))
        }
        "sleep" => {
            let duration_ms = required_u64(item, "durationMs")?;
            Ok((
                ConversationItemKind::Sleep,
                ConversationItemOutcome::NotReported,
                format!("sleep duration_ms={duration_ms}"),
            ))
        }
        "imageGeneration" => {
            let result = required_string(item, "result")?;
            let status = required_string(item, "status")?;
            let status_label = bounded_kind_label(status);
            Ok((
                ConversationItemKind::ImageGeneration,
                outcome_from_status(status),
                format!(
                    "image generation result_bytes={}; status={status_label}",
                    result.len()
                ),
            ))
        }
        "enteredReviewMode" => {
            let review = required_string(item, "review")?;
            Ok((
                ConversationItemKind::EnteredReviewMode,
                ConversationItemOutcome::NotReported,
                format!(
                    "Codex runtime review mode entered; review_bytes={}",
                    review.len()
                ),
            ))
        }
        "exitedReviewMode" => {
            let review = required_string(item, "review")?;
            Ok((
                ConversationItemKind::ExitedReviewMode,
                ConversationItemOutcome::NotReported,
                format!(
                    "Codex runtime review mode exited; review_bytes={}",
                    review.len()
                ),
            ))
        }
        "contextCompaction" => Ok((
            ConversationItemKind::ContextCompaction,
            ConversationItemOutcome::NotReported,
            "context compacted".to_string(),
        )),
        unknown => Ok(unknown_item_classification(unknown)),
    }
}

fn unknown_item_classification(
    wire_type: &str,
) -> (ConversationItemKind, ConversationItemOutcome, String) {
    let _preserved_identity_fields = UNKNOWN_ITEM_PROJECTION_DECISION.preserved_fields;
    let _raw_payload_is_ignored = UNKNOWN_ITEM_PROJECTION_DECISION.ignore_all_other_fields;
    (
        ConversationItemKind::Unknown(bounded_kind_label(wire_type)),
        ConversationItemOutcome::Unknown("unclassified-item-kind".to_string()),
        "unclassified item kind".to_string(),
    )
}

fn outcome_from_status(status: &str) -> ConversationItemOutcome {
    match status {
        "inProgress" | "in_progress" => ConversationItemOutcome::InProgress,
        "completed" => ConversationItemOutcome::Completed,
        "failed" => ConversationItemOutcome::Failed,
        "declined" => ConversationItemOutcome::Declined,
        other => ConversationItemOutcome::Unknown(bounded_kind_label(other)),
    }
}

fn outcome_from_subagent_activity(kind: &str) -> ConversationItemOutcome {
    match kind {
        "started" | "interacted" => ConversationItemOutcome::NotReported,
        "interrupted" => ConversationItemOutcome::Interrupted,
        other => ConversationItemOutcome::Unknown(bounded_kind_label(other)),
    }
}

fn require_present<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a Value, ItemLifecycleParseError> {
    value
        .get(field)
        .ok_or(ItemLifecycleParseError::MissingField(field))
}

fn required_string<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, ItemLifecycleParseError> {
    require_present(value, field)?
        .as_str()
        .ok_or(ItemLifecycleParseError::InvalidField(field))
}

fn required_discriminator<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, ItemLifecycleParseError> {
    let discriminator = required_string(value, field)?;
    if discriminator.is_empty() {
        return Err(ItemLifecycleParseError::InvalidField(field));
    }
    Ok(discriminator)
}

fn required_identifier<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, ItemLifecycleParseError> {
    let identifier = required_string(value, field)?;
    if identifier.is_empty() {
        return Err(ItemLifecycleParseError::InvalidObservation(
            ConversationItemLifecycleRejection::MissingIdentity { field },
        ));
    }
    validate_identifier(identifier, field)?;
    Ok(identifier)
}

fn validate_identifier(
    identifier: &str,
    field: &'static str,
) -> Result<(), ItemLifecycleParseError> {
    if identifier.is_empty() {
        return Err(ItemLifecycleParseError::InvalidObservation(
            ConversationItemLifecycleRejection::MissingIdentity { field },
        ));
    }
    if identifier.len() > MAX_CONVERSATION_ITEM_IDENTIFIER_BYTES {
        return Err(ItemLifecycleParseError::InvalidObservation(
            ConversationItemLifecycleRejection::IdentifierTooLong { field },
        ));
    }
    Ok(())
}

fn optional_string<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<Option<&'a str>, ItemLifecycleParseError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or(ItemLifecycleParseError::InvalidField(field)),
    }
}

fn required_i64(value: &Value, field: &'static str) -> Result<i64, ItemLifecycleParseError> {
    require_present(value, field)?
        .as_i64()
        .ok_or(ItemLifecycleParseError::InvalidField(field))
}

fn required_u64(value: &Value, field: &'static str) -> Result<u64, ItemLifecycleParseError> {
    require_present(value, field)?
        .as_u64()
        .ok_or(ItemLifecycleParseError::InvalidField(field))
}

fn optional_bool(
    value: &Value,
    field: &'static str,
) -> Result<Option<bool>, ItemLifecycleParseError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or(ItemLifecycleParseError::InvalidField(field)),
    }
}

fn required_array<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a [Value], ItemLifecycleParseError> {
    require_present(value, field)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or(ItemLifecycleParseError::InvalidField(field))
}

fn optional_array<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<Option<&'a [Value]>, ItemLifecycleParseError> {
    match value.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .map(Some)
            .ok_or(ItemLifecycleParseError::InvalidField(field)),
    }
}

fn required_object<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a serde_json::Map<String, Value>, ItemLifecycleParseError> {
    require_present(value, field)?
        .as_object()
        .ok_or(ItemLifecycleParseError::InvalidField(field))
}

fn bounded_kind_label(value: &str) -> String {
    bounded_text(value, MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES)
}

fn bounded_summary(value: String) -> String {
    bounded_text(value.as_str(), MAX_CONVERSATION_ITEM_SUMMARY_BYTES)
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_string();
    }
    let marker = "...";
    let mut boundary = maximum_bytes.saturating_sub(marker.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}{marker}", &value[..boundary])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sensitive_item_payloads_reduce_to_non_secret_metadata() {
        let secret = "AKRA_ITEM_LIFECYCLE_SECRET_CANARY";
        let item = json!({
            "id": "command-1",
            "type": "commandExecution",
            "command": format!("curl -H Authorization:{secret}"),
            "commandActions": [{"type": "unknown", "command": secret}],
            "cwd": format!("/private/{secret}"),
            "status": "failed",
            "aggregatedOutput": secret
        });

        let observation = parse_snapshot_item_lifecycle("thread-1", "turn-1", &item).unwrap();

        assert_eq!(observation.kind, ConversationItemKind::CommandExecution);
        assert_eq!(observation.outcome, ConversationItemOutcome::Failed);
        assert!(!format!("{observation:?}").contains(secret));
        assert!(observation.summary.contains("command bytes="));
    }

    #[test]
    fn unknown_kind_keeps_only_a_bounded_label_and_drops_payload() {
        let secret = "AKRA_UNKNOWN_ITEM_PAYLOAD_SECRET";
        let item = json!({
            "id": "unknown-1",
            "type": "newUpstreamKind",
            "prompt": secret,
            "result": secret
        });

        let observation = parse_snapshot_item_lifecycle("thread-1", "turn-1", &item).unwrap();

        assert_eq!(
            observation.kind,
            ConversationItemKind::Unknown("newUpstreamKind".to_string())
        );
        assert!(!format!("{observation:?}").contains(secret));
    }

    #[test]
    fn sleep_duration_preserves_full_u64_range_and_rejects_negative_values() {
        let maximum = json!({
            "id": "sleep-max",
            "type": "sleep",
            "durationMs": u64::MAX
        });
        let observation = parse_snapshot_item_lifecycle("thread-1", "turn-1", &maximum).unwrap();
        assert_eq!(
            observation.summary,
            format!("sleep duration_ms={}", u64::MAX)
        );

        let negative = json!({
            "id": "sleep-negative",
            "type": "sleep",
            "durationMs": -1
        });
        assert!(matches!(
            parse_snapshot_item_lifecycle("thread-1", "turn-1", &negative),
            Err(ItemLifecycleParseError::InvalidField("durationMs"))
        ));
    }

    #[test]
    fn empty_started_payload_is_valid_but_type_label_remains_bounded() {
        let started = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "startedAtMs": 1,
            "item": {
                "id": "agent-empty",
                "type": "agentMessage",
                "text": ""
            }
        });
        let observation =
            parse_live_item_lifecycle(&started, ConversationItemLifecyclePhase::Started).unwrap();
        assert_eq!(
            observation.summary,
            "agent message bytes=0; phase=unspecified"
        );

        let oversized_type = "t".repeat(MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES + 1);
        let unknown = json!({
            "id": "unknown-bounded",
            "type": oversized_type,
        });
        let observation = parse_snapshot_item_lifecycle("thread-1", "turn-1", &unknown).unwrap();
        let ConversationItemKind::Unknown(label) = observation.kind else {
            panic!("oversized future type should remain an unknown item");
        };
        assert_eq!(label.len(), MAX_CONVERSATION_ITEM_KIND_LABEL_BYTES);
    }
}
