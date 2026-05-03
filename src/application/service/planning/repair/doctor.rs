/*
 * Planning doctor는 planning authority 상태를 읽기 전용으로 진단하는 표면이다.
 * runtime prompt loading은 이미 누락 authority seed, active planning file 검증, queue projection 생성을
 * 알고 있다. 이 모듈은 그 풍부한 runtime snapshot을 CLI/TUI caller가 표시하고 exit-code 판단에
 * 사용할 수 있는 compact report로 낮춘다.
 */
use crate::application::service::planning::runtime::prompt::PlanningPromptService;
use crate::application::service::planning::runtime::prompt::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::text::compact_whitespace_detail;

// runtime validation은 현재 필수 planning file 누락을 이 prefix로 표시한다.
const INCOMPLETE_PREFIX: &str = "planning files incomplete:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * doctor state는 runtime status를 1:1로 노출하지 않는 operator-facing projection이다.
 * 두 ready 상태는 모두 성공 exit이지만, 분리해 두면 UI가 healthy idle queue와 곧 실행할
 * 구체적 task가 있는 healthy workspace를 구분할 수 있다.
 */
pub enum PlanningDoctorState {
    Absent,
    Incomplete,
    Invalid,
    ReadyWithoutTask,
    ReadyWithTask,
}
impl PlanningDoctorState {
    // label은 CLI/API presentation layer가 쓰는 stable 외부 문자열이다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Incomplete => "incomplete",
            Self::Invalid => "invalid",
            Self::ReadyWithoutTask => "ready_without_task",
            Self::ReadyWithTask => "ready_with_task",
        }
    }

    // prompt loading이 inspection 중 기본 authority를 초기화할 수 있으므로 absence는 error가 아니다.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Absent | Self::ReadyWithoutTask | Self::ReadyWithTask => 0,
            Self::Incomplete | Self::Invalid => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * inbound adapter로 반환되는 compact report 객체다.
 * field를 private으로 두어 presentation code가 accessor를 거치게 한다. 그래야 runtime snapshot field와
 * doctor 전용 display fallback 사이의 내부 구분에 adapter가 우연히 의존하지 않는다.
 */
pub struct PlanningDoctorReport {
    planning_state: PlanningDoctorState,
    queue_idle_policy: Option<String>,
    queue_summary: Option<String>,
    proposal_summary: Option<String>,
    health: Option<String>,
    issue: Option<String>,
    note: Option<String>,
}
impl PlanningDoctorReport {
    // runtime snapshot loading이 report를 만들기 전에 caller가 workspace path를 거절했을 때 쓴다.
    pub fn path_issue(issue: String) -> Self {
        Self {
            planning_state: PlanningDoctorState::Invalid,
            queue_idle_policy: None,
            queue_summary: None,
            proposal_summary: None,
            health: None,
            issue: Some(issue),
            note: None,
        }
    }

    pub fn planning_state(&self) -> PlanningDoctorState {
        self.planning_state
    }
    pub fn queue_idle_policy(&self) -> Option<&str> {
        self.queue_idle_policy.as_deref()
    }
    pub fn queue_summary(&self) -> Option<&str> {
        self.queue_summary.as_deref()
    }
    pub fn proposal_summary(&self) -> Option<&str> {
        self.proposal_summary.as_deref()
    }
    pub fn health(&self) -> Option<&str> {
        self.health.as_deref()
    }
    pub fn issue(&self) -> Option<&str> {
        self.issue.as_deref()
    }
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
    pub fn exit_code(&self) -> i32 {
        self.planning_state.exit_code()
    }

    /*
     * runtime snapshot을 doctor report 계약으로 projection한다.
     * ready snapshot은 queue policy와 summary를 노출하고, incomplete/invalid snapshot은 queue detail을 숨긴 뒤
     * runtime failure reason을 actionable issue로 보존한다.
     */
    fn from_snapshot(snapshot: &PlanningRuntimeSnapshot) -> Self {
        let planning_state = classify_doctor_state(snapshot);
        let is_ready = matches!(
            planning_state,
            PlanningDoctorState::ReadyWithoutTask | PlanningDoctorState::ReadyWithTask
        );
        let note = None;
        let health = match planning_state {
            PlanningDoctorState::Absent => {
                Some("planning workspace is not initialized".to_string())
            }
            PlanningDoctorState::ReadyWithoutTask | PlanningDoctorState::ReadyWithTask => {
                Some("planning workspace is healthy".to_string())
            }
            PlanningDoctorState::Incomplete | PlanningDoctorState::Invalid => None,
        };

        Self {
            planning_state,
            queue_idle_policy: is_ready.then(|| snapshot.queue_idle_policy().label().to_string()),
            queue_summary: is_ready.then(|| doctor_queue_summary(snapshot)).flatten(),
            proposal_summary: is_ready
                .then(|| doctor_proposal_summary(snapshot))
                .flatten(),
            health,
            issue: matches!(
                planning_state,
                PlanningDoctorState::Incomplete | PlanningDoctorState::Invalid
            )
            .then(|| snapshot.failure_reason().map(str::to_string))
            .flatten(),
            note,
        }
    }
}

// 구체적인 active queue head를 우선하고, head가 없으면 snapshot의 aggregate queue copy로 후퇴한다.
fn doctor_queue_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    snapshot
        .queue_head()
        .map(|queue_head| {
            format!(
                "now: {}",
                compact_whitespace_detail(queue_head.task_title.trim(), 80)
            )
        })
        .or_else(|| snapshot.queue_summary().map(str::to_string))
}

// proposed task summary는 queue summary처럼 generic projection text보다 첫 proposed task title을 먼저 보여 준다.
fn doctor_proposal_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    snapshot
        .queue_projection()
        .and_then(|queue_projection| queue_projection.proposed_tasks.first())
        .map(|task| compact_whitespace_detail(task.task_title.trim(), 80))
        .or_else(|| snapshot.proposal_summary().map(str::to_string))
}

#[derive(Clone)]
// service wrapper는 doctor inspection이 worker prompt assembly와 같은 runtime prompt loader path를 타게 한다.
pub struct PlanningDoctorService {
    planning_prompt_service: PlanningPromptService,
}
impl PlanningDoctorService {
    // composition이 prompt service를 주입해 doctor와 worker runtime snapshot이 서로 어긋나지 않게 한다.
    pub fn new(planning_prompt_service: PlanningPromptService) -> Self {
        Self {
            planning_prompt_service,
        }
    }

    /*
     * runtime snapshot을 load해 workspace를 inspect하고, loader failure는 invalid report로 낮춘다.
     * 이렇게 하면 CLI caller는 total function을 호출하게 되고, path/IO 문제는 panic이나 부분 formatting error가
     * 아니라 report data가 된다.
     */
    pub fn inspect_workspace(&self, workspace_dir: &str) -> PlanningDoctorReport {
        let snapshot = self
            .planning_prompt_service
            .load_runtime_snapshot(workspace_dir)
            .unwrap_or_else(|error| {
                PlanningRuntimeSnapshot::invalid(format!(
                    "failed to load planning workspace: {error}"
                ))
            });
        PlanningDoctorReport::from_snapshot(&snapshot)
    }
}

// runtime status는 둘 다 Invalid로만 노출하므로, validation prefix로 incomplete와 invalid를 나눈다.
fn classify_doctor_state(snapshot: &PlanningRuntimeSnapshot) -> PlanningDoctorState {
    match snapshot.workspace_status() {
        PlanningRuntimeWorkspaceStatus::Uninitialized => PlanningDoctorState::Absent,
        PlanningRuntimeWorkspaceStatus::Invalid => {
            if snapshot
                .failure_reason()
                .is_some_and(|reason| reason.starts_with(INCOMPLETE_PREFIX))
            {
                PlanningDoctorState::Incomplete
            } else {
                PlanningDoctorState::Invalid
            }
        }
        PlanningRuntimeWorkspaceStatus::ReadyNoTask => PlanningDoctorState::ReadyWithoutTask,
        PlanningRuntimeWorkspaceStatus::ReadyWithTask => PlanningDoctorState::ReadyWithTask,
    }
}

#[cfg(test)]
// snapshot loading이 기본 planning authority를 seed할 수 있으므로, test는 doctor service 경계를 검증한다.
mod tests {
    use std::sync::Arc;

    use super::{PlanningDoctorService, PlanningDoctorState};
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning::runtime::prompt::PlanningPromptService;
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::domain::planning::PriorityQueueService;

    // temp workspace helper는 runtime bootstrap-through-inspection 동작을 검증하려고 일부러 빈 상태로 시작한다.
    fn create_temp_workspace(label: &str) -> String {
        let unique = format!(
            "{}-{}",
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("temp workspace directory should be created");
        path.to_string_lossy().into_owned()
    }

    // 실제 service stack을 만들어 filesystem loading과 runtime validation을 함께 검증한다.
    fn doctor_service() -> PlanningDoctorService {
        let workspace_port = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let validation_service = PlanningValidationService::new();
        let prompt_service = PlanningPromptService::new(
            workspace_port,
            validation_service,
            PriorityQueueService::new(),
        );
        PlanningDoctorService::new(prompt_service)
    }

    #[test]
    /*
     * 초기화되지 않은 workspace를 inspect하면 runtime prompt service가 기본 DB authority를 seed한다.
     * 그래서 doctor는 raw Uninitialized status 대신 ready task가 없는 healthy workspace로 보고한다.
     */
    fn inspect_workspace_seeds_default_authority_for_uninitialized_workspace() {
        let workspace_dir = create_temp_workspace("planning-doctor-absent");
        let report = doctor_service().inspect_workspace(&workspace_dir);

        assert_eq!(
            report.planning_state(),
            PlanningDoctorState::ReadyWithoutTask
        );
        assert_eq!(report.health(), Some("planning workspace is healthy"));
        assert_eq!(report.exit_code(), 0);
        std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
