/// StoredCommandContext의 확인 증거와 준비 조회 metadata를 이 포트가 소유한다.
pub mod issue_confirmation;

use anyhow::Result;

use crate::domain::issue_automation::autonomy::{IssueAutonomyLevel, IssueAutonomyProjection};
use crate::domain::issue_automation::collection::{IssueCollectionMode, IssueCollectionProjection};
use crate::domain::issue_automation::decision::{IssueDecisionFact, IssueDecisionOutcome};
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest,
};

use crate::domain::kernel_spec::{KernelCommandKind, ReasonCode};
use crate::domain::planning::{
    DirectionState, PlanningMutationRefusal, TaskAcceptanceCriterion, TaskStatus,
};

/*
 * 저장 명령의 실행기이다 (plan 18 K4-9d, 설계 `docs/plan/18-k4-9d-planning-mutations.md` §3.1).
 *
 * 저장 명령은 저장된 저장소 데이터만 바꾼다. 도는 커널의 상태가 필요 없다. 그래서 저장소 잠금을 쥔 쪽이 이
 * 실행기를 부른다: Gateway 의 커널 런타임, 명령 전용 조립, 또는 TUI 가 커널을 쥔 저장소의 직접 경로이다. 실행은
 * 동기이고, 돌아올 때 변경은 저장되어 있다. 커널 시뮬레이션은 가짜 실행기를 끼운다.
 *
 * 뜻은 사용자(operator)의 변경이다: 고치기는 작업의 필드 전체를 바꾸고, 출처는 사용자이고, 변경 guard 를 잡는다.
 * worker 의 부분 고치기는 이 실행기를 쓰지 않는다.
 */
pub trait StoredCommandPort: Send + Sync {
    fn execute(&self, command: StoredCommand) -> Result<StoredCommandOutcome>;

    /// 전송 계층이 정한 호출 문맥이다. caller의 자기 소개는 사람 확인 증명이 아니다.
    fn execute_with_context(
        &self,
        command: StoredCommand,
        _context: &StoredCommandContext,
    ) -> Result<StoredCommandOutcome> {
        self.execute(command)
    }
}

/// 저장 명령을 실제로 받은 서버가 정한다. 클라이언트 명령 payload에서 받지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCommandContext {
    pub human_confirmation: Option<issue_confirmation::VerifiedHumanConfirmation>,
    pub caller: Option<String>,
    pub channel: String,
    pub at: String,
}

/*
 * 작업의 필드이다. 만들기에서 없는 값은 기본값이다. 고치기는 admin 의 폼과 같다: 이유와 참조는 없으면 비우고, 방향,
 * 설명, 상태, 우선순위, 완료 기준은 없으면 그대로 둔다 (`update_input`). 완료 기준의 빈 배열은 명시적 비우기다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTaskFields {
    pub direction_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub acceptance: Option<Vec<TaskAcceptanceCriterion>>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Vec<String>,
    pub blocked_by: Vec<String>,
}

/*
 * 방향의 필드이다. 만들기와 고치기가 같은 뜻으로 읽는다 (admin 의 폼과 같다): 요약이 없으면 제목을 쓰고, 상태가
 * 없으면 `active` 이고, 완료 기준은 하나 이상이어야 한다. 고치기는 방향 전체를 이 값으로 바꾼다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDirectionFields {
    pub title: String,
    pub summary: Option<String>,
    pub success_criteria: Vec<String>,
    pub scope_hints: Vec<String>,
    pub detail_doc_path: Option<String>,
    pub state: Option<DirectionState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCommand {
    InterveneIssue {
        request: IssueInterventionRequest,
    },
    AnswerIssueDecision {
        decision_id: String,
        option: String,
        note: Option<String>,
    },
    SetIssueAutomationMode {
        mode: IssueCollectionMode,
    },
    SetIssueAutonomy {
        command_id: String,
        level: IssueAutonomyLevel,
    },
    CreateTask {
        task: StoredTaskFields,
    },
    UpdateTask {
        task_id: String,
        task: StoredTaskFields,
        expected_planning_revision: i64,
    },
    DeleteTask {
        task_id: String,
        expected_planning_revision: i64,
    },
    CreateDirection {
        direction: StoredDirectionFields,
    },
    UpdateDirection {
        direction_id: String,
        direction: StoredDirectionFields,
        expected_planning_revision: i64,
    },
    /// 방향과 그 방향의 작업을 지운다. 기본 방향은 지우지 않고 남긴다.
    DeleteDirection {
        direction_id: String,
        expected_planning_revision: i64,
    },
}

impl StoredCommand {
    /// 이 저장 명령의 스펙 항목이다.
    pub const fn kind(&self) -> KernelCommandKind {
        match self {
            Self::InterveneIssue { request } => match &request.action {
                IssueInterventionAction::Hold => KernelCommandKind::HoldIssue,
                IssueInterventionAction::Release => KernelCommandKind::ReleaseIssue,
                IssueInterventionAction::Steer { .. } => KernelCommandKind::SteerIssue,
                IssueInterventionAction::TakeOver => KernelCommandKind::TakeOverIssue,
                IssueInterventionAction::Retriage => KernelCommandKind::RetriageIssue,
                IssueInterventionAction::Cancel { .. } => KernelCommandKind::CancelIssueWork,
                IssueInterventionAction::Track => KernelCommandKind::TrackIssue,
                IssueInterventionAction::Untrack => KernelCommandKind::UntrackIssue,
            },
            Self::SetIssueAutomationMode { .. } => KernelCommandKind::SetIssueAutomationMode,
            Self::SetIssueAutonomy { .. } => KernelCommandKind::SetIssueAutonomy,
            Self::AnswerIssueDecision { .. } => KernelCommandKind::AnswerIssueDecision,
            Self::CreateTask { .. } => KernelCommandKind::CreateTask,
            Self::UpdateTask { .. } => KernelCommandKind::UpdateTask,
            Self::DeleteTask { .. } => KernelCommandKind::DeleteTask,
            Self::CreateDirection { .. } => KernelCommandKind::CreateDirection,
            Self::UpdateDirection { .. } => KernelCommandKind::UpdateDirection,
            Self::DeleteDirection { .. } => KernelCommandKind::DeleteDirection,
        }
    }
}

/// 성공한 저장 명령의 결과이다. 명령의 답에 실린다. 기록에는 싣지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCommandResult {
    IssueIntervened {
        receipt: Box<IssueInterventionReceipt>,
    },
    IssueDecisionResolved {
        fact: Box<IssueDecisionFact>,
        outcome: IssueDecisionOutcome,
        changed: bool,
        catalog_revision: u64,
    },
    IssueAutomationModeChanged {
        previous: Option<IssueCollectionMode>,
        projection: Box<IssueCollectionProjection>,
        changed: bool,
    },
    IssueAutonomyChanged {
        command_id: String,
        changed: bool,
        autonomy: Box<IssueAutonomyProjection>,
    },
    TaskCreated {
        task_id: String,
        planning_revision: i64,
    },
    TaskUpdated {
        task_id: String,
        planning_revision: i64,
    },
    TaskDeleted {
        task_id: String,
        planning_revision: i64,
    },
    DirectionCreated {
        direction_id: String,
        planning_revision: i64,
    },
    DirectionUpdated {
        direction_id: String,
        planning_revision: i64,
    },
    /// `default_retained` 가 참이면 기본 방향이라서 지우지 않았다.
    DirectionDeleted {
        direction_id: String,
        removed_task_count: usize,
        default_retained: bool,
        planning_revision: i64,
    },
}

/// 저장 명령이 아무것도 바꾸지 않고 거절되었다. 이유 코드와 사람이 읽는 글이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCommandRejection {
    pub reason: ReasonCode,
    pub message: String,
    /// 거절할 때 저장소에서 읽은 planning revision 이다. 낡은 revision 의 거절에만 있다. 커널은 자기 planning 이 이
    /// 값에 닿은 뒤에 답한다 (plan 18 K4-9d).
    pub planning_revision: Option<i64>,
}

impl From<&PlanningMutationRefusal> for StoredCommandRejection {
    fn from(refusal: &PlanningMutationRefusal) -> Self {
        Self {
            reason: match refusal {
                PlanningMutationRefusal::RevisionStale { .. } => ReasonCode::PlanningRevisionStale,
                PlanningMutationRefusal::Locked { .. } => ReasonCode::PlanningLocked,
            },
            message: refusal.message().to_string(),
            planning_revision: match refusal {
                PlanningMutationRefusal::RevisionStale {
                    current_revision, ..
                } => Some(*current_revision),
                PlanningMutationRefusal::Locked { .. } => None,
            },
        }
    }
}

/// 실행의 끝이다. 실패(그 밖의 오류)는 `execute` 의 `Err` 이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCommandOutcome {
    Done(StoredCommandResult),
    Rejected(StoredCommandRejection),
}
