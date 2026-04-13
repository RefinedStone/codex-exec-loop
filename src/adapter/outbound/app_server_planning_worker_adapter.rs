use std::sync::Arc;
use std::sync::mpsc;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::codex_app_server_port::{
    CodexAppServerPort, NewThreadReasoningEffort, NewThreadStreamRequest,
};
use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::domain::conversation::ConversationStreamEvent;

#[derive(Clone)]
pub struct AppServerPlanningWorkerAdapter {
    codex_app_server_port: Arc<dyn CodexAppServerPort>,
}

const PLANNING_WORKER_MODEL: &str = "gpt-5.4";
const PLANNING_WORKER_REASONING_EFFORT: NewThreadReasoningEffort = NewThreadReasoningEffort::Medium;

impl AppServerPlanningWorkerAdapter {
    pub fn new(codex_app_server_port: Arc<dyn CodexAppServerPort>) -> Self {
        Self {
            codex_app_server_port,
        }
    }
}

impl PlanningWorkerPort for AppServerPlanningWorkerAdapter {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse> {
        let (tx, rx) = mpsc::channel();
        let stream_result = self
            .codex_app_server_port
            .run_new_thread_stream_with_overrides(
                NewThreadStreamRequest {
                    cwd: request.workspace_directory.clone(),
                    prompt: request.prompt.clone(),
                    model: Some(PLANNING_WORKER_MODEL.to_string()),
                    reasoning_effort: Some(PLANNING_WORKER_REASONING_EFFORT),
                },
                tx,
            );

        let mut final_agent_message = None;
        let mut changed_planning_file_paths = Vec::new();
        let mut failure_message = None;

        if let Err(error) = stream_result {
            return Err(error);
        }

        for event in rx.iter() {
            match event {
                ConversationStreamEvent::AgentMessageCompleted { text, .. } => {
                    final_agent_message = Some(text);
                }
                ConversationStreamEvent::TurnCompleted {
                    changed_planning_file_paths: paths,
                    ..
                } => {
                    changed_planning_file_paths = paths;
                }
                ConversationStreamEvent::ThreadPrepared { .. }
                | ConversationStreamEvent::TurnStarted { .. }
                | ConversationStreamEvent::StatusUpdated { .. }
                | ConversationStreamEvent::AgentMessageDelta { .. }
                | ConversationStreamEvent::ToolActivity { .. }
                | ConversationStreamEvent::ApprovalReviewUpdated { .. } => {}
                ConversationStreamEvent::Failed { message } => {
                    failure_message = Some(message);
                }
            }
        }

        if let Some(message) = failure_message {
            return Err(anyhow!("planning worker stream failed: {message}"));
        }

        Ok(PlanningWorkerResponse {
            operation: request.operation,
            final_agent_message,
            changed_planning_file_paths,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::mpsc::Sender;

    use anyhow::Result;

    use super::AppServerPlanningWorkerAdapter;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort, NewThreadReasoningEffort,
        NewThreadStreamRequest,
    };
    use crate::application::port::outbound::planning_worker_port::{
        PlanningWorkerOperation, PlanningWorkerPort, PlanningWorkerRequest,
    };
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
    use crate::domain::recent_sessions::RecentSessions;

    struct FakeCodexAppServerPort {
        events: Vec<ConversationStreamEvent>,
        requests: Mutex<Vec<NewThreadStreamRequest>>,
    }

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            unreachable!("not used in test")
        }

        fn load_recent_sessions(&self, _limit: usize) -> Result<RecentSessions> {
            unreachable!("not used in test")
        }

        fn load_conversation_snapshot(&self, _thread_id: &str) -> Result<ConversationSnapshot> {
            unreachable!("not used in test")
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            event_sender: Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            for event in self.events.clone() {
                let _ = event_sender.send(event);
            }
            Ok(())
        }

        fn run_new_thread_stream_with_overrides(
            &self,
            request: NewThreadStreamRequest,
            event_sender: Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            self.requests
                .lock()
                .expect("requests lock should succeed")
                .push(request);
            self.run_new_thread_stream("", "", event_sender)
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _event_sender: Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            unreachable!("not used in test")
        }
    }

    #[test]
    fn run_planning_session_collects_completed_message_and_changed_paths() {
        let fake_port = Arc::new(FakeCodexAppServerPort {
            events: vec![
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Planner".to_string(),
                    cwd: "/tmp/workspace".to_string(),
                },
                ConversationStreamEvent::AgentMessageCompleted {
                    item_id: "item-1".to_string(),
                    phase: None,
                    text: "planning updated".to_string(),
                },
                ConversationStreamEvent::TurnCompleted {
                    turn_id: "turn-1".to_string(),
                    changed_planning_file_paths: vec![
                        ".codex-exec-loop/planning/task-ledger.json".to_string(),
                    ],
                },
            ],
            requests: Mutex::new(Vec::new()),
        });
        let adapter = AppServerPlanningWorkerAdapter::new(fake_port.clone());

        let result = adapter
            .run_planning_session(PlanningWorkerRequest {
                operation: PlanningWorkerOperation::RefreshQueue,
                workspace_directory: "/tmp/workspace".to_string(),
                prompt: "refresh".to_string(),
            })
            .expect("planning worker should succeed");

        assert_eq!(
            result.final_agent_message.as_deref(),
            Some("planning updated")
        );
        assert_eq!(
            result.changed_planning_file_paths,
            vec![".codex-exec-loop/planning/task-ledger.json".to_string()]
        );
        assert_eq!(
            fake_port
                .requests
                .lock()
                .expect("requests lock should succeed")
                .as_slice(),
            &[NewThreadStreamRequest {
                cwd: "/tmp/workspace".to_string(),
                prompt: "refresh".to_string(),
                model: Some("gpt-5.4".to_string()),
                reasoning_effort: Some(NewThreadReasoningEffort::Medium),
            }]
        );
    }

    #[test]
    fn run_planning_session_returns_error_when_stream_reports_failure() {
        let adapter = AppServerPlanningWorkerAdapter::new(Arc::new(FakeCodexAppServerPort {
            events: vec![ConversationStreamEvent::Failed {
                message: "planner crashed".to_string(),
            }],
            requests: Mutex::new(Vec::new()),
        }));

        let error = adapter
            .run_planning_session(PlanningWorkerRequest {
                operation: PlanningWorkerOperation::RepairTaskLedger,
                workspace_directory: "/tmp/workspace".to_string(),
                prompt: "repair".to_string(),
            })
            .expect_err("failed stream should surface as error");

        assert!(error.to_string().contains("planner crashed"));
    }
}
