use serde::{Deserialize, Serialize};
use serde_json::Value;

mod turn_notifications;

use self::turn_notifications::to_conversation_message;
pub(super) use self::turn_notifications::{
    AppServerNotification, TurnNotificationHandling, handle_turn_notification,
};
use crate::domain::conversation::{ConversationReasoningEffort, ConversationSnapshot};
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
    /*
     * The initialize response is reduced to a compact environment label because the
     * TUI shows it in startup diagnostics and terminal attachment events. Keeping the
     * full response out of higher layers prevents UI copy from depending on upstream
     * initialize fields that are not part of Akra's user-facing contract.
     */
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
        path: thread_record.path.unwrap_or_default(),
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
    /*
     * app-server may return an empty thread name for older or auto-created sessions.
     * Falling back to the preview's first non-empty line keeps resume screens and the
     * session catalog using the same title rule instead of letting each adapter view
     * invent its own placeholder.
     */
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
    /*
     * Warnings are normalized at the projection edge because they can be gathered from
     * stderr, delayed notifications, shared runtime retry notices, and isolated
     * fallback notices. Stable sort/dedup makes repeated retry paths deterministic for
     * tests and prevents duplicate operator copy.
     */
    warnings.sort();
    warnings.dedup();
}

pub(super) fn partition_runtime_notices(warnings: Vec<String>) -> (Vec<String>, Vec<String>) {
    /*
     * Runtime notices describe adapter operations, not conversation content. Splitting
     * them before snapshot projection lets the TUI place reconnect/fallback copy near
     * runtime status while preserving actual app-server warnings beside the transcript.
     */
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
    /*
     * The classifier is intentionally string-based because notices are assembled in
     * lower transport/runtime layers as human-readable diagnostics. The stable prefix
     * and fragment are the adapter's contract for routing those messages.
     */
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
        /*
         * Authentication is not equivalent to "has an account object". API-key setups
         * can be valid without ChatGPT account metadata, so the app-server-provided
         * requiresOpenAIAuth flag is the gate that keeps startup checks from rejecting
         * supported headless configurations.
         */
        self.account.is_some() || !self.requires_openai_auth.unwrap_or(false)
    }

    pub(super) fn to_summary_text(&self) -> String {
        /*
         * Startup copy needs a readable account summary, not the full account schema.
         * This keeps provider-specific fields localized while still surfacing enough
         * detail for operators to recognize ChatGPT, API key, and unauthenticated
         * states.
         */
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
        /*
         * app-server uses two wire shapes for the same policy concept: thread
         * start/resume accepts the legacy sandbox mode enum, while turn/start expects
         * a tagged sandboxPolicy object. Keeping the conversion here prevents request
         * assembly code from knowing both protocol spellings.
         */
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

impl From<ConversationReasoningEffort> for ReasoningEffortValue {
    fn from(effort: ConversationReasoningEffort) -> Self {
        match effort {
            ConversationReasoningEffort::None => Self::None,
            ConversationReasoningEffort::Minimal => Self::Minimal,
            ConversationReasoningEffort::Low => Self::Low,
            ConversationReasoningEffort::Medium => Self::Medium,
            ConversationReasoningEffort::High => Self::High,
            ConversationReasoningEffort::XHigh => Self::XHigh,
        }
    }
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
    // Skill points app-server at a local SKILL.md asset before the text prompt is interpreted.
    #[serde(rename = "skill")]
    Skill { name: String, path: String },
}

impl TurnInputItem {
    pub(super) fn text(text: impl Into<String>) -> Self {
        /*
         * The constructor hides serde's tagged enum shape from prompt assembly code.
         * That keeps input ordering decisions near workers/controllers while protocol
         * field spelling remains centralized in this module.
         */
        Self::Text { text: text.into() }
    }

    pub(super) fn skill(name: impl Into<String>, path: impl Into<String>) -> Self {
        /*
         * Skill items must precede the text prompt when hidden workers need a local
         * evaluator contract. Representing them as first-class turn input keeps the
         * app-server responsible for loading the SKILL.md asset rather than embedding long
         * contract text into every prompt body.
         */
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
    /*
     * id is the stable app-server thread identifier shared by resume, turn start, and
     * snapshot reads. Domain projections keep it verbatim because this adapter cannot
     * synthesize or repair thread identity.
     */
    pub(super) id: String,
    /*
     * name can be missing or blank on upstream records. thread_title is the only
     * projection path that decides whether preview should become the display title.
     */
    pub(super) name: Option<String>,
    pub(super) preview: String,
    pub(super) cwd: String,
    pub(super) source: String,
    pub(super) model_provider: String,
    pub(super) updated_at: i64,
    /*
     * Ephemeral app-server threads, including hidden planning workers, can report
     * `path: null` because there is no durable session record yet. Catalog
     * projections keep the existing domain String contract and fall back to "".
     */
    pub(super) path: Option<String>,
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

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ThreadStartResponse, to_session_summary};

    #[test]
    fn thread_start_response_accepts_ephemeral_thread_with_null_path() {
        let response = serde_json::from_value::<ThreadStartResponse>(json!({
            "thread": {
                "id": "thread-1",
                "name": null,
                "preview": "",
                "cwd": "/repo",
                "source": "vscode",
                "modelProvider": "openai",
                "updatedAt": 1777910591,
                "path": null,
                "status": { "type": "idle" },
                "gitInfo": null,
                "turns": []
            }
        }))
        .expect("ephemeral thread/start response with null path should deserialize");

        assert!(response.thread.path.is_none());

        let summary = to_session_summary(response.thread);
        assert_eq!(summary.path, "");
    }
}
