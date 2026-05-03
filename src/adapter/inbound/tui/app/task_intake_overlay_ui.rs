use crate::application::service::planning::{
    PlanningTaskIntakeCommitResult, PlanningTaskIntakeProposal,
};

// Modal phase for `:task`, shared by controller key handling and popup key-copy projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum TaskIntakeOverlayStep {
    // Raw prompt editing; Enter asks the planning runtime to prepare a task proposal.
    #[default]
    Prompt,
    // Concrete proposal review; keys switch to commit/edit/cancel instead of text editing.
    Preview,
}

/*
 * Controller-owned state for the `:task` overlay.
 * The planning runtime prepares and commits task proposals; this struct only preserves the modal snapshot
 * that rendering needs and centralizes invalidation rules so stale proposals or errors do not survive prompt edits.
 */
#[derive(Debug, Clone, Default)]
pub(super) struct TaskIntakeOverlayUiState {
    // Raw prompt as edited in the modal; controller trims it only when constructing the runtime request.
    prompt_buffer: String,
    // Runtime-generated proposal tied to the prompt at preview time.
    proposal: Option<PlanningTaskIntakeProposal>,
    // Transient accepted task/revision, mostly visible in tests or the frame before queue overlay opens.
    commit_result: Option<PlanningTaskIntakeCommitResult>,
    // Prepare/commit failure or invalid action message shown in the status lane.
    error: Option<String>,
    // Single state-machine axis that keeps key handling and displayed key lines in sync.
    step: TaskIntakeOverlayStep,
}

// Transition API used by shell_controller; fields stay private to keep cleanup rules local.
impl TaskIntakeOverlayUiState {
    // Start a fresh intake session from an optional inline command argument.
    pub(super) fn open(&mut self, prompt: Option<&str>) {
        self.prompt_buffer = prompt.unwrap_or_default().trim().to_string();
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        // Even argument-backed commands enter Prompt first; controller may immediately call preview afterward.
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    // Clear the modal after close or after a successful commit hands control to the queue overlay.
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    // The step is read by both key handling and popup projection.
    pub(super) fn step(&self) -> TaskIntakeOverlayStep {
        self.step
    }

    // Expose the editable prompt without giving callers mutation access that could bypass invalidation.
    pub(super) fn prompt_buffer(&self) -> &str {
        &self.prompt_buffer
    }

    // Commit and render paths read the same prepared proposal snapshot.
    pub(super) fn proposal(&self) -> Option<&PlanningTaskIntakeProposal> {
        self.proposal.as_ref()
    }

    // Result is optional because normal success immediately transitions to the queue overlay.
    pub(super) fn commit_result(&self) -> Option<&PlanningTaskIntakeCommitResult> {
        self.commit_result.as_ref()
    }

    // Borrow the current status error so rendering does not clone service error text just to inspect it.
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    // Prompt edits invalidate the prepared proposal because it describes the previous prompt text.
    pub(super) fn push_character(&mut self, character: char) {
        self.prompt_buffer.push(character);
        self.proposal = None;
        self.error = None;
    }

    // Backspace follows the same invalidation rule even when the buffer was already empty.
    pub(super) fn pop_character(&mut self) {
        self.prompt_buffer.pop();
        self.proposal = None;
        self.error = None;
    }

    // Ctrl+u returns the overlay to an unprepared prompt state.
    pub(super) fn clear_prompt(&mut self) {
        self.prompt_buffer.clear();
        self.proposal = None;
        self.error = None;
    }

    // Store a runtime proposal and switch the keymap from editing to commit review.
    pub(super) fn show_preview(&mut self, proposal: PlanningTaskIntakeProposal) {
        self.proposal = Some(proposal);
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Preview;
    }

    // Keep the prompt but discard the proposal when the operator returns from Preview to editing.
    pub(super) fn return_to_editing(&mut self) {
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    // Errors are attached to the current step so the operator can retry or edit without losing context.
    pub(super) fn show_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    // Record commit success before controller refreshes planning state and moves to queue inspection.
    pub(super) fn record_commit_result(&mut self, result: PlanningTaskIntakeCommitResult) {
        self.commit_result = Some(result);
        self.error = None;
    }
}

// Tests exercise the modal state machine without booting the planning runtime service.
#[cfg(test)]
mod tests {
    use super::{TaskIntakeOverlayStep, TaskIntakeOverlayUiState};

    // Opening a new command must clear previous transient state and normalize the initial prompt.
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
