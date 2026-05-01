use serde::{Deserialize, Serialize};
use serde_json::Value;

mod turn_notifications;

use self::turn_notifications::to_conversation_message;
pub(super) use self::turn_notifications::{
    AppServerNotification, TurnNotificationHandling, handle_turn_notification,
};
use crate::domain::conversation::ConversationSnapshot;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnInterruptParams {
    pub(super) thread_id: String,
    pub(super) turn_id: String,
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
pub(super) struct TurnInterruptResponse {}

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
