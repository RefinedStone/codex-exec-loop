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
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::ConversationSnapshot;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};
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
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn make_test_app(workspace: &TempPlanningWorkspace) -> NativeTuiApp {
        let codex_port = Arc::new(FakeAppServerPort);
        let planning = crate::adapter::inbound::tui::app::test_helpers::test_planning_services(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
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
