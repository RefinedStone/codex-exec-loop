use crossterm::event::{self, KeyCode, KeyModifiers};

use super::super::planning_draft_editor_ui::{
    PlanningDraftEditorCloseRequest, PlanningDraftEditorCloseRisk,
};
use super::super::{
    ConversationInputEvent, DetailDocConfirmChoice, DirectionsMaintenanceOverlayStep, NativeTuiApp,
    PlanningInitDetailSelection, PlanningInitModeSelection, PlanningInitOverlayStep,
    ShellChromeEvent, ShellOverlay,
};
use crate::application::service::planning::{
    PlanningDoctorReport, PlanningDoctorState, PlanningDraftEditorSession, PlanningResetTarget,
    PlanningTrackedDirectionsApplyResult, PlanningTrackedTaskLedgerApplyResult,
    PlanningWorkspaceResetResult,
};

type PlanningEditorSessionResult = anyhow::Result<PlanningDraftEditorSession>;

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

    pub(in crate::adapter::inbound::tui::app) fn show_directions_maintenance_overlay(&mut self) {
        self.present_directions_maintenance_overview(
            "opened directions maintenance".to_string(),
            true,
        );
    }

    pub(in crate::adapter::inbound::tui::app) fn handle_directions_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        match argument.map(str::trim).filter(|value| !value.is_empty()) {
            None => self.show_directions_maintenance_overlay(),
            Some(value) if value.eq_ignore_ascii_case("apply") => {
                self.apply_tracked_directions_from_workspace()
            }
            Some(value) => self.dispatch_conversation_input(
                ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "unsupported :directions argument `{value}` / supported: :directions, :directions apply"
                    ),
                },
            ),
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn handle_queue_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        match argument.map(str::trim).filter(|value| !value.is_empty()) {
            None => self.show_queue_overlay(),
            Some(value) if value.eq_ignore_ascii_case("apply") => {
                self.apply_tracked_task_ledger_from_workspace()
            }
            Some(value) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "unsupported :queue argument `{value}` / supported: :queue, :queue apply"
                    ),
                })
            }
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn present_directions_maintenance_overview(
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

    fn apply_tracked_directions_from_workspace(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        match self
            .planning
            .workspace
            .apply_tracked_directions(&workspace_directory)
        {
            Ok(result) if result.applied() => {
                self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                    &workspace_directory,
                );
                self.present_directions_maintenance_overview(
                    tracked_directions_apply_status_text(&result),
                    true,
                );
            }
            Ok(result) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: tracked_directions_apply_status_text(&result),
                });
            }
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("tracked directions apply failed: {error}"),
                });
            }
        }
    }

    fn apply_tracked_task_ledger_from_workspace(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        match self
            .planning
            .workspace
            .apply_tracked_task_ledger(&workspace_directory)
        {
            Ok(result) if result.applied() => {
                self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                    &workspace_directory,
                );
                self.show_queue_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: tracked_task_ledger_apply_status_text(&result),
                });
            }
            Ok(result) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: tracked_task_ledger_apply_status_text(&result),
                });
            }
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("tracked task catalog apply failed: {error}"),
                });
            }
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn show_planning_init_overlay(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        if snapshot.workspace_present() {
            self.planning_init_overlay_ui_state
                .open_existing_workspace();
        } else {
            self.planning_init_overlay_ui_state
                .open_command_center_mode_selection();
        }
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: if snapshot.workspace_present() {
                "operator surface: planning setup / existing workspace".to_string()
            } else {
                "operator surface: planning setup / workspace: not initialized".to_string()
            },
        });
    }

    pub(in crate::adapter::inbound::tui::app) fn open_first_run_planning_simple_review(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        self.planning_init_overlay_ui_state
            .open_command_center_mode_selection();
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.stage_simple_mode_planning_init_draft();
    }

    pub(in crate::adapter::inbound::tui::app) fn ensure_default_active_planning_workspace(
        &mut self,
    ) -> anyhow::Result<()> {
        let workspace_directory = self.planning_workspace_directory();
        match self
            .planning
            .workspace
            .has_planning_workspace(&workspace_directory)
        {
            Ok(true) => Ok(()),
            Ok(false) => {
                if self
                    .planning
                    .workspace
                    .has_planning_candidate_workspace(&workspace_directory)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "planning candidate workspace check failed before operator action: {error}"
                        )
                    })?
                {
                    anyhow::bail!(
                        "planning default bootstrap blocked: tracked planning candidates exist without active authority / run :directions apply or :queue apply before :task"
                    );
                }
                let stage_result = self
                    .planning
                    .workspace
                    .stage_simple_mode_draft(&workspace_directory)
                    .map_err(|error| {
                        anyhow::anyhow!("planning default bootstrap failed while staging: {error}")
                    })?;
                let promote_result = self
                    .planning
                    .workspace
                    .promote_staged_draft(&workspace_directory, &stage_result.draft_name)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "planning default bootstrap failed while promoting: {error}"
                        )
                    })?;
                if promote_result.promoted_file_count == 0 {
                    let first_error = promote_result
                        .validation_report
                        .errors()
                        .first()
                        .map(|issue| issue.message.as_str())
                        .unwrap_or("planning validation failed");
                    anyhow::bail!(
                        "planning default bootstrap was blocked by validation: {first_error}"
                    );
                }
                self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                    &workspace_directory,
                );
                Ok(())
            }
            Err(error) => Err(anyhow::anyhow!(
                "planning workspace check failed before operator action: {error}"
            )),
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn handle_planning_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        match argument.map(str::trim).filter(|value| !value.is_empty()) {
            None => {
                let workspace_directory = self.planning_workspace_directory();
                match self
                    .planning
                    .workspace
                    .has_planning_workspace(&workspace_directory)
                {
                    Ok(true) => self.show_planning_init_overlay(),
                    Ok(false) => self.open_first_run_planning_simple_review(),
                    Err(error) => {
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text: format!("planning setup unavailable: {error}"),
                            },
                        );
                    }
                }
            }
            Some(value) if value.eq_ignore_ascii_case("doctor") => self.run_planning_doctor(),
            Some(value) => self.dispatch_conversation_input(
                ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "unsupported :planning argument `{value}` / supported: :planning, :planning doctor, :doctor"
                    ),
                },
            ),
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn run_planning_doctor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let report = self
            .planning
            .workspace
            .inspect_workspace(&workspace_directory);

        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        if report.planning_state() == PlanningDoctorState::Absent {
            self.show_planning_init_overlay();
        } else if self.shell_overlay == ShellOverlay::PlanningInit {
            self.planning_init_overlay_ui_state
                .open_existing_workspace();
        }
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: planning_doctor_status_text(&report),
        });
    }

    pub(in crate::adapter::inbound::tui::app) fn handle_init_shell_command(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        match self
            .planning
            .workspace
            .has_planning_workspace(&workspace_directory)
        {
            Ok(true) => {
                self.show_planning_init_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "planning workspace already exists / next action: review existing controls or use :reset before replacing it".to_string(),
                });
            }
            Ok(false) => {
                self.open_first_run_planning_simple_review();
            }
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("planning init unavailable: {error}"),
                });
            }
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn handle_reset_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        let parsed = match parse_reset_shell_argument(argument) {
            Ok(parsed) => parsed,
            Err(usage) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: usage,
                });
                return;
            }
        };

        if matches!(
            parsed.target,
            PlanningResetTarget::Directions | PlanningResetTarget::All
        ) && !parsed.confirmed
        {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: planning_reset_preview_text(parsed.target),
            });
            return;
        }

        let workspace_directory = self.planning_workspace_directory();
        match self
            .planning
            .workspace
            .reset_workspace(&workspace_directory, parsed.target)
        {
            Ok(result) => {
                self.pause_post_turn_continuation();
                self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                    &workspace_directory,
                );
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: planning_reset_status_text(&result),
                });
            }
            Err(error) => {
                let fallback_status = if !self
                    .planning
                    .workspace
                    .has_planning_workspace(&workspace_directory)
                    .unwrap_or(false)
                {
                    self.show_planning_init_overlay();
                    "planning workspace: missing / next action: open :planning to initialize it"
                        .to_string()
                } else {
                    format!("planning reset failed: {error}")
                };
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: fallback_status,
                });
            }
        }
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

    pub(in crate::adapter::inbound::tui::app) fn request_close_planning_manual_editor(&mut self) {
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

    pub(in crate::adapter::inbound::tui::app) fn request_close_directions_manual_editor(&mut self) {
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
                    "planning simple review ready / staged draft: {} / validation state: {} / simple behavior: no next task yet; queue-idle review stays enabled / next action: Enter or Ctrl+P promotes the low-ceremony scaffold. Ctrl+E inspects the draft. D opens detail-mode authoring.",
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

fn planning_manual_editor_close_warning_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "planning draft editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "planning draft editor close pending".to_string(),
    }
}

fn planning_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "planning draft editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "planning draft editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "planning draft editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "planning draft editor closed".to_string(),
    }
}

fn directions_manual_editor_close_warning_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "directions editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "directions editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "directions editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "directions editor close pending".to_string(),
    }
}

fn directions_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "directions editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "directions editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "directions editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "directions editor closed".to_string(),
    }
}

fn planning_doctor_status_text(report: &PlanningDoctorReport) -> String {
    let mut parts = vec![format!(
        "planning state: {}",
        report.planning_state().label()
    )];

    if let Some(queue_idle_policy) = report.queue_idle_policy() {
        parts.push(format!("queue-idle: {queue_idle_policy}"));
    }
    if let Some(queue_summary) = report.queue_summary() {
        parts.push(format!("queue: {queue_summary}"));
    }
    if let Some(proposal_summary) = report.proposal_summary() {
        parts.push(format!("proposals: {proposal_summary}"));
    }
    if let Some(issue) = report.issue() {
        parts.push(format!("issue: {issue}"));
    } else if let Some(health) = report.health() {
        parts.push(format!("health: {health}"));
    }
    if let Some(note) = report.note() {
        parts.push(format!("note: {note}"));
    }
    if report.planning_state() == PlanningDoctorState::Absent {
        parts.push("next action: run :init to stage the default planning scaffold".to_string());
    }

    parts.join(" / ")
}

fn tracked_directions_apply_status_text(result: &PlanningTrackedDirectionsApplyResult) -> String {
    if result.applied() {
        return format!(
            "tracked directions applied / files: {} / next action: review the refreshed directions overlay",
            result.applied_paths.len()
        );
    }

    let issue = result
        .validation_report
        .errors()
        .first()
        .map(|issue| issue.message.as_str())
        .unwrap_or("planning validation failed");
    format!("tracked directions apply blocked / issue: {issue}")
}

fn tracked_task_ledger_apply_status_text(result: &PlanningTrackedTaskLedgerApplyResult) -> String {
    if result.applied() {
        return format!(
            "tracked task catalog applied / files: {} / next action: review the refreshed planning queue",
            result.applied_paths.len()
        );
    }

    let issue = result
        .validation_report
        .errors()
        .first()
        .map(|issue| issue.message.as_str())
        .unwrap_or("planning validation failed");
    format!("tracked task catalog apply blocked / issue: {issue}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedResetShellCommand {
    target: PlanningResetTarget,
    confirmed: bool,
}

fn parse_reset_shell_argument(argument: Option<&str>) -> Result<ParsedResetShellCommand, String> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(
            "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                .to_string(),
        );
    };
    let mut parts = argument.split_whitespace();
    let Some(target) = parts.next() else {
        return Err(
            "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                .to_string(),
        );
    };
    let confirmation = parts.next();
    let confirmed = match confirmation {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("confirm") => true,
        Some(_) => {
            return Err(
                "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                    .to_string(),
            );
        }
    };
    if parts.next().is_some() {
        return Err(
            "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                .to_string(),
        );
    }
    let target = match target.to_ascii_lowercase().as_str() {
        "queue" => PlanningResetTarget::Queue,
        "directions" => PlanningResetTarget::Directions,
        "all" => PlanningResetTarget::All,
        _ => {
            return Err(
                "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                    .to_string(),
            );
        }
    };
    Ok(ParsedResetShellCommand { target, confirmed })
}

fn planning_reset_preview_text(target: PlanningResetTarget) -> String {
    match target {
        PlanningResetTarget::Queue => {
            "reset queue preview: rewrites the task catalog compatibility file and clears derived queue state"
                .to_string()
        }
        PlanningResetTarget::Directions => "reset directions preview: rewrites directions.toml, recreates the default queue-idle prompt, removes direction detail docs and prompt artifacts, and clears derived queue state / rerun `:reset directions confirm` to continue".to_string(),
        PlanningResetTarget::All => "reset all preview: replaces the full active planning scaffold, clears derived queue state, and refreshes the planning authority / rerun `:reset all confirm` to continue".to_string(),
    }
}

fn planning_reset_status_text(result: &PlanningWorkspaceResetResult) -> String {
    format!(
        "planning reset applied / target: {} / rewritten: {} / removed: {}",
        result.target.label(),
        result.rewritten_paths.len(),
        result.removed_paths.len(),
    )
}
