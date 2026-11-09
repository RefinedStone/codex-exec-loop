//! Akra 커널 응용 프로토콜 v0 (plan 18 K3-1). JSON-RPC `akra/kernel/*` 메서드의 직렬화 타입과, 그 타입과
//! core 타입 사이의 변환을 둔다. core 는 직렬화를 모르므로 변환은 이 inbound adapter 만 한다. 커널 호스트는
//! `src/composition/kernel_host.rs` 이다. 전송(Gateway)은 K3-5 에서 붙인다.

use serde::{Deserialize, Serialize};

use crate::core::app::{
    ActiveTurnPhase, AppCommand, AppEvent, ApprovalAuthorityPhase, ApprovalAuthoritySnapshot,
    ConversationRuntimeSnapshot, PlanningWorkspaceResetIntent, PlanningWorkspaceResetTarget,
    PostTurnAuthoritySnapshot, SESSION_CATALOG_PAGE_SIZE, SessionCatalogLoadIntent,
    SessionCatalogSnapshot, ThreadActivitySummary, ThreadPromptAdmission, ThreadPromptOutcome,
    ThreadPromptSubmission, TurnSubmissionAdmission,
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
    /// 붙은 커널의 저장소 신원이다. 같은 저장소의 worktree 들은 같은 값을 받는다 (§3.4).
    pub kernel: String,
}

// command -----------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub command: KernelCommand,
    // 멱등 키는 v0 에 없다. 재전송을 가리려면 커널이 처리한 명령을 기억해야 하므로 사건 기록(K5)과 함께 더한다.
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResponse {
    /// 커널이 명령을 처리했다. 받아들였는지와 결과는 그 전에 `akra/kernel/event` 로 온다 (예:
    /// `prompt_submission`, `thread_activity`). `false` 는 커널이 끝나서 명령을 넣지 못한 경우이다.
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/*
 * v0 명령이다. 화면 흐름 단계가 아니라 한 번에 끝나는 도메인 명령이다 (plan 18 §5.5). 원격 클라이언트는 보는
 * 자리를 바꾸지 않고 thread 나 승인 id 를 지정한다 (K3-3a). 프롬프트 제출은 준비와 제출을 하나로 한다 (K3-3b).
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 필드가 없는 명령도 `{}` 로 둔다. serde 는 unit variant 의 모르는 필드를 거절하지 못한다.
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelCommand {
    LoadSessions {},
    RenameSession {
        thread_id: String,
        name: String,
    },
    /// thread 에 프롬프트를 보낸다. 커널이 planning 조각을 붙여 준비한 뒤 턴을 제출한다.
    SubmitPrompt {
        thread_id: String,
        prompt: String,
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

const COMMANDS: [&str; 9] = [
    "load_sessions",
    "rename_session",
    "submit_prompt",
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
            Self::RenameSession { thread_id, name } => {
                AppCommand::RenameSession(SessionRenameRequest::new(
                    non_blank("thread_id", thread_id)?,
                    non_blank("name", name)?,
                ))
            }
            Self::SubmitPrompt { thread_id, prompt } => {
                AppCommand::SubmitThreadPrompt(Box::new(ThreadPromptSubmission {
                    workspace_directory: workspace_directory.to_string(),
                    thread_id: non_blank("thread_id", thread_id)?,
                    prompt: non_blank("prompt", prompt)?,
                }))
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
                AppCommand::ResetPlanningWorkspace(
                    PlanningWorkspaceResetIntent::new(
                        workspace_directory,
                        match target {
                            ResetTarget::Queue => PlanningWorkspaceResetTarget::Queue,
                            ResetTarget::Directions => PlanningWorkspaceResetTarget::Directions,
                            ResetTarget::All => PlanningWorkspaceResetTarget::All,
                        },
                    )
                    .with_confirmation(confirm),
                )
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
    /// `submit_prompt` 의 진행이다. 턴이 시작되면 `thread_activity` 에도 나타난다.
    PromptSubmission(PromptSubmissionState),
    /// 이 클라이언트에게 보낼 이벤트가 넘쳐 버렸다. 클라이언트는 조회로 다시 맞춘다.
    Resync,
}

const EVENTS: [&str; 5] = [
    "sessions",
    "conversation_runtime",
    "thread_activity",
    "prompt_submission",
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
            AppEvent::ThreadPromptAdmissionResolved {
                thread_id,
                admission,
            } => Some(Self::PromptSubmission(PromptSubmissionState::admitted(
                thread_id, admission,
            ))),
            AppEvent::ThreadPromptSubmitted { thread_id, outcome } => Some(Self::PromptSubmission(
                PromptSubmissionState::submitted(thread_id, outcome),
            )),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalRequest>,
    pub auto_follow_active: bool,
}

/// 결정을 기다리는 승인 요청이다. `decide_approval` 에 두 id 를 그대로 보낸다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub server_request_id: String,
    pub summary: String,
    pub details: Vec<String>,
}

impl ThreadActivity {
    pub fn from_core(activity: &ThreadActivitySummary) -> Self {
        Self {
            thread_id: activity.thread_id.clone(),
            running: activity.running,
            active_turn_id: activity.active_turn_id.clone(),
            approval: activity.approval.as_ref().map(|request| ApprovalRequest {
                approval_id: request.approval_id.clone(),
                server_request_id: request.server_request_id.clone(),
                summary: request.summary.clone(),
                details: request.details.clone(),
            }),
            auto_follow_active: activity.auto_follow_active,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSubmissionState {
    pub thread_id: String,
    pub status: PromptSubmissionStatus,
    /// `rejected`, `failed` 의 이유 코드이다. 클라이언트와 에이전트는 이 값으로 다음 행동을 고른다 (plan 19 P3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PromptSubmissionReason>,
    /// 사람이 읽는 설명이다. 판단에는 `reason` 을 쓴다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSubmissionStatus {
    /// 받아들였고 준비 중이다.
    Preparing,
    /// 준비가 끝나고 턴을 제출했다.
    Submitted,
    /// 받아들이지 않았다. 이유는 `reason` 이다.
    Rejected,
    /// 준비가 턴을 만들지 못했다.
    Failed,
}

/// 제출하지 못한 이유이다. 이름이 다음 행동을 알려 준다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSubmissionReason {
    /// 턴이 돌거나 그 thread 에 일이 남아 있다. `thread_activity` 가 비면 다시 보낸다.
    Busy,
    /// 다른 프롬프트를 준비하고 있다. 그 준비가 끝나면 다시 보낸다.
    PreparationActive,
    /// 커널이 모르는 thread 이다. `load_sessions` 로 목록을 읽는다.
    UnknownThread,
    /// 병렬 모드가 켜져 있다. TUI 에서 제출한다.
    ParallelMode,
    /// 준비하는 사이에 다른 턴이 시작했다. 준비가 만든 planning 작업은 큐에 남는다.
    TurnStartedDuringPreparation,
    /// 정지 요청이 아직 끝나지 않았다.
    StopPending,
    /// 그 thread 가 지금 프롬프트를 받을 수 없다.
    Unavailable,
    /// 준비가 실패했다. `message` 가 원인이다.
    PreparationFailed,
}

impl PromptSubmissionState {
    fn admitted(thread_id: &str, admission: &ThreadPromptAdmission) -> Self {
        let rejected = match admission {
            ThreadPromptAdmission::Accepted { .. } => None,
            ThreadPromptAdmission::RejectedBusy => Some((
                PromptSubmissionReason::Busy,
                "a turn is running or the thread is still busy",
            )),
            ThreadPromptAdmission::RejectedPreparationActive { .. } => Some((
                PromptSubmissionReason::PreparationActive,
                "another prompt is being prepared",
            )),
            ThreadPromptAdmission::RejectedUnknownThread => Some((
                PromptSubmissionReason::UnknownThread,
                "unknown thread; load_sessions lists the known threads",
            )),
            ThreadPromptAdmission::RejectedParallelMode => Some((
                PromptSubmissionReason::ParallelMode,
                "parallel mode is on; submit the prompt in the TUI",
            )),
        };
        match rejected {
            None => Self::progress(thread_id, PromptSubmissionStatus::Preparing),
            Some((reason, message)) => Self::stopped(
                thread_id,
                PromptSubmissionStatus::Rejected,
                reason,
                message.to_string(),
            ),
        }
    }

    fn submitted(thread_id: &str, outcome: &ThreadPromptOutcome) -> Self {
        let (reason, message) = match outcome {
            ThreadPromptOutcome::Submitted(TurnSubmissionAdmission::Accepted { .. }) => {
                return Self::progress(thread_id, PromptSubmissionStatus::Submitted);
            }
            ThreadPromptOutcome::Submitted(TurnSubmissionAdmission::RejectedActive { .. }) => (
                PromptSubmissionReason::TurnStartedDuringPreparation,
                "another turn started while the prompt was prepared; its planning task stays queued"
                    .to_string(),
            ),
            ThreadPromptOutcome::Submitted(TurnSubmissionAdmission::RejectedStopPending {
                ..
            }) => (
                PromptSubmissionReason::StopPending,
                "a stop request is still settling".to_string(),
            ),
            ThreadPromptOutcome::Submitted(TurnSubmissionAdmission::RejectedUnavailable) => (
                PromptSubmissionReason::Unavailable,
                "the thread cannot take a prompt now".to_string(),
            ),
            ThreadPromptOutcome::PreparationFailed { reason } => {
                return Self::stopped(
                    thread_id,
                    PromptSubmissionStatus::Failed,
                    PromptSubmissionReason::PreparationFailed,
                    reason.clone(),
                );
            }
        };
        Self::stopped(thread_id, PromptSubmissionStatus::Rejected, reason, message)
    }

    fn progress(thread_id: &str, status: PromptSubmissionStatus) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            status,
            reason: None,
            message: None,
        }
    }

    fn stopped(
        thread_id: &str,
        status: PromptSubmissionStatus,
        reason: PromptSubmissionReason,
        message: String,
    ) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            status,
            reason: Some(reason),
            message: Some(message),
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
