use super::{ConversationLoadCorrelation, StartupCheckCorrelation, TurnSubmissionRequest};
use crate::domain::planning::{ManualPromptRequest, PostTurnRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffect {
    RunStartupChecks {
        correlation: StartupCheckCorrelation,
    },
    LoadSessionCatalog {
        limit: usize,
        workspace_directory: String,
    },
    LoadConversation {
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
    },
    LoadParallelPeekConversation {
        request_id: u64,
        thread_id: String,
    },
    PrepareManualPrompt(Box<ManualPromptRequest>),
    SubmitTurn(TurnSubmissionRequest),
    EvaluatePostTurn(Box<PostTurnRequest>),
}
