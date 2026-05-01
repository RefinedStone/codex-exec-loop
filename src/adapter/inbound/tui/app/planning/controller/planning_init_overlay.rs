use super::*;

impl NativeTuiApp {
    pub(crate) fn handle_planning_init_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match self.planning_init_overlay_ui_state.step() {
            PlanningInitOverlayStep::ExistingWorkspace => match key.code {
                KeyCode::Enter if key.modifiers.is_empty() => {
                    self.close_shell_overlay();
                    self.show_queue_overlay();
                }
                KeyCode::Char('q') | KeyCode::Char('Q')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.close_shell_overlay();
                    self.show_queue_overlay();
                }
                KeyCode::Char('d') | KeyCode::Char('D')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.close_shell_overlay();
                    self.show_directions_maintenance_overlay();
                }
                _ => {}
            },
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
                    .return_from_detail_selection(),
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
            PlanningInitOverlayStep::SimpleReview => {
                match key.code {
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        self.promote_simple_mode_planning_draft()
                    }
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.planning_init_overlay_ui_state.open_detail_selection();
                        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: "planning detail authoring: choose how the advanced draft should open".to_string(),
                    });
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
                }
            }
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
}
