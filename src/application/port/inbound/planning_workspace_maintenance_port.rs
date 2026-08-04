use crate::domain::planning::PlanningResetTarget;

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
        Self::from_parts(
            PlanningDoctorState::Invalid,
            None,
            None,
            None,
            None,
            Some(issue),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        planning_state: PlanningDoctorState,
        queue_idle_policy: Option<String>,
        queue_summary: Option<String>,
        proposal_summary: Option<String>,
        health: Option<String>,
        issue: Option<String>,
        note: Option<String>,
    ) -> Self {
        Self {
            planning_state,
            queue_idle_policy,
            queue_summary,
            proposal_summary,
            health,
            issue,
            note,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceResetResult {
    pub target: PlanningResetTarget,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

pub trait PlanningWorkspaceMaintenancePort: Send + Sync {
    fn inspect_workspace(&self, workspace_dir: &str) -> PlanningDoctorReport;

    fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> anyhow::Result<PlanningWorkspaceResetResult>;
}
