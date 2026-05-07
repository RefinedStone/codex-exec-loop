/*
 * runtime policy는 PlanningRuntimeSnapshot을 adapter-facing 의미로 해석하는 계층이다. prompt builder는 raw snapshot
 * assembly를 담당하고, 이 service는 automation이 다음 turn으로 진행해도 되는지, TUI footer/preview/diagnostics에
 * 어떤 compact status string을 보여 줄지 결정한다. 즉 여기의 출력은 domain state 자체가 아니라 operator와 adapter가
 * 소비하는 실행 정책 projection이다.
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
    // block reason은 validation error보다 의도적으로 거칠다. schema diagnostic이 아니라 operator-facing automation
    // gate라서 "왜 다음 turn을 만들지 않는가"만 설명한다.
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
    // popup과 status line이 공유하는 runtime state vocabulary다. repair/running-turn 같은 live overlay가 raw file
    // state를 덮어쓸 수 있도록 policy projection에서 한 번 정규화한다.
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
         * auto follow-up은 queue-driven으로만 허용된다. workspace가 valid하다는 사실만으로는 부족하고, snapshot에
         * actionable queue head가 있어야 하며, pause guard가 같은 head를 이미 handoff하지 않았음을 확인해야 한다.
         * 이렇게 해야 proposal refresh나 empty planning state가 무한한 assistant turn을 만들지 않는다.
         */
        if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
            return PlanningAutoFollowPolicyDecision::Blocked(
                PlanningAutoFollowBlockReason::InvalidWorkspace,
            );
        }
        if snapshot.auto_follow_pause_reason().is_some() {
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
         * summary view는 live runtime state를 planning domain state vocabulary로 접는다. snapshot file state는
         * "ready"라고 말할 수 있지만, 실제 app은 repair 중이거나 이미 turn을 실행 중일 수 있다. 이 overlay를 여기서
         * projection해 footer, popup, status command가 하나의 status model을 공유하게 한다.
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
                .or_else(|| request.snapshot.auto_follow_pause_reason())
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
         * preview는 policy를 다시 계산하지 않고 이미 내려진 decision에서 파생한다. blocked decision은 automation을 멈춘
         * gate를 설명하고, allowed decision은 다음 generated prompt를 설명할 queue/proposal/failure detail을 보여 준다.
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
                    .auto_follow_pause_reason()
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
         * footer line은 의도적으로 sparse하다. uninitialized planning은 notice나 explicit display request가 없으면
         * 숨기고, active/repair state는 다음 turn이 생성되거나 생성되지 않는 이유를 설명할 만큼의 queue/proposal/failure
         * context만 담는다.
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
            PlanningWorkspaceState::Uninitialized => {}
        }

        Some(segments.join("  |  "))
    }

    pub fn build_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        // status projection은 같은 summary model을 command output/diagnostics용 개별 line으로 펼친다. caller가 compact
        // footer string을 parsing하지 않고 필요한 line만 선택해 렌더링할 수 있게 한다.
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
    // queue head가 없을 때 idle policy는 그 부재가 예상된 상태인지 operator-actionable 상태인지 설명한다. generic queue
    // summary를 TUI용으로 compact하면 이 signal이 사라질 수 있어 뒤에 붙인다.
    if snapshot.queue_head().is_none() {
        detail.push_str(&format!(
            " / policy {}",
            snapshot.queue_idle_policy().label()
        ));
    }
    detail
}

fn workspace_status_label(state: PlanningWorkspaceState) -> &'static str {
    // label은 user-facing status vocabulary다. PlanningWorkspaceState variant 이름을 그대로 노출하지 않고,
    // TUI가 오래 유지해 온 짧은 상태 문자열로 낮춘다.
    match state {
        PlanningWorkspaceState::Uninitialized => "inactive",
        PlanningWorkspaceState::Ready => "valid",
        PlanningWorkspaceState::Executing => "stale",
        PlanningWorkspaceState::Repairing => "repairing",
        PlanningWorkspaceState::BlockedInvalid => "invalid",
    }
}

fn preview_block_label(reason: PlanningAutoFollowBlockReason) -> &'static str {
    // preview block label은 auto-follow preview header에 쓰인다. block reason보다 더 짧고 UI 상태에 가까운 단어로
    // 축약해, 자세한 설명은 detail line에 맡긴다.
    match reason {
        PlanningAutoFollowBlockReason::InvalidWorkspace => "blocked",
        PlanningAutoFollowBlockReason::ActionableQueueRequired => "queue-empty",
        PlanningAutoFollowBlockReason::RepeatedQueueHead => "paused",
    }
}

fn non_blocked_preview_detail(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    // non-blocked preview는 actionable queue context를 먼저 보여 주고, 다음으로 proposal을 보여 준다. live planning
    // work가 없을 때만 failure text로 fallback한다.
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
    // projection detail은 여러 UI surface에서 같은 whitespace/length 정책으로 줄인다. policy layer가 이 helper를 감싸
    // caller가 domain text normalization 위치를 알 필요 없게 한다.
    compact_whitespace_detail(text, max_len)
}

#[cfg(test)]
#[path = "policy/tests.rs"]
mod tests;
