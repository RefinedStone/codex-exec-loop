use crate::domain::planning::ManualPromptCorrelation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptPreparationIntent {
    pub workspace_directory: String,
    pub raw_prompt: String,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualPromptPreparationAdmission {
    Accepted {
        correlation: ManualPromptCorrelation,
    },
    RejectedActive {
        active_correlation: ManualPromptCorrelation,
    },
}
