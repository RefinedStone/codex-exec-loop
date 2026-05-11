use super::TurnSubmissionRequest;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationRequest;

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
    EvaluatePostTurn(Box<PostTurnEvaluationRequest>),
}
