use super::*;

#[derive(Debug, Clone)]
pub(super) enum FollowupControlEvent {
    DraftWorkspaceSynced {
        workspace_directory: String,
        template_load_result: FollowupTemplateCatalogLoadResult,
    },
    AutoFollowToggled,
    StopKeywordToggled,
    NoFileChangeStopToggled,
    TemplateCycledForward,
    TemplateCycledBackward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FollowupControlEffect {
    SyncTemplateOverlayUi,
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
            template_load_result,
        } => {
            if !state.has_active_thread() && state.cwd != workspace_directory {
                let template_count = template_load_result.catalog.items.len();
                state.cwd = workspace_directory;
                state.auto_follow_state = AutoFollowState::new(template_load_result.catalog);
                state.warnings = template_load_result.warnings;
                state.status_text = format!("draft workspace synced / templates: {template_count}");
                effects.push(FollowupControlEffect::SyncTemplateOverlayUi);
            }
        }
        FollowupControlEvent::AutoFollowToggled => {
            state.auto_follow_state.toggle();
            state.clear_auto_followup_skip();
            state.status_text =
                format!("auto follow-up {}", state.auto_follow_state.status_label());
        }
        FollowupControlEvent::StopKeywordToggled => {
            state.auto_follow_state.toggle_stop_keyword();
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto stop keyword {}",
                state.auto_follow_state.stop_keyword_label()
            );
        }
        FollowupControlEvent::NoFileChangeStopToggled => {
            state.auto_follow_state.toggle_no_file_change_stop();
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto stop without file changes {}",
                state.auto_follow_state.no_file_change_stop_label()
            );
        }
        FollowupControlEvent::TemplateCycledForward => {
            state.auto_follow_state.cycle_template_kind();
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto follow-up template: {}",
                state.auto_follow_state.template_label()
            );
            effects.push(FollowupControlEffect::SyncTemplateOverlayUi);
        }
        FollowupControlEvent::TemplateCycledBackward => {
            state.auto_follow_state.cycle_template_kind_backward();
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto follow-up template: {}",
                state.auto_follow_state.template_label()
            );
            effects.push(FollowupControlEffect::SyncTemplateOverlayUi);
        }
    }

    FollowupControlReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::followup_template::{
        FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
    };

    #[test]
    fn draft_workspace_sync_updates_blank_draft_and_emits_ui_sync() {
        let draft = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result("builtin next-task", "follow up"),
        );

        let reduced = reduce_followup_controls(
            draft,
            FollowupControlEvent::DraftWorkspaceSynced {
                workspace_directory: "/tmp/alt".to_string(),
                template_load_result: sample_template_load_result("workspace review", "review"),
            },
        );

        assert_eq!(reduced.state.cwd, "/tmp/alt");
        assert_eq!(
            reduced.state.auto_follow_state.template_label(),
            "workspace review"
        );
        assert!(reduced.state.status_text.contains("draft workspace synced"));
        assert_eq!(
            reduced.effects,
            vec![FollowupControlEffect::SyncTemplateOverlayUi]
        );
    }

    #[test]
    fn workspace_sync_ignores_active_thread() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result("builtin next-task", "follow up"),
        );
        state.thread_id = "thread-1".to_string();

        let reduced = reduce_followup_controls(
            state,
            FollowupControlEvent::DraftWorkspaceSynced {
                workspace_directory: "/tmp/alt".to_string(),
                template_load_result: sample_template_load_result("workspace review", "review"),
            },
        );

        assert_eq!(reduced.state.cwd, "/tmp/root");
        assert_eq!(
            reduced.state.auto_follow_state.template_label(),
            "builtin next-task"
        );
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn toggling_auto_follow_clears_skip_and_updates_status() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result("builtin next-task", "follow up"),
        );
        state.record_auto_followup_skip(AutoFollowupSkipReason::Disabled);

        let reduced = reduce_followup_controls(state, FollowupControlEvent::AutoFollowToggled);

        assert!(!reduced.state.auto_follow_state.enabled);
        assert!(reduced.state.last_auto_followup_skip.is_none());
        assert_eq!(reduced.state.status_text, "auto follow-up off");
    }

    #[test]
    fn cycling_template_resets_overlay_ui_and_wraps_backward() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result_pair(),
        );
        state.auto_follow_state.template_state.selected_index = 0;

        let reduced = reduce_followup_controls(state, FollowupControlEvent::TemplateCycledBackward);

        assert_eq!(
            reduced.state.auto_follow_state.template_label(),
            "workspace review"
        );
        assert_eq!(
            reduced.effects,
            vec![FollowupControlEffect::SyncTemplateOverlayUi]
        );
    }

    fn sample_template_load_result(label: &str, body: &str) -> FollowupTemplateCatalogLoadResult {
        FollowupTemplateCatalogLoadResult {
            catalog: FollowupTemplateCatalog {
                items: vec![FollowupTemplateDefinition {
                    id: label.to_string(),
                    label: label.to_string(),
                    body: body.to_string(),
                    source: FollowupTemplateSource::Builtin,
                }],
            },
            warnings: Vec::new(),
        }
    }

    fn sample_template_load_result_pair() -> FollowupTemplateCatalogLoadResult {
        FollowupTemplateCatalogLoadResult {
            catalog: FollowupTemplateCatalog {
                items: vec![
                    FollowupTemplateDefinition {
                        id: "builtin-next-task".to_string(),
                        label: "builtin next-task".to_string(),
                        body: "follow up".to_string(),
                        source: FollowupTemplateSource::Builtin,
                    },
                    FollowupTemplateDefinition {
                        id: "workspace-review".to_string(),
                        label: "workspace review".to_string(),
                        body: "review".to_string(),
                        source: FollowupTemplateSource::WorkspaceFile {
                            path: "/tmp/root/.codex-exec-loop/followups/review.md".to_string(),
                        },
                    },
                ],
            },
            warnings: Vec::new(),
        }
    }
}
