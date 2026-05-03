/*
 * Directions maintenance key routing is the inbound edge of the overlay.
 * The service-owned summary and DirectionsMaintenanceOverlayUiState decide what can be shown; this
 * controller turns shell key events into app actions such as opening an editor, entering the detail-doc
 * confirmation flow, or publishing a status message when the requested action is unsafe.
 */
use super::*;

impl NativeTuiApp {
    /*
     * Shell routing calls this while the directions maintenance overlay owns focus.
     * Returning true keeps every key inside the overlay context, including manual-editor keys that are
     * delegated to the shared draft editor instead of falling through to shell-wide shortcuts.
     */
    pub(crate) fn handle_directions_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match self.directions_maintenance_overlay_ui_state.step() {
            DirectionsMaintenanceOverlayStep::Overview => match key.code {
                /*
                 * Enter starts with the most common recovery path: the queue-idle prompt editor.
                 * The prompt is a directions supporting file, so it reuses the manual editor flow rather
                 * than creating a separate editing surface.
                 */
                KeyCode::Enter if key.modifiers.is_empty() => self.open_queue_idle_prompt_editor(),
                /*
                 * Detail-doc generation only makes sense after the DB direction authority parses cleanly.
                 * With a parse error, the app cannot reliably identify targets or output paths, so the key
                 * reports the recovery requirement through the shared status channel.
                 */
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
                        /*
                         * An empty actionable list means the service snapshot has no missing or broken
                         * detail-doc mappings. The controller keeps the operator on overview and explains
                         * the no-op instead of opening an empty selection step.
                         */
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
                /*
                 * `p` is the explicit queue-idle prompt shortcut. Prompt generation and validation still
                 * depend on the direction authority, so parse errors block the editor with the same status
                 * feedback used for detail-doc generation.
                 */
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
                /*
                 * Reload replaces the overlay state with a fresh workspace summary from the service.
                 * present_directions_maintenance_overview already owns loading, visibility, and status
                 * dispatch, so the key handler reuses that entrypoint.
                 */
                KeyCode::Char('r') if key.modifiers.is_empty() => self
                    .present_directions_maintenance_overview(
                        "reloaded directions maintenance".to_string(),
                        true,
                    ),
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::DetailDocSelection => match key.code {
                // Back/left leaves selection without creating a pending generation target.
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .return_to_overview(),
                // Movement is clamped by UI state against the filtered actionable detail-doc list.
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_missing_detail_doc_selection(-1),
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_missing_detail_doc_selection(1),
                /*
                 * Enter snapshots the selected direction and opens confirmation instead of starting file
                 * generation immediately. The later Yes action runs against that captured id/title pair.
                 */
                KeyCode::Enter if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .open_detail_doc_confirm(),
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::DetailDocConfirm => match key.code {
                // Back/left returns to the selection list so the operator can pick a different direction.
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .open_detail_doc_selection(),
                /*
                 * The confirm choice is a two-position Yes/No control. Numeric keys mirror the displayed
                 * option order, while j/k keep the same keyboard-only navigation model as selection lists.
                 */
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
                            /*
                             * Execution uses the stable direction id, not the display title.
                             * A missing pending snapshot means the confirm state is incomplete, so the
                             * controller declines to start any editor/service work.
                             */
                            let direction_id = self
                                .directions_maintenance_overlay_ui_state
                                .pending_detail_doc_creation()
                                .map(|pending| pending.direction_id().to_string());
                            if let Some(direction_id) = direction_id {
                                self.open_directions_detail_doc_editor(&direction_id);
                            }
                        }
                        DetailDocConfirmChoice::No => {
                            /*
                             * No is an explicit cancellation path. It returns to overview and emits a status
                             * message confirming that the directions files were not changed.
                             */
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
                /*
                 * ManualEditor nests the shared draft editor inside the directions overlay.
                 * Close-confirmation keys run first to protect dirty or invalid drafts; all other editing
                 * keys use the generic draft editor handler with directions-specific save/promote hooks.
                 */
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
