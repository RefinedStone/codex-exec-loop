/*
 * Planning-init overlay input is a step-aware key router. The UI state owns
 * wizard selection and breadcrumbs, while this controller translates terminal
 * keys into either local UI-state moves or app-level planning actions such as
 * staging, promoting, opening maintenance surfaces, and entering draft editor
 * mode.
 */
use super::*;

impl NativeTuiApp {
    pub(crate) fn handle_planning_init_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        /*
         * The same key means different things in each wizard step, so route by
         * step before route by key. Returning true at the end tells the outer
         * shell router that the planning overlay owns the key stream while it
         * is visible.
         */
        match self.planning_init_overlay_ui_state.step() {
            PlanningInitOverlayStep::ExistingWorkspace => match key.code {
                /*
                 * Existing workspace mode is an inspection/redirect screen:
                 * Enter and Q leave setup and move to queue inspection because
                 * there is already a planning workspace to operate.
                 */
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
                /*
                 * Direction catalog maintenance remains available even when
                 * setup is skipped; it is the supported path for editing
                 * workspace instructions without reinitializing planning.
                 */
                KeyCode::Char('d') | KeyCode::Char('D')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.close_shell_overlay();
                    self.show_directions_maintenance_overlay();
                }
                // Other keys are consumed to keep global shortcuts from firing while the overlay is focused.
                _ => {}
            },
            PlanningInitOverlayStep::ModeSelection => match key.code {
                // Vim/arrow navigation only moves the cursor; it does not stage a draft.
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_mode_selection(-1)
                }
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_mode_selection(1)
                }
                // Letter shortcuts set the same selection cursor used by Enter, preserving one execution path.
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
                /*
                 * Enter is the first point where mode selection becomes an
                 * operation. Simple mode immediately stages a generated draft;
                 * detail mode opens another selection step because it needs a
                 * concrete authoring backend.
                 */
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
                // Unknown keys are consumed inside this focused wizard step without changing state.
                _ => {}
            },
            PlanningInitOverlayStep::DetailSelection => match key.code {
                /*
                 * Back navigation respects the UI state's breadcrumb logic:
                 * from first-run detail selection it returns to mode choice,
                 * while from simple review it returns to the staged review.
                 */
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .planning_init_overlay_ui_state
                    .return_from_detail_selection(),
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .planning_init_overlay_ui_state
                    .move_detail_selection(-1),
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_detail_selection(1)
                }
                // Letter shortcuts select an authoring backend; they do not open the editor until Enter.
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
                /*
                 * Manual detail opens the planning draft editor through the
                 * service/controller path. LLM-assisted mode is visible in the
                 * selector but still disabled, so it reports status without
                 * leaving the overlay or discarding the selection context.
                 */
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
                /*
                 * SimpleReview is a post-stage confirmation surface. It can
                 * promote the generated draft, branch into detail/manual
                 * authoring, or hand focus to the max-auto-turns inline editor.
                 */
                match key.code {
                    // Enter follows the primary review action: promote the staged simple draft if validation allows it.
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        self.promote_simple_mode_planning_draft()
                    }
                    /*
                     * Detail authoring from simple review keeps the staged
                     * simple draft as context but reopens the detail selector
                     * so the operator can choose a richer authoring path.
                     */
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.planning_init_overlay_ui_state.open_detail_selection();
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "planning detail authoring: choose how the advanced draft should open"
                                        .to_string(),
                            },
                        );
                    }
                    // Ctrl-L transfers key ownership to the follow-up max-auto-turns inline editor.
                    KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                        self.start_max_auto_turns_edit()
                    }
                    // Ctrl-E opens the generated simple draft in the shared manual editor core for direct edits.
                    KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => {
                        self.open_simple_mode_planning_editor()
                    }
                    // Ctrl-P is the explicit promotion shortcut, matching Enter's primary action.
                    KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                        self.promote_simple_mode_planning_draft()
                    }
                    _ => {}
                }
            }
            PlanningInitOverlayStep::ManualEditor => {
                /*
                 * Close confirmation owns keys before normal editing. Without
                 * this priority, Esc/Enter could mutate the draft or promote it
                 * while the UI is asking the operator to confirm a risky close.
                 */
                if self.handle_planning_manual_editor_close_confirmation_key(key) {
                    return true;
                }
                /*
                 * The draft editor core is shared by planning-init manual
                 * editing and directions maintenance. Injecting save/promote
                 * callbacks here keeps text navigation/editing reusable while
                 * preserving planning-init-specific service actions.
                 */
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
