use crate::domain::conversation::{ConversationToolActivity, ConversationToolActivityKind};

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnActivityState {
    pub(crate) current_turn_file_change_count: usize,
    pub(crate) current_turn_command_count: usize,
    pub(crate) current_turn_last_summary: Option<String>,
    pub(crate) current_turn_changed_planning_file_paths: Vec<String>,
    pub(crate) last_completed_turn_id: Option<String>,
    pub(crate) last_completed_turn_file_change_count: usize,
    pub(crate) last_completed_turn_command_count: usize,
    pub(crate) last_completed_turn_last_summary: Option<String>,
    pub(crate) last_completed_turn_changed_planning_file_paths: Vec<String>,
}

impl TurnActivityState {
    pub(crate) fn start_new_turn(&mut self) {
        self.current_turn_file_change_count = 0;
        self.current_turn_command_count = 0;
        self.current_turn_last_summary = None;
        self.current_turn_changed_planning_file_paths.clear();
    }

    pub(crate) fn register_tool_activity(&mut self, activity: &ConversationToolActivity) {
        self.current_turn_last_summary = Some(activity.text.clone());
        match activity.kind {
            ConversationToolActivityKind::FileChange => {
                self.current_turn_file_change_count += activity.file_change_count;
            }
            ConversationToolActivityKind::CommandExecution => {
                self.current_turn_command_count += 1;
            }
        }
    }

    pub(crate) fn complete_turn(&mut self, turn_id: &str) {
        self.last_completed_turn_id = Some(turn_id.to_string());
        self.last_completed_turn_file_change_count =
            std::mem::replace(&mut self.current_turn_file_change_count, 0);
        self.last_completed_turn_command_count =
            std::mem::replace(&mut self.current_turn_command_count, 0);
        self.last_completed_turn_last_summary = self.current_turn_last_summary.take();
        self.last_completed_turn_changed_planning_file_paths =
            std::mem::take(&mut self.current_turn_changed_planning_file_paths);
    }

    pub(crate) fn register_changed_planning_file_paths(&mut self, paths: &[String]) {
        for path in paths {
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

    pub(crate) fn last_completed_file_change_count(&self) -> usize {
        self.last_completed_turn_file_change_count
    }

    fn has_current_turn_activity(&self) -> bool {
        self.current_turn_file_change_count > 0
            || self.current_turn_command_count > 0
            || self.current_turn_last_summary.is_some()
    }

    pub(crate) fn activity_scope_label(&self, turn_running: bool) -> &'static str {
        if turn_running {
            "current turn"
        } else if self.has_current_turn_activity() {
            "recent turn"
        } else {
            "last turn"
        }
    }

    pub(crate) fn activity_command_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_command_count
        } else {
            self.last_completed_turn_command_count
        }
    }

    pub(crate) fn activity_file_change_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_file_change_count
        } else {
            self.last_completed_turn_file_change_count
        }
    }

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
