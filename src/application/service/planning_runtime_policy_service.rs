use crate::application::service::planning_auto_follow_copy::{
    BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT, PLANNING_QUEUE_REFRESH_WITH_PROPOSALS_TRANSCRIPT_TEXT,
    PLANNING_QUEUE_REFRESH_WITHOUT_PROPOSALS_TRANSCRIPT_TEXT,
};
use crate::application::service::planning_prompt_service::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::planning::PlanningWorkspaceState;
use crate::domain::text::compact_whitespace_detail;

const INTERNAL_QUEUE_METADATA_DETAIL_LIMIT: usize = 96;
const EMPTY_QUEUE_FRAMING_SUMMARY: &str =
    "now: none  |  next: none  |  proposed: none  |  blocked: none";

#[derive(Debug, Clone, Default)]
pub struct PlanningRuntimePolicyService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowBlockReason {
    PlanningDisabled,
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
    RefreshPlanningQueue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeSummaryView {
    pub workspace_state: PlanningWorkspaceState,
    pub status_label: &'static str,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub failure_summary: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimePreviewView {
    pub status_label: &'static str,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRuntimeOperatorGuidance {
    pub current_state: &'static str,
    pub cause: String,
    pub next_action: String,
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
    pub current_state_line: String,
    pub cause_line: String,
    pub next_action_line: String,
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
        if snapshot.workspace_present() && !snapshot.plan_enabled() {
            return PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::PlanningDisabled,
            );
        }

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
                PlanningAutoFollowPolicyDecision::QueuePrompt(
                    PlanningAutoFollowPromptMode::RefreshPlanningQueue,
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

    pub fn auto_follow_transcript_text(
        &self,
        snapshot: &PlanningRuntimeSnapshot,
        prompt_mode: PlanningAutoFollowPromptMode,
    ) -> String {
        match prompt_mode {
            PlanningAutoFollowPromptMode::ContinueQueuedTask => {
                BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string()
            }
            PlanningAutoFollowPromptMode::RefreshPlanningQueue => {
                if snapshot.has_proposal_candidates() {
                    PLANNING_QUEUE_REFRESH_WITH_PROPOSALS_TRANSCRIPT_TEXT.to_string()
                } else {
                    PLANNING_QUEUE_REFRESH_WITHOUT_PROPOSALS_TRANSCRIPT_TEXT.to_string()
                }
            }
        }
    }

    pub fn build_summary_view(
        &self,
        request: PlanningRuntimeSummaryRequest<'_>,
    ) -> PlanningRuntimeSummaryView {
        let workspace_state = if !request.snapshot.plan_enabled() {
            PlanningWorkspaceState::Uninitialized
        } else if request.is_repairing {
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
            status_label: workspace_status_label(request.snapshot, workspace_state),
            queue_summary: request
                .snapshot
                .compact_queue_framing_summary(INTERNAL_QUEUE_METADATA_DETAIL_LIMIT),
            proposal_summary: request
                .snapshot
                .compact_proposal_summary_detail(INTERNAL_QUEUE_METADATA_DETAIL_LIMIT),
            failure_summary: request
                .repair_failure_summary
                .or_else(|| request.snapshot.auto_followup_pause_reason())
                .or_else(|| request.snapshot.failure_reason())
                .map(str::to_string),
            workspace_state,
        }
    }

    #[cfg(test)]
    pub fn build_preview_view_for_decision(
        &self,
        decision: PlanningAutoFollowPolicyDecision,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> PlanningRuntimePreviewView {
        if let PlanningAutoFollowPolicyDecision::Blocked(reason) = decision {
            let detail = match reason {
                PlanningAutoFollowBlockReason::PlanningDisabled => {
                    "planning mode is off, so queue-driven continuation is waiting for you to turn it back on".to_string()
                }
                PlanningAutoFollowBlockReason::InvalidWorkspace => {
                    invalid_planning_cause(snapshot, None)
                }
                PlanningAutoFollowBlockReason::ActionableQueueRequired => {
                    if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Uninitialized
                    {
                        "no planning workspace is active for this shell yet".to_string()
                    } else if let Some(proposal_summary) = snapshot
                        .compact_proposal_summary_detail(INTERNAL_QUEUE_METADATA_DETAIL_LIMIT)
                    {
                        format!(
                            "planning is valid but has no next task yet; proposed work: {proposal_summary}"
                        )
                    } else {
                        idle_queue_cause(snapshot)
                    }
                }
                PlanningAutoFollowBlockReason::RepeatedQueueHead => snapshot
                    .auto_followup_pause_reason()
                    .unwrap_or(
                        "automation is paused because the queue did not advance beyond the previous task",
                    )
                    .to_string(),
            };
            return PlanningRuntimePreviewView {
                status_label: preview_block_label(reason, snapshot),
                detail: Some(detail),
            };
        }

        PlanningRuntimePreviewView {
            status_label: snapshot.preview_status_label(),
            detail: non_blocked_preview_detail(snapshot, INTERNAL_QUEUE_METADATA_DETAIL_LIMIT),
        }
    }

    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        let summary_request = PlanningRuntimeSummaryRequest {
            snapshot: request.snapshot,
            has_running_turn: request.has_running_turn,
            is_repairing: request.is_repairing,
            repair_failure_summary: request.repair_failure_summary,
        };
        let summary = self.build_summary_view(summary_request);
        if !request.always_show
            && summary.workspace_state == PlanningWorkspaceState::Uninitialized
            && !request.has_notice
        {
            return None;
        }

        let operator_guidance = build_operator_guidance(summary_request);
        let mut segments = vec![
            format!("current state: {}", operator_guidance.current_state),
            format!(
                "cause: {}",
                compact_projection_detail(&operator_guidance.cause, request.max_detail_len)
            ),
            format!(
                "next action: {}",
                compact_projection_detail(&operator_guidance.next_action, request.max_detail_len)
            ),
        ];
        if let Some(repair_attempt) = request.repair_attempt {
            segments.push(format!(
                "repair attempt: {}/{}",
                repair_attempt.attempts_used, repair_attempt.max_attempts
            ));
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
        let operator_guidance = build_operator_guidance(PlanningRuntimeSummaryRequest {
            snapshot: request.snapshot,
            has_running_turn: request.has_running_turn,
            is_repairing: request.is_repairing,
            repair_failure_summary: request.repair_failure_summary,
        });

        PlanningRuntimeStatusProjection {
            current_state_line: format!("current state: {}", operator_guidance.current_state),
            cause_line: format!(
                "cause: {}",
                compact_projection_detail(&operator_guidance.cause, request.max_detail_len)
            ),
            next_action_line: format!(
                "next action: {}",
                compact_projection_detail(&operator_guidance.next_action, request.max_detail_len)
            ),
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
                format!("{queue_label}: {queue_summary}")
            }),
            proposal_line: summary
                .proposal_summary
                .as_deref()
                .map(|proposal_summary| format!("planning proposals: {proposal_summary}")),
            failure_line: summary.failure_summary.as_deref().map(|failure_summary| {
                format!(
                    "last planning failure: {}",
                    compact_projection_detail(failure_summary, request.max_detail_len)
                )
            }),
        }
    }
}

fn workspace_status_label(
    snapshot: &PlanningRuntimeSnapshot,
    state: PlanningWorkspaceState,
) -> &'static str {
    match state {
        PlanningWorkspaceState::Uninitialized => "waiting",
        PlanningWorkspaceState::Authoring => "review needed",
        PlanningWorkspaceState::Ready => {
            if snapshot.auto_followup_pause_reason().is_some() {
                "paused"
            } else if snapshot.queue_head().is_none() && snapshot.proposal_summary().is_some() {
                "review needed"
            } else if snapshot.queue_head().is_none() {
                "waiting"
            } else {
                "ready"
            }
        }
        PlanningWorkspaceState::Executing => "running",
        PlanningWorkspaceState::Repairing => "repairing",
        PlanningWorkspaceState::BlockedInvalid => "blocked",
    }
}

#[cfg(test)]
fn preview_block_label(
    reason: PlanningAutoFollowBlockReason,
    snapshot: &PlanningRuntimeSnapshot,
) -> &'static str {
    match reason {
        PlanningAutoFollowBlockReason::PlanningDisabled => "waiting",
        PlanningAutoFollowBlockReason::InvalidWorkspace => "blocked",
        PlanningAutoFollowBlockReason::ActionableQueueRequired => {
            if snapshot.proposal_summary().is_some() {
                "review needed"
            } else {
                "waiting"
            }
        }
        PlanningAutoFollowBlockReason::RepeatedQueueHead => "paused",
    }
}

fn build_operator_guidance(
    request: PlanningRuntimeSummaryRequest<'_>,
) -> PlanningRuntimeOperatorGuidance {
    let snapshot = request.snapshot;
    if request.is_repairing {
        return PlanningRuntimeOperatorGuidance {
            current_state: "repairing",
            cause: invalid_planning_cause(snapshot, request.repair_failure_summary),
            next_action:
                "wait for repair to finish, or reopen planning if repair needs another fix"
                    .to_string(),
        };
    }

    if request.has_running_turn {
        return PlanningRuntimeOperatorGuidance {
            current_state: "running",
            cause: running_turn_cause(snapshot),
            next_action: "wait for the current turn to finish".to_string(),
        };
    }

    if !snapshot.plan_enabled() {
        return PlanningRuntimeOperatorGuidance {
            current_state: "waiting",
            cause: if snapshot.workspace_present() {
                "planning mode is off, so queue-driven continuation is waiting".to_string()
            } else {
                "no planning workspace is active for this shell yet".to_string()
            },
            next_action: if snapshot.workspace_present() {
                "run :planning and turn Plan on when you want queue-driven continuation".to_string()
            } else {
                "continue manually, or run :planning to set up queue-driven continuation"
                    .to_string()
            },
        };
    }

    if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
        return PlanningRuntimeOperatorGuidance {
            current_state: "blocked",
            cause: invalid_planning_cause(snapshot, request.repair_failure_summary),
            next_action: "reopen planning and fix the validation errors before resuming automation"
                .to_string(),
        };
    }

    if let Some(pause_reason) = snapshot.auto_followup_pause_reason() {
        return PlanningRuntimeOperatorGuidance {
            current_state: "paused",
            cause: pause_reason.to_string(),
            next_action: if snapshot.queue_head().is_some() || snapshot.proposal_summary().is_some()
            {
                "review the queue and choose the next actionable task before resuming automation"
                    .to_string()
            } else {
                "review the queue or reopen planning before resuming automation".to_string()
            },
        };
    }

    match snapshot.workspace_status() {
        PlanningRuntimeWorkspaceStatus::ReadyWithTask => PlanningRuntimeOperatorGuidance {
            current_state: "ready",
            cause: ready_queue_cause(snapshot),
            next_action: "let automation continue, or run the queued task manually".to_string(),
        },
        PlanningRuntimeWorkspaceStatus::ReadyNoTask => {
            if snapshot.proposal_summary().is_some() {
                PlanningRuntimeOperatorGuidance {
                    current_state: "review needed",
                    cause: proposal_review_cause(snapshot),
                    next_action: "review the queue and promote the next actionable task"
                        .to_string(),
                }
            } else if snapshot.queue_idle_policy() == crate::domain::planning::QueueIdlePolicy::Stop
            {
                PlanningRuntimeOperatorGuidance {
                    current_state: "paused",
                    cause: "planning is valid but the queue is idle, and automation stops here"
                        .to_string(),
                    next_action: "review the queue or add the next task before resuming automation"
                        .to_string(),
                }
            } else {
                PlanningRuntimeOperatorGuidance {
                    current_state: "waiting",
                    cause: idle_queue_cause(snapshot),
                    next_action:
                        "finish the next turn or review the queue so the next task can be queued"
                            .to_string(),
                }
            }
        }
        PlanningRuntimeWorkspaceStatus::Uninitialized => PlanningRuntimeOperatorGuidance {
            current_state: "waiting",
            cause: "no planning workspace is active for this shell yet".to_string(),
            next_action: "continue manually, or run :planning to set up queue-driven continuation"
                .to_string(),
        },
        PlanningRuntimeWorkspaceStatus::Invalid => PlanningRuntimeOperatorGuidance {
            current_state: "blocked",
            cause: invalid_planning_cause(snapshot, request.repair_failure_summary),
            next_action: "reopen planning and fix the validation errors before resuming automation"
                .to_string(),
        },
    }
}

#[cfg(test)]
fn non_blocked_preview_detail(
    snapshot: &PlanningRuntimeSnapshot,
    max_detail_len: usize,
) -> Option<String> {
    snapshot
        .compact_queue_framing_summary(max_detail_len)
        .or_else(|| snapshot.compact_proposal_summary_detail(max_detail_len))
        .or_else(|| {
            snapshot
                .failure_reason()
                .map(|detail| compact_projection_detail(detail, max_detail_len))
        })
}

fn compact_projection_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
}

fn running_turn_cause(snapshot: &PlanningRuntimeSnapshot) -> String {
    snapshot
        .queue_head()
        .map(|queue_head| {
            format!(
                "the shell is executing the current queued task: {}",
                queue_head.task_title.trim()
            )
        })
        .unwrap_or_else(|| "the shell is executing the current turn".to_string())
}

fn ready_queue_cause(snapshot: &PlanningRuntimeSnapshot) -> String {
    snapshot
        .queue_head()
        .map(|queue_head| format!("the next queued task is {}", queue_head.task_title.trim()))
        .unwrap_or_else(|| "planning is valid and ready for the next task".to_string())
}

fn idle_queue_cause(snapshot: &PlanningRuntimeSnapshot) -> String {
    if snapshot
        .queue_summary()
        .is_none_or(|queue_summary| queue_summary.trim().is_empty())
    {
        "planning is valid but has no next task yet".to_string()
    } else if let Some(summary) =
        snapshot.compact_queue_framing_summary(INTERNAL_QUEUE_METADATA_DETAIL_LIMIT)
    {
        if summary == EMPTY_QUEUE_FRAMING_SUMMARY {
            "planning is valid but has no next task yet".to_string()
        } else {
            format!("planning is valid but has no next task yet; queue detail: {summary}")
        }
    } else {
        "planning is valid but has no next task yet".to_string()
    }
}

fn proposal_review_cause(snapshot: &PlanningRuntimeSnapshot) -> String {
    if let Some(proposal_summary) =
        snapshot.compact_proposal_summary_detail(INTERNAL_QUEUE_METADATA_DETAIL_LIMIT)
    {
        format!("planning has proposals but no executable next task: {proposal_summary}")
    } else {
        "planning has proposals but no executable next task".to_string()
    }
}

fn invalid_planning_cause(
    snapshot: &PlanningRuntimeSnapshot,
    repair_failure_summary: Option<&str>,
) -> String {
    match repair_failure_summary.or_else(|| snapshot.failure_reason()) {
        Some(detail) => {
            format!("planning needs repair before automation can continue: {detail}")
        }
        None => "planning needs repair before automation can continue".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningAutoFollowBlockReason, PlanningAutoFollowPolicyDecision,
        PlanningAutoFollowPromptMode, PlanningRuntimePolicyService, PlanningRuntimeRepairAttempt,
        PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
        PlanningRuntimeSummaryRequest,
    };
    use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
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
            "waiting"
        );
    }

    #[test]
    fn builtin_next_task_allows_proposals_when_queue_is_empty() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: none  |  next: none  |  proposed: Plan A (+1 more)  |  blocked: none".to_string(),
            Some("Plan A (+1 more)".to_string()),
            None,
        );
        let decision = service.decide_auto_follow(&snapshot);

        assert_eq!(
            decision,
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::RefreshPlanningQueue
            )
        );

        let preview = service.build_preview_view_for_decision(decision, &snapshot);

        assert_eq!(preview.status_label, "review needed");
        assert_eq!(
            preview.detail.as_deref(),
            Some("now: none  |  next: none  |  proposed: Plan A (+1 more)  |  blocked: none")
        );
    }

    #[test]
    fn builtin_next_task_allows_ready_no_task_state_without_existing_proposals() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: none  |  next: none  |  proposed: none  |  blocked: none".to_string(),
            None,
            None,
        );
        let decision = service.decide_auto_follow(&snapshot);

        assert_eq!(
            decision,
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::RefreshPlanningQueue
            )
        );
        assert_eq!(
            service
                .build_preview_view_for_decision(decision, &snapshot)
                .status_label,
            "waiting"
        );
    }

    #[test]
    fn summary_line_uses_canonical_idle_queue_metadata_as_queue_detail() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: Draft roadmap  |  next: none  |  proposed: none  |  blocked: none".to_string(),
            None,
            None,
        )
        .with_queue_idle_policy(
            crate::domain::planning::QueueIdlePolicy::ReviewAndEnqueue,
            None,
        );

        let summary_line = service.build_summary_line(PlanningRuntimeSummaryLineRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
            repair_attempt: None,
            has_notice: false,
            max_detail_len: 160,
            always_show: true,
        });

        let summary_line = summary_line.expect("summary line should be projected");
        assert!(summary_line.contains(
            "cause: planning is valid but has no next task yet; queue detail: now: Draft roadmap"
        ));
        assert!(summary_line.contains("proposed: none"));
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
                    detail.contains("no planning workspace is active for this shell yet")
                })
        );
    }

    #[test]
    fn repeated_queue_head_blocks_queue_driven_automation() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "now: Implement queue-aware policy  |  next: none  |  proposed: none  |  blocked: none"
                .to_string(),
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
    fn builtin_next_task_uses_refresh_mode_when_queue_is_idle() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: none  |  next: none  |  proposed: Plan A (+1 more)  |  blocked: none".to_string(),
            Some("Plan A (+1 more)".to_string()),
            None,
        );

        assert_eq!(
            service.decide_auto_follow(&snapshot),
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::RefreshPlanningQueue
            )
        );
        assert!(
            service
                .auto_follow_transcript_text(
                    &snapshot,
                    PlanningAutoFollowPromptMode::RefreshPlanningQueue
                )
                .contains("existing proposal 작업 목록을 priority queue에 넣고")
        );
    }

    #[test]
    fn ready_queue_head_uses_continue_mode() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "now: Implement queue-aware policy  |  next: none  |  proposed: none  |  blocked: none"
                .to_string(),
            Some(queue_head()),
        );

        assert_eq!(
            service.decide_auto_follow(&snapshot),
            PlanningAutoFollowPolicyDecision::QueuePrompt(
                PlanningAutoFollowPromptMode::ContinueQueuedTask
            )
        );
        assert_eq!(
            service.auto_follow_transcript_text(
                &snapshot,
                PlanningAutoFollowPromptMode::ContinueQueuedTask,
            ),
            crate::application::service::planning_auto_follow_copy::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT
        );
    }

    #[test]
    fn summary_view_marks_running_ready_planning_as_executing() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "now: Implement queue-aware policy  |  next: none  |  proposed: none  |  blocked: none"
                .to_string(),
            Some(queue_head()),
        );

        let summary = service.build_summary_view(PlanningRuntimeSummaryRequest {
            snapshot: &snapshot,
            has_running_turn: true,
            is_repairing: false,
            repair_failure_summary: None,
        });

        assert_eq!(summary.workspace_state, PlanningWorkspaceState::Executing);
        assert_eq!(summary.status_label, "running");
        assert_eq!(
            summary.queue_summary.as_deref(),
            Some(
                "now: Implement queue-aware policy  |  next: none  |  proposed: none  |  blocked: none"
            )
        );
    }

    #[test]
    fn summary_view_keeps_proposal_summary_when_present() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: none  |  next: none  |  proposed: Draft sushi roadmap  |  blocked: none"
                .to_string(),
            Some("Draft sushi roadmap".to_string()),
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
            summary.queue_summary.as_deref(),
            Some("now: none  |  next: none  |  proposed: Draft sushi roadmap  |  blocked: none")
        );
        assert_eq!(
            summary.proposal_summary.as_deref(),
            Some("Draft sushi roadmap")
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

    #[test]
    fn summary_line_uses_current_state_cause_and_next_action() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: none  |  next: none  |  proposed: Draft roadmap (+1 more)  |  blocked: none"
                .to_string(),
            Some("Draft roadmap (+1 more)".to_string()),
            None,
        );

        let summary_line = service.build_summary_line(PlanningRuntimeSummaryLineRequest {
            snapshot: &snapshot,
            has_running_turn: false,
            is_repairing: true,
            repair_failure_summary: Some(
                "task-ledger.json is missing direction_id and contains extra trailing data",
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
        assert!(summary_line.contains("current state: repairing"));
        assert!(summary_line.contains("cause: planning needs rep"));
        assert!(summary_line.contains("next action: wait for repair"));
        assert!(summary_line.contains("repair attempt: 1/2"));
    }

    #[test]
    fn status_projection_uses_queue_head_label_when_actionable_work_exists() {
        let service = PlanningRuntimePolicyService::new();
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "now: Implement queue-aware policy  |  next: none  |  proposed: none  |  blocked: none"
                .to_string(),
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
            Some(
                "planning queue head: now: Implement queue-aware policy  |  next: none  |  proposed: none  |  blocked: none"
            )
        );
        assert_eq!(projection.current_state_line, "current state: ready");
        assert!(
            projection
                .cause_line
                .starts_with("cause: the next queued task is Implement queue-aware")
        );
        assert!(
            projection
                .next_action_line
                .starts_with("next action: let automation continue")
        );
    }

    #[test]
    fn status_projection_reframes_queue_snapshot_metadata() {
        let service = PlanningRuntimePolicyService::new();
        let queue_head = queue_head();
        let snapshot = PlanningRuntimeSnapshot::ready_with_queue_snapshot(
            "Planning Context".to_string(),
            "now: Implement queue-aware policy  |  next: Follow-up queue reconciliation  |  proposed: Draft sushi roadmap  |  blocked: none".to_string(),
            Some("Draft sushi roadmap".to_string()),
            Some(queue_head.clone()),
            crate::domain::planning::PriorityQueueSnapshot {
                next_task: Some(queue_head.clone()),
                active_tasks: vec![
                    queue_head,
                    crate::domain::planning::PriorityQueueTask {
                        rank: 2,
                        task_id: "task-2".to_string(),
                        direction_id: "general-workstream".to_string(),
                        direction_title: "General workstream".to_string(),
                        task_title: "Follow-up queue reconciliation".to_string(),
                        status: crate::domain::planning::TaskStatus::Ready,
                        combined_priority: 8,
                        updated_at: "2026-04-10T01:00:00Z".to_string(),
                        rank_reasons: vec!["status=ready".to_string()],
                    },
                ],
                proposed_tasks: vec![crate::domain::planning::PriorityQueueTask {
                    rank: 3,
                    task_id: "task-proposal-1".to_string(),
                    direction_id: "general-workstream".to_string(),
                    direction_title: "General workstream".to_string(),
                    task_title: "Draft sushi roadmap".to_string(),
                    status: crate::domain::planning::TaskStatus::Proposed,
                    combined_priority: 6,
                    updated_at: "2026-04-10T02:00:00Z".to_string(),
                    rank_reasons: vec!["combined_priority=6".to_string()],
                }],
                skipped_tasks: vec![crate::domain::planning::PriorityQueueSkippedTask {
                    task_id: "task-blocked-1".to_string(),
                    task_title: "Resolve blocked review thread".to_string(),
                    direction_id: "general-workstream".to_string(),
                    status: crate::domain::planning::TaskStatus::Blocked,
                    reason: "blocked by tasks: task-2(in_progress)".to_string(),
                }],
            },
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
            Some(
                "planning queue head: now: Implement queue-aware policy  |  next: Follow-up queue reconciliation  |  proposed: Draft sushi roadmap  |  blocked: Resolve blocked review thread (blocked by tasks: task-2(in_progress))"
            )
        );
        assert_eq!(
            projection.proposal_line.as_deref(),
            Some("planning proposals: Draft sushi roadmap")
        );
    }
}
