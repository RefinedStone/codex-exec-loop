use crate::domain::conversation::ConversationTurnOptions;
use crate::domain::planning::ParallelTurnHandoff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorePromptOrigin {
    Manual,
    ManualIntake,
    AutoFollow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSubmissionRequest {
    pub workspace_directory: String,
    pub thread_id: Option<String>,
    pub prompt: String,
    pub prompt_origin: CorePromptOrigin,
    pub turn_options: ConversationTurnOptions,
    pub slot_lease_handoff: Option<ParallelTurnHandoff>,
}

impl TurnSubmissionRequest {
    pub(crate) fn request_label(&self) -> &'static str {
        if self.thread_id.is_some() {
            "turn stream"
        } else {
            "new-thread stream"
        }
    }
}
