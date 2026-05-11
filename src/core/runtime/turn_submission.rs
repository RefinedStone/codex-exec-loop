use std::any::Any;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::turn::{
    ParallelModeTurnService, ParallelTurnStreamLaunchRequest,
};
use crate::application::service::planning::{
    PlanningRuntimeUseCases, PlanningTurnExecutionSnapshotCapture,
    PlanningTurnExecutionSnapshotCaptureRequest,
};
use crate::core::app::{CoreInput, TurnSubmissionRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamExecutionObservation {
    terminal_failure_message: Option<String>,
    runtime_notice: Option<String>,
}

pub(crate) fn spawn_turn_submission_worker(
    request: TurnSubmissionRequest,
    conversation_service: ConversationService,
    planning_runtime: PlanningRuntimeUseCases,
    parallel_mode_turn_service: ParallelModeTurnService,
    input_sender: Sender<CoreInput>,
) {
    thread::spawn(move || {
        let (request, launch_notice, invalidate_supervisor_snapshot) =
            match resolve_stream_launch_request(&parallel_mode_turn_service, request) {
                Ok(result) => result,
                Err(error) => {
                    let _ = input_sender.send(CoreInput::ConversationStreamUpdated(
                        ConversationStreamEvent::Failed {
                            message: format!("parallel mode launch blocked: {error}"),
                        },
                    ));
                    return;
                }
            };

        let _ = input_sender.send(CoreInput::ConversationTurnWorkspaceChanged {
            workspace_directory: request.workspace_directory.clone(),
        });
        let execution_snapshot_capture =
            capture_turn_execution_snapshot(&planning_runtime, &request.workspace_directory);

        if invalidate_supervisor_snapshot {
            let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
        }
        if let Some(notice) = launch_notice {
            let _ = input_sender.send(CoreInput::ConversationRuntimeNotice(notice));
        }

        run_conversation_stream_worker(
            request,
            execution_snapshot_capture,
            conversation_service,
            parallel_mode_turn_service,
            input_sender,
        );
    });
}

fn run_conversation_stream_worker(
    request: TurnSubmissionRequest,
    execution_snapshot_capture: PlanningTurnExecutionSnapshotCapture,
    conversation_service: ConversationService,
    parallel_mode_turn_service: ParallelModeTurnService,
    input_sender: Sender<CoreInput>,
) {
    let (event_tx, event_rx) = mpsc::channel();

    let request_for_service = request.clone();
    let service_thread = thread::spawn(move || {
        run_stream_request(conversation_service, request_for_service, event_tx)
    });
    let mut stream_lifecycle =
        parallel_mode_turn_service.stream_lifecycle(request.workspace_directory.clone());

    let mut saw_terminal_event = false;

    while let Ok(event) = event_rx.recv() {
        let lifecycle_outcome = stream_lifecycle.observe_event(&event);
        if lifecycle_outcome.invalidate_supervisor_snapshot {
            let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
        }
        if let Some(notice) = lifecycle_outcome.runtime_notice {
            let _ = input_sender.send(CoreInput::ConversationRuntimeNotice(notice));
        }

        if lifecycle_outcome.should_stop_stream_forwarding {
            saw_terminal_event = true;
        }
        let _ = input_sender.send(conversation_stream_core_input(
            event,
            &execution_snapshot_capture,
        ));
        if lifecycle_outcome.should_stop_stream_forwarding {
            break;
        }
    }

    let observation = match service_thread.join() {
        Ok(result) => observe_stream_completion(&request, saw_terminal_event, result),
        Err(payload) => observe_stream_panic(&request, saw_terminal_event, payload),
    };

    if let Some(message) = observation.terminal_failure_message.as_ref() {
        let _ = input_sender.send(CoreInput::ConversationStreamUpdated(
            ConversationStreamEvent::Failed {
                message: message.clone(),
            },
        ));
    }

    let completion_outcome = stream_lifecycle
        .finalize_after_stream_completion(observation.terminal_failure_message.is_some());
    if completion_outcome.invalidate_supervisor_snapshot {
        let _ = input_sender.send(CoreInput::ParallelModeSupervisorSnapshotInvalidated);
    }
    if let Some(notice) = completion_outcome.runtime_notice {
        let _ = input_sender.send(CoreInput::ConversationRuntimeNotice(notice));
    }

    if let Some(notice) = observation.runtime_notice {
        let _ = input_sender.send(CoreInput::ConversationRuntimeNotice(notice));
    }
}

fn resolve_stream_launch_request(
    parallel_mode_turn_service: &ParallelModeTurnService,
    request: TurnSubmissionRequest,
) -> Result<(TurnSubmissionRequest, Option<String>, bool), String> {
    let prompt_origin = request.prompt_origin;
    let outcome =
        parallel_mode_turn_service.prepare_stream_launch(ParallelTurnStreamLaunchRequest {
            workspace_directory: request.workspace_directory,
            thread_id: request.thread_id,
            prompt: request.prompt,
            slot_lease_handoff: request.slot_lease_handoff,
        })?;
    Ok((
        TurnSubmissionRequest {
            workspace_directory: outcome.request.workspace_directory,
            thread_id: outcome.request.thread_id,
            prompt: outcome.request.prompt,
            prompt_origin,
            slot_lease_handoff: outcome.request.slot_lease_handoff,
        },
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
    event: ConversationStreamEvent,
    execution_snapshot_capture: &PlanningTurnExecutionSnapshotCapture,
) -> CoreInput {
    match event {
        ConversationStreamEvent::TurnCompleted {
            turn_id,
            changed_planning_file_paths,
        } => CoreInput::ConversationTurnCompleted {
            turn_id,
            changed_planning_file_paths,
            execution_snapshot_capture: execution_snapshot_capture.clone(),
        },
        event => CoreInput::ConversationStreamUpdated(event),
    }
}

fn run_stream_request(
    conversation_service: ConversationService,
    request: TurnSubmissionRequest,
    event_sender: Sender<ConversationStreamEvent>,
) -> Result<(), String> {
    match request.thread_id.as_deref() {
        Some(thread_id) => conversation_service
            .run_turn_stream(thread_id, &request.prompt, event_sender)
            .map_err(|error| error.to_string()),
        None => conversation_service
            .run_new_thread_stream(&request.workspace_directory, &request.prompt, event_sender)
            .map_err(|error| error.to_string()),
    }
}

fn observe_stream_completion(
    request: &TurnSubmissionRequest,
    saw_terminal_event: bool,
    result: Result<(), String>,
) -> StreamExecutionObservation {
    match (saw_terminal_event, result) {
        (true, Ok(())) => StreamExecutionObservation {
            terminal_failure_message: None,
            runtime_notice: None,
        },
        (false, Ok(())) => StreamExecutionObservation {
            terminal_failure_message: Some(format!(
                "{} ended without a terminal event; forcing a failure so the conversation can recover",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} completed without a terminal event",
                request.request_label()
            )),
        },
        (false, Err(error)) => StreamExecutionObservation {
            terminal_failure_message: Some(format!(
                "{} failed before a terminal event: {error}",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} returned an error before a terminal event: {error}",
                request.request_label()
            )),
        },
        (true, Err(error)) => StreamExecutionObservation {
            terminal_failure_message: None,
            runtime_notice: Some(format!(
                "{} returned an error after the terminal event: {error}",
                request.request_label()
            )),
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
            terminal_failure_message: None,
            runtime_notice: Some(format!(
                "{} panicked after the terminal event: {panic_summary}",
                request.request_label()
            )),
        }
    } else {
        StreamExecutionObservation {
            terminal_failure_message: Some(format!(
                "{} panicked before a terminal event: {panic_summary}",
                request.request_label()
            )),
            runtime_notice: Some(format!(
                "{} panicked before a terminal event: {panic_summary}",
                request.request_label()
            )),
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
            slot_lease_handoff: None,
        }
    }

    #[test]
    fn missing_terminal_event_becomes_forced_failure_and_notice() {
        let observation = observe_stream_completion(&sample_request(), false, Ok(()));

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
    fn transport_error_before_terminal_event_becomes_failure_and_notice() {
        let observation = observe_stream_completion(
            &sample_request(),
            false,
            Err("transport closed".to_string()),
        );

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
    fn late_stream_error_becomes_runtime_notice_only() {
        let observation =
            observe_stream_completion(&sample_request(), true, Err("transport closed".to_string()));

        assert!(observation.terminal_failure_message.is_none());
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
    fn panic_after_terminal_event_becomes_runtime_notice_only() {
        let observation = observe_stream_panic(&sample_request(), true, Box::new("worker crashed"));

        assert!(observation.terminal_failure_message.is_none());
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
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["new/docs/plan.md".to_string()],
            },
            &snapshot_capture,
        );

        let CoreInput::ConversationTurnCompleted {
            turn_id,
            changed_planning_file_paths,
            execution_snapshot_capture,
        } = input
        else {
            panic!("turn completion should use the completion-specific core input");
        };
        assert_eq!(turn_id, "turn-1");
        assert_eq!(
            changed_planning_file_paths,
            vec!["new/docs/plan.md".to_string()]
        );
        assert_eq!(execution_snapshot_capture, snapshot_capture);
    }
}
