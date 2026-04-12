use std::sync::Arc;
use std::sync::mpsc;

use anyhow::Result;

use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::domain::conversation::ConversationStreamEvent;

#[derive(Clone)]
pub struct AppServerPlanningWorkerAdapter {
    codex_app_server_port: Arc<dyn CodexAppServerPort>,
}

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
        self.codex_app_server_port.run_new_thread_stream(
            &request.workspace_directory,
            &request.prompt,
            tx,
        )?;

        let mut final_agent_message = None;
        let mut changed_planning_file_paths = Vec::new();

        for event in rx.try_iter() {
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
                | ConversationStreamEvent::ApprovalReviewUpdated { .. }
                | ConversationStreamEvent::Failed { .. } => {}
            }
        }

        Ok(PlanningWorkerResponse {
            operation: request.operation,
            final_agent_message,
            changed_planning_file_paths,
        })
    }
}
