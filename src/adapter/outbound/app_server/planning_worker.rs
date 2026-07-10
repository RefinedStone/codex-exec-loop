use std::sync::Arc;
use std::thread;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerOperation, PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::service::conversation_runtime_event::{
    ConversationStreamEvent, ConversationStreamSender, conversation_stream_channel,
};
use crate::diagnostics::event_log;
use serde_json::json;

use super::persisted_error_summary;

/*
 * PlanningThreadLauncher는 planning worker port와 실제 app-server thread 실행 사이의 좁은 seam이다.
 * AppServerPlanningWorkerAdapter는 stream을 해석하는 책임만 갖고, hidden thread를 새로 만들고 turn을
 * 실행하는 세부 orchestration은 app-server adapter 본체가 구현한다.
 */
pub(crate) trait PlanningThreadLauncher: Send + Sync {
    /*
     * The launcher owns the app-server side effects: creating a hidden thread,
     * sending the prompt, and forwarding raw stream events. The planning worker
     * adapter below only sees the normalized ConversationStreamEvent stream so
     * the application port remains independent from JSON-RPC and thread setup.
     */
    fn run_hidden_planning_thread(
        &self,
        workspace_directory: &str,
        prompt: &str,
        event_sender: ConversationStreamSender,
        continuation_permit: Option<crate::domain::planning::PostTurnContinuationPermit>,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct AppServerPlanningWorkerAdapter {
    // launcher를 trait object로 잡아 application port test가 app-server process 없이 stream 축약만 검증하게 한다.
    planning_thread_launcher: Arc<dyn PlanningThreadLauncher>,
}

impl AppServerPlanningWorkerAdapter {
    pub(crate) fn new(planning_thread_launcher: Arc<dyn PlanningThreadLauncher>) -> Self {
        Self {
            planning_thread_launcher,
        }
    }
}

impl PlanningWorkerPort for AppServerPlanningWorkerAdapter {
    /*
     * planning worker는 사용자-facing conversation stream을 그대로 노출하지 않는다. hidden worker가 보낸
     * ConversationStreamEvent 중 최종 agent message, planning file 변경 목록, 실패 신호만 application
     * response로 축약한다. 이렇게 해야 queue refresh/repair service가 app-server protocol의 세부 event
     * vocabulary에 직접 의존하지 않는다.
     */
    #[tracing::instrument(level = "trace", skip(self, request))]
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse> {
        if request
            .continuation_permit
            .as_ref()
            .is_some_and(|permit| !permit.is_current())
        {
            anyhow::bail!("post-turn continuation was superseded before planning worker launch");
        }
        let (tx, rx) = conversation_stream_channel();
        event_log::emit_lazy("planning_worker_session_starting", || {
            json!({
                "thread_id": serde_json::Value::Null,
                "operation": operation_label(request.operation),
                "phase": "starting",
                "workspace_directory": &request.workspace_directory,
                "prompt_chars": request.prompt.chars().count(),
            })
        });
        let planning_thread_launcher = self.planning_thread_launcher.clone();
        let workspace_directory = request.workspace_directory.clone();
        let prompt = request.prompt.clone();
        let continuation_permit = request.continuation_permit.clone();
        let service_thread = thread::spawn(move || {
            planning_thread_launcher.run_hidden_planning_thread(
                &workspace_directory,
                &prompt,
                tx,
                continuation_permit,
            )
        });

        let mut final_agent_message = None;
        let mut changed_planning_file_paths = Vec::new();
        let mut failure_message = None;
        let mut captured_thread_id = None;
        let mut captured_turn_id = None;

        // The producer runs concurrently because the stream queue is bounded.
        // Stop receiving at the first terminal event, then disconnect the queue
        // before joining so a buggy producer cannot deadlock the join by sending
        // post-terminal events.
        for event in rx.iter() {
            let terminal = matches!(
                event,
                ConversationStreamEvent::TurnCompleted { .. }
                    | ConversationStreamEvent::Failed { .. }
            );
            match event {
                ConversationStreamEvent::AgentMessageCompleted { text, .. } => {
                    /*
                     * Deltas are intentionally ignored; a completed message is
                     * the stable unit that worker orchestration can quote in
                     * logs or validation summaries without replaying stream
                     * fragments.
                     */
                    final_agent_message = Some(text);
                }
                ConversationStreamEvent::TurnCompleted {
                    changed_planning_file_paths: paths,
                    ..
                } => {
                    /*
                     * TurnCompleted is the only event that carries the planning
                     * file change summary reduced by the app-server adapter. It
                     * replaces any earlier value because a hidden worker turn has
                     * one authoritative completion boundary.
                     */
                    changed_planning_file_paths = paths;
                }
                ConversationStreamEvent::ThreadPrepared { thread_id, .. } => {
                    captured_thread_id = Some(thread_id);
                }
                ConversationStreamEvent::TurnStarted { turn_id } => {
                    captured_turn_id = Some(turn_id);
                }
                ConversationStreamEvent::AttachmentObserved { .. }
                | ConversationStreamEvent::StatusUpdated { .. }
                | ConversationStreamEvent::AgentMessageDelta { .. }
                | ConversationStreamEvent::ToolActivity { .. }
                | ConversationStreamEvent::ApprovalReviewUpdated { .. }
                | ConversationStreamEvent::ApprovalRequested { .. }
                | ConversationStreamEvent::ApprovalResolved { .. }
                | ConversationStreamEvent::TurnInterruptRequestFailed { .. } => {}
                ConversationStreamEvent::Failed { message } => {
                    /*
                     * Keep draining after seeing a failure so channel closure
                     * remains the synchronization point. The final response below
                     * still treats any failure event as a hard worker error.
                     */
                    failure_message = Some(message);
                }
            }
            if terminal {
                break;
            }
        }
        drop(rx);

        let stream_result = service_thread
            .join()
            .map_err(|_| anyhow!("planning worker stream producer panicked"))?;
        if let Err(error) = stream_result {
            event_log::emit_lazy("planning_worker_session_launch_failed", || {
                json!({
                    "thread_id": captured_thread_id.as_deref(),
                    "operation": operation_label(request.operation),
                    "phase": "launch_failed",
                    "workspace_directory": &request.workspace_directory,
                    "error_summary": persisted_error_summary(&error),
                })
            });
            return Err(error);
        }

        if let Some(message) = failure_message {
            event_log::emit_lazy("planning_worker_session_stream_failed", || {
                json!({
                    "thread_id": captured_thread_id.as_deref(),
                    "turn_id": captured_turn_id.as_deref(),
                    "operation": operation_label(request.operation),
                    "phase": "stream_failed",
                    "workspace_directory": &request.workspace_directory,
                    "message_chars": message.chars().count(),
                    "changed_planning_file_count": changed_planning_file_paths.len(),
                    "has_final_agent_message": final_agent_message.is_some(),
                })
            });
            return Err(anyhow!("planning worker stream failed: {message}"));
        }

        event_log::emit_lazy("planning_worker_session_reduced", || {
            json!({
                "thread_id": captured_thread_id.as_deref(),
                "turn_id": captured_turn_id.as_deref(),
                "operation": operation_label(request.operation),
                "phase": "reduced",
                "workspace_directory": &request.workspace_directory,
                "changed_planning_file_count": changed_planning_file_paths.len(),
                "has_final_agent_message": final_agent_message.is_some(),
                "final_agent_message_chars": final_agent_message
                    .as_deref()
                    .map(|message| message.chars().count()),
            })
        });
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            thread_id: captured_thread_id,
            turn_id: captured_turn_id,
            final_agent_message,
            changed_planning_file_paths,
        })
    }
}

fn operation_label(operation: PlanningWorkerOperation) -> &'static str {
    match operation {
        PlanningWorkerOperation::RefreshQueue => "refresh",
        PlanningWorkerOperation::RepairTaskAuthority => "repair",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use anyhow::Result;

    use super::{AppServerPlanningWorkerAdapter, PlanningThreadLauncher};
    use crate::application::port::outbound::planning_worker_port::{
        PlanningWorkerOperation, PlanningWorkerPort, PlanningWorkerRequest,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct HiddenPlanningThreadCall {
        workspace_directory: String,
        prompt: String,
    }

    struct FakePlanningThreadLauncher {
        events: Vec<ConversationStreamEvent>,
        calls: Mutex<Vec<HiddenPlanningThreadCall>>,
    }

    impl PlanningThreadLauncher for FakePlanningThreadLauncher {
        fn run_hidden_planning_thread(
            &self,
            workspace_directory: &str,
            prompt: &str,
            event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
            _continuation_permit: Option<crate::domain::planning::PostTurnContinuationPermit>,
        ) -> Result<()> {
            /*
             * The fake records launch input before sending events. That gives
             * the success test coverage for both halves of the port contract:
             * request forwarding into the hidden thread and stream reduction out
             * of it.
             */
            self.calls
                .lock()
                .expect("calls lock should succeed")
                .push(HiddenPlanningThreadCall {
                    workspace_directory: workspace_directory.to_string(),
                    prompt: prompt.to_string(),
                });
            for event in self.events.clone() {
                let _ = event_sender.send(event);
            }
            Ok(())
        }
    }

    #[test]
    fn run_planning_session_collects_completed_message_and_changed_paths() {
        /*
         * 정상 stream test는 hidden planning thread가 여러 UI-facing event를 보내도 port response에는
         * final message와 changed planning path만 남는다는 축약 계약을 고정한다.
         */
        let fake_launcher = Arc::new(FakePlanningThreadLauncher {
            events: vec![
                ConversationStreamEvent::codex_app_server_launch_attachment(),
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Planning Worker".to_string(),
                    cwd: "/tmp/workspace".to_string(),
                },
                ConversationStreamEvent::AgentMessageCompleted {
                    item_id: "item-1".to_string(),
                    phase: None,
                    text: "planning updated".to_string(),
                },
                ConversationStreamEvent::TurnStarted {
                    turn_id: "turn-1".to_string(),
                },
                ConversationStreamEvent::TurnCompleted {
                    turn_id: "turn-1".to_string(),
                    changed_planning_file_paths: vec!["DB task authority".to_string()],
                },
            ],
            calls: Mutex::new(Vec::new()),
        });
        let adapter = AppServerPlanningWorkerAdapter::new(fake_launcher.clone());

        let result = adapter
            .run_planning_session(PlanningWorkerRequest {
                operation: PlanningWorkerOperation::RefreshQueue,
                workspace_directory: "/tmp/workspace".to_string(),
                prompt: "refresh".to_string(),
                continuation_permit: None,
            })
            .expect("planning worker should succeed");

        assert_eq!(
            result.final_agent_message.as_deref(),
            Some("planning updated")
        );
        assert_eq!(
            result.changed_planning_file_paths,
            vec!["DB task authority".to_string()]
        );
        assert_eq!(result.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(result.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(
            fake_launcher
                .calls
                .lock()
                .expect("calls lock should succeed")
                .as_slice(),
            &[HiddenPlanningThreadCall {
                workspace_directory: "/tmp/workspace".to_string(),
                prompt: "refresh".to_string(),
            }]
        );
    }

    #[test]
    fn canceled_continuation_never_invokes_hidden_thread_launcher() {
        let fake_launcher = Arc::new(FakePlanningThreadLauncher {
            events: Vec::new(),
            calls: Mutex::new(Vec::new()),
        });
        let adapter = AppServerPlanningWorkerAdapter::new(fake_launcher.clone());
        let gate = crate::domain::planning::PostTurnContinuationGate::default();
        let permit = gate.capture();
        gate.advance();

        let error = adapter
            .run_planning_session(PlanningWorkerRequest {
                operation: PlanningWorkerOperation::RefreshQueue,
                workspace_directory: "/tmp/workspace".to_string(),
                prompt: "must not launch".to_string(),
                continuation_permit: Some(permit),
            })
            .expect_err("superseded continuation must fail closed before launch");

        assert!(error.to_string().contains("superseded"));
        assert!(
            fake_launcher
                .calls
                .lock()
                .expect("calls lock should succeed")
                .is_empty()
        );
    }

    #[test]
    fn run_planning_session_returns_error_when_stream_reports_failure() {
        /*
         * Failed events are promoted to anyhow errors instead of being mixed into
         * a successful response. Worker orchestration then follows its repair or
         * retry path rather than trusting a partial planning update.
         */
        let adapter = AppServerPlanningWorkerAdapter::new(Arc::new(FakePlanningThreadLauncher {
            events: vec![ConversationStreamEvent::Failed {
                message: "planning worker crashed".to_string(),
            }],
            calls: Mutex::new(Vec::new()),
        }));

        let error = adapter
            .run_planning_session(PlanningWorkerRequest {
                operation: PlanningWorkerOperation::RepairTaskAuthority,
                workspace_directory: "/tmp/workspace".to_string(),
                prompt: "repair".to_string(),
                continuation_permit: None,
            })
            .expect_err("failed stream should surface as error");

        assert!(error.to_string().contains("planning worker crashed"));
    }
}
