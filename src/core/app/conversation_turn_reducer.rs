use super::{
    ApprovalDecisionAdmission, ApprovalDecisionCorrelation, ConversationLoadCorrelation,
    PostTurnEvaluationCorrelation, SessionRenameCorrelation, StopRequestAdmission,
    StopRequestAttempt, StopRequestCorrelation, TurnSteerAdmission, TurnSteerCorrelation,
    TurnStreamEvent, TurnStreamSnapshot, TurnStreamState, TurnStreamUpdate,
    TurnSubmissionAdmission, TurnSubmissionCorrelation,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalReview, ConversationTurnSteerReceipt,
    ConversationTurnSteerRequest,
};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjection;
use crate::domain::planning::{PostTurnContinuationPermit, PostTurnExecution, PostTurnRequest};

#[derive(Debug, Clone)]
struct DeferredConversationLoadIntent {
    thread_id: String,
    fallback_workspace_directory: String,
}

#[derive(Debug, Clone)]
struct ActiveTurnSteer {
    correlation: TurnSteerCorrelation,
    expected_turn_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalDecisionPhase {
    Submitting,
    Submitted,
}

#[derive(Debug, Clone)]
struct ActiveApprovalDecision {
    correlation: ApprovalDecisionCorrelation,
    phase: ApprovalDecisionPhase,
}

#[derive(Debug, Clone, Copy)]
struct ActiveStopRequest {
    correlation: StopRequestCorrelation,
    pending_attempt: Option<StopRequestAttempt>,
    synchronize_after_turn_started: bool,
    invalidated: bool,
}

#[derive(Debug, Clone)]
struct ActivePostTurnEvaluation {
    correlation: PostTurnEvaluationCorrelation,
    continuation_permit: PostTurnContinuationPermit,
}

#[derive(Debug)]
pub(super) struct LoadedConversationStreamIdentity {
    pub(super) thread_id: String,
    pub(super) title: String,
    pub(super) workspace_directory: String,
    pub(super) item_lifecycle: ConversationItemLifecycleProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopEffectIntent {
    Request {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
    },
    Invalidate {
        correlation: StopRequestCorrelation,
    },
}

#[derive(Debug)]
pub(super) enum ConversationLoadAdmission {
    Deferred {
        stop_effects: Vec<StopEffectIntent>,
    },
    Started {
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
        stop_effects: Vec<StopEffectIntent>,
    },
}

#[derive(Debug)]
pub(super) struct ConversationLifecycleReduction {
    pub(super) stop_effects: Vec<StopEffectIntent>,
}

#[derive(Debug)]
pub(super) struct ApprovalReviewPersistenceIntent {
    pub(super) turn_submission: TurnSubmissionCorrelation,
    pub(super) workspace_directory: String,
    pub(super) thread_id: String,
    pub(super) review: ConversationApprovalReview,
}

#[derive(Debug)]
pub(super) struct TurnStreamReduction {
    pub(super) snapshots: Vec<TurnStreamSnapshot>,
    pub(super) stop_effects: Vec<StopEffectIntent>,
    pub(super) approval_review: Option<ApprovalReviewPersistenceIntent>,
}

#[derive(Debug)]
pub(super) struct StopRequestCompletionReduction {
    pub(super) publish_completion: bool,
    pub(super) settlement_finished: bool,
    pub(super) stop_effects: Vec<StopEffectIntent>,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationTurnFeatureReducer {
    turn_stream_state: TurnStreamState,
    guarded_session_rename_stream: Option<(TurnSubmissionCorrelation, SessionRenameCorrelation)>,
    next_conversation_load_generation: u64,
    in_flight_conversation_load: Option<ConversationLoadCorrelation>,
    deferred_conversation_load: Option<DeferredConversationLoadIntent>,
    next_turn_submission_generation: u64,
    active_turn_submission: Option<TurnSubmissionCorrelation>,
    next_post_turn_evaluation_generation: u64,
    in_flight_post_turn_evaluation: Option<ActivePostTurnEvaluation>,
    next_stop_request_generation: u64,
    active_stop_request: Option<ActiveStopRequest>,
    next_turn_steer_generation: u64,
    active_turn_steer: Option<ActiveTurnSteer>,
    next_approval_decision_generation: u64,
    active_approval_decision: Option<ActiveApprovalDecision>,
}

impl ConversationTurnFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            turn_stream_state: TurnStreamState::new(),
            guarded_session_rename_stream: None,
            next_conversation_load_generation: 1,
            in_flight_conversation_load: None,
            deferred_conversation_load: None,
            next_turn_submission_generation: 1,
            active_turn_submission: None,
            next_post_turn_evaluation_generation: 1,
            in_flight_post_turn_evaluation: None,
            next_stop_request_generation: 1,
            active_stop_request: None,
            next_turn_steer_generation: 1,
            active_turn_steer: None,
            next_approval_decision_generation: 1,
            active_approval_decision: None,
        }
    }

    pub(super) fn active_conversation_load_for_thread(
        &self,
        thread_id: &str,
    ) -> Option<ConversationLoadCorrelation> {
        self.in_flight_conversation_load
            .as_ref()
            .filter(|load| load.requested_thread_id == thread_id)
            .cloned()
    }

    pub(super) fn admit_conversation_load(
        &mut self,
        thread_id: String,
        fallback_workspace_directory: String,
        blocked_by_session_rename: bool,
    ) -> ConversationLoadAdmission {
        self.cancel_active_post_turn_evaluation();
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        if self.stop_request_settlement_pending() || blocked_by_session_rename {
            self.deferred_conversation_load = Some(DeferredConversationLoadIntent {
                thread_id,
                fallback_workspace_directory,
            });
            return ConversationLoadAdmission::Deferred { stop_effects };
        }

        self.deferred_conversation_load = None;
        self.active_turn_submission = None;
        self.active_stop_request = None;
        self.active_turn_steer = None;
        self.active_approval_decision = None;
        self.guarded_session_rename_stream = None;
        let correlation = ConversationLoadCorrelation::new(
            take_generation(
                &mut self.next_conversation_load_generation,
                "conversation load",
            ),
            thread_id,
        );
        self.in_flight_conversation_load = Some(correlation.clone());
        self.turn_stream_state = TurnStreamState::new();
        self.prune_post_turn_evaluation_for_lifecycle();
        ConversationLoadAdmission::Started {
            correlation,
            fallback_workspace_directory,
            stop_effects,
        }
    }

    pub(super) fn take_deferred_conversation_load(&mut self) -> Option<(String, String)> {
        self.deferred_conversation_load
            .take()
            .map(|intent| (intent.thread_id, intent.fallback_workspace_directory))
    }

    pub(super) fn reduce_conversation_invalidation(&mut self) -> ConversationLifecycleReduction {
        self.cancel_active_post_turn_evaluation();
        self.deferred_conversation_load = None;
        self.in_flight_conversation_load = None;
        self.active_turn_submission = None;
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        self.active_turn_steer = None;
        self.active_approval_decision = None;
        self.guarded_session_rename_stream = None;
        self.turn_stream_state = TurnStreamState::new();
        ConversationLifecycleReduction { stop_effects }
    }

    pub(super) fn complete_conversation_load(
        &mut self,
        correlation: &ConversationLoadCorrelation,
        loaded_identity: Option<LoadedConversationStreamIdentity>,
    ) -> Option<ConversationLifecycleReduction> {
        if self.in_flight_conversation_load.as_ref() != Some(correlation) {
            return None;
        }
        self.in_flight_conversation_load = None;
        self.active_turn_submission = None;
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        self.active_turn_steer = None;
        self.active_approval_decision = None;
        self.guarded_session_rename_stream = None;
        self.turn_stream_state = TurnStreamState::new();
        if let Some(identity) = loaded_identity {
            self.turn_stream_state.seed_loaded_thread_projection(
                identity.thread_id,
                identity.title,
                identity.workspace_directory,
                identity.item_lifecycle,
            );
        }
        self.prune_post_turn_evaluation_for_lifecycle();
        Some(ConversationLifecycleReduction { stop_effects })
    }

    pub(super) fn admit_turn_submission(&mut self) -> TurnSubmissionAdmission {
        if let Some(active_correlation) = self.active_turn_submission {
            return TurnSubmissionAdmission::RejectedActive { active_correlation };
        }
        if let Some(active_stop) = self
            .active_stop_request
            .filter(|active| active.pending_attempt.is_some())
        {
            return TurnSubmissionAdmission::RejectedStopPending {
                stop_correlation: active_stop.correlation,
            };
        }
        let correlation = TurnSubmissionCorrelation::new(take_generation(
            &mut self.next_turn_submission_generation,
            "turn submission",
        ));
        self.guarded_session_rename_stream = None;
        self.active_approval_decision = None;
        self.active_turn_submission = Some(correlation);
        self.turn_stream_state.begin_submission();
        self.prune_post_turn_evaluation_for_lifecycle();
        TurnSubmissionAdmission::Accepted { correlation }
    }

    pub(super) fn reduce_session_rename_projection(
        &mut self,
        correlation: &SessionRenameCorrelation,
    ) -> Option<TurnStreamSnapshot> {
        if self
            .turn_stream_state
            .matches_thread(&correlation.request.thread_id)
            && let Some(turn_correlation) = self.active_turn_submission
        {
            self.guarded_session_rename_stream = Some((turn_correlation, correlation.clone()));
        }
        self.turn_stream_state
            .apply_session_rename(&correlation.request.thread_id, &correlation.request.name)
    }

    pub(super) fn apply_correlated_turn_stream_event(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        mut event: TurnStreamEvent,
    ) -> Option<TurnStreamReduction> {
        if self.active_turn_submission != Some(correlation) {
            return None;
        }

        if let TurnStreamEvent::ThreadPrepared {
            thread_id, title, ..
        } = &mut event
            && let Some(guarded_rename) = self
                .guarded_session_rename_stream
                .as_ref()
                .filter(|(turn, _)| *turn == correlation)
                .map(|(_, rename)| rename.clone())
        {
            if thread_id == &guarded_rename.request.thread_id {
                *title = guarded_rename.request.name;
            }
            self.guarded_session_rename_stream = None;
        }
        let stream_snapshot = self.turn_stream_state.apply_stream_event(event);
        let closes_submission = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnCompleted { .. }
                | TurnStreamUpdate::TurnTerminal { .. }
                | TurnStreamUpdate::Failed { .. }
        );
        let rejected_terminal = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnTerminalIgnored { .. }
        );
        let turn_started = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnStarted { .. }
        );
        let retry_reopens_stop = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnRetrying {
                correlation_failure: None,
                ..
            }
        );
        let approval_review = match &stream_snapshot.update {
            TurnStreamUpdate::ApprovalReviewUpdated { review } => stream_snapshot
                .cwd
                .clone()
                .zip(stream_snapshot.thread_id.clone())
                .map(
                    |(workspace_directory, thread_id)| ApprovalReviewPersistenceIntent {
                        turn_submission: correlation,
                        workspace_directory,
                        thread_id,
                        review: review.clone(),
                    },
                ),
            _ => None,
        };
        let mut snapshots = vec![stream_snapshot];
        let mut stop_effects = Vec::new();
        if rejected_terminal {
            snapshots.push(
                self.turn_stream_state
                    .apply_stream_event(TurnStreamEvent::Failed {
                        message: "active turn returned a terminal receipt with mismatched identity"
                            .to_string(),
                    }),
            );
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.active_turn_submission = None;
            self.guarded_session_rename_stream = None;
        } else if closes_submission {
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.active_turn_submission = None;
            self.guarded_session_rename_stream = None;
        } else if retry_reopens_stop {
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
        } else if turn_started {
            self.schedule_stop_synchronization_after_turn_started(&mut stop_effects);
        }
        self.prune_post_turn_evaluation_for_lifecycle();
        if self
            .active_approval_decision
            .as_ref()
            .is_some_and(|active| {
                self.active_turn_submission != Some(active.correlation.turn_submission)
                    || !self
                        .turn_stream_state
                        .matches_pending_approval(&active.correlation.approval_id)
            })
        {
            self.active_approval_decision = None;
        }
        Some(TurnStreamReduction {
            snapshots,
            stop_effects,
            approval_review,
        })
    }

    pub(super) fn apply_runtime_notice(&mut self, notice: String) -> TurnStreamSnapshot {
        self.turn_stream_state.apply_runtime_notice(notice)
    }

    pub(super) fn apply_correlated_runtime_notice(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        notice: String,
    ) -> Option<TurnStreamSnapshot> {
        (self.active_turn_submission == Some(correlation))
            .then(|| self.turn_stream_state.apply_runtime_notice(notice))
    }

    pub(super) fn accepts_turn_workspace_change(
        &self,
        correlation: TurnSubmissionCorrelation,
    ) -> bool {
        self.active_turn_submission == Some(correlation)
    }

    pub(super) fn admit_stop_request(&mut self) -> StopRequestAdmission {
        if let Some(active) = self.active_stop_request {
            return StopRequestAdmission::RejectedActive {
                active_correlation: active.correlation,
            };
        }
        let correlation = StopRequestCorrelation::new(
            take_generation(
                &mut self.next_stop_request_generation,
                "runtime stop request",
            ),
            self.active_turn_submission,
        );
        self.active_stop_request = Some(ActiveStopRequest {
            correlation,
            pending_attempt: Some(StopRequestAttempt::Initial),
            synchronize_after_turn_started: correlation.turn_submission.is_some()
                && !self.turn_stream_state.has_active_turn(),
            invalidated: false,
        });
        StopRequestAdmission::Accepted { correlation }
    }

    pub(super) fn complete_stop_request(
        &mut self,
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        failed: bool,
    ) -> Option<StopRequestCompletionReduction> {
        if self.active_stop_request.is_none_or(|active| {
            active.correlation != correlation || active.pending_attempt != Some(attempt)
        }) {
            return None;
        }
        self.active_stop_request
            .as_mut()
            .expect("exact active stop request must remain present")
            .pending_attempt = None;
        let invalidated = self
            .active_stop_request
            .is_some_and(|active| active.invalidated);
        if failed || invalidated || correlation.turn_submission.is_none() {
            self.active_stop_request = None;
        }
        let mut stop_effects = Vec::new();
        if !failed && !invalidated {
            self.schedule_stop_synchronization_after_turn_started(&mut stop_effects);
        }
        Some(StopRequestCompletionReduction {
            publish_completion: !invalidated,
            settlement_finished: self.active_stop_request.is_none(),
            stop_effects,
        })
    }

    pub(super) fn stop_request_settlement_pending(&self) -> bool {
        self.active_stop_request
            .is_some_and(|active| active.pending_attempt.is_some())
    }

    pub(super) fn admit_turn_steer(
        &mut self,
        request: &ConversationTurnSteerRequest,
    ) -> TurnSteerAdmission {
        if let Some(active) = &self.active_turn_steer {
            return TurnSteerAdmission::RejectedActive {
                active_correlation: active.correlation,
            };
        }
        let Some(turn_submission) = self.active_turn_submission else {
            return TurnSteerAdmission::RejectedUnavailable;
        };
        if !self
            .turn_stream_state
            .matches_active_turn(&request.thread_id, &request.expected_turn_id)
        {
            return TurnSteerAdmission::RejectedUnavailable;
        }
        let correlation = TurnSteerCorrelation::new(
            take_generation(&mut self.next_turn_steer_generation, "active turn steer"),
            turn_submission,
        );
        self.active_turn_steer = Some(ActiveTurnSteer {
            correlation,
            expected_turn_id: request.expected_turn_id.clone(),
        });
        TurnSteerAdmission::Accepted { correlation }
    }

    pub(super) fn complete_turn_steer(
        &mut self,
        correlation: TurnSteerCorrelation,
        result: Result<ConversationTurnSteerReceipt, String>,
    ) -> Option<Result<ConversationTurnSteerReceipt, String>> {
        if self
            .active_turn_steer
            .as_ref()
            .is_none_or(|active| active.correlation != correlation)
        {
            return None;
        }
        let active = self
            .active_turn_steer
            .take()
            .expect("exact active turn steer must remain present");
        Some(result.and_then(|receipt| {
            if receipt.turn_id == active.expected_turn_id {
                Ok(receipt)
            } else {
                Err("turn steer provider returned a different turn".to_string())
            }
        }))
    }

    pub(super) fn admit_approval_decision(
        &mut self,
        approval_id: String,
        decision: ConversationApprovalDecision,
    ) -> ApprovalDecisionAdmission {
        if let Some(active) = &self.active_approval_decision {
            return ApprovalDecisionAdmission::RejectedActive {
                active_correlation: active.correlation.clone(),
            };
        }
        let Some(turn_submission) = self.active_turn_submission else {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        };
        if !self
            .turn_stream_state
            .matches_pending_approval(&approval_id)
        {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        }
        let correlation = ApprovalDecisionCorrelation::new(
            take_generation(
                &mut self.next_approval_decision_generation,
                "approval decision",
            ),
            turn_submission,
            approval_id,
            decision,
        );
        self.active_approval_decision = Some(ActiveApprovalDecision {
            correlation: correlation.clone(),
            phase: ApprovalDecisionPhase::Submitting,
        });
        ApprovalDecisionAdmission::Accepted { correlation }
    }

    pub(super) fn complete_approval_decision(
        &mut self,
        correlation: &ApprovalDecisionCorrelation,
        succeeded: bool,
    ) -> bool {
        if self.active_approval_decision.as_ref().is_none_or(|active| {
            active.correlation != *correlation || active.phase != ApprovalDecisionPhase::Submitting
        }) {
            return false;
        }
        if succeeded {
            self.active_approval_decision
                .as_mut()
                .expect("exact active approval decision must remain present")
                .phase = ApprovalDecisionPhase::Submitted;
        } else {
            self.active_approval_decision = None;
        }
        true
    }

    pub(super) fn admit_post_turn_evaluation(
        &mut self,
        request: &PostTurnRequest,
    ) -> Option<PostTurnEvaluationCorrelation> {
        self.prune_post_turn_evaluation_for_lifecycle();
        if self.in_flight_post_turn_evaluation.is_some()
            || !self.turn_stream_state.can_start_post_turn_evaluation(
                &request.context.thread_id,
                &request.completed_turn_id,
            )
        {
            return None;
        }
        let correlation = PostTurnEvaluationCorrelation::new(
            take_generation(
                &mut self.next_post_turn_evaluation_generation,
                "post-turn evaluation",
            ),
            request.context.thread_id.clone(),
            request.completed_turn_id.clone(),
            request.workspace_directory.clone(),
            request.context.planning_workspace_directory.clone(),
        );
        self.in_flight_post_turn_evaluation = Some(ActivePostTurnEvaluation {
            correlation: correlation.clone(),
            continuation_permit: request.continuation_permit.clone(),
        });
        Some(correlation)
    }

    pub(super) fn complete_post_turn_evaluation(
        &mut self,
        correlation: &PostTurnEvaluationCorrelation,
        execution: &PostTurnExecution,
    ) -> bool {
        if self.active_post_turn_evaluation_correlation() != Some(correlation)
            || !correlation.matches_execution(execution)
        {
            return false;
        }
        self.in_flight_post_turn_evaluation = None;
        if self.in_flight_conversation_load.is_some() {
            return false;
        }
        self.turn_stream_state
            .accept_post_turn_evaluation_completion(execution)
    }

    pub(super) fn active_post_turn_evaluation_correlation(
        &self,
    ) -> Option<&PostTurnEvaluationCorrelation> {
        self.in_flight_post_turn_evaluation
            .as_ref()
            .map(|active| &active.correlation)
    }

    pub(super) fn matches_conversation(&self, workspace_directory: &str, thread_id: &str) -> bool {
        self.turn_stream_state
            .matches_conversation(workspace_directory, thread_id)
    }

    fn prune_post_turn_evaluation_for_lifecycle(&mut self) {
        let should_prune = self
            .in_flight_post_turn_evaluation
            .as_ref()
            .is_some_and(|active| {
                !self.turn_stream_state.can_start_post_turn_evaluation(
                    &active.correlation.thread_id,
                    &active.correlation.completed_turn_id,
                )
            });
        if should_prune {
            self.cancel_active_post_turn_evaluation();
        }
    }

    fn cancel_active_post_turn_evaluation(&mut self) {
        if let Some(active) = self.in_flight_post_turn_evaluation.take() {
            active.continuation_permit.invalidate_if_current();
        }
    }

    fn schedule_stop_synchronization_after_turn_started(
        &mut self,
        stop_effects: &mut Vec<StopEffectIntent>,
    ) {
        let Some(active) = self.active_stop_request.as_mut() else {
            return;
        };
        if active.invalidated
            || !active.synchronize_after_turn_started
            || active.pending_attempt.is_some()
            || active.correlation.turn_submission != self.active_turn_submission
            || !self.turn_stream_state.has_active_turn()
        {
            return;
        }
        active.synchronize_after_turn_started = false;
        active.pending_attempt = Some(StopRequestAttempt::AfterTurnStarted);
        stop_effects.push(StopEffectIntent::Request {
            correlation: active.correlation,
            attempt: StopRequestAttempt::AfterTurnStarted,
        });
    }

    fn clear_stop_request_for_turn(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        stop_effects: &mut Vec<StopEffectIntent>,
    ) {
        if self
            .active_stop_request
            .is_none_or(|active| active.correlation.turn_submission != Some(correlation))
        {
            return;
        }
        self.invalidate_stop_request_for_lifecycle(stop_effects);
    }

    fn invalidate_stop_request_for_lifecycle(&mut self, stop_effects: &mut Vec<StopEffectIntent>) {
        let Some(active) = self.active_stop_request.as_mut() else {
            return;
        };
        if active.pending_attempt.is_none() {
            self.active_stop_request = None;
            return;
        }
        if active.invalidated {
            return;
        }
        active.invalidated = true;
        active.synchronize_after_turn_started = false;
        stop_effects.push(StopEffectIntent::Invalidate {
            correlation: active.correlation,
        });
    }

    #[cfg(test)]
    pub(super) fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        assert!(
            self.active_turn_submission.is_none(),
            "test turn submission must not supersede an active generation"
        );
        let TurnSubmissionAdmission::Accepted { correlation } = self.admit_turn_submission() else {
            panic!("test turn submission should be admitted");
        };
        correlation
    }

    #[cfg(test)]
    pub(super) fn begin_test_post_turn_evaluation(
        &mut self,
        thread_id: &str,
        completed_turn_id: &str,
        turn_workspace_directory: &str,
        planning_workspace_directory: &str,
    ) -> PostTurnEvaluationCorrelation {
        assert!(
            self.in_flight_post_turn_evaluation.is_none(),
            "test post-turn evaluation must not supersede an active lease"
        );
        assert!(
            self.turn_stream_state
                .can_start_post_turn_evaluation(thread_id, completed_turn_id),
            "test post-turn evaluation must target the latest confirmed completed turn"
        );
        let correlation = PostTurnEvaluationCorrelation::new(
            take_generation(
                &mut self.next_post_turn_evaluation_generation,
                "post-turn evaluation",
            ),
            thread_id,
            completed_turn_id,
            turn_workspace_directory,
            planning_workspace_directory,
        );
        self.in_flight_post_turn_evaluation = Some(ActivePostTurnEvaluation {
            correlation: correlation.clone(),
            continuation_permit: crate::domain::planning::PostTurnContinuationGate::default()
                .capture(),
        });
        correlation
    }

    #[cfg(test)]
    pub(super) fn active_turn_submission_for_test(&self) -> Option<TurnSubmissionCorrelation> {
        self.active_turn_submission
    }

    #[cfg(test)]
    pub(super) fn active_turn_steer_for_test(&self) -> Option<TurnSteerCorrelation> {
        self.active_turn_steer
            .as_ref()
            .map(|active| active.correlation)
    }

    #[cfg(test)]
    pub(super) fn active_approval_decision_for_test(&self) -> Option<&ApprovalDecisionCorrelation> {
        self.active_approval_decision
            .as_ref()
            .map(|active| &active.correlation)
    }

    #[cfg(test)]
    pub(super) fn approval_decision_is_submitted_for_test(&self) -> bool {
        self.active_approval_decision
            .as_ref()
            .is_some_and(|active| active.phase == ApprovalDecisionPhase::Submitted)
    }

    #[cfg(test)]
    pub(super) fn exhaust_stop_generation_for_test(&mut self) {
        self.next_stop_request_generation = u64::MAX;
    }

    #[cfg(test)]
    pub(super) fn exhaust_approval_generation_for_test(&mut self) {
        self.next_approval_decision_generation = u64::MAX;
    }

    #[cfg(test)]
    pub(super) fn exhaust_post_turn_generation_for_test(&mut self) {
        self.next_post_turn_evaluation_generation = u64::MAX;
    }
}

impl Default for ConversationTurnFeatureReducer {
    fn default() -> Self {
        Self::new()
    }
}

fn take_generation(next_generation: &mut u64, operation: &str) -> u64 {
    let generation = *next_generation;
    *next_generation = generation
        .checked_add(1)
        .unwrap_or_else(|| panic!("{operation} generation exhausted"));
    generation
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started_load(
        reducer: &mut ConversationTurnFeatureReducer,
        thread_id: &str,
    ) -> ConversationLoadCorrelation {
        let ConversationLoadAdmission::Started { correlation, .. } =
            reducer.admit_conversation_load(thread_id.to_string(), "/workspace".to_string(), false)
        else {
            panic!("conversation load should start");
        };
        correlation
    }

    #[test]
    fn conversation_load_completion_requires_the_latest_exact_generation_once() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        let first = started_load(&mut reducer, "thread-a");
        let second = started_load(&mut reducer, "thread-b");
        let current = started_load(&mut reducer, "thread-a");

        assert!(reducer.complete_conversation_load(&first, None).is_none());
        assert!(reducer.complete_conversation_load(&second, None).is_none());
        assert!(reducer.complete_conversation_load(&current, None).is_some());
        assert!(reducer.complete_conversation_load(&current, None).is_none());
    }

    #[test]
    fn deferred_load_and_invalidation_share_the_same_authority_slice() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        assert!(matches!(
            reducer.admit_conversation_load(
                "thread-a".to_string(),
                "/workspace-a".to_string(),
                true,
            ),
            ConversationLoadAdmission::Deferred { .. }
        ));
        assert_eq!(
            reducer.take_deferred_conversation_load(),
            Some(("thread-a".to_string(), "/workspace-a".to_string()))
        );

        let correlation = started_load(&mut reducer, "thread-a");
        reducer.reduce_conversation_invalidation();
        assert!(
            reducer
                .complete_conversation_load(&correlation, None)
                .is_none()
        );
        assert!(reducer.take_deferred_conversation_load().is_none());
    }

    #[test]
    fn turn_and_stop_admissions_cannot_bypass_each_other() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        let TurnSubmissionAdmission::Accepted { correlation: turn } =
            reducer.admit_turn_submission()
        else {
            panic!("first turn should start");
        };
        assert!(matches!(
            reducer.admit_turn_submission(),
            TurnSubmissionAdmission::RejectedActive {
                active_correlation
            } if active_correlation == turn
        ));

        let StopRequestAdmission::Accepted { correlation: stop } = reducer.admit_stop_request()
        else {
            panic!("first stop should start");
        };
        assert_eq!(stop.turn_submission, Some(turn));
        assert!(matches!(
            reducer.admit_stop_request(),
            StopRequestAdmission::RejectedActive {
                active_correlation
            } if active_correlation == stop
        ));
        assert!(matches!(
            reducer.admit_turn_submission(),
            TurnSubmissionAdmission::RejectedActive {
                active_correlation
            } if active_correlation == turn
        ));

        let mut idle = ConversationTurnFeatureReducer::new();
        let StopRequestAdmission::Accepted {
            correlation: idle_stop,
        } = idle.admit_stop_request()
        else {
            panic!("idle stop should start");
        };
        assert!(matches!(
            idle.admit_turn_submission(),
            TurnSubmissionAdmission::RejectedStopPending {
                stop_correlation
            } if stop_correlation == idle_stop
        ));

        let settlement = reducer
            .complete_stop_request(stop, StopRequestAttempt::Initial, true)
            .expect("exact stop completion should settle");
        assert!(settlement.publish_completion);
        assert!(settlement.settlement_finished);
        reducer.reduce_conversation_invalidation();
        assert!(matches!(
            reducer.admit_turn_submission(),
            TurnSubmissionAdmission::Accepted {
                correlation: TurnSubmissionCorrelation { generation: 2 }
            }
        ));
    }
}
