use crate::application::service::planning_prompt_service::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::followup_template::FollowupTemplateDefinition;
use crate::domain::planning::PlanningWorkspaceState;

const BUILTIN_NEXT_TASK_TEMPLATE_ID: &str = "builtin-next-task";

#[derive(Debug, Clone, Default)]
pub struct PlanningRuntimePolicyService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowBlockReason {
    InvalidWorkspace,
    ActionableQueueRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeSummaryView {
    pub workspace_state: PlanningWorkspaceState,
    pub status_label: &'static str,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub failure_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimePreviewView {
    pub status_label: &'static str,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRuntimeSummaryRequest<'a> {
    pub snapshot: &'a PlanningRuntimeSnapshot,
    pub has_running_turn: bool,
    pub is_repairing: bool,
    pub repair_failure_summary: Option<&'a str>,
}

impl PlanningRuntimePolicyService {
    pub fn new() -> Self {
        Self
    }

    pub fn auto_follow_block_reason(
        &self,
        template: &FollowupTemplateDefinition,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<PlanningAutoFollowBlockReason> {
        if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
            return Some(PlanningAutoFollowBlockReason::InvalidWorkspace);
        }

        if template.id == BUILTIN_NEXT_TASK_TEMPLATE_ID
            && snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Uninitialized
        {
            return Some(PlanningAutoFollowBlockReason::ActionableQueueRequired);
        }

        None
    }

    pub fn build_summary_view(
        &self,
        request: PlanningRuntimeSummaryRequest<'_>,
    ) -> PlanningRuntimeSummaryView {
        let workspace_state = if request.is_repairing {
            PlanningWorkspaceState::Repairing
        } else {
            match request.snapshot.workspace_status() {
                PlanningRuntimeWorkspaceStatus::Uninitialized => {
                    PlanningWorkspaceState::Uninitialized
                }
                PlanningRuntimeWorkspaceStatus::Invalid => PlanningWorkspaceState::BlockedInvalid,
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
                | PlanningRuntimeWorkspaceStatus::ReadyWithTask
                    if request.has_running_turn =>
                {
                    PlanningWorkspaceState::Executing
                }
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
                | PlanningRuntimeWorkspaceStatus::ReadyWithTask => PlanningWorkspaceState::Ready,
            }
        };

        PlanningRuntimeSummaryView {
            status_label: workspace_status_label(workspace_state),
            queue_summary: request.snapshot.queue_summary().map(str::to_string),
            proposal_summary: request.snapshot.proposal_summary().map(str::to_string),
            failure_summary: request
                .repair_failure_summary
                .or_else(|| request.snapshot.failure_reason())
                .map(str::to_string),
            workspace_state,
        }
    }

    pub fn build_preview_view(
        &self,
        template: &FollowupTemplateDefinition,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> PlanningRuntimePreviewView {
        if let Some(reason) = self.auto_follow_block_reason(template, snapshot) {
            let detail = match reason {
                PlanningAutoFollowBlockReason::InvalidWorkspace => {
                    "planning files are invalid or incomplete".to_string()
                }
                PlanningAutoFollowBlockReason::ActionableQueueRequired => {
                    if let Some(proposal_summary) = snapshot.proposal_summary() {
                        format!(
                            "selected template requires an actionable planning queue head; {proposal_summary}"
                        )
                    } else {
                        "selected template requires an actionable planning queue head".to_string()
                    }
                }
            };
            return PlanningRuntimePreviewView {
                status_label: preview_block_label(reason),
                detail: Some(detail),
            };
        }

        PlanningRuntimePreviewView {
            status_label: match snapshot.workspace_status() {
                PlanningRuntimeWorkspaceStatus::Uninitialized => "inactive",
                PlanningRuntimeWorkspaceStatus::Invalid => "blocked",
                PlanningRuntimeWorkspaceStatus::ReadyNoTask
                | PlanningRuntimeWorkspaceStatus::ReadyWithTask => "ready",
            },
            detail: non_blocked_preview_detail(snapshot),
        }
    }
}

fn workspace_status_label(state: PlanningWorkspaceState) -> &'static str {
    match state {
        PlanningWorkspaceState::Uninitialized => "inactive",
        PlanningWorkspaceState::Authoring => "authoring",
        PlanningWorkspaceState::Ready => "valid",
        PlanningWorkspaceState::Executing => "stale",
        PlanningWorkspaceState::Repairing => "repairing",
        PlanningWorkspaceState::BlockedInvalid => "invalid",
    }
}

fn preview_block_label(reason: PlanningAutoFollowBlockReason) -> &'static str {
    match reason {
        PlanningAutoFollowBlockReason::InvalidWorkspace => "blocked",
        PlanningAutoFollowBlockReason::ActionableQueueRequired => "queue-empty",
    }
}

fn non_blocked_preview_detail(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    match (snapshot.queue_summary(), snapshot.proposal_summary()) {
        (Some(queue_summary), Some(proposal_summary)) => {
            Some(format!("{queue_summary}  |  {proposal_summary}"))
        }
        (Some(queue_summary), None) => Some(queue_summary.to_string()),
        (None, Some(proposal_summary)) => Some(proposal_summary.to_string()),
        (None, None) => snapshot.failure_reason().map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningAutoFollowBlockReason, PlanningRuntimePolicyService, PlanningRuntimeSummaryRequest,
    };
    use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
    use crate::domain::followup_template::{FollowupTemplateDefinition, FollowupTemplateSource};
    use crate::domain::planning::{PlanningWorkspaceState, PriorityQueueTask, TaskStatus};

    fn builtin_next_task_template() -> FollowupTemplateDefinition {
        FollowupTemplateDefinition {
            id: "builtin-next-task".to_string(),
            label: "builtin next-task".to_string(),
            body: "body".to_string(),
            source: FollowupTemplateSource::Builtin,
        }
    }

    fn workspace_template() -> FollowupTemplateDefinition {
        FollowupTemplateDefinition {
            id: "workspace-review".to_string(),
            label: "workspace review".to_string(),
            body: "body".to_string(),
            source: FollowupTemplateSource::WorkspaceFile {
                path: "/tmp/workspace/.codex-exec-loop/followups/review.md".to_string(),
            },
        }
    }

    fn queue_head() -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General workstream".to_string(),
            task_title: "Implement queue-aware policy".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 10,
            updated_at: "2026-04-10T00:00:00Z".to_string(),
            rank_reasons: vec!["status=ready".to_string()],
        }
    }

    #[test]
    fn builtin_next_task_blocks_when_planning_is_uninitialized() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::uninitialized();

        assert_eq!(
            service.auto_follow_block_reason(&builtin_next_task_template(), &snapshot),
            Some(PlanningAutoFollowBlockReason::ActionableQueueRequired)
        );
        assert_eq!(
            service
                .build_preview_view(&builtin_next_task_template(), &snapshot)
                .status_label,
            "queue-empty"
        );
    }

    #[test]
    fn builtin_next_task_allows_proposals_when_queue_is_empty() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            Some("2 promotable follow-up proposals available: Plan A | +1 more".to_string()),
            None,
        );

        assert_eq!(
            service.auto_follow_block_reason(&builtin_next_task_template(), &snapshot),
            None
        );

        let preview = service.build_preview_view(&builtin_next_task_template(), &snapshot);

        assert_eq!(preview.status_label, "ready");
        assert!(preview.detail.as_deref().is_some_and(|detail| {
            detail.contains("queue idle: no executable planning task")
                && detail.contains("promotable follow-up proposals available")
        }));
    }

    #[test]
    fn builtin_next_task_allows_ready_no_task_state_without_existing_proposals() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
            None,
        );

        assert_eq!(
            service.auto_follow_block_reason(&builtin_next_task_template(), &snapshot),
            None
        );
        assert_eq!(
            service
                .build_preview_view(&builtin_next_task_template(), &snapshot)
                .status_label,
            "ready"
        );
    }

    #[test]
    fn builtin_next_task_blocks_when_queue_head_and_proposals_are_both_missing() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::uninitialized();

        assert_eq!(
            service.auto_follow_block_reason(&builtin_next_task_template(), &snapshot),
            Some(PlanningAutoFollowBlockReason::ActionableQueueRequired)
        );
        assert!(
            service
                .build_preview_view(&builtin_next_task_template(), &snapshot)
                .detail
                .as_deref()
                .is_some_and(|detail| {
                    detail.contains("selected template requires an actionable planning queue head")
                })
        );
    }

    #[test]
    fn workspace_template_stays_allowed_without_planning_queue_head() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::uninitialized();

        assert_eq!(
            service.auto_follow_block_reason(&workspace_template(), &snapshot),
            None
        );
    }

    #[test]
    fn summary_view_marks_running_ready_planning_as_executing() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "next task: rank 1 / task-1".to_string(),
            Some(queue_head()),
        );

        let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: &snapshot,
            has_running_turn: true,
            is_repairing: false,
            repair_failure_summary: None,
        });

        assert_eq!(summary.workspace_state, PlanningWorkspaceState::Executing);
        assert_eq!(summary.status_label, "stale");
        assert_eq!(
            summary.queue_summary.as_deref(),
            Some("next task: rank 1 / task-1")
        );
    }

    #[test]
    fn summary_view_keeps_proposal_summary_when_present() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            Some("1 promotable follow-up proposal available: Draft sushi roadmap".to_string()),
            None,
        );

        let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
        });

        assert_eq!(summary.workspace_state, PlanningWorkspaceState::Ready);
        assert_eq!(
            summary.proposal_summary.as_deref(),
            Some("1 promotable follow-up proposal available: Draft sushi roadmap")
        );
    }

    #[test]
    fn summary_view_prefers_repair_failure_when_present() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::invalid(
            "planning validation failed: task-ledger.json".to_string(),
        );

        let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: true,
            repair_failure_summary: Some("task-ledger.json is missing direction_id"),
        });

        assert_eq!(summary.workspace_state, PlanningWorkspaceState::Repairing);
        assert_eq!(summary.status_label, "repairing");
        assert_eq!(
            summary.failure_summary.as_deref(),
            Some("task-ledger.json is missing direction_id")
        );
    }
}
