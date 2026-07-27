use super::normalize_max_auto_turns_candidate;
use crate::core::app::AppCommand;

/*
 * Terminal controls emit intent only. Runtime budget, pause, and rearm state are
 * reduced by Core; the TUI keeps only the unfinished editor buffer and status
 * copy derived after the authoritative snapshot is applied.
 */
#[derive(Debug, Clone)]
pub(super) enum AutoFollowControlEvent {
    DraftWorkspaceSynced { workspace_directory: String },
    AutoFollowPaused,
    PlanningAuthorityMutationSettled,
    MaxAutoTurnsUpdated { value: String },
}

pub(super) fn max_auto_turns_command(value: &str) -> Option<AppCommand> {
    normalize_max_auto_turns_candidate(value)
        .map(|value| AppCommand::SetAutoFollowMaxTurns { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_budget_maps_to_core_command() {
        assert!(matches!(
            max_auto_turns_command("5"),
            Some(AppCommand::SetAutoFollowMaxTurns { value: 5 })
        ));
        assert!(matches!(
            max_auto_turns_command("off"),
            Some(AppCommand::SetAutoFollowMaxTurns { value: 0 })
        ));
    }

    #[test]
    fn invalid_budget_does_not_fabricate_a_runtime_write() {
        assert!(max_auto_turns_command("not-a-budget").is_none());
    }
}
