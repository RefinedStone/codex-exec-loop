use crate::core::app::{
    AppCommand, AppSnapshot, CoreController, CoreDispatchOutcome, CoreEffect, CoreInput,
    ParallelModeProjection, RevisionedPlanningParallelProjection,
};

use super::input_mailbox::CoreInputReceiver;

/*
 * CoreRuntime is the headless command loop around CoreController. Inbound
 * adapters can submit commands, while background workers send CoreInput back
 * through the queue so all app state changes still pass through the controller.
 */
pub struct CoreRuntime<E> {
    controller: CoreController,
    effect_executor: E,
    input_receiver: CoreInputReceiver,
}

pub trait CoreEffectExecutor {
    // Bounded local effects may return an immediate completion when their mutation must remain
    // serialized with the command dispatch. Long-running provider work returns None and reports
    // completion through CoreInputSender instead.
    fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput>;
}

impl<E> CoreRuntime<E>
where
    E: CoreEffectExecutor,
{
    pub fn new(effect_executor: E, input_receiver: CoreInputReceiver) -> Self {
        Self::from_parts(CoreController::new(), effect_executor, input_receiver)
    }

    fn from_parts(
        controller: CoreController,
        effect_executor: E,
        input_receiver: CoreInputReceiver,
    ) -> Self {
        Self {
            controller,
            effect_executor,
            input_receiver,
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.controller.snapshot()
    }

    pub fn revisioned_planning_parallel_projection(&self) -> RevisionedPlanningParallelProjection {
        self.controller.revisioned_planning_parallel_projection()
    }

    pub fn parallel_mode_projection(&self) -> ParallelModeProjection {
        self.controller.parallel_mode_projection()
    }

    pub fn dispatch_command(&mut self, command: AppCommand) -> CoreDispatchOutcome {
        self.dispatch_input(CoreInput::Command(command))
    }

    #[cfg(test)]
    pub(crate) fn drain_pending_inputs(&mut self, max_inputs: usize) -> Vec<CoreDispatchOutcome> {
        let mut outcomes = Vec::new();

        for _ in 0..max_inputs {
            let Some(outcome) = self.poll_pending_input() else {
                break;
            };
            outcomes.push(outcome);
        }

        outcomes
    }

    pub fn poll_pending_input(&mut self) -> Option<CoreDispatchOutcome> {
        let input = self.input_receiver.try_recv().ok()?;
        Some(self.dispatch_input(input))
    }

    pub fn dispatch_input(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        let mut outcome = self.controller.handle_input(input);
        let effects = outcome.effects.clone();
        for effect in effects {
            let Some(immediate_input) = self.effect_executor.run_effect(effect) else {
                continue;
            };
            let immediate_outcome = self.dispatch_input(immediate_input);
            outcome.events.extend(immediate_outcome.events);
            outcome.effects.extend(immediate_outcome.effects);
            outcome.snapshot = immediate_outcome.snapshot;
        }
        outcome
    }

    #[cfg(test)]
    pub(crate) fn begin_test_turn_submission(
        &mut self,
    ) -> crate::core::app::TurnSubmissionCorrelation {
        self.controller.begin_test_turn_submission()
    }

    #[cfg(test)]
    pub(crate) fn begin_test_post_turn_evaluation(
        &mut self,
        thread_id: &str,
        completed_turn_id: &str,
    ) {
        self.controller
            .begin_test_post_turn_evaluation(thread_id, completed_turn_id);
    }

    #[cfg(test)]
    pub(crate) fn test_post_turn_evaluation_is_in_flight(
        &self,
        thread_id: &str,
        completed_turn_id: &str,
    ) -> bool {
        self.controller
            .test_post_turn_evaluation_is_in_flight(thread_id, completed_turn_id)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc::TrySendError;

    use super::*;
    use crate::core::app::{
        AppEvent, ApprovalDecisionAdmission, ApprovalDecisionCorrelation, CoreEffectCompletion,
        CorePromptOrigin, GithubReviewPollCorrelation, GithubReviewPollingSetupCorrelation,
        GithubReviewPollingSetupMode, GithubReviewPollingSetupRequest,
        GithubReviewPollingSetupResult, ManualPromptPreparationAdmission,
        ManualPromptPreparationIntent, PlanningRuntimeRefreshCorrelation,
        PlanningRuntimeRefreshSnapshot, QueueAuthorityLoadCorrelation, QueueAuthoritySnapshot,
        QueueMutationCommitSnapshot, QueueMutationCorrelation, QueueMutationIntent,
        QueueMutationKind, QueueMutationResult, QueueMutationTarget, ReviewCenterLoadCorrelation,
        ReviewCenterSnapshot, SessionRenameAdmission, SessionRenameCorrelation,
        StartupAttachmentSnapshot, StartupCheckCorrelation, StartupDiagnosticSnapshot,
        StartupReadySnapshot, StartupSnapshot, StopRequestAdmission, StopRequestAttempt,
        StopRequestCorrelation, TurnSteerAdmission, TurnSteerCorrelation, TurnStreamEvent,
        TurnSubmissionAdmission, TurnSubmissionCorrelation, TurnSubmissionRequest,
    };
    use crate::core::runtime::input_mailbox::{CORE_INPUT_CHANNEL_CAPACITY, core_input_channel};
    use crate::domain::conversation::{
        ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalRequestKind,
        ConversationTurnSteerRequest,
    };
    use crate::domain::github_review::{
        GithubPullRequestActivitySnapshot, GithubPullRequestPollResult, GithubPullRequestPollState,
        GithubPullRequestTarget,
    };
    use crate::domain::planning::{
        ManualPromptCorrelation, ManualPromptRequest, PlanningWorkerPanelState,
        PlanningWorkerStatus, PostTurnAutoFollowSkipReason, PostTurnContext,
        PostTurnContinuationAction, PostTurnContinuationGate, PostTurnExecution, PostTurnOutcome,
        PostTurnProvenance, PostTurnRequest, RuntimeProjection, TaskStatus,
    };
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    #[test]
    fn core_input_channel_applies_backpressure_and_reports_disconnect() {
        let (sender, receiver) = core_input_channel();
        for _ in 0..CORE_INPUT_CHANNEL_CAPACITY {
            sender
                .try_send(CoreInput::Command(AppCommand::Noop))
                .expect("inputs within the fixed capacity should be admitted");
        }
        assert!(matches!(
            sender.try_send(CoreInput::Command(AppCommand::Noop)),
            Err(TrySendError::Full(_))
        ));
        drop(receiver);
        assert!(sender.send(CoreInput::Command(AppCommand::Noop)).is_err());
    }

    #[derive(Clone, Default)]
    struct RecordingEffectExecutor {
        effects: Rc<RefCell<Vec<CoreEffect>>>,
    }

    impl RecordingEffectExecutor {
        fn recorded_effects(&self) -> Vec<CoreEffect> {
            self.effects.borrow().clone()
        }
    }

    impl CoreEffectExecutor for RecordingEffectExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            self.effects.borrow_mut().push(effect);
            None
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateManualPromptExecutor;

    impl CoreEffectExecutor for ImmediateManualPromptExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::PrepareManualPrompt(request) = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::ManualPromptPrepared(Box::new(
                    crate::domain::planning::ManualPromptOutcome::Rejected {
                        correlation: request.correlation,
                        transcript_text: request.raw_prompt,
                        runtime_projection: Box::new(
                            crate::domain::planning::RuntimeProjection::invalid("blocked"),
                        ),
                        reason: "blocked".to_string(),
                    },
                )),
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediatePostTurnExecutor;

    impl CoreEffectExecutor for ImmediatePostTurnExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::EvaluatePostTurn(request) = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted(Box::new(PostTurnExecution {
                    thread_id: request.context.thread_id.clone(),
                    completed_turn_id: request.completed_turn_id.clone(),
                    runtime_projection_workspace_directory: request.workspace_directory.clone(),
                    evaluation: PostTurnOutcome {
                        provenance: PostTurnProvenance::new(request.completed_turn_id.clone()),
                        runtime_projection: request.context.current_runtime_projection.clone(),
                        planning_repair_state: None,
                        runtime_notices: Vec::new(),
                        action: PostTurnContinuationAction::SkipAutoFollow {
                            reason: PostTurnAutoFollowSkipReason::PlanningQueueDrained,
                        },
                        operator_alerts: Vec::new(),
                    },
                    planning_worker_panel_state: request.planning_worker_panel_state,
                })),
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateReviewCenterExecutor;

    impl CoreEffectExecutor for ImmediateReviewCenterExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::LoadReviewCenter { correlation } = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::ReviewCenterLoaded {
                    correlation,
                    snapshot: ReviewCenterSnapshot {
                        current_thread_reviews: Ok(Vec::new()),
                        pending_inbox: Ok(Vec::new()),
                        recent_history: Ok(Vec::new()),
                    },
                },
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateQueueAuthorityExecutor;

    impl CoreEffectExecutor for ImmediateQueueAuthorityExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::LoadQueueAuthority { correlation } = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::QueueAuthorityLoaded {
                    correlation,
                    result: Ok(Box::new(QueueAuthoritySnapshot {
                        runtime_projection:
                            crate::domain::planning::RuntimeProjection::uninitialized(),
                        planning_revision: 0,
                        tasks: Vec::new(),
                    })),
                },
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediatePlanningRuntimeExecutor;

    impl CoreEffectExecutor for ImmediatePlanningRuntimeExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::LoadPlanningRuntime { correlation } = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::PlanningRuntimeLoaded {
                    correlation,
                    result: Ok(Box::new(PlanningRuntimeRefreshSnapshot::new(
                        crate::domain::planning::RuntimeProjection::invalid("loaded"),
                    ))),
                },
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateQueueMutationExecutor;

    impl CoreEffectExecutor for ImmediateQueueMutationExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::ExecuteQueueMutation { correlation } = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::QueueMutationCompleted {
                    correlation,
                    result: Box::new(QueueMutationResult {
                        mutation: Ok(QueueMutationCommitSnapshot {
                            committed_planning_revision: 8,
                            committed_task_ids: vec!["task-1".to_string()],
                        }),
                        authority: Ok(QueueAuthoritySnapshot {
                            runtime_projection:
                                crate::domain::planning::RuntimeProjection::uninitialized(),
                            planning_revision: 8,
                            tasks: Vec::new(),
                        }),
                    }),
                },
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateSessionRenameExecutor;

    impl CoreEffectExecutor for ImmediateSessionRenameExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::RenameSession { correlation } = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::SessionRenamed {
                    correlation,
                    result: Ok(()),
                },
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateStopRequestExecutor;

    impl CoreEffectExecutor for ImmediateStopRequestExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::RequestStopAllSessions {
                correlation,
                attempt,
            } = effect
            else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::StopRequestAttemptCompleted {
                    correlation,
                    attempt,
                    result: Ok(()),
                },
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateApprovalDecisionExecutor;

    impl CoreEffectExecutor for ImmediateApprovalDecisionExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            let CoreEffect::SubmitApprovalDecision { correlation } = effect else {
                return None;
            };
            Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::ApprovalDecisionSubmitted {
                    correlation,
                    result: Ok(()),
                },
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateGithubReviewPollExecutor;

    impl CoreEffectExecutor for ImmediateGithubReviewPollExecutor {
        fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
            match effect {
                CoreEffect::SetupGithubReviewPolling {
                    correlation,
                    request,
                } => {
                    let target = request
                        .mode
                        .explicit_target()
                        .expect("test setup should be explicit")
                        .clone();
                    Some(CoreInput::EffectCompleted(
                        CoreEffectCompletion::GithubReviewPollingSetupCompleted {
                            correlation,
                            result: Ok(GithubReviewPollingSetupResult::Active { target }),
                        },
                    ))
                }
                CoreEffect::PollGithubReview { correlation, .. } => {
                    let target = correlation.target.clone();
                    Some(CoreInput::EffectCompleted(
                        CoreEffectCompletion::GithubReviewPollCompleted {
                            correlation,
                            result: Ok(Box::new(GithubPullRequestPollResult {
                                snapshot: GithubPullRequestActivitySnapshot {
                                    target,
                                    title: "Review poll".to_string(),
                                    url: "https://github.com/acme/widgets/pull/42".to_string(),
                                    head_branch: "feature".to_string(),
                                    base_branch: "prerelease".to_string(),
                                    events: Vec::new(),
                                },
                                changes: Vec::new(),
                                next_state: GithubPullRequestPollState {
                                    latest_submitted_at: Some("2026-07-19T10:00:00Z".to_string()),
                                    seen_events_at_latest_timestamp: Vec::new(),
                                },
                            })),
                        },
                    ))
                }
                _ => None,
            }
        }
    }

    #[test]
    fn dispatch_command_updates_state_and_runs_returned_effects() {
        let (_tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);

        let outcome = runtime.dispatch_command(AppCommand::RunStartupChecks {
            workspace_directory: "/tmp/workspace".to_string(),
        });

        assert_eq!(outcome.snapshot.startup, StartupSnapshot::Loading);
        assert_eq!(
            outcome.events,
            vec![AppEvent::StartupChanged {
                correlation: StartupCheckCorrelation::new(1, "/tmp/workspace"),
                snapshot: StartupSnapshot::Loading,
            }]
        );
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::RunStartupChecks {
                correlation: StartupCheckCorrelation::new(1, "/tmp/workspace"),
            }]
        );
        assert_eq!(runtime.snapshot().startup, StartupSnapshot::Loading);
    }

    #[test]
    fn immediate_session_rename_emits_admission_before_completion() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateSessionRenameExecutor, rx);
        let correlation = SessionRenameCorrelation::new(
            1,
            crate::domain::recent_sessions::SessionRenameRequest::new("thread-1", "Renamed"),
        );

        let outcome =
            runtime.dispatch_command(AppCommand::RenameSession(correlation.request.clone()));

        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::SessionRenameAdmissionResolved(SessionRenameAdmission::Accepted {
                    correlation: admitted,
                }),
                AppEvent::SessionRenameCompleted {
                    correlation: completed,
                    result: Ok(_),
                },
            ] if admitted == &correlation && completed == &correlation
        ));
    }

    #[test]
    fn submit_turn_admission_runs_exactly_one_worker_until_the_active_turn_closes() {
        let (_tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
        let request = TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        };

        let first = runtime.dispatch_command(AppCommand::SubmitTurn(request.clone()));
        let rejected = runtime.dispatch_command(AppCommand::SubmitTurn(request.clone()));

        assert_eq!(
            first.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                },
            )]
        );
        assert_eq!(
            rejected.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::RejectedActive {
                    active_correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                },
            )]
        );
        assert!(rejected.effects.is_empty());
        assert_eq!(*first.snapshot, AppSnapshot::initial());
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::SubmitTurn {
                correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                request: request.clone(),
            }]
        );

        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
            event: TurnStreamEvent::Failed {
                message: "worker stopped before turn/start".to_string(),
            },
        });
        let retried = runtime.dispatch_command(AppCommand::SubmitTurn(request.clone()));

        assert_eq!(
            retried.events,
            vec![AppEvent::TurnSubmissionAdmissionResolved(
                TurnSubmissionAdmission::Accepted {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(2),
                },
            )]
        );
        assert_eq!(
            effects.recorded_effects(),
            vec![
                CoreEffect::SubmitTurn {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
                    request: request.clone(),
                },
                CoreEffect::SubmitTurn {
                    correlation: crate::core::app::TurnSubmissionCorrelation::new(2),
                    request,
                },
            ]
        );
    }

    #[test]
    fn steer_turn_admission_runs_exactly_one_worker_until_completion() {
        let (_tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
        let submission_request = TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        };
        let turn_submission = crate::core::app::TurnSubmissionCorrelation::new(1);
        runtime.dispatch_command(AppCommand::SubmitTurn(submission_request));
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core runtime".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        });
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        });
        let request = ConversationTurnSteerRequest {
            thread_id: "thread-1".to_string(),
            expected_turn_id: "turn-1".to_string(),
            prompt: "focus the active work".to_string(),
        };

        let accepted = runtime.dispatch_command(AppCommand::SteerTurn(request.clone()));
        let rejected = runtime.dispatch_command(AppCommand::SteerTurn(request.clone()));
        let correlation = TurnSteerCorrelation::new(1, turn_submission);
        assert_eq!(
            accepted.events,
            vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::Accepted { correlation },
            )]
        );
        assert_eq!(
            rejected.events,
            vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::RejectedActive {
                    active_correlation: correlation,
                },
            )]
        );
        assert_eq!(
            effects
                .recorded_effects()
                .into_iter()
                .filter(|effect| matches!(effect, CoreEffect::SteerTurn { .. }))
                .collect::<Vec<_>>(),
            vec![CoreEffect::SteerTurn {
                correlation,
                request: request.clone(),
            }]
        );

        runtime.dispatch_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::TurnSteered {
                correlation,
                result: Ok(crate::domain::conversation::ConversationTurnSteerReceipt {
                    turn_id: "turn-1".to_string(),
                }),
            },
        ));
        let retried = runtime.dispatch_command(AppCommand::SteerTurn(request.clone()));
        let retry_correlation = TurnSteerCorrelation::new(2, turn_submission);
        assert_eq!(
            retried.events,
            vec![AppEvent::TurnSteerAdmissionResolved(
                TurnSteerAdmission::Accepted {
                    correlation: retry_correlation,
                },
            )]
        );
        assert_eq!(
            effects
                .recorded_effects()
                .into_iter()
                .filter(|effect| matches!(effect, CoreEffect::SteerTurn { .. }))
                .collect::<Vec<_>>(),
            vec![
                CoreEffect::SteerTurn {
                    correlation,
                    request: request.clone(),
                },
                CoreEffect::SteerTurn {
                    correlation: retry_correlation,
                    request,
                },
            ]
        );
    }

    #[test]
    fn immediate_approval_effect_emits_admission_before_completion() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateApprovalDecisionExecutor, rx);
        let turn_submission = runtime.begin_test_turn_submission();
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core runtime".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        });
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        });
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::ApprovalRequested {
                request: ConversationApprovalRequest {
                    approval_id: "approval-1".to_string(),
                    server_request_id: "server-approval-1".to_string(),
                    method: "item/commandExecution/requestApproval".to_string(),
                    kind: ConversationApprovalRequestKind::CommandExecution,
                    summary: "Command execution requested.".to_string(),
                    details: Vec::new(),
                },
            },
        });
        let correlation = ApprovalDecisionCorrelation::new(
            1,
            turn_submission,
            "approval-1",
            ConversationApprovalDecision::Accept,
        );

        let outcome = runtime.dispatch_command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-1".to_string(),
            decision: ConversationApprovalDecision::Accept,
        });

        assert_eq!(
            outcome.effects,
            vec![CoreEffect::SubmitApprovalDecision {
                correlation: correlation.clone(),
            }]
        );
        assert_eq!(
            outcome.events,
            vec![
                AppEvent::ApprovalDecisionAdmissionResolved(ApprovalDecisionAdmission::Accepted {
                    correlation: correlation.clone(),
                },),
                AppEvent::ApprovalDecisionSubmissionCompleted {
                    correlation: correlation.clone(),
                    result: Ok(()),
                },
            ]
        );
        let waiting_for_resolution = runtime.dispatch_command(AppCommand::SubmitApprovalDecision {
            approval_id: "approval-1".to_string(),
            decision: ConversationApprovalDecision::Decline,
        });
        assert_eq!(
            waiting_for_resolution.events,
            vec![AppEvent::ApprovalDecisionAdmissionResolved(
                ApprovalDecisionAdmission::RejectedActive {
                    active_correlation: correlation,
                },
            )]
        );
        assert!(waiting_for_resolution.effects.is_empty());
    }

    #[test]
    fn immediate_github_review_poll_emits_started_before_completed() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateGithubReviewPollExecutor, rx);
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let setup_correlation = GithubReviewPollingSetupCorrelation::new(1, "/workspace");
        runtime.dispatch_command(AppCommand::SetupGithubReviewPolling(
            GithubReviewPollingSetupRequest::new(
                "/workspace",
                GithubReviewPollingSetupMode::Explicit {
                    target: target.clone(),
                },
            ),
        ));
        let correlation = GithubReviewPollCorrelation::new(1, target.clone());

        let outcome = runtime.dispatch_command(AppCommand::PollGithubReview);

        assert_eq!(
            outcome.effects,
            vec![CoreEffect::PollGithubReview {
                setup_correlation: setup_correlation.clone(),
                correlation: correlation.clone(),
                previous_state: None,
            }]
        );
        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::GithubReviewPollStarted {
                    correlation: started,
                },
                AppEvent::GithubReviewPollCompleted {
                    correlation: completed,
                    result: Ok(result),
                },
            ] if started == &correlation
                && completed == &correlation
                && result.snapshot.target == target
        ));

        let next = runtime.dispatch_command(AppCommand::PollGithubReview);
        assert!(matches!(
            next.effects.as_slice(),
            [CoreEffect::PollGithubReview {
                setup_correlation: next_setup,
                correlation: GithubReviewPollCorrelation { generation: 2, .. },
                previous_state: Some(GithubPullRequestPollState {
                    latest_submitted_at: Some(latest),
                    ..
                }),
            }] if next_setup == &setup_correlation && latest == "2026-07-19T10:00:00Z"
        ));
    }

    #[test]
    fn prepare_manual_prompt_command_runs_prepare_effect_without_snapshot_change() {
        let (_tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
        let intent = ManualPromptPreparationIntent {
            workspace_directory: "/tmp/workspace".to_string(),
            raw_prompt: "ship it".to_string(),
            parent_thread_id: Some("thread-1".to_string()),
            parent_turn_id: None,
        };
        let correlation = ManualPromptCorrelation {
            request_id: 1,
            generation: 1,
            workspace_directory: "/tmp/workspace".to_string(),
        };
        let request = ManualPromptRequest {
            correlation: correlation.clone(),
            raw_prompt: intent.raw_prompt.clone(),
            parent_thread_id: intent.parent_thread_id.clone(),
            parent_turn_id: intent.parent_turn_id.clone(),
        };

        let outcome = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(intent)));

        assert_eq!(
            outcome.events,
            vec![AppEvent::ManualPromptPreparationAdmissionResolved(
                ManualPromptPreparationAdmission::Accepted { correlation },
            )]
        );
        assert_eq!(*outcome.snapshot, AppSnapshot::initial());
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::PrepareManualPrompt(Box::new(request))]
        );
    }

    #[test]
    fn immediate_manual_prompt_effect_is_accepted_before_dispatch_returns() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateManualPromptExecutor, rx);
        let intent = ManualPromptPreparationIntent {
            workspace_directory: "/tmp/workspace".to_string(),
            raw_prompt: "ship it".to_string(),
            parent_thread_id: None,
            parent_turn_id: None,
        };
        let correlation = ManualPromptCorrelation {
            request_id: 1,
            generation: 1,
            workspace_directory: "/tmp/workspace".to_string(),
        };
        let request = ManualPromptRequest {
            correlation: correlation.clone(),
            raw_prompt: intent.raw_prompt.clone(),
            parent_thread_id: None,
            parent_turn_id: None,
        };

        let outcome =
            runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(intent.clone())));

        assert_eq!(
            outcome.effects,
            vec![CoreEffect::PrepareManualPrompt(Box::new(request.clone()))]
        );
        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::ManualPromptPreparationAdmissionResolved(
                    ManualPromptPreparationAdmission::Accepted {
                        correlation: accepted,
                    },
                ),
                AppEvent::ManualPromptPrepared(result),
            ] if accepted == &correlation && result.correlation() == &correlation
        ));
        let second = runtime.dispatch_command(AppCommand::PrepareManualPrompt(Box::new(intent)));
        assert!(matches!(
            second.events.as_slice(),
            [
                AppEvent::ManualPromptPreparationAdmissionResolved(
                    ManualPromptPreparationAdmission::Accepted {
                        correlation: accepted,
                    },
                ),
                AppEvent::ManualPromptPrepared(result),
            ] if accepted.generation == 2
                && accepted.request_id == accepted.generation
                && result.correlation() == accepted
        ));
    }

    #[test]
    fn immediate_post_turn_completion_keeps_started_event_first() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediatePostTurnExecutor, rx);
        let turn_submission = runtime.begin_test_turn_submission();
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core runtime".to_string(),
                cwd: "/tmp/workspace".to_string(),
                runtime_envelope: Box::default(),
            },
        });
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        });
        runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnTerminal {
                receipt: ConversationTurnTerminalReceipt::completed(
                    "thread-1",
                    "turn-1",
                    Vec::new(),
                )
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                execution_snapshot_capture: None,
            },
        });
        let previous = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshSucceeded,
            last_summary: Some("previous summary".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let request = PostTurnRequest {
            context: PostTurnContext {
                thread_id: "thread-1".to_string(),
                planning_workspace_directory: "/tmp/workspace".to_string(),
                latest_user_message: None,
                latest_main_reply: None,
                previous_handoff_task: None,
                current_runtime_projection: RuntimeProjection::invalid("refresh required"),
                parallel_mode_enabled: false,
                parallel_automation_epoch_id: None,
                planning_settlement_paused: false,
                continuation_paused: false,
                can_queue_next: false,
                stop_keyword: ":stop".to_string(),
                stop_keyword_matched: false,
                no_file_changes_stop_matched: false,
                mode_label: "test".to_string(),
            },
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
            planning_worker_panel_state: previous,
            continuation_permit: PostTurnContinuationGate::default().capture(),
        };

        let outcome = runtime.dispatch_command(AppCommand::EvaluatePostTurn(Box::new(request)));

        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::PostTurnEvaluationStarted(started),
                AppEvent::PostTurnEvaluationCompleted(completed),
            ] if started.status == PlanningWorkerStatus::RefreshRunning
                && started.last_summary.as_deref() == Some("previous summary")
                && completed.planning_worker_panel_state == *started
        ));
    }

    #[test]
    fn immediate_review_center_effect_is_started_before_loaded_and_advances_generation() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateReviewCenterExecutor, rx);
        let snapshot = ReviewCenterSnapshot {
            current_thread_reviews: Ok(Vec::new()),
            pending_inbox: Ok(Vec::new()),
            recent_history: Ok(Vec::new()),
        };
        let first_correlation =
            ReviewCenterLoadCorrelation::new(1, "/tmp/workspace", Some("thread-1".to_string()));

        let first = runtime.dispatch_command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-1".to_string()),
        });

        assert_eq!(
            first.events,
            vec![
                AppEvent::ReviewCenterLoadStarted {
                    correlation: first_correlation.clone(),
                },
                AppEvent::ReviewCenterLoaded {
                    correlation: first_correlation.clone(),
                    snapshot: snapshot.clone(),
                },
            ]
        );
        assert_eq!(
            first.effects,
            vec![CoreEffect::LoadReviewCenter {
                correlation: first_correlation,
            }]
        );

        let second_correlation =
            ReviewCenterLoadCorrelation::new(2, "/tmp/workspace", Some("thread-1".to_string()));
        let second = runtime.dispatch_command(AppCommand::LoadReviewCenter {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-1".to_string()),
        });
        assert_eq!(
            second.events,
            vec![
                AppEvent::ReviewCenterLoadStarted {
                    correlation: second_correlation.clone(),
                },
                AppEvent::ReviewCenterLoaded {
                    correlation: second_correlation,
                    snapshot,
                },
            ]
        );
    }

    #[test]
    fn immediate_queue_authority_effect_is_started_before_loaded_and_advances_generation() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateQueueAuthorityExecutor, rx);
        let snapshot = QueueAuthoritySnapshot {
            runtime_projection: crate::domain::planning::RuntimeProjection::uninitialized(),
            planning_revision: 0,
            tasks: Vec::new(),
        };
        let first_correlation =
            QueueAuthorityLoadCorrelation::new(1, "/tmp/workspace", Some("thread-1".to_string()));

        let first = runtime.dispatch_command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-1".to_string()),
        });

        assert_eq!(
            first.events,
            vec![
                AppEvent::QueueAuthorityLoadStarted {
                    correlation: first_correlation.clone(),
                },
                AppEvent::QueueAuthorityLoaded {
                    correlation: first_correlation.clone(),
                    result: Ok(Box::new(snapshot.clone())),
                },
            ]
        );
        assert_eq!(
            first.effects,
            vec![CoreEffect::LoadQueueAuthority {
                correlation: first_correlation,
            }]
        );

        let second_correlation =
            QueueAuthorityLoadCorrelation::new(2, "/tmp/workspace", Some("thread-1".to_string()));
        let second = runtime.dispatch_command(AppCommand::LoadQueueAuthority {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-1".to_string()),
        });
        assert_eq!(
            second.events,
            vec![
                AppEvent::QueueAuthorityLoadStarted {
                    correlation: second_correlation.clone(),
                },
                AppEvent::QueueAuthorityLoaded {
                    correlation: second_correlation,
                    result: Ok(Box::new(snapshot)),
                },
            ]
        );
    }

    #[test]
    fn immediate_planning_runtime_effect_is_started_before_refreshed() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediatePlanningRuntimeExecutor, rx);
        let correlation = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/workspace");
        let doctor = PlanningRuntimeRefreshSnapshot::new(
            crate::domain::planning::RuntimeProjection::invalid("loaded"),
        )
        .doctor;

        let outcome = runtime.dispatch_command(AppCommand::RefreshPlanningRuntime {
            workspace_directory: "/tmp/workspace".to_string(),
        });

        assert_eq!(
            outcome.events,
            vec![
                AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: correlation.clone(),
                },
                AppEvent::PlanningRuntimeRefreshed {
                    correlation: correlation.clone(),
                    result: Ok(doctor),
                },
            ]
        );
        assert_eq!(
            outcome.effects,
            vec![CoreEffect::LoadPlanningRuntime { correlation }]
        );
        assert_eq!(
            *outcome.snapshot.planning_parallel.planning_runtime,
            crate::domain::planning::RuntimeProjection::invalid("loaded")
        );
        let current = runtime.dispatch_command(AppCommand::Noop);
        assert!(
            std::sync::Arc::ptr_eq(&outcome.snapshot, &current.snapshot),
            "immediate completion must return the runtime's final shared snapshot"
        );
    }

    #[test]
    fn immediate_queue_mutation_effect_is_started_before_completed_and_reopens_the_gate() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateQueueMutationExecutor, rx);
        let intent = QueueMutationIntent {
            workspace_directory: "/tmp/workspace".to_string(),
            active_thread_id: Some("thread-1".to_string()),
            kind: QueueMutationKind::RemoveSelected,
            expected_planning_revision: 7,
            targets: vec![QueueMutationTarget {
                task_id: "task-1".to_string(),
                expected_status: TaskStatus::Ready,
                expected_updated_at: "2026-07-19T00:00:00Z".to_string(),
            }],
            receipt_at_start: None,
        };
        let first_correlation = QueueMutationCorrelation::new(1, intent.clone());

        let first =
            runtime.dispatch_command(AppCommand::SubmitQueueMutation(Box::new(intent.clone())));

        assert_eq!(
            first.effects,
            vec![CoreEffect::ExecuteQueueMutation {
                correlation: first_correlation.clone(),
            }]
        );
        assert!(matches!(
            first.events.as_slice(),
            [
                AppEvent::QueueMutationStarted {
                    correlation: started,
                },
                AppEvent::QueueMutationCompleted {
                    correlation: completed,
                    result,
                },
            ] if started == &first_correlation
                && completed == &first_correlation
                && matches!(
                    &result.mutation,
                    Ok(QueueMutationCommitSnapshot {
                        committed_planning_revision: 8,
                        committed_task_ids,
                    }) if committed_task_ids == &["task-1".to_string()]
                )
                && matches!(
                    &result.authority,
                    Ok(QueueAuthoritySnapshot {
                        planning_revision: 8,
                        ..
                    })
                )
        ));

        let second =
            runtime.dispatch_command(AppCommand::SubmitQueueMutation(Box::new(intent.clone())));
        assert_eq!(
            second.effects,
            vec![CoreEffect::ExecuteQueueMutation {
                correlation: QueueMutationCorrelation::new(2, intent),
            }]
        );
    }

    #[test]
    fn immediate_stop_request_is_admitted_before_completion_and_resynchronizes_after_start() {
        let (_tx, rx) = core_input_channel();
        let mut runtime = CoreRuntime::new(ImmediateStopRequestExecutor, rx);
        let turn_submission = TurnSubmissionCorrelation::new(1);
        runtime.dispatch_command(AppCommand::SubmitTurn(TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        }));
        let correlation = StopRequestCorrelation::new(1, Some(turn_submission));

        let requested = runtime.dispatch_command(AppCommand::RequestStopAllSessions);
        assert_eq!(
            requested.events,
            vec![
                AppEvent::StopRequestAdmissionResolved(StopRequestAdmission::Accepted {
                    correlation,
                }),
                AppEvent::StopRequestAttemptCompleted {
                    correlation,
                    attempt: StopRequestAttempt::Initial,
                    result: Ok(()),
                },
            ]
        );
        assert_eq!(
            requested.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation,
                attempt: StopRequestAttempt::Initial,
            }]
        );

        let started = runtime.dispatch_input(CoreInput::ConversationStreamUpdated {
            correlation: turn_submission,
            event: TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        });
        assert_eq!(
            started.effects,
            vec![CoreEffect::RequestStopAllSessions {
                correlation,
                attempt: StopRequestAttempt::AfterTurnStarted,
            }]
        );
        assert!(matches!(
            started.events.as_slice(),
            [
                AppEvent::TurnStreamSnapshotChanged(_),
                AppEvent::StopRequestAttemptCompleted {
                    correlation: completed,
                    attempt: StopRequestAttempt::AfterTurnStarted,
                    result: Ok(()),
                },
            ] if *completed == correlation
        ));

        let duplicate = runtime.dispatch_command(AppCommand::RequestStopAllSessions);
        assert_eq!(
            duplicate.events,
            vec![AppEvent::StopRequestAdmissionResolved(
                StopRequestAdmission::RejectedActive {
                    active_correlation: correlation,
                },
            )]
        );
        assert!(duplicate.effects.is_empty());
    }

    #[test]
    fn drain_pending_inputs_reenters_completions_through_controller() {
        let (tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects.clone(), rx);
        runtime.dispatch_command(AppCommand::RunStartupChecks {
            workspace_directory: "/tmp/workspace".to_string(),
        });
        let ready = StartupReadySnapshot {
            cwd: "/tmp/workspace".to_string(),
            workspace_path: "/tmp/workspace".to_string(),
            can_continue: true,
            codex_binary: StartupDiagnosticSnapshot {
                ok: true,
                detail: "/usr/bin/codex".to_string(),
            },
            workspace: StartupDiagnosticSnapshot {
                ok: true,
                detail: "git repo: /tmp/workspace".to_string(),
            },
            app_server_initialize: StartupDiagnosticSnapshot {
                ok: true,
                detail: "initialized".to_string(),
            },
            account: StartupDiagnosticSnapshot {
                ok: true,
                detail: "authenticated".to_string(),
            },
            attachment: StartupAttachmentSnapshot {
                mode_label: "provider-launched".to_string(),
                recovery_anchor_label: "provider-thread-id".to_string(),
            },
            warnings: Vec::new(),
            schema_snapshot: "embedded schema".to_string(),
        };

        tx.send(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: StartupCheckCorrelation::new(1, "/tmp/workspace"),
                result: Ok(Box::new(ready.clone())),
            },
        ))
        .unwrap();

        let outcomes = runtime.drain_pending_inputs(8);

        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].events,
            vec![AppEvent::StartupChanged {
                correlation: StartupCheckCorrelation::new(1, "/tmp/workspace"),
                snapshot: StartupSnapshot::Ready(Box::new(ready.clone())),
            }]
        );
        assert_eq!(
            runtime.snapshot().startup,
            StartupSnapshot::Ready(Box::new(ready))
        );
        assert_eq!(
            effects.recorded_effects(),
            vec![CoreEffect::RunStartupChecks {
                correlation: StartupCheckCorrelation::new(1, "/tmp/workspace"),
            }]
        );
    }

    #[test]
    fn drain_pending_inputs_respects_batch_limit() {
        let (tx, rx) = core_input_channel();
        let effects = RecordingEffectExecutor::default();
        let mut runtime = CoreRuntime::new(effects, rx);

        tx.send(CoreInput::Command(AppCommand::Noop)).unwrap();
        tx.send(CoreInput::Command(AppCommand::Noop)).unwrap();

        let first_batch = runtime.drain_pending_inputs(1);

        assert_eq!(first_batch.len(), 1);

        let second_batch = runtime.drain_pending_inputs(1);

        assert_eq!(second_batch.len(), 1);
        assert!(runtime.drain_pending_inputs(1).is_empty());
    }
}
