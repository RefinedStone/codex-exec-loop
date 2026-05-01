use super::*;

impl NativeTuiApp {
    pub(crate) fn handle_directions_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match self.directions_maintenance_overlay_ui_state.step() {
            DirectionsMaintenanceOverlayStep::Overview => match key.code {
                KeyCode::Enter if key.modifiers.is_empty() => self.open_queue_idle_prompt_editor(),
                KeyCode::Char('d') if key.modifiers.is_empty() => {
                    if self
                        .directions_maintenance_overlay_ui_state
                        .summary()
                        .and_then(|summary| summary.parse_error.as_deref())
                        .is_some()
                    {
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "fix DB direction authority errors before generating detail docs"
                                        .to_string(),
                            },
                        );
                    } else if self
                        .directions_maintenance_overlay_ui_state
                        .actionable_detail_doc_directions()
                        .is_empty()
                    {
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "every direction already has a healthy detail doc mapping"
                                        .to_string(),
                            },
                        );
                    } else {
                        self.directions_maintenance_overlay_ui_state
                            .open_detail_doc_selection();
                    }
                }
                KeyCode::Char('p') if key.modifiers.is_empty() => {
                    if self
                        .directions_maintenance_overlay_ui_state
                        .summary()
                        .and_then(|summary| summary.parse_error.as_deref())
                        .is_some()
                    {
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "fix DB direction authority errors before editing queue-idle prompt"
                                        .to_string(),
                            },
                        );
                    } else {
                        self.open_queue_idle_prompt_editor();
                    }
                }
                KeyCode::Char('r') if key.modifiers.is_empty() => self
                    .present_directions_maintenance_overview(
                        "reloaded directions maintenance".to_string(),
                        true,
                    ),
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::DetailDocSelection => match key.code {
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .return_to_overview(),
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_missing_detail_doc_selection(-1),
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_missing_detail_doc_selection(1),
                KeyCode::Enter if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .open_detail_doc_confirm(),
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::DetailDocConfirm => match key.code {
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .open_detail_doc_selection(),
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(-1),
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(1),
                KeyCode::Char('1') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(-1),
                KeyCode::Char('2') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(1),
                KeyCode::Enter if key.modifiers.is_empty() => {
                    match self
                        .directions_maintenance_overlay_ui_state
                        .detail_doc_confirm_choice()
                    {
                        DetailDocConfirmChoice::Yes => {
                            let direction_id = self
                                .directions_maintenance_overlay_ui_state
                                .pending_detail_doc_creation()
                                .map(|pending| pending.direction_id().to_string());
                            if let Some(direction_id) = direction_id {
                                self.open_directions_detail_doc_editor(&direction_id);
                            }
                        }
                        DetailDocConfirmChoice::No => {
                            self.directions_maintenance_overlay_ui_state
                                .return_to_overview();
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text:
                                        "detail doc creation skipped; directions remain unchanged"
                                            .to_string(),
                                },
                            );
                        }
                    }
                }
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::ManualEditor => {
                if self.handle_directions_manual_editor_close_confirmation_key(key) {
                    return true;
                }
                self.handle_draft_editor_key(
                    key,
                    Self::save_directions_manual_editor,
                    Self::promote_directions_manual_editor,
                );
            }
        }

        true
    }
}
