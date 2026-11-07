//! Akra 커널 응용 프로토콜 v0 (plan 18 K3-1). JSON-RPC `akra/kernel/*` 메서드의 직렬화 타입과, 그 타입과
//! core 타입 사이의 변환을 둔다. core 는 직렬화를 모르므로 변환은 이 inbound adapter 만 한다. 전송(Gateway)과
//! 커널 호스트는 K3-2 이후 단계가 붙인다.

use serde::{Deserialize, Serialize};

use crate::core::app::{
    ActiveTurnPhase, AppCommand, AppEvent, ApprovalAuthorityPhase, ApprovalAuthoritySnapshot,
    ConversationRuntimeSnapshot, PlanningWorkspaceResetIntent, PlanningWorkspaceResetTarget,
    PostTurnAuthoritySnapshot, SESSION_CATALOG_PAGE_SIZE, SessionCatalogLoadIntent,
    SessionCatalogSnapshot, ThreadActivitySummary,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequestIdentity, ConversationTurnSteerRequest,
};
use crate::domain::recent_sessions::{SessionCatalog, SessionRenameRequest};

#[cfg(test)]
mod tests;

/// 프로토콜 판이다. 호환되지 않게 바꾸면 올린다. K4 에서 명령 스펙으로 생성하면서 1 이 된다.
pub const PROTOCOL_VERSION: u32 = 0;

pub const METHOD_ATTACH: &str = "akra/kernel/attach";
pub const METHOD_DESCRIBE: &str = "akra/kernel/describe";
pub const METHOD_COMMAND: &str = "akra/kernel/command";
pub const METHOD_QUERY: &str = "akra/kernel/query";
pub const METHOD_EVENT: &str = "akra/kernel/event";
pub const METHOD_DETACH: &str = "akra/kernel/detach";

const METHODS: [&str; 6] = [
    METHOD_ATTACH,
    METHOD_DESCRIBE,
    METHOD_COMMAND,
    METHOD_QUERY,
    METHOD_EVENT,
    METHOD_DETACH,
];

/// JSON-RPC 의 잘못된 인자 오류 코드이다.
pub const ERROR_INVALID_PARAMS: i64 = -32602;
/// 확인이 필요한 명령을 확인 없이 보냈다 (K-D1).
pub const ERROR_CONFIRMATION_REQUIRED: i64 = -32010;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelProtocolError {
    pub code: i64,
    pub message: String,
}

impl KernelProtocolError {
    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: ERROR_INVALID_PARAMS,
            message: message.into(),
        }
    }
}

// attach -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachRequest {
    /// 커널을 고르는 작업 공간 루트이다.
    pub workspace: String,
    /// 진단용 클라이언트 이름이다 (예: `akra-desktop`).
    pub client_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachResponse {
    pub client_id: String,
    pub protocol_version: u32,
}

// command -----------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub command: KernelCommand,
    /// 같은 클라이언트가 같은 키로 다시 보내면 한 번만 처리한다.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResponse {
    /// 커널이 명령을 받아들였다. 결과는 `akra/kernel/event` 로 온다.
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/*
 * v0 명령이다. 화면 흐름 단계가 아니라 한 번에 끝나는 도메인 명령이다 (plan 18 §5.5). 프롬프트 제출(준비 +
 * 제출)은 core 가 준비 뒤 제출을 이어서 해야 하므로 K3-3 에서 더한다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 필드가 없는 명령도 `{}` 로 둔다. serde 는 unit variant 의 모르는 필드를 거절하지 못한다.
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelCommand {
    LoadSessions {},
    OpenConversation {
        thread_id: String,
    },
    NewDraft {},
    RenameSession {
        thread_id: String,
        name: String,
    },
    SteerTurn {
        thread_id: String,
        expected_turn_id: String,
        prompt: String,
    },
    StopAll {},
    DecideApproval {
        approval_id: String,
        server_request_id: String,
        decision: ApprovalDecision,
    },
    SetAutoFollowMaxTurns {
        value: usize,
    },
    PauseAutoFollow {},
    ResetPlanning {
        target: ResetTarget,
        /// 지우는 명령이라 `true` 가 아니면 거절한다 (K-D1).
        #[serde(default)]
        confirm: bool,
    },
}

const COMMANDS: [&str; 10] = [
    "load_sessions",
    "open_conversation",
    "new_draft",
    "rename_session",
    "steer_turn",
    "stop_all",
    "decide_approval",
    "set_auto_follow_max_turns",
    "pause_auto_follow",
    "reset_planning",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Accept,
    Decline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetTarget {
    Queue,
    Directions,
    All,
}

impl KernelCommand {
    /// 커널이 처리할 core 명령으로 바꾼다. 작업 공간은 attach 때 고른 커널의 것이다.
    pub fn into_core(self, workspace_directory: &str) -> Result<AppCommand, KernelProtocolError> {
        let command = match self {
            Self::LoadSessions {} => {
                AppCommand::LoadSessionCatalog(SessionCatalogLoadIntent::ensure_loaded(
                    SESSION_CATALOG_PAGE_SIZE,
                    workspace_directory,
                ))
            }
            Self::OpenConversation { thread_id } => AppCommand::LoadConversation {
                thread_id: non_blank("thread_id", thread_id)?,
                fallback_workspace_directory: workspace_directory.to_string(),
            },
            Self::NewDraft {} => AppCommand::InvalidateConversationLoad,
            Self::RenameSession { thread_id, name } => {
                AppCommand::RenameSession(SessionRenameRequest::new(
                    non_blank("thread_id", thread_id)?,
                    non_blank("name", name)?,
                ))
            }
            Self::SteerTurn {
                thread_id,
                expected_turn_id,
                prompt,
            } => AppCommand::SteerTurn(ConversationTurnSteerRequest {
                thread_id: non_blank("thread_id", thread_id)?,
                expected_turn_id: non_blank("expected_turn_id", expected_turn_id)?,
                prompt: non_blank("prompt", prompt)?,
            }),
            Self::StopAll {} => AppCommand::RequestStopAllSessions,
            Self::DecideApproval {
                approval_id,
                server_request_id,
                decision,
            } => AppCommand::SubmitApprovalDecision {
                request_identity: ConversationApprovalRequestIdentity {
                    approval_id: non_blank("approval_id", approval_id)?,
                    server_request_id: non_blank("server_request_id", server_request_id)?,
                },
                decision: match decision {
                    ApprovalDecision::Accept => ConversationApprovalDecision::Accept,
                    ApprovalDecision::Decline => ConversationApprovalDecision::Decline,
                },
            },
            Self::SetAutoFollowMaxTurns { value } => AppCommand::SetAutoFollowMaxTurns { value },
            Self::PauseAutoFollow {} => AppCommand::PausePostTurnContinuation,
            Self::ResetPlanning { target, confirm } => {
                if !confirm {
                    return Err(KernelProtocolError {
                        code: ERROR_CONFIRMATION_REQUIRED,
                        message: "reset_planning deletes planning data; send \"confirm\": true"
                            .to_string(),
                    });
                }
                AppCommand::ResetPlanningWorkspace(PlanningWorkspaceResetIntent::new(
                    workspace_directory,
                    match target {
                        ResetTarget::Queue => PlanningWorkspaceResetTarget::Queue,
                        ResetTarget::Directions => PlanningWorkspaceResetTarget::Directions,
                        ResetTarget::All => PlanningWorkspaceResetTarget::All,
                    },
                ))
            }
        };
        Ok(command)
    }
}

fn non_blank(field: &str, value: String) -> Result<String, KernelProtocolError> {
    if value.trim().is_empty() {
        return Err(KernelProtocolError::invalid_params(format!(
            "{field} must not be blank"
        )));
    }
    Ok(value)
}

// query -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelQuery {
    Sessions {},
    ConversationRuntime {},
    ThreadActivity {},
}

const QUERIES: [&str; 3] = ["sessions", "conversation_runtime", "thread_activity"];

// event -------------------------------------------------------------------------------------------

/*
 * v0 이벤트는 굵은 단위이다 (plan 18 §5.5). 클라이언트가 지금 상태를 다시 그릴 수 있는 값을 통째로 보낸다.
 * 증분 스트림은 보내지 않는다. 같은 모양을 조회 결과로도 쓴다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelEvent {
    Sessions(SessionsState),
    ConversationRuntime(ConversationRuntimeSummary),
    ThreadActivity {
        threads: Vec<ThreadActivity>,
    },
    /// 이 클라이언트에게 보낼 이벤트가 넘쳐 버렸다. 클라이언트는 조회로 다시 맞춘다.
    Resync,
}

const EVENTS: [&str; 4] = [
    "sessions",
    "conversation_runtime",
    "thread_activity",
    "resync",
];

impl KernelEvent {
    /// 프로토콜로 내보낼 core 이벤트만 바꾼다. 나머지는 v0 에서 보내지 않는다.
    pub fn from_core(event: &AppEvent) -> Option<Self> {
        match event {
            AppEvent::SessionCatalogChanged(snapshot) => {
                Some(Self::Sessions(SessionsState::from_core(snapshot)))
            }
            AppEvent::ConversationRuntimeAuthorityChanged(runtime) => Some(
                Self::ConversationRuntime(ConversationRuntimeSummary::from_core(runtime)),
            ),
            AppEvent::ThreadActivityChanged(threads) => Some(Self::ThreadActivity {
                threads: threads.iter().map(ThreadActivity::from_core).collect(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsState {
    pub status: SessionsStatus,
    pub sessions: Vec<SessionEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionsStatus {
    Idle,
    Loading,
    Ready,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub thread_id: String,
    pub title: String,
    pub workspace: String,
    pub updated_at_epoch: i64,
}

impl SessionsState {
    pub fn from_core(snapshot: &SessionCatalogSnapshot) -> Self {
        let (status, sessions, message) = match snapshot {
            SessionCatalogSnapshot::Idle => (SessionsStatus::Idle, Vec::new(), None),
            SessionCatalogSnapshot::Loading => (SessionsStatus::Loading, Vec::new(), None),
            SessionCatalogSnapshot::Failed { message } => {
                (SessionsStatus::Failed, Vec::new(), Some(message.clone()))
            }
            SessionCatalogSnapshot::Ready(ready) => match ready.catalog.as_ref() {
                SessionCatalog::Ready {
                    recent_sessions, ..
                } => (
                    SessionsStatus::Ready,
                    recent_sessions
                        .items
                        .iter()
                        .map(|session| SessionEntry {
                            thread_id: session.id.clone(),
                            title: session.title(),
                            workspace: session.cwd.clone(),
                            updated_at_epoch: session.updated_at_epoch,
                        })
                        .collect(),
                    None,
                ),
                SessionCatalog::Unsupported(_) | SessionCatalog::Partial(_) => {
                    (SessionsStatus::Unavailable, Vec::new(), None)
                }
            },
        };
        Self {
            status,
            sessions,
            message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRuntimeSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<ActiveTurn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<PendingApproval>,
    pub auto_follow: AutoFollow,
    pub post_turn: PostTurnStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveTurn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingApproval {
    pub approval_id: String,
    pub server_request_id: String,
    pub summary: String,
    pub details: Vec<String>,
    pub decision_in_flight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFollow {
    pub max_turns: usize,
    pub completed_turns: usize,
    pub active: bool,
    pub paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostTurnStatus {
    Idle,
    Evaluating,
    AwaitingRoute,
    Settled,
}

impl ConversationRuntimeSummary {
    pub fn from_core(runtime: &ConversationRuntimeSnapshot) -> Self {
        Self {
            active_turn: runtime.active_turn.as_ref().map(|turn| ActiveTurn {
                turn_id: turn.turn_id.clone(),
                running: turn.phase == ActiveTurnPhase::Running,
            }),
            approval: runtime.approval.as_ref().map(pending_approval),
            auto_follow: AutoFollow {
                max_turns: runtime.auto_follow.max_auto_turns,
                completed_turns: runtime.auto_follow.completed_auto_turns,
                active: runtime.auto_follow.has_live_activity(),
                paused: runtime.auto_follow.continuation_paused,
            },
            post_turn: match runtime.post_turn {
                PostTurnAuthoritySnapshot::Idle => PostTurnStatus::Idle,
                PostTurnAuthoritySnapshot::Evaluating { .. } => PostTurnStatus::Evaluating,
                PostTurnAuthoritySnapshot::AwaitingRoute { .. } => PostTurnStatus::AwaitingRoute,
                PostTurnAuthoritySnapshot::Settled { .. } => PostTurnStatus::Settled,
            },
        }
    }
}

fn pending_approval(approval: &ApprovalAuthoritySnapshot) -> PendingApproval {
    PendingApproval {
        approval_id: approval.request.approval_id.clone(),
        server_request_id: approval.request.server_request_id.clone(),
        summary: approval.request.summary.clone(),
        details: approval.request.details.clone(),
        decision_in_flight: approval.phase != ApprovalAuthorityPhase::Pending,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadActivity {
    pub thread_id: String,
    pub running: bool,
    pub approval_pending: bool,
    pub auto_follow_active: bool,
}

impl ThreadActivity {
    pub fn from_core(activity: &ThreadActivitySummary) -> Self {
        Self {
            thread_id: activity.thread_id.clone(),
            running: activity.running,
            approval_pending: activity.approval_pending,
            auto_follow_active: activity.auto_follow_active,
        }
    }
}

// describe ----------------------------------------------------------------------------------------

/// 자기 설명이다 (plan 18 §4.3). 클라이언트와 에이전트가 쓸 수 있는 메서드, 명령, 조회, 이벤트를 알린다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelDescription {
    pub protocol_version: u32,
    pub methods: Vec<String>,
    pub commands: Vec<String>,
    pub queries: Vec<String>,
    pub events: Vec<String>,
}

pub fn describe() -> KernelDescription {
    let owned = |names: &[&str]| names.iter().map(|name| (*name).to_string()).collect();
    KernelDescription {
        protocol_version: PROTOCOL_VERSION,
        methods: owned(&METHODS),
        commands: owned(&COMMANDS),
        queries: owned(&QUERIES),
        events: owned(&EVENTS),
    }
}
