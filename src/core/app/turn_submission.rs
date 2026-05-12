use crate::application::service::parallel_mode::turn::ParallelTurnSlotLeaseHandoff;
use crate::domain::conversation::ConversationTurnOptions;

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
    pub slot_lease_handoff: Option<ParallelTurnSlotLeaseHandoff>,
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
