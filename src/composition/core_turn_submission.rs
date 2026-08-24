use crate::application::port::conversation_stream::{
    ConversationStreamEvent, ConversationStreamSender, conversation_stream_channel,
};
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::turn::{
    ParallelModeTurnService, ParallelTurnStreamLaunchRequest, ParallelTurnStreamLifecycle,
};
use crate::application::service::planning::{
    PlanningRuntimeUseCases, PlanningTurnExecutionSnapshotCapture,
    PlanningTurnExecutionSnapshotCaptureRequest,
};
use crate::core::app::{
    CoreInput, TurnStreamEvent, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::core::runtime::CoreInputSender;
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use crate::domain::turn_terminal::ConversationTurnTerminalReceipt;
use crate::panic_observation::catch_redacted_worker_unwind;

use super::core_effect_worker::{spawn_joinable_redacted_worker, spawn_worker_with_panic_fallback};

const TURN_SUBMISSION_WORKER_PANIC_MESSAGE: &str = "turn submission worker panicked";

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamExecutionObservation {
    projected_terminal_receipt: Option<ConversationTurnTerminalReceipt>,
    terminal_failure_message: Option<String>,
    terminal_failure_observed: bool,
    runtime_notice: Option<String>,
}

#[derive(Default)]
struct TurnSubmissionWorkerSettlement {
    stream_lifecycle: Option<ParallelTurnStreamLifecycle>,
    lifecycle_finalized: bool,
    terminal_published: bool,
    supervisor_reconciliation_required: bool,
}

pub(crate) fn spawn_turn_submission_worker(
    correlation: TurnSubmissionCorrelation,
    request: TurnSubmissionRequest,
    conversation_service: ConversationService,
    planning_runtime: PlanningRuntimeUseCases,
    parallel_mode_turn_service: ParallelModeTurnService,
    input_sender: CoreInputSender,
) {
    let panic_input_sender = input_sender.clone();
    let panic_correlation = correlation;
    spawn_worker_with_panic_fallback(
        move || {
            run_guarded_turn_submission_worker(
                correlation,
                request,
                conversation_service,
                planning_runtime,
                parallel_mode_turn_service,
                input_sender,
            );
        },
        move || {
            publish_turn_submission_failure(
                &panic_input_sender,
                panic_correlation,
                TURN_SUBMISSION_WORKER_PANIC_MESSAGE,
            );
        },
    );
}

fn run_guarded_turn_submission_worker(
    correlation: TurnSubmissionCorrelation,
    request: TurnSubmissionRequest,
    conversation_service: ConversationService,
    planning_runtime: PlanningRuntimeUseCases,
    parallel_mode_turn_service: ParallelModeTurnService,
    input_sender: CoreInputSender,
) {
    let mut settlement = TurnSubmissionWorkerSettlement {
        supervisor_reconciliation_required: request.slot_lease_handoff.is_some(),
        ..TurnSubmissionWorkerSettlement::default()
    };
    let result = catch_redacted_worker_unwind(|| {
        let (resolved_request, expected_lease, launch_notice, invalidate_supervisor_snapshot) =
            match resolve_stream_launch_request(&parallel_mode_turn_service, request) {
                Ok(result) => result,
                Err(error) => {
                    settlement.terminal_published = publish_turn_submission_failure(
                        &input_sender,
                        correlation,
                        format!("parallel mode launch blocked: {error}"),
                    );
                    return;
                }
            };

        settlement.stream_lifecycle = Some(expected_lease.clone().map_or_else(
            || {
                parallel_mode_turn_service
                    .stream_lifecycle(resolved_request.workspace_directory.clone())
            },
            |lease| parallel_mode_turn_service.stream_lifecycle_for_lease(lease),
        ));

        let _ = input_sender.send(CoreInput::ConversationTurnWorkspaceChanged {
            correlation,
            workspace_directory: resolved_request.workspace_directory.clone(),
        });
        let execution_snapshot_capture = capture_turn_execution_snapshot(
            &planning_runtime,
            &resolved_request.workspace_directory,
        )
        .with_parallel_slot_lease(expected_lease);

        if invalidate_supervisor_snapshot {
            let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
        }
        if let Some(notice) = launch_notice {
            let _ = input_sender.send(CoreInput::ConversationTurnRuntimeNotice {
                correlation,
                notice,
            });
        }

        run_conversation_stream_worker(
            correlation,
            resolved_request,
            execution_snapshot_capture,
            conversation_service,
            input_sender.clone(),
            &mut settlement,
        );
    });

    if result.is_err() {
        settle_turn_submission_panic(&input_sender, correlation, &mut settlement);
    }
}

fn run_conversation_stream_worker(
    correlation: TurnSubmissionCorrelation,
    request: TurnSubmissionRequest,
    execution_snapshot_capture: PlanningTurnExecutionSnapshotCapture,
    conversation_service: ConversationService,
    input_sender: CoreInputSender,
    settlement: &mut TurnSubmissionWorkerSettlement,
) {
    let (event_tx, event_rx) = conversation_stream_channel();

    let request_for_service = request.clone();
    let service_thread = spawn_joinable_redacted_worker(move || {
        run_stream_request(conversation_service, request_for_service, event_tx)
    });

    let mut observed_terminal_receipt = None;
    let mut observed_terminal_failure = None;

    while let Ok(event) = event_rx.recv() {
        let lifecycle_outcome = settlement
            .stream_lifecycle
            .as_mut()
            .expect("turn submission must create its stream lifecycle before execution")
            .observe_event(&event);
        if lifecycle_outcome.invalidate_supervisor_snapshot {
            let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
        }
        if let Some(notice) = lifecycle_outcome.runtime_notice {
            let _ = input_sender.send(CoreInput::ConversationTurnRuntimeNotice {
                correlation,
                notice,
            });
        }

        if lifecycle_outcome.should_stop_stream_forwarding {
            match event {
                ConversationStreamEvent::TurnTerminal { receipt } => {
                    observed_terminal_receipt = Some(receipt);
                }
                ConversationStreamEvent::Failed { message } => {
                    observed_terminal_failure = Some(message);
                }
                _ => unreachable!("stream lifecycle stopped on a non-terminal event"),
            }
            break;
        }
        let core_input =
            conversation_stream_core_input(correlation, event, &execution_snapshot_capture);
        if let Err(error) = input_sender.send(core_input) {
            if matches!(
                error.0,
                CoreInput::ConversationStreamUpdated {
                    event: TurnStreamEvent::ProgressiveActivityObserved { .. },
                    ..
                }
            ) {
                let message =
                    "Core progressive ingress rejected a bounded stream segment".to_string();
                observed_terminal_failure = Some(message);
            }
            break;
        }
    }

    // A bounded producer may still attempt to send after a terminal event.
    // Disconnect before joining so that send fails instead of blocking forever
    // against a receiver that intentionally stopped draining.
    drop(event_rx);

    let mut observation = match service_thread.join() {
        Ok(result) => {
            observe_stream_completion(&request, observed_terminal_receipt.as_ref(), result)
        }
        Err(_) => observe_stream_panic(
            &request,
            observed_terminal_receipt.is_some() || observed_terminal_failure.is_some(),
        ),
    };
    if let Some(message) = observed_terminal_failure {
        observation.projected_terminal_receipt = None;
        observation.terminal_failure_message = Some(message);
        observation.terminal_failure_observed = true;
        observation.runtime_notice.get_or_insert_with(|| {
            format!(
                "{} emitted a terminal runtime failure",
                request.request_label()
            )
        });
    }

    let StreamExecutionObservation {
        projected_terminal_receipt,
        terminal_failure_message,
        terminal_failure_observed,
        runtime_notice,
    } = observation;
    let completion_outcome = settlement
        .stream_lifecycle
        .as_ref()
        .expect("turn submission must retain its stream lifecycle through finalization")
        .finalize_after_stream_completion(terminal_failure_observed);
    settlement.lifecycle_finalized = true;
    if completion_outcome.invalidate_supervisor_snapshot {
        let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
    }
    if let Some(notice) = completion_outcome.runtime_notice {
        let _ = input_sender.send(CoreInput::ConversationTurnRuntimeNotice {
            correlation,
            notice,
        });
    }

    if let Some(notice) = runtime_notice {
        let _ = input_sender.send(CoreInput::ConversationTurnRuntimeNotice {
            correlation,
            notice,
        });
    }

    settlement.terminal_published = if let Some(receipt) = projected_terminal_receipt {
        input_sender
            .send(CoreInput::ConversationStreamUpdated {
                correlation,
                event: TurnStreamEvent::TurnTerminal {
                    receipt,
                    execution_snapshot_capture: Some(execution_snapshot_capture),
                },
            })
            .is_ok()
    } else if let Some(message) = terminal_failure_message {
        publish_turn_submission_failure(&input_sender, correlation, message)
    } else {
        false
    };
}

fn settle_turn_submission_panic(
    input_sender: &CoreInputSender,
    correlation: TurnSubmissionCorrelation,
    settlement: &mut TurnSubmissionWorkerSettlement,
) {
    if !settlement.lifecycle_finalized
        && let Some(stream_lifecycle) = settlement.stream_lifecycle.as_ref()
    {
        match catch_redacted_worker_unwind(|| {
            stream_lifecycle.finalize_after_stream_completion(true)
        }) {
            Ok(completion_outcome) => {
                settlement.lifecycle_finalized = true;
                if completion_outcome.invalidate_supervisor_snapshot {
                    let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
                }
                if let Some(notice) = completion_outcome.runtime_notice {
                    let _ = input_sender.send(CoreInput::ConversationTurnRuntimeNotice {
                        correlation,
                        notice,
                    });
                }
            }
            Err(_) => {
                let _ = input_sender.send(CoreInput::ConversationTurnRuntimeNotice {
                    correlation,
                    notice: "parallel turn lifecycle recovery panicked; supervisor reconciliation required"
                        .to_string(),
                });
            }
        }
    }
    if settlement.supervisor_reconciliation_required && !settlement.lifecycle_finalized {
        let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
    }
    if !settlement.terminal_published {
        settlement.terminal_published = publish_turn_submission_failure(
            input_sender,
            correlation,
            TURN_SUBMISSION_WORKER_PANIC_MESSAGE,
        );
    }
}

fn publish_turn_submission_failure(
    input_sender: &CoreInputSender,
    correlation: TurnSubmissionCorrelation,
    message: impl Into<String>,
) -> bool {
    input_sender
        .send(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::Failed {
                message: message.into(),
            },
        })
        .is_ok()
}

fn resolve_stream_launch_request(
    parallel_mode_turn_service: &ParallelModeTurnService,
    request: TurnSubmissionRequest,
) -> Result<
    (
        TurnSubmissionRequest,
        Option<ParallelModeSlotLeaseSnapshot>,
        Option<String>,
        bool,
    ),
    String,
> {
    let TurnSubmissionRequest {
        workspace_directory,
        thread_id,
        prompt,
        image_paths,
        prompt_origin,
        auto_follow_source,
        planning_handoff,
        turn_options,
        slot_lease_handoff,
    } = request;
    let requested_slot_lease = slot_lease_handoff.is_some();
    let outcome =
        parallel_mode_turn_service.prepare_stream_launch(ParallelTurnStreamLaunchRequest {
            workspace_directory,
            thread_id,
            prompt,
            slot_lease_handoff,
        })?;
    /*
     * Plain conversation turns pass through `prepare_stream_launch` unchanged
     * and must keep their operator image attachments; only a real parallel
     * slot launch (which converts the prompt into worker task turns) drops
     * them. Upstream submission gating already blocks attaching images while
     * parallel task intake is active, so this strip is defense in depth.
     */
    let image_paths = if requested_slot_lease || outcome.request.slot_lease_handoff.is_some() {
        Vec::new()
    } else {
        image_paths
    };
    Ok((
        TurnSubmissionRequest {
            workspace_directory: outcome.request.workspace_directory,
            thread_id: outcome.request.thread_id,
            prompt: outcome.request.prompt,
            image_paths,
            prompt_origin,
            auto_follow_source,
            planning_handoff,
            turn_options,
            slot_lease_handoff: outcome.request.slot_lease_handoff,
        },
        outcome.expected_lease,
        outcome.launch_notice,
        outcome.invalidate_supervisor_snapshot,
    ))
}

fn capture_turn_execution_snapshot(
    planning_runtime: &PlanningRuntimeUseCases,
    workspace_directory: &str,
) -> PlanningTurnExecutionSnapshotCapture {
    planning_runtime.capture_turn_execution_snapshot(
        PlanningTurnExecutionSnapshotCaptureRequest::new(workspace_directory),
    )
}

fn conversation_stream_core_input(
    correlation: TurnSubmissionCorrelation,
    event: ConversationStreamEvent,
    execution_snapshot_capture: &PlanningTurnExecutionSnapshotCapture,
) -> CoreInput {
    match event {
        ConversationStreamEvent::TurnTerminal { receipt } => CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::TurnTerminal {
                receipt,
                execution_snapshot_capture: Some(execution_snapshot_capture.clone()),
            },
        },
        event => CoreInput::ConversationStreamUpdated {
            correlation,
            event: turn_stream_event_from_application(event),
        },
    }
}

fn turn_stream_event_from_application(event: ConversationStreamEvent) -> TurnStreamEvent {
    match event {
        ConversationStreamEvent::AttachmentObserved { profile } => {
            TurnStreamEvent::AttachmentObserved { profile }
        }
        ConversationStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
            runtime_envelope,
        } => TurnStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
            runtime_envelope,
        },
        ConversationStreamEvent::TurnStarted {
            turn_id,
            runtime_request,
        } => TurnStreamEvent::TurnStarted {
            turn_id,
            runtime_request,
        },
        ConversationStreamEvent::RuntimeEnvelopeObserved { observation } => {
            TurnStreamEvent::RuntimeEnvelopeObserved { observation }
        }
        ConversationStreamEvent::ItemLifecycleObserved { observation } => {
            TurnStreamEvent::ItemLifecycleObserved { observation }
        }
        ConversationStreamEvent::ProgressiveActivityObserved { batch } => {
            TurnStreamEvent::ProgressiveActivityObserved { batch }
        }
        ConversationStreamEvent::StatusUpdated { text } => TurnStreamEvent::StatusUpdated { text },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        } => TurnStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        },
        ConversationStreamEvent::ToolActivity { activity } => {
            TurnStreamEvent::ToolActivity { activity }
        }
        ConversationStreamEvent::ApprovalReviewUpdated { review } => {
            TurnStreamEvent::ApprovalReviewUpdated { review }
        }
        ConversationStreamEvent::ApprovalRequested { request } => {
            TurnStreamEvent::ApprovalRequested { request }
        }
        ConversationStreamEvent::ApprovalResolved {
            request_identity,
            resolution,
        } => TurnStreamEvent::ApprovalResolved {
            request_identity,
            resolution,
        },
        ConversationStreamEvent::TurnInterruptRequestFailed { message } => {
            TurnStreamEvent::TurnInterruptRequestFailed { message }
        }
        ConversationStreamEvent::TurnRetrying {
            thread_id,
            turn_id,
            error,
        } => TurnStreamEvent::TurnRetrying {
            thread_id,
            turn_id,
            error,
        },
        ConversationStreamEvent::TurnTerminal { .. } => {
            unreachable!("typed terminal receipt is handled before stream event conversion")
        }
        ConversationStreamEvent::Failed { message } => TurnStreamEvent::Failed { message },
    }
}

fn run_stream_request(
    conversation_service: ConversationService,
    request: TurnSubmissionRequest,
    event_sender: ConversationStreamSender,
) -> Result<ConversationTurnTerminalReceipt, String> {
    match request.thread_id.as_deref() {
        Some(thread_id) => conversation_service
            .run_turn_stream(
                thread_id,
                &request.prompt,
                &request.image_paths,
                request.turn_options.clone(),
                event_sender,
            )
            .map_err(|error| error.to_string()),
        None => conversation_service
            .run_new_thread_stream(
                &request.workspace_directory,
                &request.prompt,
                &request.image_paths,
                request.turn_options.clone(),
                event_sender,
            )
            .map_err(|error| error.to_string()),
    }
}

fn observe_stream_completion(
    request: &TurnSubmissionRequest,
    observed_terminal_receipt: Option<&ConversationTurnTerminalReceipt>,
    result: Result<ConversationTurnTerminalReceipt, String>,
) -> StreamExecutionObservation {
    match (observed_terminal_receipt, result) {
        (Some(observed), Ok(returned)) if observed == &returned => StreamExecutionObservation {
            terminal_failure_observed: !returned.is_completed_and_confirmed(),
            projected_terminal_receipt: Some(returned),
            terminal_failure_message: None,
            runtime_notice: None,
        },
        (Some(_), Ok(_)) => StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} returned a terminal receipt that did not match its terminal event",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} terminal receipt/event mismatch",
                request.request_label()
            )),
            terminal_failure_observed: true,
        },
        (None, Ok(returned))
            if matches!(
                returned.application_delivery,
                crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Unconfirmed(_)
            ) =>
        {
            StreamExecutionObservation {
                projected_terminal_receipt: Some(returned),
                terminal_failure_message: None,
                terminal_failure_observed: true,
                runtime_notice: Some(format!(
                    "{} reached an upstream terminal outcome with application delivery recovery pending",
                    request.request_label()
                )),
            }
        }
        (None, Ok(_)) => StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} ended without a terminal event; forcing a failure so the conversation can recover",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} completed without a terminal event",
                request.request_label()
            )),
            terminal_failure_observed: true,
        },
        (None, Err(error)) => StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} failed before a terminal event: {error}",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} returned an error before a terminal event: {error}",
                request.request_label()
            )),
            terminal_failure_observed: true,
        },
        (Some(_), Err(error)) => StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} returned an error after emitting a terminal receipt: {error}",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} returned an error after the terminal event: {error}",
                request.request_label()
            )),
            terminal_failure_observed: true,
        },
    }
}

fn observe_stream_panic(
    request: &TurnSubmissionRequest,
    saw_terminal_event: bool,
) -> StreamExecutionObservation {
    if saw_terminal_event {
        StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} panicked after emitting an unverified terminal receipt",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} panicked after the terminal event",
                request.request_label()
            )),
            terminal_failure_observed: true,
        }
    } else {
        StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} panicked before a terminal event",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} panicked before a terminal event",
                request.request_label()
            )),
            terminal_failure_observed: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::application::service::planning::{
        PlanningExecutionSnapshot, PlanningTurnExecutionSnapshotCapture,
    };
    use crate::core::app::{AppEvent, CoreEffect, CorePromptOrigin, TurnStreamUpdate};
    use crate::core::runtime::{CoreEffectExecutor, CoreRuntime, core_input_channel};

    struct NoopEffectExecutor;

    impl CoreEffectExecutor for NoopEffectExecutor {
        fn run_effect(&self, _effect: CoreEffect) -> Option<CoreInput> {
            None
        }
    }

    fn sample_request() -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            image_paths: Vec::new(),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        }
    }

    fn test_turn_correlation() -> TurnSubmissionCorrelation {
        TurnSubmissionCorrelation::new(7)
    }

    /*
     * Regression for PR #2126 review (P1): plain conversation turns pass
     * through prepare_stream_launch unchanged, so their operator image
     * attachments must survive into run_stream_request.
     */
    #[test]
    fn plain_stream_launch_preserves_operator_image_attachments() {
        let parallel_mode_turn_service = ParallelModeTurnService::new(
            crate::application::service::parallel_mode::ParallelModeService::new(
                std::sync::Arc::new(
                    crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter::new(),
                ),
                std::sync::Arc::new(
                    crate::adapter::outbound::github::GithubAutomationAdapter::new(),
                ),
                std::sync::Arc::new(
                    crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter::new(),
                ),
            ),
        );
        let mut request = sample_request();
        request.image_paths = vec!["/tmp/shot.png".to_string()];

        let (resolved_request, expected_lease, launch_notice, invalidate_snapshot) =
            resolve_stream_launch_request(&parallel_mode_turn_service, request)
                .expect("plain conversation launch should pass through");

        assert_eq!(resolved_request.image_paths, vec!["/tmp/shot.png"]);
        assert!(expected_lease.is_none());
        assert!(launch_notice.is_none());
        assert!(!invalidate_snapshot);
    }

    fn completed_receipt() -> ConversationTurnTerminalReceipt {
        ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            vec!["new/docs/plan.md".to_string()],
        )
        .with_application_delivery(
            crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
        )
    }

    #[test]
    fn missing_terminal_event_becomes_forced_failure_and_notice() {
        let observation =
            observe_stream_completion(&sample_request(), None, Ok(completed_receipt()));

        assert_eq!(
            observation.terminal_failure_message,
            Some(
                "turn stream ended without a terminal event; forcing a failure so the conversation can recover"
                    .to_string()
            )
        );
        assert_eq!(
            observation.runtime_notice,
            Some("turn stream completed without a terminal event".to_string())
        );
    }

    #[test]
    fn matching_terminal_event_and_return_receipt_are_verified() {
        let receipt = completed_receipt();
        let observation =
            observe_stream_completion(&sample_request(), Some(&receipt), Ok(receipt.clone()));

        assert_eq!(observation.projected_terminal_receipt, Some(receipt));
        assert!(observation.terminal_failure_message.is_none());
        assert!(!observation.terminal_failure_observed);
        assert!(observation.runtime_notice.is_none());
    }

    #[test]
    fn mismatched_terminal_event_and_return_receipt_fail_closed() {
        let observed = completed_receipt();
        let returned =
            ConversationTurnTerminalReceipt::completed("thread-1", "different-turn", Vec::new())
                .with_application_delivery(
                    crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
                );
        let observation =
            observe_stream_completion(&sample_request(), Some(&observed), Ok(returned));

        assert!(observation.projected_terminal_receipt.is_none());
        assert!(
            observation
                .terminal_failure_message
                .as_deref()
                .is_some_and(|message| message.contains("did not match"))
        );
    }

    #[test]
    fn transport_error_before_terminal_event_becomes_failure_and_notice() {
        let observation =
            observe_stream_completion(&sample_request(), None, Err("transport closed".to_string()));

        assert_eq!(
            observation.terminal_failure_message,
            Some("turn stream failed before a terminal event: transport closed".to_string())
        );
        assert_eq!(
            observation.runtime_notice,
            Some(
                "turn stream returned an error before a terminal event: transport closed"
                    .to_string()
            )
        );
    }

    #[test]
    fn unconfirmed_terminal_without_event_preserves_typed_recovery_pending_receipt() {
        let receipt = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            Vec::new(),
        )
        .with_application_delivery(
            crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Unconfirmed(
                crate::domain::turn_terminal::ConversationTurnApplicationDeliveryFailure::Disconnected,
            ),
        );

        let observation = observe_stream_completion(&sample_request(), None, Ok(receipt.clone()));

        assert_eq!(observation.projected_terminal_receipt, Some(receipt));
        assert!(observation.terminal_failure_message.is_none());
        assert!(observation.terminal_failure_observed);
        assert!(
            observation
                .runtime_notice
                .as_deref()
                .is_some_and(|notice| notice.contains("recovery pending"))
        );
    }

    #[test]
    fn error_after_terminal_event_rejects_the_unverified_terminal() {
        let receipt = completed_receipt();
        let observation = observe_stream_completion(
            &sample_request(),
            Some(&receipt),
            Err("transport closed".to_string()),
        );

        assert_eq!(
            observation.terminal_failure_message,
            Some(
                "turn stream returned an error after emitting a terminal receipt: transport closed"
                    .to_string()
            )
        );
        assert_eq!(
            observation.runtime_notice,
            Some(
                "turn stream returned an error after the terminal event: transport closed"
                    .to_string()
            )
        );
    }

    #[test]
    fn panic_before_terminal_event_becomes_failure_and_notice() {
        let observation = observe_stream_panic(&sample_request(), false);

        assert_eq!(
            observation.terminal_failure_message,
            Some("turn stream panicked before a terminal event".to_string())
        );
        assert_eq!(
            observation.runtime_notice,
            Some("turn stream panicked before a terminal event".to_string())
        );
    }

    #[test]
    fn panic_after_terminal_event_rejects_the_unverified_terminal() {
        let observation = observe_stream_panic(&sample_request(), true);

        assert_eq!(
            observation.terminal_failure_message,
            Some("turn stream panicked after emitting an unverified terminal receipt".to_string())
        );
        assert_eq!(
            observation.runtime_notice,
            Some("turn stream panicked after the terminal event".to_string())
        );
    }

    #[test]
    fn turn_submission_panic_settlement_publishes_one_exact_correlated_terminal() {
        let (input_sender, input_receiver) = core_input_channel();
        let mut runtime = CoreRuntime::new(NoopEffectExecutor, input_receiver);
        let correlation = runtime.begin_test_turn_submission();
        let mut settlement = TurnSubmissionWorkerSettlement::default();

        settle_turn_submission_panic(&input_sender, correlation, &mut settlement);
        settle_turn_submission_panic(&input_sender, correlation, &mut settlement);

        let outcomes = runtime.drain_pending_inputs(2);
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0].events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(authority),
                AppEvent::TurnStreamSnapshotChanged(snapshot),
            ]
                if matches!(
                    &snapshot.update,
                    TurnStreamUpdate::Failed { message, .. }
                        if message == TURN_SUBMISSION_WORKER_PANIC_MESSAGE
                )
                    && authority.active_turn.is_none()
                    && outcomes[0].snapshot.conversation_runtime == **authority
        ));
        assert!(settlement.terminal_published);
    }

    #[test]
    fn pre_lifecycle_parallel_panic_requests_reconciliation_before_terminal() {
        let (input_sender, input_receiver) = core_input_channel();
        let mut runtime = CoreRuntime::new(NoopEffectExecutor, input_receiver);
        let correlation = runtime.begin_test_turn_submission();
        let mut settlement = TurnSubmissionWorkerSettlement {
            supervisor_reconciliation_required: true,
            ..TurnSubmissionWorkerSettlement::default()
        };

        settle_turn_submission_panic(&input_sender, correlation, &mut settlement);

        let outcomes = runtime.drain_pending_inputs(3);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(
            outcomes[0].events,
            vec![AppEvent::ParallelModeSupervisorSnapshotInvalidated]
        );
        assert!(matches!(
            outcomes[1].events.as_slice(),
            [
                AppEvent::ConversationRuntimeAuthorityChanged(authority),
                AppEvent::TurnStreamSnapshotChanged(snapshot),
            ]
                if matches!(
                    &snapshot.update,
                    TurnStreamUpdate::Failed { message, .. }
                        if message == TURN_SUBMISSION_WORKER_PANIC_MESSAGE
                )
                    && authority.active_turn.is_none()
                    && outcomes[1].snapshot.conversation_runtime == **authority
        ));
    }

    #[test]
    fn turn_completed_core_input_carries_execution_snapshot_capture() {
        let snapshot_capture = PlanningTurnExecutionSnapshotCapture::ready(
            "/tmp/workspace",
            PlanningExecutionSnapshot::default(),
        );

        let input = conversation_stream_core_input(
            test_turn_correlation(),
            ConversationStreamEvent::TurnTerminal {
                receipt: completed_receipt(),
            },
            &snapshot_capture,
        );

        let CoreInput::ConversationStreamUpdated {
            correlation,
            event:
                TurnStreamEvent::TurnTerminal {
                    receipt,
                    execution_snapshot_capture: Some(execution_snapshot_capture),
                },
        } = input
        else {
            panic!("terminal receipt should use the typed core stream input");
        };
        assert_eq!(correlation, test_turn_correlation());
        assert_eq!(receipt.turn_id, "turn-1");
        assert_eq!(
            receipt.observations.changed_planning_file_paths,
            vec!["new/docs/plan.md".to_string()]
        );
        assert_eq!(execution_snapshot_capture, snapshot_capture);
    }

    #[test]
    fn terminal_receiver_disconnects_post_terminal_producer_without_deadlock() {
        let (sender, receiver) = conversation_stream_channel();
        let producer = thread::spawn(move || {
            sender
                .send(ConversationStreamEvent::TurnTerminal {
                    receipt: completed_receipt(),
                })
                .expect("terminal event should be admitted");
            loop {
                if let Err(error) = sender.send(ConversationStreamEvent::StatusUpdated {
                    text: "invalid post-terminal event".to_string(),
                }) {
                    return error.0;
                }
            }
        });

        assert!(matches!(
            receiver.recv().expect("terminal event should arrive"),
            ConversationStreamEvent::TurnTerminal { .. }
        ));
        drop(receiver);
        assert_eq!(
            producer
                .join()
                .expect("post-terminal producer should observe disconnect"),
            ConversationStreamEvent::StatusUpdated {
                text: "invalid post-terminal event".to_string(),
            }
        );
    }
}
