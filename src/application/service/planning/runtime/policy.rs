use crate::application::service::planning::runtime::prompt::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::planning::PlanningWorkspaceState;
use crate::domain::text::compact_whitespace_detail;

#[derive(Debug, Clone, Default)]
pub struct PlanningRuntimePolicyService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowBlockReason {
    InvalidWorkspace,
    ActionableQueueRequired,
    RepeatedQueueHead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowPolicyDecision {
    Blocked(PlanningAutoFollowBlockReason),
    QueuePrompt(PlanningAutoFollowPromptMode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowPromptMode {
    ContinueQueuedTask,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRuntimeRepairAttempt {
    pub attempts_used: usize,
    pub max_attempts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRuntimeSummaryLineRequest<'a> {
    pub snapshot: &'a PlanningRuntimeSnapshot,
    pub has_running_turn: bool,
    pub is_repairing: bool,
    pub repair_failure_summary: Option<&'a str>,
    pub repair_attempt: Option<PlanningRuntimeRepairAttempt>,
    pub has_notice: bool,
    pub max_detail_len: usize,
    pub always_show: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRuntimeStatusProjectionRequest<'a> {
    pub snapshot: &'a PlanningRuntimeSnapshot,
    pub has_running_turn: bool,
    pub is_repairing: bool,
    pub repair_failure_summary: Option<&'a str>,
    pub repair_attempt: Option<PlanningRuntimeRepairAttempt>,
    pub max_detail_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeStatusProjection {
    pub planning_status_line: String,
    pub repair_attempt_line: Option<String>,
    pub queue_head_line: Option<String>,
    pub proposal_line: Option<String>,
    pub failure_line: Option<String>,
}

impl PlanningRuntimePolicyService {
    pub fn new() -> Self {
        Self
    }

    pub fn decide_auto_follow(
        &self,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> PlanningAutoFollowPolicyDecision {
        if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
            return PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::InvalidWorkspace,
            );
        }

        if snapshot.auto_followup_pause_reason().is_some() {
            return PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::RepeatedQueueHead,
            );
        }

        match snapshot.workspace_status() {
            PlanningRuntimeWorkspaceStatus::Uninitialized => {
                PlanningAutoFollowPolicyDecision::Blocked(
                    PlanningAutoFollowBlockReason::ActionableQueueRequired,
                )
            }
            PlanningRuntimeWorkspaceStatus::ReadyNoTask => {
                PlanningAutoFollowPolicyDecision::Blocked(
                    PlanningAutoFollowBlockReason::ActionableQueueRequired,
                )
            }
            PlanningRuntimeWorkspaceStatus::ReadyWithTask => {
                PlanningAutoFollowPolicyDecision::QueuePrompt(
                    PlanningAutoFollowPromptMode::ContinueQueuedTask,
                )
            }
            PlanningRuntimeWorkspaceStatus::Invalid => PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::InvalidWorkspace,
            ),
        }
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
                .or_else(|| request.snapshot.auto_followup_pause_reason())
                .or_else(|| request.snapshot.failure_reason())
                .map(str::to_string),
            workspace_state,
        }
    }

    pub fn build_preview_view_for_decision(
        &self,
        decision: PlanningAutoFollowPolicyDecision,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> PlanningRuntimePreviewView {
        if let PlanningAutoFollowPolicyDecision::Blocked(reason) = decision {
            let detail = match reason {
                PlanningAutoFollowBlockReason::InvalidWorkspace => {
                    "planning files are invalid or incomplete".to_string()
                }
                PlanningAutoFollowBlockReason::ActionableQueueRequired => {
                    if let Some(proposal_summary) = snapshot.proposal_summary() {
                        format!(
                            "queue-driven auto follow-up requires an actionable planning queue head; {proposal_summary}"
                        )
                    } else {
                        "queue-driven auto follow-up requires an actionable planning queue head"
                            .to_string()
                    }
                }
                PlanningAutoFollowBlockReason::RepeatedQueueHead => snapshot
                    .auto_followup_pause_reason()
                    .unwrap_or(
                        "queue-driven auto follow-up is paused until the planning queue advances beyond the previously handed-off task",
                    )
                    .to_string(),
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

    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        let summary = self.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: request.snapshot,
            has_running_turn: request.has_running_turn,
            is_repairing: request.is_repairing,
            repair_failure_summary: request.repair_failure_summary,
        });
        if !request.always_show
            && summary.workspace_state == PlanningWorkspaceState::Uninitialized
            && !request.has_notice
        {
            return None;
        }

        let mut segments = vec![format!("planning: {}", summary.status_label)];
        if let Some(repair_attempt) = request.repair_attempt {
            segments.push(format!(
                "repair: {}/{}",
                repair_attempt.attempts_used, repair_attempt.max_attempts
            ));
        }

        match summary.workspace_state {
            PlanningWorkspaceState::Ready | PlanningWorkspaceState::Executing => {
                if let Some(queue_summary) = summary.queue_summary.as_deref() {
                    segments.push(format!(
                        "queue: {}",
                        compact_queue_summary(
                            request.snapshot,
                            queue_summary,
                            request.max_detail_len
                        )
                    ));
                }
                if let Some(proposal_summary) = summary.proposal_summary.as_deref() {
                    segments.push(format!(
                        "proposals: {}",
                        compact_projection_detail(proposal_summary, request.max_detail_len)
                    ));
                }
            }
            PlanningWorkspaceState::Repairing => {
                if let Some(failure_summary) = summary.failure_summary.as_deref() {
                    segments.push(format!(
                        "failure: {}",
                        compact_projection_detail(failure_summary, request.max_detail_len)
                    ));
                }
                if let Some(queue_summary) = summary.queue_summary.as_deref() {
                    segments.push(format!(
                        "queue: {}",
                        compact_queue_summary(
                            request.snapshot,
                            queue_summary,
                            request.max_detail_len
                        )
                    ));
                }
                if let Some(proposal_summary) = summary.proposal_summary.as_deref() {
                    segments.push(format!(
                        "proposals: {}",
                        compact_projection_detail(proposal_summary, request.max_detail_len)
                    ));
                }
            }
            PlanningWorkspaceState::BlockedInvalid => {
                if let Some(failure_summary) = summary.failure_summary.as_deref() {
                    segments.push(format!(
                        "failure: {}",
                        compact_projection_detail(failure_summary, request.max_detail_len)
                    ));
                }
            }
            PlanningWorkspaceState::Uninitialized | PlanningWorkspaceState::Authoring => {}
        }

        Some(segments.join("  |  "))
    }

    pub fn build_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        let summary = self.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: request.snapshot,
            has_running_turn: request.has_running_turn,
            is_repairing: request.is_repairing,
            repair_failure_summary: request.repair_failure_summary,
        });

        PlanningRuntimeStatusProjection {
            planning_status_line: format!("planning status: {}", summary.status_label),
            repair_attempt_line: request.repair_attempt.map(|repair_attempt| {
                format!(
                    "planning repair attempt: {}/{}",
                    repair_attempt.attempts_used, repair_attempt.max_attempts
                )
            }),
            queue_head_line: summary.queue_summary.as_deref().map(|queue_summary| {
                let queue_label = if request.snapshot.queue_head().is_some() {
                    "planning queue head"
                } else {
                    "planning queue"
                };
                format!(
                    "{queue_label}: {}",
                    compact_queue_summary(request.snapshot, queue_summary, request.max_detail_len)
                )
            }),
            proposal_line: summary.proposal_summary.as_deref().map(|proposal_summary| {
                format!(
                    "planning proposals: {}",
                    compact_projection_detail(proposal_summary, request.max_detail_len)
                )
            }),
            failure_line: summary.failure_summary.as_deref().map(|failure_summary| {
                format!(
                    "last planning failure: {}",
                    compact_projection_detail(failure_summary, request.max_detail_len)
                )
            }),
        }
    }
}

fn compact_queue_summary(
    snapshot: &PlanningRuntimeSnapshot,
    queue_summary: &str,
    max_detail_len: usize,
) -> String {
    let mut detail = compact_projection_detail(queue_summary, max_detail_len);
    if snapshot.queue_head().is_none() {
        detail.push_str(&format!(
            " / policy {}",
            snapshot.queue_idle_policy().label()
        ));
    }
    detail
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
        PlanningAutoFollowBlockReason::RepeatedQueueHead => "paused",
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

fn compact_projection_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningAutoFollowBlockReason, PlanningAutoFollowPolicyDecision,
        PlanningAutoFollowPromptMode, PlanningRuntimePolicyService, PlanningRuntimeRepairAttempt,
        PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
        PlanningRuntimeSummaryRequest,
    };
    use crate::application::service::planning::runtime::prompt::PlanningRuntimeSnapshot;
    use crate::domain::planning::{PlanningWorkspaceState, PriorityQueueTask, TaskStatus};

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
        let decision = service.decide_auto_follow(&snapshot);

        assert_eq!(
            decision,
            PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::ActionableQueueRequired
            )
        );
        assert_eq!(
            service
                .build_preview_view_for_decision(decision, &snapshot)
                .status_label,
            "queue-empty"
        );
    }

    #[test]
    fn builtin_next_task_blocks_main_prompt_when_queue_is_empty_with_proposals() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            Some("2 promotable follow-up proposals available: Plan A | +1 more".to_string()),
            None,
        );
        let decision = service.decide_auto_follow(&snapshot);

        assert_eq!(
            decision,
            PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::ActionableQueueRequired
            )
        );

        let preview = service.build_preview_view_for_decision(decision, &snapshot);

        assert_eq!(preview.status_label, "queue-empty");
        assert!(preview.detail.as_deref().is_some_and(|detail| {
            detail
                .contains("queue-driven auto follow-up requires an actionable planning queue head")
                && detail.contains("promotable follow-up proposals available")
        }));
    }

    #[test]
    fn builtin_next_task_blocks_ready_no_task_state_without_existing_proposals() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
            None,
        );
        let decision = service.decide_auto_follow(&snapshot);

        assert_eq!(
            decision,
            PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::ActionableQueueRequired
            )
        );
        assert_eq!(
            service
                .build_preview_view_for_decision(decision, &snapshot)
                .status_label,
            "queue-empty"
        );
    }

    #[test]
    fn builtin_next_task_blocks_when_queue_head_and_proposals_are_both_missing() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::uninitialized();
        let decision = service.decide_auto_follow(&snapshot);

        assert_eq!(
            decision,
            PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::ActionableQueueRequired
            )
        );
        assert!(
            service
                .build_preview_view_for_decision(decision, &snapshot)
                .detail
                .as_deref()
                .is_some_and(|detail| {
                    detail.contains(
                        "queue-driven auto follow-up requires an actionable planning queue head",
                    )
                })
        );
    }

    #[test]
    fn repeated_queue_head_blocks_queue_driven_automation() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "next task: rank 1 / task-1".to_string(),
            Some(queue_head()),
        )
        .with_auto_followup_pause_reason(
            "planner refresh kept the previously handed-off task as the queue head",
        );

        assert_eq!(
            service.decide_auto_follow(&snapshot),
            PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::RepeatedQueueHead
            )
        );
    }

    #[test]
    fn builtin_next_task_never_builds_main_refresh_prompt_when_queue_is_idle() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            Some("2 promotable follow-up proposals available: Plan A | +1 more".to_string()),
            None,
        );

        assert_eq!(
            service.decide_auto_follow(&snapshot),
            PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::ActionableQueueRequired
            )
        );
    }

    #[test]
    fn ready_queue_head_uses_continue_mode() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "next task: rank 1 / task-1".to_string(),
            Some(queue_head()),
        );

        assert_eq!(
            service.decide_auto_follow(&snapshot),
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::ContinueQueuedTask
            )
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
            "planning validation failed: task authority".to_string(),
        );

        let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: true,
            repair_failure_summary: Some("task authority is missing direction_id"),
        });

        assert_eq!(summary.workspace_state, PlanningWorkspaceState::Repairing);
        assert_eq!(summary.status_label, "repairing");
        assert_eq!(
            summary.failure_summary.as_deref(),
            Some("task authority is missing direction_id")
        );
    }

    #[test]
    fn summary_line_compacts_repair_queue_and_proposal_details() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            Some(
                "2 promotable follow-up proposals available: Draft roadmap | Draft checklist"
                    .to_string(),
            ),
            None,
        );

        let summary_line = service.build_summary_line(PlanningRuntimeSummaryLineRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: true,
            repair_failure_summary: Some(
                "task authority is missing direction_id and contains extra trailing data",
            ),
            repair_attempt: Some(PlanningRuntimeRepairAttempt {
                attempts_used: 1,
                max_attempts: 2,
            }),
            has_notice: false,
            max_detail_len: 24,
            always_show: true,
        });

        let summary_line = summary_line.expect("summary line should be projected");
        assert!(summary_line.contains("planning: repairing"));
        assert!(summary_line.contains("repair: 1/2"));
        assert!(summary_line.contains("failure: task authority"));
        assert!(summary_line.contains("queue: queue idle:"));
        assert!(summary_line.contains("proposals: 2 promotable"));
    }

    #[test]
    fn status_projection_uses_queue_head_label_when_actionable_work_exists() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "next task: rank 1 / task-1".to_string(),
            Some(queue_head()),
        );

        let projection = service.build_status_projection(PlanningRuntimeStatusProjectionRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
            repair_attempt: None,
            max_detail_len: 48,
        });

        assert_eq!(
            projection.queue_head_line.as_deref(),
            Some("planning queue head: next task: rank 1 / task-1")
        );
    }
}
