//! Akra 커널 응용 프로토콜 v0 (plan 18 K3-1). JSON-RPC `akra/kernel/*` 메서드의 직렬화 타입과, 그 타입과
//! core 타입 사이의 변환을 둔다. core 는 직렬화를 모르므로 변환은 이 inbound adapter 만 한다. 커널 호스트는
//! `src/composition/kernel_host.rs` 이다. 전송(Gateway)은 K3-5 에서 붙인다.

use crate::domain::kernel_event::{KernelEventPage, RecordedKernelEvent};
use serde::{Deserialize, Serialize};

use crate::core::app::{
    AppCommand, AppEvent, AppSnapshot, ApprovalDecisionAdmission, ConversationRuntimeSnapshot,
    PlanningWorkspaceResetIntent, PlanningWorkspaceResetTarget, SESSION_CATALOG_PAGE_SIZE,
    SessionCatalogLoadIntent, SessionCatalogSnapshot, ThreadActivitySummary, ThreadPromptAdmission,
    ThreadPromptOutcome, ThreadPromptPreparationFailure, ThreadPromptSubmission,
    TurnSteerAdmission, TurnSubmissionAdmission,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequestIdentity, ConversationTurnSteerRequest,
};
use crate::domain::parallel_mode::{ParallelModeSupervisorSnapshot, ParallelModeSupervisorState};
use crate::domain::planning::{
    PriorityQueueTask, RuntimeProjection, RuntimeWorkspaceStatus, TaskStatus,
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
/// 이 접속은 아직 커널에 붙지 않았다. `akra/kernel/attach` 를 먼저 보낸다.
pub const ERROR_NOT_ATTACHED: i64 = -32011;
/// 커널에 붙거나 닿지 못했다. 예: 다른 프로세스(TUI)가 그 저장소의 커널을 돌린다, 커널이 끝났다.
pub const ERROR_KERNEL_UNAVAILABLE: i64 = -32012;
/// 커널이 정한 시간 안에 답하지 않았다. 명령은 그 뒤에 처리될 수 있으므로, 클라이언트는 조회로 결과를 확인한다.
pub const ERROR_KERNEL_TIMEOUT: i64 = -32013;

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
    /// 커널이 명령을 처리했다. `false` 는 커널이 끝나서 명령을 넣지 못한 경우이다.
    pub processed: bool,
    /// 커널이 이 명령을 바로 거절한 이유이다 (`steer_turn`, `decide_approval`). `submit_prompt` 의 진행과 거절은
    /// `prompt_submission` 이벤트로 온다. 받아들인 명령의 결과는 이벤트(`thread_activity` 등)로 온다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<CommandRejection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRejection {
    pub reason: CommandRejectionReason,
    /// 사람이 읽는 설명이다. 판단에는 `reason` 을 쓴다.
    pub message: String,
}

/// 명령을 거절한 이유이다. 이름이 다음 행동을 알려 준다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandRejectionReason {
    /// 같은 턴에 보낸 steer 가 아직 처리 중이다. 끝난 뒤 다시 보낸다.
    SteerInFlight,
    /// 그 thread 에 `expected_turn_id` 로 도는 턴이 없다. `thread_activity` 의 `active_turn_id` 를 쓴다.
    NoMatchingTurn,
    /// 같은 승인에 보낸 결정이 아직 처리 중이다.
    DecisionInFlight,
    /// 기다리는 승인 중에 그 id 가 없다. `thread_activity` 의 `approval` 을 쓴다.
    NoMatchingApproval,
}

impl CommandResponse {
    /// 커널이 명령을 처리하며 낸 core 이벤트로 응답을 만든다. `None` 은 커널이 끝난 경우이다.
    pub fn from_core(events: Option<&[AppEvent]>) -> Self {
        let Some(events) = events else {
            return Self {
                processed: false,
                rejection: None,
            };
        };
        Self {
            processed: true,
            rejection: events.iter().find_map(CommandRejection::from_core),
        }
    }
}

impl CommandRejection {
    fn from_core(event: &AppEvent) -> Option<Self> {
        let (reason, message) = match event {
            AppEvent::TurnSteerAdmissionResolved(TurnSteerAdmission::RejectedActive { .. }) => (
                CommandRejectionReason::SteerInFlight,
                "another steer for this turn is still in flight",
            ),
            AppEvent::TurnSteerAdmissionResolved(TurnSteerAdmission::RejectedUnavailable) => (
                CommandRejectionReason::NoMatchingTurn,
                "the thread has no running turn with that id",
            ),
            AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedActive { .. },
            ) => (
                CommandRejectionReason::DecisionInFlight,
                "a decision for this approval is still in flight",
            ),
            AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedUnavailable,
            ) => (
                CommandRejectionReason::NoMatchingApproval,
                "no pending approval has that id",
            ),
            _ => return None,
        };
        Some(Self {
            reason,
            message: message.to_string(),
        })
    }
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
    /// planning 큐를 다시 읽는다. 결과는 `planning` 이벤트로 온다.
    RefreshPlanning {},
}

const COMMANDS: [&str; 10] = [
    "load_sessions",
    "rename_session",
    "submit_prompt",
    "steer_turn",
    "stop_all",
    "decide_approval",
    "set_auto_follow_max_turns",
    "pause_auto_follow",
    "reset_planning",
    "refresh_planning",
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
            Self::RefreshPlanning {} => AppCommand::RefreshPlanningRuntime {
                workspace_directory: workspace_directory.to_string(),
            },
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
    AutoFollow {},
    ThreadActivity {},
    Planning {},
    Parallel {},
    /*
     * 판단 사건 기록이다 (plan 19 Q3-3). `after` 보다 뒤의 사건을 기록 순서대로 최대 `limit` 개 준다. 처음에는
     * `after` 를 비운다. 답의 `next_after` 를 다음 조회의 `after` 로 보낸다.
     */
    History {
        #[serde(default)]
        after: Option<i64>,
        #[serde(default)]
        limit: Option<u32>,
    },
}

/// 기록 조회의 한 쪽 기본 크기이다.
const DEFAULT_HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    pub query: KernelQuery,
}

impl KernelQuery {
    /// 기록 조회이면 읽을 cursor 와 개수이다. 기록은 스냅샷이 아니라 커널의 사건 기록에서 읽는다.
    pub fn history_range(&self) -> Option<(i64, usize)> {
        match self {
            Self::History { after, limit } => Some((
                after.unwrap_or(0),
                limit.map_or(DEFAULT_HISTORY_LIMIT, |limit| limit as usize),
            )),
            _ => None,
        }
    }

    /// 조회 결과는 같은 이름의 이벤트와 같은 모양이다. 클라이언트는 둘을 같은 코드로 그린다. 기록 조회는 호출자가
    /// 읽은 `history` 쪽을 그대로 담는다.
    pub fn answer(
        self,
        snapshot: &AppSnapshot,
        threads: &[ThreadActivitySummary],
        parallel: ParallelSource<'_>,
        history: Option<KernelEventPage>,
    ) -> KernelEvent {
        match self {
            Self::Sessions {} => {
                KernelEvent::Sessions(SessionsState::from_core(&snapshot.session_catalog))
            }
            Self::AutoFollow {} => KernelEvent::AutoFollow(AutoFollowSettings::from_core(
                &snapshot.conversation_runtime,
            )),
            Self::ThreadActivity {} => KernelEvent::ThreadActivity {
                threads: threads.iter().map(ThreadActivity::from_core).collect(),
            },
            Self::Planning {} => KernelEvent::Planning(PlanningState::from_core(
                &snapshot.planning_parallel.planning_runtime,
            )),
            Self::Parallel {} => KernelEvent::Parallel(ParallelState::from_core(parallel)),
            Self::History { after, .. } => {
                KernelEvent::History(history.unwrap_or_else(|| KernelEventPage {
                    records: Vec::new(),
                    next_after: after.unwrap_or(0),
                }))
            }
        }
    }
}

const QUERIES: [&str; 6] = [
    "sessions",
    "auto_follow",
    "thread_activity",
    "planning",
    "parallel",
    "history",
];

// event -------------------------------------------------------------------------------------------

/*
 * v0 이벤트는 굵은 단위이다 (plan 18 §5.5). 클라이언트가 지금 상태를 다시 그릴 수 있는 값을 통째로 보낸다.
 * 증분 스트림은 보내지 않는다. 같은 모양을 조회 결과로도 쓴다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelEvent {
    Sessions(SessionsState),
    AutoFollow(AutoFollowSettings),
    ThreadActivity {
        threads: Vec<ThreadActivity>,
    },
    /// `submit_prompt` 의 진행이다. 턴이 시작되면 `thread_activity` 에도 나타난다.
    PromptSubmission(PromptSubmissionState),
    /// planning 큐가 바뀌었다 (K3-6).
    Planning(PlanningState),
    /// 병렬 작업 상태가 바뀌었다 (K3-6). 클라이언트가 붙어 있는 동안 커널이 감독 스냅샷을 주기적으로 새로 고친다.
    Parallel(ParallelState),
    /// 이 클라이언트에게 보낼 이벤트가 넘쳐 버렸다. 클라이언트는 조회로 다시 맞춘다.
    Resync,
    /*
     * 기록 조회(`history`)의 답이다 (plan 19 Q3-3). 이벤트로는 보내지 않는다. 사건 하나는 `sequence`, `recorded_at`,
     * `event` 이고 `event.kind` 가 종류이다 (`turn_started` 등).
     */
    History(KernelEventPage),
    /*
     * 커널이 판단 사건 하나를 기록했다 (plan 19 Q3-3b). `sequence` 는 기록과 같으므로 `history` 의 cursor 로 쓴다.
     * `resync` 를 받으면 마지막으로 받은 `sequence` 를 `after` 로 `history` 를 읽어 빠진 사건을 채운다. 붙은 뒤
     * `history` 와 겹칠 수 있으므로 클라이언트는 `sequence` 로 겹침을 지운다.
     */
    Recorded(RecordedKernelEvent),
}

/// `akra/kernel/event` 알림의 params 이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventNotification {
    pub event: KernelEvent,
}

const EVENTS: [&str; 8] = [
    "sessions",
    "auto_follow",
    "thread_activity",
    "prompt_submission",
    "planning",
    "parallel",
    "resync",
    "recorded",
];

impl KernelEvent {
    /// 프로토콜로 내보낼 core 이벤트만 바꾼다. 나머지는 v0 에서 보내지 않는다.
    pub fn from_core(event: &AppEvent) -> Option<Self> {
        match event {
            AppEvent::SessionCatalogChanged(snapshot) => {
                Some(Self::Sessions(SessionsState::from_core(snapshot)))
            }
            AppEvent::ConversationRuntimeAuthorityChanged(runtime) => {
                Some(Self::AutoFollow(AutoFollowSettings::from_core(runtime)))
            }
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
                PromptSubmissionState::from_outcome(thread_id, outcome),
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

/*
 * 자동 후속 설정이다. 설정은 모든 thread 에 같이 걸린다. thread 별 진행(자동 후속이 도는지, 턴, 승인)은
 * `thread_activity` 가 알린다. 보는 대화의 턴 상태는 TUI 화면용이라 v0 에서 보내지 않는다 (K3 §3.2).
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFollowSettings {
    pub max_turns: usize,
    pub paused: bool,
}

impl AutoFollowSettings {
    pub fn from_core(runtime: &ConversationRuntimeSnapshot) -> Self {
        Self {
            max_turns: runtime.auto_follow.max_auto_turns,
            paused: runtime.auto_follow.continuation_paused,
        }
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
    /// `rejected`, `queued`, `failed` 의 이유 코드이다. 클라이언트와 에이전트는 이 값으로 다음 행동을 고른다
    /// (plan 19 P3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PromptSubmissionReason>,
    /// 준비가 큐에 만들거나 고친 planning 작업이다. 이 값이 있으면 같은 프롬프트를 다시 보내지 않는다. 다시 보내면
    /// 작업이 하나 더 생긴다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
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
    /// 준비가 만든 작업이 큐에 남았고 턴은 시작하지 않았다. 이유는 `reason`, 작업은 `task_id` 이다.
    Queued,
    /// 받아들이지 않았다. 이유는 `reason` 이다. planning 작업은 만들어지지 않았다.
    Rejected,
    /// 준비가 턴을 만들지 못했다. planning 작업은 만들어지지 않았다.
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
    /// 병렬 모드가 켜져 있다. 병렬 모드가 꺼질 때까지 커널은 thread 에 프롬프트를 내지 않는다.
    ParallelMode,
    /// 커널이 병렬 모드 값을 아직 모른다. 잠시 뒤 다시 보낸다.
    ParallelModeUnknown,
    /// 준비하는 사이에 병렬 모드가 켜졌다 (`queued`). 작업은 병렬 slot 이 맡는다.
    ParallelModeStarted,
    /// 준비하는 사이에 다른 턴이 시작했다 (`queued`). 작업은 큐에 남는다. 다시 보내지 않는다. 커널이 쉬면 한 번
    /// 다시 내고 `submitted` 를 알린다 (plan 19 Q2-3).
    TurnStartedDuringPreparation,
    /// 정지 요청이 아직 끝나지 않았다 (`queued`). 커널이 쉬면 한 번 다시 낸다.
    StopPending,
    /// 그 thread 가 지금 프롬프트를 받을 수 없다 (`queued`). 커널이 쉬면 한 번 다시 낸다.
    Unavailable,
    /// 준비가 취소되었다.
    Cancelled,
    /// 준비 결과가 보낸 프롬프트와 맞지 않는다.
    TextMismatch,
    /// planning 초안을 사람이 검토해야 한다. 원격 클라이언트는 검토할 수 없다. `message` 가 초안 이름이다.
    BootstrapReviewRequired,
    /// planning 이 프롬프트를 받지 않았다. `message` 가 원인이다.
    PlanningRefused,
}

impl PromptSubmissionState {
    fn admitted(thread_id: &str, admission: &ThreadPromptAdmission) -> Self {
        let (reason, message) = match admission {
            ThreadPromptAdmission::Accepted { .. } => {
                return Self::progress(thread_id, PromptSubmissionStatus::Preparing);
            }
            ThreadPromptAdmission::RejectedBusy => (
                PromptSubmissionReason::Busy,
                "a turn is running or the thread is still busy",
            ),
            ThreadPromptAdmission::RejectedPreparationActive { .. } => (
                PromptSubmissionReason::PreparationActive,
                "another prompt is being prepared",
            ),
            ThreadPromptAdmission::RejectedUnknownThread => (
                PromptSubmissionReason::UnknownThread,
                "unknown thread; load_sessions lists the known threads",
            ),
            ThreadPromptAdmission::RejectedParallelMode => (
                PromptSubmissionReason::ParallelMode,
                "parallel mode is on; the kernel starts no thread prompt until it is off",
            ),
            ThreadPromptAdmission::RejectedParallelModeUnknown => (
                PromptSubmissionReason::ParallelModeUnknown,
                "the kernel does not know the parallel mode yet; try again",
            ),
        };
        Self::stopped(
            thread_id,
            PromptSubmissionStatus::Rejected,
            reason,
            None,
            message.to_string(),
        )
    }

    fn from_outcome(thread_id: &str, outcome: &ThreadPromptOutcome) -> Self {
        match outcome {
            ThreadPromptOutcome::Submitted { admission, task_id } => {
                let (reason, message) = match admission {
                    TurnSubmissionAdmission::Accepted { .. } => {
                        return Self {
                            task_id: Some(task_id.clone()),
                            ..Self::progress(thread_id, PromptSubmissionStatus::Submitted)
                        };
                    }
                    TurnSubmissionAdmission::RejectedActive { .. } => (
                        PromptSubmissionReason::TurnStartedDuringPreparation,
                        "another turn started while the prompt was prepared",
                    ),
                    TurnSubmissionAdmission::RejectedStopPending { .. } => (
                        PromptSubmissionReason::StopPending,
                        "a stop request is still settling",
                    ),
                    TurnSubmissionAdmission::RejectedUnavailable => (
                        PromptSubmissionReason::Unavailable,
                        "the thread cannot take a prompt now",
                    ),
                };
                Self::queued(thread_id, reason, task_id, message)
            }
            ThreadPromptOutcome::ParallelModeStarted { task_id } => Self::queued(
                thread_id,
                PromptSubmissionReason::ParallelModeStarted,
                task_id,
                "parallel mode started while the prompt was prepared",
            ),
            ThreadPromptOutcome::PreparationFailed(failure) => {
                let (reason, message) = match failure {
                    ThreadPromptPreparationFailure::Cancelled => (
                        PromptSubmissionReason::Cancelled,
                        "prompt preparation was cancelled".to_string(),
                    ),
                    ThreadPromptPreparationFailure::TextMismatch => (
                        PromptSubmissionReason::TextMismatch,
                        "prepared prompt does not match the submitted text".to_string(),
                    ),
                    ThreadPromptPreparationFailure::BootstrapReviewRequired { draft_name } => (
                        PromptSubmissionReason::BootstrapReviewRequired,
                        format!("planning bootstrap needs review / draft: {draft_name}"),
                    ),
                    ThreadPromptPreparationFailure::PlanningRefused { reason } => {
                        (PromptSubmissionReason::PlanningRefused, reason.clone())
                    }
                };
                Self::stopped(
                    thread_id,
                    PromptSubmissionStatus::Failed,
                    reason,
                    None,
                    message,
                )
            }
        }
    }

    fn progress(thread_id: &str, status: PromptSubmissionStatus) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            status,
            reason: None,
            task_id: None,
            message: None,
        }
    }

    fn queued(
        thread_id: &str,
        reason: PromptSubmissionReason,
        task_id: &str,
        situation: &str,
    ) -> Self {
        Self::stopped(
            thread_id,
            PromptSubmissionStatus::Queued,
            reason,
            Some(task_id.to_string()),
            format!("{situation}; planning task {task_id} stays queued, do not resend the prompt"),
        )
    }

    fn stopped(
        thread_id: &str,
        status: PromptSubmissionStatus,
        reason: PromptSubmissionReason,
        task_id: Option<String>,
        message: String,
    ) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            status,
            reason: Some(reason),
            task_id,
            message: Some(message),
        }
    }
}

// planning ----------------------------------------------------------------------------------------

/*
 * planning 큐이다 (K3-6). 커널 작업 공간(저장소의 대표 checkout)의 planning 투영을 보낸다. 커널이 아직 읽지 않았으면
 * `uninitialized` 이다. 클라이언트는 붙은 뒤 `refresh_planning` 을 보낸다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningState {
    pub status: PlanningStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_summary: Option<String>,
    /// `invalid` 의 원인이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// planning 이 자동 후속을 멈춘 이유이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_follow_pause_reason: Option<String>,
    /// 다음에 실행할 작업이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_task: Option<PlanningTask>,
    pub active_tasks: Vec<PlanningTask>,
    /// 아직 받아들이지 않은 후속 제안이다.
    pub proposed_tasks: Vec<PlanningTask>,
    pub skipped_tasks: Vec<SkippedPlanningTask>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningStatus {
    /// planning 이 없거나 커널이 아직 읽지 않았다.
    Uninitialized,
    Invalid,
    ReadyWithoutTask,
    ReadyWithTask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningTask {
    pub rank: usize,
    pub task_id: String,
    pub direction_title: String,
    pub title: String,
    pub status: PlanningTaskStatus,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedPlanningTask {
    pub task_id: String,
    pub title: String,
    pub status: PlanningTaskStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningTaskStatus {
    Ready,
    Blocked,
    InProgress,
    Done,
    Cancelled,
    AwaitingUser,
    Proposed,
}

impl PlanningTaskStatus {
    fn from_core(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Ready => Self::Ready,
            TaskStatus::Blocked => Self::Blocked,
            TaskStatus::InProgress => Self::InProgress,
            TaskStatus::Done => Self::Done,
            TaskStatus::Cancelled => Self::Cancelled,
            TaskStatus::AwaitingUser => Self::AwaitingUser,
            TaskStatus::Proposed => Self::Proposed,
        }
    }
}

impl PlanningTask {
    fn from_core(task: &PriorityQueueTask) -> Self {
        Self {
            rank: task.rank,
            task_id: task.task_id.clone(),
            direction_title: task.direction_title.clone(),
            title: task.task_title.clone(),
            status: PlanningTaskStatus::from_core(task.status),
            priority: task.combined_priority,
        }
    }
}

impl PlanningState {
    pub fn from_core(projection: &RuntimeProjection) -> Self {
        let queue = projection.queue_projection();
        let tasks = |pick: fn(
            &crate::domain::planning::PriorityQueueProjection,
        ) -> &Vec<PriorityQueueTask>| {
            queue
                .map(|queue| pick(queue).iter().map(PlanningTask::from_core).collect())
                .unwrap_or_default()
        };
        Self {
            status: match projection.workspace_status() {
                RuntimeWorkspaceStatus::Uninitialized => PlanningStatus::Uninitialized,
                RuntimeWorkspaceStatus::Invalid => PlanningStatus::Invalid,
                RuntimeWorkspaceStatus::ReadyNoTask => PlanningStatus::ReadyWithoutTask,
                RuntimeWorkspaceStatus::ReadyWithTask => PlanningStatus::ReadyWithTask,
            },
            revision: projection.planning_revision(),
            queue_summary: projection.queue_summary().map(str::to_string),
            failure_reason: projection.failure_reason().map(str::to_string),
            auto_follow_pause_reason: projection.auto_follow_pause_reason().map(str::to_string),
            next_task: queue
                .and_then(|queue| queue.next_task.as_ref())
                .or(projection.queue_head())
                .map(PlanningTask::from_core),
            active_tasks: tasks(|queue| &queue.active_tasks),
            proposed_tasks: tasks(|queue| &queue.proposed_tasks),
            skipped_tasks: queue
                .map(|queue| {
                    queue
                        .skipped_tasks
                        .iter()
                        .map(|task| SkippedPlanningTask {
                            task_id: task.task_id.clone(),
                            title: task.task_title.clone(),
                            status: PlanningTaskStatus::from_core(task.status),
                            reason: task.reason.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

// parallel ----------------------------------------------------------------------------------------

/// 커널 호스트가 모은 병렬 작업 상태이다. 호스트 타입을 모르게 하려고 값만 빌린다.
#[derive(Debug, Clone, Copy)]
pub struct ParallelSource<'a> {
    pub enabled: bool,
    pub supervisor: Option<&'a ParallelModeSupervisorSnapshot>,
    pub status: Option<&'a str>,
}

/*
 * 병렬 작업 상태이다 (K3-6). 감독 스냅샷은 TUI 의 병렬 화면과 같은 값이다. 상태 이름(`state`)은 TUI 와 같은 표시
 * 이름을 snake_case 로 바꾼 것이다. 명령 스펙(K4)에서 열거형으로 고정한다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelState {
    pub enabled: bool,
    /// 병렬 제어부가 마지막으로 알린 상태 문구이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// 커널이 아직 감독 스냅샷을 읽지 않았으면 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor: Option<ParallelSupervisor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelSupervisor {
    pub state: ParallelSupervisorState,
    pub pool: ParallelPool,
    pub agents: Vec<ParallelAgent>,
    /// 병합 큐의 머리 요약이다.
    pub merge_head: String,
    pub merge_queue: Vec<ParallelMergeItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_validation: Option<ParallelPrValidation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelSupervisorState {
    Prepare,
    Supervise,
    Recover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelPool {
    pub configured: usize,
    pub idle: usize,
    pub leased: usize,
    pub running: usize,
    pub awaiting_cleanup: usize,
    pub blocked: usize,
    pub missing: usize,
    pub unavailable: usize,
    pub exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelAgent {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub task_title: String,
    pub slot_id: String,
    pub branch: String,
    pub state: String,
    pub duration: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelMergeItem {
    pub task_title: String,
    pub state: String,
    pub branch: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelPrValidation {
    pub pull_request_number: u64,
    pub url: String,
    pub state: String,
    pub phase: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

fn state_name(label: &str) -> String {
    label.replace(' ', "_")
}

impl ParallelState {
    pub fn from_core(source: ParallelSource<'_>) -> Self {
        Self {
            enabled: source.enabled,
            status: source.status.map(str::to_string),
            supervisor: source.supervisor.map(ParallelSupervisor::from_core),
        }
    }
}

impl ParallelSupervisor {
    fn from_core(snapshot: &ParallelModeSupervisorSnapshot) -> Self {
        let pool = &snapshot.pool;
        Self {
            state: match snapshot.state {
                ParallelModeSupervisorState::Prepare => ParallelSupervisorState::Prepare,
                ParallelModeSupervisorState::Supervise => ParallelSupervisorState::Supervise,
                ParallelModeSupervisorState::Recover => ParallelSupervisorState::Recover,
            },
            pool: ParallelPool {
                configured: pool.configured_size,
                idle: pool.idle_slots,
                leased: pool.leased_slots,
                running: pool.running_slots,
                awaiting_cleanup: pool.awaiting_cleanup_slots,
                blocked: pool.blocked_slots,
                missing: pool.missing_slots,
                unavailable: pool.unavailable_slots,
                exhausted: pool.exhausted,
            },
            agents: snapshot
                .roster
                .entries
                .iter()
                .map(|entry| ParallelAgent {
                    agent_id: entry.agent_id.clone(),
                    thread_id: entry.thread_id.clone(),
                    task_title: entry.task_title.clone(),
                    slot_id: entry.slot_id.clone(),
                    branch: entry.branch_name.clone(),
                    state: entry.state_label.clone(),
                    duration: entry.duration_label.clone(),
                    summary: entry.latest_summary.clone(),
                })
                .collect(),
            merge_head: snapshot.distributor.head_summary.clone(),
            merge_queue: snapshot
                .distributor
                .queue_items
                .iter()
                .map(|item| ParallelMergeItem {
                    task_title: item.task_title.clone(),
                    state: state_name(item.queue_state.label()),
                    branch: item.branch_name.clone(),
                    commit: item.commit_short_sha.clone(),
                })
                .collect(),
            pr_validation: snapshot
                .pr_validation
                .as_ref()
                .map(|summary| ParallelPrValidation {
                    pull_request_number: summary.pull_request_number,
                    url: summary.canonical_pr_url.clone(),
                    state: state_name(summary.state.label()),
                    phase: summary.phase_label().to_string(),
                    reason: summary.reason.clone(),
                    blocker: summary.blocker.clone(),
                }),
            notice: snapshot.top_notice.clone(),
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
