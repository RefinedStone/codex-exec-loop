use anyhow::Result;

use crate::domain::kernel_spec::{KernelCommandKind, ReasonCode};
use crate::domain::planning::{PlanningMutationRefusal, TaskStatus};

/*
 * 저장 명령의 실행기이다 (plan 18 K4-9d, 설계 `docs/plan/18-k4-9d-planning-mutations.md` §3.1).
 *
 * 저장 명령은 저장된 planning 데이터만 바꾼다. 도는 커널의 상태가 필요 없다. 그래서 저장소 잠금을 쥔 쪽이 이
 * 실행기를 부른다: Gateway 의 커널 런타임, 명령 전용 조립, 또는 TUI 가 커널을 쥔 저장소의 직접 경로이다. 실행은
 * 동기이고, 돌아올 때 변경은 저장되어 있다. 커널 시뮬레이션은 가짜 실행기를 끼운다.
 *
 * 뜻은 사용자(operator)의 변경이다: 고치기는 작업의 필드 전체를 바꾸고, 출처는 사용자이고, 변경 guard 를 잡는다.
 * worker 의 부분 고치기는 이 실행기를 쓰지 않는다.
 */
pub trait StoredCommandPort: Send + Sync {
    fn execute(&self, command: StoredCommand) -> Result<StoredCommandOutcome>;
}

/// 작업의 필드이다. 만들기에서 없는 값은 기본값이고, 고치기에서 없는 값은 "비운다"이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTaskFields {
    pub direction_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Vec<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCommand {
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
}

impl StoredCommand {
    /// 이 저장 명령의 스펙 항목이다.
    pub const fn kind(&self) -> KernelCommandKind {
        match self {
            Self::CreateTask { .. } => KernelCommandKind::CreateTask,
            Self::UpdateTask { .. } => KernelCommandKind::UpdateTask,
            Self::DeleteTask { .. } => KernelCommandKind::DeleteTask,
        }
    }
}

/// 성공한 저장 명령의 결과이다. 명령의 답에 실린다. 기록에는 싣지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCommandResult {
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
}

/// 저장 명령이 아무것도 바꾸지 않고 거절되었다. 이유 코드와 사람이 읽는 글이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCommandRejection {
    pub reason: ReasonCode,
    pub message: String,
}

impl From<&PlanningMutationRefusal> for StoredCommandRejection {
    fn from(refusal: &PlanningMutationRefusal) -> Self {
        Self {
            reason: match refusal {
                PlanningMutationRefusal::RevisionStale { .. } => ReasonCode::PlanningRevisionStale,
                PlanningMutationRefusal::Locked { .. } => ReasonCode::PlanningLocked,
            },
            message: refusal.message().to_string(),
        }
    }
}

/// 실행의 끝이다. 실패(그 밖의 오류)는 `execute` 의 `Err` 이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCommandOutcome {
    Done(StoredCommandResult),
    Rejected(StoredCommandRejection),
}
