use chrono::Utc;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const COMMAND_HISTORY_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AkraCommandState {
    Accepted,
    Running,
    Completed,
    Blocked,
}

impl AkraCommandState {
    fn is_settled(self) -> bool {
        matches!(self, Self::Completed | Self::Blocked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraCommandView {
    pub command_id: String,
    pub action: String,
    pub state: AkraCommandState,
    pub submitted_at: String,
    pub settled_at: Option<String>,
    pub message: String,
}

#[derive(Debug, Default)]
struct AdminCommandLedgerState {
    next_sequence: u64,
    commands: VecDeque<AkraCommandView>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct AdminCommandLedger {
    state: Arc<Mutex<AdminCommandLedgerState>>,
}

impl AdminCommandLedger {
    pub fn begin(&self, action: &str, message: &str) -> AkraCommandView {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.next_sequence = state.next_sequence.saturating_add(1);
        let command = AkraCommandView {
            command_id: format!(
                "admin-command-{}-{}",
                Utc::now().timestamp_millis(),
                state.next_sequence
            ),
            action: action.to_string(),
            state: AkraCommandState::Accepted,
            submitted_at: Utc::now().to_rfc3339(),
            settled_at: None,
            message: message.to_string(),
        };
        state.commands.push_back(command.clone());
        while state.commands.len() > COMMAND_HISTORY_LIMIT {
            state.commands.pop_front();
        }
        command
    }

    pub fn reconcile_latest(
        &self,
        control_effect_in_flight: bool,
        last_dispatch_withheld_reason: Option<&str>,
    ) -> Option<AkraCommandView> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let command = state.commands.back_mut()?;
        if command.state.is_settled() {
            return Some(command.clone());
        }
        if control_effect_in_flight {
            command.state = AkraCommandState::Running;
            return Some(command.clone());
        }
        if command.action == "dispatch"
            && let Some(reason) = last_dispatch_withheld_reason.filter(|reason| !reason.is_empty())
        {
            command.state = AkraCommandState::Blocked;
            command.message = reason.to_string();
        } else {
            command.state = AkraCommandState::Completed;
        }
        command.settled_at = Some(Utc::now().to_rfc3339());
        Some(command.clone())
    }

    pub fn get(&self, command_id: &str) -> Option<AkraCommandView> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .commands
            .iter()
            .find(|command| command.command_id == command_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::{AdminCommandLedger, AkraCommandState};

    #[test]
    fn command_ledger_tracks_running_and_settled_lifecycle() {
        let ledger = AdminCommandLedger::default();
        let accepted = ledger.begin("enable", "accepted");
        assert_eq!(accepted.state, AkraCommandState::Accepted);

        let running = ledger
            .reconcile_latest(true, None)
            .expect("latest command should exist");
        assert_eq!(running.state, AkraCommandState::Running);
        assert!(running.settled_at.is_none());

        let completed = ledger
            .reconcile_latest(false, None)
            .expect("latest command should exist");
        assert_eq!(completed.state, AkraCommandState::Completed);
        assert!(completed.settled_at.is_some());
        assert_eq!(ledger.get(&accepted.command_id), Some(completed));
    }

    #[test]
    fn dispatch_withheld_reason_settles_command_as_blocked() {
        let ledger = AdminCommandLedger::default();
        ledger.begin("dispatch", "accepted");

        let blocked = ledger
            .reconcile_latest(false, Some("pool unavailable"))
            .expect("latest command should exist");
        assert_eq!(blocked.state, AkraCommandState::Blocked);
        assert_eq!(blocked.message, "pool unavailable");
    }
}
