use super::super::planning_draft_editor_ui::{
    PlanningDraftEditorCloseRequest, PlanningDraftEditorCloseRisk,
};
use super::super::{
    ConversationInputEvent, DetailDocConfirmChoice, DirectionsMaintenanceOverlayStep,
    DirectionsMaintenanceProjectionKind, NativeTuiApp, PlanningInitDetailSelection,
    PlanningInitModeSelection, PlanningInitOverlayStep, PlanningInitRuntimeRefreshIntent,
    PlanningRuntimeRefreshOperation, PlanningRuntimeRefreshUiState,
    PlanningWorkspaceOperationUiSettlement, ShellChromeEvent, ShellOverlay,
};
use crate::application::service::planning::{PlanningDraftEditorSession, PlanningResetTarget};
use crate::core::app::{
    AppCommand, AppEvent, PlanningDoctorSnapshot, PlanningDoctorSnapshotState,
    PlanningEditorSessionSnapshot, PlanningSimpleDraftPromotionSnapshot,
    PlanningSimpleDraftStageSnapshot, PlanningWorkspaceOperationAdmission,
    PlanningWorkspaceOperationCorrelation, PlanningWorkspaceOperationKind,
    PlanningWorkspaceResetIntent, PlanningWorkspaceResetSnapshot, PlanningWorkspaceResetTarget,
};
use crossterm::event::{self, KeyCode, KeyModifiers};
type PlanningEditorSessionResult = anyhow::Result<PlanningDraftEditorSession>;

fn core_planning_reset_target(target: PlanningResetTarget) -> PlanningWorkspaceResetTarget {
    match target {
        PlanningResetTarget::Queue => PlanningWorkspaceResetTarget::Queue,
        PlanningResetTarget::Directions => PlanningWorkspaceResetTarget::Directions,
        PlanningResetTarget::All => PlanningWorkspaceResetTarget::All,
    }
}

mod directions_overlay;
mod editor;
mod planning_init_overlay;
mod status_text;
use self::status_text::{
    directions_manual_editor_close_warning_status, directions_manual_editor_closed_status,
    planning_doctor_status_text, planning_manual_editor_close_warning_status,
    planning_manual_editor_closed_status, planning_reset_preview_text, planning_reset_status_text,
};
use super::super::planning_overlay_shell_command::parse_planning_overlay_shell_argument;
use super::super::planning_reset_shell_command::{
    PLANNING_RESET_USAGE_TEXT, parse_planning_reset_shell_argument,
};
use super::super::planning_shell_command::{
    PLANNING_SHELL_USAGE_TEXT, ParsedPlanningShellCommand, parse_planning_shell_argument,
};

// Planning control is the TUI adapter layer for workspace mutations: it keeps
// shell overlays, editor state, and conversation status messages in sync while
// delegating filesystem authority to the planning application service.
impl NativeTuiApp {
    // Shell command handlers normalize user input before touching overlay state.
    // Unsupported arguments are surfaced as status rows so command mistakes do
    // not leave partial UI transitions behind.
    pub(in crate::adapter::inbound::tui::app) fn show_directions_maintenance_overlay(&mut self) {
        self.start_directions_maintenance_overview_load(Some(
            "opened directions maintenance".to_string(),
        ));
    }
    pub(in crate::adapter::inbound::tui::app) fn handle_directions_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        match parse_planning_overlay_shell_argument(argument) {
            Ok(()) => self.show_directions_maintenance_overlay(),
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "unsupported :directions argument `{}` / supported: :directions",
                        error.argument()
                    ),
                })
            }
        }
    }
    pub(in crate::adapter::inbound::tui::app) fn handle_queue_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        match parse_planning_overlay_shell_argument(argument) {
            Ok(()) => self.show_queue_overlay(),
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "`:queue` does not accept arguments (`{}`); use :queue to open queue inspection",
                        error.argument()
                    ),
                })
            }
        }
    }

    // The overlay opens immediately in Loading while Core reads DB/filesystem authority.
    // Correlation is bound before any background completion can be projected into the TUI.
    pub(in crate::adapter::inbound::tui::app) fn start_directions_maintenance_overview_load(
        &mut self,
        status_text: Option<String>,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::LoadDirectionsMaintenance {
                workspace_directory,
            });
        let correlation = outcome.events.iter().find_map(|event| match event {
            AppEvent::DirectionsMaintenanceLoadStarted { correlation } => Some(correlation.clone()),
            _ => None,
        });
        if let Some(correlation) = correlation {
            self.directions_maintenance_overlay_ui_state
                .begin_load(correlation);
        }
        if let Some(status_text) = status_text {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text,
            });
        }
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(in crate::adapter::inbound::tui::app) fn reconcile_directions_maintenance_context(
        &mut self,
    ) -> bool {
        if !self.directions_maintenance_load_required() {
            return false;
        }
        self.start_directions_maintenance_overview_load(None);
        true
    }

    // Planning init binds one Core-owned runtime refresh to the loading surface.
    // Only that accepted completion may choose setup versus existing-workspace controls.
    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn show_planning_init_overlay(&mut self) {
        self.begin_planning_init_overlay_refresh(PlanningInitRuntimeRefreshIntent::Inspect);
    }

    fn begin_planning_init_overlay_refresh(&mut self, intent: PlanningInitRuntimeRefreshIntent) {
        let workspace_directory = self.planning_workspace_directory();
        let Some((correlation, outcome)) =
            self.begin_planning_runtime_projection_refresh(&workspace_directory)
        else {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "planning setup unavailable while conversation is loading".to_string(),
            });
            return;
        };
        self.planning_init_overlay_ui_state.begin_runtime_refresh();
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "operator surface: planning setup / loading workspace".to_string(),
        });
        self.planning_runtime_refresh_ui_state.begin(
            correlation,
            PlanningRuntimeRefreshOperation::Init(intent),
            self.planning_ui_intent_revision,
        );
        // Bind the overlay generation before any immediate test executor completion is applied.
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_planning_runtime_refresh_completion(
        &mut self,
        operation: PlanningRuntimeRefreshOperation,
        result: Result<PlanningDoctorSnapshot, String>,
    ) {
        match operation {
            PlanningRuntimeRefreshOperation::Init(intent) => {
                self.apply_planning_init_runtime_refresh(intent, result)
            }
            PlanningRuntimeRefreshOperation::Doctor => self.apply_planning_doctor_refresh(result),
            PlanningRuntimeRefreshOperation::ResetRecovery { reset_error } => {
                self.apply_planning_reset_recovery_refresh(reset_error, result)
            }
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn discard_superseded_planning_runtime_refresh(
        &mut self,
        operation: PlanningRuntimeRefreshOperation,
    ) {
        if matches!(
            operation,
            PlanningRuntimeRefreshOperation::Init(_) | PlanningRuntimeRefreshOperation::Doctor
        ) && self.shell_overlay == ShellOverlay::PlanningInit
            && self.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::Loading
        {
            self.close_shell_overlay();
        }
    }

    fn apply_planning_init_runtime_refresh(
        &mut self,
        intent: PlanningInitRuntimeRefreshIntent,
        result: Result<PlanningDoctorSnapshot, String>,
    ) {
        let doctor = match result {
            Ok(doctor) => doctor,
            Err(error) => {
                if self.shell_overlay != ShellOverlay::PlanningInit {
                    return;
                }
                self.close_shell_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("planning setup unavailable: {error}"),
                });
                return;
            }
        };
        if self.shell_overlay != ShellOverlay::PlanningInit {
            return;
        }
        let Some(should_open_simple_review) = self
            .planning_init_overlay_ui_state
            .apply_runtime_refresh(doctor.workspace_present(), intent)
        else {
            return;
        };
        if should_open_simple_review {
            self.stage_simple_mode_planning_init_draft();
            return;
        }
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: if let Some(reason) = doctor.issue() {
                format!("planning setup unavailable: {reason}")
            } else if doctor.workspace_present() {
                "operator surface: planning setup / existing workspace".to_string()
            } else {
                "operator surface: planning setup / workspace: not initialized".to_string()
            },
        });
    }

    fn apply_planning_doctor_refresh(&mut self, result: Result<PlanningDoctorSnapshot, String>) {
        let doctor = match result {
            Ok(doctor) => doctor,
            Err(error) => {
                if self.shell_overlay == ShellOverlay::PlanningInit
                    && self.planning_init_overlay_ui_state.step()
                        == PlanningInitOverlayStep::Loading
                {
                    self.close_shell_overlay();
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!("planning doctor unavailable: {error}"),
                });
                return;
            }
        };
        if doctor.planning_state() == PlanningDoctorSnapshotState::Absent
            || self.shell_overlay == ShellOverlay::PlanningInit
        {
            self.open_planning_init_from_doctor(&doctor);
        }
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: planning_doctor_status_text(&doctor),
        });
    }

    fn apply_planning_reset_recovery_refresh(
        &mut self,
        reset_error: String,
        result: Result<PlanningDoctorSnapshot, String>,
    ) {
        let status_text = match result {
            Ok(doctor) if doctor.planning_state() == PlanningDoctorSnapshotState::Absent => {
                self.open_planning_init_from_doctor(&doctor);
                format!(
                    "planning reset failed: {reset_error} / planning workspace: missing / next action: open :planning to initialize it"
                )
            }
            Ok(doctor) => doctor.issue().map_or_else(
                || format!("planning reset failed: {reset_error}"),
                |issue| {
                    format!("planning reset failed: {reset_error} / workspace inspection: {issue}")
                },
            ),
            Err(inspection_error) => format!(
                "planning reset failed: {reset_error} / workspace inspection failed: {inspection_error}"
            ),
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    fn open_planning_init_from_doctor(&mut self, doctor: &PlanningDoctorSnapshot) {
        self.planning_draft_editor_ui_state.reset();
        self.planning_init_overlay_ui_state.begin_runtime_refresh();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        let _ = self.planning_init_overlay_ui_state.apply_runtime_refresh(
            doctor.workspace_present(),
            PlanningInitRuntimeRefreshIntent::Inspect,
        );
    }
    pub(in crate::adapter::inbound::tui::app) fn open_first_run_planning_simple_review(&mut self) {
        self.begin_planning_init_overlay_refresh(
            PlanningInitRuntimeRefreshIntent::OpenSimpleReviewWhenAbsent,
        );
    }

    // `:planning` is an overlay command first and a diagnostic command only
    // when explicitly asked for `doctor`; this keeps first-run setup discoverable
    // without making the happy path depend on a full inspection report.
    pub(in crate::adapter::inbound::tui::app) fn handle_planning_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        match parse_planning_shell_argument(argument) {
            Ok(ParsedPlanningShellCommand::OpenControlCenter) => {
                self.open_first_run_planning_simple_review();
            }
            Ok(ParsedPlanningShellCommand::Doctor) => self.run_planning_doctor(),
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "unsupported :planning argument `{}` / {}",
                        error.argument(),
                        PLANNING_SHELL_USAGE_TEXT
                    ),
                })
            }
        }
    }
    pub(in crate::adapter::inbound::tui::app) fn run_planning_doctor(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let Some((correlation, outcome)) =
            self.begin_planning_runtime_projection_refresh(&workspace_directory)
        else {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: "planning doctor unavailable while conversation is loading"
                    .to_string(),
            });
            return;
        };
        if self.shell_overlay == ShellOverlay::PlanningInit {
            self.planning_init_overlay_ui_state.begin_runtime_refresh();
        }
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "planning doctor: loading workspace".to_string(),
        });
        self.planning_runtime_refresh_ui_state.begin(
            correlation,
            PlanningRuntimeRefreshOperation::Doctor,
            self.planning_ui_intent_revision,
        );
        self.apply_core_dispatch_outcome(outcome);
    }
    // Reset is a destructive workspace operation, so directions/all resets use
    // a preview status unless the command argument already carried confirmation.
    // On missing workspace errors the UI falls back to planning setup instead
    // of leaving the operator in a dead command state.
    pub(in crate::adapter::inbound::tui::app) fn handle_reset_shell_command(
        &mut self,
        argument: Option<&str>,
    ) {
        let parsed = match parse_planning_reset_shell_argument(argument) {
            Ok(parsed) => parsed,
            Err(_) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: PLANNING_RESET_USAGE_TEXT.to_string(),
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
        let intent = PlanningWorkspaceResetIntent::new(
            self.planning_workspace_directory(),
            core_planning_reset_target(parsed.target),
        );
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::ResetPlanningWorkspace(intent));
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_planning_workspace_operation_admission(
        &mut self,
        admission: PlanningWorkspaceOperationAdmission,
    ) {
        match admission {
            PlanningWorkspaceOperationAdmission::Started { correlation } => {
                let status_text = match &correlation.operation {
                    PlanningWorkspaceOperationKind::Reset { target } => {
                        format!("planning reset in progress / target: {}", target.label())
                    }
                    PlanningWorkspaceOperationKind::StageSimpleDraft => {
                        "planning simple draft staging in progress".to_string()
                    }
                    PlanningWorkspaceOperationKind::LoadSimpleEditor { .. } => {
                        "planning simple draft editor loading".to_string()
                    }
                    PlanningWorkspaceOperationKind::PromoteSimpleDraft { .. } => {
                        "planning simple draft promotion in progress".to_string()
                    }
                };
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
                if !matches!(
                    &correlation.operation,
                    PlanningWorkspaceOperationKind::Reset { .. }
                ) && self.shell_overlay == ShellOverlay::PlanningInit
                {
                    self.planning_init_overlay_ui_state
                        .begin_simple_authoring_operation();
                }
                self.planning_workspace_operation_ui_state
                    .begin(correlation, self.planning_ui_intent_revision);
            }
            PlanningWorkspaceOperationAdmission::Coalesced { correlation } => {
                let is_reset = matches!(
                    &correlation.operation,
                    PlanningWorkspaceOperationKind::Reset { .. }
                );
                if !is_reset && self.shell_overlay == ShellOverlay::PlanningInit {
                    self.planning_init_overlay_ui_state
                        .begin_simple_authoring_operation();
                }
                if !is_reset
                    || self
                        .planning_workspace_operation_ui_state
                        .active_correlation()
                        != Some(&correlation)
                {
                    self.planning_workspace_operation_ui_state
                        .begin(correlation, self.planning_ui_intent_revision);
                }
            }
            PlanningWorkspaceOperationAdmission::Busy {
                active_correlation,
                requested,
            } => {
                let status_text = match (active_correlation.reset_target(), &requested.operation) {
                    (
                        Some(active_target),
                        PlanningWorkspaceOperationKind::Reset {
                            target: requested_target,
                        },
                    ) => format!(
                        "planning workspace busy / active reset: {} / requested reset: {} / wait for the active operation to finish",
                        active_target.label(),
                        requested_target.label(),
                    ),
                    (Some(active_target), _) => format!(
                        "planning workspace busy / reset {} is still in progress",
                        active_target.label()
                    ),
                    _ => format!(
                        "planning workspace busy / active: {} / requested: {} / wait for the active operation to finish",
                        active_correlation.label(),
                        requested.label(),
                    ),
                };
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_planning_workspace_reset_completion(
        &mut self,
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningWorkspaceResetSnapshot>, String>,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        let settlement = self.planning_workspace_operation_ui_state.settle(
            &correlation,
            &workspace_directory,
            self.planning_ui_intent_revision,
        );
        if matches!(
            settlement,
            PlanningWorkspaceOperationUiSettlement::Rejected
                | PlanningWorkspaceOperationUiSettlement::WorkspaceSuperseded
        ) {
            return;
        }
        let apply_presentation = settlement == PlanningWorkspaceOperationUiSettlement::Applied;
        self.pause_post_turn_continuation_after_authority_mutation();

        match result {
            Ok(result) => {
                self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
                    &workspace_directory,
                );
                if apply_presentation {
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: planning_reset_status_text(&result),
                    });
                }
            }
            Err(reset_error) => {
                if !apply_presentation {
                    self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
                        &workspace_directory,
                    );
                    return;
                }
                let Some((refresh_correlation, outcome)) =
                    self.begin_planning_runtime_projection_refresh(&workspace_directory)
                else {
                    self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                        status_text: format!(
                            "planning reset failed: {reset_error} / workspace inspection unavailable while conversation is loading"
                        ),
                    });
                    return;
                };
                self.planning_runtime_refresh_ui_state.begin(
                    refresh_correlation,
                    PlanningRuntimeRefreshOperation::ResetRecovery { reset_error },
                    self.planning_ui_intent_revision,
                );
                self.apply_core_dispatch_outcome(outcome);
            }
        }
    }

    pub(super) fn planning_workspace_operation_blocks_direct_mutation(&mut self) -> bool {
        let Some(active) = self
            .planning_workspace_operation_ui_state
            .active_correlation()
            .cloned()
        else {
            return false;
        };
        let status_text = active.reset_target().map_or_else(
            || {
                format!(
                    "planning workspace busy / {} is still in progress",
                    active.label()
                )
            },
            |target| {
                format!(
                    "planning workspace busy / reset {} is still in progress",
                    target.label()
                )
            },
        );
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
        true
    }

    // Simple mode stages a low-ceremony draft and keeps validation attached to
    // the review step. Promotion later reuses that validation state so blocked
    // drafts remain inspectable through the same overlay.
    pub(super) fn stage_simple_mode_planning_init_draft(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::StageSimplePlanningDraft {
                workspace_directory,
            });
        self.apply_core_dispatch_outcome(outcome);
    }
    pub(super) fn open_simple_mode_planning_editor(&mut self) {
        let Some((draft_name, source_session)) = self
            .planning_init_overlay_ui_state
            .simple_review()
            .map(|review| {
                (
                    review.draft_name().to_string(),
                    review.session_identity().clone(),
                )
            })
        else {
            return;
        };
        let workspace_directory = self.planning_workspace_directory();
        if source_session.workspace_directory != workspace_directory
            || source_session.draft_name != draft_name
        {
            return;
        }
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::LoadSimplePlanningEditor {
                workspace_directory,
                draft_name,
                source_session,
            });
        self.apply_core_dispatch_outcome(outcome);
    }
    pub(super) fn promote_simple_mode_planning_draft(&mut self) {
        let Some((draft_name, source_session)) = self
            .planning_init_overlay_ui_state
            .simple_review()
            .map(|review| {
                (
                    review.draft_name().to_string(),
                    review.session_identity().clone(),
                )
            })
        else {
            return;
        };
        let workspace_directory = self.planning_workspace_directory();
        if source_session.workspace_directory != workspace_directory
            || source_session.draft_name != draft_name
        {
            return;
        }
        let outcome = self
            .core_runtime
            .dispatch_command(AppCommand::PromoteSimplePlanningDraft {
                workspace_directory,
                draft_name,
                source_session,
            });
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_simple_planning_draft_stage_completion(
        &mut self,
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningSimpleDraftStageSnapshot>, String>,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        let settlement = self.planning_workspace_operation_ui_state.settle(
            &correlation,
            &workspace_directory,
            self.planning_ui_intent_revision,
        );
        match settlement {
            PlanningWorkspaceOperationUiSettlement::Applied => {}
            PlanningWorkspaceOperationUiSettlement::PresentationSuperseded => {
                if self.shell_overlay == ShellOverlay::PlanningInit
                    && self.planning_init_overlay_ui_state.step()
                        == PlanningInitOverlayStep::Loading
                    && !matches!(
                        &self.planning_runtime_refresh_ui_state,
                        PlanningRuntimeRefreshUiState::Loading { .. }
                    )
                {
                    self.planning_init_overlay_ui_state
                        .restore_simple_review_or_selection();
                }
                return;
            }
            PlanningWorkspaceOperationUiSettlement::WorkspaceSuperseded
            | PlanningWorkspaceOperationUiSettlement::Rejected => return,
        }

        let status_text = match result {
            Ok(stage_result) => {
                let validation_ok = stage_result.validation_report.is_valid();
                let draft_name = stage_result.session_identity.draft_name.clone();
                self.planning_init_overlay_ui_state
                    .open_simple_review_summary(
                        stage_result.session_identity,
                        draft_name.clone(),
                        stage_result.staged_file_count,
                        stage_result.validation_report,
                    );
                format!(
                    "planning simple review ready / staged draft: {} / validation state: {} / simple behavior: no queue head yet; queue-idle review stays enabled / next action: Enter or Ctrl+P promotes the low-ceremony scaffold. Ctrl+E inspects the draft. D opens detail-mode authoring.",
                    draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                )
            }
            Err(error) => {
                self.planning_init_overlay_ui_state
                    .restore_simple_review_or_selection();
                format!("planning init failed: {error}")
            }
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_simple_planning_editor_load_completion(
        &mut self,
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningEditorSessionSnapshot>, String>,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        let settlement = self.planning_workspace_operation_ui_state.settle(
            &correlation,
            &workspace_directory,
            self.planning_ui_intent_revision,
        );
        let source_is_current = correlation.source_session().is_some_and(|source| {
            self.planning_init_overlay_ui_state
                .simple_review()
                .is_some_and(|review| review.session_identity() == source)
        });
        match settlement {
            PlanningWorkspaceOperationUiSettlement::Applied => {}
            PlanningWorkspaceOperationUiSettlement::PresentationSuperseded => {
                if self.shell_overlay == ShellOverlay::PlanningInit
                    && source_is_current
                    && self.planning_init_overlay_ui_state.step()
                        == PlanningInitOverlayStep::Loading
                {
                    self.planning_init_overlay_ui_state
                        .restore_simple_review_or_selection();
                }
                return;
            }
            PlanningWorkspaceOperationUiSettlement::WorkspaceSuperseded
            | PlanningWorkspaceOperationUiSettlement::Rejected => return,
        }
        if !source_is_current {
            return;
        }
        if self
            .planning_draft_editor_ui_state
            .session_identity()
            .is_some_and(|active| active.generation >= correlation.generation)
        {
            return;
        }

        let status_text = match result {
            Ok(session) => {
                let validation_ok = session.validation_report.is_valid();
                let draft_name = session.session_identity.draft_name.clone();
                self.planning_draft_editor_ui_state
                    .open_correlated_session(*session);
                self.planning_init_overlay_ui_state.open_simple_editor();
                format!(
                    "planning simple draft editor ready / draft: {} / validation: {}",
                    draft_name,
                    if validation_ok {
                        "ok"
                    } else {
                        "needs attention"
                    }
                )
            }
            Err(error) => {
                self.planning_init_overlay_ui_state
                    .restore_simple_review_or_selection();
                format!("planning init failed: {error}")
            }
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_simple_planning_draft_promotion_completion(
        &mut self,
        correlation: PlanningWorkspaceOperationCorrelation,
        result: Result<Box<PlanningSimpleDraftPromotionSnapshot>, String>,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        let settlement = self.planning_workspace_operation_ui_state.settle(
            &correlation,
            &workspace_directory,
            self.planning_ui_intent_revision,
        );
        if matches!(
            settlement,
            PlanningWorkspaceOperationUiSettlement::Rejected
                | PlanningWorkspaceOperationUiSettlement::WorkspaceSuperseded
        ) {
            return;
        }

        self.pause_post_turn_continuation_after_authority_mutation();
        self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
            &workspace_directory,
        );
        if settlement == PlanningWorkspaceOperationUiSettlement::PresentationSuperseded
            && self.shell_overlay == ShellOverlay::PlanningInit
            && correlation.source_session().is_some_and(|source| {
                self.planning_init_overlay_ui_state
                    .simple_review()
                    .is_some_and(|review| review.session_identity() == source)
            })
            && self.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::Loading
        {
            self.planning_init_overlay_ui_state
                .restore_simple_review_or_selection();
        }
        if settlement != PlanningWorkspaceOperationUiSettlement::Applied
            || self.shell_overlay != ShellOverlay::PlanningInit
            || correlation.source_session().is_none_or(|source| {
                self.planning_init_overlay_ui_state
                    .simple_review()
                    .is_none_or(|review| review.session_identity() != source)
            })
        {
            return;
        }
        self.planning_init_overlay_ui_state
            .restore_simple_review_or_selection();

        let status_text = match result {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::{
        ConversationState, DirectionsMaintenanceScreenModel, NativeTuiParallelModeBinding,
        PendingResumedSessionPlanningRefresh, PlanningRuntimeRefreshUiState,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::core::app::{
        DirectionsMaintenanceDirectionSnapshot, DirectionsMaintenanceSummarySnapshot,
        DirectionsSupportingFileStatus,
    };
    use crate::domain::conversation::{
        ConversationControlSupport, ConversationSnapshot, ConversationTurnOptions,
    };
    use crate::domain::planning::{PlanningFileKind, PlanningValidationReport, QueueIdlePolicy};
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const CONTROLLER_RS: &str = include_str!("controller.rs");
    const DIRECTIONS_OVERLAY_RS: &str = include_str!("controller/directions_overlay.rs");
    const EDITOR_RS: &str = include_str!("controller/editor.rs");
    const PLANNING_INIT_OVERLAY_RS: &str = include_str!("controller/planning_init_overlay.rs");

    #[derive(Default)]
    struct FakeAppServerPort;

    impl StartupProbePort for FakeAppServerPort {
        fn load_startup_context(&self) -> anyhow::Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }
    }

    impl SessionCatalogPort for FakeAppServerPort {
        fn load_session_catalog(
            &self,
            _request: crate::domain::recent_sessions::SessionCatalogRequest,
        ) -> anyhow::Result<SessionCatalog> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }
    }

    impl InteractiveTurnRuntimePort for FakeAppServerPort {
        fn runtime_control_truth(
            &self,
        ) -> crate::domain::conversation::ConversationRuntimeControlTruth {
            crate::domain::conversation::ConversationRuntimeControlTruth::codex_app_server()
        }

        fn load_conversation_snapshot(
            &self,
            thread_id: &str,
        ) -> anyhow::Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            })
        }

        fn request_stop_all_sessions(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            cwd: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> anyhow::Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
                &event_sender,
                "test-thread",
                cwd,
            )
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> anyhow::Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
                &event_sender,
                thread_id,
                "/tmp/test-workspace",
            )
        }
    }

    fn make_test_app(workspace: &TempPlanningWorkspace) -> NativeTuiApp {
        make_test_app_with_planning_workspace_port(
            workspace,
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        )
    }

    fn make_test_app_with_planning_workspace_port(
        workspace: &TempPlanningWorkspace,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> NativeTuiApp {
        let codex_port = Arc::new(FakeAppServerPort);
        let planning = crate::adapter::inbound::tui::app::test_helpers::test_planning_services(
            planning_workspace_port,
        );
        let parallel_mode_control_plane_composition = ParallelModeControlPlaneComposition::new(
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            planning,
            Arc::new(NoopParallelAgentWorkerPort),
        );
        let parallel_mode_binding =
            NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
        let mut app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            parallel_mode_binding,
        );
        app.sync_draft_shell_workspace(workspace.path_str());
        app
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PlanningWorkspacePortFailure {
        LoadWorkspace,
        StageDraft,
        LoadDraft,
        ReplaceDraft,
        ReplaceWorkspace,
        SlowOptionalLoad,
    }

    struct PlanningTestGate {
        armed: Arc<AtomicBool>,
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl PlanningTestGate {
        fn wait_if_armed(&self) -> anyhow::Result<()> {
            if !self.armed.swap(false, Ordering::SeqCst) {
                return Ok(());
            }
            self.entered
                .send(())
                .map_err(|error| anyhow::anyhow!("test gate enter failed: {error}"))?;
            self.release
                .lock()
                .expect("test gate release should not be poisoned")
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| anyhow::anyhow!("test gate release failed: {error}"))
        }
    }

    struct PlanningTestGateControl {
        armed: Arc<AtomicBool>,
        entered: mpsc::Receiver<()>,
        release: mpsc::SyncSender<()>,
    }

    impl PlanningTestGateControl {
        fn arm(&self) {
            self.armed.store(true, Ordering::SeqCst);
        }

        fn wait_until_entered(&self) {
            self.entered
                .recv_timeout(Duration::from_secs(5))
                .expect("test worker should enter its gate");
        }

        fn release(&self) {
            self.release
                .send(())
                .expect("test worker gate should release");
        }
    }

    fn planning_test_gate(armed: bool) -> (PlanningTestGate, PlanningTestGateControl) {
        let armed = Arc::new(AtomicBool::new(armed));
        let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        (
            PlanningTestGate {
                armed: armed.clone(),
                entered: entered_sender,
                release: Mutex::new(release_receiver),
            },
            PlanningTestGateControl {
                armed,
                entered: entered_receiver,
                release: release_sender,
            },
        )
    }

    struct FailingPlanningWorkspacePort {
        inner: FilesystemPlanningWorkspaceAdapter,
        failure: PlanningWorkspacePortFailure,
        load_observation: Option<PlanningWorkspaceLoadObservation>,
        stage_gate: Option<PlanningTestGate>,
        load_gate: Option<PlanningTestGate>,
    }

    #[derive(Clone)]
    struct PlanningWorkspaceLoadObservation {
        load_count: Arc<AtomicUsize>,
        completed_load_count: Arc<AtomicUsize>,
        slow: Arc<AtomicBool>,
        fail: Arc<AtomicBool>,
        target_workspace: Arc<Mutex<Option<String>>>,
    }

    impl PlanningWorkspaceLoadObservation {
        fn new() -> Self {
            Self {
                load_count: Arc::new(AtomicUsize::new(0)),
                completed_load_count: Arc::new(AtomicUsize::new(0)),
                slow: Arc::new(AtomicBool::new(false)),
                fail: Arc::new(AtomicBool::new(false)),
                target_workspace: Arc::new(Mutex::new(None)),
            }
        }

        fn reset_and_enable(&self, workspace_directory: &str, fail: bool) {
            self.load_count.store(0, Ordering::SeqCst);
            self.completed_load_count.store(0, Ordering::SeqCst);
            self.fail.store(fail, Ordering::SeqCst);
            *self
                .target_workspace
                .lock()
                .expect("workspace observation target should not be poisoned") =
                Some(workspace_directory.to_string());
            self.slow.store(true, Ordering::SeqCst);
        }
    }

    impl FailingPlanningWorkspacePort {
        fn new(failure: PlanningWorkspacePortFailure) -> Self {
            Self {
                inner: FilesystemPlanningWorkspaceAdapter::new(),
                failure,
                load_observation: None,
                stage_gate: None,
                load_gate: None,
            }
        }

        fn observed() -> (Self, PlanningWorkspaceLoadObservation) {
            Self::observed_with_failure(PlanningWorkspacePortFailure::SlowOptionalLoad)
        }

        fn observed_with_failure(
            failure: PlanningWorkspacePortFailure,
        ) -> (Self, PlanningWorkspaceLoadObservation) {
            let observation = PlanningWorkspaceLoadObservation::new();
            (
                Self {
                    inner: FilesystemPlanningWorkspaceAdapter::new(),
                    failure,
                    load_observation: Some(observation.clone()),
                    stage_gate: None,
                    load_gate: None,
                },
                observation,
            )
        }

        fn gated_stage() -> (Self, PlanningTestGateControl, PlanningTestGateControl) {
            let (stage_gate, stage_control) = planning_test_gate(true);
            let (load_gate, load_control) = planning_test_gate(false);
            (
                Self {
                    inner: FilesystemPlanningWorkspaceAdapter::new(),
                    failure: PlanningWorkspacePortFailure::ReplaceDraft,
                    load_observation: None,
                    stage_gate: Some(stage_gate),
                    load_gate: Some(load_gate),
                },
                stage_control,
                load_control,
            )
        }

        fn fail_if(&self, failure: PlanningWorkspacePortFailure) -> anyhow::Result<()> {
            if self.failure == failure {
                anyhow::bail!("forced {failure:?} failure");
            }
            Ok(())
        }
    }

    impl PlanningWorkspacePort for FailingPlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            workspace_dir: &str,
            draft_name: &str,
            files: &[PlanningDraftFileRecord],
        ) -> anyhow::Result<PlanningDraftStageRecord> {
            self.fail_if(PlanningWorkspacePortFailure::StageDraft)?;
            if let Some(gate) = &self.stage_gate {
                gate.wait_if_armed()?;
            }
            self.inner
                .stage_planning_draft_files(workspace_dir, draft_name, files)
        }

        fn load_planning_draft_files(
            &self,
            workspace_dir: &str,
            draft_name: &str,
        ) -> anyhow::Result<PlanningDraftLoadRecord> {
            self.fail_if(PlanningWorkspacePortFailure::LoadDraft)?;
            self.inner
                .load_planning_draft_files(workspace_dir, draft_name)
        }

        fn replace_planning_draft_file(
            &self,
            workspace_dir: &str,
            draft_name: &str,
            active_path: &str,
            body: &str,
        ) -> anyhow::Result<String> {
            self.fail_if(PlanningWorkspacePortFailure::ReplaceDraft)?;
            self.inner
                .replace_planning_draft_file(workspace_dir, draft_name, active_path, body)
        }

        fn load_planning_workspace_files(
            &self,
            workspace_dir: &str,
        ) -> anyhow::Result<PlanningWorkspaceLoadRecord> {
            if let Some(gate) = &self.load_gate {
                gate.wait_if_armed()?;
            }
            let observation = self.load_observation.as_ref().filter(|observation| {
                observation
                    .target_workspace
                    .lock()
                    .expect("workspace observation target should not be poisoned")
                    .as_deref()
                    .is_none_or(|target| target == workspace_dir)
            });
            if let Some(observation) = observation {
                observation.load_count.fetch_add(1, Ordering::SeqCst);
                if observation.slow.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(600));
                }
                if observation.fail.load(Ordering::SeqCst) {
                    observation
                        .completed_load_count
                        .fetch_add(1, Ordering::SeqCst);
                    anyhow::bail!("forced observed workspace inspection failure");
                }
            }
            let result = self
                .fail_if(PlanningWorkspacePortFailure::LoadWorkspace)
                .and_then(|()| self.inner.load_planning_workspace_files(workspace_dir));
            if let Some(observation) = observation {
                observation
                    .completed_load_count
                    .fetch_add(1, Ordering::SeqCst);
            }
            result
        }

        fn load_planning_workspace_candidate_files(
            &self,
            workspace_dir: &str,
        ) -> anyhow::Result<PlanningWorkspaceLoadRecord> {
            self.fail_if(PlanningWorkspacePortFailure::LoadWorkspace)?;
            self.inner
                .load_planning_workspace_candidate_files(workspace_dir)
        }

        fn commit_planning_workspace_files(
            &self,
            workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> anyhow::Result<()> {
            self.fail_if(PlanningWorkspacePortFailure::ReplaceWorkspace)?;
            self.inner
                .commit_planning_workspace_files(workspace_dir, record)
        }

        fn load_optional_planning_file(
            &self,
            workspace_dir: &str,
            relative_path: &str,
        ) -> anyhow::Result<Option<String>> {
            if self.failure == PlanningWorkspacePortFailure::SlowOptionalLoad {
                std::thread::sleep(Duration::from_millis(600));
            }
            self.inner
                .load_optional_planning_file(workspace_dir, relative_path)
        }

        fn load_optional_planning_candidate_file(
            &self,
            workspace_dir: &str,
            relative_path: &str,
        ) -> anyhow::Result<Option<String>> {
            self.inner
                .load_optional_planning_candidate_file(workspace_dir, relative_path)
        }

        fn replace_planning_workspace_file(
            &self,
            workspace_dir: &str,
            relative_path: &str,
            body: Option<&str>,
        ) -> anyhow::Result<()> {
            self.fail_if(PlanningWorkspacePortFailure::ReplaceWorkspace)?;
            self.inner
                .replace_planning_workspace_file(workspace_dir, relative_path, body)
        }

        fn remove_planning_workspace_entry(
            &self,
            workspace_dir: &str,
            relative_path: &str,
        ) -> anyhow::Result<()> {
            self.inner
                .remove_planning_workspace_entry(workspace_dir, relative_path)
        }

        fn archive_rejected_planning_file(
            &self,
            workspace_dir: &str,
            archive_name: &str,
            active_path: &str,
            body: &str,
        ) -> anyhow::Result<String> {
            self.inner.archive_rejected_planning_file(
                workspace_dir,
                archive_name,
                active_path,
                body,
            )
        }
    }

    fn ready_status(app: &NativeTuiApp) -> &str {
        match &app.conversation_state {
            ConversationState::Ready(conversation) => conversation.status_text.as_str(),
            other => panic!("conversation should be ready, got {other:?}"),
        }
    }

    fn wait_for_planning_init_refresh(app: &mut NativeTuiApp) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            app.poll_core_runtime_inputs(16);
            if app.planning_init_overlay_ui_state.step() != PlanningInitOverlayStep::Loading {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("planning init runtime refresh should complete");
    }

    fn wait_for_planning_runtime_refresh(app: &mut NativeTuiApp) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            app.poll_core_runtime_inputs(16);
            if !matches!(
                app.planning_runtime_refresh_ui_state,
                PlanningRuntimeRefreshUiState::Loading { .. }
            ) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("planning runtime refresh should complete");
    }

    fn wait_for_planning_workspace_operation(app: &mut NativeTuiApp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.poll_core_runtime_inputs(16);
            if app
                .planning_workspace_operation_ui_state
                .active_correlation()
                .is_none()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("planning workspace operation should complete");
    }

    fn begin_gated_simple_stage(
        workspace_name: &str,
    ) -> (
        TempPlanningWorkspace,
        NativeTuiApp,
        PlanningWorkspaceOperationCorrelation,
        PlanningTestGateControl,
        PlanningTestGateControl,
    ) {
        let workspace = TempPlanningWorkspace::new(workspace_name);
        let (port, stage_gate, load_gate) = FailingPlanningWorkspacePort::gated_stage();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        app.stage_simple_mode_planning_init_draft();
        stage_gate.wait_until_entered();
        let correlation = app
            .planning_workspace_operation_ui_state
            .active_correlation()
            .cloned()
            .expect("gated simple stage should remain active");
        assert!(matches!(
            &correlation.operation,
            PlanningWorkspaceOperationKind::StageSimpleDraft
        ));
        (workspace, app, correlation, stage_gate, load_gate)
    }

    fn wait_for_observed_loads_to_settle(
        app: &mut NativeTuiApp,
        observation: &PlanningWorkspaceLoadObservation,
    ) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut last_count = observation.load_count.load(Ordering::SeqCst);
        let mut stable_since = Instant::now();
        while Instant::now() < deadline {
            app.poll_core_runtime_inputs(16);
            let count = observation.load_count.load(Ordering::SeqCst);
            if count != last_count {
                last_count = count;
                stable_since = Instant::now();
            }
            if count > 0 && stable_since.elapsed() >= Duration::from_millis(300) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("initial planning runtime loads should settle");
    }

    fn wait_for_observed_load_completions(
        app: &mut NativeTuiApp,
        observation: &PlanningWorkspaceLoadObservation,
        expected: usize,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut completed_at = None;
        while Instant::now() < deadline {
            app.poll_core_runtime_inputs(16);
            if observation.completed_load_count.load(Ordering::SeqCst) >= expected {
                let completed_at = completed_at.get_or_insert_with(Instant::now);
                if completed_at.elapsed() >= Duration::from_millis(25) {
                    app.poll_core_runtime_inputs(16);
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("expected {expected} planning workspace load completions");
    }

    fn post_turn_continuation_is_paused(app: &NativeTuiApp) -> bool {
        match &app.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .auto_follow_state
                .post_turn_continuation_paused(),
            ConversationState::Loading | ConversationState::Failed(_) => false,
        }
    }

    fn wait_for_directions_maintenance_load(app: &mut NativeTuiApp) {
        // The non-blocking assertion is made before this wait. Give the worker
        // enough settlement time under the full suite's process-heavy load.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.poll_core_runtime_inputs(16);
            if app
                .directions_maintenance_overlay_ui_state
                .projection_kind()
                != DirectionsMaintenanceProjectionKind::Loading
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("directions maintenance load should complete");
    }

    fn key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn shift_key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    struct TempPlanningWorkspace {
        path: PathBuf,
        path_text: String,
    }

    impl TempPlanningWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp planning workspace should be created");
            let path_text = path.display().to_string();
            Self { path, path_text }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn path_str(&self) -> &str {
            &self.path_text
        }
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn occurrence_count(source: &str, needle: &str) -> usize {
        source.match_indices(needle).count()
    }

    fn directions_summary(
        detail_doc_status: DirectionsSupportingFileStatus,
        parse_error: Option<&str>,
    ) -> DirectionsMaintenanceSummarySnapshot {
        DirectionsMaintenanceSummarySnapshot {
            directions: vec![DirectionsMaintenanceDirectionSnapshot {
                id: "general-workstream".to_string(),
                title: "General workstream".to_string(),
                detail_doc_path: Some(
                    crate::application::service::planning::default_direction_detail_doc_path(
                        "general-workstream",
                    ),
                ),
                detail_doc_status,
            }],
            missing_detail_doc_count: usize::from(
                detail_doc_status == DirectionsSupportingFileStatus::MissingMapping,
            ),
            broken_detail_doc_count: usize::from(
                detail_doc_status == DirectionsSupportingFileStatus::BrokenMapping,
            ),
            queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
            queue_idle_prompt_path: Some(
                crate::application::service::planning::DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
                    .to_string(),
            ),
            queue_idle_prompt_status: DirectionsSupportingFileStatus::Ready,
            parse_error: parse_error.map(str::to_string),
        }
    }

    fn staged_draft_file_path(
        workspace: &TempPlanningWorkspace,
        draft_name: &str,
        active_path: &str,
    ) -> PathBuf {
        let draft_relative_path = active_path
            .strip_prefix(".codex-exec-loop/planning/")
            .unwrap_or(active_path);
        workspace
            .path()
            .join(crate::application::service::planning::PLANNING_DRAFTS_DIRECTORY)
            .join(draft_name)
            .join(draft_relative_path)
    }

    #[test]
    fn controller_fixture_ports_cover_boundary_methods() {
        let codex_port = FakeAppServerPort;

        let startup = codex_port
            .load_startup_context()
            .expect("startup context should load");
        assert!(startup.account_ok);

        let catalog = codex_port
            .load_session_catalog(SessionCatalogRequest::for_workspace(5, "/tmp/root"))
            .expect("session catalog should load");
        assert_eq!(
            catalog
                .recent_sessions()
                .expect("catalog should be ready")
                .items
                .len(),
            0
        );

        let truth = codex_port.runtime_control_truth();
        assert_eq!(truth.approval, ConversationControlSupport::RuntimeNative);
        assert_eq!(truth.interrupt, ConversationControlSupport::RuntimeNative);

        let snapshot = codex_port
            .load_conversation_snapshot("thread-fixture")
            .expect("snapshot should load");
        assert_eq!(snapshot.thread_id, "thread-fixture");
        codex_port
            .request_stop_all_sessions()
            .expect("stop should be accepted");
        let (new_thread_sender, _new_thread_receiver) =
            crate::application::service::conversation_runtime_event::conversation_stream_channel();
        codex_port
            .run_new_thread_stream(
                "/tmp/root",
                "prompt",
                ConversationTurnOptions::default(),
                new_thread_sender,
            )
            .expect("new thread stream should be accepted");
        let (turn_sender, _turn_receiver) =
            crate::application::service::conversation_runtime_event::conversation_stream_channel();
        codex_port
            .run_turn_stream(
                "thread-fixture",
                "prompt",
                ConversationTurnOptions::default(),
                turn_sender,
            )
            .expect("turn stream should be accepted");

        let workspace = TempPlanningWorkspace::new("tui-fixture-port-coverage");
        let planning_port =
            FailingPlanningWorkspacePort::new(PlanningWorkspacePortFailure::StageDraft);
        let candidate_record = planning_port
            .load_planning_workspace_candidate_files(workspace.path_str())
            .expect("candidate workspace should load");
        planning_port
            .commit_planning_workspace_files(workspace.path_str(), &candidate_record)
            .expect("candidate workspace should commit");
        assert_eq!(
            planning_port
                .load_optional_planning_candidate_file(workspace.path_str(), "missing.md")
                .expect("candidate optional read should succeed"),
            None
        );
        planning_port
            .remove_planning_workspace_entry(workspace.path_str(), "missing.md")
            .expect("missing workspace entry removal should succeed");
        let archived_path = planning_port
            .archive_rejected_planning_file(
                workspace.path_str(),
                "rejected-fixture",
                crate::application::service::planning::RESULT_OUTPUT_FILE_PATH,
                "# rejected\n",
            )
            .expect("rejected planning file should archive");
        assert!(Path::new(&archived_path).is_file());
    }

    #[test]
    fn planning_shell_commands_route_statuses_and_doctor_paths() {
        let workspace = TempPlanningWorkspace::new("tui-planning-shell-command");
        let mut app = make_test_app(&workspace);

        app.handle_directions_shell_command(Some("now"));
        assert_eq!(
            ready_status(&app),
            "unsupported :directions argument `now` / supported: :directions"
        );

        app.handle_queue_shell_command(Some("later"));
        assert_eq!(
            ready_status(&app),
            "`:queue` does not accept arguments (`later`); use :queue to open queue inspection"
        );

        app.handle_planning_shell_command(Some("status"));
        assert_eq!(
            ready_status(&app),
            "unsupported :planning argument `status` / supported: :planning, :planning doctor, :doctor"
        );

        fs::remove_dir_all(workspace.path()).expect("seeded planning fixture should be removable");
        fs::create_dir_all(workspace.path()).expect("planning fixture should be recreated");
        app.handle_planning_shell_command(None);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::Loading
        );
        assert!(app.planning_init_overlay_ui_state.simple_review().is_none());
        wait_for_planning_init_refresh(&mut app);
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert!(ready_status(&app).contains("planning simple review ready / staged draft: "));

        let existing_workspace = TempPlanningWorkspace::new("tui-planning-shell-existing");
        let mut existing_app = make_test_app(&existing_workspace);
        existing_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut existing_app);
        existing_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut existing_app);
        assert_eq!(existing_app.shell_overlay, ShellOverlay::Hidden);

        existing_app.handle_planning_shell_command(None);
        wait_for_planning_init_refresh(&mut existing_app);
        assert_eq!(existing_app.shell_overlay, ShellOverlay::PlanningInit);
        assert_eq!(
            existing_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ExistingWorkspace
        );
        assert_eq!(
            ready_status(&existing_app),
            "operator surface: planning setup / existing workspace"
        );

        existing_app.run_planning_doctor();
        assert_eq!(
            existing_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::Loading
        );
        wait_for_planning_init_refresh(&mut existing_app);
        assert_eq!(
            existing_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ExistingWorkspace
        );
        assert!(ready_status(&existing_app).starts_with("planning state: "));

        let absent_doctor_workspace = TempPlanningWorkspace::new("tui-planning-doctor-absent");
        let mut absent_doctor_app = make_test_app(&absent_doctor_workspace);
        absent_doctor_app.handle_planning_shell_command(Some("doctor"));
        assert!(matches!(
            absent_doctor_app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Loading { .. }
        ));
        wait_for_planning_runtime_refresh(&mut absent_doctor_app);
        assert_eq!(absent_doctor_app.shell_overlay, ShellOverlay::PlanningInit);
        assert!(ready_status(&absent_doctor_app).starts_with("planning state: absent"));
    }

    #[test]
    fn overlay_shell_open_paths_present_planning_surfaces() {
        let workspace = TempPlanningWorkspace::new("tui-overlay-shell-open-paths");
        let mut app = make_test_app(&workspace);

        app.handle_directions_shell_command(None);
        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(ready_status(&app), "opened directions maintenance");
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Loading
        );
        wait_for_directions_maintenance_load(&mut app);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Ready
        );

        app.handle_queue_shell_command(None);
        assert_eq!(app.shell_overlay, ShellOverlay::Queue);
    }

    #[test]
    fn directions_maintenance_open_does_not_wait_for_workspace_io() {
        let workspace = TempPlanningWorkspace::new("tui-directions-non-blocking");
        let mut app = make_test_app_with_planning_workspace_port(
            &workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::SlowOptionalLoad,
            )),
        );

        let started_at = Instant::now();
        app.show_directions_maintenance_overlay();
        let elapsed = started_at.elapsed();

        assert!(
            elapsed < Duration::from_millis(300),
            "overlay open waited for workspace I/O: {elapsed:?}"
        );
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Loading
        );
        wait_for_directions_maintenance_load(&mut app);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Ready
        );
    }

    #[test]
    fn planning_doctor_dispatch_is_non_blocking_and_reads_absent_workspace_once() {
        let workspace = TempPlanningWorkspace::new("tui-planning-doctor-non-blocking");
        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);

        let started_at = Instant::now();
        app.run_planning_doctor();
        let elapsed = started_at.elapsed();

        assert!(
            elapsed < Duration::from_millis(300),
            "doctor dispatch waited for 600ms workspace I/O: {elapsed:?}"
        );
        assert!(matches!(
            &app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Loading { .. }
        ));
        wait_for_planning_runtime_refresh(&mut app);
        assert_eq!(observation.load_count.load(Ordering::SeqCst), 1);
        assert!(
            matches!(
                app.planning_runtime_refresh_ui_state,
                PlanningRuntimeRefreshUiState::Ready { .. }
            ),
            "state: {:?}",
            app.planning_runtime_refresh_ui_state
        );
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
        assert!(ready_status(&app).starts_with("planning state: absent"));
    }

    #[test]
    fn planning_doctor_reads_a_present_workspace_once() {
        let workspace = TempPlanningWorkspace::new("tui-planning-doctor-present-one-read");
        FilesystemPlanningWorkspaceAdapter::new()
            .replace_planning_workspace_file(
                workspace.path_str(),
                crate::application::service::planning::RESULT_OUTPUT_FILE_PATH,
                Some("# Result Output\n\n- Report completed work.\n"),
            )
            .expect("present planning workspace should be seeded");
        let (port, observation) = FailingPlanningWorkspacePort::observed_with_failure(
            PlanningWorkspacePortFailure::LoadDraft,
        );
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);

        let started_at = Instant::now();
        app.run_planning_doctor();
        let elapsed = started_at.elapsed();

        assert!(
            elapsed < Duration::from_millis(300),
            "present-workspace doctor dispatch waited for 600ms I/O: {elapsed:?}"
        );
        wait_for_planning_runtime_refresh(&mut app);
        assert_eq!(observation.load_count.load(Ordering::SeqCst), 1);
        assert!(
            matches!(
                app.planning_runtime_refresh_ui_state,
                PlanningRuntimeRefreshUiState::Ready { .. }
            ),
            "state: {:?}",
            app.planning_runtime_refresh_ui_state
        );
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert!(ready_status(&app).starts_with("planning state: ready_"));
    }

    #[test]
    fn planning_projection_writer_restarts_doctor_and_stale_success_cannot_complete_it() {
        let workspace = TempPlanningWorkspace::new("tui-planning-doctor-writer-race");
        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);

        app.run_planning_doctor();
        let first_generation = match &app.planning_runtime_refresh_ui_state {
            PlanningRuntimeRefreshUiState::Loading { correlation, .. } => correlation.generation,
            state => panic!("doctor should start loading, got {state:?}"),
        };
        app.sync_core_planning_runtime_projection(
            crate::application::service::planning::PlanningRuntimeProjection::invalid(
                "unrelated writer",
            ),
        );

        let replacement_generation = match &app.planning_runtime_refresh_ui_state {
            PlanningRuntimeRefreshUiState::Loading { correlation, .. } => correlation.generation,
            state => panic!("writer should rebind the doctor inspection, got {state:?}"),
        };
        assert!(replacement_generation > first_generation);
        assert_eq!(ready_status(&app), "planning doctor: loading workspace");
        wait_for_planning_runtime_refresh(&mut app);
        assert_eq!(observation.load_count.load(Ordering::SeqCst), 2);
        assert!(matches!(
            app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Ready {
                ref correlation,
                ..
            } if correlation.generation == replacement_generation
        ));
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
        assert!(ready_status(&app).starts_with("planning state: absent"));
    }

    #[test]
    fn planning_projection_writer_restart_preserves_newer_doctor_status() {
        for (suffix, fail) in [("success", false), ("failure", true)] {
            let workspace =
                TempPlanningWorkspace::new(&format!("tui-planning-doctor-writer-drift-{suffix}"));
            let (port, observation) = FailingPlanningWorkspacePort::observed();
            let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
            wait_for_observed_loads_to_settle(&mut app, &observation);
            observation.reset_and_enable(workspace.path_str(), fail);

            app.run_planning_doctor();
            let (first_generation, operation_revision) =
                match &app.planning_runtime_refresh_ui_state {
                    PlanningRuntimeRefreshUiState::Loading {
                        correlation,
                        presentation_revision,
                        ..
                    } => (correlation.generation, *presentation_revision),
                    state => panic!("doctor should start loading, got {state:?}"),
                };
            let status_text = format!("newer operator status after {suffix}");
            app.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: status_text.clone(),
            });
            assert_ne!(app.planning_ui_intent_revision, operation_revision);

            app.sync_core_planning_runtime_projection(
                crate::application::service::planning::PlanningRuntimeProjection::invalid(
                    "same-workspace writer",
                ),
            );
            assert!(matches!(
                app.planning_runtime_refresh_ui_state,
                PlanningRuntimeRefreshUiState::Loading {
                    ref correlation,
                    presentation_revision,
                    ..
                } if correlation.generation > first_generation
                    && presentation_revision == operation_revision
            ));

            wait_for_planning_runtime_refresh(&mut app);

            assert_eq!(observation.load_count.load(Ordering::SeqCst), 2);
            assert!(matches!(
                app.planning_runtime_refresh_ui_state,
                PlanningRuntimeRefreshUiState::Idle
            ));
            assert_eq!(ready_status(&app), status_text);
            assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        }
    }

    #[test]
    fn planning_doctor_failure_preserves_newer_status_after_background_inspection() {
        let workspace = TempPlanningWorkspace::new("tui-planning-doctor-failure");
        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), true);

        app.run_planning_doctor();
        app.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "newer unrelated status".to_string(),
        });
        wait_for_planning_runtime_refresh(&mut app);

        assert_eq!(observation.load_count.load(Ordering::SeqCst), 1);
        assert!(matches!(
            app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Idle
        ));
        assert_eq!(ready_status(&app), "newer unrelated status");
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn planning_doctor_completion_preserves_a_newer_overlay_intent() {
        let workspace = TempPlanningWorkspace::new("tui-planning-doctor-overlay-intent");
        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);

        app.run_planning_doctor();
        app.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);
        wait_for_planning_runtime_refresh(&mut app);

        assert!(matches!(
            app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Idle
        ));
        assert_eq!(app.shell_overlay, ShellOverlay::Queue);
    }

    #[test]
    fn planning_init_loading_blocks_keys_and_reopen_accepts_only_the_latest_refresh() {
        let workspace = TempPlanningWorkspace::new("tui-planning-init-loading");
        let mut app = make_test_app(&workspace);
        app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut app);
        app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut app);

        app.show_planning_init_overlay();

        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::Loading
        );
        assert_eq!(
            ready_status(&app),
            "operator surface: planning setup / loading workspace"
        );
        for code in [KeyCode::Enter, KeyCode::Char('d'), KeyCode::Char('q')] {
            assert!(app.handle_planning_init_overlay_key(key(code)));
            assert_eq!(
                app.planning_init_overlay_ui_state.step(),
                PlanningInitOverlayStep::Loading
            );
        }

        app.close_shell_overlay();
        assert!(matches!(
            app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Idle
        ));
        app.show_planning_init_overlay();
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::Loading
        );
        wait_for_planning_init_refresh(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ExistingWorkspace
        );

        app.close_shell_overlay();
        let drain_deadline = std::time::Instant::now() + Duration::from_millis(100);
        while std::time::Instant::now() < drain_deadline {
            app.poll_core_runtime_inputs(16);
            std::thread::sleep(Duration::from_millis(2));
        }

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ModeSelection
        );
    }

    #[test]
    fn planning_init_completion_closes_loading_overlay_after_workspace_drift() {
        let workspace = TempPlanningWorkspace::new("tui-planning-init-workspace-drift");
        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);
        app.show_planning_init_overlay();

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should have a ready conversation");
        };
        conversation.cwd = "/tmp/replacement-planning-workspace".to_string();
        conversation.draft_workspace_directory = "/tmp/replacement-planning-workspace".to_string();

        wait_for_planning_runtime_refresh(&mut app);

        assert!(matches!(
            app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Idle
        ));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            ready_status(&app),
            "planning setup closed because its workspace context changed"
        );
    }

    #[test]
    fn planning_init_completion_preserves_a_newer_status_intent() {
        let workspace = TempPlanningWorkspace::new("tui-planning-init-status-intent");
        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);
        app.show_planning_init_overlay();
        app.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "newer operator status".to_string(),
        });

        wait_for_planning_runtime_refresh(&mut app);

        assert!(matches!(
            app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Idle
        ));
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(ready_status(&app), "newer operator status");
    }

    #[test]
    fn resumed_session_refresh_does_not_replace_a_newer_status() {
        let workspace = TempPlanningWorkspace::new("tui-resume-refresh-status-gate");
        let mut app = make_test_app(&workspace);
        let (thread_id, status_text) = match &app.conversation_state {
            ConversationState::Ready(conversation) => (
                conversation.thread_id.clone(),
                conversation.status_text.clone(),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => {
                panic!("test conversation should be ready")
            }
        };
        let pending = PendingResumedSessionPlanningRefresh {
            correlation: crate::core::app::PlanningRuntimeRefreshCorrelation::new(
                1,
                app.planning_workspace_directory(),
            ),
            thread_id,
            status_text,
        };
        app.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "newer operator status".to_string(),
        });

        app.surface_resumed_session_planning_context_if_unchanged(&pending);

        assert_eq!(ready_status(&app), "newer operator status");
    }

    #[test]
    fn reset_shell_command_handles_usage_preview_success_and_fallbacks() {
        let workspace = TempPlanningWorkspace::new("tui-reset-command-missing");
        let mut app = make_test_app(&workspace);

        app.handle_reset_shell_command(Some("queue now"));
        assert_eq!(ready_status(&app), PLANNING_RESET_USAGE_TEXT);

        app.handle_reset_shell_command(Some("directions"));
        assert_eq!(
            ready_status(&app),
            "reset directions preview: rewrites DB direction authority, recreates the default queue-idle prompt, removes direction detail docs and prompt artifacts, and clears derived queue state / rerun `:reset directions confirm` to continue"
        );

        app.handle_reset_shell_command(Some("all"));
        assert!(ready_status(&app).starts_with("reset all preview:"));

        fs::remove_dir_all(workspace.path()).expect("seeded planning fixture should be removable");
        fs::create_dir_all(workspace.path()).expect("planning fixture should be recreated");
        app.handle_reset_shell_command(Some("queue"));
        assert_eq!(
            ready_status(&app),
            "planning reset in progress / target: queue"
        );
        wait_for_planning_workspace_operation(&mut app);
        wait_for_planning_runtime_refresh(&mut app);
        assert!(ready_status(&app).contains("planning reset failed:"));
        assert!(ready_status(&app).contains("planning workspace: missing"));
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);

        let success_workspace = TempPlanningWorkspace::new("tui-reset-command-success");
        let mut success_app = make_test_app(&success_workspace);
        success_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut success_app);
        success_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut success_app);
        success_app.handle_reset_shell_command(Some("queue"));
        assert_eq!(
            ready_status(&success_app),
            "planning reset in progress / target: queue"
        );
        wait_for_planning_workspace_operation(&mut success_app);
        assert_eq!(
            ready_status(&success_app),
            "planning reset applied / target: queue / rewritten: 0 / removed: 0"
        );
        success_app.handle_reset_shell_command(Some("directions confirm"));
        wait_for_planning_workspace_operation(&mut success_app);
        assert!(
            ready_status(&success_app).starts_with("planning reset applied / target: directions")
        );

        let failure_workspace = TempPlanningWorkspace::new("tui-reset-command-failure");
        let mut seed_app = make_test_app(&failure_workspace);
        seed_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut seed_app);
        seed_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut seed_app);
        let (failure_port, observation) = FailingPlanningWorkspacePort::observed_with_failure(
            PlanningWorkspacePortFailure::ReplaceWorkspace,
        );
        let mut failure_app =
            make_test_app_with_planning_workspace_port(&failure_workspace, Arc::new(failure_port));
        wait_for_observed_loads_to_settle(&mut failure_app, &observation);
        observation.reset_and_enable(failure_workspace.path_str(), true);

        failure_app.handle_reset_shell_command(Some("all confirm"));
        wait_for_planning_workspace_operation(&mut failure_app);
        wait_for_planning_runtime_refresh(&mut failure_app);
        assert!(
            ready_status(&failure_app).starts_with("planning reset failed: forced "),
            "status: {}",
            ready_status(&failure_app)
        );
        assert!(
            ready_status(&failure_app).contains(
                "workspace inspection failed: forced observed workspace inspection failure"
            )
        );
        assert_eq!(failure_app.shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn successful_reset_refreshes_runtime_under_newer_busy_status_without_blocking_dispatch() {
        let workspace = TempPlanningWorkspace::new("tui-reset-non-blocking-busy");
        let mut seed_app = make_test_app(&workspace);
        seed_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut seed_app);
        seed_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut seed_app);

        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);

        let started_at = Instant::now();
        app.handle_reset_shell_command(Some("queue"));
        let elapsed = started_at.elapsed();
        let first = app
            .planning_workspace_operation_ui_state
            .active_correlation()
            .cloned()
            .expect("reset should remain active while workspace I/O is slow");
        assert!(
            elapsed < Duration::from_millis(300),
            "reset dispatch waited for 600ms workspace I/O: {elapsed:?}"
        );

        app.handle_reset_shell_command(Some("queue"));
        assert_eq!(
            app.planning_workspace_operation_ui_state
                .active_correlation(),
            Some(&first)
        );
        app.handle_reset_shell_command(Some("all confirm"));
        assert!(ready_status(&app).starts_with("planning workspace busy / active reset: queue"));

        app.stage_simple_mode_planning_init_draft();
        assert_eq!(
            ready_status(&app),
            "planning workspace busy / reset queue is still in progress"
        );
        wait_for_planning_workspace_operation(&mut app);
        wait_for_observed_load_completions(&mut app, &observation, 2);

        assert_eq!(observation.load_count.load(Ordering::SeqCst), 2);
        assert!(
            app.planning_workspace_operation_ui_state
                .active_correlation()
                .is_none()
        );
        assert!(post_turn_continuation_is_paused(&app));
        assert_eq!(
            ready_status(&app),
            "planning workspace busy / reset queue is still in progress",
            "late reset completion must refresh authority state without overwriting the newer busy presentation"
        );
    }

    #[test]
    fn failed_reset_refreshes_runtime_and_preserves_newer_status_presentation() {
        let workspace = TempPlanningWorkspace::new("tui-reset-failure-newer-status");
        let mut seed_app = make_test_app(&workspace);
        seed_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut seed_app);
        seed_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut seed_app);

        let (port, observation) = FailingPlanningWorkspacePort::observed_with_failure(
            PlanningWorkspacePortFailure::ReplaceWorkspace,
        );
        let mut app = make_test_app_with_planning_workspace_port(&workspace, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace.path_str(), false);

        app.handle_reset_shell_command(Some("queue"));
        app.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "newer operator status".to_string(),
        });
        wait_for_planning_workspace_operation(&mut app);
        wait_for_observed_load_completions(&mut app, &observation, 2);

        assert_eq!(observation.load_count.load(Ordering::SeqCst), 2);
        assert!(post_turn_continuation_is_paused(&app));
        assert_eq!(ready_status(&app), "newer operator status");
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn reset_busy_gate_and_exact_completion_survive_workspace_aba() {
        let workspace_a = TempPlanningWorkspace::new("tui-reset-workspace-aba-a");
        let workspace_b = TempPlanningWorkspace::new("tui-reset-workspace-aba-b");
        let mut seed_app = make_test_app(&workspace_a);
        seed_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut seed_app);
        seed_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut seed_app);

        let (port, observation) = FailingPlanningWorkspacePort::observed();
        let mut app = make_test_app_with_planning_workspace_port(&workspace_a, Arc::new(port));
        wait_for_observed_loads_to_settle(&mut app, &observation);
        observation.reset_and_enable(workspace_a.path_str(), false);

        app.handle_reset_shell_command(Some("queue"));
        app.sync_draft_shell_workspace(workspace_b.path_str());
        app.sync_draft_shell_workspace(workspace_a.path_str());
        assert_eq!(
            app.planning_workspace_directory(),
            workspace_a.path_str(),
            "test must return to the original workspace before settlement"
        );

        app.stage_simple_mode_planning_init_draft();
        assert_eq!(
            ready_status(&app),
            "planning workspace busy / reset queue is still in progress"
        );
        wait_for_planning_workspace_operation(&mut app);
        wait_for_observed_load_completions(&mut app, &observation, 3);

        assert_eq!(observation.load_count.load(Ordering::SeqCst), 3);
        assert!(post_turn_continuation_is_paused(&app));
        assert_eq!(
            ready_status(&app),
            "planning workspace busy / reset queue is still in progress",
            "A→B→A completion must refresh the current authority without replaying stale presentation"
        );
    }

    #[test]
    fn simple_authoring_completions_do_not_reopen_after_close_or_workspace_drift() {
        let workspace_a = TempPlanningWorkspace::new("tui-simple-authoring-close-a");
        let workspace_b = TempPlanningWorkspace::new("tui-simple-authoring-close-b");
        let mut closed_app = make_test_app(&workspace_a);
        closed_app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        let stage = PlanningWorkspaceOperationCorrelation {
            generation: 70,
            workspace_directory: workspace_a.path_str().to_string(),
            operation: PlanningWorkspaceOperationKind::StageSimpleDraft,
        };
        closed_app
            .planning_init_overlay_ui_state
            .begin_simple_authoring_operation();
        closed_app
            .planning_workspace_operation_ui_state
            .begin(stage.clone(), closed_app.planning_ui_intent_revision);
        closed_app.close_shell_overlay();

        closed_app.apply_simple_planning_draft_stage_completion(
            stage.clone(),
            Ok(Box::new(PlanningSimpleDraftStageSnapshot {
                session_identity: stage.editor_session_identity("draft-close"),
                staged_file_count: 3,
                validation_report: Default::default(),
            })),
        );

        assert_eq!(closed_app.shell_overlay, ShellOverlay::Hidden);
        assert!(
            closed_app
                .planning_init_overlay_ui_state
                .simple_review()
                .is_none()
        );
        assert!(
            closed_app
                .planning_workspace_operation_ui_state
                .active_correlation()
                .is_none()
        );

        let mut drifted_app = make_test_app(&workspace_a);
        drifted_app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        let source = crate::core::app::PlanningEditorSessionIdentity::new(
            71,
            workspace_a.path_str(),
            "draft-drift",
        );
        drifted_app
            .planning_init_overlay_ui_state
            .open_simple_review_summary(
                source.clone(),
                source.draft_name.clone(),
                3,
                Default::default(),
            );
        let load = PlanningWorkspaceOperationCorrelation {
            generation: 72,
            workspace_directory: workspace_a.path_str().to_string(),
            operation: PlanningWorkspaceOperationKind::LoadSimpleEditor {
                draft_name: source.draft_name.clone(),
                source_session: source,
            },
        };
        drifted_app
            .planning_init_overlay_ui_state
            .begin_simple_authoring_operation();
        drifted_app
            .planning_workspace_operation_ui_state
            .begin(load.clone(), drifted_app.planning_ui_intent_revision);
        drifted_app.sync_draft_shell_workspace(workspace_b.path_str());
        drifted_app.apply_simple_planning_editor_load_completion(
            load.clone(),
            Ok(Box::new(PlanningEditorSessionSnapshot {
                session_identity: load.editor_session_identity("draft-drift"),
                draft_directory: "/tmp/draft-drift".to_string(),
                editable_files: Vec::new(),
                validation_report: Default::default(),
            })),
        );

        assert!(
            drifted_app
                .planning_draft_editor_ui_state
                .session_identity()
                .is_none()
        );
        assert!(
            drifted_app
                .planning_workspace_operation_ui_state
                .active_correlation()
                .is_none()
        );
    }

    #[test]
    fn reopened_simple_stage_keeps_new_runtime_loading_when_old_stage_finishes_first() {
        let (_workspace, mut app, _correlation, stage_gate, load_gate) =
            begin_gated_simple_stage("tui-simple-stage-reopen-stage-first");

        app.close_shell_overlay();
        load_gate.arm();
        app.open_first_run_planning_simple_review();
        load_gate.wait_until_entered();
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::Loading
        );
        assert!(matches!(
            &app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Loading { .. }
        ));

        stage_gate.release();
        wait_for_planning_workspace_operation(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::Loading,
            "the old stage completion must not replace the reopened runtime loading surface"
        );
        assert!(matches!(
            app.planning_runtime_refresh_ui_state,
            PlanningRuntimeRefreshUiState::Loading { .. }
        ));

        load_gate.release();
        wait_for_planning_runtime_refresh(&mut app);
        wait_for_planning_workspace_operation(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
    }

    #[test]
    fn reopened_simple_stage_rebinds_coalesced_stage_when_refresh_finishes_first() {
        let (_workspace, mut app, correlation, stage_gate, _load_gate) =
            begin_gated_simple_stage("tui-simple-stage-reopen-refresh-first");

        app.close_shell_overlay();
        app.open_first_run_planning_simple_review();
        wait_for_planning_runtime_refresh(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::Loading
        );
        assert_eq!(
            app.planning_workspace_operation_ui_state
                .active_correlation(),
            Some(&correlation),
            "the reopened simple request must rebind the coalesced stage"
        );

        stage_gate.release();
        wait_for_planning_workspace_operation(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
    }

    #[test]
    fn stale_simple_promotion_refreshes_authority_without_closing_or_replacing_newer_status() {
        let workspace = TempPlanningWorkspace::new("tui-simple-promotion-stale-presentation");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        let source = crate::core::app::PlanningEditorSessionIdentity::new(
            80,
            workspace.path_str(),
            "draft-stale",
        );
        app.planning_init_overlay_ui_state
            .open_simple_review_summary(
                source.clone(),
                source.draft_name.clone(),
                3,
                Default::default(),
            );
        let promotion = PlanningWorkspaceOperationCorrelation {
            generation: 81,
            workspace_directory: workspace.path_str().to_string(),
            operation: PlanningWorkspaceOperationKind::PromoteSimpleDraft {
                draft_name: source.draft_name.clone(),
                source_session: source,
            },
        };
        app.planning_init_overlay_ui_state
            .begin_simple_authoring_operation();
        app.planning_workspace_operation_ui_state
            .begin(promotion.clone(), app.planning_ui_intent_revision);
        let permit = app.post_turn_continuation_gate.capture();
        app.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "newer operator status".to_string(),
        });

        app.apply_simple_planning_draft_promotion_completion(
            promotion,
            Ok(Box::new(PlanningSimpleDraftPromotionSnapshot {
                draft_name: "draft-stale".to_string(),
                promoted_file_count: 3,
                validation_report: Default::default(),
            })),
        );

        assert!(!permit.is_current());
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert_eq!(ready_status(&app), "newer operator status");
    }

    #[test]
    fn late_simple_editor_load_cannot_replace_a_newer_editor_session_identity() {
        let workspace = TempPlanningWorkspace::new("tui-simple-editor-session-identity");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        let source = crate::core::app::PlanningEditorSessionIdentity::new(
            90,
            workspace.path_str(),
            "draft-session",
        );
        app.planning_init_overlay_ui_state
            .open_simple_review_summary(
                source.clone(),
                source.draft_name.clone(),
                3,
                Default::default(),
            );
        let load = PlanningWorkspaceOperationCorrelation {
            generation: 91,
            workspace_directory: workspace.path_str().to_string(),
            operation: PlanningWorkspaceOperationKind::LoadSimpleEditor {
                draft_name: source.draft_name.clone(),
                source_session: source,
            },
        };
        app.planning_init_overlay_ui_state
            .begin_simple_authoring_operation();
        app.planning_workspace_operation_ui_state
            .begin(load.clone(), app.planning_ui_intent_revision);
        let newer_identity = crate::core::app::PlanningEditorSessionIdentity::new(
            92,
            workspace.path_str(),
            "draft-session",
        );
        app.planning_draft_editor_ui_state
            .open_correlated_session(PlanningEditorSessionSnapshot {
                session_identity: newer_identity.clone(),
                draft_directory: "/tmp/newer-draft-session".to_string(),
                editable_files: Vec::new(),
                validation_report: Default::default(),
            });
        app.planning_init_overlay_ui_state.open_simple_editor();

        app.apply_simple_planning_editor_load_completion(
            load.clone(),
            Ok(Box::new(PlanningEditorSessionSnapshot {
                session_identity: load.editor_session_identity("draft-session"),
                draft_directory: "/tmp/late-draft-session".to_string(),
                editable_files: Vec::new(),
                validation_report: Default::default(),
            })),
        );

        assert_eq!(
            app.planning_draft_editor_ui_state.session_identity(),
            Some(&newer_identity)
        );
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ManualEditor
        );
    }

    #[test]
    fn simple_mode_editor_loads_and_promotion_blocks_invalid_staged_drafts() {
        let editor_workspace = TempPlanningWorkspace::new("tui-simple-editor-invalid-load");
        let mut editor_app = make_test_app(&editor_workspace);
        editor_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut editor_app);
        let editor_draft_name = editor_app
            .planning_init_overlay_ui_state
            .simple_review()
            .expect("simple review should be staged")
            .draft_name()
            .to_string();
        fs::write(
            staged_draft_file_path(
                &editor_workspace,
                &editor_draft_name,
                crate::application::service::planning::RESULT_OUTPUT_FILE_PATH,
            ),
            "invalid result output without heading",
        )
        .expect("staged result output should be writable");

        editor_app.open_simple_mode_planning_editor();
        wait_for_planning_workspace_operation(&mut editor_app);

        assert_eq!(
            editor_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ManualEditor
        );
        assert!(ready_status(&editor_app).contains("planning simple draft editor ready / draft: "));
        assert!(ready_status(&editor_app).contains("validation: needs attention"));

        let promote_workspace = TempPlanningWorkspace::new("tui-simple-promote-invalid");
        let mut promote_app = make_test_app(&promote_workspace);
        promote_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut promote_app);
        let promote_draft_name = promote_app
            .planning_init_overlay_ui_state
            .simple_review()
            .expect("simple review should be staged")
            .draft_name()
            .to_string();
        fs::write(
            staged_draft_file_path(
                &promote_workspace,
                &promote_draft_name,
                crate::application::service::planning::RESULT_OUTPUT_FILE_PATH,
            ),
            "invalid result output without heading",
        )
        .expect("staged result output should be writable");

        promote_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut promote_app);

        assert_eq!(promote_app.shell_overlay, ShellOverlay::PlanningInit);
        assert_eq!(
            promote_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert!(ready_status(&promote_app).contains("planning simple draft promote blocked"));
        assert!(ready_status(&promote_app).contains("validation: needs attention"));
    }

    #[test]
    fn controller_reports_workspace_port_failures() {
        let load_workspace = TempPlanningWorkspace::new("tui-controller-load-failure");
        let mut load_app = make_test_app_with_planning_workspace_port(
            &load_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::LoadWorkspace,
            )),
        );

        load_app.handle_planning_shell_command(None);
        wait_for_planning_init_refresh(&mut load_app);
        assert!(
            ready_status(&load_app).starts_with("planning setup unavailable: forced "),
            "status: {}",
            ready_status(&load_app)
        );
        assert_eq!(load_app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            load_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ModeSelection
        );
        assert!(
            load_app
                .planning_init_overlay_ui_state
                .simple_review()
                .is_none()
        );

        load_app.start_directions_maintenance_overview_load(Some(
            "directions maintenance reload requested".to_string(),
        ));
        wait_for_directions_maintenance_load(&mut load_app);
        assert!(matches!(
            load_app
                .directions_maintenance_overlay_ui_state
                .screen_model(),
            DirectionsMaintenanceScreenModel::Failed { error, .. }
                if error.starts_with("forced ")
        ));
        assert_eq!(
            ready_status(&load_app),
            "directions maintenance reload requested"
        );

        let stage_workspace = TempPlanningWorkspace::new("tui-controller-stage-failure");
        let mut stage_app = make_test_app_with_planning_workspace_port(
            &stage_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::StageDraft,
            )),
        );
        stage_app.stage_simple_mode_planning_init_draft();
        wait_for_planning_workspace_operation(&mut stage_app);
        assert!(
            ready_status(&stage_app).starts_with("planning init failed: forced "),
            "status: {}",
            ready_status(&stage_app)
        );

        stage_app.open_planning_manual_editor();
        assert!(
            ready_status(&stage_app).starts_with("planning init failed: forced "),
            "status: {}",
            ready_status(&stage_app)
        );

        stage_app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut stage_app);
        stage_app.open_queue_idle_prompt_editor();
        assert!(
            ready_status(&stage_app).starts_with("directions editor failed: forced "),
            "status: {}",
            ready_status(&stage_app)
        );

        let load_draft_workspace = TempPlanningWorkspace::new("tui-controller-load-draft-failure");
        let mut load_draft_app = make_test_app_with_planning_workspace_port(
            &load_draft_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::LoadDraft,
            )),
        );
        load_draft_app.stage_simple_mode_planning_init_draft();
        wait_for_planning_workspace_operation(&mut load_draft_app);
        assert!(
            ready_status(&load_draft_app).contains("planning simple review ready"),
            "status: {}",
            ready_status(&load_draft_app)
        );
        load_draft_app.open_simple_mode_planning_editor();
        wait_for_planning_workspace_operation(&mut load_draft_app);
        assert!(
            ready_status(&load_draft_app).starts_with("planning init failed: forced "),
            "status: {}",
            ready_status(&load_draft_app)
        );
    }

    #[test]
    fn planning_init_overlay_key_router_handles_detail_selection_and_manual_editor() {
        let workspace = TempPlanningWorkspace::new("tui-planning-init-detail-keys");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);

        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Char('b'))));
        assert_eq!(
            app.planning_init_overlay_ui_state.selected_mode(),
            PlanningInitModeSelection::Detail
        );
        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::DetailSelection
        );

        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Down)));
        assert_eq!(
            app.planning_init_overlay_ui_state.selected_detail(),
            PlanningInitDetailSelection::WorkerAssisted
        );
        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            ready_status(&app),
            "planning worker-assisted detail mode is not supported yet"
        );

        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Char('a'))));
        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Enter)));

        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ManualEditor
        );
        assert!(ready_status(&app).starts_with("planning draft editor ready / draft: "));

        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Char('!'))));
        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert!(app.handle_planning_init_overlay_key(ctrl_key(KeyCode::Char('s'))));

        assert!(ready_status(&app).contains("planning draft saved / draft: "));
        assert!(ready_status(&app).contains("validation: needs attention"));
    }

    #[test]
    fn planning_init_overlay_key_router_handles_simple_review_and_existing_workspace() {
        let workspace = TempPlanningWorkspace::new("tui-planning-init-simple-keys");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);

        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Enter)));
        wait_for_planning_workspace_operation(&mut app);

        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert!(ready_status(&app).contains("planning simple review ready / staged draft: "));

        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Char('D'))));
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::DetailSelection
        );
        assert_eq!(
            ready_status(&app),
            "planning detail authoring: choose how the advanced draft should open"
        );

        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Backspace)));
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert!(app.handle_planning_init_overlay_key(ctrl_key(KeyCode::Char('l'))));
        assert!(app.handle_planning_init_overlay_key(ctrl_key(KeyCode::Char('e'))));
        wait_for_planning_workspace_operation(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ManualEditor
        );
        assert!(ready_status(&app).contains("planning simple draft editor ready / draft: "));
        app.promote_planning_manual_editor();

        app.close_shell_overlay();
        app.show_planning_init_overlay();
        wait_for_planning_init_refresh(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ExistingWorkspace
        );
        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Char('D'))));
        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);

        app.show_planning_init_overlay();
        wait_for_planning_init_refresh(&mut app);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ExistingWorkspace
        );
        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Enter)));
        assert_eq!(app.shell_overlay, ShellOverlay::Queue);

        let promote_workspace = TempPlanningWorkspace::new("tui-planning-init-promote-keys");
        let mut promote_app = make_test_app(&promote_workspace);
        promote_app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        assert!(promote_app.handle_planning_init_overlay_key(key(KeyCode::Enter)));
        wait_for_planning_workspace_operation(&mut promote_app);
        assert_eq!(
            promote_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert!(promote_app.handle_planning_init_overlay_key(ctrl_key(KeyCode::Char('p'))));
        wait_for_planning_workspace_operation(&mut promote_app);
        assert_eq!(promote_app.shell_overlay, ShellOverlay::Hidden);
        assert!(ready_status(&promote_app).contains("planning draft promoted / draft: "));
    }

    #[test]
    fn directions_overlay_key_router_handles_detail_doc_confirmation_and_manual_editor() {
        let workspace = TempPlanningWorkspace::new("tui-directions-detail-keys");
        let mut app = make_test_app(&workspace);
        app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut app);

        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('d'))));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::DetailDocSelection
        );
        assert!(app.handle_directions_overlay_key(key(KeyCode::Down)));
        assert!(app.handle_directions_overlay_key(key(KeyCode::Up)));
        assert!(app.handle_directions_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::DetailDocConfirm
        );
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .pending_detail_doc_creation()
                .map(|pending| pending.direction_id()),
            Some("general-workstream")
        );

        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('2'))));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .detail_doc_confirm_choice(),
            DetailDocConfirmChoice::No
        );
        assert!(app.handle_directions_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
        assert_eq!(
            ready_status(&app),
            "detail doc creation skipped; directions remain unchanged"
        );

        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('d'))));
        assert!(app.handle_directions_overlay_key(key(KeyCode::Enter)));
        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('1'))));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .detail_doc_confirm_choice(),
            DetailDocConfirmChoice::Yes
        );
        assert!(app.handle_directions_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::ManualEditor
        );
        assert!(ready_status(&app).contains("directions detail doc editor ready / draft: "));

        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('!'))));
        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert!(app.handle_directions_overlay_key(ctrl_key(KeyCode::Char('s'))));
        assert!(ready_status(&app).contains("directions draft saved / draft: "));
        assert!(app.handle_directions_overlay_key(ctrl_key(KeyCode::Char('p'))));
        let promoted_status = ready_status(&app).to_string();
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Loading
        );
        wait_for_directions_maintenance_load(&mut app);

        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
        assert_eq!(ready_status(&app), promoted_status);
    }

    #[test]
    fn directions_overlay_key_router_reports_overview_guards_and_reload() {
        let workspace = TempPlanningWorkspace::new("tui-directions-overview-keys");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
        app.directions_maintenance_overlay_ui_state
            .open_summary(directions_summary(
                DirectionsSupportingFileStatus::MissingMapping,
                Some("bad directions json"),
            ));

        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('d'))));
        assert_eq!(
            ready_status(&app),
            "fix DB direction authority errors before generating detail docs"
        );
        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('p'))));
        assert_eq!(
            ready_status(&app),
            "fix DB direction authority errors before editing queue-idle prompt"
        );

        app.directions_maintenance_overlay_ui_state
            .open_summary(directions_summary(
                DirectionsSupportingFileStatus::Ready,
                None,
            ));
        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('d'))));
        assert_eq!(
            ready_status(&app),
            "every direction already has a healthy detail doc mapping"
        );

        assert!(app.handle_directions_overlay_key(key(KeyCode::Char('r'))));
        assert_eq!(
            ready_status(&app),
            "directions maintenance reload requested"
        );
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Loading
        );
        assert!(app.handle_directions_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Loading
        );
        wait_for_directions_maintenance_load(&mut app);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );

        assert!(app.handle_directions_overlay_key(key(KeyCode::Enter)));
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::ManualEditor
        );
        assert!(ready_status(&app).contains("queue-idle prompt editor ready / draft: "));
    }

    #[test]
    fn planning_manual_editor_keymap_saves_and_blocks_invalid_promotion() {
        let workspace = TempPlanningWorkspace::new("tui-planning-editor-invalid");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);

        app.open_planning_manual_editor();

        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ManualEditor
        );
        assert_eq!(
            app.planning_draft_editor_ui_state
                .selected_buffer()
                .expect("planning editor buffer should open")
                .active_path(),
            crate::application::service::planning::RESULT_OUTPUT_FILE_PATH
        );
        assert!(ready_status(&app).starts_with("planning draft editor ready / draft: "));

        app.handle_draft_editor_key(
            key(KeyCode::Char('!')),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );
        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());

        app.handle_draft_editor_key(
            ctrl_key(KeyCode::Char('s')),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );

        assert!(ready_status(&app).contains("planning draft saved / draft: "));
        assert!(ready_status(&app).contains("validation: needs attention"));
        assert!(!app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert!(
            app.planning_draft_editor_ui_state
                .has_invalid_staged_draft()
        );

        app.handle_draft_editor_key(
            ctrl_key(KeyCode::Char('p')),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );

        assert!(ready_status(&app).contains("planning draft promote blocked / draft: "));
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
        assert!(app.planning_draft_editor_ui_state.draft_name().is_some());
    }

    #[test]
    fn planning_manual_editor_successful_promotion_closes_planning_overlay() {
        let workspace = TempPlanningWorkspace::new("tui-planning-editor-promote");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        app.stage_simple_mode_planning_init_draft();
        wait_for_planning_workspace_operation(&mut app);
        app.open_simple_mode_planning_editor();
        wait_for_planning_workspace_operation(&mut app);

        app.promote_planning_manual_editor();

        assert!(
            ready_status(&app).contains("planning draft promoted / draft: "),
            "status: {}",
            ready_status(&app)
        );
        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert!(app.planning_draft_editor_ui_state.draft_name().is_none());
        assert!(
            workspace
                .path()
                .join(crate::application::service::planning::RESULT_OUTPUT_FILE_PATH)
                .is_file()
        );
    }

    #[test]
    fn planning_manual_editor_close_confirmation_can_cancel_and_confirm() {
        let workspace = TempPlanningWorkspace::new("tui-planning-editor-close");
        let mut app = make_test_app(&workspace);
        app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        app.open_planning_manual_editor();
        app.planning_draft_editor_ui_state.insert_character('!');

        app.request_close_planning_manual_editor();

        assert!(
            app.planning_draft_editor_ui_state
                .is_close_confirmation_pending()
        );
        assert!(ready_status(&app).contains("planning draft editor close pending"));

        assert!(app.handle_planning_manual_editor_close_confirmation_key(key(KeyCode::Char('N'))));
        assert!(
            !app.planning_draft_editor_ui_state
                .is_close_confirmation_pending()
        );
        assert_eq!(
            ready_status(&app),
            "planning draft editor close canceled; keep editing"
        );

        app.request_close_planning_manual_editor();
        assert!(app.handle_planning_manual_editor_close_confirmation_key(key(KeyCode::Enter)));

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert!(ready_status(&app).contains("planning draft editor closed"));
    }

    #[test]
    fn directions_detail_doc_editor_promotes_back_to_maintenance_overview() {
        let workspace = TempPlanningWorkspace::new("tui-directions-detail-promote");
        let mut app = make_test_app(&workspace);
        app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut app);

        app.open_directions_detail_doc_editor("general-workstream");

        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::ManualEditor
        );
        assert_eq!(
            app.planning_draft_editor_ui_state
                .selected_buffer()
                .expect("directions editor buffer should open")
                .active_path(),
            crate::application::service::planning::default_direction_detail_doc_path(
                "general-workstream"
            )
        );
        assert!(ready_status(&app).contains("directions detail doc editor ready / draft: "));

        app.save_directions_manual_editor();

        assert!(ready_status(&app).contains("directions draft saved / draft: "));
        assert!(ready_status(&app).contains("validation: ok"));

        app.promote_directions_manual_editor();

        let promoted_status = ready_status(&app).to_string();
        assert!(promoted_status.contains("directions draft promoted / draft: "));
        wait_for_directions_maintenance_load(&mut app);
        assert_eq!(ready_status(&app), promoted_status);
        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
        assert!(app.planning_draft_editor_ui_state.draft_name().is_none());
    }

    #[test]
    fn directions_editor_preserves_buffers_and_blocks_writes_after_workspace_drift() {
        let workspace = TempPlanningWorkspace::new("tui-directions-editor-workspace-drift");
        let mut app = make_test_app(&workspace);
        app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut app);
        app.open_directions_detail_doc_editor("general-workstream");
        app.planning_draft_editor_ui_state.insert_character('!');
        let draft_name = app
            .planning_draft_editor_ui_state
            .draft_name()
            .expect("directions draft should be open")
            .to_string();

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should have a ready conversation");
        };
        conversation.cwd = "/tmp/replacement-workspace".to_string();
        conversation.draft_workspace_directory = "/tmp/replacement-workspace".to_string();

        assert!(!app.directions_maintenance_load_required());
        app.save_directions_manual_editor();
        assert!(ready_status(&app).contains("workspace changed; save blocked"));
        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert_eq!(
            app.planning_draft_editor_ui_state.draft_name(),
            Some(draft_name.as_str())
        );

        app.promote_directions_manual_editor();
        assert!(ready_status(&app).contains("workspace changed; promote blocked"));
        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert_eq!(
            app.planning_draft_editor_ui_state.draft_name(),
            Some(draft_name.as_str())
        );
    }

    #[test]
    fn approval_overlay_suspends_and_restores_dirty_directions_editor() {
        let workspace = TempPlanningWorkspace::new("tui-directions-editor-approval-suspend");
        let mut app = make_test_app(&workspace);
        app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut app);
        app.open_directions_detail_doc_editor("general-workstream");
        app.planning_draft_editor_ui_state.insert_character('!');
        let draft_name = app
            .planning_draft_editor_ui_state
            .draft_name()
            .expect("directions draft should be open")
            .to_string();
        let edited_body = app
            .planning_draft_editor_ui_state
            .selected_buffer()
            .expect("directions editor buffer should be selected")
            .body();

        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);

        assert_eq!(app.shell_overlay, ShellOverlay::Approval);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::ManualEditor
        );
        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert_eq!(
            app.planning_draft_editor_ui_state
                .selected_buffer()
                .expect("suspended editor buffer should remain")
                .body(),
            edited_body
        );

        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayClosed);

        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::ManualEditor
        );
        assert_eq!(
            app.planning_draft_editor_ui_state.draft_name(),
            Some(draft_name.as_str())
        );
        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert_eq!(
            app.planning_draft_editor_ui_state
                .selected_buffer()
                .expect("restored editor buffer should remain")
                .body(),
            edited_body
        );
    }

    #[test]
    fn directions_manual_editor_close_confirmation_returns_to_overview() {
        let workspace = TempPlanningWorkspace::new("tui-directions-editor-close");
        let mut app = make_test_app(&workspace);
        app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut app);
        app.open_queue_idle_prompt_editor();

        assert_eq!(
            app.planning_draft_editor_ui_state
                .selected_buffer()
                .expect("queue-idle editor buffer should open")
                .active_path(),
            crate::application::service::planning::DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH
        );

        app.planning_draft_editor_ui_state.insert_character('!');
        app.request_close_directions_manual_editor();

        assert!(
            app.planning_draft_editor_ui_state
                .is_close_confirmation_pending()
        );
        assert!(ready_status(&app).contains("directions editor close pending"));

        assert!(
            app.handle_directions_manual_editor_close_confirmation_key(key(KeyCode::Char('n')))
        );
        assert_eq!(
            ready_status(&app),
            "directions editor close canceled; keep editing"
        );

        app.request_close_directions_manual_editor();
        assert!(app.handle_directions_manual_editor_close_confirmation_key(key(KeyCode::Enter)));
        let closed_status = ready_status(&app).to_string();
        wait_for_directions_maintenance_load(&mut app);

        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
        assert_eq!(ready_status(&app), closed_status);
        assert!(ready_status(&app).contains("directions editor closed"));
        assert!(app.planning_draft_editor_ui_state.draft_name().is_none());
    }

    #[test]
    fn editor_controller_handles_empty_session_immediate_close_and_confirmation_fallthrough() {
        let workspace = TempPlanningWorkspace::new("tui-editor-empty-session");
        let mut app = make_test_app(&workspace);
        let unchanged_status = ready_status(&app).to_string();

        app.open_simple_mode_planning_editor();
        app.promote_simple_mode_planning_draft();
        app.save_planning_manual_editor();
        app.save_directions_manual_editor();
        app.promote_planning_manual_editor();
        app.promote_directions_manual_editor();

        assert_eq!(ready_status(&app), unchanged_status);
        assert!(app.planning_draft_editor_ui_state.draft_name().is_none());

        let planning_close_workspace = TempPlanningWorkspace::new("tui-planning-clean-close");
        let mut planning_close_app = make_test_app(&planning_close_workspace);
        planning_close_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut planning_close_app);
        planning_close_app.open_simple_mode_planning_editor();
        wait_for_planning_workspace_operation(&mut planning_close_app);
        planning_close_app.request_close_planning_manual_editor();

        assert_eq!(planning_close_app.shell_overlay, ShellOverlay::Hidden);
        assert!(
            !planning_close_app
                .planning_draft_editor_ui_state
                .is_close_confirmation_pending()
        );

        let directions_close_workspace = TempPlanningWorkspace::new("tui-directions-clean-close");
        let mut directions_close_app = make_test_app(&directions_close_workspace);
        directions_close_app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut directions_close_app);
        directions_close_app.open_directions_detail_doc_editor("general-workstream");
        directions_close_app.request_close_directions_manual_editor();
        let closed_status = ready_status(&directions_close_app).to_string();
        wait_for_directions_maintenance_load(&mut directions_close_app);

        assert_eq!(
            directions_close_app.shell_overlay,
            ShellOverlay::DirectionsMaintenance
        );
        assert_eq!(
            directions_close_app
                .directions_maintenance_overlay_ui_state
                .step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
        assert_eq!(ready_status(&directions_close_app), closed_status);

        let confirmation_workspace = TempPlanningWorkspace::new("tui-editor-confirm-fallthrough");
        let mut confirmation_app = make_test_app(&confirmation_workspace);
        confirmation_app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        confirmation_app.open_planning_manual_editor();
        let mut invalid_report = PlanningValidationReport::new();
        invalid_report.push_error(
            PlanningFileKind::ResultOutput,
            "test-invalid-draft",
            "invalid staged draft",
        );
        confirmation_app
            .planning_draft_editor_ui_state
            .apply_save_result(invalid_report);
        confirmation_app.request_close_planning_manual_editor();
        assert!(
            confirmation_app
                .planning_draft_editor_ui_state
                .is_close_confirmation_pending()
        );

        assert!(
            !confirmation_app
                .handle_planning_manual_editor_close_confirmation_key(key(KeyCode::Char('x')))
        );
        assert!(
            !confirmation_app
                .planning_draft_editor_ui_state
                .is_close_confirmation_pending()
        );

        confirmation_app.request_close_planning_manual_editor();
        assert!(
            confirmation_app
                .planning_draft_editor_ui_state
                .is_close_confirmation_pending()
        );
        assert!(matches!(
            confirmation_app.planning_draft_editor_ui_state.close_risk(),
            Some(risk) if !risk.has_dirty_buffers() && risk.has_invalid_staged_draft()
        ));
        confirmation_app.request_close_planning_manual_editor();

        assert_eq!(confirmation_app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            ready_status(&confirmation_app),
            "planning draft editor closed; invalid staged draft remains in drafts for review"
        );
    }

    #[test]
    fn draft_editor_keymap_exercises_navigation_editing_and_default_keys() {
        let workspace = TempPlanningWorkspace::new("tui-draft-keymap-navigation");
        let mut app = make_test_app(&workspace);
        app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut app);
        app.open_simple_mode_planning_editor();
        wait_for_planning_workspace_operation(&mut app);

        app.handle_draft_editor_key(
            key(KeyCode::Tab),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );
        app.handle_draft_editor_key(
            key(KeyCode::BackTab),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );
        for code in [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Backspace,
        ] {
            app.handle_draft_editor_key(
                key(code),
                NativeTuiApp::save_planning_manual_editor,
                NativeTuiApp::promote_planning_manual_editor,
            );
        }
        app.handle_draft_editor_key(
            shift_key(KeyCode::Char('A')),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );
        app.handle_draft_editor_key(
            ctrl_key(KeyCode::Char('w')),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );
        app.handle_draft_editor_key(
            key(KeyCode::Esc),
            NativeTuiApp::save_planning_manual_editor,
            NativeTuiApp::promote_planning_manual_editor,
        );

        assert!(app.planning_draft_editor_ui_state.has_dirty_buffers());
        assert_eq!(
            app.planning_draft_editor_ui_state.selected_file_index(),
            Some(0)
        );
    }

    #[test]
    fn editor_save_and_promote_report_workspace_port_failures() {
        let save_workspace = TempPlanningWorkspace::new("tui-planning-save-failure");
        let mut save_app = make_test_app_with_planning_workspace_port(
            &save_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::ReplaceDraft,
            )),
        );
        save_app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        save_app.open_planning_manual_editor();
        save_app.save_planning_manual_editor();
        assert!(
            ready_status(&save_app).starts_with("planning draft save failed: forced "),
            "status: {}",
            ready_status(&save_app)
        );

        let directions_save_workspace = TempPlanningWorkspace::new("tui-directions-save-failure");
        let mut directions_save_app = make_test_app_with_planning_workspace_port(
            &directions_save_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::ReplaceDraft,
            )),
        );
        directions_save_app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut directions_save_app);
        directions_save_app.open_directions_detail_doc_editor("general-workstream");
        directions_save_app.save_directions_manual_editor();
        assert!(
            ready_status(&directions_save_app).starts_with("directions draft save failed: forced "),
            "status: {}",
            ready_status(&directions_save_app)
        );

        let promote_workspace = TempPlanningWorkspace::new("tui-planning-promote-failure");
        let mut promote_app = make_test_app_with_planning_workspace_port(
            &promote_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::ReplaceWorkspace,
            )),
        );
        promote_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut promote_app);
        promote_app.open_simple_mode_planning_editor();
        wait_for_planning_workspace_operation(&mut promote_app);
        promote_app.promote_planning_manual_editor();
        assert!(
            ready_status(&promote_app).starts_with("planning draft promote failed: forced "),
            "status: {}",
            ready_status(&promote_app)
        );
        assert_eq!(promote_app.shell_overlay, ShellOverlay::PlanningInit);

        let directions_promote_workspace =
            TempPlanningWorkspace::new("tui-directions-promote-failure");
        let mut seed_app = make_test_app(&directions_promote_workspace);
        seed_app.open_first_run_planning_simple_review();
        wait_for_planning_init_refresh(&mut seed_app);
        seed_app.promote_simple_mode_planning_draft();
        wait_for_planning_workspace_operation(&mut seed_app);
        let mut directions_promote_app = make_test_app_with_planning_workspace_port(
            &directions_promote_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::ReplaceWorkspace,
            )),
        );
        directions_promote_app.show_directions_maintenance_overlay();
        wait_for_directions_maintenance_load(&mut directions_promote_app);
        directions_promote_app.open_directions_detail_doc_editor("general-workstream");
        directions_promote_app.promote_directions_manual_editor();
        assert!(
            ready_status(&directions_promote_app)
                .starts_with("directions draft promote failed: forced "),
            "status: {}",
            ready_status(&directions_promote_app)
        );
        assert_eq!(
            directions_promote_app
                .directions_maintenance_overlay_ui_state
                .step(),
            DirectionsMaintenanceOverlayStep::ManualEditor
        );
    }

    #[test]
    fn editor_overlay_keymaps_stay_tui_local_and_delegate_mutations() {
        let controller_runtime_source = CONTROLLER_RS
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or(CONTROLLER_RS);

        for ui_action in [
            "move_file_selection",
            "move_cursor_left",
            "move_cursor_right",
            "move_cursor_up",
            "move_cursor_down",
            "insert_newline",
            "backspace",
            "delete_previous_word",
            "insert_character",
        ] {
            assert!(
                controller_runtime_source.contains(ui_action),
                "draft editor keymap should keep UI-only action local: {ui_action}"
            );
        }

        assert!(controller_runtime_source.contains("save: fn(&mut Self)"));
        assert!(controller_runtime_source.contains("promote: fn(&mut Self)"));
        assert_eq!(
            occurrence_count(
                controller_runtime_source,
                "KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => save(self)"
            ),
            1
        );
        assert_eq!(
            occurrence_count(
                controller_runtime_source,
                "KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => promote(self)"
            ),
            1
        );

        for (overlay_name, source, save_hook, promote_hook, close_hook) in [
            (
                "planning init",
                PLANNING_INIT_OVERLAY_RS,
                "Self::save_planning_manual_editor",
                "Self::promote_planning_manual_editor",
                "handle_planning_manual_editor_close_confirmation_key",
            ),
            (
                "directions",
                DIRECTIONS_OVERLAY_RS,
                "Self::save_directions_manual_editor",
                "Self::promote_directions_manual_editor",
                "handle_directions_manual_editor_close_confirmation_key",
            ),
        ] {
            assert!(
                source.contains("self.handle_draft_editor_key("),
                "{overlay_name} overlay should delegate editor keys to the shared TUI keymap"
            );
            assert!(
                source.contains(save_hook),
                "{overlay_name} overlay should inject its service save hook"
            );
            assert!(
                source.contains(promote_hook),
                "{overlay_name} overlay should inject its service promote hook"
            );
            assert!(
                source.contains(close_hook),
                "{overlay_name} overlay should guard close confirmation before text editing"
            );
        }

        assert_eq!(
            occurrence_count(EDITOR_RS, ".save_draft_editor_files("),
            2,
            "planning and directions editor saves should both delegate through the application planning handle"
        );
        assert_eq!(
            occurrence_count(EDITOR_RS, ".promote_draft_editor_files("),
            2,
            "planning and directions editor promotions should both delegate through the application planning handle"
        );

        for forbidden in [
            "PlanningAdmin",
            "PlanningControlCommand",
            "PlanningControlRequest",
            "run_orchestrator_tick",
            "process_distributor_queue",
        ] {
            for (source_name, source) in [
                ("controller", controller_runtime_source),
                ("editor", EDITOR_RS),
                ("planning init overlay", PLANNING_INIT_OVERLAY_RS),
                ("directions overlay", DIRECTIONS_OVERLAY_RS),
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{source_name} should not route editor keymaps through cross-surface command vocabulary: {forbidden}"
                );
            }
        }
    }
}
