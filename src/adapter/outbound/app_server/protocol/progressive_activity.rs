use std::io::{self, Write};

use serde_json::Value;

use crate::domain::conversation_item_lifecycle::ConversationItemKind;
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
    ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    ConversationProgressiveActivityRejection, ConversationProgressiveFileChange,
    ConversationProgressiveFileChangeKind, ConversationProgressivePlanStep,
    ConversationProgressivePlanStepStatus, ConversationProgressiveTokenUsage,
    ConversationProgressiveTokenUsageBreakdown, MAX_PROGRESSIVE_ACTIVITY_IDENTIFIER_BYTES,
    MAX_PROGRESSIVE_ACTIVITY_PATH_BYTES, MAX_PROGRESSIVE_AGENT_DRAFT_BYTES,
    MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES, MAX_PROGRESSIVE_DIFF_DETAIL_BYTES,
    MAX_PROGRESSIVE_GUARDIAN_WARNING_BYTES, MAX_PROGRESSIVE_MCP_DETAIL_BYTES,
    MAX_PROGRESSIVE_PLAN_DETAIL_BYTES, MAX_PROGRESSIVE_PLAN_STEP_BYTES, MAX_PROGRESSIVE_PLAN_STEPS,
    MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS, bounded_progressive_prefix,
    bounded_progressive_tail,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityDisposition {
    Handled,
    ExplicitlyIgnored,
    DiagnosticOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityStability {
    Stable,
    Experimental,
    Deprecated,
    Diagnostic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityCoalescingMode {
    AppendByItem,
    ReplaceByItem,
    ReplaceByTurn,
    ReplaceByThread,
    CountByItem,
    CountByItemAndIndex,
    CountByTurn,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProgressiveActivityManifestOptions {
    disposition: ProgressiveActivityDisposition,
    stability: ProgressiveActivityStability,
    coalescing: ProgressiveActivityCoalescingMode,
}

impl ProgressiveActivityManifestOptions {
    const fn new(
        disposition: ProgressiveActivityDisposition,
        stability: ProgressiveActivityStability,
        coalescing: ProgressiveActivityCoalescingMode,
    ) -> Self {
        Self {
            disposition,
            stability,
            coalescing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProgressiveActivityManifestRow {
    pub method: &'static str,
    pub required_fields: &'static [&'static str],
    pub preserved_fields: &'static [&'static str],
    pub bounded_redacted_fields: &'static [&'static str],
    pub ignored_fields: &'static [&'static str],
    pub compatibility_fields: &'static [&'static str],
    pub disposition: ProgressiveActivityDisposition,
    pub stability: ProgressiveActivityStability,
    pub coalescing: ProgressiveActivityCoalescingMode,
}

pub(crate) const PROGRESSIVE_ACTIVITY_MANIFEST: &[ProgressiveActivityManifestRow] = &[
    manifest_row(
        "item/agentMessage/delta",
        &["delta", "itemId", "threadId", "turnId"],
        &["itemId", "threadId", "turnId"],
        &["delta", "phase"],
        &[],
        &["phase"],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::AppendByItem,
        ),
    ),
    manifest_row(
        "item/commandExecution/outputDelta",
        &["delta", "itemId", "threadId", "turnId"],
        &["itemId", "threadId", "turnId"],
        &["delta"],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::AppendByItem,
        ),
    ),
    manifest_row(
        "item/commandExecution/terminalInteraction",
        &["itemId", "processId", "stdin", "threadId", "turnId"],
        &["itemId", "processId", "threadId", "turnId"],
        &[],
        &["stdin"],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::CountByItem,
        ),
    ),
    manifest_row(
        "item/fileChange/patchUpdated",
        &["changes", "itemId", "threadId", "turnId"],
        &["itemId", "threadId", "turnId"],
        &["changes"],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::ReplaceByItem,
        ),
    ),
    manifest_row(
        "turn/diff/updated",
        &["diff", "threadId", "turnId"],
        &["threadId", "turnId"],
        &["diff"],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::ReplaceByTurn,
        ),
    ),
    manifest_row(
        "turn/plan/updated",
        &["plan", "threadId", "turnId"],
        &["threadId", "turnId"],
        &["explanation", "plan"],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::ReplaceByTurn,
        ),
    ),
    manifest_row(
        "thread/tokenUsage/updated",
        &["threadId", "tokenUsage", "turnId"],
        &["threadId", "tokenUsage", "turnId"],
        &[],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::ReplaceByTurn,
        ),
    ),
    manifest_row(
        "item/mcpToolCall/progress",
        &["itemId", "message", "threadId", "turnId"],
        &["itemId", "threadId", "turnId"],
        &["message"],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::ReplaceByItem,
        ),
    ),
    manifest_row(
        "item/plan/delta",
        &["delta", "itemId", "threadId", "turnId"],
        &["itemId", "threadId", "turnId"],
        &[],
        &["delta"],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Experimental,
            ProgressiveActivityCoalescingMode::CountByItem,
        ),
    ),
    manifest_row(
        "item/reasoning/summaryTextDelta",
        &["delta", "itemId", "summaryIndex", "threadId", "turnId"],
        &["itemId", "summaryIndex", "threadId", "turnId"],
        &[],
        &["delta"],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::CountByItemAndIndex,
        ),
    ),
    manifest_row(
        "item/reasoning/summaryPartAdded",
        &["itemId", "summaryIndex", "threadId", "turnId"],
        &["itemId", "summaryIndex", "threadId", "turnId"],
        &[],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::CountByItemAndIndex,
        ),
    ),
    manifest_row(
        "item/reasoning/textDelta",
        &["contentIndex", "delta", "itemId", "threadId", "turnId"],
        &["contentIndex", "itemId", "threadId", "turnId"],
        &[],
        &["delta"],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::CountByItemAndIndex,
        ),
    ),
    manifest_row(
        "turn/moderationMetadata",
        &["metadata", "threadId", "turnId"],
        &["threadId", "turnId"],
        &[],
        &["metadata"],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::CountByTurn,
        ),
    ),
    manifest_row(
        "guardianWarning",
        &["message", "threadId"],
        &["threadId"],
        &["message"],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::Handled,
            ProgressiveActivityStability::Stable,
            ProgressiveActivityCoalescingMode::ReplaceByThread,
        ),
    ),
    manifest_row(
        "item/fileChange/outputDelta",
        &["delta", "itemId", "threadId", "turnId"],
        &["itemId", "threadId", "turnId"],
        &[],
        &["delta"],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::ExplicitlyIgnored,
            ProgressiveActivityStability::Deprecated,
            ProgressiveActivityCoalescingMode::None,
        ),
    ),
    manifest_row(
        "thread/compacted",
        &["threadId", "turnId"],
        &["threadId", "turnId"],
        &[],
        &[],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::ExplicitlyIgnored,
            ProgressiveActivityStability::Deprecated,
            ProgressiveActivityCoalescingMode::None,
        ),
    ),
    manifest_row(
        "model/safetyBuffering/updated",
        &[
            "model",
            "reasons",
            "showBufferingUi",
            "threadId",
            "turnId",
            "useCases",
        ],
        &["threadId", "turnId"],
        &[],
        &[
            "fasterModel",
            "model",
            "reasons",
            "showBufferingUi",
            "useCases",
        ],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::DiagnosticOnly,
            ProgressiveActivityStability::Diagnostic,
            ProgressiveActivityCoalescingMode::None,
        ),
    ),
    manifest_row(
        "model/verification",
        &["threadId", "turnId", "verifications"],
        &["threadId", "turnId"],
        &[],
        &["verifications"],
        &[],
        ProgressiveActivityManifestOptions::new(
            ProgressiveActivityDisposition::DiagnosticOnly,
            ProgressiveActivityStability::Diagnostic,
            ProgressiveActivityCoalescingMode::None,
        ),
    ),
];

const fn manifest_row(
    method: &'static str,
    required_fields: &'static [&'static str],
    preserved_fields: &'static [&'static str],
    bounded_redacted_fields: &'static [&'static str],
    ignored_fields: &'static [&'static str],
    compatibility_fields: &'static [&'static str],
    options: ProgressiveActivityManifestOptions,
) -> ProgressiveActivityManifestRow {
    ProgressiveActivityManifestRow {
        method,
        required_fields,
        preserved_fields,
        bounded_redacted_fields,
        ignored_fields,
        compatibility_fields,
        disposition: options.disposition,
        stability: options.stability,
        coalescing: options.coalescing,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityParseError {
    MissingField(&'static str),
    InvalidField(&'static str),
    InvalidObservation(ConversationProgressiveActivityRejection),
}

impl ProgressiveActivityParseError {
    pub(crate) const fn notice_label(&self) -> &'static str {
        match self {
            Self::MissingField(_) => "missing required progressive activity field",
            Self::InvalidField(_) => "invalid progressive activity field",
            Self::InvalidObservation(rejection) => rejection.notice_label(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedProgressiveActivity {
    pub(crate) batch: ConversationProgressiveActivityBatch,
    pub(crate) expected_item_kind: Option<ConversationItemKind>,
}

#[derive(Debug, Clone)]
pub(crate) enum ProgressiveActivityNotificationHandling {
    Activity(ParsedProgressiveActivity),
    ExplicitlyIgnored,
    DiagnosticOnly,
    NotOwned,
    Invalid(ProgressiveActivityParseError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityScope {
    Active,
    Stale,
    Malformed,
    NotOwned,
}

pub(crate) fn classify_progressive_activity_scope(
    method: &str,
    params: &Value,
    active_thread_id: &str,
    active_turn_id: &str,
) -> ProgressiveActivityScope {
    let Some(row) = PROGRESSIVE_ACTIVITY_MANIFEST
        .iter()
        .find(|row| row.method == method)
    else {
        return ProgressiveActivityScope::NotOwned;
    };
    let Ok(observed_thread_id) = required_identifier(params, "threadId") else {
        return ProgressiveActivityScope::Malformed;
    };
    if observed_thread_id != active_thread_id {
        return ProgressiveActivityScope::Stale;
    }
    let observed_turn_id = if row.required_fields.contains(&"turnId") {
        let Ok(observed_turn_id) = required_identifier(params, "turnId") else {
            return ProgressiveActivityScope::Malformed;
        };
        Some(observed_turn_id)
    } else {
        None
    };

    if observed_turn_id.is_none_or(|turn_id| turn_id == active_turn_id) {
        ProgressiveActivityScope::Active
    } else {
        ProgressiveActivityScope::Stale
    }
}

pub(crate) fn parse_progressive_activity_notification(
    method: &str,
    params: &Value,
    sequence: u64,
) -> ProgressiveActivityNotificationHandling {
    let Some(row) = PROGRESSIVE_ACTIVITY_MANIFEST
        .iter()
        .find(|row| row.method == method)
    else {
        return ProgressiveActivityNotificationHandling::NotOwned;
    };

    if let Err(error) = validate_classified_identity_fields(params, row) {
        return ProgressiveActivityNotificationHandling::Invalid(error);
    }
    if let Err(error) = validate_non_activity_fields(method, params, row.disposition) {
        return ProgressiveActivityNotificationHandling::Invalid(error);
    }
    match row.disposition {
        ProgressiveActivityDisposition::ExplicitlyIgnored => {
            ProgressiveActivityNotificationHandling::ExplicitlyIgnored
        }
        ProgressiveActivityDisposition::DiagnosticOnly => {
            ProgressiveActivityNotificationHandling::DiagnosticOnly
        }
        ProgressiveActivityDisposition::Handled => {
            match parse_handled(method, params, sequence).and_then(|parsed| {
                let batch = ConversationProgressiveActivityBatch::single(parsed.observation)
                    .map_err(ProgressiveActivityParseError::InvalidObservation)?;
                Ok(ParsedProgressiveActivity {
                    batch,
                    expected_item_kind: parsed.expected_item_kind,
                })
            }) {
                Ok(parsed) => ProgressiveActivityNotificationHandling::Activity(parsed),
                Err(error) => ProgressiveActivityNotificationHandling::Invalid(error),
            }
        }
    }
}

struct ParsedObservation {
    observation: ConversationProgressiveActivityObservation,
    expected_item_kind: Option<ConversationItemKind>,
}

fn parse_handled(
    method: &str,
    params: &Value,
    sequence: u64,
) -> Result<ParsedObservation, ProgressiveActivityParseError> {
    let thread_id = required_identifier(params, "threadId")?;
    let (turn_id, item_id, kind, payload, expected_item_kind) = match method {
        "item/agentMessage/delta" => {
            let delta = required_string(params, "delta")?;
            let (text, truncated_bytes) =
                bounded_progressive_prefix(delta, MAX_PROGRESSIVE_AGENT_DRAFT_BYTES);
            let phase = optional_identifier(params, "phase")?;
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::AgentMessageDelta,
                ConversationProgressiveActivityPayload::AgentMessageDelta {
                    phase,
                    text,
                    source_bytes: byte_count(delta),
                    truncated_bytes,
                },
                Some(ConversationItemKind::AgentMessage),
            )
        }
        "item/commandExecution/outputDelta" => {
            let delta = required_string(params, "delta")?;
            let (tail, truncated_bytes) =
                bounded_progressive_tail(delta, MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES);
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::CommandOutput,
                ConversationProgressiveActivityPayload::CommandOutput {
                    tail,
                    chunk_count: 1,
                    source_bytes: byte_count(delta),
                    newline_count: count_newlines(delta),
                    ends_with_newline: delta.ends_with('\n'),
                    truncated_bytes,
                },
                Some(ConversationItemKind::CommandExecution),
            )
        }
        "item/commandExecution/terminalInteraction" => {
            let stdin = required_string(params, "stdin")?;
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::TerminalInteraction,
                ConversationProgressiveActivityPayload::TerminalInteraction {
                    process_id: required_identifier(params, "processId")?,
                    interaction_count: 1,
                    input_bytes: byte_count(stdin),
                },
                Some(ConversationItemKind::CommandExecution),
            )
        }
        "item/fileChange/patchUpdated" => {
            let (changes, omitted_change_count, source_bytes, truncated_bytes) =
                parse_file_changes(params)?;
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::FileChangePatch,
                ConversationProgressiveActivityPayload::FileChangePatch {
                    changes,
                    omitted_change_count,
                    source_bytes,
                    truncated_bytes,
                },
                Some(ConversationItemKind::FileChange),
            )
        }
        "turn/diff/updated" => {
            let diff = required_string(params, "diff")?;
            let (detail, truncated_bytes) =
                bounded_progressive_prefix(diff, MAX_PROGRESSIVE_DIFF_DETAIL_BYTES);
            (
                Some(required_identifier(params, "turnId")?),
                None,
                ConversationProgressiveActivityKind::TurnDiff,
                ConversationProgressiveActivityPayload::TurnDiff {
                    detail,
                    source_bytes: byte_count(diff),
                    line_count: count_lines(diff),
                    addition_count: count_diff_lines(diff, '+', "+++"),
                    deletion_count: count_diff_lines(diff, '-', "---"),
                    hunk_count: saturating_usize_to_u64(
                        diff.lines().filter(|line| line.starts_with("@@")).count(),
                    ),
                    truncated_bytes,
                },
                None,
            )
        }
        "turn/plan/updated" => {
            let payload = parse_turn_plan(params)?;
            (
                Some(required_identifier(params, "turnId")?),
                None,
                ConversationProgressiveActivityKind::TurnPlan,
                payload,
                None,
            )
        }
        "thread/tokenUsage/updated" => (
            Some(required_identifier(params, "turnId")?),
            None,
            ConversationProgressiveActivityKind::TokenUsage,
            ConversationProgressiveActivityPayload::TokenUsage {
                usage: parse_token_usage(params)?,
            },
            None,
        ),
        "item/mcpToolCall/progress" => {
            let message = required_string(params, "message")?;
            let (message, truncated_bytes) =
                bounded_progressive_prefix(message, MAX_PROGRESSIVE_MCP_DETAIL_BYTES);
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::McpProgress,
                ConversationProgressiveActivityPayload::McpProgress {
                    message,
                    update_count: 1,
                    source_bytes: byte_count(required_string(params, "message")?),
                    truncated_bytes,
                },
                Some(ConversationItemKind::McpToolCall),
            )
        }
        "item/plan/delta" => {
            let delta = required_string(params, "delta")?;
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::PlanDelta,
                ConversationProgressiveActivityPayload::PlanDelta {
                    chunk_count: 1,
                    source_bytes: byte_count(delta),
                },
                Some(ConversationItemKind::Plan),
            )
        }
        "item/reasoning/summaryTextDelta" => {
            let delta = required_string(params, "delta")?;
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::ReasoningSummaryTextDelta,
                ConversationProgressiveActivityPayload::ReasoningSummaryTextDelta {
                    summary_index: required_i64(params, "summaryIndex")?,
                    chunk_count: 1,
                    source_bytes: byte_count(delta),
                },
                Some(ConversationItemKind::Reasoning),
            )
        }
        "item/reasoning/summaryPartAdded" => (
            Some(required_identifier(params, "turnId")?),
            Some(required_identifier(params, "itemId")?),
            ConversationProgressiveActivityKind::ReasoningSummaryPartAdded,
            ConversationProgressiveActivityPayload::ReasoningSummaryPartAdded {
                summary_index: required_i64(params, "summaryIndex")?,
                part_count: 1,
            },
            Some(ConversationItemKind::Reasoning),
        ),
        "item/reasoning/textDelta" => {
            let delta = required_string(params, "delta")?;
            (
                Some(required_identifier(params, "turnId")?),
                Some(required_identifier(params, "itemId")?),
                ConversationProgressiveActivityKind::ReasoningTextDelta,
                ConversationProgressiveActivityPayload::ReasoningTextDelta {
                    content_index: required_i64(params, "contentIndex")?,
                    chunk_count: 1,
                    source_bytes: byte_count(delta),
                },
                Some(ConversationItemKind::Reasoning),
            )
        }
        "turn/moderationMetadata" => {
            let metadata = required_value(params, "metadata")?;
            (
                Some(required_identifier(params, "turnId")?),
                None,
                ConversationProgressiveActivityKind::Moderation,
                ConversationProgressiveActivityPayload::Moderation {
                    update_count: 1,
                    metadata_bytes: capped_json_byte_count(metadata),
                },
                None,
            )
        }
        "guardianWarning" => {
            let message = required_string(params, "message")?;
            let (message, truncated_bytes) =
                bounded_progressive_prefix(message, MAX_PROGRESSIVE_GUARDIAN_WARNING_BYTES);
            (
                None,
                None,
                ConversationProgressiveActivityKind::GuardianWarning,
                ConversationProgressiveActivityPayload::GuardianWarning {
                    message,
                    update_count: 1,
                    source_bytes: byte_count(required_string(params, "message")?),
                    truncated_bytes,
                },
                None,
            )
        }
        _ => return Err(ProgressiveActivityParseError::InvalidField("method")),
    };

    Ok(ParsedObservation {
        observation: ConversationProgressiveActivityObservation {
            sequence,
            thread_id,
            turn_id,
            item_id,
            kind,
            payload,
        },
        expected_item_kind,
    })
}

fn validate_classified_identity_fields(
    params: &Value,
    row: &ProgressiveActivityManifestRow,
) -> Result<(), ProgressiveActivityParseError> {
    for field in ["threadId", "turnId", "itemId"] {
        if row.required_fields.contains(&field) {
            let _ = required_identifier(params, field)?;
        }
    }
    Ok(())
}

fn validate_non_activity_fields(
    method: &str,
    params: &Value,
    disposition: ProgressiveActivityDisposition,
) -> Result<(), ProgressiveActivityParseError> {
    if disposition == ProgressiveActivityDisposition::Handled {
        return Ok(());
    }
    // Deprecated and diagnostic-only notifications intentionally create no activity,
    // but still fail closed on wire-shape drift before their payloads are discarded.
    match method {
        "item/fileChange/outputDelta" => {
            let _ = required_string(params, "delta")?;
        }
        "thread/compacted" => {}
        "model/safetyBuffering/updated" => {
            let _ = required_string(params, "model")?;
            validate_optional_string(params, "fasterModel")?;
            validate_string_array(params, "reasons")?;
            validate_string_array(params, "useCases")?;
            let _ = required_boolean(params, "showBufferingUi")?;
        }
        "model/verification" => {
            // This surface stays diagnostic-only, but its schema enum is closed. A new
            // verification value must be reviewed before the notification is discarded.
            for verification in required_array(params, "verifications")? {
                if verification.as_str() != Some("trustedAccessForCyber") {
                    return Err(ProgressiveActivityParseError::InvalidField("verifications"));
                }
            }
        }
        _ => return Err(ProgressiveActivityParseError::InvalidField("method")),
    }
    Ok(())
}

fn parse_file_changes(
    params: &Value,
) -> Result<(Vec<ConversationProgressiveFileChange>, u64, u64, u64), ProgressiveActivityParseError>
{
    let source_changes = required_array(params, "changes")?;
    let mut changes = Vec::new();
    let mut omitted_change_count = 0u64;
    let mut source_bytes = 0u64;
    let mut truncated_bytes = 0u64;
    let mut remaining = MAX_PROGRESSIVE_DIFF_DETAIL_BYTES;

    for change in source_changes {
        let path = required_string(change, "path")?;
        let diff = required_string(change, "diff")?;
        let (kind, omitted_kind_bytes) = parse_file_change_kind(change)?;
        source_bytes = source_bytes
            .saturating_add(byte_count(path))
            .saturating_add(byte_count(diff))
            .saturating_add(omitted_kind_bytes);
        if changes.len() >= MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS || remaining == 0 {
            omitted_change_count = omitted_change_count.saturating_add(1);
            truncated_bytes = truncated_bytes
                .saturating_add(byte_count(path))
                .saturating_add(byte_count(diff))
                .saturating_add(omitted_kind_bytes);
            continue;
        }
        let path_limit = remaining.min(MAX_PROGRESSIVE_ACTIVITY_PATH_BYTES);
        let (retained_path, path_truncated) = bounded_progressive_prefix(path, path_limit);
        if retained_path.is_empty() {
            omitted_change_count = omitted_change_count.saturating_add(1);
            truncated_bytes = truncated_bytes
                .saturating_add(byte_count(path))
                .saturating_add(byte_count(diff))
                .saturating_add(omitted_kind_bytes);
            continue;
        }
        remaining = remaining.saturating_sub(retained_path.len());
        let (retained_diff, diff_truncated) = bounded_progressive_prefix(diff, remaining);
        remaining = remaining.saturating_sub(retained_diff.len());
        truncated_bytes = truncated_bytes
            .saturating_add(path_truncated)
            .saturating_add(diff_truncated)
            .saturating_add(omitted_kind_bytes);
        changes.push(ConversationProgressiveFileChange {
            path: retained_path,
            diff: retained_diff,
            kind,
        });
    }
    Ok((changes, omitted_change_count, source_bytes, truncated_bytes))
}

fn parse_file_change_kind(
    change: &Value,
) -> Result<(ConversationProgressiveFileChangeKind, u64), ProgressiveActivityParseError> {
    let kind = required_object(change, "kind")?;
    match required_string_from_object(kind, "type")? {
        "add" => Ok((ConversationProgressiveFileChangeKind::Add, 0)),
        "delete" => Ok((ConversationProgressiveFileChangeKind::Delete, 0)),
        "update" => {
            let move_path = match kind.get("move_path") {
                None | Some(Value::Null) => None,
                Some(Value::String(move_path)) => Some(move_path.as_str()),
                Some(_) => {
                    return Err(ProgressiveActivityParseError::InvalidField("move_path"));
                }
            };
            Ok((
                ConversationProgressiveFileChangeKind::Update {
                    move_path_present: move_path.is_some(),
                },
                move_path.map_or(0, byte_count),
            ))
        }
        _ => Err(ProgressiveActivityParseError::InvalidField("kind.type")),
    }
}

fn parse_turn_plan(
    params: &Value,
) -> Result<ConversationProgressiveActivityPayload, ProgressiveActivityParseError> {
    let source_steps = required_array(params, "plan")?;
    let source_explanation = optional_string(params, "explanation")?;
    let mut source_bytes = source_explanation.map_or(0, byte_count);
    let mut remaining = MAX_PROGRESSIVE_PLAN_DETAIL_BYTES;
    let (explanation, mut truncated_bytes) = match source_explanation {
        Some(explanation) => {
            let (retained, truncated) = bounded_progressive_prefix(explanation, remaining);
            remaining = remaining.saturating_sub(retained.len());
            (Some(retained), truncated)
        }
        None => (None, 0),
    };
    let mut steps = Vec::new();
    let mut omitted_step_count = 0u64;
    for source_step in source_steps {
        let text = required_string(source_step, "step")?;
        let status = parse_plan_status(required_string(source_step, "status")?)?;
        source_bytes = source_bytes.saturating_add(byte_count(text));
        if steps.len() >= MAX_PROGRESSIVE_PLAN_STEPS {
            omitted_step_count = omitted_step_count.saturating_add(1);
            truncated_bytes = truncated_bytes.saturating_add(byte_count(text));
            continue;
        }
        let limit = remaining.min(MAX_PROGRESSIVE_PLAN_STEP_BYTES);
        let (text, truncated) = bounded_progressive_prefix(text, limit);
        remaining = remaining.saturating_sub(text.len());
        truncated_bytes = truncated_bytes.saturating_add(truncated);
        steps.push(ConversationProgressivePlanStep { status, text });
    }
    Ok(ConversationProgressiveActivityPayload::TurnPlan {
        explanation,
        steps,
        omitted_step_count,
        source_bytes,
        truncated_bytes,
    })
}

fn parse_plan_status(
    status: &str,
) -> Result<ConversationProgressivePlanStepStatus, ProgressiveActivityParseError> {
    match status {
        "pending" => Ok(ConversationProgressivePlanStepStatus::Pending),
        "inProgress" => Ok(ConversationProgressivePlanStepStatus::InProgress),
        "completed" => Ok(ConversationProgressivePlanStepStatus::Completed),
        _ => Err(ProgressiveActivityParseError::InvalidField("plan.status")),
    }
}

fn parse_token_usage(
    params: &Value,
) -> Result<ConversationProgressiveTokenUsage, ProgressiveActivityParseError> {
    let token_usage = required_object(params, "tokenUsage")?;
    let model_context_window = match token_usage.get("modelContextWindow") {
        None | Some(Value::Null) => None,
        Some(value) => Some(nonzero_u64(value, "modelContextWindow")?),
    };
    Ok(ConversationProgressiveTokenUsage {
        last: parse_token_breakdown(token_usage, "last")?,
        total: parse_token_breakdown(token_usage, "total")?,
        model_context_window,
    })
}

fn parse_token_breakdown(
    token_usage: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<ConversationProgressiveTokenUsageBreakdown, ProgressiveActivityParseError> {
    let breakdown = token_usage
        .get(field)
        .ok_or(ProgressiveActivityParseError::MissingField(field))?
        .as_object()
        .ok_or(ProgressiveActivityParseError::InvalidField(field))?;
    Ok(ConversationProgressiveTokenUsageBreakdown {
        cached_input_tokens: required_u64_from_object(breakdown, "cachedInputTokens")?,
        input_tokens: required_u64_from_object(breakdown, "inputTokens")?,
        output_tokens: required_u64_from_object(breakdown, "outputTokens")?,
        reasoning_output_tokens: required_u64_from_object(breakdown, "reasoningOutputTokens")?,
        total_tokens: required_u64_from_object(breakdown, "totalTokens")?,
    })
}

fn required_value<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a Value, ProgressiveActivityParseError> {
    value
        .get(field)
        .ok_or(ProgressiveActivityParseError::MissingField(field))
}

fn required_string<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, ProgressiveActivityParseError> {
    required_value(value, field)?
        .as_str()
        .ok_or(ProgressiveActivityParseError::InvalidField(field))
}

fn optional_string<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<Option<&'a str>, ProgressiveActivityParseError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or(ProgressiveActivityParseError::InvalidField(field)),
    }
}

fn validate_optional_string(
    value: &Value,
    field: &'static str,
) -> Result<(), ProgressiveActivityParseError> {
    let _ = optional_string(value, field)?;
    Ok(())
}

fn validate_string_array(
    value: &Value,
    field: &'static str,
) -> Result<(), ProgressiveActivityParseError> {
    if required_array(value, field)?
        .iter()
        .any(|value| !value.is_string())
    {
        return Err(ProgressiveActivityParseError::InvalidField(field));
    }
    Ok(())
}

fn required_boolean(
    value: &Value,
    field: &'static str,
) -> Result<bool, ProgressiveActivityParseError> {
    required_value(value, field)?
        .as_bool()
        .ok_or(ProgressiveActivityParseError::InvalidField(field))
}

fn required_identifier(
    value: &Value,
    field: &'static str,
) -> Result<String, ProgressiveActivityParseError> {
    let identifier = required_string(value, field)?;
    validate_identifier(identifier, field)?;
    Ok(identifier.to_string())
}

fn optional_identifier(
    value: &Value,
    field: &'static str,
) -> Result<Option<String>, ProgressiveActivityParseError> {
    optional_string(value, field)?
        .map(|identifier| {
            validate_identifier(identifier, field)?;
            Ok(identifier.to_string())
        })
        .transpose()
}

fn validate_identifier(
    identifier: &str,
    field: &'static str,
) -> Result<(), ProgressiveActivityParseError> {
    if identifier.is_empty() || identifier.len() > MAX_PROGRESSIVE_ACTIVITY_IDENTIFIER_BYTES {
        return Err(ProgressiveActivityParseError::InvalidField(field));
    }
    Ok(())
}

fn required_i64(value: &Value, field: &'static str) -> Result<i64, ProgressiveActivityParseError> {
    required_value(value, field)?
        .as_i64()
        .ok_or(ProgressiveActivityParseError::InvalidField(field))
}

fn nonzero_u64(value: &Value, field: &'static str) -> Result<u64, ProgressiveActivityParseError> {
    let value = value
        .as_i64()
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(ProgressiveActivityParseError::InvalidField(field))?;
    if value == 0 {
        return Err(ProgressiveActivityParseError::InvalidField(field));
    }
    Ok(value)
}

fn required_u64_from_object(
    value: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<u64, ProgressiveActivityParseError> {
    value
        .get(field)
        .ok_or(ProgressiveActivityParseError::MissingField(field))?
        .as_i64()
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(ProgressiveActivityParseError::InvalidField(field))
}

fn required_array<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a [Value], ProgressiveActivityParseError> {
    required_value(value, field)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or(ProgressiveActivityParseError::InvalidField(field))
}

fn required_object<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a serde_json::Map<String, Value>, ProgressiveActivityParseError> {
    required_value(value, field)?
        .as_object()
        .ok_or(ProgressiveActivityParseError::InvalidField(field))
}

fn required_string_from_object<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, ProgressiveActivityParseError> {
    value
        .get(field)
        .ok_or(ProgressiveActivityParseError::MissingField(field))?
        .as_str()
        .ok_or(ProgressiveActivityParseError::InvalidField(field))
}

fn byte_count(value: &str) -> u64 {
    saturating_usize_to_u64(value.len())
}

pub(crate) fn capped_json_byte_count(value: &Value) -> u64 {
    // The transport has already bounded the parsed JSON line to 128 MiB. Count that
    // in-place once and saturate only at u64::MAX instead of allocating a second copy.
    let mut counter = SaturatingJsonByteCounter::default();
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => counter.written,
        Err(_) => u64::MAX,
    }
}

#[derive(Default)]
struct SaturatingJsonByteCounter {
    written: u64,
}

impl Write for SaturatingJsonByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.written = self
            .written
            .saturating_add(saturating_usize_to_u64(buffer.len()));
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn saturating_usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn count_lines(value: &str) -> u64 {
    saturating_usize_to_u64(value.lines().count())
}

fn count_newlines(value: &str) -> u64 {
    saturating_usize_to_u64(value.bytes().filter(|byte| *byte == b'\n').count())
}

fn count_diff_lines(value: &str, marker: char, excluded_prefix: &str) -> u64 {
    saturating_usize_to_u64(
        value
            .lines()
            .filter(|line| line.starts_with(marker) && !line.starts_with(excluded_prefix))
            .count(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{Value, json};

    use super::*;

    const FIXTURE: &str = include_str!("fixtures/progressive_turn_notifications.json");
    const SECRET_CANARY: &str = "AKRA_PROGRESSIVE_SECRET_CANARY";

    #[test]
    fn fixture_covers_every_owned_method_and_projects_fourteen_activities() {
        let notifications: Vec<Value> =
            serde_json::from_str(FIXTURE).expect("fixture should parse");
        let fixture_methods = notifications
            .iter()
            .map(|notification| {
                notification["method"]
                    .as_str()
                    .expect("fixture method should be a string")
                    .to_string()
            })
            .collect::<BTreeSet<_>>();
        let manifest_methods = PROGRESSIVE_ACTIVITY_MANIFEST
            .iter()
            .map(|row| row.method.to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(fixture_methods, manifest_methods);

        let mut handled = 0;
        let mut ignored = 0;
        let mut diagnostic = 0;
        for (index, notification) in notifications.iter().enumerate() {
            let method = notification["method"].as_str().unwrap();
            match parse_progressive_activity_notification(
                method,
                &notification["params"],
                index as u64 + 1,
            ) {
                ProgressiveActivityNotificationHandling::Activity(parsed) => {
                    handled += 1;
                    assert_eq!(parsed.batch.records().len(), 1);
                    assert_eq!(parsed.batch.first_sequence(), Some(index as u64 + 1));
                }
                ProgressiveActivityNotificationHandling::ExplicitlyIgnored => ignored += 1,
                ProgressiveActivityNotificationHandling::DiagnosticOnly => diagnostic += 1,
                other => panic!("fixture method {method} was not classified: {other:?}"),
            }
        }
        assert_eq!(handled, 14);
        assert_eq!(ignored, 2);
        assert_eq!(diagnostic, 2);
    }

    #[test]
    fn item_methods_report_the_lifecycle_kind_root_must_validate() {
        let cases = [
            (
                "item/agentMessage/delta",
                ConversationItemKind::AgentMessage,
            ),
            ("item/plan/delta", ConversationItemKind::Plan),
            (
                "item/commandExecution/outputDelta",
                ConversationItemKind::CommandExecution,
            ),
            (
                "item/commandExecution/terminalInteraction",
                ConversationItemKind::CommandExecution,
            ),
            (
                "item/fileChange/patchUpdated",
                ConversationItemKind::FileChange,
            ),
            (
                "item/mcpToolCall/progress",
                ConversationItemKind::McpToolCall,
            ),
            (
                "item/reasoning/summaryTextDelta",
                ConversationItemKind::Reasoning,
            ),
            (
                "item/reasoning/summaryPartAdded",
                ConversationItemKind::Reasoning,
            ),
            ("item/reasoning/textDelta", ConversationItemKind::Reasoning),
        ];
        let notifications: Vec<Value> = serde_json::from_str(FIXTURE).unwrap();
        for (method, expected) in cases {
            let notification = notifications
                .iter()
                .find(|notification| notification["method"] == method)
                .unwrap();
            let handling =
                parse_progressive_activity_notification(method, &notification["params"], 1);
            let ProgressiveActivityNotificationHandling::Activity(parsed) = handling else {
                panic!("{method} should produce progressive activity");
            };
            assert_eq!(
                parsed.expected_item_kind.as_ref(),
                Some(&expected),
                "{method}"
            );
        }
    }

    #[test]
    fn guardian_keeps_absent_turn_identity_and_global_methods_have_no_item_kind() {
        let params = json!({
            "threadId": "thread-1",
            "message": "bounded guardian message"
        });
        let ProgressiveActivityNotificationHandling::Activity(parsed) =
            parse_progressive_activity_notification("guardianWarning", &params, 9)
        else {
            panic!("guardian warning should parse");
        };
        let observation = parsed.batch.records()[0].observation();
        assert_eq!(observation.turn_id, None);
        assert_eq!(observation.item_id, None);
        assert_eq!(parsed.expected_item_kind, None);
    }

    #[test]
    fn scope_preflight_only_classifies_bounded_thread_and_turn_identity() {
        let active = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": [],
            "delta": 7
        });
        assert_eq!(
            classify_progressive_activity_scope(
                "item/agentMessage/delta",
                &active,
                "thread-1",
                "turn-1",
            ),
            ProgressiveActivityScope::Active
        );

        let mut stale = active.clone();
        stale["turnId"] = json!("turn-stale");
        assert_eq!(
            classify_progressive_activity_scope(
                "item/agentMessage/delta",
                &stale,
                "thread-1",
                "turn-1",
            ),
            ProgressiveActivityScope::Stale
        );

        let stale_thread_with_malformed_turn = json!({
            "threadId": "thread-stale",
            "turnId": [],
        });
        assert_eq!(
            classify_progressive_activity_scope(
                "item/agentMessage/delta",
                &stale_thread_with_malformed_turn,
                "thread-1",
                "turn-1",
            ),
            ProgressiveActivityScope::Stale
        );

        stale["turnId"] = json!("x".repeat(MAX_PROGRESSIVE_ACTIVITY_IDENTIFIER_BYTES + 1));
        assert_eq!(
            classify_progressive_activity_scope(
                "item/agentMessage/delta",
                &stale,
                "thread-1",
                "turn-1",
            ),
            ProgressiveActivityScope::Malformed
        );
        assert_eq!(
            classify_progressive_activity_scope(
                "item/future/outputDelta",
                &active,
                "thread-1",
                "turn-1",
            ),
            ProgressiveActivityScope::NotOwned
        );
    }

    #[test]
    fn raw_stdin_reasoning_plan_delta_and_moderation_are_discarded_with_counts() {
        let notifications: Vec<Value> = serde_json::from_str(FIXTURE).unwrap();
        for notification in notifications.iter().filter(|notification| {
            notification.to_string().contains(SECRET_CANARY)
                && matches!(
                    notification["method"].as_str(),
                    Some("item/commandExecution/terminalInteraction")
                        | Some("item/plan/delta")
                        | Some("item/reasoning/summaryTextDelta")
                        | Some("item/reasoning/textDelta")
                        | Some("turn/moderationMetadata")
                )
        }) {
            let method = notification["method"].as_str().unwrap();
            let ProgressiveActivityNotificationHandling::Activity(parsed) =
                parse_progressive_activity_notification(method, &notification["params"], 1)
            else {
                panic!("{method} should parse");
            };
            let observation = parsed.batch.records()[0].observation();
            assert!(observation.source_bytes() > 0);
            assert!(!format!("{observation:?}").contains(SECRET_CANARY));
            match &observation.payload {
                ConversationProgressiveActivityPayload::TerminalInteraction {
                    interaction_count,
                    input_bytes,
                    ..
                } => assert_eq!((*interaction_count, *input_bytes > 0), (1, true)),
                ConversationProgressiveActivityPayload::PlanDelta {
                    chunk_count,
                    source_bytes,
                }
                | ConversationProgressiveActivityPayload::ReasoningSummaryTextDelta {
                    chunk_count,
                    source_bytes,
                    ..
                }
                | ConversationProgressiveActivityPayload::ReasoningTextDelta {
                    chunk_count,
                    source_bytes,
                    ..
                } => assert_eq!((*chunk_count, *source_bytes > 0), (1, true)),
                ConversationProgressiveActivityPayload::Moderation {
                    update_count,
                    metadata_bytes,
                } => assert_eq!((*update_count, *metadata_bytes > 0), (1, true)),
                payload => panic!("unexpected secret-bearing fixture payload: {payload:?}"),
            }
        }
    }

    #[test]
    fn unicode_details_are_bounded_at_character_boundaries() {
        let delta = "한".repeat(MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES / 3 + 9);
        let params = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "command-1",
            "delta": delta,
        });
        let ProgressiveActivityNotificationHandling::Activity(parsed) =
            parse_progressive_activity_notification(
                "item/commandExecution/outputDelta",
                &params,
                1,
            )
        else {
            panic!("command delta should parse");
        };
        let observation = parsed.batch.records()[0].observation();
        let ConversationProgressiveActivityPayload::CommandOutput {
            tail,
            source_bytes,
            truncated_bytes,
            ..
        } = &observation.payload
        else {
            panic!("command payload should be retained");
        };
        assert!(tail.len() <= MAX_PROGRESSIVE_COMMAND_DETAIL_BYTES);
        assert!(tail.is_char_boundary(tail.len()));
        assert_eq!(*source_bytes, delta.len() as u64);
        assert_eq!(*truncated_bytes, (delta.len() - tail.len()) as u64);
    }

    #[test]
    fn turn_plan_preserves_step_status_after_the_text_budget_is_exhausted() {
        let explanation = "x".repeat(MAX_PROGRESSIVE_PLAN_DETAIL_BYTES);
        let params = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "explanation": explanation,
            "plan": [
                {"step": "first", "status": "pending"},
                {"step": "", "status": "completed"}
            ]
        });

        let ProgressiveActivityNotificationHandling::Activity(parsed) =
            parse_progressive_activity_notification("turn/plan/updated", &params, 1)
        else {
            panic!("turn plan should parse");
        };
        let observation = parsed.batch.records()[0].observation();
        let ConversationProgressiveActivityPayload::TurnPlan {
            steps,
            omitted_step_count,
            source_bytes,
            truncated_bytes,
            ..
        } = &observation.payload
        else {
            panic!("turn plan should retain typed plan state");
        };

        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[0].status,
            ConversationProgressivePlanStepStatus::Pending
        );
        assert_eq!(steps[0].text, "");
        assert_eq!(
            steps[1].status,
            ConversationProgressivePlanStepStatus::Completed
        );
        assert_eq!(steps[1].text, "");
        assert_eq!(*omitted_step_count, 0);
        assert_eq!(
            *source_bytes,
            (MAX_PROGRESSIVE_PLAN_DETAIL_BYTES + "first".len()) as u64
        );
        assert_eq!(*truncated_bytes, "first".len() as u64);
        assert_eq!(parsed.batch.payload_truncation_count(), 1);
        assert!(parsed.batch.history_incomplete());
    }

    #[test]
    fn all_reported_identities_must_be_nonempty_and_at_most_four_kib() {
        let base = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "agent-1",
            "delta": "text"
        });
        for field in ["threadId", "turnId", "itemId"] {
            for invalid in [
                String::new(),
                "x".repeat(MAX_PROGRESSIVE_ACTIVITY_IDENTIFIER_BYTES + 1),
            ] {
                let mut params = base.clone();
                params[field] = Value::String(invalid);
                let handling =
                    parse_progressive_activity_notification("item/agentMessage/delta", &params, 1);
                assert_invalid_field(handling, field);
            }
        }
    }

    #[test]
    fn malformed_types_missing_fields_and_unknown_plan_status_fail_closed() {
        let malformed = [
            (
                "item/commandExecution/outputDelta",
                json!({"threadId":"t","turnId":"u","itemId":"i","delta":7}),
            ),
            (
                "item/fileChange/patchUpdated",
                json!({"threadId":"t","turnId":"u","itemId":"i","changes":"patch"}),
            ),
            (
                "turn/plan/updated",
                json!({"threadId":"t","turnId":"u","plan":[{"step":"x","status":"future"}]}),
            ),
            (
                "item/reasoning/textDelta",
                json!({"threadId":"t","turnId":"u","itemId":"i","delta":"x"}),
            ),
        ];
        for (method, params) in malformed {
            let ProgressiveActivityNotificationHandling::Invalid(error) =
                parse_progressive_activity_notification(method, &params, 1)
            else {
                panic!("malformed {method} should fail");
            };
            assert!(!error.notice_label().contains(SECRET_CANARY));
        }
    }

    #[test]
    fn file_change_kind_is_validated_even_after_the_retention_budget_is_exhausted() {
        let mut changes = (0..MAX_RETAINED_PROGRESSIVE_ACTIVITY_RECORDS)
            .map(|index| {
                json!({
                    "path": format!("src/{index}.rs"),
                    "diff": "x",
                    "kind": {"type": "update", "move_path": null}
                })
            })
            .collect::<Vec<_>>();
        changes.push(json!({
            "path": "src/not-retained.rs",
            "diff": "not retained",
            "kind": {"type": "futureKind"}
        }));
        let params = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "file-1",
            "changes": changes
        });

        assert_invalid_field(
            parse_progressive_activity_notification("item/fileChange/patchUpdated", &params, 1),
            "kind.type",
        );
    }

    #[test]
    fn file_change_move_path_is_fully_accounted_as_unretained_detail() {
        let path = "src/current.rs";
        let diff = "@@ rename @@";
        let move_path = format!("renamed/{SECRET_CANARY}/{}", "한".repeat(8_192));
        let params = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "file-1",
            "changes": [{
                "path": path,
                "diff": diff,
                "kind": {"type": "update", "move_path": move_path}
            }]
        });

        let ProgressiveActivityNotificationHandling::Activity(parsed) =
            parse_progressive_activity_notification("item/fileChange/patchUpdated", &params, 1)
        else {
            panic!("valid rename patch should parse")
        };
        let observation = parsed.batch.records()[0].observation();
        let ConversationProgressiveActivityPayload::FileChangePatch {
            changes,
            source_bytes,
            truncated_bytes,
            ..
        } = &observation.payload
        else {
            panic!("rename patch should retain patch summary")
        };
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            changes[0].kind,
            ConversationProgressiveFileChangeKind::Update {
                move_path_present: true
            }
        ));
        assert_eq!(*truncated_bytes, move_path.len() as u64);
        assert_eq!(
            *source_bytes,
            (path.len() + diff.len() + move_path.len()) as u64
        );
        assert_eq!(parsed.batch.payload_truncation_count(), 1);
        assert!(parsed.batch.history_incomplete());
        assert!(!format!("{:?}", parsed.batch).contains(SECRET_CANARY));
    }

    #[test]
    fn moderation_json_byte_count_is_exact_without_a_second_json_allocation() {
        assert_eq!(capped_json_byte_count(&json!({"safe": true})), 13);

        let larger = json!({"metadata": "x".repeat(128)});
        assert_eq!(capped_json_byte_count(&larger), 143);
    }

    #[test]
    fn token_usage_rejects_negative_zero_context_overflow_and_inconsistent_totals() {
        let valid = json!({
            "threadId":"thread-1",
            "turnId":"turn-1",
            "tokenUsage":{
                "last": token_breakdown(),
                "total": token_breakdown(),
                "modelContextWindow": 1000
            }
        });
        let mutations = [
            ("last", "inputTokens", json!(-1)),
            ("last", "inputTokens", json!(u64::MAX)),
            ("total", "totalTokens", json!(1)),
            ("total", "totalTokens", json!(16)),
            ("last", "reasoningOutputTokens", json!(6)),
        ];
        for (section, field, invalid) in mutations {
            let mut params = valid.clone();
            params["tokenUsage"][section][field] = invalid;
            assert!(matches!(
                parse_progressive_activity_notification("thread/tokenUsage/updated", &params, 1),
                ProgressiveActivityNotificationHandling::Invalid(_)
            ));
        }
        let mut inconsistent_snapshot = valid.clone();
        inconsistent_snapshot["tokenUsage"]["total"] = json!({
            "cachedInputTokens": 1,
            "inputTokens": 9,
            "outputTokens": 6,
            "reasoningOutputTokens": 3,
            "totalTokens": 15
        });
        assert!(matches!(
            parse_progressive_activity_notification(
                "thread/tokenUsage/updated",
                &inconsistent_snapshot,
                1
            ),
            ProgressiveActivityNotificationHandling::Invalid(_)
        ));

        let mut zero_context = valid;
        zero_context["tokenUsage"]["modelContextWindow"] = json!(0);
        assert!(matches!(
            parse_progressive_activity_notification("thread/tokenUsage/updated", &zero_context, 1),
            ProgressiveActivityNotificationHandling::Invalid(_)
        ));
    }

    #[test]
    fn manifest_matches_checked_schema_required_and_source_fields_exactly() {
        let schema = schema_root();
        for row in PROGRESSIVE_ACTIVITY_MANIFEST {
            let params_schema = notification_params_schema(&schema, row.method);
            let schema_required = string_set(
                params_schema
                    .get("required")
                    .and_then(Value::as_array)
                    .expect("params schema should declare required fields"),
            );
            assert_eq!(
                row.required_fields.iter().copied().collect::<BTreeSet<_>>(),
                schema_required,
                "required field drift for {}",
                row.method
            );

            let schema_fields = params_schema["properties"]
                .as_object()
                .expect("params schema should declare properties")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let compatibility = row
                .compatibility_fields
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            let classified = row
                .preserved_fields
                .iter()
                .chain(row.bounded_redacted_fields)
                .chain(row.ignored_fields)
                .copied()
                .filter(|field| !compatibility.contains(field))
                .collect::<BTreeSet<_>>();
            assert_eq!(
                classified, schema_fields,
                "source field drift for {}",
                row.method
            );
            for extension in row.compatibility_fields {
                assert!(
                    !schema_fields.contains(extension),
                    "compatibility field became schema-owned"
                );
            }
        }
    }

    #[test]
    fn manifest_matches_checked_schema_refs_types_and_nested_shapes_exactly() {
        let schema = schema_root();
        assert_eq!(PROGRESSIVE_ACTIVITY_MANIFEST.len(), 18);
        for row in PROGRESSIVE_ACTIVITY_MANIFEST {
            let (expected_reference, expected_shape) = expected_schema_contract(row.method);
            assert_eq!(
                notification_params_reference(&schema, row.method),
                expected_reference,
                "params reference drift for {}",
                row.method
            );
            assert_eq!(
                relevant_schema_shape(&schema, notification_params_schema(&schema, row.method)),
                expected_shape,
                "nested schema shape drift for {}",
                row.method
            );
        }
    }

    #[test]
    fn manifest_method_sets_stability_and_plan_statuses_are_closed() {
        let handled = PROGRESSIVE_ACTIVITY_MANIFEST
            .iter()
            .filter(|row| row.disposition == ProgressiveActivityDisposition::Handled)
            .map(|row| row.method)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            handled,
            BTreeSet::from([
                "guardianWarning",
                "item/agentMessage/delta",
                "item/commandExecution/outputDelta",
                "item/commandExecution/terminalInteraction",
                "item/fileChange/patchUpdated",
                "item/mcpToolCall/progress",
                "item/plan/delta",
                "item/reasoning/summaryPartAdded",
                "item/reasoning/summaryTextDelta",
                "item/reasoning/textDelta",
                "thread/tokenUsage/updated",
                "turn/diff/updated",
                "turn/moderationMetadata",
                "turn/plan/updated",
            ])
        );
        let ignored = PROGRESSIVE_ACTIVITY_MANIFEST
            .iter()
            .filter(|row| row.disposition == ProgressiveActivityDisposition::ExplicitlyIgnored)
            .map(|row| row.method)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            ignored,
            BTreeSet::from(["item/fileChange/outputDelta", "thread/compacted"])
        );
        let diagnostic = PROGRESSIVE_ACTIVITY_MANIFEST
            .iter()
            .filter(|row| row.disposition == ProgressiveActivityDisposition::DiagnosticOnly)
            .map(|row| row.method)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            diagnostic,
            BTreeSet::from(["model/safetyBuffering/updated", "model/verification"])
        );
        assert!(
            PROGRESSIVE_ACTIVITY_MANIFEST
                .iter()
                .filter(|row| row.disposition == ProgressiveActivityDisposition::Handled)
                .all(|row| row.coalescing != ProgressiveActivityCoalescingMode::None)
        );
        assert!(
            !PROGRESSIVE_ACTIVITY_MANIFEST
                .iter()
                .any(|row| row.method == "model/rerouted")
        );

        let schema = schema_root();
        for method in ["item/fileChange/outputDelta", "thread/compacted"] {
            let row = manifest_row_for(method);
            assert_eq!(row.stability, ProgressiveActivityStability::Deprecated);
            let description = notification_params_schema(&schema, method)["description"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            assert!(description.contains("deprecated"), "{method}");
        }
        let plan_delta = manifest_row_for("item/plan/delta");
        assert_eq!(
            plan_delta.stability,
            ProgressiveActivityStability::Experimental
        );
        assert!(
            notification_params_schema(&schema, plan_delta.method)["description"]
                .as_str()
                .unwrap_or_default()
                .contains("EXPERIMENTAL")
        );

        let statuses = schema["definitions"]["TurnPlanStepStatus"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            statuses,
            BTreeSet::from(["completed", "inProgress", "pending"])
        );
    }

    #[test]
    fn deprecated_and_diagnostic_methods_validate_ids_but_never_create_activity() {
        for method in [
            "item/fileChange/outputDelta",
            "thread/compacted",
            "model/safetyBuffering/updated",
            "model/verification",
        ] {
            let row = manifest_row_for(method);
            let mut params = Value::Object(
                row.required_fields
                    .iter()
                    .map(|field| {
                        (
                            (*field).to_string(),
                            Value::String("placeholder".to_string()),
                        )
                    })
                    .collect(),
            );
            params["threadId"] = Value::String(String::new());
            assert!(matches!(
                parse_progressive_activity_notification(method, &params, 1),
                ProgressiveActivityNotificationHandling::Invalid(_)
            ));
        }
        assert!(matches!(
            parse_progressive_activity_notification("model/rerouted", &json!({}), 1),
            ProgressiveActivityNotificationHandling::NotOwned
        ));
    }

    #[test]
    fn deprecated_and_diagnostic_methods_validate_discarded_value_types() {
        let missing_cases = [
            (
                "item/fileChange/outputDelta",
                json!({"threadId":"t","turnId":"u","itemId":"i"}),
                "delta",
            ),
            (
                "model/safetyBuffering/updated",
                json!({
                    "threadId":"t","turnId":"u","reasons":["r"],
                    "showBufferingUi":true,"useCases":["u"]
                }),
                "model",
            ),
            (
                "model/verification",
                json!({"threadId":"t","turnId":"u"}),
                "verifications",
            ),
        ];
        for (method, params, field) in missing_cases {
            assert_missing_field(
                parse_progressive_activity_notification(method, &params, 1),
                field,
            );
        }

        let invalid_cases = [
            (
                "item/fileChange/outputDelta",
                json!({"threadId":"t","turnId":"u","itemId":"i","delta":7}),
                "delta",
            ),
            (
                "model/safetyBuffering/updated",
                json!({
                    "threadId":"t","turnId":"u","model":"m","reasons":["r"],
                    "showBufferingUi":"yes","useCases":["u"]
                }),
                "showBufferingUi",
            ),
            (
                "model/safetyBuffering/updated",
                json!({
                    "threadId":"t","turnId":"u","model":"m","reasons":[7],
                    "showBufferingUi":true,"useCases":["u"]
                }),
                "reasons",
            ),
            (
                "model/safetyBuffering/updated",
                json!({
                    "threadId":"t","turnId":"u","model":"m","fasterModel":7,
                    "reasons":["r"],"showBufferingUi":true,"useCases":["u"]
                }),
                "fasterModel",
            ),
            (
                "model/verification",
                json!({"threadId":"t","turnId":"u","verifications":"trustedAccessForCyber"}),
                "verifications",
            ),
            (
                "model/verification",
                json!({"threadId":"t","turnId":"u","verifications":["futureVerification"]}),
                "verifications",
            ),
        ];
        for (method, params, field) in invalid_cases {
            assert_invalid_field(
                parse_progressive_activity_notification(method, &params, 1),
                field,
            );
        }

        assert!(matches!(
            parse_progressive_activity_notification(
                "model/safetyBuffering/updated",
                &json!({
                    "threadId":"t","turnId":"u","model":"m","fasterModel":null,
                    "reasons":["r"],"showBufferingUi":true,"useCases":["u"]
                }),
                1,
            ),
            ProgressiveActivityNotificationHandling::DiagnosticOnly
        ));
        assert!(matches!(
            parse_progressive_activity_notification(
                "model/verification",
                &json!({
                    "threadId":"t","turnId":"u",
                    "verifications":["trustedAccessForCyber"]
                }),
                1,
            ),
            ProgressiveActivityNotificationHandling::DiagnosticOnly
        ));
    }

    fn token_breakdown() -> Value {
        json!({
            "cachedInputTokens": 2,
            "inputTokens": 10,
            "outputTokens": 5,
            "reasoningOutputTokens": 3,
            "totalTokens": 15
        })
    }

    fn assert_invalid_field(
        handling: ProgressiveActivityNotificationHandling,
        field: &'static str,
    ) {
        let ProgressiveActivityNotificationHandling::Invalid(
            ProgressiveActivityParseError::InvalidField(actual),
        ) = handling
        else {
            panic!("{field} should be rejected as invalid");
        };
        assert_eq!(actual, field);
    }

    fn assert_missing_field(
        handling: ProgressiveActivityNotificationHandling,
        field: &'static str,
    ) {
        let ProgressiveActivityNotificationHandling::Invalid(
            ProgressiveActivityParseError::MissingField(actual),
        ) = handling
        else {
            panic!("{field} should be rejected as missing");
        };
        assert_eq!(actual, field);
    }

    fn manifest_row_for(method: &str) -> &'static ProgressiveActivityManifestRow {
        PROGRESSIVE_ACTIVITY_MANIFEST
            .iter()
            .find(|row| row.method == method)
            .unwrap()
    }

    fn schema_root() -> Value {
        serde_json::from_str(include_str!(
            "../../../../../schema/codex_app_server_protocol.v2.schemas.json"
        ))
        .expect("checked schema should parse")
    }

    fn notification_params_schema<'a>(schema: &'a Value, method: &str) -> &'a Value {
        resolve_reference(schema, notification_params_reference(schema, method))
    }

    fn notification_params_reference<'a>(schema: &'a Value, method: &str) -> &'a str {
        let notification = schema["definitions"]["ServerNotification"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|notification| {
                notification["properties"]["method"]["enum"]
                    .as_array()
                    .is_some_and(|methods| methods.iter().any(|value| value == method))
            })
            .unwrap_or_else(|| panic!("schema notification missing {method}"));
        notification["properties"]["params"]["$ref"]
            .as_str()
            .expect("params should use a schema reference")
    }

    fn resolve_reference<'a>(schema: &'a Value, reference: &str) -> &'a Value {
        reference
            .strip_prefix("#/")
            .expect("local schema reference")
            .split('/')
            .fold(schema, |node, segment| &node[segment])
    }

    fn string_set(values: &[Value]) -> BTreeSet<&str> {
        values.iter().map(|value| value.as_str().unwrap()).collect()
    }

    fn relevant_schema_shape(schema: &Value, node: &Value) -> String {
        if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            let label = reference.rsplit('/').next().unwrap();
            return format!(
                "ref({label})->{}",
                relevant_schema_shape(schema, resolve_reference(schema, reference))
            );
        }
        if let Some(alternatives) = node.get("oneOf").and_then(Value::as_array) {
            return format!(
                "oneOf[{}]",
                alternatives
                    .iter()
                    .map(|alternative| relevant_schema_shape(schema, alternative))
                    .collect::<Vec<_>>()
                    .join("|")
            );
        }
        if node == &Value::Bool(true) {
            return "any".to_string();
        }
        if node == &Value::Bool(false) {
            return "never".to_string();
        }

        let mut shape = match node.get("type") {
            Some(Value::String(kind)) if kind == "object" => {
                let required = node
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|values| string_set(values))
                    .unwrap_or_default();
                let fields = node
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("object schema should expose its fields");
                let mut names = fields.keys().map(String::as_str).collect::<Vec<_>>();
                names.sort_unstable();
                format!(
                    "object{{{}}}",
                    names
                        .into_iter()
                        .map(|name| {
                            let required_marker = if required.contains(name) { '!' } else { '?' };
                            format!(
                                "{name}{required_marker}:{}",
                                relevant_schema_shape(schema, &fields[name])
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Some(Value::String(kind)) if kind == "array" => format!(
                "array<{}>",
                relevant_schema_shape(
                    schema,
                    node.get("items")
                        .expect("array schema should expose item shape")
                )
            ),
            Some(Value::String(kind)) => kind.clone(),
            Some(Value::Array(kinds)) => {
                let mut kinds = kinds
                    .iter()
                    .map(|kind| kind.as_str().expect("schema type should be a string"))
                    .collect::<Vec<_>>();
                kinds.sort_unstable();
                format!("union[{}]", kinds.join("|"))
            }
            other => panic!("unsupported schema type shape: {other:?}"),
        };
        if let Some(format) = node.get("format").and_then(Value::as_str) {
            shape.push('<');
            shape.push_str(format);
            shape.push('>');
        }
        if let Some(values) = node.get("enum").and_then(Value::as_array) {
            shape.push('[');
            shape.push_str(
                &values
                    .iter()
                    .map(|value| value.as_str().expect("schema enum should be a string"))
                    .collect::<Vec<_>>()
                    .join("|"),
            );
            shape.push(']');
        }
        shape
    }

    fn expected_schema_contract(method: &str) -> (&'static str, &'static str) {
        const DELTA: &str = "object{delta!:string,itemId!:string,threadId!:string,turnId!:string}";
        const TERMINAL: &str = "object{itemId!:string,processId!:string,stdin!:string,threadId!:string,turnId!:string}";
        const FILE_PATCH: &str = "object{changes!:array<ref(FileUpdateChange)->object{diff!:string,kind!:ref(PatchChangeKind)->oneOf[object{type!:string[add]}|object{type!:string[delete]}|object{move_path?:union[null|string],type!:string[update]}],path!:string}>,itemId!:string,threadId!:string,turnId!:string}";
        const TURN_DIFF: &str = "object{diff!:string,threadId!:string,turnId!:string}";
        const TURN_PLAN: &str = "object{explanation?:union[null|string],plan!:array<ref(TurnPlanStep)->object{status!:ref(TurnPlanStepStatus)->string[pending|inProgress|completed],step!:string}>,threadId!:string,turnId!:string}";
        const TOKEN_USAGE: &str = "object{threadId!:string,tokenUsage!:ref(ThreadTokenUsage)->object{last!:ref(TokenUsageBreakdown)->object{cachedInputTokens!:integer<int64>,inputTokens!:integer<int64>,outputTokens!:integer<int64>,reasoningOutputTokens!:integer<int64>,totalTokens!:integer<int64>},modelContextWindow?:union[integer|null]<int64>,total!:ref(TokenUsageBreakdown)->object{cachedInputTokens!:integer<int64>,inputTokens!:integer<int64>,outputTokens!:integer<int64>,reasoningOutputTokens!:integer<int64>,totalTokens!:integer<int64>}},turnId!:string}";
        const MCP: &str = "object{itemId!:string,message!:string,threadId!:string,turnId!:string}";
        const REASONING_SUMMARY_TEXT: &str = "object{delta!:string,itemId!:string,summaryIndex!:integer<int64>,threadId!:string,turnId!:string}";
        const REASONING_SUMMARY_PART: &str =
            "object{itemId!:string,summaryIndex!:integer<int64>,threadId!:string,turnId!:string}";
        const REASONING_TEXT: &str = "object{contentIndex!:integer<int64>,delta!:string,itemId!:string,threadId!:string,turnId!:string}";
        const MODERATION: &str = "object{metadata!:any,threadId!:string,turnId!:string}";
        const GUARDIAN: &str = "object{message!:string,threadId!:string}";
        const COMPACTED: &str = "object{threadId!:string,turnId!:string}";
        const SAFETY: &str = "object{fasterModel?:union[null|string],model!:string,reasons!:array<string>,showBufferingUi!:boolean,threadId!:string,turnId!:string,useCases!:array<string>}";
        const VERIFICATION: &str = "object{threadId!:string,turnId!:string,verifications!:array<ref(ModelVerification)->string[trustedAccessForCyber]>}";

        match method {
            "item/agentMessage/delta" => ("#/definitions/AgentMessageDeltaNotification", DELTA),
            "item/commandExecution/outputDelta" => (
                "#/definitions/CommandExecutionOutputDeltaNotification",
                DELTA,
            ),
            "item/commandExecution/terminalInteraction" => {
                ("#/definitions/TerminalInteractionNotification", TERMINAL)
            }
            "item/fileChange/patchUpdated" => (
                "#/definitions/FileChangePatchUpdatedNotification",
                FILE_PATCH,
            ),
            "turn/diff/updated" => ("#/definitions/TurnDiffUpdatedNotification", TURN_DIFF),
            "turn/plan/updated" => ("#/definitions/TurnPlanUpdatedNotification", TURN_PLAN),
            "thread/tokenUsage/updated" => (
                "#/definitions/ThreadTokenUsageUpdatedNotification",
                TOKEN_USAGE,
            ),
            "item/mcpToolCall/progress" => ("#/definitions/McpToolCallProgressNotification", MCP),
            "item/plan/delta" => ("#/definitions/PlanDeltaNotification", DELTA),
            "item/reasoning/summaryTextDelta" => (
                "#/definitions/ReasoningSummaryTextDeltaNotification",
                REASONING_SUMMARY_TEXT,
            ),
            "item/reasoning/summaryPartAdded" => (
                "#/definitions/ReasoningSummaryPartAddedNotification",
                REASONING_SUMMARY_PART,
            ),
            "item/reasoning/textDelta" => (
                "#/definitions/ReasoningTextDeltaNotification",
                REASONING_TEXT,
            ),
            "turn/moderationMetadata" => (
                "#/definitions/TurnModerationMetadataNotification",
                MODERATION,
            ),
            "guardianWarning" => ("#/definitions/GuardianWarningNotification", GUARDIAN),
            "item/fileChange/outputDelta" => {
                ("#/definitions/FileChangeOutputDeltaNotification", DELTA)
            }
            "thread/compacted" => ("#/definitions/ContextCompactedNotification", COMPACTED),
            "model/safetyBuffering/updated" => (
                "#/definitions/ModelSafetyBufferingUpdatedNotification",
                SAFETY,
            ),
            "model/verification" => ("#/definitions/ModelVerificationNotification", VERIFICATION),
            _ => panic!("missing schema contract for {method}"),
        }
    }
}
