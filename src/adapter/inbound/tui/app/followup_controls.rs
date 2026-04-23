use super::{AutoFollowState, ConversationViewModel, StopKeywordRule};

#[derive(Debug, Clone)]
pub(super) enum FollowupControlEvent {
    DraftWorkspaceSynced { workspace_directory: String },
    AutoFollowToggled,
    AutoFollowStopped,
    MaxAutoTurnsUpdated { value: String },
    StopKeywordToggled,
    StopKeywordValueUpdated { value: String },
    NoFileChangeStopToggled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FollowupControlEffect {
    OverlayUi,
    MaxAutoTurnsEditor { value: String },
    StopKeywordEditor { value: String },
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
        FollowupControlEvent::AutoFollowToggled => {
            if state.auto_follow_state.enabled {
                state.auto_follow_state.stop();
                state.record_automation_stopped();
                state.status_text = "automation off".to_string();
            } else {
                state.auto_follow_state.enable();
                state.clear_auto_followup_skip();
                state.status_text = "automation on".to_string();
            }
        }
        FollowupControlEvent::AutoFollowStopped => {
            state.auto_follow_state.stop();
            state.record_automation_stopped();
            state.status_text = "automation off".to_string();
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
        FollowupControlEvent::StopKeywordToggled => {
            state.auto_follow_state.toggle_stop_keyword();
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto stop keyword {}",
                state.auto_follow_state.stop_keyword_label()
            );
        }
        FollowupControlEvent::StopKeywordValueUpdated { value } => {
            let Some(value) = StopKeywordRule::normalize_candidate(&value) else {
                state.status_text =
                    "auto stop keyword must use only letters, numbers, or underscores".to_string();
                return FollowupControlReduction { state, effects };
            };

            state
                .auto_follow_state
                .set_stop_keyword_value(value.clone());
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto stop keyword value {}",
                state.auto_follow_state.stop_keyword_label()
            );
            effects.push(FollowupControlEffect::StopKeywordEditor { value });
        }
        FollowupControlEvent::NoFileChangeStopToggled => {
            state.auto_follow_state.toggle_no_file_change_stop();
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto stop without file changes {}",
                state.auto_follow_state.no_file_change_stop_label()
            );
        }
    }

    FollowupControlReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::{
        AutoFollowupSkipReason, DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
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
    fn toggling_auto_follow_clears_skip_and_updates_status() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.record_auto_followup_skip(AutoFollowupSkipReason::NoAgentReply);

        let reduced = reduce_followup_controls(state, FollowupControlEvent::AutoFollowToggled);

        assert!(!reduced.state.auto_follow_state.enabled);
        assert_eq!(reduced.state.status_text, "automation off");
        assert_eq!(
            reduced
                .state
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str()),
            Some("stopped: automation off")
        );
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
    fn updating_stop_keyword_value_clears_skip_and_emits_editor_sync() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.record_auto_followup_skip(AutoFollowupSkipReason::NoAgentReply);

        let reduced = reduce_followup_controls(
            state,
            FollowupControlEvent::StopKeywordValueUpdated {
                value: "DONE_NOW".to_string(),
            },
        );

        assert_eq!(
            reduced.state.auto_follow_state.stop_keyword_value(),
            "DONE_NOW"
        );
        assert!(reduced.state.last_auto_followup_activity.is_none());
        assert_eq!(
            reduced.effects,
            vec![FollowupControlEffect::StopKeywordEditor {
                value: "DONE_NOW".to_string()
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
    fn invalid_stop_keyword_value_keeps_existing_rule() {
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_followup_controls(
            state,
            FollowupControlEvent::StopKeywordValueUpdated {
                value: "not-valid!".to_string(),
            },
        );

        assert_eq!(
            reduced.state.auto_follow_state.stop_keyword_value(),
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
        assert!(reduced.effects.is_empty());
    }
}
