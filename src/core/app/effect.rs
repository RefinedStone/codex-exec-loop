use super::TurnSubmissionRequest;
use crate::domain::planning::{ManualPromptRequest, PostTurnRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffect {
    RunStartupChecks,
    LoadSessionCatalog {
        limit: usize,
        workspace_directory: String,
    },
    LoadConversation {
        thread_id: String,
        fallback_workspace_directory: String,
    },
    PrepareManualPrompt(Box<ManualPromptRequest>),
    SubmitTurn(TurnSubmissionRequest),
    EvaluatePostTurn(Box<PostTurnRequest>),
}
