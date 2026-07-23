use crate::core::app::{
    AppCommand, ConversationReadySnapshot, CoreInput, ParallelPeekLoadCorrelation,
};
use crate::domain::parallel_mode::ParallelModeAgentRosterEntry;

use super::parallel_peek_overlay_ui::ParallelPeekConversationPreview;
use super::*;

impl NativeTuiApp {
    pub(super) fn open_parallel_peek_overlay(&mut self, argument: Option<&str>) {
        let active_agents = self.active_parallel_peek_entries();
        let active_agent_count = active_agents.len();
        self.parallel_peek_overlay_ui_state
            .select_initial_agent(&active_agents);
        self.dispatch_shell_chrome(ShellChromeEvent::ParallelPeekOverlayShown);

        let status_text = if argument.is_some() {
            "`:peek` ignores arguments; choose an active parallel agent from the picker".to_string()
        } else if active_agent_count == 0 {
            "parallel peek: no active parallel agents are available".to_string()
        } else {
            format!("parallel peek: {active_agent_count} active agent(s) available")
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(crate) fn active_parallel_peek_entries(&self) -> Vec<ParallelModeAgentRosterEntry> {
        self.parallel_mode_supervisor_snapshot()
            .roster
            .entries
            .iter()
            .filter(|entry| entry.counts_as_active())
            .cloned()
            .collect()
    }

    pub(super) fn handle_parallel_peek_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::ParallelPeek {
            return false;
        }

        let active_agents = self.active_parallel_peek_entries();
        self.parallel_peek_overlay_ui_state
            .sync_selection(&active_agents);

        match self.parallel_peek_overlay_ui_state.step() {
            ParallelPeekOverlayStep::AgentList => {
                self.handle_parallel_peek_agent_list_key(key, &active_agents)
            }
            ParallelPeekOverlayStep::ConversationPreview => {
                self.handle_parallel_peek_conversation_key(key)
            }
        }
    }

    fn handle_parallel_peek_agent_list_key(
        &mut self,
        key: event::KeyEvent,
        active_agents: &[ParallelModeAgentRosterEntry],
    ) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .move_selection(active_agents, -1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .move_selection(active_agents, 1);
                true
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.open_selected_parallel_peek_conversation(active_agents);
                true
            }
            KeyCode::Esc => {
                self.close_shell_overlay();
                true
            }
            KeyCode::Char('o') | KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.close_shell_overlay();
                true
            }
            _ => true,
        }
    }

    fn handle_parallel_peek_conversation_key(&mut self, key: event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .scroll_conversation_older(1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .scroll_conversation_newer(1);
                true
            }
            KeyCode::PageUp if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .scroll_conversation_older(10);
                true
            }
            KeyCode::PageDown if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .scroll_conversation_newer(10);
                true
            }
            KeyCode::Home if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .scroll_conversation_to_oldest();
                true
            }
            KeyCode::End if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state
                    .scroll_conversation_to_latest();
                true
            }
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace if key.modifiers.is_empty() => {
                self.parallel_peek_overlay_ui_state.back_to_agent_list();
                true
            }
            KeyCode::Char('o') | KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.close_shell_overlay();
                true
            }
            _ => true,
        }
    }

    fn open_selected_parallel_peek_conversation(
        &mut self,
        active_agents: &[ParallelModeAgentRosterEntry],
    ) {
        let Some(entry) = self.selected_parallel_peek_entry(active_agents) else {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "parallel peek: selected lease changed; choose an active agent again"
                    .to_string(),
            });
            return;
        };

        let thread_id = entry.thread_id;
        let preview = ParallelPeekConversationPreview {
            agent_id: entry.agent_id,
            slot_id: entry.slot_id,
            task_title: entry.task_title,
            thread_id: thread_id.clone(),
            snapshot: None,
            status_text: if thread_id.is_some() {
                "conversation snapshot loading".to_string()
            } else {
                "thread id has not been captured yet".to_string()
            },
        };
        let status_text = format!(
            "parallel peek: {} / {} / {}",
            preview.agent_id, preview.slot_id, preview.status_text
        );
        if let Some(thread_id) = thread_id {
            self.parallel_peek_overlay_ui_state.open_preview(preview);
            self.dispatch_client_event(CoreInput::Command(
                AppCommand::LoadParallelPeekConversation { thread_id },
            ));
        } else {
            self.parallel_peek_overlay_ui_state.open_preview(preview);
        }
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn apply_parallel_peek_conversation_load(
        &mut self,
        correlation: ParallelPeekLoadCorrelation,
        result: Result<Box<ConversationReadySnapshot>, String>,
    ) {
        if self.shell_overlay != ShellOverlay::ParallelPeek {
            return;
        }
        let result = result.map(|ready| *ready.conversation);
        if !self
            .parallel_peek_overlay_ui_state
            .complete_conversation_load(correlation.requested_thread_id.as_str(), result)
        {
            return;
        }

        let Some(preview) = self.parallel_peek_overlay_ui_state.preview() else {
            return;
        };
        let status_text = format!(
            "parallel peek: {} / {} / {}",
            preview.agent_id, preview.slot_id, preview.status_text
        );
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    fn selected_parallel_peek_entry(
        &self,
        active_agents: &[ParallelModeAgentRosterEntry],
    ) -> Option<ParallelModeAgentRosterEntry> {
        let selected_agent_index = self
            .parallel_peek_overlay_ui_state
            .selected_agent_index(active_agents)?;
        active_agents.get(selected_agent_index).cloned()
    }
}
