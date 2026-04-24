use crate::application::service::planning::{
    PlanningTaskIntakeCommitResult, PlanningTaskIntakeProposal,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskIntakeOverlayStep {
    Prompt,
    Preview,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TaskIntakeOverlayUiState {
    prompt_buffer: String,
    proposal: Option<PlanningTaskIntakeProposal>,
    commit_result: Option<PlanningTaskIntakeCommitResult>,
    error: Option<String>,
    step: TaskIntakeOverlayStep,
}

impl Default for TaskIntakeOverlayStep {
    fn default() -> Self {
        Self::Prompt
    }
}

impl TaskIntakeOverlayUiState {
    pub(super) fn open(&mut self, prompt: Option<&str>) {
        self.prompt_buffer = prompt.unwrap_or_default().trim().to_string();
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn step(&self) -> TaskIntakeOverlayStep {
        self.step
    }

    pub(super) fn prompt_buffer(&self) -> &str {
        &self.prompt_buffer
    }

    pub(super) fn proposal(&self) -> Option<&PlanningTaskIntakeProposal> {
        self.proposal.as_ref()
    }

    pub(super) fn commit_result(&self) -> Option<&PlanningTaskIntakeCommitResult> {
        self.commit_result.as_ref()
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn push_character(&mut self, character: char) {
        self.prompt_buffer.push(character);
        self.proposal = None;
        self.error = None;
    }

    pub(super) fn pop_character(&mut self) {
        self.prompt_buffer.pop();
        self.proposal = None;
        self.error = None;
    }

    pub(super) fn clear_prompt(&mut self) {
        self.prompt_buffer.clear();
        self.proposal = None;
        self.error = None;
    }

    pub(super) fn show_preview(&mut self, proposal: PlanningTaskIntakeProposal) {
        self.proposal = Some(proposal);
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Preview;
    }

    pub(super) fn return_to_editing(&mut self) {
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    pub(super) fn show_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    pub(super) fn record_commit_result(&mut self, result: PlanningTaskIntakeCommitResult) {
        self.commit_result = Some(result);
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskIntakeOverlayStep, TaskIntakeOverlayUiState};

    #[test]
    fn open_resets_preview_and_preserves_initial_prompt() {
        let mut state = TaskIntakeOverlayUiState::default();
        state.show_error("old error");

        state.open(Some("  ship task intake  "));

        assert_eq!(state.step(), TaskIntakeOverlayStep::Prompt);
        assert_eq!(state.prompt_buffer(), "ship task intake");
        assert!(state.error().is_none());
        assert!(state.proposal().is_none());
    }
}
