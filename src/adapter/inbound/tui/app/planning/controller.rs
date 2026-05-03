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
    PlanningWorkspaceResetResult,
};
use crossterm::event::{self, KeyCode, KeyModifiers};
type PlanningEditorSessionResult = anyhow::Result<PlanningDraftEditorSession>;
mod directions_overlay;
mod editor;
mod planning_init_overlay;
mod status_text;
use self::status_text::{
    directions_manual_editor_close_warning_status, directions_manual_editor_closed_status,
    parse_reset_shell_argument, planning_doctor_status_text,
    planning_manual_editor_close_warning_status, planning_manual_editor_closed_status,
    planning_reset_preview_text, planning_reset_status_text,
};

// Planning control is the TUI adapter layer for workspace mutations: it keeps
// shell overlays, editor state, and conversation status messages in sync while
// delegating filesystem authority to the planning application service.
impl NativeTuiApp {
    // Shell command handlers normalize user input before touching overlay state.
    // Unsupported arguments are surfaced as status rows so command mistakes do
    // not leave partial UI transitions behind.
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
            Some(value) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "unsupported :directions argument `{value}` / supported: :directions"
                    ),
                })
            }
        }
    }
    pub(in crate::adapter::inbound::tui::app) fn handle_queue_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        match argument.map(str::trim).filter(|value| !value.is_empty()) {
            None => self.show_queue_overlay(),
            Some(value) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "`:queue` does not accept arguments (`{value}`); use :queue to open queue inspection"
                    ),
                })
            }
        }
    }

    // Opening directions maintenance resets any draft editor first. Only one
    // planning-adjacent overlay owns the editor at a time, otherwise stale
    // buffers can be saved into the wrong workspace draft.
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

    // Planning init chooses between existing-workspace controls and first-run
    // setup from a fresh service snapshot, then refreshes the ready
    // conversation snapshot so the inline shell reports the same authority.
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

    // Default bootstrap is used by task submission paths that need a planning
    // workspace before the operator has explicitly initialized one. Candidate
    // workspaces block this automatic path because they require doctor review.
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
                        "planning default bootstrap blocked: tracked planning candidates exist without active authority / run :doctor before :task"
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

    // `:planning` is an overlay command first and a diagnostic command only
    // when explicitly asked for `doctor`; this keeps first-run setup discoverable
    // without making the happy path depend on a full inspection report.
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

    // Reset is a destructive workspace operation, so directions/all resets use
    // a preview status unless the command argument already carried confirmation.
    // On missing workspace errors the UI falls back to init instead of leaving
    // the operator in a dead command state.
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

    // Simple mode stages a low-ceremony draft and keeps validation attached to
    // the review step. Promotion later reuses that validation state so blocked
    // drafts remain inspectable through the same overlay.
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

    // Guided editor sessions share the same draft editor UI, but the owning
    // overlay decides where the user returns after save/promote/close.
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

    // The draft editor consumes text-editing keys locally and delegates only the
    // save/promote commands back to the caller. This keeps planning and
    // directions editor flows consistent while preserving separate persistence
    // actions.
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
