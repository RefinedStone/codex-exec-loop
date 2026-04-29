use std::sync::mpsc::Sender;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::canonical_active_planning_file_path;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessage,
    ConversationMessageKind, ConversationSnapshot, ConversationToolActivity,
    ConversationToolActivityKind,
};
use crate::domain::session_summary::SessionSummary;

pub(super) const SHARED_RUNTIME_NOTICE_PREFIX: &str = "shared runtime ";
pub(super) const ACTIVE_STREAM_ISOLATION_NOTICE_FRAGMENT: &str =
    "app-server connection while a turn stream was active";

pub(super) fn initialize_detail(initialize_response: &InitializeResponse) -> String {
    format!(
        "{} / {} / {}",
        initialize_response.platform_os,
        initialize_response.platform_family,
        initialize_response.user_agent,
    )
}

pub(super) fn to_session_summary(thread_record: ThreadRecord) -> SessionSummary {
    SessionSummary {
        id: thread_record.id,
        name: thread_record.name,
        preview: thread_record.preview,
        cwd: thread_record.cwd,
        source: thread_record.source,
        model_provider: thread_record.model_provider,
        updated_at_epoch: thread_record.updated_at,
        status_type: thread_record.status.status_type,
        path: thread_record.path,
        git_branch: thread_record.git_info.and_then(|git_info| git_info.branch),
    }
}

pub(super) fn to_conversation_snapshot(
    thread_record: ThreadRecord,
    warnings: Vec<String>,
) -> ConversationSnapshot {
    let (warnings, runtime_notices) = partition_runtime_notices(warnings);
    let title = thread_title(&thread_record);

    let messages = thread_record
        .turns
        .into_iter()
        .flat_map(|turn| turn.items.into_iter())
        .filter_map(to_conversation_message)
        .collect::<Vec<_>>();

    ConversationSnapshot {
        thread_id: thread_record.id,
        title,
        cwd: thread_record.cwd,
        messages,
        warnings,
        runtime_notices,
    }
}

pub(super) fn thread_title(thread_record: &ThreadRecord) -> String {
    thread_record
        .name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            thread_record
                .preview
                .lines()
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Untitled thread")
                .to_string()
        })
}

pub(super) fn sort_and_dedup_warnings(warnings: &mut Vec<String>) {
    warnings.sort();
    warnings.dedup();
}

pub(super) fn partition_runtime_notices(warnings: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut conversation_warnings = Vec::new();
    let mut runtime_notices = Vec::new();

    for warning in warnings {
        if is_runtime_notice(&warning) {
            runtime_notices.push(warning);
        } else {
            conversation_warnings.push(warning);
        }
    }

    (conversation_warnings, runtime_notices)
}

pub(super) fn is_runtime_notice(warning: &str) -> bool {
    warning.starts_with(SHARED_RUNTIME_NOTICE_PREFIX)
        || warning.contains(ACTIVE_STREAM_ISOLATION_NOTICE_FRAGMENT)
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AppServerNotification {
    method: String,
    params: Value,
}

impl AppServerNotification {
    pub(super) fn from_value(value: Value) -> Option<Self> {
        let method = value.get("method").and_then(Value::as_str)?.to_string();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        Some(Self { method, params })
    }

    pub(super) fn method(&self) -> &str {
        &self.method
    }

    pub(super) fn params(&self) -> &Value {
        &self.params
    }

    pub(super) fn should_defer_to_turn_stream(&self) -> bool {
        self.method == "error"
            || self.method == "thread/status/changed"
            || self.method.starts_with("turn/")
            || self.method.starts_with("item/")
    }

    pub(super) fn warning_text(&self, context: &str) -> String {
        match self.method.as_str() {
            "configWarning" => self
                .params
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("app-server sent a config warning {context}")),
            "error" => format!(
                "app-server reported an error {context}: {}",
                self.params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            ),
            _ => format!("app-server sent notification `{}` {context}", self.method),
        }
    }
}

pub(super) enum TurnNotificationHandling {
    Consumed,
    Completed,
    Dropped(String),
}

pub(super) fn handle_turn_notification(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
    changed_planning_file_paths: &mut Vec<String>,
    event_sender: &Sender<ConversationStreamEvent>,
) -> Result<TurnNotificationHandling> {
    let params = notification.params();

    match notification.method() {
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
            if !matches_active_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            if let Some(event) = parse_approval_review_event(notification, thread_id, turn_id) {
                let _ = event_sender.send(event);
                return Ok(TurnNotificationHandling::Consumed);
            }

            Ok(TurnNotificationHandling::Dropped(
                notification.warning_text(
                    "with an approval review payload the adapter could not translate",
                ),
            ))
        }
        "thread/status/changed" => {
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let status = params
                .get("status")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let _ = event_sender.send(ConversationStreamEvent::StatusUpdated {
                text: format!("thread status: {status}"),
            });
            Ok(TurnNotificationHandling::Consumed)
        }
        "turn/started" => {
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let started_turn_id = params
                .get("turn")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
                .unwrap_or(turn_id);
            let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                turn_id: started_turn_id.to_string(),
            });
            Ok(TurnNotificationHandling::Consumed)
        }
        "item/agentMessage/delta" => {
            if params.get("turnId").and_then(Value::as_str) != Some(turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let item_id = params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let delta = params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let _ = event_sender.send(ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase: params
                    .get("phase")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                delta,
            });
            Ok(TurnNotificationHandling::Consumed)
        }
        "item/completed" => {
            if params.get("turnId").and_then(Value::as_str) != Some(turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            record_changed_planning_file_paths(params.get("item"), changed_planning_file_paths);
            handle_completed_item(params.get("item"), event_sender);
            Ok(TurnNotificationHandling::Consumed)
        }
        "error" => {
            let message = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("app-server reported an error");
            bail!(message.to_string());
        }
        "turn/completed" => {
            let completed_thread_id = params.get("threadId").and_then(Value::as_str);
            let completed_turn_id = params
                .get("turn")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str);

            if completed_thread_id != Some(thread_id) || completed_turn_id != Some(turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let _ = event_sender.send(ConversationStreamEvent::TurnCompleted {
                turn_id: turn_id.to_string(),
                changed_planning_file_paths: changed_planning_file_paths.clone(),
            });
            Ok(TurnNotificationHandling::Completed)
        }
        _ => Ok(TurnNotificationHandling::Dropped(
            notification.warning_text("that has no adapter translation for the active turn stream"),
        )),
    }
}

pub(super) fn changed_planning_file_paths(item: &Value) -> Vec<String> {
    let mut paths = Vec::new();

    for path in changed_file_paths(item) {
        if let Some(canonical_path) = canonical_active_planning_file_path(&path) {
            push_unique_path(&mut paths, canonical_path.to_string());
        }
    }

    paths
}

pub(super) fn format_file_change_summary(item: &Value) -> String {
    let changes = item
        .get("changes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if changes.is_empty() {
        return "file change completed".to_string();
    }

    let entries = changes
        .iter()
        .map(|change| {
            let path = change
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("unknown-path");
            let kind = change
                .get("kind")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("update");
            format!("{kind} {path}")
        })
        .collect::<Vec<_>>();

    format!("file change: {}", entries.join(", "))
}

pub(super) fn count_file_changes(item: &Value) -> usize {
    item.get("changes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

pub(super) fn format_command_execution_summary(item: &Value) -> String {
    let command = item
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("command");
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    format!("command: {command} [{status}]")
}

fn to_conversation_message(item: Value) -> Option<ConversationMessage> {
    let item_type = item.get("type")?.as_str()?;
    match item_type {
        "userMessage" => {
            let text = item
                .get("content")
                .and_then(Value::as_array)
                .map(|content| extract_user_input_text(content.as_slice()))
                .filter(|value| !value.trim().is_empty())?;

            Some(ConversationMessage::new(
                ConversationMessageKind::User,
                text,
                None,
                item.get("id").and_then(Value::as_str).map(str::to_string),
            ))
        }
        "agentMessage" => Some(ConversationMessage::new(
            ConversationMessageKind::Agent,
            item.get("text").and_then(Value::as_str).unwrap_or_default(),
            item.get("phase")
                .and_then(Value::as_str)
                .map(str::to_string),
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        "fileChange" => Some(ConversationMessage::new(
            ConversationMessageKind::Tool,
            format_file_change_summary(&item),
            None,
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        "commandExecution" => Some(ConversationMessage::new(
            ConversationMessageKind::Tool,
            format_command_execution_summary(&item),
            None,
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        _ => None,
    }
}

fn extract_user_input_text(items: &[Value]) -> String {
    items
        .iter()
        .filter_map(|content| {
            if content.get("type").and_then(Value::as_str) == Some("text") {
                content.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn changed_file_paths(item: &Value) -> Vec<String> {
    let mut paths = Vec::new();

    for change in item
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(path) = change.get("path").and_then(Value::as_str) {
            paths.push(path.to_string());
        }
        if let Some(move_path) = change
            .get("kind")
            .and_then(|kind| kind.get("move_path"))
            .and_then(Value::as_str)
        {
            paths.push(move_path.to_string());
        }
    }

    paths
}

fn push_unique_path(target: &mut Vec<String>, path: String) {
    if !target.iter().any(|existing| existing == &path) {
        target.push(path);
    }
}

fn parse_approval_review_event(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
) -> Option<ConversationStreamEvent> {
    if !matches!(
        notification.method(),
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed"
    ) {
        return None;
    }

    let params = notification.params();
    if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
        || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
    {
        return None;
    }

    let review = params.get("review")?;
    let status = review
        .get("status")
        .and_then(Value::as_str)
        .map(parse_approval_review_status)?;
    let target_item_id = params.get("targetItemId").and_then(Value::as_str)?;

    Some(ConversationStreamEvent::ApprovalReviewUpdated {
        review: ConversationApprovalReview {
            target_item_id: target_item_id.to_string(),
            status,
            risk_level: review
                .get("riskLevel")
                .and_then(Value::as_str)
                .map(str::to_string),
            rationale: review
                .get("rationale")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
    })
}

fn parse_approval_review_status(value: &str) -> ConversationApprovalReviewStatus {
    match value {
        "inProgress" => ConversationApprovalReviewStatus::InProgress,
        "approved" => ConversationApprovalReviewStatus::Approved,
        "denied" => ConversationApprovalReviewStatus::Denied,
        "aborted" => ConversationApprovalReviewStatus::Aborted,
        other => ConversationApprovalReviewStatus::Unknown(other.to_string()),
    }
}

fn handle_completed_item(item: Option<&Value>, event_sender: &Sender<ConversationStreamEvent>) {
    let Some(item) = item else {
        return;
    };

    let item_type = item.get("type").and_then(Value::as_str);
    match item_type {
        Some("agentMessage") => {
            let item_id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let phase = item
                .get("phase")
                .and_then(Value::as_str)
                .map(str::to_string);
            let _ = event_sender.send(ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            });
        }
        Some("fileChange") => {
            let _ = event_sender.send(ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::FileChange,
                    text: format_file_change_summary(item),
                    file_change_count: count_file_changes(item),
                },
            });
        }
        Some("commandExecution") => {
            let _ = event_sender.send(ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::CommandExecution,
                    text: format_command_execution_summary(item),
                    file_change_count: 0,
                },
            });
        }
        _ => {}
    }
}

fn record_changed_planning_file_paths(
    item: Option<&Value>,
    changed_file_paths_for_turn: &mut Vec<String>,
) {
    let Some(item) = item else {
        return;
    };
    if item.get("type").and_then(Value::as_str) != Some("fileChange") {
        return;
    }

    for path in changed_planning_file_paths(item) {
        push_unique_path(changed_file_paths_for_turn, path);
    }
}

fn matches_active_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    params.get("threadId").and_then(Value::as_str) == Some(thread_id)
        && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeResponse {
    pub(super) user_agent: String,
    pub(super) platform_family: String,
    pub(super) platform_os: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AccountReadResponse {
    account: Option<AccountRecord>,
    requires_openai_auth: Option<bool>,
}

impl AccountReadResponse {
    pub(super) fn is_authenticated(&self) -> bool {
        self.account.is_some() || !self.requires_openai_auth.unwrap_or(false)
    }

    pub(super) fn to_summary_text(&self) -> String {
        match &self.account {
            Some(account) if account.account_type == "chatgpt" => format!(
                "chatgpt / {} / {}",
                account.email.as_deref().unwrap_or("unknown-email"),
                account.plan_type.as_deref().unwrap_or("unknown-plan"),
            ),
            Some(account) if account.account_type == "apiKey" => "api key account".to_string(),
            Some(account) => format!("account type: {}", account.account_type),
            None if self.requires_openai_auth.unwrap_or(false) => {
                "not logged in (OpenAI auth required)".to_string()
            }
            None => "no account configured".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountRecord {
    #[serde(rename = "type")]
    account_type: String,
    email: Option<String>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) search_term: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_kinds: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum ApprovalPolicyValue {
    #[serde(rename = "untrusted")]
    Untrusted,
    #[serde(rename = "on-failure")]
    OnFailure,
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "never")]
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum ApprovalsReviewerValue {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "guardian_subagent")]
    GuardianSubagent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum SandboxModeValue {
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "workspace-write")]
    WorkspaceWrite,
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

impl SandboxModeValue {
    pub(super) fn as_turn_sandbox_policy(self) -> SandboxPolicyValue {
        match self {
            Self::ReadOnly => SandboxPolicyValue::ReadOnly,
            Self::WorkspaceWrite => SandboxPolicyValue::WorkspaceWrite,
            Self::DangerFullAccess => SandboxPolicyValue::DangerFullAccess,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub(super) enum SandboxPolicyValue {
    #[serde(rename = "readOnly")]
    ReadOnly,
    #[serde(rename = "workspaceWrite")]
    WorkspaceWrite,
    #[serde(rename = "dangerFullAccess")]
    DangerFullAccess,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum ReasoningEffortValue {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "minimal")]
    Minimal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approval_policy: Option<ApprovalPolicyValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approvals_reviewer: Option<ApprovalsReviewerValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sandbox: Option<SandboxModeValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadResumeParams {
    pub(super) thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approval_policy: Option<ApprovalPolicyValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approvals_reviewer: Option<ApprovalsReviewerValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sandbox: Option<SandboxModeValue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnStartParams {
    pub(super) thread_id: String,
    pub(super) input: Vec<TurnInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approval_policy: Option<ApprovalPolicyValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approvals_reviewer: Option<ApprovalsReviewerValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sandbox_policy: Option<SandboxPolicyValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<ReasoningEffortValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub(super) enum TurnInputItem {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "skill")]
    Skill { name: String, path: String },
}

impl TurnInputItem {
    pub(super) fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub(super) fn skill(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Skill {
            name: name.into(),
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadListResponse {
    pub(super) data: Vec<ThreadRecord>,
    pub(super) next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadReadResponse {
    pub(super) thread: ThreadRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadStartResponse {
    pub(super) thread: ThreadRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadResumeResponse {
    #[serde(rename = "thread")]
    pub(super) _thread: ThreadRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TurnStartResponse {
    pub(super) turn: TurnRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TurnRecord {
    pub(super) id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadRecord {
    pub(super) id: String,
    pub(super) name: Option<String>,
    pub(super) preview: String,
    pub(super) cwd: String,
    pub(super) source: String,
    pub(super) model_provider: String,
    pub(super) updated_at: i64,
    pub(super) path: String,
    pub(super) status: ThreadStatus,
    pub(super) git_info: Option<ThreadGitInfo>,
    #[serde(default)]
    pub(super) turns: Vec<ThreadTurnRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadTurnRecord {
    #[serde(default)]
    items: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadStatus {
    #[serde(rename = "type")]
    pub(super) status_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadGitInfo {
    branch: Option<String>,
}
