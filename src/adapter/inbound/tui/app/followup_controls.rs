use super::{AutoFollowState, ConversationViewModel};

#[derive(Debug, Clone)]
pub(super) enum FollowupControlEvent {
    DraftWorkspaceSynced { workspace_directory: String },
    AutoFollowPaused,
    MaxAutoTurnsUpdated { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FollowupControlEffect {
    OverlayUi,
    MaxAutoTurnsEditor { value: String },
}

#[derive(Debug, Clone)]
pub(super) struct FollowupControlReduction {
    pub state: ConversationViewModel,
    pub effects: Vec<FollowupControlEffect>,
}

pub(super) fn reduce_followup_controls(
    mut state: ConversationViewModel,
    event: FollowupControlEvent,
) -> FollowupControlReduction {
    let mut effects = Vec::new();

    match event {
        FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory,
        } => {
            if state.sync_draft_workspace(workspace_directory) {
                effects.push(FollowupControlEffect::OverlayUi);
            }
        }
        FollowupControlEvent::AutoFollowPaused => {
            state.pause_post_turn_continuation();
            state.record_internal_continuation_paused();
            state.status_text = "internal continuation paused".to_string();
        }
        FollowupControlEvent::MaxAutoTurnsUpdated { value } => {
            let Some(value) = AutoFollowState::normalize_max_auto_turns_candidate(&value) else {
                state.status_text = "auto follow-up max turns must be a whole number greater than 0 or the word infinite".to_string();
                return FollowupControlReduction { state, effects };
            };

            state.auto_follow_state.set_max_auto_turns(value);
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto follow-up max turns {}",
                state.auto_follow_state.max_auto_turns_label()
            );
            effects.push(FollowupControlEffect::MaxAutoTurnsEditor {
                value: state.auto_follow_state.max_auto_turns_label(),
            });
        }
    }

    FollowupControlReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::{
        AutoFollowupSkipReason, DEFAULT_AUTO_FOLLOW_MAX_TURNS,
    };

    #[test]
    fn draft_workspace_sync_updates_blank_draft_and_emits_ui_sync() {
        let draft = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_followup_controls(
            draft,
            FollowupControlEvent::DraftWorkspaceSynced {
                workspace_directory: "/tmp/alt".to_string(),
            },
        );

        assert_eq!(reduced.state.cwd, "/tmp/alt");
        assert!(reduced.state.status_text.contains("draft workspace synced"));
        assert_eq!(reduced.effects, vec![FollowupControlEffect::OverlayUi]);
    }

    #[test]
    fn draft_workspace_sync_clears_skip_state() {
        let mut draft = ConversationViewModel::new_draft("/tmp/root".to_string());
        draft.record_auto_followup_skip(AutoFollowupSkipReason::NoAgentReply);

        let reduced = reduce_followup_controls(
            draft,
            FollowupControlEvent::DraftWorkspaceSynced {
                workspace_directory: "/tmp/alt".to_string(),
            },
        );

        assert!(reduced.state.last_auto_followup_activity.is_none());
    }

    #[test]
    fn updating_max_auto_turns_clears_skip_and_emits_editor_sync() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.record_auto_followup_skip(AutoFollowupSkipReason::NoAgentReply);

        let reduced = reduce_followup_controls(
            state,
            FollowupControlEvent::MaxAutoTurnsUpdated {
                value: "5".to_string(),
            },
        );

        assert_eq!(reduced.state.auto_follow_state.max_auto_turns_value(), 5);
        assert!(reduced.state.last_auto_followup_activity.is_none());
        assert_eq!(
            reduced.effects,
            vec![FollowupControlEffect::MaxAutoTurnsEditor {
                value: "5".to_string()
            }]
        );
    }

    #[test]
    fn invalid_max_auto_turns_keeps_existing_limit() {
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_followup_controls(
            state,
            FollowupControlEvent::MaxAutoTurnsUpdated {
                value: "0".to_string(),
            },
        );

        assert_eq!(
            reduced.state.auto_follow_state.max_auto_turns_value(),
            DEFAULT_AUTO_FOLLOW_MAX_TURNS
        );
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn pausing_internal_continuation_keeps_running_phase_for_turn_budget() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.auto_follow_state.mark_auto_turn_submitted();

        let reduced = reduce_followup_controls(state, FollowupControlEvent::AutoFollowPaused);

        assert!(reduced.state.auto_follow_state.has_live_activity());
        assert!(
            reduced
                .state
                .auto_follow_state
                .post_turn_continuation_paused()
        );
        assert_eq!(reduced.state.auto_follow_state.completed_auto_turns, 0);
    }
}
