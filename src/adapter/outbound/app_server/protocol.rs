use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod item_lifecycle;
mod progressive_activity;
mod runtime_envelope;
mod turn_notifications;
mod user_message_projection;

#[cfg(test)]
use self::item_lifecycle::{
    ITEM_PROJECTION_MANIFEST, ItemProjectionManifestRow, SUBAGENT_ACTIVITY_OUTCOME_KINDS,
    UNKNOWN_ITEM_PROJECTION_DECISION,
};
use self::item_lifecycle::{parse_live_item_lifecycle, parse_snapshot_item_lifecycle};
pub(super) use self::progressive_activity::{
    ProgressiveActivityNotificationHandling, parse_progressive_activity_notification,
};
pub(super) use self::runtime_envelope::{
    model_reroute, runtime_configuration_request, settings_observation, status_observation,
    to_runtime_envelope,
};
#[cfg(test)]
use self::turn_notifications::MAX_RETAINED_ITEM_EFFECT_IDENTITIES;
use self::turn_notifications::to_conversation_message;
pub(super) use self::turn_notifications::{
    ActiveTurnNotificationState, AppServerNotification, TurnNotificationHandling,
    handle_turn_notification,
};
use self::user_message_projection::{project_akra_user_message, visible_akra_user_prompt};
use super::{
    MAX_SNAPSHOT_MESSAGES, MAX_SNAPSHOT_TOTAL_TEXT_BYTES, MAX_STREAM_COMPLETED_MESSAGE_BYTES,
    MAX_STREAM_IDENTIFIER_BYTES, MAX_STREAM_METADATA_BYTES, STREAM_TRUNCATION_MARKER,
    bounded_stream_text,
};
use crate::domain::conversation::{ConversationReasoningEffort, ConversationSnapshot};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjection;
use crate::domain::conversation_runtime_envelope::ConversationRuntimeThreadSource;
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
        id: bounded_stream_text(thread_record.id, MAX_STREAM_IDENTIFIER_BYTES),
        name: thread_record
            .name
            .map(|name| bounded_stream_text(name, MAX_STREAM_METADATA_BYTES)),
        preview: bounded_stream_text(
            project_akra_user_message(thread_record.preview),
            MAX_STREAM_METADATA_BYTES,
        ),
        cwd: bounded_stream_text(thread_record.cwd, MAX_STREAM_METADATA_BYTES),
        source: thread_record.source.label().to_string(),
        model_provider: bounded_stream_text(
            thread_record.model_provider,
            MAX_STREAM_IDENTIFIER_BYTES,
        ),
        updated_at_epoch: thread_record.updated_at,
        status_type: bounded_stream_text(
            thread_record.status.status_type,
            MAX_STREAM_IDENTIFIER_BYTES,
        ),
        path: bounded_stream_text(
            thread_record.path.unwrap_or_default(),
            MAX_STREAM_METADATA_BYTES,
        ),
        git_branch: thread_record.git_info.and_then(|git_info| {
            git_info
                .branch
                .map(|branch| bounded_stream_text(branch, MAX_STREAM_IDENTIFIER_BYTES))
        }),
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
    let (mut warnings, runtime_notices) = partition_runtime_notices(warnings);
    let title = bounded_stream_text(thread_title(&thread_record), MAX_STREAM_METADATA_BYTES);
    let source_thread_id = thread_record.id;
    let thread_id = bounded_stream_text(source_thread_id.clone(), MAX_STREAM_IDENTIFIER_BYTES);
    let cwd = bounded_stream_text(thread_record.cwd, MAX_STREAM_METADATA_BYTES);
    let mut messages = Vec::new();
    let mut retained_text_bytes = 0usize;
    let mut snapshot_truncated = false;
    let mut item_lifecycle = ConversationItemLifecycleProjection::default();

    for turn in &thread_record.turns {
        for item in &turn.items {
            match parse_snapshot_item_lifecycle(&source_thread_id, &turn.id, item) {
                Ok(observation) => {
                    let _ = item_lifecycle.apply(observation);
                }
                Err(_) => item_lifecycle.record_invalid_observation(),
            }
        }
    }

    'turns: for turn in thread_record.turns.into_iter().rev() {
        for item in turn.items.into_iter().rev() {
            if messages.len() >= MAX_SNAPSHOT_MESSAGES {
                snapshot_truncated = true;
                break 'turns;
            }
            let Some(mut message) = to_conversation_message(item) else {
                continue;
            };
            let remaining_text_bytes =
                MAX_SNAPSHOT_TOTAL_TEXT_BYTES.saturating_sub(retained_text_bytes);
            if remaining_text_bytes <= STREAM_TRUNCATION_MARKER.len() {
                snapshot_truncated = true;
                break 'turns;
            }
            let body_limit = MAX_STREAM_COMPLETED_MESSAGE_BYTES
                .min(remaining_text_bytes.saturating_sub(STREAM_TRUNCATION_MARKER.len()));
            snapshot_truncated |= message.text.len() > body_limit;
            message.text = bounded_stream_text(message.text, body_limit);
            message.phase = message
                .phase
                .map(|phase| bounded_stream_text(phase, MAX_STREAM_IDENTIFIER_BYTES));
            message.item_id = message
                .item_id
                .map(|item_id| bounded_stream_text(item_id, MAX_STREAM_IDENTIFIER_BYTES));
            retained_text_bytes = retained_text_bytes.saturating_add(message.text.len());
            messages.push(message);
        }
    }
    messages.reverse();
    if snapshot_truncated {
        warnings.push(format!(
            "conversation history was bounded to the newest {} messages / {} text bytes",
            messages.len(),
            retained_text_bytes
        ));
    }

    ConversationSnapshot {
        thread_id,
        title,
        cwd,
        messages,
        warnings,
        runtime_notices,
        item_lifecycle: item_lifecycle.snapshot(),
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
            visible_akra_user_prompt(&thread_record.preview)
                .unwrap_or(&thread_record.preview)
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadSetNameParams {
    pub(super) thread_id: String,
    pub(super) name: String,
}

/*
 * execution policy enums mirror app-server wire vocabulary. execution_policy.rs parses Akra env vars into
 * these values, and thread/turn params below decide whether they are sent at thread scope or turn scope.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum ApprovalPolicyValue {
    #[serde(rename = "untrusted")]
    Untrusted,
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "never")]
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum ApprovalsReviewerValue {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "auto_review")]
    AutoReview,
    // Accepted by current app-server versions only as a compatibility alias.
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
    #[serde(rename = "max")]
    Max,
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
            ConversationReasoningEffort::Max => Self::Max,
        }
    }
}

impl ReasoningEffortValue {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

impl ApprovalPolicyValue {
    pub(super) const fn runtime_policy(
        self,
    ) -> crate::domain::conversation_runtime_envelope::ConversationRuntimeApprovalPolicy {
        use crate::domain::conversation_runtime_envelope::ConversationRuntimeApprovalPolicy;

        match self {
            Self::Untrusted => ConversationRuntimeApprovalPolicy::Untrusted,
            Self::OnRequest => ConversationRuntimeApprovalPolicy::OnRequest,
            Self::Never => ConversationRuntimeApprovalPolicy::Never,
        }
    }
}

impl ApprovalsReviewerValue {
    pub(super) const fn runtime_reviewer(
        self,
    ) -> crate::domain::conversation_runtime_envelope::ConversationRuntimeApprovalsReviewer {
        use crate::domain::conversation_runtime_envelope::ConversationRuntimeApprovalsReviewer;

        match self {
            Self::User => ConversationRuntimeApprovalsReviewer::User,
            Self::AutoReview => ConversationRuntimeApprovalsReviewer::AutoReview,
            Self::GuardianSubagent => ConversationRuntimeApprovalsReviewer::GuardianSubagent,
        }
    }
}

impl SandboxModeValue {
    pub(super) const fn runtime_policy(
        self,
    ) -> crate::domain::conversation_runtime_envelope::ConversationRuntimeSandboxPolicy {
        use crate::domain::conversation_runtime_envelope::ConversationRuntimeSandboxPolicy;

        match self {
            Self::ReadOnly => ConversationRuntimeSandboxPolicy::ReadOnly {
                network_access: None,
            },
            Self::WorkspaceWrite => ConversationRuntimeSandboxPolicy::WorkspaceWrite {
                network_access: None,
                writable_roots: None,
                writable_roots_truncated: false,
                exclude_tmpdir_env_var: None,
                exclude_slash_tmp: None,
            },
            Self::DangerFullAccess => ConversationRuntimeSandboxPolicy::DangerFullAccess,
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
    pub(super) config: Option<BTreeMap<String, Value>>,
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
    pub(super) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approval_policy: Option<ApprovalPolicyValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) approvals_reviewer: Option<ApprovalsReviewerValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sandbox: Option<SandboxModeValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) config: Option<BTreeMap<String, Value>>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnSteerParams {
    pub(super) thread_id: String,
    pub(super) input: Vec<TurnInputItem>,
    pub(super) expected_turn_id: String,
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
    /*
     * LocalImage points app-server at an on-disk image (clipboard paste staging
     * or a referenced file). app-server reads and uploads the bytes, so this
     * client never embeds base64 payloads in the request line.
     */
    #[serde(rename = "localImage")]
    LocalImage { path: String },
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

    pub(super) fn local_image(path: impl Into<String>) -> Self {
        Self::LocalImage { path: path.into() }
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
pub(super) struct ThreadSetNameResponse {}

// Raw flattened response extras remain adapter-local. The custom Debug below
// exposes only structural counts so provider metadata or instruction paths cannot enter logs.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadStartResponse {
    pub(super) thread: ThreadRecord,
    #[serde(flatten)]
    pub(super) runtime_envelope_fields: BTreeMap<String, Value>,
}

impl fmt::Debug for ThreadStartResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ThreadStartResponse")
            .field("thread_id_nonempty", &!self.thread.id.is_empty())
            .field(
                "runtime_envelope_field_count",
                &self.runtime_envelope_fields.len(),
            )
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadResumeResponse {
    pub(super) thread: ThreadRecord,
    #[serde(flatten)]
    pub(super) runtime_envelope_fields: BTreeMap<String, Value>,
}

impl fmt::Debug for ThreadResumeResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ThreadResumeResponse")
            .field("thread_id_nonempty", &!self.thread.id.is_empty())
            .field(
                "runtime_envelope_field_count",
                &self.runtime_envelope_fields.len(),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TurnStartResponse {
    pub(super) turn: TurnRecord,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnSteerResponse {
    pub(super) turn_id: String,
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
    pub(super) source: SessionSourceValue,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionSourceValue {
    Cli,
    Vscode,
    Exec,
    AppServer,
    Custom,
    SubAgentReview,
    SubAgentCompact,
    SubAgentMemoryConsolidation,
    SubAgentThreadSpawn,
    SubAgentOther,
    Unknown,
}

impl SessionSourceValue {
    fn from_wire_value(value: Value) -> Self {
        if let Some(source) = value.as_str() {
            return match source {
                "cli" => Self::Cli,
                "vscode" => Self::Vscode,
                "exec" => Self::Exec,
                "appServer" => Self::AppServer,
                "unknown" => Self::Unknown,
                _ => Self::Unknown,
            };
        }
        let Some(source) = value.as_object() else {
            return Self::Unknown;
        };
        if source.get("custom").is_some_and(Value::is_string) {
            return Self::Custom;
        }
        let Some(sub_agent) = source.get("subAgent") else {
            return Self::Unknown;
        };
        if let Some(sub_agent) = sub_agent.as_str() {
            return match sub_agent {
                "review" => Self::SubAgentReview,
                "compact" => Self::SubAgentCompact,
                "memory_consolidation" => Self::SubAgentMemoryConsolidation,
                _ => Self::SubAgentOther,
            };
        }
        let Some(sub_agent) = sub_agent.as_object() else {
            return Self::Unknown;
        };
        if sub_agent.contains_key("thread_spawn") {
            Self::SubAgentThreadSpawn
        } else if sub_agent.get("other").is_some_and(Value::is_string) {
            Self::SubAgentOther
        } else {
            Self::Unknown
        }
    }

    pub(super) const fn runtime_source(self) -> ConversationRuntimeThreadSource {
        match self {
            Self::Cli => ConversationRuntimeThreadSource::Cli,
            Self::Vscode => ConversationRuntimeThreadSource::Vscode,
            Self::Exec => ConversationRuntimeThreadSource::Exec,
            Self::AppServer => ConversationRuntimeThreadSource::AppServer,
            Self::Custom => ConversationRuntimeThreadSource::Custom,
            Self::SubAgentReview => ConversationRuntimeThreadSource::SubAgentReview,
            Self::SubAgentCompact => ConversationRuntimeThreadSource::SubAgentCompact,
            Self::SubAgentMemoryConsolidation => {
                ConversationRuntimeThreadSource::SubAgentMemoryConsolidation
            }
            Self::SubAgentThreadSpawn => ConversationRuntimeThreadSource::SubAgentThreadSpawn,
            Self::SubAgentOther => ConversationRuntimeThreadSource::SubAgentOther,
            Self::Unknown => ConversationRuntimeThreadSource::Unknown,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Vscode => "vscode",
            Self::Exec => "exec",
            Self::AppServer => "appServer",
            Self::Custom => "custom",
            Self::SubAgentReview => "subAgentReview",
            Self::SubAgentCompact => "subAgentCompact",
            Self::SubAgentMemoryConsolidation => "subAgentMemoryConsolidation",
            Self::SubAgentThreadSpawn => "subAgentThreadSpawn",
            Self::SubAgentOther => "subAgentOther",
            Self::Unknown => "unknown",
        }
    }
}

impl<'de> Deserialize<'de> for SessionSourceValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(Self::from_wire_value)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadTurnRecord {
    #[serde(default)]
    id: String,
    // item schemas are varied and evolving, so raw Value is parsed by turn_notifications::to_conversation_message.
    #[serde(default)]
    items: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ThreadStatus {
    #[serde(rename = "type")]
    pub(super) status_type: String,
    #[serde(rename = "activeFlags", default)]
    pub(super) active_flags: Value,
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

    use super::{
        SessionSourceValue, ThreadReadResponse, ThreadRecord, ThreadSetNameParams,
        ThreadSetNameResponse, ThreadStartResponse, TurnInputItem, TurnSteerParams,
        TurnSteerResponse, thread_title, to_conversation_snapshot, to_session_summary,
    };

    #[test]
    fn session_catalog_preview_and_fallback_title_hide_akra_prompt_contracts() {
        let raw_prompt = concat!(
            "# akra-main-session-turn\n\n",
            "[execution-contract]\ninternal execution rule\n\n",
            "[reporting-contract]\ninternal reporting rule\n\n",
            "[user-prompt]\nresume-visible prompt"
        );
        let record = serde_json::from_value::<ThreadRecord>(json!({
            "id": "thread-akra",
            "name": null,
            "preview": raw_prompt,
            "cwd": "/workspace",
            "source": "appServer",
            "modelProvider": "openai",
            "updatedAt": 1,
            "path": null,
            "status": { "type": "idle" },
            "gitInfo": null
        }))
        .expect("thread record should deserialize");

        assert_eq!(thread_title(&record), "resume-visible prompt");
        let summary = to_session_summary(record);
        assert_eq!(summary.preview, "resume-visible prompt");
        assert!(!summary.preview.contains("execution-contract"));
    }

    #[test]
    fn turn_steer_contract_serializes_exact_precondition_and_text_input() {
        let params = TurnSteerParams {
            thread_id: "thread-1".to_string(),
            input: vec![TurnInputItem::text("correct course")],
            expected_turn_id: "turn-7".to_string(),
        };

        assert_eq!(
            serde_json::to_value(params).expect("turn/steer params should serialize"),
            json!({
                "threadId": "thread-1",
                "input": [{ "type": "text", "text": "correct course" }],
                "expectedTurnId": "turn-7"
            })
        );

        let response = serde_json::from_value::<TurnSteerResponse>(json!({
            "turnId": "turn-7"
        }))
        .expect("turn/steer response should deserialize");
        assert_eq!(response.turn_id, "turn-7");
    }

    #[test]
    fn local_image_items_serialize_with_the_app_server_local_image_tag() {
        // The wire tag must match the app-server LocalImageUserInput contract
        // exactly; a renamed field would silently drop attachments server-side.
        let items = vec![
            TurnInputItem::local_image("/tmp/akra-paste-1.png"),
            TurnInputItem::text("review this screenshot"),
        ];

        assert_eq!(
            serde_json::to_value(items).expect("turn input should serialize"),
            json!([
                { "type": "localImage", "path": "/tmp/akra-paste-1.png" },
                { "type": "text", "text": "review this screenshot" }
            ])
        );
    }

    #[test]
    fn thread_set_name_contract_uses_exact_thread_identity() {
        let params = ThreadSetNameParams {
            thread_id: "thread-1".to_string(),
            name: "Release follow-up".to_string(),
        };

        assert_eq!(
            serde_json::to_value(params).expect("thread/name/set params should serialize"),
            json!({
                "threadId": "thread-1",
                "name": "Release follow-up"
            })
        );
        serde_json::from_value::<ThreadSetNameResponse>(json!({}))
            .expect("thread/name/set response should deserialize");
    }

    #[test]
    fn thread_start_response_accepts_ephemeral_thread_with_null_path() {
        let secret = "AKRA_TEST_SECRET_CANARY_RESPONSE_DEBUG";
        let response = serde_json::from_value::<ThreadStartResponse>(json!({
            "thread": {
                "id": "thread-1",
                "name": null,
                "preview": "",
                "cwd": "/repo",
                "source": { "custom": secret },
                "modelProvider": "openai",
                "updatedAt": 1777910591,
                "path": null,
                "status": { "type": "idle" },
                "gitInfo": null,
                "turns": []
            },
            "providerMetadata": {
                "apiKey": secret
            },
            "instructionSources": [{
                "path": secret,
                "content": secret
            }]
        }))
        .expect("ephemeral thread/start response with null path should deserialize");

        assert!(response.thread.path.is_none());
        let debug = format!("{response:?}");
        assert!(!debug.contains(secret));
        assert!(!debug.contains("providerMetadata"));
        assert!(!debug.contains("instructionSources"));

        let summary = to_session_summary(response.thread);
        assert_eq!(summary.path, "");
        assert_eq!(summary.source, "custom");
    }

    #[test]
    fn session_source_union_classifies_subagent_without_retaining_nested_metadata() {
        let secret = "AKRA_TEST_SECRET_CANARY_SUBAGENT_SOURCE";
        let source = serde_json::from_value::<SessionSourceValue>(json!({
            "subAgent": {
                "thread_spawn": {
                    "depth": 2,
                    "parent_thread_id": secret,
                    "agent_nickname": secret,
                    "agent_role": secret,
                    "agent_path": null
                }
            }
        }))
        .expect("stable subagent source should deserialize");

        assert_eq!(source, SessionSourceValue::SubAgentThreadSpawn);
        assert!(!format!("{source:?}").contains(secret));
    }

    #[test]
    fn conversation_snapshot_bounds_provider_message_before_core_queueing() {
        let oversized = "한".repeat(super::MAX_STREAM_COMPLETED_MESSAGE_BYTES);
        let response = serde_json::from_value::<ThreadReadResponse>(json!({
            "thread": {
                "id": "thread-1",
                "name": "bounded history",
                "preview": "preview",
                "cwd": "/repo",
                "source": "vscode",
                "modelProvider": "openai",
                "updatedAt": 1777910591,
                "path": "/tmp/thread.jsonl",
                "status": { "type": "idle" },
                "gitInfo": null,
                "turns": [{
                    "id": "turn-1",
                    "status": "completed",
                    "items": [{
                        "type": "agentMessage",
                        "id": "agent-1",
                        "phase": "final",
                        "text": oversized
                    }]
                }]
            }
        }))
        .expect("thread/read response should deserialize");

        let snapshot = to_conversation_snapshot(response.thread, Vec::new());
        assert_eq!(snapshot.messages.len(), 1);
        assert!(
            snapshot.messages[0]
                .text
                .ends_with(super::STREAM_TRUNCATION_MARKER)
        );
        assert!(
            snapshot.messages[0].text.len()
                <= super::MAX_STREAM_COMPLETED_MESSAGE_BYTES
                    + super::STREAM_TRUNCATION_MARKER.len()
        );
        assert!(
            snapshot
                .warnings
                .iter()
                .any(|warning| warning.contains("conversation history was bounded"))
        );
    }
}
