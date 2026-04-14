use std::any::Any;
use std::sync::mpsc;
use std::thread;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedTurnStreamRequest {
    pub workspace_directory: String,
    pub thread_id: Option<String>,
    pub prompt: String,
}

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
        self.active_turn_planning_capture =
            Some(self.capture_active_turn_planning(&request.workspace_directory));
        spawn_conversation_stream_worker(
            request,
            self.conversation_service.clone(),
            self.tx.clone(),
        );
    }

    fn capture_active_turn_planning(&self, workspace_directory: &str) -> ActiveTurnPlanningCapture {
        match self
            .planning
            .runtime
            .load_execution_snapshot(workspace_directory)
        {
            Ok(snapshot) => ActiveTurnPlanningCapture::ready(workspace_directory, snapshot),
            Err(error) => ActiveTurnPlanningCapture::capture_failed(
                workspace_directory,
                format!(
                    "planning reconciliation could not capture the accepted planning snapshot before the turn started: {error}"
                ),
            ),
        }
    }
}

fn spawn_conversation_stream_worker(
    request: PreparedTurnStreamRequest,
    service: ConversationService,
    outer_tx: std::sync::mpsc::Sender<BackgroundMessage>,
) {
    thread::spawn(move || {
        let (event_tx, event_rx) = mpsc::channel();
        let request_for_service = request.clone();
        let service_thread =
            thread::spawn(move || run_stream_request(service, request_for_service, event_tx));

        let mut saw_terminal_event = false;
        while let Ok(event) = event_rx.recv() {
            let should_stop = matches!(
                event,
                ConversationStreamEvent::TurnCompleted { .. }
                    | ConversationStreamEvent::Failed { .. }
            );
            if should_stop {
                saw_terminal_event = true;
            }
            let _ = outer_tx.send(BackgroundMessage::ConversationStream(event));
            if should_stop {
                break;
            }
        }

        let observation = match service_thread.join() {
            Ok(result) => observe_stream_completion(&request, saw_terminal_event, result),
            Err(payload) => observe_stream_panic(&request, saw_terminal_event, payload),
        };

        if let Some(message) = observation.terminal_failure_message {
            let _ = outer_tx.send(BackgroundMessage::ConversationStream(
                ConversationStreamEvent::Failed { message },
            ));
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
            .map_err(|error| error.to_string()),
        None => service
            .run_new_thread_stream(&request.workspace_directory, &request.prompt, event_sender)
            .map_err(|error| error.to_string()),
    }
}

fn observe_stream_completion(
    request: &PreparedTurnStreamRequest,
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
