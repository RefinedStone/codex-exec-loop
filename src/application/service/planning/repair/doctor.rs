use crate::domain::text::compact_whitespace_detail;

use crate::application::service::planning::runtime::prompt::PlanningPromptService;
use crate::application::service::planning::runtime::prompt::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};

const INCOMPLETE_PREFIX: &str = "planning files incomplete:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningDoctorState {
    Absent,
    Incomplete,
    Invalid,
    ReadyWithoutTask,
    ReadyWithTask,
}

impl PlanningDoctorState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Incomplete => "incomplete",
            Self::Invalid => "invalid",
            Self::ReadyWithoutTask => "ready_without_task",
            Self::ReadyWithTask => "ready_with_task",
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Self::Absent | Self::ReadyWithoutTask | Self::ReadyWithTask => 0,
            Self::Incomplete | Self::Invalid => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

fn doctor_proposal_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    snapshot
        .queue_snapshot()
        .and_then(|queue_snapshot| queue_snapshot.proposed_tasks.first())
        .map(|task| compact_whitespace_detail(task.task_title.trim(), 80))
        .or_else(|| snapshot.proposal_summary().map(str::to_string))
}

#[derive(Clone)]
pub struct PlanningDoctorService {
    planning_prompt_service: PlanningPromptService,
}

impl PlanningDoctorService {
    pub fn new(planning_prompt_service: PlanningPromptService) -> Self {
        Self {
            planning_prompt_service,
        }
    }

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
mod tests {
    use std::sync::Arc;

    use super::{PlanningDoctorService, PlanningDoctorState};
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning::runtime::prompt::PlanningPromptService;
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;

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
    fn inspect_workspace_reports_absent_state_for_uninitialized_workspace() {
        let workspace_dir = create_temp_workspace("planning-doctor-absent");

        let report = doctor_service().inspect_workspace(&workspace_dir);

        assert_eq!(report.planning_state(), PlanningDoctorState::Absent);
        assert_eq!(
            report.health(),
            Some("planning workspace is not initialized")
        );
        assert_eq!(report.exit_code(), 0);

        std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
