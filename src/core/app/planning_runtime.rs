use super::PlanningRuntimeRefreshCorrelation;
use crate::domain::planning::{RuntimeProjection, RuntimeWorkspaceStatus};
use crate::domain::text::compact_whitespace_detail;

const INCOMPLETE_PREFIX: &str = "planning files incomplete:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningDoctorSnapshotState {
    Absent,
    Incomplete,
    Invalid,
    ReadyWithoutTask,
    ReadyWithTask,
}

impl PlanningDoctorSnapshotState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Incomplete => "incomplete",
            Self::Invalid => "invalid",
            Self::ReadyWithoutTask => "ready_without_task",
            Self::ReadyWithTask => "ready_with_task",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDoctorSnapshot {
    planning_state: PlanningDoctorSnapshotState,
    queue_idle_policy: Option<String>,
    queue_summary: Option<String>,
    proposal_summary: Option<String>,
    health: Option<String>,
    issue: Option<String>,
    note: Option<String>,
}

impl PlanningDoctorSnapshot {
    pub(crate) fn from_runtime_projection(projection: &RuntimeProjection) -> Self {
        let planning_state = match projection.workspace_status() {
            RuntimeWorkspaceStatus::Uninitialized => PlanningDoctorSnapshotState::Absent,
            RuntimeWorkspaceStatus::Invalid
                if projection
                    .failure_reason()
                    .is_some_and(|reason| reason.starts_with(INCOMPLETE_PREFIX)) =>
            {
                PlanningDoctorSnapshotState::Incomplete
            }
            RuntimeWorkspaceStatus::Invalid => PlanningDoctorSnapshotState::Invalid,
            RuntimeWorkspaceStatus::ReadyNoTask => PlanningDoctorSnapshotState::ReadyWithoutTask,
            RuntimeWorkspaceStatus::ReadyWithTask => PlanningDoctorSnapshotState::ReadyWithTask,
        };
        let is_ready = matches!(
            planning_state,
            PlanningDoctorSnapshotState::ReadyWithoutTask
                | PlanningDoctorSnapshotState::ReadyWithTask
        );
        Self {
            planning_state,
            queue_idle_policy: is_ready.then(|| projection.queue_idle_policy().label().to_string()),
            queue_summary: is_ready
                .then(|| {
                    projection
                        .queue_head()
                        .map(|queue_head| {
                            format!(
                                "now: {}",
                                compact_whitespace_detail(queue_head.task_title.trim(), 80)
                            )
                        })
                        .or_else(|| projection.queue_summary().map(str::to_string))
                })
                .flatten(),
            proposal_summary: is_ready
                .then(|| {
                    projection
                        .queue_projection()
                        .and_then(|queue_projection| queue_projection.proposed_tasks.first())
                        .map(|task| compact_whitespace_detail(task.task_title.trim(), 80))
                        .or_else(|| projection.proposal_summary().map(str::to_string))
                })
                .flatten(),
            health: match planning_state {
                PlanningDoctorSnapshotState::Absent => {
                    Some("planning workspace is not initialized".to_string())
                }
                PlanningDoctorSnapshotState::ReadyWithoutTask
                | PlanningDoctorSnapshotState::ReadyWithTask => {
                    Some("planning workspace is healthy".to_string())
                }
                PlanningDoctorSnapshotState::Incomplete | PlanningDoctorSnapshotState::Invalid => {
                    None
                }
            },
            issue: matches!(
                planning_state,
                PlanningDoctorSnapshotState::Incomplete | PlanningDoctorSnapshotState::Invalid
            )
            .then(|| projection.failure_reason().map(str::to_string))
            .flatten(),
            note: None,
        }
    }

    pub const fn planning_state(&self) -> PlanningDoctorSnapshotState {
        self.planning_state
    }

    pub const fn workspace_present(&self) -> bool {
        !matches!(self.planning_state, PlanningDoctorSnapshotState::Absent)
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeRefreshSnapshot {
    pub runtime_projection: Box<RuntimeProjection>,
    pub doctor: PlanningDoctorSnapshot,
}

impl PlanningRuntimeRefreshSnapshot {
    pub fn new(runtime_projection: RuntimeProjection) -> Self {
        let doctor = PlanningDoctorSnapshot::from_runtime_projection(&runtime_projection);
        Self {
            runtime_projection: Box::new(runtime_projection),
            doctor,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlanningRuntimeCoordinator {
    next_generation: u64,
    active: Option<PlanningRuntimeRefreshCorrelation>,
}

impl PlanningRuntimeCoordinator {
    pub(super) fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
        }
    }

    pub(super) fn begin(
        &mut self,
        workspace_directory: String,
    ) -> (
        PlanningRuntimeRefreshCorrelation,
        Option<PlanningRuntimeRefreshCorrelation>,
    ) {
        let correlation =
            PlanningRuntimeRefreshCorrelation::new(self.take_generation(), workspace_directory);
        let superseded = self.active.replace(correlation.clone());
        (correlation, superseded)
    }

    pub(super) fn cancel(&mut self) -> Option<PlanningRuntimeRefreshCorrelation> {
        self.active.take()
    }

    pub(super) fn accept(&mut self, correlation: &PlanningRuntimeRefreshCorrelation) -> bool {
        if self.active.as_ref() != Some(correlation) {
            return false;
        }
        self.active = None;
        true
    }

    pub(super) fn has_active(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn matches_workspace(&self, workspace_directory: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|correlation| correlation.workspace_directory == workspace_directory)
    }

    pub(super) fn restart_if_matches(
        &mut self,
        workspace_directory: &str,
    ) -> Option<(
        PlanningRuntimeRefreshCorrelation,
        PlanningRuntimeRefreshCorrelation,
    )> {
        if !self.matches_workspace(workspace_directory) {
            return None;
        }
        let (replacement, superseded) = self.begin(workspace_directory.to_string());
        Some((
            replacement,
            superseded.expect("matching planning runtime refresh must remain active"),
        ))
    }

    fn take_generation(&mut self) -> u64 {
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .expect("planning runtime refresh generation exhausted");
        generation
    }

    #[cfg(test)]
    pub(super) fn exhaust_generation(&mut self) {
        self.next_generation = u64::MAX;
    }
}

impl Default for PlanningRuntimeCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanningDoctorSnapshot, PlanningDoctorSnapshotState};
    use crate::domain::planning::{PriorityQueueTask, RuntimeProjection, TaskStatus};

    #[test]
    fn doctor_snapshot_maps_every_runtime_projection_state() {
        let cases = [
            (
                RuntimeProjection::uninitialized(),
                PlanningDoctorSnapshotState::Absent,
                Some("planning workspace is not initialized"),
                None,
            ),
            (
                RuntimeProjection::invalid("planning files incomplete: task file missing"),
                PlanningDoctorSnapshotState::Incomplete,
                None,
                Some("planning files incomplete: task file missing"),
            ),
            (
                RuntimeProjection::invalid("task authority is invalid"),
                PlanningDoctorSnapshotState::Invalid,
                None,
                Some("task authority is invalid"),
            ),
            (
                RuntimeProjection::ready("prompt".to_string(), "idle".to_string(), None),
                PlanningDoctorSnapshotState::ReadyWithoutTask,
                Some("planning workspace is healthy"),
                None,
            ),
            (
                RuntimeProjection::ready(
                    "prompt".to_string(),
                    "ready".to_string(),
                    Some(PriorityQueueTask {
                        rank: 1,
                        task_id: "task-1".to_string(),
                        direction_id: "direction-1".to_string(),
                        direction_title: "Direction 1".to_string(),
                        task_title: "Run the next task".to_string(),
                        status: TaskStatus::Ready,
                        combined_priority: 10,
                        updated_at: "2026-07-19T00:00:00Z".to_string(),
                        rank_reasons: Vec::new(),
                    }),
                ),
                PlanningDoctorSnapshotState::ReadyWithTask,
                Some("planning workspace is healthy"),
                None,
            ),
        ];

        for (projection, state, health, issue) in cases {
            let snapshot = PlanningDoctorSnapshot::from_runtime_projection(&projection);
            assert_eq!(snapshot.planning_state(), state);
            assert_eq!(snapshot.health(), health);
            assert_eq!(snapshot.issue(), issue);
        }
    }

    #[test]
    fn doctor_snapshot_state_labels_are_stable() {
        let cases = [
            (PlanningDoctorSnapshotState::Absent, "absent"),
            (PlanningDoctorSnapshotState::Incomplete, "incomplete"),
            (PlanningDoctorSnapshotState::Invalid, "invalid"),
            (
                PlanningDoctorSnapshotState::ReadyWithoutTask,
                "ready_without_task",
            ),
            (
                PlanningDoctorSnapshotState::ReadyWithTask,
                "ready_with_task",
            ),
        ];

        for (state, label) in cases {
            assert_eq!(state.label(), label);
        }
    }
}
