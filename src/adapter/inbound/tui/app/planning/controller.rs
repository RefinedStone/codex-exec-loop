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
        self.present_directions_maintenance_overview(
            "opened directions maintenance".to_string(),
            true,
        );
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

    // Opening directions maintenance resets any draft editor first. Only one
    // planning-adjacent overlay owns the editor at a time, otherwise stale
    // buffers can be saved into the wrong workspace draft.
    pub(in crate::adapter::inbound::tui::app) fn present_directions_maintenance_overview(
        &mut self,
        status_text: String,
        ensure_overlay_visible: bool,
    ) {
        let workspace_directory = self.planning_workspace_directory();
        match self
            .application
            .planning()
            .workspace()
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

    // Planning init chooses between existing-workspace controls and first-run
    // setup from a fresh runtime projection, then refreshes the ready
    // conversation projection so the inline shell reports the same authority.
    pub(in crate::adapter::inbound::tui::app) fn show_planning_init_overlay(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let runtime_projection = self.load_planning_runtime_projection(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
            &workspace_directory,
        );
        if runtime_projection.workspace_present() {
            self.planning_init_overlay_ui_state
                .open_existing_workspace();
        } else {
            self.planning_init_overlay_ui_state
                .open_command_center_mode_selection();
        }
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: if runtime_projection.workspace_present() {
                "operator surface: planning setup / existing workspace".to_string()
            } else {
                "operator surface: planning setup / workspace: not initialized".to_string()
            },
        });
    }
    pub(in crate::adapter::inbound::tui::app) fn open_first_run_planning_simple_review(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
            &workspace_directory,
        );
        self.planning_init_overlay_ui_state
            .open_command_center_mode_selection();
        self.planning_draft_editor_ui_state.reset();
        self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        self.stage_simple_mode_planning_init_draft();
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
                let workspace_directory = self.planning_workspace_directory();
                match self
                    .application
                    .planning()
                    .workspace()
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
        let report = self
            .application
            .planning()
            .workspace()
            .inspect_workspace(&workspace_directory);

        self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
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
        let workspace_directory = self.planning_workspace_directory();
        match self
            .application
            .planning()
            .workspace()
            .reset_workspace(&workspace_directory, parsed.target)
        {
            Ok(result) => {
                self.pause_post_turn_continuation();
                self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
                    &workspace_directory,
                );
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: planning_reset_status_text(&result),
                });
            }
            Err(error) => {
                let fallback_status = if !self
                    .application
                    .planning()
                    .workspace()
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
            .application
            .planning()
            .workspace()
            .stage_simple_mode_draft(&workspace_directory)
        {
            Ok(stage_result) => {
                let validation_ok = stage_result.validation_report.is_valid();
                let draft_name = stage_result.draft_name.clone();
                self.planning_init_overlay_ui_state
                    .open_simple_review(stage_result);
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
            self.application
                .planning()
                .workspace()
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
            .application
            .planning()
            .workspace()
            .promote_staged_draft(&workspace_directory, &draft_name);
        self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::{ConversationState, NativeTuiParallelModeBinding};
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
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
    use crate::application::service::planning::{
        DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
        DirectionsSupportingFileStatus,
    };
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{
        ConversationControlSupport, ConversationSnapshot, ConversationTurnOptions,
    };
    use crate::domain::planning::QueueIdlePolicy;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

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
            })
        }

        fn request_stop_all_sessions(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> anyhow::Result<()> {
            Ok(())
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
    }

    struct FailingPlanningWorkspacePort {
        inner: FilesystemPlanningWorkspaceAdapter,
        failure: PlanningWorkspacePortFailure,
    }

    impl FailingPlanningWorkspacePort {
        fn new(failure: PlanningWorkspacePortFailure) -> Self {
            Self {
                inner: FilesystemPlanningWorkspaceAdapter::new(),
                failure,
            }
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
            self.fail_if(PlanningWorkspacePortFailure::LoadWorkspace)?;
            self.inner.load_planning_workspace_files(workspace_dir)
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
    ) -> DirectionsMaintenanceSummary {
        DirectionsMaintenanceSummary {
            directions: vec![DirectionsMaintenanceDirectionSummary {
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
        assert_eq!(truth.approval, ConversationControlSupport::ManualHandoff);
        assert_eq!(truth.interrupt, ConversationControlSupport::Unsupported);

        let snapshot = codex_port
            .load_conversation_snapshot("thread-fixture")
            .expect("snapshot should load");
        assert_eq!(snapshot.thread_id, "thread-fixture");
        codex_port
            .request_stop_all_sessions()
            .expect("stop should be accepted");
        let (new_thread_sender, _new_thread_receiver) = std::sync::mpsc::channel();
        codex_port
            .run_new_thread_stream(
                "/tmp/root",
                "prompt",
                ConversationTurnOptions::default(),
                new_thread_sender,
            )
            .expect("new thread stream should be accepted");
        let (turn_sender, _turn_receiver) = std::sync::mpsc::channel();
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
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert!(ready_status(&app).contains("planning simple review ready / staged draft: "));

        let existing_workspace = TempPlanningWorkspace::new("tui-planning-shell-existing");
        let mut existing_app = make_test_app(&existing_workspace);
        existing_app.open_first_run_planning_simple_review();
        existing_app.promote_simple_mode_planning_draft();
        assert_eq!(existing_app.shell_overlay, ShellOverlay::Hidden);

        existing_app.handle_planning_shell_command(None);
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
            PlanningInitOverlayStep::ExistingWorkspace
        );
        assert!(ready_status(&existing_app).starts_with("planning state: "));

        let absent_doctor_workspace = TempPlanningWorkspace::new("tui-planning-doctor-absent");
        let mut absent_doctor_app = make_test_app(&absent_doctor_workspace);
        absent_doctor_app.handle_planning_shell_command(Some("doctor"));
        assert_eq!(absent_doctor_app.shell_overlay, ShellOverlay::Hidden);
        assert!(ready_status(&absent_doctor_app).starts_with("planning state: ready_without_task"));
    }

    #[test]
    fn overlay_shell_open_paths_present_planning_surfaces() {
        let workspace = TempPlanningWorkspace::new("tui-overlay-shell-open-paths");
        let mut app = make_test_app(&workspace);

        app.handle_directions_shell_command(None);
        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(ready_status(&app), "opened directions maintenance");

        app.handle_queue_shell_command(None);
        assert_eq!(app.shell_overlay, ShellOverlay::Queue);

        app.close_shell_overlay();
        app.present_directions_maintenance_overview("directions loaded quietly".to_string(), false);

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(ready_status(&app), "directions loaded quietly");
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
            "planning workspace: missing / next action: open :planning to initialize it"
        );
        assert_eq!(app.shell_overlay, ShellOverlay::PlanningInit);

        let success_workspace = TempPlanningWorkspace::new("tui-reset-command-success");
        let mut success_app = make_test_app(&success_workspace);
        success_app.open_first_run_planning_simple_review();
        success_app.promote_simple_mode_planning_draft();
        success_app.handle_reset_shell_command(Some("queue"));
        assert_eq!(
            ready_status(&success_app),
            "planning reset applied / target: queue / rewritten: 0 / removed: 0"
        );
        success_app.handle_reset_shell_command(Some("directions confirm"));
        assert!(
            ready_status(&success_app).starts_with("planning reset applied / target: directions")
        );

        let failure_workspace = TempPlanningWorkspace::new("tui-reset-command-failure");
        let mut seed_app = make_test_app(&failure_workspace);
        seed_app.open_first_run_planning_simple_review();
        seed_app.promote_simple_mode_planning_draft();
        let mut failure_app = make_test_app_with_planning_workspace_port(
            &failure_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::ReplaceWorkspace,
            )),
        );

        failure_app.handle_reset_shell_command(Some("all confirm"));
        assert!(
            ready_status(&failure_app).starts_with("planning reset failed: forced "),
            "status: {}",
            ready_status(&failure_app)
        );
    }

    #[test]
    fn simple_mode_editor_loads_and_promotion_blocks_invalid_staged_drafts() {
        let editor_workspace = TempPlanningWorkspace::new("tui-simple-editor-invalid-load");
        let mut editor_app = make_test_app(&editor_workspace);
        editor_app.open_first_run_planning_simple_review();
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

        assert_eq!(
            editor_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ManualEditor
        );
        assert!(ready_status(&editor_app).contains("planning simple draft editor ready / draft: "));
        assert!(ready_status(&editor_app).contains("validation: needs attention"));

        let promote_workspace = TempPlanningWorkspace::new("tui-simple-promote-invalid");
        let mut promote_app = make_test_app(&promote_workspace);
        promote_app.open_first_run_planning_simple_review();
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
        assert!(
            ready_status(&load_app).starts_with("planning setup unavailable: forced "),
            "status: {}",
            ready_status(&load_app)
        );

        load_app.present_directions_maintenance_overview("reload directions".to_string(), false);
        assert!(
            ready_status(&load_app).starts_with("directions maintenance unavailable: forced "),
            "status: {}",
            ready_status(&load_app)
        );

        let stage_workspace = TempPlanningWorkspace::new("tui-controller-stage-failure");
        let mut stage_app = make_test_app_with_planning_workspace_port(
            &stage_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::StageDraft,
            )),
        );
        stage_app.stage_simple_mode_planning_init_draft();
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
        assert!(
            ready_status(&load_draft_app).contains("planning simple review ready"),
            "status: {}",
            ready_status(&load_draft_app)
        );
        load_draft_app.open_simple_mode_planning_editor();
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
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ManualEditor
        );
        assert!(ready_status(&app).contains("planning simple draft editor ready / draft: "));

        app.close_shell_overlay();
        app.show_planning_init_overlay();
        assert_eq!(
            app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::ExistingWorkspace
        );
        assert!(app.handle_planning_init_overlay_key(key(KeyCode::Char('D'))));
        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);

        app.show_planning_init_overlay();
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
        assert_eq!(
            promote_app.planning_init_overlay_ui_state.step(),
            PlanningInitOverlayStep::SimpleReview
        );
        assert!(promote_app.handle_planning_init_overlay_key(ctrl_key(KeyCode::Char('p'))));
        assert_eq!(promote_app.shell_overlay, ShellOverlay::Hidden);
        assert!(ready_status(&promote_app).contains("planning draft promoted / draft: "));
    }

    #[test]
    fn directions_overlay_key_router_handles_detail_doc_confirmation_and_manual_editor() {
        let workspace = TempPlanningWorkspace::new("tui-directions-detail-keys");
        let mut app = make_test_app(&workspace);
        app.show_directions_maintenance_overlay();

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

        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
        assert!(ready_status(&app).contains("directions draft promoted / draft: "));
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
        assert_eq!(ready_status(&app), "reloaded directions maintenance");
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
        app.open_simple_mode_planning_editor();

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

        assert!(ready_status(&app).contains("directions draft promoted / draft: "));
        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
        assert!(app.planning_draft_editor_ui_state.draft_name().is_none());
    }

    #[test]
    fn directions_manual_editor_close_confirmation_returns_to_overview() {
        let workspace = TempPlanningWorkspace::new("tui-directions-editor-close");
        let mut app = make_test_app(&workspace);
        app.show_directions_maintenance_overlay();
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

        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state.step(),
            DirectionsMaintenanceOverlayStep::Overview
        );
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
        planning_close_app.open_simple_mode_planning_editor();
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
        directions_close_app.open_directions_detail_doc_editor("general-workstream");
        directions_close_app.request_close_directions_manual_editor();

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
        assert_eq!(
            ready_status(&directions_close_app),
            "directions editor closed"
        );

        let confirmation_workspace = TempPlanningWorkspace::new("tui-editor-confirm-fallthrough");
        let mut confirmation_app = make_test_app(&confirmation_workspace);
        confirmation_app.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
        confirmation_app.open_planning_manual_editor();
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
        app.open_simple_mode_planning_editor();

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
        promote_app.open_simple_mode_planning_editor();
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
        seed_app.promote_simple_mode_planning_draft();
        let mut directions_promote_app = make_test_app_with_planning_workspace_port(
            &directions_promote_workspace,
            Arc::new(FailingPlanningWorkspacePort::new(
                PlanningWorkspacePortFailure::ReplaceWorkspace,
            )),
        );
        directions_promote_app.show_directions_maintenance_overlay();
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
            .split("#[cfg(test)]")
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
