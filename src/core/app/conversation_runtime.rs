use super::{
    ApprovalDecisionCorrelation, CorePromptOrigin, PostTurnEvaluationCorrelation,
    TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::domain::conversation::{
    ConversationApprovalRequest, ConversationApprovalRequestIdentity, ConversationApprovalReview,
};
use crate::domain::planning::PostTurnExecution;
use crate::domain::planning::TaskHandoff;
use std::time::Instant;

pub const DEFAULT_AUTO_FOLLOW_MAX_TURNS: usize = 0;
pub const DEFAULT_AUTO_FOLLOW_STOP_KEYWORD: &str = "AUTO_STOP";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTurnPhase {
    Submitting,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTurnSnapshot {
    pub correlation: TurnSubmissionCorrelation,
    pub phase: ActiveTurnPhase,
    pub workspace_directory: String,
    pub turn_id: Option<String>,
    pub prompt_origin: CorePromptOrigin,
    pub started_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAuthorityPhase {
    Pending,
    Submitting,
    Submitted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalAuthoritySnapshot {
    pub request: ConversationApprovalRequest,
    pub decision: Option<ApprovalDecisionCorrelation>,
    pub phase: ApprovalAuthorityPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoFollowPhase {
    Idle,
    Queued {
        turn_index: usize,
        started_at: Instant,
    },
    Submitting {
        turn_index: usize,
        started_at: Instant,
    },
    Running {
        turn_index: usize,
        started_at: Instant,
    },
}

impl AutoFollowPhase {
    pub const fn is_live(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub const fn turn_index(&self) -> Option<usize> {
        match self {
            Self::Idle => None,
            Self::Queued { turn_index, .. }
            | Self::Submitting { turn_index, .. }
            | Self::Running { turn_index, .. } => Some(*turn_index),
        }
    }

    pub const fn started_at(&self) -> Option<Instant> {
        match self {
            Self::Idle => None,
            Self::Queued { started_at, .. }
            | Self::Submitting { started_at, .. }
            | Self::Running { started_at, .. } => Some(*started_at),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFollowAuthoritySnapshot {
    pub max_auto_turns: usize,
    pub completed_auto_turns: usize,
    pub phase: AutoFollowPhase,
    pub continuation_paused: bool,
    pub parallel_rearmed_after_stop: bool,
    pub stop_keyword_enabled: bool,
    pub stop_keyword: String,
    pub stop_on_no_file_changes: bool,
}

impl AutoFollowAuthoritySnapshot {
    pub const fn is_enabled(&self) -> bool {
        self.max_auto_turns > 0
    }

    pub const fn can_queue_next(&self) -> bool {
        !self.continuation_paused && self.completed_auto_turns < self.max_auto_turns
    }

    pub const fn parallel_post_turn_continuation_allowed(&self) -> bool {
        !self.continuation_paused || self.parallel_rearmed_after_stop
    }

    pub const fn post_turn_continuation_paused(&self) -> bool {
        self.continuation_paused
    }

    pub const fn has_live_activity(&self) -> bool {
        self.phase.is_live()
    }

    pub(super) fn matches_stop_keyword(&self, text: &str) -> bool {
        self.stop_keyword_enabled
            && text.split_whitespace().any(|token| {
                token
                    .trim_matches(|character: char| {
                        !character.is_alphanumeric() && character != '_'
                    })
                    .eq_ignore_ascii_case(&self.stop_keyword)
            })
    }
}

impl Default for AutoFollowAuthoritySnapshot {
    fn default() -> Self {
        Self {
            max_auto_turns: DEFAULT_AUTO_FOLLOW_MAX_TURNS,
            completed_auto_turns: 0,
            phase: AutoFollowPhase::Idle,
            continuation_paused: false,
            parallel_rearmed_after_stop: false,
            stop_keyword_enabled: true,
            stop_keyword: DEFAULT_AUTO_FOLLOW_STOP_KEYWORD.to_string(),
            stop_on_no_file_changes: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostTurnRouteResolution {
    ParallelConsumed,
    AutoSubmit,
    NoContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostTurnAuthoritySnapshot {
    Idle,
    Evaluating {
        correlation: PostTurnEvaluationCorrelation,
        started_at: Instant,
    },
    AwaitingRoute {
        correlation: PostTurnEvaluationCorrelation,
        started_at: Instant,
    },
    Settled {
        correlation: PostTurnEvaluationCorrelation,
        resolution: PostTurnRouteResolution,
    },
}

impl PostTurnAuthoritySnapshot {
    pub const fn is_in_flight(&self) -> bool {
        matches!(self, Self::Evaluating { .. } | Self::AwaitingRoute { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRuntimeSnapshot {
    pub active_turn: Option<ActiveTurnSnapshot>,
    pub approval: Option<ApprovalAuthoritySnapshot>,
    pub approval_review: Option<ConversationApprovalReview>,
    pub auto_follow: AutoFollowAuthoritySnapshot,
    pub post_turn: PostTurnAuthoritySnapshot,
    pub planning_handoff: Option<TaskHandoff>,
}

impl ConversationRuntimeSnapshot {
    pub fn initial() -> Self {
        Self {
            active_turn: None,
            approval: None,
            approval_review: None,
            auto_follow: AutoFollowAuthoritySnapshot::default(),
            post_turn: PostTurnAuthoritySnapshot::Idle,
            planning_handoff: None,
        }
    }

    pub const fn has_active_turn(&self) -> bool {
        self.active_turn.is_some()
    }

    pub fn has_running_turn(&self) -> bool {
        self.active_turn
            .as_ref()
            .is_some_and(|turn| turn.phase == ActiveTurnPhase::Running)
    }

    pub const fn can_accept_runtime_prompt(&self) -> bool {
        self.active_turn.is_none()
    }

    pub const fn can_accept_manual_prompt(&self) -> bool {
        self.can_accept_runtime_prompt()
            && !self.auto_follow.has_live_activity()
            && !self.post_turn.is_in_flight()
    }
}

impl Default for ConversationRuntimeSnapshot {
    fn default() -> Self {
        Self::initial()
    }
}

#[derive(Debug, Clone)]
struct PendingPostTurnRoute {
    correlation: PostTurnEvaluationCorrelation,
    execution: Box<PostTurnExecution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnAuthorityAdmission {
    Accepted,
    RejectedPreservingLease,
    RejectedMalformedCurrentAutoFollowTarget,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationRuntimeAuthority {
    snapshot: ConversationRuntimeSnapshot,
    pending_post_turn_route: Option<PendingPostTurnRoute>,
}

impl ConversationRuntimeAuthority {
    pub(super) fn new() -> Self {
        Self {
            snapshot: ConversationRuntimeSnapshot::initial(),
            pending_post_turn_route: None,
        }
    }

    pub(super) fn snapshot(&self) -> ConversationRuntimeSnapshot {
        self.snapshot.clone()
    }

    pub(super) fn active_turn_correlation(&self) -> Option<TurnSubmissionCorrelation> {
        self.snapshot
            .active_turn
            .as_ref()
            .map(|turn| turn.correlation)
    }

    pub(super) fn active_approval_decision(&self) -> Option<&ApprovalDecisionCorrelation> {
        self.snapshot
            .approval
            .as_ref()
            .and_then(|approval| approval.decision.as_ref())
    }

    pub(super) fn classify_turn_admission(
        &self,
        request: &TurnSubmissionRequest,
    ) -> TurnAuthorityAdmission {
        match request.prompt_origin {
            CorePromptOrigin::Manual => {
                if request.auto_follow_source.is_none()
                    && request.planning_handoff.is_none()
                    && self.snapshot.can_accept_manual_prompt()
                {
                    TurnAuthorityAdmission::Accepted
                } else {
                    TurnAuthorityAdmission::RejectedPreservingLease
                }
            }
            CorePromptOrigin::ManualIntake => {
                if request.auto_follow_source.is_none() && self.snapshot.can_accept_manual_prompt()
                {
                    TurnAuthorityAdmission::Accepted
                } else {
                    TurnAuthorityAdmission::RejectedPreservingLease
                }
            }
            CorePromptOrigin::AutoFollow => {
                if request.planning_handoff.is_some() {
                    return TurnAuthorityAdmission::RejectedPreservingLease;
                }
                let Some(source) = request.auto_follow_source.as_ref() else {
                    return TurnAuthorityAdmission::RejectedPreservingLease;
                };
                let owns_current_queued_lease = matches!(
                    &self.snapshot.post_turn,
                    PostTurnAuthoritySnapshot::Settled {
                        correlation,
                        resolution: PostTurnRouteResolution::AutoSubmit,
                    } if correlation == source
                ) && matches!(
                    self.snapshot.auto_follow.phase,
                    AutoFollowPhase::Queued { .. }
                );
                if !owns_current_queued_lease {
                    return TurnAuthorityAdmission::RejectedPreservingLease;
                }
                if request.thread_id.as_deref() == Some(source.thread_id.as_str())
                    && request.workspace_directory == source.turn_workspace_directory
                {
                    TurnAuthorityAdmission::Accepted
                } else {
                    TurnAuthorityAdmission::RejectedMalformedCurrentAutoFollowTarget
                }
            }
        }
    }

    pub(super) fn begin_turn(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        request: &TurnSubmissionRequest,
    ) {
        self.snapshot.approval = None;
        self.snapshot.approval_review = None;
        self.snapshot.post_turn = PostTurnAuthoritySnapshot::Idle;
        self.pending_post_turn_route = None;
        match request.prompt_origin {
            CorePromptOrigin::Manual => {
                self.snapshot.planning_handoff = None;
                self.snapshot.auto_follow.completed_auto_turns = 0;
                self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
            }
            CorePromptOrigin::ManualIntake => {
                self.snapshot.planning_handoff = request.planning_handoff.clone();
                self.snapshot.auto_follow.completed_auto_turns = 0;
                self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
            }
            CorePromptOrigin::AutoFollow => {
                let turn_index = self
                    .snapshot
                    .auto_follow
                    .phase
                    .turn_index()
                    .unwrap_or(self.snapshot.auto_follow.completed_auto_turns + 1);
                self.snapshot.auto_follow.phase = AutoFollowPhase::Submitting {
                    turn_index,
                    started_at: Instant::now(),
                };
            }
        }
        self.snapshot.active_turn = Some(ActiveTurnSnapshot {
            correlation,
            phase: ActiveTurnPhase::Submitting,
            workspace_directory: request.workspace_directory.clone(),
            turn_id: None,
            prompt_origin: request.prompt_origin,
            started_at: Instant::now(),
        });
    }

    pub(super) fn mark_turn_started(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        turn_id: String,
    ) {
        let Some(active) = self
            .snapshot
            .active_turn
            .as_mut()
            .filter(|active| active.correlation == correlation)
        else {
            return;
        };
        if active.phase == ActiveTurnPhase::Running {
            return;
        }
        active.phase = ActiveTurnPhase::Running;
        active.turn_id = Some(turn_id);
        if active.prompt_origin == CorePromptOrigin::AutoFollow {
            let turn_index = self
                .snapshot
                .auto_follow
                .phase
                .turn_index()
                .unwrap_or(self.snapshot.auto_follow.completed_auto_turns + 1);
            self.snapshot.auto_follow.phase = AutoFollowPhase::Running {
                turn_index,
                started_at: Instant::now(),
            };
        }
        self.snapshot.approval = None;
        self.snapshot.approval_review = None;
    }

    pub(super) fn replace_active_turn_workspace(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        workspace_directory: String,
    ) -> bool {
        let Some(active) = self
            .snapshot
            .active_turn
            .as_mut()
            .filter(|active| active.correlation == correlation)
        else {
            return false;
        };
        active.workspace_directory = workspace_directory;
        true
    }

    pub(super) fn finish_turn(&mut self, correlation: TurnSubmissionCorrelation) {
        if self
            .snapshot
            .active_turn
            .as_ref()
            .is_none_or(|active| active.correlation != correlation)
        {
            return;
        }
        let active = self
            .snapshot
            .active_turn
            .take()
            .expect("exact active turn must remain present");
        if active.prompt_origin == CorePromptOrigin::AutoFollow
            && matches!(
                self.snapshot.auto_follow.phase,
                AutoFollowPhase::Submitting { .. } | AutoFollowPhase::Running { .. }
            )
        {
            self.snapshot.auto_follow.completed_auto_turns += 1;
        }
        self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
        self.snapshot.approval = None;
        self.snapshot.approval_review = None;
    }

    pub(super) fn invalidate_conversation(&mut self) {
        self.snapshot.active_turn = None;
        self.snapshot.approval = None;
        self.snapshot.approval_review = None;
        self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
        self.snapshot.post_turn = PostTurnAuthoritySnapshot::Idle;
        self.snapshot.planning_handoff = None;
        self.pending_post_turn_route = None;
    }

    pub(super) fn set_pending_approval(&mut self, request: ConversationApprovalRequest) {
        if let Some(active) = self.snapshot.approval.as_mut()
            && active.request.identity() == request.identity()
        {
            active.request = request;
            return;
        }
        self.snapshot.approval = Some(ApprovalAuthoritySnapshot {
            request,
            decision: None,
            phase: ApprovalAuthorityPhase::Pending,
        });
    }

    pub(super) fn set_approval_review(&mut self, review: ConversationApprovalReview) {
        self.snapshot.approval_review = Some(review);
    }

    pub(super) fn clear_pending_approval(
        &mut self,
        request_identity: &ConversationApprovalRequestIdentity,
    ) {
        if self
            .snapshot
            .approval
            .as_ref()
            .is_some_and(|approval| approval.request.identity() == *request_identity)
        {
            self.snapshot.approval = None;
        }
    }

    pub(super) fn begin_approval_decision(
        &mut self,
        correlation: ApprovalDecisionCorrelation,
    ) -> bool {
        let Some(approval) = self.snapshot.approval.as_mut() else {
            return false;
        };
        if approval.request.identity() != correlation.request_identity {
            return false;
        }
        approval.decision = Some(correlation);
        approval.phase = ApprovalAuthorityPhase::Submitting;
        true
    }

    pub(super) fn complete_approval_decision(
        &mut self,
        correlation: &ApprovalDecisionCorrelation,
        succeeded: bool,
    ) -> bool {
        let Some(approval) = self.snapshot.approval.as_mut() else {
            return false;
        };
        if approval.decision.as_ref() != Some(correlation)
            || approval.phase != ApprovalAuthorityPhase::Submitting
        {
            return false;
        }
        if succeeded {
            approval.phase = ApprovalAuthorityPhase::Submitted;
        } else {
            approval.decision = None;
            approval.phase = ApprovalAuthorityPhase::Pending;
        }
        true
    }

    pub(super) fn set_auto_follow_max_turns(&mut self, value: usize) {
        self.cancel_queued_auto_follow_submission();
        self.snapshot.auto_follow.max_auto_turns = value;
        self.snapshot.auto_follow.completed_auto_turns = 0;
        if value > 0 {
            self.snapshot.auto_follow.continuation_paused = false;
        }
        if value == 0
            && !matches!(
                self.snapshot.auto_follow.phase,
                AutoFollowPhase::Submitting { .. } | AutoFollowPhase::Running { .. }
            )
        {
            self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
        }
    }

    pub(super) fn pause_post_turn_continuation(&mut self) {
        self.cancel_queued_auto_follow_submission();
        self.snapshot.auto_follow.continuation_paused = true;
        self.snapshot.auto_follow.parallel_rearmed_after_stop = false;
    }

    pub(super) fn set_parallel_post_turn_rearm(&mut self, rearmed: bool) {
        let preserves_single_session_continuation =
            !rearmed && self.snapshot.auto_follow.can_queue_next();
        if !preserves_single_session_continuation {
            self.cancel_queued_auto_follow_submission();
        }
        self.snapshot.auto_follow.parallel_rearmed_after_stop = rearmed;
    }

    pub(super) fn cancel_queued_auto_follow_submission(&mut self) {
        if matches!(
            self.snapshot.auto_follow.phase,
            AutoFollowPhase::Queued { .. }
        ) {
            self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
        }
    }

    pub(super) fn begin_post_turn_evaluation(
        &mut self,
        correlation: PostTurnEvaluationCorrelation,
    ) {
        self.pending_post_turn_route = None;
        self.snapshot.post_turn = PostTurnAuthoritySnapshot::Evaluating {
            correlation,
            started_at: Instant::now(),
        };
    }

    pub(super) fn await_post_turn_route(
        &mut self,
        correlation: PostTurnEvaluationCorrelation,
        execution: Box<PostTurnExecution>,
    ) -> bool {
        if !matches!(
            &self.snapshot.post_turn,
            PostTurnAuthoritySnapshot::Evaluating {
                correlation: active,
                ..
            } if active == &correlation
        ) {
            return false;
        }
        self.pending_post_turn_route = Some(PendingPostTurnRoute {
            correlation: correlation.clone(),
            execution,
        });
        self.snapshot.post_turn = PostTurnAuthoritySnapshot::AwaitingRoute {
            correlation,
            started_at: Instant::now(),
        };
        true
    }

    pub(super) fn resolve_post_turn_route(
        &mut self,
        correlation: &PostTurnEvaluationCorrelation,
        resolution: PostTurnRouteResolution,
    ) -> Option<(Box<PostTurnExecution>, PostTurnRouteResolution)> {
        if self
            .pending_post_turn_route
            .as_ref()
            .is_none_or(|pending| &pending.correlation != correlation)
        {
            return None;
        }
        let pending = self
            .pending_post_turn_route
            .take()
            .expect("exact pending post-turn route must remain present");
        let accepted_resolution = if resolution == PostTurnRouteResolution::AutoSubmit
            && !self.snapshot.auto_follow.can_queue_next()
        {
            PostTurnRouteResolution::NoContinuation
        } else {
            resolution
        };
        match accepted_resolution {
            PostTurnRouteResolution::AutoSubmit => {
                self.snapshot.planning_handoff =
                    pending.execution.evaluation.provenance.handoff_task.clone();
                let turn_index = self.snapshot.auto_follow.completed_auto_turns + 1;
                self.snapshot.auto_follow.phase = AutoFollowPhase::Queued {
                    turn_index,
                    started_at: Instant::now(),
                };
            }
            PostTurnRouteResolution::ParallelConsumed | PostTurnRouteResolution::NoContinuation => {
                self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
            }
        }
        self.snapshot.post_turn = PostTurnAuthoritySnapshot::Settled {
            correlation: correlation.clone(),
            resolution: accepted_resolution,
        };
        Some((pending.execution, accepted_resolution))
    }

    pub(super) fn cancel_post_turn_evaluation(&mut self) {
        self.pending_post_turn_route = None;
        self.snapshot.post_turn = PostTurnAuthoritySnapshot::Idle;
    }

    pub(super) fn settle_in_flight_post_turn_without_continuation(
        &mut self,
    ) -> Option<PostTurnEvaluationCorrelation> {
        let correlation = match &self.snapshot.post_turn {
            PostTurnAuthoritySnapshot::Evaluating { correlation, .. }
            | PostTurnAuthoritySnapshot::AwaitingRoute { correlation, .. } => correlation.clone(),
            PostTurnAuthoritySnapshot::Idle | PostTurnAuthoritySnapshot::Settled { .. } => {
                return None;
            }
        };
        self.pending_post_turn_route = None;
        self.snapshot.auto_follow.phase = AutoFollowPhase::Idle;
        self.snapshot.post_turn = PostTurnAuthoritySnapshot::Settled {
            correlation: correlation.clone(),
            resolution: PostTurnRouteResolution::NoContinuation,
        };
        Some(correlation)
    }
}

impl Default for ConversationRuntimeAuthority {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{
        ConversationApprovalDecision, ConversationApprovalRequestKind, ConversationTurnOptions,
    };
    use crate::domain::planning::{
        PlanningWorkerPanelState, PostTurnAutoFollowSkipReason, PostTurnContinuationAction,
        PostTurnOutcome, PostTurnProvenance, RuntimeProjection,
    };

    fn turn_request(origin: CorePromptOrigin) -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory: "/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "continue".to_string(),
            prompt_origin: origin,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: ConversationTurnOptions::default(),
            slot_lease_handoff: None,
        }
    }

    fn post_turn_correlation(generation: u64, turn_id: &str) -> PostTurnEvaluationCorrelation {
        PostTurnEvaluationCorrelation::new(
            generation,
            "thread-1",
            turn_id,
            "/workspace",
            "/workspace",
        )
    }

    fn post_turn_execution(turn_id: &str) -> Box<PostTurnExecution> {
        Box::new(PostTurnExecution {
            thread_id: "thread-1".to_string(),
            completed_turn_id: turn_id.to_string(),
            runtime_projection_workspace_directory: "/workspace".to_string(),
            evaluation: PostTurnOutcome {
                provenance: PostTurnProvenance::new(turn_id.to_string()),
                runtime_projection: RuntimeProjection::invalid("planning blocked"),
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
                },
                operator_alerts: Vec::new(),
            },
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        })
    }

    fn approval_request(approval_id: &str, server_request_id: &str) -> ConversationApprovalRequest {
        ConversationApprovalRequest {
            approval_id: approval_id.to_string(),
            server_request_id: server_request_id.to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "run a command".to_string(),
            details: Vec::new(),
        }
    }

    fn turn_is_admissible(
        authority: &ConversationRuntimeAuthority,
        request: &TurnSubmissionRequest,
    ) -> bool {
        authority.classify_turn_admission(request) == TurnAuthorityAdmission::Accepted
    }

    #[test]
    fn stale_turn_completion_and_workspace_change_preserve_the_newer_authority() {
        let mut authority = ConversationRuntimeAuthority::new();
        let first = TurnSubmissionCorrelation::new(1);
        let current = TurnSubmissionCorrelation::new(2);
        authority.begin_turn(first, &turn_request(CorePromptOrigin::Manual));
        authority.begin_turn(current, &turn_request(CorePromptOrigin::Manual));

        authority.finish_turn(first);
        assert_eq!(authority.active_turn_correlation(), Some(current));
        assert!(!authority.replace_active_turn_workspace(first, "/stale".to_string()));
        assert!(
            authority.replace_active_turn_workspace(current, "/current".to_string()),
            "the exact active generation should own its workspace projection"
        );
        assert_eq!(
            authority
                .snapshot()
                .active_turn
                .as_ref()
                .map(|turn| turn.workspace_directory.as_str()),
            Some("/current")
        );
    }

    #[test]
    fn post_turn_route_resolution_is_exact_once_and_aba_safe() {
        let mut authority = ConversationRuntimeAuthority::new();
        authority.set_auto_follow_max_turns(2);
        let first = post_turn_correlation(1, "turn-a");
        let current = post_turn_correlation(2, "turn-a");

        authority.begin_post_turn_evaluation(first.clone());
        assert!(authority.await_post_turn_route(first.clone(), post_turn_execution("turn-a"),));
        authority.begin_post_turn_evaluation(current.clone());
        assert!(
            authority
                .resolve_post_turn_route(&first, PostTurnRouteResolution::AutoSubmit)
                .is_none(),
            "an old A completion must not take a newer A route lease"
        );
        assert!(authority.await_post_turn_route(current.clone(), post_turn_execution("turn-a"),));

        let (_, resolution) = authority
            .resolve_post_turn_route(&current, PostTurnRouteResolution::AutoSubmit)
            .expect("the exact route should settle once");
        assert_eq!(resolution, PostTurnRouteResolution::AutoSubmit);
        assert!(
            authority
                .resolve_post_turn_route(&current, PostTurnRouteResolution::NoContinuation)
                .is_none(),
            "a duplicate route resolution must not emit another final completion"
        );
        assert!(matches!(
            authority.snapshot().post_turn,
            PostTurnAuthoritySnapshot::Settled {
                correlation,
                resolution: PostTurnRouteResolution::AutoSubmit,
            } if correlation == current
        ));
        let mut stale_submission = turn_request(CorePromptOrigin::AutoFollow);
        stale_submission.auto_follow_source = Some(first);
        assert!(
            !turn_is_admissible(&authority, &stale_submission),
            "an old A source must not consume the newer A queued lease"
        );
        stale_submission.auto_follow_source = Some(current);
        stale_submission.workspace_directory = "/wrong-workspace".to_string();
        assert_eq!(
            authority.classify_turn_admission(&stale_submission),
            TurnAuthorityAdmission::RejectedMalformedCurrentAutoFollowTarget
        );
        stale_submission.workspace_directory = "/workspace".to_string();
        assert!(turn_is_admissible(&authority, &stale_submission));
    }

    #[test]
    fn policy_settlement_cancels_an_exact_evaluation_or_pending_route_once() {
        let mut authority = ConversationRuntimeAuthority::new();
        let evaluating = post_turn_correlation(1, "turn-evaluating");
        authority.begin_post_turn_evaluation(evaluating.clone());

        assert_eq!(
            authority.settle_in_flight_post_turn_without_continuation(),
            Some(evaluating.clone())
        );
        assert!(matches!(
            authority.snapshot().post_turn,
            PostTurnAuthoritySnapshot::Settled {
                correlation,
                resolution: PostTurnRouteResolution::NoContinuation,
            } if correlation == evaluating
        ));
        assert!(
            authority
                .settle_in_flight_post_turn_without_continuation()
                .is_none(),
            "the same evaluation must settle only once"
        );

        let awaiting_route = post_turn_correlation(2, "turn-awaiting-route");
        authority.begin_post_turn_evaluation(awaiting_route.clone());
        assert!(authority.await_post_turn_route(
            awaiting_route.clone(),
            post_turn_execution("turn-awaiting-route"),
        ));
        assert_eq!(
            authority.settle_in_flight_post_turn_without_continuation(),
            Some(awaiting_route.clone())
        );
        assert!(
            authority
                .resolve_post_turn_route(&awaiting_route, PostTurnRouteResolution::AutoSubmit)
                .is_none(),
            "policy settlement must discard the stale pending route payload"
        );
    }

    #[test]
    fn auto_follow_budget_is_consumed_only_by_an_admitted_auto_turn() {
        let mut authority = ConversationRuntimeAuthority::new();
        authority.set_auto_follow_max_turns(1);
        let first_route = post_turn_correlation(1, "turn-manual");
        authority.begin_post_turn_evaluation(first_route.clone());
        assert!(
            authority
                .await_post_turn_route(first_route.clone(), post_turn_execution("turn-manual"),)
        );
        assert_eq!(
            authority
                .resolve_post_turn_route(&first_route, PostTurnRouteResolution::AutoSubmit)
                .map(|(_, resolution)| resolution),
            Some(PostTurnRouteResolution::AutoSubmit)
        );

        let auto_turn = TurnSubmissionCorrelation::new(1);
        let direct = turn_request(CorePromptOrigin::AutoFollow);
        assert!(
            !turn_is_admissible(&authority, &direct),
            "AutoFollow without the exact settled source must not bypass Core"
        );
        let mut admitted = direct;
        admitted.auto_follow_source = Some(first_route.clone());
        assert!(turn_is_admissible(&authority, &admitted));
        authority.begin_turn(auto_turn, &admitted);
        assert!(
            !turn_is_admissible(&authority, &admitted),
            "the exact source lease must be consumed by the first admitted turn"
        );
        authority.mark_turn_started(auto_turn, "turn-auto".to_string());
        let running = authority.snapshot();
        authority.mark_turn_started(auto_turn, "turn-auto".to_string());
        assert_eq!(
            authority.snapshot(),
            running,
            "a duplicate exact start must not restart the authoritative activity clock"
        );
        authority.mark_turn_started(auto_turn, "turn-forged".to_string());
        assert_eq!(
            authority.snapshot(),
            running,
            "a conflicting start must not replace the first accepted turn identity"
        );
        authority.finish_turn(auto_turn);
        assert_eq!(authority.snapshot().auto_follow.completed_auto_turns, 1);

        let exhausted_route = post_turn_correlation(2, "turn-auto");
        authority.begin_post_turn_evaluation(exhausted_route.clone());
        assert!(
            authority
                .await_post_turn_route(exhausted_route.clone(), post_turn_execution("turn-auto"),)
        );
        assert_eq!(
            authority
                .resolve_post_turn_route(&exhausted_route, PostTurnRouteResolution::AutoSubmit,)
                .map(|(_, resolution)| resolution),
            Some(PostTurnRouteResolution::NoContinuation),
            "Core must downgrade a stale AutoSubmit decision after the budget is exhausted"
        );
    }

    #[test]
    fn operator_pause_remains_sticky_while_parallel_rearm_is_scoped() {
        let mut authority = ConversationRuntimeAuthority::new();
        authority.set_auto_follow_max_turns(3);
        authority.pause_post_turn_continuation();
        assert!(authority.snapshot().auto_follow.continuation_paused);
        assert!(
            !authority
                .snapshot()
                .auto_follow
                .parallel_post_turn_continuation_allowed()
        );

        authority.set_parallel_post_turn_rearm(true);
        let snapshot = authority.snapshot();
        assert!(snapshot.auto_follow.continuation_paused);
        assert!(
            snapshot
                .auto_follow
                .parallel_post_turn_continuation_allowed()
        );
        assert!(!snapshot.auto_follow.can_queue_next());
    }

    #[test]
    fn policy_revision_cancels_only_the_queued_auto_follow_lease() {
        let mut authority = ConversationRuntimeAuthority::new();
        authority.set_auto_follow_max_turns(2);
        let source = post_turn_correlation(1, "turn-1");
        authority.begin_post_turn_evaluation(source.clone());
        assert!(authority.await_post_turn_route(source.clone(), post_turn_execution("turn-1")));
        assert!(
            authority
                .resolve_post_turn_route(&source, PostTurnRouteResolution::AutoSubmit)
                .is_some()
        );
        let mut request = turn_request(CorePromptOrigin::AutoFollow);
        request.auto_follow_source = Some(source);
        assert!(turn_is_admissible(&authority, &request));

        authority.pause_post_turn_continuation();
        assert!(matches!(
            authority.snapshot().auto_follow.phase,
            AutoFollowPhase::Idle
        ));
        assert!(!turn_is_admissible(&authority, &request));
        assert!(
            authority.snapshot().can_accept_manual_prompt(),
            "cancelling a queued lease must not leave manual input locked"
        );

        authority.set_auto_follow_max_turns(2);
        let revised_source = post_turn_correlation(2, "turn-2");
        authority.begin_post_turn_evaluation(revised_source.clone());
        assert!(
            authority.await_post_turn_route(revised_source.clone(), post_turn_execution("turn-2"),)
        );
        assert!(
            authority
                .resolve_post_turn_route(&revised_source, PostTurnRouteResolution::AutoSubmit)
                .is_some()
        );
        authority.set_auto_follow_max_turns(4);
        assert!(matches!(
            authority.snapshot().auto_follow.phase,
            AutoFollowPhase::Idle
        ));

        let running = TurnSubmissionCorrelation::new(9);
        let running_request = turn_request(CorePromptOrigin::AutoFollow);
        authority.snapshot.auto_follow.phase = AutoFollowPhase::Queued {
            turn_index: 1,
            started_at: Instant::now(),
        };
        authority.begin_turn(running, &running_request);
        authority.set_auto_follow_max_turns(5);
        assert!(matches!(
            authority.snapshot().auto_follow.phase,
            AutoFollowPhase::Submitting { .. }
        ));
    }

    #[test]
    fn duplicate_approval_request_preserves_the_active_decision_lease() {
        let mut authority = ConversationRuntimeAuthority::new();
        let original = approval_request("approval-1", "server-1");
        let identity = original.identity();
        let correlation = ApprovalDecisionCorrelation::new(
            1,
            TurnSubmissionCorrelation::new(4),
            identity.clone(),
            ConversationApprovalDecision::Accept,
        );
        authority.set_pending_approval(original);
        assert!(authority.begin_approval_decision(correlation.clone()));

        let mut duplicate = approval_request("approval-1", "server-1");
        duplicate.summary = "same request with refreshed presentation".to_string();
        authority.set_pending_approval(duplicate.clone());

        let approval = authority
            .snapshot()
            .approval
            .expect("the exact approval should remain active");
        assert_eq!(approval.request, duplicate);
        assert_eq!(approval.decision, Some(correlation));
        assert_eq!(approval.phase, ApprovalAuthorityPhase::Submitting);

        authority.set_pending_approval(approval_request("approval-1", "server-2"));
        let replacement = authority
            .snapshot()
            .approval
            .expect("a different full identity should replace the pending request");
        assert_eq!(replacement.request.server_request_id, "server-2");
        assert!(replacement.decision.is_none());
        assert_eq!(replacement.phase, ApprovalAuthorityPhase::Pending);
    }
}
