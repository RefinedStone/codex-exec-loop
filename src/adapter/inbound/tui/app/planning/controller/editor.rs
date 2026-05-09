/*
 * Draft editor controller glue connects three surfaces that deliberately share
 * one text-buffer UI state: planning-init manual editor, simple-draft editor,
 * and directions maintenance editors. The pure editor state owns cursor,
 * dirty flags, close guards, and editable file bodies; this module chooses the
 * correct planning workspace service call and the correct overlay/status
 * transition for each caller.
 */
use super::*;

impl NativeTuiApp {
    pub(super) fn open_planning_manual_editor(&mut self) {
        /*
         * Detail-mode planning authoring starts by asking the workspace
         * service to stage a manual draft editor session. The shared opener
         * then installs the returned buffers into PlanningDraftEditorUiState
         * and moves planning-init into ManualEditor mode.
         */
        let workspace_directory = self.planning_workspace_directory();
        self.open_guided_planning_editor_session(
            self.application
                .planning()
                .workspace()
                .stage_manual_editor_session(&workspace_directory),
            "planning draft editor ready",
            PlanningInitModeSelection::Detail,
        );
    }

    pub(super) fn open_directions_detail_doc_editor(&mut self, direction_id: &str) {
        /*
         * Directions detail docs use the same editor buffer mechanics, but
         * their service staging path is keyed by direction id and their overlay
         * returns to directions maintenance rather than planning init.
         */
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.application
                .planning()
                .workspace()
                .stage_detail_doc_editor_session(&workspace_directory, direction_id),
            "directions detail doc editor ready",
        );
    }

    pub(super) fn open_queue_idle_prompt_editor(&mut self) {
        /*
         * Queue-idle prompt editing is modeled as a directions-maintenance
         * draft because it changes planning authority text, not the active
         * task queue. It still enters the shared manual editor UI.
         */
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.application
                .planning()
                .workspace()
                .stage_queue_idle_prompt_editor_session(&workspace_directory),
            "queue-idle prompt editor ready",
        );
    }

    pub(super) fn save_planning_manual_editor(&mut self) {
        /*
         * Saving planning-init editor content writes the current UI buffers
         * back to the staged draft and refreshes validation. Promotion remains
         * a separate Ctrl+P action so invalid drafts can stay open for repair.
         */
        let Some(draft_name) = self
            .planning_draft_editor_ui_state
            .draft_name()
            .map(str::to_string)
        else {
            return;
        };
        /*
         * The draft name is copied out before collecting buffers because save mutates
         * editor state below. The service call must target the session that was open
         * when Ctrl+S was pressed, not any later overlay state.
         */
        self.planning_draft_editor_ui_state
            .clear_close_confirmation();
        let workspace_directory = self.planning_workspace_directory();
        /*
         * collect_editable_files converts UI buffers back into service records. This
         * controller does not inspect file bodies; validation and path ownership stay
         * inside planning workspace services.
         */
        let editable_files = self.planning_draft_editor_ui_state.collect_editable_files();
        let status_text = match self
            .application
            .planning()
            .workspace()
            .save_draft_editor_files(&workspace_directory, &draft_name, &editable_files)
        {
            Ok(result) => {
                /*
                 * The service returns the canonical validation report for the
                 * saved staged files. Applying it also clears dirty flags so
                 * close-risk and next-action copy stop treating the buffer as
                 * unsaved.
                 */
                let validation_ok = result.validation_report.is_valid();
                self.planning_draft_editor_ui_state
                    .apply_save_result(result.validation_report.clone());
                format!(
                    "planning draft saved / draft: {} / validation: {} / next: {}",
                    result.draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    },
                    if validation_ok {
                        "press Ctrl+P to promote into accepted planning state"
                    } else {
                        "fix validation issues before promoting"
                    },
                )
            }
            Err(error) => format!("planning draft save failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn save_directions_manual_editor(&mut self) {
        /*
         * Directions editor save follows the same staged-draft persistence
         * contract as planning-init save. Only the operator-facing copy differs
         * because a successful promotion returns to directions maintenance.
         */
        let Some(draft_name) = self
            .planning_draft_editor_ui_state
            .draft_name()
            .map(str::to_string)
        else {
            return;
        };
        /*
         * Directions save uses the same staged draft namespace as planning-init save,
         * so the active draft name is captured before status copy or validation state
         * can be replaced by the save result.
         */
        self.planning_draft_editor_ui_state
            .clear_close_confirmation();
        let workspace_directory = self.planning_workspace_directory();
        let editable_files = self.planning_draft_editor_ui_state.collect_editable_files();
        let status_text = match self
            .application
            .planning()
            .workspace()
            .save_draft_editor_files(&workspace_directory, &draft_name, &editable_files)
        {
            Ok(result) => {
                let validation_ok = result.validation_report.is_valid();
                self.planning_draft_editor_ui_state
                    .apply_save_result(result.validation_report.clone());
                format!(
                    "directions draft saved / draft: {} / validation: {} / next: {}",
                    result.draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    },
                    if validation_ok {
                        "press Ctrl+P to promote into accepted planning state"
                    } else {
                        "fix validation issues before promoting"
                    },
                )
            }
            Err(error) => format!("directions draft save failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn promote_planning_manual_editor(&mut self) {
        /*
         * Promotion writes current editor buffers through the workspace service
         * and, on success, replaces accepted planning authority files. The
         * runtime snapshot is refreshed regardless of success so footer/queue
         * status reflects the latest validation attempt.
         */
        let Some(draft_name) = self
            .planning_draft_editor_ui_state
            .draft_name()
            .map(str::to_string)
        else {
            return;
        };
        self.planning_draft_editor_ui_state
            .clear_close_confirmation();
        let workspace_directory = self.planning_workspace_directory();
        let editable_files = self.planning_draft_editor_ui_state.collect_editable_files();
        let promote_result = self
            .application
            .planning()
            .workspace()
            .promote_draft_editor_files(&workspace_directory, &draft_name, &editable_files);
        /*
         * Runtime snapshot refresh happens even on blocked promotion. A validation
         * failure may still update editor validation state or planning status copy, and
         * the inline footer should reflect that latest attempt immediately.
         */
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        let status_text = match promote_result {
            Ok(result) => {
                let validation_ok = result.validation_report.is_valid();
                self.planning_draft_editor_ui_state
                    .apply_save_result(result.validation_report.clone());
                if result.promoted_file_count == 0 {
                    /*
                     * Zero promoted files is a service-level "not accepted" outcome, not
                     * a controller exception. Keep the editor open with fresh validation
                     * so the operator can repair the same buffers.
                     */
                    format!(
                        "planning draft promote blocked / draft: {} / validation: {} / next: fix validation issues or keep editing",
                        result.draft_name,
                        if validation_ok {
                            "ok"
                        } else {
                            "needs attention"
                        }
                    )
                } else {
                    self.close_shell_overlay();
                    format!(
                        "planning draft promoted / draft: {} / files: {} / planning context refreshed",
                        result.draft_name, result.promoted_file_count
                    )
                }
            }
            Err(error) => format!("planning draft promote failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn promote_directions_manual_editor(&mut self) {
        /*
         * Directions promotion shares the same staged-draft promotion service,
         * but a successful result should reopen the directions maintenance
         * overview so the operator sees the refreshed authority catalog.
         */
        let Some(draft_name) = self
            .planning_draft_editor_ui_state
            .draft_name()
            .map(str::to_string)
        else {
            return;
        };
        self.planning_draft_editor_ui_state
            .clear_close_confirmation();
        let workspace_directory = self.planning_workspace_directory();
        let editable_files = self.planning_draft_editor_ui_state.collect_editable_files();
        let promote_result = self
            .application
            .planning()
            .workspace()
            .promote_draft_editor_files(&workspace_directory, &draft_name, &editable_files);
        /*
         * Directions promotion also refreshes the ready conversation snapshot because
         * changing direction detail docs or queue-idle prompt can alter queue/runtime
         * guidance shown outside the directions overlay.
         */
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        let status_text = match promote_result {
            Ok(result) => {
                let validation_ok = result.validation_report.is_valid();
                self.planning_draft_editor_ui_state
                    .apply_save_result(result.validation_report.clone());
                if result.promoted_file_count == 0 {
                    format!(
                        "directions draft promote blocked / draft: {} / validation: {} / next: fix validation issues or keep editing",
                        result.draft_name,
                        if validation_ok {
                            "ok"
                        } else {
                            "needs attention"
                        }
                    )
                } else {
                    /*
                     * A successful directions promotion returns to the maintenance overview
                     * instead of closing to the shell. That overview is the user's context
                     * for the direction catalog they just edited.
                     */
                    self.present_directions_maintenance_overview(
                        format!(
                            "directions draft promoted / draft: {} / files: {} / planning context refreshed",
                            result.draft_name, result.promoted_file_count
                        ),
                        true,
                    );
                    return;
                }
            }
            Err(error) => format!("directions draft promote failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(in crate::adapter::inbound::tui::app) fn request_close_planning_manual_editor(&mut self) {
        /*
         * Close requests delegate risk calculation to the editor UI state,
         * which knows about dirty buffers and invalid staged validation. This
         * controller only chooses the planning-init close destination and copy.
         */
        match self.planning_draft_editor_ui_state.request_close() {
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
        match self.planning_draft_editor_ui_state.request_close() {
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
        self.present_directions_maintenance_overview(status_text, true);
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
            .planning_draft_editor_ui_state
            .is_close_confirmation_pending()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => {
                let Some(risk) = self.planning_draft_editor_ui_state.pending_close_risk() else {
                    return false;
                };
                self.close_planning_manual_editor_after_confirmation(risk);
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.planning_draft_editor_ui_state
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
                self.planning_draft_editor_ui_state
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
            .planning_draft_editor_ui_state
            .is_close_confirmation_pending()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => {
                let Some(risk) = self.planning_draft_editor_ui_state.pending_close_risk() else {
                    return false;
                };
                self.close_directions_manual_editor_after_confirmation(risk);
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.planning_draft_editor_ui_state
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
                self.planning_draft_editor_ui_state
                    .clear_close_confirmation();
                false
            }
        }
    }
}
