/*
 * Draft editor controller glue connects three surfaces that deliberately share
 * one text-buffer UI state: planning-init manual editor, simple-draft editor,
 * and directions maintenance editors. The pure editor state owns cursor,
 * dirty flags, close guards, and editable file bodies; this module submits the
 * target-specific Core command and delegates correlated overlay settlement to
 * the planning controller.
 */
use super::*;

impl NativeTuiApp {
    pub(super) fn open_planning_manual_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let outcome =
            self.reduce_core_client_event(CoreInput::Command(AppCommand::StagePlanningEditor {
                workspace_directory,
                target: PlanningEditorStageTarget::PlanningManual,
            }));
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn open_directions_detail_doc_editor(&mut self, direction_id: &str) {
        let workspace_directory = self.planning_workspace_directory();
        let outcome =
            self.reduce_core_client_event(CoreInput::Command(AppCommand::StagePlanningEditor {
                workspace_directory,
                target: PlanningEditorStageTarget::DirectionDetail {
                    direction_id: direction_id.to_string(),
                },
            }));
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn open_queue_idle_prompt_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let outcome =
            self.reduce_core_client_event(CoreInput::Command(AppCommand::StagePlanningEditor {
                workspace_directory,
                target: PlanningEditorStageTarget::QueueIdlePrompt,
            }));
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn save_planning_manual_editor(&mut self) {
        self.dispatch_planning_editor_mutation(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Planning,
        );
    }

    pub(super) fn save_directions_manual_editor(&mut self) {
        if self
            .planning
            .planning_draft_editor_ui_state
            .session_identity()
            .is_none()
        {
            return;
        }
        if !self.directions_editor_workspace_is_current() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "directions editor workspace changed; save blocked / close and reopen maintenance in the current workspace".to_string(),
            });
            return;
        }
        self.dispatch_planning_editor_mutation(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Directions,
        );
    }

    pub(super) fn promote_planning_manual_editor(&mut self) {
        self.dispatch_planning_editor_mutation(
            PlanningEditorMutationAction::Promote,
            PlanningEditorMutationTarget::Planning,
        );
    }

    pub(super) fn promote_directions_manual_editor(&mut self) {
        if self
            .planning
            .planning_draft_editor_ui_state
            .session_identity()
            .is_none()
        {
            return;
        }
        if !self.directions_editor_workspace_is_current() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "directions editor workspace changed; promote blocked / close and reopen maintenance in the current workspace".to_string(),
            });
            return;
        }
        self.dispatch_planning_editor_mutation(
            PlanningEditorMutationAction::Promote,
            PlanningEditorMutationTarget::Directions,
        );
    }

    fn dispatch_planning_editor_mutation(
        &mut self,
        action: PlanningEditorMutationAction,
        target: PlanningEditorMutationTarget,
    ) {
        let Some(source_session) = self
            .planning
            .planning_draft_editor_ui_state
            .session_identity()
            .cloned()
        else {
            return;
        };
        let workspace_directory = self.planning_workspace_directory();
        if source_session.workspace_directory != workspace_directory {
            let target = match target {
                PlanningEditorMutationTarget::Planning => "planning",
                PlanningEditorMutationTarget::Directions => "directions",
            };
            let action = match action {
                PlanningEditorMutationAction::Save => "save",
                PlanningEditorMutationAction::Promote => "promote",
            };
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: format!(
                    "{target} editor workspace changed; {action} blocked / close and reopen the editor in the current workspace"
                ),
            });
            return;
        }
        let Some(buffer_revision) = self
            .planning
            .planning_draft_editor_ui_state
            .buffer_revision()
        else {
            return;
        };
        let source_planning_revision = self
            .planning
            .planning_draft_editor_ui_state
            .source_planning_revision();
        let draft_name = source_session.draft_name.clone();
        self.planning
            .planning_draft_editor_ui_state
            .clear_close_confirmation();
        let mut identity = PlanningEditorMutationIdentity::new(
            action,
            target,
            draft_name,
            source_session,
            buffer_revision,
        );
        if let Some(source_planning_revision) = source_planning_revision {
            identity = identity.with_source_planning_revision(source_planning_revision);
        }
        let request = PlanningEditorMutationRequest {
            identity,
            editable_files: self
                .planning
                .planning_draft_editor_ui_state
                .collect_editable_file_snapshots(),
        };
        let outcome =
            self.reduce_core_client_event(CoreInput::Command(AppCommand::MutatePlanningEditor {
                workspace_directory,
                request: Box::new(request),
            }));
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(in crate::adapter::inbound::tui::app) fn request_close_planning_manual_editor(&mut self) {
        /*
         * Close requests delegate risk calculation to the editor UI state,
         * which knows about dirty buffers and invalid staged validation. This
         * controller only chooses the planning-init close destination and copy.
         */
        match self.planning.planning_draft_editor_ui_state.request_close() {
            PlanningDraftEditorCloseRequest::CloseImmediately => self.close_shell_overlay(),
            PlanningDraftEditorCloseRequest::ConfirmationRequired(risk) => {
                /*
                 * First close attempt only arms confirmation and reports the risk. The UI
                 * state keeps the pending risk so the next Enter can close without
                 * recalculating after unrelated rendering or cursor movement.
                 */
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: planning_manual_editor_close_warning_status(risk),
                });
            }
            PlanningDraftEditorCloseRequest::Confirmed(risk) => {
                self.close_planning_manual_editor_after_confirmation(risk);
            }
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn request_close_directions_manual_editor(&mut self) {
        /*
         * Directions editor close uses the same risk state, but the safe close
         * destination is the directions maintenance overview rather than the
         * main shell.
         */
        match self.planning.planning_draft_editor_ui_state.request_close() {
            PlanningDraftEditorCloseRequest::CloseImmediately => self
                .close_directions_manual_editor_without_prompt(
                    "directions editor closed".to_string(),
                ),
            PlanningDraftEditorCloseRequest::ConfirmationRequired(risk) => {
                /*
                 * Directions close confirmation uses directions-specific copy because the
                 * consequence is returning to maintenance, not just hiding planning init.
                 */
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: directions_manual_editor_close_warning_status(risk),
                });
            }
            PlanningDraftEditorCloseRequest::Confirmed(risk) => {
                self.close_directions_manual_editor_after_confirmation(risk);
            }
        }
    }

    fn close_planning_manual_editor_after_confirmation(
        &mut self,
        risk: PlanningDraftEditorCloseRisk,
    ) {
        // Confirmed planning-init close leaves the overlay and records why the risky close was accepted.
        self.close_shell_overlay();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: planning_manual_editor_closed_status(risk),
        });
    }

    fn close_directions_manual_editor_after_confirmation(
        &mut self,
        risk: PlanningDraftEditorCloseRisk,
    ) {
        /*
         * Confirmed directions close should rebuild the maintenance overview,
         * because that surface owns the catalog/status context the editor was
         * launched from.
         */
        self.close_directions_manual_editor_without_prompt(directions_manual_editor_closed_status(
            risk,
        ));
    }

    fn close_directions_manual_editor_without_prompt(&mut self, status_text: String) {
        /*
         * All non-prompt directions exits flow through the overview presenter. This
         * centralizes the reset/reload behavior that makes the file list and status
         * lines match the just-saved or just-discarded draft state.
         */
        self.start_directions_maintenance_overview_load(Some(status_text));
    }

    pub(super) fn handle_planning_manual_editor_close_confirmation_key(
        &mut self,
        key: event::KeyEvent,
    ) -> bool {
        /*
         * Planning-init key routing gives this handler priority over the
         * shared editor input handler. While confirmation is pending, Enter
         * confirms, N cancels, and any unrelated key clears the prompt then
         * falls through for normal handling.
         */
        if !self
            .planning
            .planning_draft_editor_ui_state
            .is_close_confirmation_pending()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => {
                let Some(risk) = self
                    .planning
                    .planning_draft_editor_ui_state
                    .pending_close_risk()
                else {
                    return false;
                };
                self.close_planning_manual_editor_after_confirmation(risk);
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.planning
                    .planning_draft_editor_ui_state
                    .clear_close_confirmation();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "planning draft editor close canceled; keep editing".to_string(),
                });
                true
            }
            _ => {
                /*
                 * Any other key cancels the confirmation and falls through. This lets
                 * normal editor navigation or typing resume immediately without a stale
                 * modal close prompt intercepting later keys.
                 */
                self.planning
                    .planning_draft_editor_ui_state
                    .clear_close_confirmation();
                false
            }
        }
    }

    pub(super) fn handle_directions_manual_editor_close_confirmation_key(
        &mut self,
        key: event::KeyEvent,
    ) -> bool {
        /*
         * Directions maintenance has the same confirmation keyboard contract
         * as planning-init, but its confirm/cancel copy and close target are
         * directions-specific.
         */
        if !self
            .planning
            .planning_draft_editor_ui_state
            .is_close_confirmation_pending()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => {
                let Some(risk) = self
                    .planning
                    .planning_draft_editor_ui_state
                    .pending_close_risk()
                else {
                    return false;
                };
                self.close_directions_manual_editor_after_confirmation(risk);
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.planning
                    .planning_draft_editor_ui_state
                    .clear_close_confirmation();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "directions editor close canceled; keep editing".to_string(),
                });
                true
            }
            _ => {
                /*
                 * Directions confirmation follows the same fallthrough rule as planning
                 * init so shared editor muscle memory stays consistent across both entry
                 * points.
                 */
                self.planning
                    .planning_draft_editor_ui_state
                    .clear_close_confirmation();
                false
            }
        }
    }
}
