/*
 * Runtime policy is the adapter-facing interpretation layer for
 * PlanningRuntimeSnapshot.  The prompt builder owns raw snapshot assembly, while
 * this service decides whether automation may advance and which compact status
 * strings the TUI should surface for footer lines, previews, and diagnostics.
 */
use crate::application::service::planning::runtime::prompt::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::planning::PlanningWorkspaceState;
use crate::domain::text::compact_whitespace_detail;

#[derive(Debug, Clone, Default)]
pub struct PlanningRuntimePolicyService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowBlockReason {
    // These reasons intentionally stay coarser than validation errors.  They
    // are operator-facing automation gates, not schema diagnostics.
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
    // This is the shared state vocabulary used by the popup and status line, so
    // runtime overlays such as repair/running-turn can override raw file state.
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
        /*
         * Auto follow-up is queue-driven only.  A valid workspace is not enough:
         * the snapshot must contain an actionable queue head, and the pause
         * guard must confirm that the same head was not already handed off.
         * This keeps proposal refreshes and empty planning states from creating
         * unbounded assistant turns.
         */
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
        /*
         * Summary view folds runtime state back onto the planning domain state.
         * Snapshot file state can say "ready", while the live app may be
         * repairing the files or already executing a turn; projecting that here
         * keeps footer, popup, and status commands aligned on one status model.
         */
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
        /*
         * The preview is deliberately derived from the decision instead of
         * recalculating policy.  Blocked decisions explain the gate that stopped
         * automation; allowed decisions expose the queue/proposal/failure detail
         * that will contextualize the next generated prompt.
         */
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
        /*
         * The footer line is intentionally sparse.  Uninitialized planning is
         * hidden unless a notice or explicit display request exists, while
         * active/repair states include just enough queue, proposal, and failure
         * context to explain why the next turn will or will not be generated.
         */
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
        // Status projection expands the same summary model into separate lines
        // for command output or diagnostics, where callers can choose which
        // line to render instead of parsing the compact footer string.
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
    // When no queue head exists, the idle policy explains whether the absence is
    // expected or operator-actionable.  That signal would otherwise disappear
    // after the generic queue summary is compacted for the TUI.
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
    // Non-blocked previews prefer actionable queue context, then proposals, and
    // only fall back to failure text when there is no live planning work to show.
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
#[path = "policy/tests.rs"]
mod tests;
