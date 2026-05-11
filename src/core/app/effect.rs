use super::TurnSubmissionRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffect {
    RunStartupChecks,
    LoadSessionCatalog {
        limit: usize,
        workspace_directory: String,
    },
    LoadConversation {
        thread_id: String,
    },
    SubmitTurn(TurnSubmissionRequest),
}
