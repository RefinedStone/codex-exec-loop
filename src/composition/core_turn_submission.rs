use std::any::Any;
use std::thread;

use crate::application::service::conversation_runtime_event::{
    ConversationStreamEvent, ConversationStreamSender, conversation_stream_channel,
};
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::turn::{
    ParallelModeTurnService, ParallelTurnStreamLaunchRequest,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamExecutionObservation {
    projected_terminal_receipt: Option<ConversationTurnTerminalReceipt>,
    terminal_failure_message: Option<String>,
    terminal_failure_observed: bool,
    runtime_notice: Option<String>,
}

pub(crate) fn spawn_turn_submission_worker(
    correlation: TurnSubmissionCorrelation,
    request: TurnSubmissionRequest,
    conversation_service: ConversationService,
    planning_runtime: PlanningRuntimeUseCases,
    parallel_mode_turn_service: ParallelModeTurnService,
    input_sender: CoreInputSender,
) {
    thread::spawn(move || {
        let (resolved_request, expected_lease, launch_notice, invalidate_supervisor_snapshot) =
            match resolve_stream_launch_request(&parallel_mode_turn_service, request) {
                Ok(result) => result,
                Err(error) => {
                    let _ = input_sender.send(CoreInput::ConversationStreamUpdated {
                        correlation,
                        event: TurnStreamEvent::Failed {
                            message: format!("parallel mode launch blocked: {error}"),
                        },
                    });
                    return;
                }
            };

        let _ = input_sender.send(CoreInput::ConversationTurnWorkspaceChanged {
            correlation,
            workspace_directory: resolved_request.workspace_directory.clone(),
        });
        let execution_snapshot_capture = capture_turn_execution_snapshot(
            &planning_runtime,
            &resolved_request.workspace_directory,
        )
        .with_parallel_slot_lease(expected_lease.clone());

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
            expected_lease,
            execution_snapshot_capture,
            conversation_service,
            parallel_mode_turn_service,
            input_sender,
        );
    });
}

fn run_conversation_stream_worker(
    correlation: TurnSubmissionCorrelation,
    request: TurnSubmissionRequest,
    expected_lease: Option<ParallelModeSlotLeaseSnapshot>,
    execution_snapshot_capture: PlanningTurnExecutionSnapshotCapture,
    conversation_service: ConversationService,
    parallel_mode_turn_service: ParallelModeTurnService,
    input_sender: CoreInputSender,
) {
    let (event_tx, event_rx) = conversation_stream_channel();

    let request_for_service = request.clone();
    let service_thread = thread::spawn(move || {
        run_stream_request(conversation_service, request_for_service, event_tx)
    });
    let mut stream_lifecycle = expected_lease.map_or_else(
        || parallel_mode_turn_service.stream_lifecycle(request.workspace_directory.clone()),
        |lease| parallel_mode_turn_service.stream_lifecycle_for_lease(lease),
    );

    let mut observed_terminal_receipt = None;
    let mut observed_terminal_failure = None;

    while let Ok(event) = event_rx.recv() {
        let lifecycle_outcome = stream_lifecycle.observe_event(&event);
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
        let _ = input_sender.send(conversation_stream_core_input(
            correlation,
            event,
            &execution_snapshot_capture,
        ));
    }

    // A bounded producer may still attempt to send after a terminal event.
    // Disconnect before joining so that send fails instead of blocking forever
    // against a receiver that intentionally stopped draining.
    drop(event_rx);

    let mut observation = match service_thread.join() {
        Ok(result) => {
            observe_stream_completion(&request, observed_terminal_receipt.as_ref(), result)
        }
        Err(payload) => observe_stream_panic(
            &request,
            observed_terminal_receipt.is_some() || observed_terminal_failure.is_some(),
            payload,
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
    let completion_outcome =
        stream_lifecycle.finalize_after_stream_completion(terminal_failure_observed);
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

    if let Some(receipt) = projected_terminal_receipt {
        let _ = input_sender.send(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::TurnTerminal {
                receipt,
                execution_snapshot_capture: Some(execution_snapshot_capture),
            },
        });
    } else if let Some(message) = terminal_failure_message {
        let _ = input_sender.send(CoreInput::ConversationStreamUpdated {
            correlation,
            event: TurnStreamEvent::Failed { message },
        });
    }
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
        prompt_origin,
        turn_options,
        slot_lease_handoff,
    } = request;
    let outcome =
        parallel_mode_turn_service.prepare_stream_launch(ParallelTurnStreamLaunchRequest {
            workspace_directory,
            thread_id,
            prompt,
            slot_lease_handoff,
        })?;
    Ok((
        TurnSubmissionRequest {
            workspace_directory: outcome.request.workspace_directory,
            thread_id: outcome.request.thread_id,
            prompt: outcome.request.prompt,
            prompt_origin,
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
        } => TurnStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
        },
        ConversationStreamEvent::TurnStarted { turn_id } => {
            TurnStreamEvent::TurnStarted { turn_id }
        }
        ConversationStreamEvent::StatusUpdated { text } => TurnStreamEvent::StatusUpdated { text },
        ConversationStreamEvent::AgentMessageDelta {
            item_id,
            phase,
            delta,
        } => TurnStreamEvent::AgentMessageDelta {
            item_id,
            phase,
            delta,
        },
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
            approval_id,
            resolution,
        } => TurnStreamEvent::ApprovalResolved {
            approval_id,
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
                request.turn_options.clone(),
                event_sender,
            )
            .map_err(|error| error.to_string()),
        None => conversation_service
            .run_new_thread_stream(
                &request.workspace_directory,
                &request.prompt,
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
    payload: Box<dyn Any + Send>,
) -> StreamExecutionObservation {
    let panic_summary = panic_payload_summary(payload);

    if saw_terminal_event {
        StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} panicked after emitting an unverified terminal receipt: {panic_summary}",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} panicked after the terminal event: {panic_summary}",
                request.request_label()
            )),
            terminal_failure_observed: true,
        }
    } else {
        StreamExecutionObservation {
            projected_terminal_receipt: None,
            terminal_failure_message: Some(format!(
                "{} panicked before a terminal event: {panic_summary}",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} panicked before a terminal event: {panic_summary}",
                request.request_label()
            )),
            terminal_failure_observed: true,
        }
    }
}

fn panic_payload_summary(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }

    "unknown panic payload".to_string()
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::application::service::planning::{
        PlanningExecutionSnapshot, PlanningTurnExecutionSnapshotCapture,
    };
    use crate::core::app::CorePromptOrigin;

    fn sample_request() -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "ship it".to_string(),
            prompt_origin: CorePromptOrigin::Manual,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        }
    }

    fn test_turn_correlation() -> TurnSubmissionCorrelation {
        TurnSubmissionCorrelation::new(7)
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
        let observation =
            observe_stream_panic(&sample_request(), false, Box::new("worker crashed"));

        assert_eq!(
            observation.terminal_failure_message,
            Some("turn stream panicked before a terminal event: worker crashed".to_string())
        );
        assert_eq!(
            observation.runtime_notice,
            Some("turn stream panicked before a terminal event: worker crashed".to_string())
        );
    }

    #[test]
    fn panic_after_terminal_event_rejects_the_unverified_terminal() {
        let observation = observe_stream_panic(&sample_request(), true, Box::new("worker crashed"));

        assert_eq!(
            observation.terminal_failure_message,
            Some(
                "turn stream panicked after emitting an unverified terminal receipt: worker crashed"
                    .to_string()
            )
        );
        assert_eq!(
            observation.runtime_notice,
            Some("turn stream panicked after the terminal event: worker crashed".to_string())
        );
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
