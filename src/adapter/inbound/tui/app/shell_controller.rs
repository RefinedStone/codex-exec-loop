use super::planning_draft_editor_ui::{
    PlanningDraftEditorCloseRequest, PlanningDraftEditorCloseRisk,
};
use super::*;
use crate::application::service::planning::PlanningDraftEditorSession;

type PlanningEditorSessionResult = anyhow::Result<PlanningDraftEditorSession>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellActionAvailability {
    Ready,
    Pending,
    Blocked,
}

impl ShellActionAvailability {
    pub(super) fn allows_actions(self) -> bool {
        self == Self::Ready
    }

    pub(super) fn status_text(self) -> &'static str {
        match self {
            Self::Ready => "startup ready",
            Self::Pending => "startup checks still running",
            Self::Blocked => "startup diagnostics need attention",
        }
    }
}

impl NativeTuiApp {
    pub(super) fn can_open_session_list(&self) -> bool {
        matches!(
            &self.startup_state,
            StartupState::Ready(diagnostics) if diagnostics.can_continue()
        )
    }

    pub(super) fn shell_action_availability(&self) -> ShellActionAvailability {
        match &self.startup_state {
            StartupState::Ready(diagnostics) if diagnostics.can_continue() => {
                ShellActionAvailability::Ready
            }
            StartupState::Idle | StartupState::Loading => ShellActionAvailability::Pending,
            StartupState::Ready(_) | StartupState::Failed(_) => ShellActionAvailability::Blocked,
        }
    }

    pub(super) fn submission_blocked_status(&self, prompt_origin: PromptOrigin) -> String {
        match (prompt_origin, self.shell_action_availability()) {
            (_, ShellActionAvailability::Ready) => "ready".to_string(),
            (PromptOrigin::Manual, state) => {
                format!("{}; open diagnostics with Ctrl+d", state.status_text())
            }
            (PromptOrigin::AutoFollow(_), ShellActionAvailability::Pending) => {
                "auto follow-up paused while startup checks are still running".to_string()
            }
            (PromptOrigin::AutoFollow(_), ShellActionAvailability::Blocked) => {
                "auto follow-up paused because startup diagnostics need attention".to_string()
            }
        }
    }

    pub(super) fn conversation_has_running_turn(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation) if conversation.has_running_turn()
        )
    }

    pub(super) fn show_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayShown);
    }

    pub(super) fn show_session_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayShown {
            limit: SESSION_PAGE_SIZE,
        });
    }

    pub(super) fn show_queue_overlay(&mut self) {
        self.refresh_ready_conversation_planning_runtime_snapshot();
        self.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
    }

    pub(super) fn show_directions_maintenance_overlay(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        if !snapshot.plan_enabled() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "Plan off - initialize with :planning first".to_string(),
            });
            return;
        }
        self.present_directions_maintenance_overview(
            "opened directions maintenance".to_string(),
            true,
        );
    }

    pub(super) fn present_directions_maintenance_overview(
        &mut self,
        status_text: String,
        ensure_overlay_visible: bool,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        match self.planning.workspace.load_summary(&workspace_directory) {
            Ok(summary) => {
                self.directions_maintenance_overlay_ui_state
                    .open_summary(summary);
                self.planning_draft_editor_ui_state.reset();
                if ensure_overlay_visible {
                    self.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("directions maintenance unavailable: {error}"),
                });
            }
        }
    }

    pub(super) fn show_planning_init_overlay(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        if snapshot.workspace_present() {
            self.planning_init_overlay_ui_state
                .open_existing_workspace(PlanningInitEntryMode::CommandCenter);
        } else {
            self.planning_init_overlay_ui_state
                .open_command_center_mode_selection();
        }
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: if snapshot.workspace_present() {
                if snapshot.plan_enabled() {
                    "opened planning workspace controls".to_string()
                } else {
                    "opened planning workspace controls / Plan off".to_string()
                }
            } else {
                "opened planning initialization selector".to_string()
            },
        });
    }

    pub(super) fn show_planning_workflow_gate(&mut self, bootstrap_objective: Option<String>) {
        let workspace_directory = self.planning_workspace_directory();
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        if snapshot.workspace_present() {
            self.planning_init_overlay_ui_state
                .open_existing_workspace(PlanningInitEntryMode::WorkflowGate);
        } else {
            self.planning_init_overlay_ui_state.open_bootstrap_gate(
                PlanningInitEntryMode::WorkflowGate,
                bootstrap_objective.unwrap_or_default(),
            );
        }
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: if snapshot.workspace_present() {
                if snapshot.plan_enabled() {
                    "planning-first flow ready / review the current workspace before continuing"
                        .to_string()
                } else {
                    "planning-first flow requires Plan on before continuing".to_string()
                }
            } else {
                "planning-first flow requires a bootstrap objective before the first turn"
                    .to_string()
            },
        });
    }

    pub(super) fn show_planning_resume_gate(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        if snapshot.workspace_present() {
            self.planning_init_overlay_ui_state
                .open_existing_workspace(PlanningInitEntryMode::ResumeGate);
        } else {
            self.planning_init_overlay_ui_state
                .open_bootstrap_gate(PlanningInitEntryMode::ResumeGate, String::new());
        }
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: if snapshot.workspace_present() {
                "session loaded / review the planning resume card before continuing".to_string()
            } else {
                "session loaded / this workspace still needs a planning bootstrap objective"
                    .to_string()
            },
        });
    }

    pub(super) fn maybe_show_startup_planning_workflow_gate(&mut self) {
        if self.shell_action_availability() != ShellActionAvailability::Ready {
            return;
        }
        let should_gate = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if conversation.is_blank_draft()
                    && conversation.input_state.can_submit_now()
                    && !conversation.startup_submit_armed
                    && !conversation.has_running_turn()
                    && self.planning_requires_manual_gate(
                        &conversation.planning_runtime_snapshot
                    )
        );
        if should_gate {
            self.show_planning_workflow_gate(None);
        }
    }

    pub(super) fn maybe_show_resume_planning_gate(&mut self) {
        let should_gate = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if conversation.has_active_thread() && !conversation.has_running_turn()
        );
        if should_gate {
            self.show_planning_resume_gate();
        }
    }

    pub(super) fn toggle_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayToggled);
    }

    pub(super) fn toggle_session_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayToggled {
            limit: SESSION_PAGE_SIZE,
        });
    }

    pub(super) fn close_shell_overlay(&mut self) {
        match self.shell_overlay {
            ShellOverlay::DirectionsMaintenance => {
                self.directions_maintenance_overlay_ui_state.reset();
                self.planning_draft_editor_ui_state.reset();
            }
            ShellOverlay::PlanningInit => {
                self.planning_init_overlay_ui_state.reset();
                self.planning_draft_editor_ui_state.reset();
            }
            _ => {}
        }
        self.dispatch_shell_chrome(ShellChromeEvent::OverlayClosed);
    }

    pub(super) fn open_new_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::NewDraftRequested);
    }

    pub(super) fn execute_inline_shell_command_input(
        &mut self,
        command_input: InlineShellCommandInput,
    ) {
        match command_input.command() {
            InlineShellCommand::Diagnostics => self.show_startup_overlay(),
            InlineShellCommand::Sessions => self.show_session_overlay(),
            InlineShellCommand::Queue => self.show_queue_overlay(),
            InlineShellCommand::Directions => self.show_directions_maintenance_overlay(),
            InlineShellCommand::Stop => self.stop_post_turn_automation(),
            InlineShellCommand::Templates => self.show_followup_template_overlay(),
            InlineShellCommand::PlanningInit => {
                self.handle_planning_shell_command(command_input.argument())
            }
            InlineShellCommand::MaxAutoTurns => {
                let Some(value) = command_input.argument().map(str::to_string) else {
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: "usage: :turns <1-50>  |  alias: :auto-turns <1-50>"
                            .to_string(),
                    });
                    self.clear_input_buffer();
                    return;
                };
                self.dispatch_followup_controls(FollowupControlEvent::MaxAutoTurnsUpdated {
                    value,
                });
            }
            InlineShellCommand::NewDraft => self.open_new_conversation_shell(),
            InlineShellCommand::Help => {}
        }

        if let Some(status_text) = command_input.execution_status() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text,
            });
        }
        self.clear_input_buffer();
    }

    fn handle_planning_shell_command(&mut self, argument: Option<&str>) {
        match argument.map(str::trim).filter(|value| !value.is_empty()) {
            None => self.show_planning_init_overlay(),
            Some(value) if value.eq_ignore_ascii_case("off") => self.turn_plan_off(),
            Some(value) if value.eq_ignore_ascii_case("on") => self.turn_plan_on(),
            Some(value) => self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: format!(
                    "unsupported :planning argument `{value}` / supported: :planning, :planning on, :planning off"
                ),
            }),
        }
    }

    pub(super) fn turn_plan_on(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let entry_mode = self.planning_init_overlay_ui_state.entry_mode();
        if let Err(error) = self
            .planning
            .workspace
            .set_plan_enabled(&workspace_directory, true)
        {
            let fallback_status = if !self
                .planning
                .workspace
                .has_planning_workspace(&workspace_directory)
                .unwrap_or(false)
            {
                self.show_planning_init_overlay();
                "planning workspace missing; open :planning to initialize it".to_string()
            } else {
                format!("failed to enable planning mode: {error}")
            };
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: fallback_status,
            });
            return;
        }

        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        if self.shell_overlay == ShellOverlay::PlanningInit {
            if entry_mode == PlanningInitEntryMode::CommandCenter {
                self.planning_init_overlay_ui_state
                    .open_existing_workspace(entry_mode);
            } else {
                self.close_shell_overlay();
            }
        }
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: if entry_mode == PlanningInitEntryMode::CommandCenter {
                "Plan on / using the existing planning workspace".to_string()
            } else {
                "Plan on / planning-first flow resumed".to_string()
            },
        });
    }

    pub(super) fn turn_plan_off(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let entry_mode = self.planning_init_overlay_ui_state.entry_mode();
        match self
            .planning
            .workspace
            .set_plan_enabled(&workspace_directory, false)
        {
            Ok(()) => {
                self.stop_post_turn_automation();
                self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                    &workspace_directory,
                );
                if self.shell_overlay == ShellOverlay::DirectionsMaintenance {
                    self.close_shell_overlay();
                } else if self.shell_overlay == ShellOverlay::PlanningInit {
                    self.planning_init_overlay_ui_state
                        .open_existing_workspace(entry_mode);
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "Plan off / planning workspace retained for later resume"
                        .to_string(),
                });
            }
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("failed to turn Plan off: {error}"),
                })
            }
        }
    }

    pub(super) fn planning_requires_manual_gate(
        &self,
        snapshot: &crate::application::service::planning::PlanningRuntimeSnapshot,
    ) -> bool {
        !snapshot.workspace_present()
            || !snapshot.plan_enabled()
            || snapshot.workspace_status()
                == crate::application::service::planning::PlanningRuntimeWorkspaceStatus::Invalid
    }

    pub(super) fn open_planning_manual_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_guided_planning_editor_session(
            self.planning
                .workspace
                .stage_manual_editor_session(&workspace_directory),
            "planning draft editor ready",
            PlanningInitModeSelection::Detail,
        );
    }

    pub(super) fn open_directions_manual_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.planning
                .workspace
                .stage_editor_session(&workspace_directory),
            "directions editor ready",
        );
    }

    pub(super) fn open_directions_detail_doc_editor(&mut self, direction_id: &str) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.planning
                .workspace
                .stage_detail_doc_editor_session(&workspace_directory, direction_id),
            "directions detail doc editor ready",
        );
    }

    pub(super) fn open_queue_idle_prompt_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.planning
                .workspace
                .stage_queue_idle_prompt_editor_session(&workspace_directory),
            "queue-idle prompt editor ready",
        );
    }

    pub(super) fn save_planning_manual_editor(&mut self) {
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
        let status_text = match self.planning.workspace.save_draft_editor_files(
            &workspace_directory,
            &draft_name,
            &editable_files,
        ) {
            Ok(result) => {
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
                        "press Ctrl+P to promote into active planning files"
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
        let status_text = match self.planning.workspace.save_draft_editor_files(
            &workspace_directory,
            &draft_name,
            &editable_files,
        ) {
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
                        "press Ctrl+P to promote into active planning files"
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
        let promote_result = self.planning.workspace.promote_draft_editor_files(
            &workspace_directory,
            &draft_name,
            &editable_files,
        );
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
        let promote_result = self.planning.workspace.promote_draft_editor_files(
            &workspace_directory,
            &draft_name,
            &editable_files,
        );
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

    fn request_close_planning_manual_editor(&mut self) {
        match self.planning_draft_editor_ui_state.request_close() {
            PlanningDraftEditorCloseRequest::CloseImmediately => self.close_shell_overlay(),
            PlanningDraftEditorCloseRequest::ConfirmationRequired(risk) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: planning_manual_editor_close_warning_status(risk),
                });
            }
            PlanningDraftEditorCloseRequest::Confirmed(risk) => {
                self.close_planning_manual_editor_after_confirmation(risk);
            }
        }
    }

    fn request_close_directions_manual_editor(&mut self) {
        match self.planning_draft_editor_ui_state.request_close() {
            PlanningDraftEditorCloseRequest::CloseImmediately => self
                .close_directions_manual_editor_without_prompt(
                    "directions editor closed".to_string(),
                ),
            PlanningDraftEditorCloseRequest::ConfirmationRequired(risk) => {
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
        self.close_shell_overlay();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: planning_manual_editor_closed_status(risk),
        });
    }

    fn close_directions_manual_editor_after_confirmation(
        &mut self,
        risk: PlanningDraftEditorCloseRisk,
    ) {
        self.close_directions_manual_editor_without_prompt(directions_manual_editor_closed_status(
            risk,
        ));
    }

    fn close_directions_manual_editor_without_prompt(&mut self, status_text: String) {
        self.present_directions_maintenance_overview(status_text, true);
    }

    pub(super) fn handle_planning_manual_editor_close_confirmation_key(
        &mut self,
        key: event::KeyEvent,
    ) -> bool {
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
                self.planning_draft_editor_ui_state
                    .clear_close_confirmation();
                false
            }
        }
    }

    pub(super) fn stage_simple_mode_planning_init_draft(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let status_text = match self
            .planning
            .workspace
            .stage_simple_mode_draft(&workspace_directory)
        {
            Ok(stage_result) => {
                let validation_ok = stage_result.validation_report.is_valid();
                let draft_name = stage_result.draft_name.clone();
                self.planning_init_overlay_ui_state
                    .open_simple_review(stage_result);
                format!(
                    "planning simple draft staged / draft: {} / validation: {} / next: Enter or Ctrl+P to promote, Ctrl+E to inspect",
                    draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                )
            }
            Err(error) => format!("planning init failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn submit_planning_bootstrap_objective(&mut self) {
        let objective = self
            .planning_init_overlay_ui_state
            .bootstrap_objective()
            .trim()
            .to_string();
        if objective.is_empty() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text:
                    "planning bootstrap needs a first objective before the workspace can start"
                        .to_string(),
            });
            return;
        }

        let workspace_directory = self.planning_workspace_directory();
        let stage_result = self
            .planning
            .workspace
            .stage_simple_mode_draft(&workspace_directory);
        match stage_result {
            Ok(stage_result) => {
                let draft_name = stage_result.draft_name.clone();
                match self
                    .planning
                    .workspace
                    .promote_staged_draft(&workspace_directory, &draft_name)
                {
                    Ok(result) if result.promoted_file_count > 0 => {
                        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                            &workspace_directory,
                        );
                        self.close_shell_overlay();
                        self.submit_manual_prompt_from_text(objective.clone());
                    }
                    Ok(result) => {
                        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                            &workspace_directory,
                        );
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text: format!(
                                    "planning bootstrap promote blocked / draft: {} / validation needs attention",
                                    result.draft_name
                                ),
                            },
                        );
                    }
                    Err(error) => self.dispatch_conversation_input(
                        ConversationInputEvent::StatusMessageShown {
                            status_text: format!("planning bootstrap promote failed: {error}"),
                        },
                    ),
                }
            }
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("planning bootstrap failed: {error}"),
                })
            }
        }
    }

    pub(super) fn open_simple_mode_planning_editor(&mut self) {
        let Some(draft_name) = self
            .planning_init_overlay_ui_state
            .simple_review()
            .map(|review| review.draft_name().to_string())
        else {
            return;
        };
        let workspace_directory = self.planning_workspace_directory();
        self.open_guided_planning_editor_session(
            self.planning
                .workspace
                .load_manual_editor_session(&workspace_directory, &draft_name),
            "planning simple draft editor ready",
            PlanningInitModeSelection::Simple,
        );
    }

    pub(super) fn promote_simple_mode_planning_draft(&mut self) {
        let Some(draft_name) = self
            .planning_init_overlay_ui_state
            .simple_review()
            .map(|review| review.draft_name().to_string())
        else {
            return;
        };
        let workspace_directory = self.planning_workspace_directory();
        let promote_result = self
            .planning
            .workspace
            .promote_staged_draft(&workspace_directory, &draft_name);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        let status_text = match promote_result {
            Ok(result) => {
                let validation_ok = result.validation_report.is_valid();
                self.planning_init_overlay_ui_state
                    .apply_simple_review_validation(result.validation_report.clone());
                if result.promoted_file_count == 0 {
                    format!(
                        "planning simple draft promote blocked / draft: {} / validation: {} / next: press Ctrl+E to inspect or fix the staged draft",
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

    pub(super) fn push_input_character(&mut self, character: char) {
        self.dispatch_conversation_input(ConversationInputEvent::CharacterTyped { character });
    }

    pub(super) fn is_inline_command_palette_active(&self) -> bool {
        matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if conversation.inline_shell_command_palette_state.is_active()
        )
    }

    pub(super) fn move_inline_command_palette_selection(&mut self, delta: isize) -> bool {
        if !self.is_inline_command_palette_active() {
            return false;
        }

        self.dispatch_conversation_input(
            ConversationInputEvent::InlineCommandPaletteSelectionMoved { delta },
        );
        true
    }

    pub(super) fn dismiss_inline_command_palette(&mut self) -> bool {
        if !self.is_inline_command_palette_active() {
            return false;
        }

        self.dispatch_conversation_input(ConversationInputEvent::InlineCommandPaletteDismissed);
        true
    }

    pub(super) fn accept_inline_command_palette_selection(&mut self) -> bool {
        let selected_command = match &self.conversation_state {
            ConversationState::Ready(conversation)
                if conversation.inline_shell_command_palette_state.is_active() =>
            {
                conversation
                    .inline_shell_command_palette_state
                    .selected_command()
            }
            _ => None,
        };
        let Some(command) = selected_command else {
            return false;
        };

        if command.requires_argument() {
            self.dispatch_conversation_input(
                ConversationInputEvent::InlineCommandPaletteCommandInserted { command },
            );
            return true;
        }

        self.execute_inline_shell_command_input(InlineShellCommandInput::from_command(command));
        true
    }

    pub(super) fn insert_input_newline(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::NewlineInserted);
    }

    pub(super) fn pop_input_character(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::BackspacePressed);
    }

    pub(super) fn delete_previous_input_word(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::PreviousWordDeleted);
    }

    pub(super) fn clear_prompt_input(&mut self) {
        self.clear_input_buffer();
    }

    pub(super) fn is_shell_overlay_visible(&self) -> bool {
        self.shell_overlay != ShellOverlay::Hidden
    }

    pub(super) fn is_exit_confirmation_visible(&self) -> bool {
        self.exit_confirmation_state == ExitConfirmationState::Visible
    }

    pub(super) fn handle_exit_confirmation_key(&mut self, key: event::KeyEvent) -> Option<bool> {
        if !self.is_exit_confirmation_visible() {
            return None;
        }

        if !key.modifiers.is_empty() && key.modifiers != KeyModifiers::SHIFT {
            return None;
        }

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);
                Some(false)
            }
            _ => Some(false),
        }
    }

    pub(super) fn handle_shell_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay == ShellOverlay::Hidden {
            return false;
        }
        let is_startup_overlay = self.shell_overlay == ShellOverlay::Startup;

        if self.handle_max_auto_turns_editor_key(key) {
            return true;
        }

        if self.handle_stop_keyword_editor_key(key) {
            return true;
        }

        if self.handle_session_search_query_editor_key(key) {
            return true;
        }

        if key.code == KeyCode::Esc
            || (key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c'))
        {
            let closing_directions_manual_editor = self.shell_overlay
                == ShellOverlay::DirectionsMaintenance
                && self.directions_maintenance_overlay_ui_state.step()
                    == DirectionsMaintenanceOverlayStep::ManualEditor;
            let closing_planning_manual_editor = self.shell_overlay == ShellOverlay::PlanningInit
                && self.planning_init_overlay_ui_state.step()
                    == PlanningInitOverlayStep::ManualEditor;
            if closing_directions_manual_editor {
                self.request_close_directions_manual_editor();
            } else if closing_planning_manual_editor {
                self.request_close_planning_manual_editor();
            } else if self.shell_overlay == ShellOverlay::PlanningInit
                && self.planning_init_overlay_ui_state.step()
                    == PlanningInitOverlayStep::BootstrapObjective
            {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text:
                        "planning-first bootstrap is required before the first turn in this workspace"
                            .to_string(),
                });
            } else {
                self.close_shell_overlay();
            }
            return true;
        }

        if is_startup_overlay {
            match key.code {
                KeyCode::Char('r') if key.modifiers.is_empty() => {
                    self.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested)
                }
                KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                    self.show_session_overlay()
                }
                _ => {}
            }
            return true;
        }

        if self.handle_followup_overlay_key(key) {
            return true;
        }

        if self.shell_overlay == ShellOverlay::DirectionsMaintenance {
            return self.handle_directions_overlay_key(key);
        }

        if self.shell_overlay == ShellOverlay::PlanningInit {
            return self.handle_planning_init_overlay_key(key);
        }

        self.handle_session_overlay_key(key);
        true
    }

    pub(super) fn handle_ctrl_c(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationHidden);

        if self.is_shell_overlay_visible() {
            self.close_shell_overlay();
            return;
        }

        self.dispatch_conversation_intent(ConversationIntentEvent::CtrlCPressed);
    }

    fn open_guided_planning_editor_session(
        &mut self,
        session_result: PlanningEditorSessionResult,
        ready_status_prefix: &str,
        mode: PlanningInitModeSelection,
    ) {
        let status_text = match session_result {
            Ok(session) => {
                let validation_ok = session.validation_report.is_valid();
                let status_text = format!(
                    "{ready_status_prefix} / draft: {} / validation: {}",
                    session.draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                );
                self.planning_draft_editor_ui_state.open_session(session);
                match mode {
                    PlanningInitModeSelection::Simple => {
                        self.planning_init_overlay_ui_state.open_simple_editor()
                    }
                    PlanningInitModeSelection::Detail => {
                        self.planning_init_overlay_ui_state.open_manual_editor()
                    }
                }
                status_text
            }
            Err(error) => format!("planning init failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    fn open_directions_editor_session(
        &mut self,
        session_result: PlanningEditorSessionResult,
        ready_status_prefix: &str,
    ) {
        let status_text = match session_result {
            Ok(session) => {
                let validation_ok = session.validation_report.is_valid();
                let status_text = format!(
                    "{ready_status_prefix} / draft: {} / validation: {}",
                    session.draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                );
                self.planning_draft_editor_ui_state.open_session(session);
                self.directions_maintenance_overlay_ui_state
                    .open_manual_editor();
                status_text
            }
            Err(error) => format!("directions editor failed: {error}"),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
}

fn planning_manual_editor_close_warning_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
    ) {
        (true, true) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "planning draft editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "planning draft editor close pending".to_string(),
    }
}

fn planning_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
    ) {
        (true, true) => "planning draft editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "planning draft editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "planning draft editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "planning draft editor closed".to_string(),
    }
}

fn directions_manual_editor_close_warning_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
    ) {
        (true, true) => "directions editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "directions editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "directions editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "directions editor close pending".to_string(),
    }
}

fn directions_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
    ) {
        (true, true) => "directions editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "directions editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "directions editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "directions editor closed".to_string(),
    }
}
