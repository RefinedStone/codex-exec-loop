use crossterm::event::{self, KeyCode, KeyModifiers};

use super::super::*;

impl NativeTuiApp {
    pub(crate) fn handle_directions_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match self.directions_maintenance_overlay_ui_state.step() {
            DirectionsMaintenanceOverlayStep::Overview => match key.code {
                KeyCode::Enter if key.modifiers.is_empty() => self.open_directions_manual_editor(),
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
                                    "fix directions.toml parse errors before generating detail docs"
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
                        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                            status_text:
                                "fix directions.toml parse errors before editing queue-idle prompt"
                                    .to_string(),
                        });
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
            DirectionsMaintenanceOverlayStep::DetailDocConfirm => {
                match key.code {
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
                                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "detail doc creation skipped; directions remain unchanged"
                                        .to_string(),
                            });
                            }
                        }
                    }
                    _ => {}
                }
            }
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

    pub(crate) fn handle_planning_init_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match self.planning_init_overlay_ui_state.step() {
            PlanningInitOverlayStep::BootstrapObjective => match key.code {
                KeyCode::Enter if key.modifiers.is_empty() => {
                    self.submit_planning_bootstrap_objective()
                }
                KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => self
                    .planning_init_overlay_ui_state
                    .insert_bootstrap_objective_newline(),
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => self
                    .planning_init_overlay_ui_state
                    .clear_bootstrap_objective(),
                KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => self
                    .planning_init_overlay_ui_state
                    .delete_previous_bootstrap_objective_word(),
                KeyCode::Backspace if key.modifiers.is_empty() => self
                    .planning_init_overlay_ui_state
                    .pop_bootstrap_objective_character(),
                KeyCode::Char(character)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .push_bootstrap_objective_character(character)
                }
                _ => {}
            },
            PlanningInitOverlayStep::ExistingWorkspace => {
                let workspace_directory = self.planning_workspace_directory();
                let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
                let entry_mode = self.planning_init_overlay_ui_state.entry_mode();
                match key.code {
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        if snapshot.plan_enabled() {
                            match entry_mode {
                                PlanningInitEntryMode::CommandCenter => {
                                    self.close_shell_overlay();
                                    self.show_queue_overlay();
                                }
                                PlanningInitEntryMode::WorkflowGate
                                | PlanningInitEntryMode::ResumeGate => self.close_shell_overlay(),
                            }
                        } else {
                            self.turn_plan_on();
                        }
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        if snapshot.plan_enabled() {
                            self.close_shell_overlay();
                            self.show_queue_overlay();
                        } else {
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text: "Plan off - turn Plan on before opening queue"
                                        .to_string(),
                                },
                            );
                        }
                    }
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        if snapshot.plan_enabled() {
                            self.close_shell_overlay();
                            self.show_directions_maintenance_overlay();
                        } else {
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text: "Plan off - turn Plan on in this menu first"
                                        .to_string(),
                                },
                            );
                        }
                    }
                    KeyCode::Char('o') | KeyCode::Char('O')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        if snapshot.plan_enabled() {
                            self.turn_plan_off();
                        } else {
                            self.turn_plan_on();
                        }
                    }
                    _ => {}
                }
            }
            PlanningInitOverlayStep::ModeSelection => match key.code {
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_mode_selection(-1)
                }
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_mode_selection(1)
                }
                KeyCode::Char('a') | KeyCode::Char('A')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_mode(PlanningInitModeSelection::Simple)
                }
                KeyCode::Char('b') | KeyCode::Char('B')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_mode(PlanningInitModeSelection::Detail)
                }
                KeyCode::Enter if key.modifiers.is_empty() => {
                    match self.planning_init_overlay_ui_state.selected_mode() {
                        PlanningInitModeSelection::Simple => {
                            self.stage_simple_mode_planning_init_draft()
                        }
                        PlanningInitModeSelection::Detail => {
                            self.planning_init_overlay_ui_state.open_detail_selection()
                        }
                    }
                }
                _ => {}
            },
            PlanningInitOverlayStep::DetailSelection => match key.code {
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .planning_init_overlay_ui_state
                    .return_to_mode_selection(),
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .planning_init_overlay_ui_state
                    .move_detail_selection(-1),
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_detail_selection(1)
                }
                KeyCode::Char('a') | KeyCode::Char('A')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_detail(PlanningInitDetailSelection::Manual)
                }
                KeyCode::Char('b') | KeyCode::Char('B')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_detail(PlanningInitDetailSelection::LlmAssisted)
                }
                KeyCode::Enter if key.modifiers.is_empty() => {
                    match self.planning_init_overlay_ui_state.selected_detail() {
                        PlanningInitDetailSelection::Manual => self.open_planning_manual_editor(),
                        PlanningInitDetailSelection::LlmAssisted => {
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text:
                                        "planning llm-assisted detail mode is not supported yet"
                                            .to_string(),
                                },
                            );
                        }
                    }
                }
                _ => {}
            },
            PlanningInitOverlayStep::SimpleReview => match key.code {
                KeyCode::Enter if key.modifiers.is_empty() => {
                    self.promote_simple_mode_planning_draft()
                }
                KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                    self.start_max_auto_turns_edit()
                }
                KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => {
                    self.open_simple_mode_planning_editor()
                }
                KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                    self.promote_simple_mode_planning_draft()
                }
                _ => {}
            },
            PlanningInitOverlayStep::ManualEditor => {
                if self.handle_planning_manual_editor_close_confirmation_key(key) {
                    return true;
                }
                self.handle_draft_editor_key(
                    key,
                    Self::save_planning_manual_editor,
                    Self::promote_planning_manual_editor,
                );
            }
        }

        true
    }

    fn handle_draft_editor_key(
        &mut self,
        key: event::KeyEvent,
        save: fn(&mut Self),
        promote: fn(&mut Self),
    ) {
        match key.code {
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.planning_draft_editor_ui_state.move_file_selection(1)
            }
            KeyCode::BackTab => self.planning_draft_editor_ui_state.move_file_selection(-1),
            KeyCode::Left if key.modifiers.is_empty() => {
                self.planning_draft_editor_ui_state.move_cursor_left()
            }
            KeyCode::Right if key.modifiers.is_empty() => {
                self.planning_draft_editor_ui_state.move_cursor_right()
            }
            KeyCode::Up if key.modifiers.is_empty() => {
                self.planning_draft_editor_ui_state.move_cursor_up()
            }
            KeyCode::Down if key.modifiers.is_empty() => {
                self.planning_draft_editor_ui_state.move_cursor_down()
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.planning_draft_editor_ui_state.insert_newline()
            }
            KeyCode::Backspace if key.modifiers.is_empty() => {
                self.planning_draft_editor_ui_state.backspace()
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                self.planning_draft_editor_ui_state.delete_previous_word()
            }
            KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => save(self),
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => promote(self),
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.planning_draft_editor_ui_state
                    .insert_character(character)
            }
            _ => {}
        }
    }
}
