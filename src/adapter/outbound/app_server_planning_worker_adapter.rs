use std::sync::Arc;
use std::sync::mpsc;
use std::sync::mpsc::Sender;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;

pub(crate) trait PlanningThreadLauncher: Send + Sync {
    fn run_hidden_planning_thread(
        &self,
        workspace_directory: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct AppServerPlanningWorkerAdapter {
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
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse> {
        let (tx, rx) = mpsc::channel();
        let stream_result = self.planning_thread_launcher.run_hidden_planning_thread(
            &request.workspace_directory,
            &request.prompt,
            tx,
        );

        let mut final_agent_message = None;
        let mut changed_planning_file_paths = Vec::new();
        let mut failure_message = None;

        stream_result?;

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
            event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
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
        let fake_launcher = Arc::new(FakePlanningThreadLauncher {
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
            calls: Mutex::new(Vec::new()),
        });
        let adapter = AppServerPlanningWorkerAdapter::new(fake_launcher.clone());

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
    fn run_planning_session_returns_error_when_stream_reports_failure() {
        let adapter = AppServerPlanningWorkerAdapter::new(Arc::new(FakePlanningThreadLauncher {
            events: vec![ConversationStreamEvent::Failed {
                message: "planner crashed".to_string(),
            }],
            calls: Mutex::new(Vec::new()),
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
