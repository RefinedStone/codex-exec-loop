use crate::domain::conversation::{ConversationToolActivity, ConversationToolActivityKind};

/*
 * Side-channel activity summary for the active and most recently completed turn.
 * The transcript keeps the full message stream; this state keeps the small counters and latest activity
 * label that footer/tail rendering and auto-follow stop rules need after stream events have been reduced.
 */
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnActivityState {
    // Streaming bucket for tool-file-change events observed before the turn completes.
    pub(crate) current_turn_file_change_count: usize,
    // Counts command execution boundaries, not command output lines.
    pub(crate) current_turn_command_count: usize,
    // Latest activity sentence for the compact live-status line; full history stays in transcript messages.
    pub(crate) current_turn_last_summary: Option<String>,
    // Planning artifacts finalized at turn completion, de-duplicated for post-turn planning evaluation.
    pub(crate) current_turn_changed_planning_file_paths: Vec<String>,
    // Snapshot moved from the current bucket at finish_turn, kept for idle footer copy and auto-follow decisions.
    pub(crate) last_completed_turn_id: Option<String>,
    pub(crate) last_completed_turn_file_change_count: usize,
    pub(crate) last_completed_turn_command_count: usize,
    pub(crate) last_completed_turn_last_summary: Option<String>,
    pub(crate) last_completed_turn_changed_planning_file_paths: Vec<String>,
}

// State machine for streaming accumulation, completion rollover, and presentation bucket selection.
impl TurnActivityState {
    // Starting a turn clears only live activity; last_completed remains available until new activity arrives.
    pub(crate) fn start_new_turn(&mut self) {
        self.current_turn_file_change_count = 0;
        self.current_turn_command_count = 0;
        self.current_turn_last_summary = None;
        self.current_turn_changed_planning_file_paths.clear();
    }

    // Register one tool-activity event emitted by the conversation stream reducer.
    pub(crate) fn register_tool_activity(&mut self, activity: &ConversationToolActivity) {
        self.current_turn_last_summary = Some(activity.text.clone());
        match activity.kind {
            // File-change events may report several files, so add their payload count.
            ConversationToolActivityKind::FileChange => {
                self.current_turn_file_change_count += activity.file_change_count;
            }
            // Command events count execution boundaries regardless of output size or exit status.
            ConversationToolActivityKind::CommandExecution => {
                self.current_turn_command_count += 1;
            }
        }
    }

    // Move live activity into the completed bucket before the active-turn flag is cleared.
    pub(crate) fn complete_turn(&mut self, turn_id: &str) {
        self.last_completed_turn_id = Some(turn_id.to_string());
        // replace/take make the rollover atomic from the model's perspective: completed gets the value, current resets.
        self.last_completed_turn_file_change_count =
            std::mem::replace(&mut self.current_turn_file_change_count, 0);
        self.last_completed_turn_command_count =
            std::mem::replace(&mut self.current_turn_command_count, 0);
        self.last_completed_turn_last_summary = self.current_turn_last_summary.take();
        self.last_completed_turn_changed_planning_file_paths =
            std::mem::take(&mut self.current_turn_changed_planning_file_paths);
    }

    // Register planning artifacts determined by finish_turn rather than streaming tool events.
    pub(crate) fn register_changed_planning_file_paths(&mut self, paths: &[String]) {
        for path in paths {
            // The list is small and order can matter in diagnostics, so use linear de-duplication instead of a set.
            if !self
                .current_turn_changed_planning_file_paths
                .iter()
                .any(|existing| existing == path)
            {
                self.current_turn_changed_planning_file_paths
                    .push(path.clone());
            }
        }
    }

    // Auto-follow no-file-change rules read only the completed bucket, never partial streaming state.
    pub(crate) fn last_completed_file_change_count(&self) -> usize {
        self.last_completed_turn_file_change_count
    }

    // Current activity may briefly outlive the running flag during finish/flush ordering.
    fn has_current_turn_activity(&self) -> bool {
        self.current_turn_file_change_count > 0
            || self.current_turn_command_count > 0
            || self.current_turn_last_summary.is_some()
    }

    // Label the bucket that presentation will read for activity counts and summary.
    pub(crate) fn activity_scope_label(&self, turn_running: bool) -> &'static str {
        if turn_running {
            "current turn"
        } else if self.has_current_turn_activity() {
            "recent turn"
        } else {
            "last turn"
        }
    }

    // Select command count from the same bucket as the scope label.
    pub(crate) fn activity_command_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_command_count
        } else {
            self.last_completed_turn_command_count
        }
    }

    // Select file-change count from the same bucket as command count so footer copy cannot mix scopes.
    pub(crate) fn activity_file_change_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_file_change_count
        } else {
            self.last_completed_turn_file_change_count
        }
    }

    // Select latest summary from the same bucket; "none" is the sentinel consumed by tail_shared.
    pub(crate) fn activity_summary(&self, turn_running: bool) -> &str {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_last_summary.as_deref().unwrap_or("none")
        } else {
            self.last_completed_turn_last_summary
                .as_deref()
                .unwrap_or("none")
        }
    }
}
