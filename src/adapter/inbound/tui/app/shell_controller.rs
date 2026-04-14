use std::time::Instant;

use super::planning_draft_editor_ui::{
    PlanningDraftEditorCloseRequest, PlanningDraftEditorCloseRisk,
};
use super::*;
use crate::application::service::planning_init_service::PlanningDraftEditorSession;
use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;

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

    pub(super) fn sync_draft_shell_workspace(&mut self, workspace_directory: &str) {
        let should_refresh_draft = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if !conversation.has_active_thread()
                    && conversation.draft_workspace_directory() != workspace_directory
        );
        if !should_refresh_draft {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory: workspace_directory.to_string(),
            template_load_result: self.load_followup_template_catalog(workspace_directory),
        });
        self.refresh_ready_conversation_planning_runtime_snapshot();
    }

    pub(super) fn reload_followup_templates(&mut self) {
        let workspace_directory = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.planning_workspace_directory().to_string()
            }
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };

        self.dispatch_followup_controls(FollowupControlEvent::TemplateCatalogReloaded {
            reload_result: self
                .followup_template_service
                .reload_catalog(&workspace_directory),
        });
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

    fn present_directions_maintenance_overview(
        &mut self,
        status_text: String,
        ensure_overlay_visible: bool,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        match self
            .planning_services
            .directions_service
            .load_summary(&workspace_directory)
        {
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

    pub(super) fn show_followup_template_overlay(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::OverlayShown {
            stop_keyword: self.current_stop_keyword_value(),
            max_auto_turns: self.current_max_auto_turns_value().to_string(),
        });
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayShown);
    }

    pub(super) fn show_planning_init_overlay(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        if snapshot.workspace_present() {
            self.planning_init_overlay_ui_state
                .open_existing_workspace();
        } else {
            self.planning_init_overlay_ui_state.reset();
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

    pub(super) fn toggle_startup_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::StartupOverlayToggled);
    }

    pub(super) fn toggle_session_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SessionsOverlayToggled {
            limit: SESSION_PAGE_SIZE,
        });
    }

    pub(super) fn toggle_followup_template_overlay(&mut self) {
        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
            return;
        }
        self.dispatch_shell_chrome(ShellChromeEvent::FollowupTemplatesOverlayToggled);
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

    fn turn_plan_on(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        if let Err(error) = self
            .planning_services
            .init_service
            .set_plan_enabled(&workspace_directory, true)
        {
            let fallback_status = if !self
                .planning_services
                .init_service
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
            self.planning_init_overlay_ui_state
                .open_existing_workspace();
        }
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "Plan on / using the existing planning workspace".to_string(),
        });
    }

    fn turn_plan_off(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        match self
            .planning_services
            .init_service
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
                        .open_existing_workspace();
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

    pub(super) fn open_planning_manual_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_guided_planning_editor_session(
            self.planning_services
                .init_service
                .stage_manual_editor_session(&workspace_directory),
            "planning draft editor ready",
            PlanningInitModeSelection::Detail,
        );
    }

    pub(super) fn open_directions_manual_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.planning_services
                .directions_service
                .stage_editor_session(&workspace_directory),
            "directions editor ready",
        );
    }

    pub(super) fn open_directions_detail_doc_editor(&mut self, direction_id: &str) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.planning_services
                .directions_service
                .stage_detail_doc_editor_session(&workspace_directory, direction_id),
            "directions detail doc editor ready",
        );
    }

    pub(super) fn open_queue_idle_prompt_editor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.open_directions_editor_session(
            self.planning_services
                .directions_service
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
        let status_text = match self.planning_services.init_service.save_draft_editor_files(
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
        let status_text = match self.planning_services.init_service.save_draft_editor_files(
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
        let promote_result = self
            .planning_services
            .init_service
            .promote_draft_editor_files(&workspace_directory, &draft_name, &editable_files);
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
        let promote_result = self
            .planning_services
            .init_service
            .promote_draft_editor_files(&workspace_directory, &draft_name, &editable_files);
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

    fn handle_planning_manual_editor_close_confirmation_key(
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

    fn handle_directions_manual_editor_close_confirmation_key(
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
            .planning_services
            .init_service
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
            self.planning_services
                .init_service
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
            .planning_services
            .init_service
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

    pub(super) fn toggle_auto_followup(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowToggled);
    }

    pub(super) fn stop_post_turn_automation(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::AutoFollowStopped);
    }

    pub(super) fn current_max_auto_turns_value(&self) -> usize {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.auto_follow_state.max_auto_turns_value()
            }
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_MAX_TURNS
            }
        }
    }

    pub(super) fn current_stop_keyword_value(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .auto_follow_state
                .stop_keyword_value()
                .to_string(),
            ConversationState::Loading | ConversationState::Failed(_) => {
                DEFAULT_AUTO_FOLLOW_STOP_KEYWORD.to_string()
            }
        }
    }

    pub(super) fn planner_visibility_label(&self) -> &'static str {
        self.planner_visibility.label()
    }

    pub(super) fn planner_shows_debug_details(&self) -> bool {
        self.planner_visibility.shows_debug_details()
    }

    pub(super) fn live_activity_pulse(&self, now: Instant) -> Option<u64> {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .auto_follow_state
                .active_started_at()
                .map(|started_at| now.saturating_duration_since(started_at).as_secs()),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    pub(super) fn is_max_auto_turns_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .max_auto_turns_editor
            .is_editing
    }

    pub(super) fn is_stop_keyword_editing(&self) -> bool {
        self.followup_overlay_ui_state
            .stop_keyword_editor
            .is_editing
    }

    pub(super) fn start_max_auto_turns_edit(&mut self) {
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        if self.shell_overlay != ShellOverlay::FollowupTemplates
            && self.shell_overlay != ShellOverlay::PlanningInit
        {
            self.show_followup_template_overlay();
        }

        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditStarted {
            current_value: self.current_max_auto_turns_value().to_string(),
        });
    }

    pub(super) fn save_max_auto_turns_edit(&mut self) {
        if !self.is_max_auto_turns_editing() {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::MaxAutoTurnsUpdated {
            value: self
                .followup_overlay_ui_state
                .max_auto_turns_editor
                .buffer
                .clone(),
        });
    }

    pub(super) fn cancel_max_auto_turns_edit(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsEditCanceled {
            current_value: self.current_max_auto_turns_value().to_string(),
        });
    }

    pub(super) fn push_max_auto_turns_character(&mut self, character: char) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped {
            character,
        });
    }

    pub(super) fn pop_max_auto_turns_character(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed);
    }

    pub(super) fn start_stop_keyword_edit(&mut self) {
        if !matches!(self.conversation_state, ConversationState::Ready(_)) {
            return;
        }

        if self.shell_overlay != ShellOverlay::FollowupTemplates {
            self.show_followup_template_overlay();
        }

        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordEditStarted {
            current_value: self.current_stop_keyword_value(),
        });
    }

    pub(super) fn save_stop_keyword_edit(&mut self) {
        if !self.is_stop_keyword_editing() {
            return;
        }

        self.dispatch_followup_controls(FollowupControlEvent::StopKeywordValueUpdated {
            value: self
                .followup_overlay_ui_state
                .stop_keyword_editor
                .buffer
                .clone(),
        });
    }

    pub(super) fn cancel_stop_keyword_edit(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordEditCanceled {
            current_value: self.current_stop_keyword_value(),
        });
    }

    pub(super) fn push_stop_keyword_character(&mut self, character: char) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordCharacterTyped {
            character,
        });
    }

    pub(super) fn pop_stop_keyword_character(&mut self) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordBackspacePressed);
    }

    pub(super) fn toggle_stop_keyword(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::StopKeywordToggled);
    }

    pub(super) fn toggle_no_file_change_stop(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::NoFileChangeStopToggled);
    }

    pub(super) fn toggle_planner_visibility(&mut self) {
        self.planner_visibility = self.planner_visibility.toggle();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: format!("planner detail {}", self.planner_visibility.label()),
        });
    }

    pub(super) fn cycle_auto_followup_template(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledForward);
    }

    pub(super) fn cycle_auto_followup_template_backward(&mut self) {
        self.dispatch_followup_controls(FollowupControlEvent::TemplateCycledBackward);
    }

    #[cfg(test)]
    pub(super) fn followup_template_selection(&self) -> Option<usize> {
        match &self.conversation_state {
            ConversationState::Ready(conversation)
                if !conversation
                    .auto_follow_state
                    .template_state
                    .items
                    .is_empty() =>
            {
                Some(conversation.auto_follow_state.selected_template_index())
            }
            _ => None,
        }
    }

    pub(super) fn scroll_followup_template_preview(&mut self, delta: i32) {
        self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::PreviewScrolled { delta });
    }

    pub(super) fn current_workspace_directory(&self) -> String {
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    pub(super) fn planning_workspace_directory(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.planning_workspace_directory().to_string()
            }
            _ => self.current_workspace_directory(),
        }
    }

    pub(super) fn load_followup_template_catalog(
        &self,
        workspace_directory: &str,
    ) -> FollowupTemplateCatalogLoadResult {
        self.followup_template_service
            .load_catalog(workspace_directory)
    }

    pub(super) fn load_planning_runtime_snapshot(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning_services
            .runtime_facade
            .load_runtime_snapshot_or_invalid(workspace_directory)
    }

    pub(super) fn refresh_ready_conversation_planning_runtime_snapshot(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
    }

    pub(super) fn refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
        &mut self,
        workspace_directory: &str,
    ) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        conversation.replace_planning_runtime_snapshot(
            self.load_planning_runtime_snapshot(workspace_directory),
        );
        self.conversation_state = ConversationState::Ready(conversation);
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

    pub(super) fn handle_stop_keyword_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::FollowupTemplates || !self.is_stop_keyword_editing()
        {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.save_stop_keyword_edit(),
            KeyCode::Esc => self.cancel_stop_keyword_edit(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_stop_keyword_edit()
            }
            KeyCode::Backspace => self.pop_stop_keyword_character(),
            KeyCode::Char(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.push_stop_keyword_character(character);
            }
            _ => {}
        }

        true
    }

    pub(super) fn handle_max_auto_turns_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if !self.is_max_auto_turns_editing() {
            return false;
        }

        let editor_supported = self.shell_overlay == ShellOverlay::FollowupTemplates
            || (self.shell_overlay == ShellOverlay::PlanningInit
                && self.planning_init_overlay_ui_state.step()
                    == PlanningInitOverlayStep::SimpleReview);
        if !editor_supported {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.save_max_auto_turns_edit(),
            KeyCode::Esc => self.cancel_max_auto_turns_edit(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_max_auto_turns_edit()
            }
            KeyCode::Backspace => self.pop_max_auto_turns_character(),
            KeyCode::Char(character)
                if (key.modifiers == KeyModifiers::NONE
                    || key.modifiers == KeyModifiers::SHIFT)
                    && character.is_ascii_digit() =>
            {
                self.push_max_auto_turns_character(character);
            }
            _ => {}
        }

        true
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

        if self.shell_overlay == ShellOverlay::FollowupTemplates {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                    self.cycle_auto_followup_template_backward()
                }
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.cycle_auto_followup_template()
                }
                KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
                    self.cycle_auto_followup_template()
                }
                KeyCode::Char('r') if key.modifiers.is_empty() => self.reload_followup_templates(),
                KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_auto_followup()
                }
                KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                    self.start_max_auto_turns_edit()
                }
                KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL => {
                    self.start_stop_keyword_edit()
                }
                KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_stop_keyword()
                }
                KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_no_file_change_stop()
                }
                KeyCode::Char('b') if key.modifiers == KeyModifiers::CONTROL => {
                    self.toggle_planner_visibility()
                }
                KeyCode::PageUp if key.modifiers.is_empty() => self
                    .scroll_followup_template_preview(
                        -(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                    ),
                KeyCode::PageDown if key.modifiers.is_empty() => self
                    .scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => self
                    .scroll_followup_template_preview(
                        -(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                    ),
                KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => self
                    .scroll_followup_template_preview(FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP as i32),
                KeyCode::Enter if key.modifiers.is_empty() => self.close_shell_overlay(),
                _ => {}
            }
            return true;
        }

        if self.shell_overlay == ShellOverlay::DirectionsMaintenance {
            match self.directions_maintenance_overlay_ui_state.step() {
                DirectionsMaintenanceOverlayStep::Overview => match key.code {
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        self.open_directions_manual_editor()
                    }
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
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text:
                                        "fix directions.toml parse errors before editing queue-idle prompt"
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

                    match key.code {
                        KeyCode::Tab if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.move_file_selection(1)
                        }
                        KeyCode::BackTab => {
                            self.planning_draft_editor_ui_state.move_file_selection(-1)
                        }
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
                        KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => {
                            self.save_directions_manual_editor()
                        }
                        KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                            self.promote_directions_manual_editor()
                        }
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
            return true;
        }

        if self.shell_overlay == ShellOverlay::PlanningInit {
            match self.planning_init_overlay_ui_state.step() {
                PlanningInitOverlayStep::ExistingWorkspace => {
                    let workspace_directory = self.planning_workspace_directory();
                    let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
                    match key.code {
                        KeyCode::Enter if key.modifiers.is_empty() => {
                            if snapshot.plan_enabled() {
                                self.close_shell_overlay();
                                self.show_queue_overlay();
                            } else {
                                self.turn_plan_on();
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
                            PlanningInitDetailSelection::Manual => {
                                self.open_planning_manual_editor();
                            }
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

                    match key.code {
                        KeyCode::Tab if key.modifiers.is_empty() => {
                            self.planning_draft_editor_ui_state.move_file_selection(1)
                        }
                        KeyCode::BackTab => {
                            self.planning_draft_editor_ui_state.move_file_selection(-1)
                        }
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
                        KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => {
                            self.save_planning_manual_editor()
                        }
                        KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                            self.promote_planning_manual_editor()
                        }
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
            return true;
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
