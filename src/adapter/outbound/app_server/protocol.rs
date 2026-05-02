use serde::{Deserialize, Serialize};
use serde_json::Value;

mod turn_notifications;

use self::turn_notifications::to_conversation_message;
pub(super) use self::turn_notifications::{
    AppServerNotification, TurnNotificationHandling, handle_turn_notification,
};
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::session_summary::SessionSummary;

/*
 * protocol.rs는 app-server JSON payload와 Akra domain projection 사이의 translation layer다.
 * connection.rs는 serde value를 주고받고, mod.rs/runtime.rs는 request 흐름을 조립하며, 이 파일은
 * wire field 이름과 domain field 이름이 달라지는 지점을 한곳에서 흡수한다.
 */
pub(super) const SHARED_RUNTIME_NOTICE_PREFIX: &str = "shared runtime ";
pub(super) const ACTIVE_STREAM_ISOLATION_NOTICE_FRAGMENT: &str =
    "app-server connection while a turn stream was active";

pub(super) fn initialize_detail(initialize_response: &InitializeResponse) -> String {
    // initialize response는 startup diagnostics와 terminal attachment event에 들어갈 짧은 environment label이 된다.
    format!(
        "{} / {} / {}",
        initialize_response.platform_os,
        initialize_response.platform_family,
        initialize_response.user_agent,
    )
}

pub(super) fn to_session_summary(thread_record: ThreadRecord) -> SessionSummary {
    /*
     * thread/list response는 app-server의 ThreadRecord 그대로지만 TUI session catalog는 domain SessionSummary를
     * 본다. 여기서 updatedAt/status/gitInfo처럼 protocol naming과 domain naming이 어긋나는 필드를 정리한다.
     */
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
    /*
     * snapshot projection은 thread/read payload를 TUI transcript model로 낮춘다. runtime notice는
     * conversation warning과 다른 UI surface에 표시되어야 하므로 먼저 분리하고, raw turn item JSON은
     * turn_notifications module의 item parser만 통과시킨다.
     */
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
    // app-server name이 비어 있으면 preview 첫 줄을 fallback으로 써서 재개 화면과 session list의 제목 기준을 맞춘다.
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
    // app-server warnings와 runtime retry notices는 여러 retry path에서 합쳐지므로 출력 직전에 안정 정렬한다.
    warnings.sort();
    warnings.dedup();
}

pub(super) fn partition_runtime_notices(warnings: Vec<String>) -> (Vec<String>, Vec<String>) {
    // runtime notice는 실패가 아니라 adapter 운영 상태이므로 transcript warning과 분리해 TUI가 다른 copy로 보여준다.
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
    // shared runtime retry와 active-stream isolated fallback은 둘 다 app-server content warning이 아니라 adapter notice다.
    warning.starts_with(SHARED_RUNTIME_NOTICE_PREFIX)
        || warning.contains(ACTIVE_STREAM_ISOLATION_NOTICE_FRAGMENT)
}

// initialize/account responses는 startup check path에서 app-server readiness와 auth summary를 만든다.
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
        // API-key mode처럼 OpenAI login이 필요 없는 setup은 account object가 없어도 startup을 통과시킨다.
        self.account.is_some() || !self.requires_openai_auth.unwrap_or(false)
    }

    pub(super) fn to_summary_text(&self) -> String {
        // startup panel은 상세 account schema 대신 사용자에게 읽을 수 있는 한 줄 요약만 필요로 한다.
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

/*
 * request parameter structs below are serialized directly into app-server method params. Optional fields use
 * skip_serializing_if so Akra가 의도적으로 override하지 않는 protocol default를 upstream app-server가 유지한다.
 */
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

/*
 * execution policy enums mirror app-server wire vocabulary. execution_policy.rs parses Akra env vars into
 * these values, and thread/turn params below decide whether they are sent at thread scope or turn scope.
 */
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
        // thread start/resume uses SandboxModeValue, while turn/start expects the tagged sandboxPolicy shape.
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

// ThreadStartParams creates new app-server threads, including hidden planning/parallel worker threads.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) developer_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ephemeral: Option<bool>,
}

// ThreadResumeParams reattaches existing threads and reapplies the adapter-owned execution policy envelope.
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

// TurnStartParams starts a turn inside a prepared thread; input ordering matters for skill items before text prompts.
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

// TurnInterruptParams is the narrow payload used when the TUI asks app-server to stop the active turn.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnInterruptParams {
    pub(super) thread_id: String,
    pub(super) turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub(super) enum TurnInputItem {
    // Text carries the user prompt or worker prompt body.
    #[serde(rename = "text")]
    Text { text: String },
    // Skill points app-server at a repo-local SKILL.md before the text prompt is interpreted.
    #[serde(rename = "skill")]
    Skill { name: String, path: String },
}

impl TurnInputItem {
    pub(super) fn text(text: impl Into<String>) -> Self {
        // helper keeps call sites away from serde's tagged enum spelling.
        Self::Text { text: text.into() }
    }

    pub(super) fn skill(name: impl Into<String>, path: impl Into<String>) -> Self {
        // planning_worker_skill.rs uses this to attach the queue mutation evaluator contract to hidden worker turns.
        Self::Skill {
            name: name.into(),
            path: path.into(),
        }
    }
}

/*
 * response structs mirror app-server method outputs. They intentionally stay close to the wire shape, then
 * projection functions above decide what the application/domain layers are allowed to see.
 */
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
    // id is the app-server thread identifier used by resume, turn start, and snapshot reads.
    pub(super) id: String,
    // name can be missing or blank, so thread_title falls back to preview.
    pub(super) name: Option<String>,
    pub(super) preview: String,
    pub(super) cwd: String,
    pub(super) source: String,
    pub(super) model_provider: String,
    pub(super) updated_at: i64,
    pub(super) path: String,
    pub(super) status: ThreadStatus,
    pub(super) git_info: Option<ThreadGitInfo>,
    // thread/list may omit turns; serde default lets the same ThreadRecord shape serve list and read responses.
    #[serde(default)]
    pub(super) turns: Vec<ThreadTurnRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadTurnRecord {
    // item schemas are varied and evolving, so raw Value is parsed by turn_notifications::to_conversation_message.
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
