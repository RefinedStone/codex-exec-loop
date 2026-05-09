use std::any::Any;
use std::sync::mpsc;
use std::thread;

use crate::adapter::inbound::tui::app::{
    ActiveTurnExecutionSnapshotCapture, BackgroundMessage, ConversationState, NativeTuiApp,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::turn::{
    ParallelModeTurnService, ParallelTurnSlotLeaseHandoff, ParallelTurnStreamLaunchRequest,
};

/* This module is the TUI-side bridge from a submitted prompt to the streaming
 * conversation service. It also mirrors every stream event into parallel-mode slot
 * state, so a turn can recover cleanly whether it is a normal session turn, a new
 * thread launch, or a parallel slot lease startup.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedTurnStreamRequest {
    pub workspace_directory: String,
    pub thread_id: Option<String>,
    pub prompt: String,
    pub slot_lease_handoff: Option<ParallelTurnSlotLeaseHandoff>,
}

// Service completion is observed after the event stream closes. The terminal
// failure is sent as a synthetic ConversationStreamEvent::Failed, while the
// runtime notice is diagnostic copy for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamExecutionObservation {
    terminal_failure_message: Option<String>,
    runtime_notice: Option<String>,
}

impl PreparedTurnStreamRequest {
    fn request_label(&self) -> &'static str {
        if self.thread_id.is_some() {
            "turn stream"
        } else {
            "new-thread stream"
        }
    }
}

impl NativeTuiApp {
    pub(super) fn execute_start_stream(&mut self, request: PreparedTurnStreamRequest) {
        let (request, launch_notice, invalidate_supervisor_snapshot) =
            match resolve_stream_launch_request(&self.parallel_mode_turn_service(), request) {
                Ok(result) => result,
                Err(error) => {
                    let _ = self.tx.send(BackgroundMessage::ConversationStream(
                        ConversationStreamEvent::Failed {
                            message: format!("parallel mode launch blocked: {error}"),
                        },
                    ));
                    return;
                }
            };

        // The stream may run against a slot worktree instead of the shell's current
        // workspace. Capture the execution snapshot before the worker can emit
        // TurnStarted and before post-turn reconciliation compares protected files.
        self.sync_active_turn_workspace_directory(&request.workspace_directory);
        self.active_turn_execution_snapshot_capture =
            Some(self.capture_active_turn_execution_snapshot(&request.workspace_directory));

        if invalidate_supervisor_snapshot {
            self.invalidate_parallel_mode_supervisor_snapshot();
        }
        if let Some(notice) = launch_notice {
            let _ = self
                .tx
                .send(BackgroundMessage::ConversationRuntimeNotice(notice));
        }
        spawn_conversation_stream_worker(
            request,
            self.conversation_service.clone(),
            self.parallel_mode_turn_service(),
            self.tx.clone(),
        );
    }

    fn capture_active_turn_execution_snapshot(
        &self,
        workspace_directory: &str,
    ) -> ActiveTurnExecutionSnapshotCapture {
        match self
            .planning
            .runtime
            .load_execution_snapshot(workspace_directory)
        {
            Ok(snapshot) => {
                ActiveTurnExecutionSnapshotCapture::ready(workspace_directory, snapshot)
            }
            Err(error) => ActiveTurnExecutionSnapshotCapture::capture_failed(
                workspace_directory,
                format!(
                    "planning reconciliation could not capture the execution snapshot before the turn started: {error}"
                ),
            ),
        }
    }

    fn sync_active_turn_workspace_directory(&mut self, workspace_directory: &str) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };

        conversation.replace_active_turn_workspace_directory(workspace_directory.to_string());
        self.conversation_state = ConversationState::ready(conversation);
    }
}

fn resolve_stream_launch_request(
    parallel_mode_turn_service: &ParallelModeTurnService,
    request: PreparedTurnStreamRequest,
) -> Result<(PreparedTurnStreamRequest, Option<String>, bool), String> {
    let outcome =
        parallel_mode_turn_service.prepare_stream_launch(ParallelTurnStreamLaunchRequest {
            workspace_directory: request.workspace_directory,
            thread_id: request.thread_id,
            prompt: request.prompt,
            slot_lease_handoff: request.slot_lease_handoff,
        })?;
    Ok((
        PreparedTurnStreamRequest {
            workspace_directory: outcome.request.workspace_directory,
            thread_id: outcome.request.thread_id,
            prompt: outcome.request.prompt,
            slot_lease_handoff: outcome.request.slot_lease_handoff,
        },
        outcome.launch_notice,
        outcome.invalidate_supervisor_snapshot,
    ))
}

fn spawn_conversation_stream_worker(
    request: PreparedTurnStreamRequest,
    service: ConversationService,
    parallel_mode_turn_service: ParallelModeTurnService,
    outer_tx: std::sync::mpsc::Sender<BackgroundMessage>,
) {
    thread::spawn(move || {
        let (event_tx, event_rx) = mpsc::channel();

        let request_for_service = request.clone();
        let service_thread =
            thread::spawn(move || run_stream_request(service, request_for_service, event_tx));

        let mut saw_terminal_event = false;
        let mut saw_turn_started = false;
        let mut saw_failed_before_turn_started = false;
        let mut saw_failed_event = false;

        // Forward events as they arrive, but let the parallel turn service observe
        // them first so slot state and operator notices stay synchronized with the
        // visible stream.
        while let Ok(event) = event_rx.recv() {
            let (notice, invalidate_supervisor_snapshot) = sync_slot_lease_for_stream_event(
                &parallel_mode_turn_service,
                &request,
                &event,
                &mut saw_turn_started,
            );
            if invalidate_supervisor_snapshot {
                let _ = outer_tx.send(BackgroundMessage::InvalidateParallelModeSupervisorSnapshot);
            }
            if let Some(notice) = notice {
                let _ = outer_tx.send(BackgroundMessage::ConversationRuntimeNotice(notice));
            }
            let should_stop = matches!(
                event,
                ConversationStreamEvent::TurnCompleted { .. }
                    | ConversationStreamEvent::Failed { .. }
            );

            if matches!(&event, ConversationStreamEvent::Failed { .. }) {
                saw_failed_event = true;
                if !saw_turn_started {
                    saw_failed_before_turn_started = true;
                }
            }
            if should_stop {
                saw_terminal_event = true;
            }
            let _ = outer_tx.send(BackgroundMessage::ConversationStream(event));
            if should_stop {
                break;
            }
        }

        // Joining the service thread distinguishes a clean terminal event from
        // transport errors that happen after the terminal event was already emitted.
        let observation = match service_thread.join() {
            Ok(result) => observe_stream_completion(&request, saw_terminal_event, result),
            Err(payload) => observe_stream_panic(&request, saw_terminal_event, payload),
        };

        if let Some(message) = observation.terminal_failure_message.as_ref() {
            let _ = outer_tx.send(BackgroundMessage::ConversationStream(
                ConversationStreamEvent::Failed {
                    message: message.clone(),
                },
            ));
        }

        let (notice, invalidate_supervisor_snapshot) = finalize_slot_lease_after_stream_completion(
            &parallel_mode_turn_service,
            &request,
            saw_turn_started,
            saw_failed_before_turn_started,
            saw_failed_event,
            &observation,
        );
        if invalidate_supervisor_snapshot {
            let _ = outer_tx.send(BackgroundMessage::InvalidateParallelModeSupervisorSnapshot);
        }
        if let Some(notice) = notice {
            let _ = outer_tx.send(BackgroundMessage::ConversationRuntimeNotice(notice));
        }

        if let Some(notice) = observation.runtime_notice {
            let _ = outer_tx.send(BackgroundMessage::ConversationRuntimeNotice(notice));
        }
    });
}

fn run_stream_request(
    service: ConversationService,
    request: PreparedTurnStreamRequest,
    event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
) -> Result<(), String> {
    match request.thread_id {
        Some(thread_id) => service
            .run_turn_stream(&thread_id, &request.prompt, event_sender)
            .map_err(|error: anyhow::Error| error.to_string()),
        None => service
            .run_new_thread_stream(&request.workspace_directory, &request.prompt, event_sender)
            .map_err(|error: anyhow::Error| error.to_string()),
    }
}

fn sync_slot_lease_for_stream_event(
    parallel_mode_turn_service: &ParallelModeTurnService,
    request: &PreparedTurnStreamRequest,
    event: &ConversationStreamEvent,
    saw_turn_started: &mut bool,
) -> (Option<String>, bool) {
    let outcome = parallel_mode_turn_service.sync_stream_event(&request.workspace_directory, event);
    *saw_turn_started |= outcome.turn_started_observed;
    (
        outcome.runtime_notice,
        outcome.invalidate_supervisor_snapshot,
    )
}

fn finalize_slot_lease_after_stream_completion(
    parallel_mode_turn_service: &ParallelModeTurnService,
    request: &PreparedTurnStreamRequest,
    saw_turn_started: bool,
    saw_failed_before_turn_started: bool,
    saw_failed_event: bool,
    observation: &StreamExecutionObservation,
) -> (Option<String>, bool) {
    let outcome = parallel_mode_turn_service.finalize_stream_completion(
        &request.workspace_directory,
        saw_turn_started,
        saw_failed_before_turn_started,
        saw_failed_event,
        observation.terminal_failure_message.is_some(),
    );
    (
        outcome.runtime_notice,
        outcome.invalidate_supervisor_snapshot,
    )
}

fn observe_stream_completion(
    request: &PreparedTurnStreamRequest,
    saw_terminal_event: bool,
    result: Result<(), String>,
) -> StreamExecutionObservation {
    // A terminal event is the conversation reducer's proof that the turn closed. If
    // the service returns without one, synthesize a failure so the UI and slot lease
    // do not remain in a running state indefinitely.
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
    request: &PreparedTurnStreamRequest,
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

    fn sample_request() -> PreparedTurnStreamRequest {
        PreparedTurnStreamRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            thread_id: Some("thread-1".to_string()),
            prompt: "ship it".to_string(),
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
}
