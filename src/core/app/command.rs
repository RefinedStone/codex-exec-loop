use super::TurnSubmissionRequest;
use crate::application::service::manual_prompt_preparation::ManualPromptPreparationRequest;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    /*
     * Noop gives the skeleton a real command path without changing product
     * behavior. Feature slices replace this with domain-specific commands such
     * as startup/session/conversation orchestration.
     */
    Noop,
    RunStartupChecks,
    LoadSessionCatalog {
        limit: usize,
        workspace_directory: String,
    },
    LoadConversation {
        thread_id: String,
    },
    PrepareManualPrompt(Box<ManualPromptPreparationRequest>),
    SubmitTurn(TurnSubmissionRequest),
    EvaluatePostTurn(Box<PostTurnEvaluationRequest>),
}
