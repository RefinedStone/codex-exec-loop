use super::approval::{ApprovalReviewPersistenceCoordinator, ApprovalReviewPersistenceSettlement};
use super::conversation_runtime::{ConversationRuntimeAuthority, TurnAuthorityAdmission};
use super::{
    ApprovalDecisionAdmission, ApprovalDecisionCorrelation, ApprovalReviewPersistenceCorrelation,
    ConversationLoadCorrelation, ConversationRuntimeSnapshot, PostTurnEvaluationCorrelation,
    PostTurnRouteResolution, SessionRenameCorrelation, StopRequestAdmission, StopRequestAttempt,
    StopRequestCorrelation, TurnSteerAdmission, TurnSteerCorrelation, TurnStreamEvent,
    TurnStreamSnapshot, TurnStreamStartRejection, TurnStreamState, TurnStreamUpdate,
    TurnSubmissionAdmission, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequestIdentity,
    ConversationTurnSteerReceipt, ConversationTurnSteerRequest,
};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjection;
use crate::domain::planning::{
    PostTurnContinuationGate, PostTurnContinuationPermit, PostTurnExecution, PostTurnRequest,
};

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
pub(super) struct TurnStreamReduction {
    pub(super) snapshots: Vec<TurnStreamSnapshot>,
    pub(super) stop_effects: Vec<StopEffectIntent>,
    pub(super) approval_review_persistence: Option<ApprovalReviewPersistenceCorrelation>,
}

#[derive(Debug)]
pub(super) struct StopRequestCompletionReduction {
    pub(super) publish_completion: bool,
    pub(super) settlement_finished: bool,
    pub(super) stop_effects: Vec<StopEffectIntent>,
}

#[derive(Clone)]
pub(super) struct ConversationTurnFeatureReducer {
    turn_stream_state: TurnStreamState,
    conversation_runtime: ConversationRuntimeAuthority,
    guarded_session_rename_stream: Option<(TurnSubmissionCorrelation, SessionRenameCorrelation)>,
    next_conversation_load_generation: u64,
    in_flight_conversation_load: Option<ConversationLoadCorrelation>,
    deferred_conversation_load: Option<DeferredConversationLoadIntent>,
    next_turn_submission_generation: u64,
    next_post_turn_evaluation_generation: u64,
    post_turn_continuation_gate: PostTurnContinuationGate,
    in_flight_post_turn_evaluation: Option<ActivePostTurnEvaluation>,
    next_stop_request_generation: u64,
    active_stop_request: Option<ActiveStopRequest>,
    next_turn_steer_generation: u64,
    active_turn_steer: Option<ActiveTurnSteer>,
    next_approval_decision_generation: u64,
    approval_review_persistence: ApprovalReviewPersistenceCoordinator,
}

impl std::fmt::Debug for ConversationTurnFeatureReducer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConversationTurnFeatureReducer")
            .field("conversation_runtime", &self.conversation_runtime)
            .field(
                "in_flight_conversation_load",
                &self.in_flight_conversation_load,
            )
            .field(
                "in_flight_post_turn_evaluation",
                &self.in_flight_post_turn_evaluation,
            )
            .field("active_stop_request", &self.active_stop_request)
            .field("active_turn_steer", &self.active_turn_steer)
            .finish_non_exhaustive()
    }
}

impl ConversationTurnFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            turn_stream_state: TurnStreamState::new(),
            conversation_runtime: ConversationRuntimeAuthority::new(),
            guarded_session_rename_stream: None,
            next_conversation_load_generation: 1,
            in_flight_conversation_load: None,
            deferred_conversation_load: None,
            next_turn_submission_generation: 1,
            next_post_turn_evaluation_generation: 1,
            post_turn_continuation_gate: PostTurnContinuationGate::default(),
            in_flight_post_turn_evaluation: None,
            next_stop_request_generation: 1,
            active_stop_request: None,
            next_turn_steer_generation: 1,
            active_turn_steer: None,
            next_approval_decision_generation: 1,
            approval_review_persistence: ApprovalReviewPersistenceCoordinator::new(),
        }
    }

    pub(super) fn runtime_snapshot(&self) -> ConversationRuntimeSnapshot {
        self.conversation_runtime.snapshot()
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
        self.invalidate_post_turn_continuation();
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
        self.active_stop_request = None;
        self.active_turn_steer = None;
        self.conversation_runtime.invalidate_conversation();
        self.approval_review_persistence.invalidate_conversation();
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
        self.invalidate_post_turn_continuation();
        self.deferred_conversation_load = None;
        self.in_flight_conversation_load = None;
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        self.active_turn_steer = None;
        self.conversation_runtime.invalidate_conversation();
        self.approval_review_persistence.invalidate_conversation();
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
        self.invalidate_post_turn_continuation();
        self.in_flight_conversation_load = None;
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        self.active_turn_steer = None;
        self.conversation_runtime.invalidate_conversation();
        self.approval_review_persistence.invalidate_conversation();
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

    pub(super) fn admit_turn_submission(
        &mut self,
        request: &TurnSubmissionRequest,
    ) -> TurnSubmissionAdmission {
        if let Some(active_correlation) = self.conversation_runtime.active_turn_correlation() {
            if request.prompt_origin == super::CorePromptOrigin::AutoFollow {
                self.conversation_runtime
                    .cancel_queued_auto_follow_submission();
            }
            return TurnSubmissionAdmission::RejectedActive { active_correlation };
        }
        if let Some(active_stop) = self
            .active_stop_request
            .filter(|active| active.pending_attempt.is_some())
        {
            if request.prompt_origin == super::CorePromptOrigin::AutoFollow {
                self.conversation_runtime
                    .cancel_queued_auto_follow_submission();
            }
            return TurnSubmissionAdmission::RejectedStopPending {
                stop_correlation: active_stop.correlation,
            };
        }
        if matches!(
            request.prompt_origin,
            super::CorePromptOrigin::Manual | super::CorePromptOrigin::ManualIntake
        ) && self.turn_stream_state.has_unapplied_confirmed_terminal()
        {
            return TurnSubmissionAdmission::RejectedUnavailable;
        }
        match self.conversation_runtime.classify_turn_admission(request) {
            TurnAuthorityAdmission::Accepted => {}
            TurnAuthorityAdmission::RejectedPreservingLease => {
                return TurnSubmissionAdmission::RejectedUnavailable;
            }
            TurnAuthorityAdmission::RejectedMalformedCurrentAutoFollowTarget => {
                self.conversation_runtime
                    .cancel_queued_auto_follow_submission();
                return TurnSubmissionAdmission::RejectedUnavailable;
            }
        }
        let correlation = TurnSubmissionCorrelation::new(take_generation(
            &mut self.next_turn_submission_generation,
            "turn submission",
        ));
        self.guarded_session_rename_stream = None;
        self.conversation_runtime.begin_turn(correlation, request);
        self.approval_review_persistence
            .begin_conversation_turn(correlation);
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
            && let Some(turn_correlation) = self.conversation_runtime.active_turn_correlation()
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
        if self.conversation_runtime.active_turn_correlation() != Some(correlation) {
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
        match &stream_snapshot.update {
            TurnStreamUpdate::TurnStarted { turn_id, .. } => {
                self.conversation_runtime
                    .mark_turn_started(correlation, turn_id.clone());
            }
            TurnStreamUpdate::ApprovalRequested { request } => {
                self.conversation_runtime
                    .set_pending_approval(request.clone());
            }
            TurnStreamUpdate::ApprovalReviewUpdated { review } => {
                self.conversation_runtime
                    .set_approval_review(review.clone());
            }
            TurnStreamUpdate::ApprovalResolved {
                request_identity, ..
            } => {
                self.conversation_runtime
                    .clear_pending_approval(request_identity);
            }
            TurnStreamUpdate::AttachmentObserved { .. }
            | TurnStreamUpdate::SessionRenamed { .. }
            | TurnStreamUpdate::ThreadPrepared { .. }
            | TurnStreamUpdate::TurnStartedIgnored { .. }
            | TurnStreamUpdate::RuntimeEnvelopeObserved { .. }
            | TurnStreamUpdate::ItemLifecycleObserved { .. }
            | TurnStreamUpdate::ProgressiveActivityObserved { .. }
            | TurnStreamUpdate::StatusUpdated { .. }
            | TurnStreamUpdate::AgentMessageCompleted { .. }
            | TurnStreamUpdate::ToolActivity { .. }
            | TurnStreamUpdate::ApprovalResolutionIgnored { .. }
            | TurnStreamUpdate::TurnInterruptRequestFailed { .. }
            | TurnStreamUpdate::TurnRetrying { .. }
            | TurnStreamUpdate::TurnCompleted { .. }
            | TurnStreamUpdate::TurnTerminal { .. }
            | TurnStreamUpdate::TurnTerminalIgnored { .. }
            | TurnStreamUpdate::Failed { .. }
            | TurnStreamUpdate::RuntimeFailureIgnored { .. }
            | TurnStreamUpdate::RuntimeNotice { .. } => {}
        }
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
        let rejected_start = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnStartedIgnored {
                rejection: TurnStreamStartRejection::TurnMismatch { .. },
                ..
            }
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
        let approval_review_persistence = match &stream_snapshot.update {
            TurnStreamUpdate::ApprovalReviewUpdated { review } => stream_snapshot
                .cwd
                .clone()
                .zip(stream_snapshot.thread_id.clone())
                .and_then(|(workspace_directory, thread_id)| {
                    self.approval_review_persistence.enqueue(
                        correlation,
                        workspace_directory,
                        thread_id,
                        review.clone(),
                    )
                }),
            _ => None,
        };
        let mut snapshots = vec![stream_snapshot];
        let mut stop_effects = Vec::new();
        if rejected_start {
            snapshots.push(
                self.turn_stream_state
                    .apply_stream_event(TurnStreamEvent::Failed {
                        message: "active turn returned a start event with mismatched turn identity"
                            .to_string(),
                    }),
            );
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.conversation_runtime.finish_turn(correlation);
            self.guarded_session_rename_stream = None;
        } else if rejected_terminal {
            snapshots.push(
                self.turn_stream_state
                    .apply_stream_event(TurnStreamEvent::Failed {
                        message: "active turn returned a terminal receipt with mismatched identity"
                            .to_string(),
                    }),
            );
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.conversation_runtime.finish_turn(correlation);
            self.guarded_session_rename_stream = None;
        } else if closes_submission {
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.conversation_runtime.finish_turn(correlation);
            self.guarded_session_rename_stream = None;
        } else if retry_reopens_stop {
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
        } else if turn_started {
            self.schedule_stop_synchronization_after_turn_started(&mut stop_effects);
        }
        self.prune_post_turn_evaluation_for_lifecycle();
        Some(TurnStreamReduction {
            snapshots,
            stop_effects,
            approval_review_persistence,
        })
    }

    pub(super) fn complete_approval_review_persistence(
        &mut self,
        correlation: &ApprovalReviewPersistenceCorrelation,
    ) -> Option<ApprovalReviewPersistenceSettlement> {
        self.approval_review_persistence.complete(correlation)
    }

    pub(super) fn apply_runtime_notice(&mut self, notice: String) -> TurnStreamSnapshot {
        self.turn_stream_state.apply_runtime_notice(notice)
    }

    pub(super) fn apply_correlated_runtime_notice(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        notice: String,
    ) -> Option<TurnStreamSnapshot> {
        (self.conversation_runtime.active_turn_correlation() == Some(correlation))
            .then(|| self.turn_stream_state.apply_runtime_notice(notice))
    }

    pub(super) fn apply_turn_workspace_change(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        workspace_directory: String,
    ) -> bool {
        self.conversation_runtime
            .replace_active_turn_workspace(correlation, workspace_directory)
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
            self.conversation_runtime.active_turn_correlation(),
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
        let Some(turn_submission) = self.conversation_runtime.active_turn_correlation() else {
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
        request_identity: ConversationApprovalRequestIdentity,
        decision: ConversationApprovalDecision,
    ) -> ApprovalDecisionAdmission {
        if let Some(active) = self.conversation_runtime.active_approval_decision() {
            return ApprovalDecisionAdmission::RejectedActive {
                active_correlation: active.clone(),
            };
        }
        let Some(turn_submission) = self.conversation_runtime.active_turn_correlation() else {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        };
        if !self
            .turn_stream_state
            .matches_pending_approval(&request_identity)
        {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        }
        let correlation = ApprovalDecisionCorrelation::new(
            take_generation(
                &mut self.next_approval_decision_generation,
                "approval decision",
            ),
            turn_submission,
            request_identity,
            decision,
        );
        if !self
            .conversation_runtime
            .begin_approval_decision(correlation.clone())
        {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        }
        ApprovalDecisionAdmission::Accepted { correlation }
    }

    pub(super) fn complete_approval_decision(
        &mut self,
        correlation: &ApprovalDecisionCorrelation,
        succeeded: bool,
    ) -> bool {
        self.conversation_runtime
            .complete_approval_decision(correlation, succeeded)
    }

    pub(super) fn admit_post_turn_evaluation(
        &mut self,
        request: &mut PostTurnRequest,
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
        let runtime = self.conversation_runtime.snapshot();
        /*
         * The TUI may carry a compatibility copy while mapping the request,
         * but it is never authoritative. Bind evaluation to the handoff owned
         * by the exact admitted turn/post-turn route before any worker starts.
         */
        request.context.previous_handoff_task = runtime.planning_handoff.clone();
        let auto_follow = runtime.auto_follow;
        let parallel_continuation_enabled = request.context.parallel_mode_enabled
            && auto_follow.parallel_post_turn_continuation_allowed();
        request.context.planning_settlement_paused =
            auto_follow.continuation_paused && !parallel_continuation_enabled;
        request.context.continuation_paused = (!auto_follow.is_enabled()
            || auto_follow.continuation_paused)
            && !parallel_continuation_enabled;
        request.context.can_queue_next =
            auto_follow.can_queue_next() || parallel_continuation_enabled;
        request.context.stop_keyword = auto_follow.stop_keyword.clone();
        request.context.stop_keyword_matched = request
            .context
            .latest_main_reply
            .as_deref()
            .is_some_and(|reply| auto_follow.matches_stop_keyword(reply));
        request.context.no_file_changes_stop_matched =
            auto_follow.stop_on_no_file_changes && request.changed_planning_file_paths.is_empty();
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
        request.continuation_permit = self.post_turn_continuation_gate.capture();
        self.in_flight_post_turn_evaluation = Some(ActivePostTurnEvaluation {
            correlation: correlation.clone(),
            continuation_permit: request.continuation_permit.clone(),
        });
        self.conversation_runtime
            .begin_post_turn_evaluation(correlation.clone());
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
        if self.in_flight_conversation_load.is_some() {
            return false;
        }
        if !self
            .turn_stream_state
            .accept_post_turn_evaluation_completion(execution)
        {
            return false;
        }
        self.in_flight_post_turn_evaluation = None;
        self.conversation_runtime
            .await_post_turn_route(correlation.clone(), Box::new(execution.clone()))
    }

    pub(super) fn resolve_post_turn_route(
        &mut self,
        correlation: &PostTurnEvaluationCorrelation,
        resolution: PostTurnRouteResolution,
    ) -> Option<(Box<PostTurnExecution>, PostTurnRouteResolution)> {
        self.conversation_runtime
            .resolve_post_turn_route(correlation, resolution)
    }

    pub(super) fn set_auto_follow_max_turns(&mut self, value: usize) {
        self.settle_post_turn_continuation_for_policy_change();
        self.conversation_runtime.set_auto_follow_max_turns(value);
    }

    pub(super) fn pause_post_turn_continuation(&mut self) {
        self.settle_post_turn_continuation_for_policy_change();
        self.conversation_runtime.pause_post_turn_continuation();
    }

    pub(super) fn set_parallel_post_turn_rearm(&mut self, rearmed: bool) {
        /*
         * Turning parallel routing off must not revoke an independently
         * authorized single-session continuation. In that case the exact
         * worker may finish and NativeClientRuntime will resolve its route to
         * AutoSubmit after the parallel control-plane declines it. Every other
         * parallel policy change still settles the in-flight correlation before
         * changing authority, so a late worker cannot regain a route.
         */
        let preserves_single_session_continuation =
            !rearmed && self.runtime_snapshot().auto_follow.can_queue_next();
        if !preserves_single_session_continuation {
            self.settle_post_turn_continuation_for_policy_change();
        }
        self.conversation_runtime
            .set_parallel_post_turn_rearm(rearmed);
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
        self.conversation_runtime.cancel_post_turn_evaluation();
    }

    fn invalidate_post_turn_continuation(&mut self) {
        self.post_turn_continuation_gate.advance();
        self.cancel_active_post_turn_evaluation();
    }

    fn settle_post_turn_continuation_for_policy_change(&mut self) {
        self.post_turn_continuation_gate.advance();
        let active = self.in_flight_post_turn_evaluation.take();
        if let Some(active) = active.as_ref()
            && !self.turn_stream_state.settle_post_turn_terminal(
                &active.correlation.thread_id,
                &active.correlation.completed_turn_id,
            )
        {
            self.conversation_runtime.cancel_post_turn_evaluation();
            return;
        }
        let settled = self
            .conversation_runtime
            .settle_in_flight_post_turn_without_continuation();
        if let Some(active) = active {
            debug_assert_eq!(
                settled.as_ref(),
                Some(&active.correlation),
                "post-turn worker and runtime authority must settle the same correlation"
            );
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
            || active.correlation.turn_submission
                != self.conversation_runtime.active_turn_correlation()
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
            self.conversation_runtime
                .active_turn_correlation()
                .is_none(),
            "test turn submission must not supersede an active generation"
        );
        let request = TurnSubmissionRequest {
            workspace_directory: "/workspace".to_string(),
            thread_id: None,
            image_paths: Vec::new(),
            prompt: "test prompt".to_string(),
            prompt_origin: super::CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: crate::domain::conversation::ConversationTurnOptions::default(),
            slot_lease_handoff: None,
        };
        let TurnSubmissionAdmission::Accepted { correlation } =
            self.admit_turn_submission(&request)
        else {
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
        self.conversation_runtime
            .begin_post_turn_evaluation(correlation.clone());
        correlation
    }

    #[cfg(test)]
    pub(super) fn active_turn_submission_for_test(&self) -> Option<TurnSubmissionCorrelation> {
        self.conversation_runtime.active_turn_correlation()
    }

    #[cfg(test)]
    pub(super) fn active_turn_steer_for_test(&self) -> Option<TurnSteerCorrelation> {
        self.active_turn_steer
            .as_ref()
            .map(|active| active.correlation)
    }

    #[cfg(test)]
    pub(super) fn active_approval_decision_for_test(&self) -> Option<&ApprovalDecisionCorrelation> {
        self.conversation_runtime.active_approval_decision()
    }

    #[cfg(test)]
    pub(super) fn approval_decision_is_submitted_for_test(&self) -> bool {
        self.conversation_runtime
            .snapshot()
            .approval
            .as_ref()
            .is_some_and(|approval| approval.phase == super::ApprovalAuthorityPhase::Submitted)
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
    use crate::domain::planning::{
        PlanningWorkerPanelState, PostTurnAutoFollowSkipReason, PostTurnContext,
        PostTurnContinuationAction, PostTurnOutcome, PostTurnProvenance, RuntimeProjection,
        TaskHandoff,
    };

    fn turn_request() -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory: "/workspace".to_string(),
            thread_id: None,
            image_paths: Vec::new(),
            prompt: "test prompt".to_string(),
            prompt_origin: super::super::CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: crate::domain::conversation::ConversationTurnOptions::default(),
            slot_lease_handoff: None,
        }
    }

    fn queued_auto_follow_authority(reducer: &mut ConversationTurnFeatureReducer) {
        let correlation =
            PostTurnEvaluationCorrelation::new(1, "thread-1", "turn-1", "/workspace", "/workspace");
        let execution = Box::new(PostTurnExecution {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
            runtime_projection_workspace_directory: "/workspace".to_string(),
            evaluation: PostTurnOutcome {
                provenance: PostTurnProvenance::new("turn-1".to_string()),
                runtime_projection: RuntimeProjection::invalid("planning blocked"),
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
                },
                operator_alerts: Vec::new(),
            },
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        });
        reducer.conversation_runtime.set_auto_follow_max_turns(1);
        reducer
            .conversation_runtime
            .begin_post_turn_evaluation(correlation.clone());
        assert!(
            reducer
                .conversation_runtime
                .await_post_turn_route(correlation.clone(), execution)
        );
        assert!(
            reducer
                .conversation_runtime
                .resolve_post_turn_route(&correlation, PostTurnRouteResolution::AutoSubmit)
                .is_some()
        );
    }

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
            reducer.admit_turn_submission(&turn_request())
        else {
            panic!("first turn should start");
        };
        assert!(matches!(
            reducer.admit_turn_submission(&turn_request()),
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
            reducer.admit_turn_submission(&turn_request()),
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
            idle.admit_turn_submission(&turn_request()),
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
            reducer.admit_turn_submission(&turn_request()),
            TurnSubmissionAdmission::Accepted {
                correlation: TurnSubmissionCorrelation { generation: 2 }
            }
        ));
    }

    #[test]
    fn stop_pending_rejection_settles_a_queued_auto_follow_submission() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        queued_auto_follow_authority(&mut reducer);
        assert!(matches!(
            reducer.runtime_snapshot().auto_follow.phase,
            super::super::AutoFollowPhase::Queued { .. }
        ));
        let StopRequestAdmission::Accepted { correlation: stop } = reducer.admit_stop_request()
        else {
            panic!("the stop request should own the idle runtime");
        };
        let mut auto_turn = turn_request();
        auto_turn.prompt_origin = super::super::CorePromptOrigin::AutoFollow;

        assert!(matches!(
            reducer.admit_turn_submission(&auto_turn),
            TurnSubmissionAdmission::RejectedStopPending {
                stop_correlation
            } if stop_correlation == stop
        ));
        assert!(matches!(
            reducer.runtime_snapshot().auto_follow.phase,
            super::super::AutoFollowPhase::Idle
        ));
        assert!(reducer.runtime_snapshot().can_accept_manual_prompt());
    }

    #[test]
    fn malformed_exact_auto_follow_target_cancels_only_the_owned_queue_lease() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        queued_auto_follow_authority(&mut reducer);
        let source =
            PostTurnEvaluationCorrelation::new(1, "thread-1", "turn-1", "/workspace", "/workspace");
        let mut malformed = turn_request();
        malformed.prompt_origin = super::super::CorePromptOrigin::AutoFollow;
        malformed.thread_id = Some("thread-1".to_string());
        malformed.workspace_directory = "/wrong-workspace".to_string();
        malformed.auto_follow_source = Some(source);

        assert_eq!(
            reducer.admit_turn_submission(&malformed),
            TurnSubmissionAdmission::RejectedUnavailable
        );
        assert!(matches!(
            reducer.runtime_snapshot().auto_follow.phase,
            super::super::AutoFollowPhase::Idle
        ));
        assert!(
            reducer.runtime_snapshot().can_accept_manual_prompt(),
            "a malformed request that owns the current lease must not leave manual input locked"
        );
    }

    #[test]
    fn duplicate_turn_start_is_ignored_and_conflicting_start_closes_the_generation() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        let TurnSubmissionAdmission::Accepted { correlation } =
            reducer.admit_turn_submission(&turn_request())
        else {
            panic!("manual turn should be admitted");
        };
        reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Core stream".to_string(),
                    cwd: "/workspace".to_string(),
                    runtime_envelope: Box::default(),
                },
            )
            .expect("thread preparation should be correlated");
        reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-1".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("first turn start should be accepted");
        let running = reducer.runtime_snapshot();

        let duplicate = reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-1".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("duplicate start should be reported as ignored");
        assert!(matches!(
            duplicate.snapshots.as_slice(),
            [TurnStreamSnapshot {
                update: TurnStreamUpdate::TurnStartedIgnored {
                    rejection: TurnStreamStartRejection::Duplicate,
                    ..
                },
                ..
            }]
        ));
        assert_eq!(reducer.runtime_snapshot(), running);

        let conflicting = reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-forged".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("conflicting start should fail the active generation closed");
        assert!(matches!(
            conflicting.snapshots.as_slice(),
            [
                TurnStreamSnapshot {
                    update: TurnStreamUpdate::TurnStartedIgnored {
                        rejection: TurnStreamStartRejection::TurnMismatch { .. },
                        ..
                    },
                    ..
                },
                TurnStreamSnapshot {
                    update: TurnStreamUpdate::Failed { .. },
                    ..
                },
            ]
        ));
        assert!(reducer.runtime_snapshot().active_turn.is_none());
    }

    #[test]
    fn rejected_parallel_intake_cannot_replace_the_completed_turn_handoff() {
        fn handoff(task_id: &str) -> TaskHandoff {
            TaskHandoff {
                task_id: task_id.to_string(),
                task_title: format!("Task {task_id}"),
                direction_id: "direction-1".to_string(),
                combined_priority: 10,
                updated_at: "2026-07-27T00:00:00Z".to_string(),
                status_label: "Ready".to_string(),
            }
        }

        let mut reducer = ConversationTurnFeatureReducer::new();
        let task_a = handoff("task-a");
        let task_b = handoff("task-b");
        let mut turn_a = turn_request();
        turn_a.prompt_origin = super::super::CorePromptOrigin::ManualIntake;
        turn_a.planning_handoff = Some(task_a.clone());
        let TurnSubmissionAdmission::Accepted {
            correlation: turn_a_correlation,
        } = reducer.admit_turn_submission(&turn_a)
        else {
            panic!("manual intake A should be admitted");
        };

        let mut parallel_intake_b = turn_request();
        parallel_intake_b.prompt_origin = super::super::CorePromptOrigin::ManualIntake;
        parallel_intake_b.planning_handoff = Some(task_b.clone());
        assert!(matches!(
            reducer.admit_turn_submission(&parallel_intake_b),
            TurnSubmissionAdmission::RejectedActive { .. }
        ));
        assert_eq!(
            reducer.runtime_snapshot().planning_handoff,
            Some(task_a.clone())
        );

        reducer
            .apply_correlated_turn_stream_event(
                turn_a_correlation,
                TurnStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Core stream".to_string(),
                    cwd: "/workspace".to_string(),
                    runtime_envelope: Box::default(),
                },
            )
            .expect("thread preparation should be correlated");
        reducer
            .apply_correlated_turn_stream_event(
                turn_a_correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-a".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("turn A should start");
        reducer
            .apply_correlated_turn_stream_event(
                turn_a_correlation,
                TurnStreamEvent::TurnTerminal {
                    receipt:
                        crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
                            "thread-1",
                            "turn-a",
                            Vec::new(),
                        )
                        .with_application_delivery(
                            crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
                        ),
                    execution_snapshot_capture: None,
                },
            )
            .expect("turn A terminal should settle");

        let mut post_turn = PostTurnRequest {
            context: PostTurnContext {
                thread_id: "thread-1".to_string(),
                planning_workspace_directory: "/workspace".to_string(),
                latest_user_message: None,
                latest_main_reply: None,
                previous_handoff_task: Some(task_b),
                current_runtime_projection: RuntimeProjection::invalid("refresh required"),
                parallel_mode_enabled: true,
                parallel_automation_epoch_id: Some(1),
                planning_settlement_paused: false,
                continuation_paused: false,
                can_queue_next: false,
                stop_keyword: ":stop".to_string(),
                stop_keyword_matched: false,
                no_file_changes_stop_matched: false,
                mode_label: "test".to_string(),
            },
            workspace_directory: "/workspace".to_string(),
            completed_turn_id: "turn-a".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
            continuation_permit: PostTurnContinuationGate::default().capture(),
        };

        assert!(reducer.admit_post_turn_evaluation(&mut post_turn).is_some());
        assert_eq!(post_turn.context.previous_handoff_task, Some(task_a));
    }
}
