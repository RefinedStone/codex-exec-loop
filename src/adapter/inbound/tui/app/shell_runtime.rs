use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{
    BackgroundMessage, ConversationLifecycleEvent, ConversationRuntimeEvent,
    FollowupOverlayUiEvent, NativeTuiApp, SESSION_PAGE_SIZE, ShellChromeEvent,
};

pub(super) struct ShellRuntime {
    app: NativeTuiApp,
    should_quit: bool,
    frame_scheduler: TuiFrameScheduler,
    last_live_activity_pulse: Option<u64>,
}

impl ShellRuntime {
    pub(super) fn new(app: NativeTuiApp) -> Self {
        let now = Instant::now();
        Self {
            app,
            should_quit: false,
            frame_scheduler: TuiFrameScheduler::new(now),
            last_live_activity_pulse: None,
        }
    }

    pub(super) fn app_mut(&mut self) -> &mut NativeTuiApp {
        &mut self.app
    }

    #[cfg(test)]
    pub(super) fn app(&self) -> &NativeTuiApp {
        &self.app
    }

    pub(super) fn should_quit(&self) -> bool {
        self.should_quit
    }

    #[cfg(test)]
    pub(super) fn take_redraw_request(&mut self) -> bool {
        self.take_due_draw_request(Instant::now())
    }

    pub(super) fn take_due_draw_request(&mut self, now: Instant) -> bool {
        self.frame_scheduler.take_due(now)
    }

    pub(super) fn next_event_poll_timeout(
        &self,
        now: Instant,
        default_timeout: Duration,
    ) -> Duration {
        self.frame_scheduler.next_poll_timeout(now, default_timeout)
    }

    fn request_redraw_at(&mut self, now: Instant) {
        self.frame_scheduler.request_immediate(now);
    }

    pub(super) fn poll_background_messages(&mut self) {
        self.poll_background_messages_at(Instant::now());
    }

    fn poll_background_messages_at(&mut self, now: Instant) {
        let mut redraw_requested = false;

        while let Ok(message) = self.app.rx.try_recv() {
            redraw_requested = true;
            match message {
                BackgroundMessage::StartupLoaded(result) => {
                    let workspace_directory = match &result {
                        Ok(diagnostics) => Some(diagnostics.workspace_path.clone()),
                        Err(_) => None,
                    };
                    self.app
                        .dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                            result,
                            session_page_size: SESSION_PAGE_SIZE,
                        });
                    if let Some(workspace_directory) = workspace_directory {
                        self.app.sync_draft_shell_workspace(&workspace_directory);
                    }
                    self.app.resolve_startup_submit_queue();
                }
                BackgroundMessage::SessionsLoaded(result) => {
                    self.app
                        .dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(result));
                    self.app.session_overlay_ui_state.reset();
                }
                BackgroundMessage::ConversationLoaded(result) => {
                    let loaded_successfully = result.is_ok();
                    let draft_workspace_directory = self.app.current_workspace_directory();
                    self.app.reset_planner_worker_panel_state();
                    self.app.dispatch_conversation_lifecycle(
                        ConversationLifecycleEvent::ConversationLoaded {
                            result,
                            draft_workspace_directory,
                        },
                    );
                    self.app
                        .refresh_ready_conversation_planning_runtime_snapshot();
                    if loaded_successfully {
                        self.app.surface_resumed_session_planning_context();
                    }
                    self.app
                        .dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                            max_auto_turns: self.app.current_max_auto_turns_label(),
                        });
                }
                BackgroundMessage::ConversationStream(event) => {
                    self.app.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::StreamUpdated(event),
                    );
                }
                BackgroundMessage::ConversationRuntimeNotice(notice) => {
                    self.app.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::StreamExecutionObserved { notice },
                    );
                }
                BackgroundMessage::InvalidateParallelModeSupervisorSnapshot => {
                    self.app.invalidate_parallel_mode_supervisor_snapshot();
                }
                BackgroundMessage::PostTurnEvaluated {
                    thread_id,
                    queued_from_turn_id,
                    evaluation,
                    planner_worker_panel_state,
                } => {
                    if !self
                        .app
                        .should_apply_post_turn_evaluation(&thread_id, &queued_from_turn_id)
                    {
                        continue;
                    }
                    self.app.planner_worker_panel_state = planner_worker_panel_state;
                    self.app.invalidate_parallel_mode_supervisor_snapshot();
                    self.app.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::PostTurnEvaluated { evaluation },
                    );
                }
                BackgroundMessage::GithubReviewPollLoaded(result) => {
                    self.app.record_github_review_poll_result(now, result)
                }
            }
        }

        redraw_requested |= self.app.maybe_start_github_review_poll(now);
        let live_activity_pulse = self.app.live_activity_pulse(now);
        if live_activity_pulse != self.last_live_activity_pulse {
            redraw_requested = true;
        }
        self.last_live_activity_pulse = live_activity_pulse;
        if redraw_requested {
            self.request_redraw_at(now);
        } else if live_activity_pulse.is_some() {
            self.frame_scheduler
                .request_delayed(now, Duration::from_millis(250));
        }
    }

    pub(super) fn handle_terminal_event(&mut self, event: Event) {
        self.handle_terminal_event_at(event, Instant::now());
    }

    fn handle_terminal_event_at(&mut self, event: Event, now: Instant) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }

                self.handle_key_press(key, now);
            }
            Event::Resize(_, _) => self.request_redraw_at(now),
            Event::FocusGained => self.frame_scheduler.set_focused(true, now),
            Event::FocusLost => self.frame_scheduler.set_focused(false, now),
            _ => {}
        }
    }

    fn handle_key_press(&mut self, key: KeyEvent, now: Instant) {
        if let Some(confirmed_exit) = self.app.handle_exit_confirmation_key(key) {
            if !confirmed_exit {
                self.request_redraw_at(now);
            }
            if confirmed_exit {
                self.should_quit = true;
            }
            return;
        }

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('q') {
            self.should_quit = true;
            return;
        }

        if self.app.handle_shell_overlay_key(key) {
            self.request_redraw_at(now);
            return;
        }

        if self.app.is_inline_command_palette_active() {
            match key.code {
                KeyCode::Esc
                    if key.modifiers.is_empty() && self.app.dismiss_inline_command_palette() =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::Up
                    if key.modifiers.is_empty()
                        && self.app.move_inline_command_palette_selection(-1) =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::Down
                    if key.modifiers.is_empty()
                        && self.app.move_inline_command_palette_selection(1) =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::Enter
                    if key.modifiers.is_empty()
                        && self.app.accept_inline_command_palette_selection() =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                _ => {}
            }
        }

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.app.handle_ctrl_c();
            self.request_redraw_at(now);
            return;
        }

        match key.code {
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_startup_overlay()
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_session_overlay()
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => self
                .app
                .dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested),
            KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.open_new_conversation_shell()
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.insert_input_newline()
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.clear_prompt_input()
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.delete_previous_input_word()
            }
            KeyCode::Backspace => self.app.pop_input_character(),
            KeyCode::Enter => self.app.start_turn_submission(),
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.app.push_input_character(character)
            }
            _ => return,
        }

        self.request_redraw_at(now);
    }
}

#[derive(Debug, Clone)]
struct TuiFrameScheduler {
    focused: bool,
    next_deadline: Option<Instant>,
}

impl TuiFrameScheduler {
    fn new(now: Instant) -> Self {
        let mut scheduler = Self {
            focused: true,
            next_deadline: None,
        };
        scheduler.request_immediate(now);
        scheduler
    }

    fn request_immediate(&mut self, now: Instant) {
        self.coalesce_deadline(now);
    }

    fn request_delayed(&mut self, now: Instant, delay: Duration) {
        self.coalesce_deadline(now + delay);
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if !self.focused {
            return false;
        }

        let Some(deadline) = self.next_deadline else {
            return false;
        };
        if deadline > now {
            return false;
        }

        self.next_deadline = None;
        true
    }

    fn next_poll_timeout(&self, now: Instant, default_timeout: Duration) -> Duration {
        if !self.focused {
            return default_timeout;
        }

        let Some(deadline) = self.next_deadline else {
            return default_timeout;
        };
        default_timeout.min(deadline.saturating_duration_since(now))
    }

    fn set_focused(&mut self, focused: bool, now: Instant) {
        self.focused = focused;
        if focused {
            self.request_immediate(now);
        }
    }

    fn coalesce_deadline(&mut self, deadline: Instant) {
        if self
            .next_deadline
            .is_none_or(|existing_deadline| deadline < existing_deadline)
        {
            self.next_deadline = Some(deadline);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use anyhow::Result;
    use crossterm::event::KeyEventState;

    use super::*;
    use crate::adapter::inbound::tui::app::conversation_runtime::ConversationPostTurnEvaluation;
    use crate::adapter::inbound::tui::app::{
        ConversationInputState, ConversationState, InlineShellCommand,
    };
    use crate::adapter::inbound::tui::shell_chrome::{ShellOverlay, StartupState};
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::github_review_poller_service::GithubReviewPollerService;
    use crate::application::service::planning::PlanningServices;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{
        ConversationMessage, ConversationMessageKind, ConversationSnapshot,
    };
    use crate::domain::github_review::{
        GithubPullRequestActivitySnapshot, GithubPullRequestTarget,
    };
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[derive(Default)]
    struct FakeCodexAppServerPort;

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }

        fn load_recent_sessions(&self, _limit: usize) -> Result<SessionCatalog> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }

        fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            })
        }

        fn request_stop_all_sessions(&self) -> Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }
    }

    struct FakeGithubReviewPollerPort;

    impl GithubReviewPollerPort for FakeGithubReviewPollerPort {
        fn load_pull_request_activity(
            &self,
            target: &GithubPullRequestTarget,
        ) -> Result<GithubPullRequestActivitySnapshot> {
            Ok(GithubPullRequestActivitySnapshot {
                target: target.clone(),
                title: "Review status".to_string(),
                url: "https://github.com/acme/widgets/pull/42".to_string(),
                head_branch: "feature/native-github-poll-scheduling".to_string(),
                base_branch: "prerelease".to_string(),
                events: Vec::new(),
            })
        }
    }

    fn make_test_runtime() -> ShellRuntime {
        let codex_port = Arc::new(FakeCodexAppServerPort);
        let app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            PlanningServices::from_workspace_port(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        );

        ShellRuntime::new(app)
    }

    fn sample_startup_diagnostics(workspace_path: &str) -> StartupDiagnostics {
        StartupDiagnostics {
            cwd: workspace_path.to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "ok".to_string(),
            workspace_ok: true,
            workspace_path: workspace_path.to_string(),
            workspace_detail: "ok".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_ok: true,
            initialize_detail: "ok".to_string(),
            account_ok: true,
            account_detail: "ok".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "schema".to_string(),
        }
    }

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    fn bootstrap_active_planning_workspace(workspace_dir: &str) {
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        let stage_result = planning
            .workspace
            .stage_simple_mode_draft(workspace_dir)
            .expect("planning workspace should stage");
        let promote_result = planning
            .workspace
            .promote_staged_draft(workspace_dir, &stage_result.draft_name)
            .expect("planning workspace should promote");
        assert!(
            promote_result.promoted_file_count > 0,
            "bootstrap planning workspace should become ready"
        );
    }

    #[test]
    fn ctrl_q_requests_quit() {
        let mut runtime = make_test_runtime();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )));

        assert!(runtime.should_quit());
    }

    #[test]
    fn non_press_key_events_are_ignored() {
        let mut runtime = make_test_runtime();

        runtime.handle_terminal_event(Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        }));

        assert!(!runtime.should_quit());
    }

    #[test]
    fn resumed_session_status_surfaces_planning_and_queue_context() {
        let mut runtime = make_test_runtime();
        let workspace_dir = create_temp_workspace("resume-planning-context");
        bootstrap_active_planning_workspace(&workspace_dir);
        runtime.app_mut().startup_state =
            StartupState::Ready(sample_startup_diagnostics(&workspace_dir));
        runtime.take_redraw_request();

        runtime
            .app
            .tx
            .send(BackgroundMessage::ConversationLoaded(Ok(
                ConversationSnapshot {
                    thread_id: "thread-1".to_string(),
                    title: "Loaded thread".to_string(),
                    cwd: workspace_dir.clone(),
                    messages: Vec::new(),
                    warnings: Vec::new(),
                    runtime_notices: Vec::new(),
                },
            )))
            .expect("background message should enqueue");

        runtime.poll_background_messages();

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert!(
            conversation
                .status_text
                .contains("thread loaded / planning status: ready")
        );
        assert!(
            conversation
                .status_text
                .contains("queue summary: now: none  |  next: none")
        );
        assert!(
            conversation
                .status_text
                .contains("proposed: none  |  blocked: none")
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn startup_background_message_updates_app_state() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();
        runtime
            .app
            .tx
            .send(BackgroundMessage::StartupLoaded(Ok(
                sample_startup_diagnostics("/tmp/root"),
            )))
            .expect("startup message should send");

        runtime.poll_background_messages();

        match &runtime.app.startup_state {
            StartupState::Ready(diagnostics) => {
                assert_eq!(diagnostics.workspace_path, "/tmp/root");
            }
            other => panic!("expected ready startup state, got {other:?}"),
        }
    }

    #[test]
    fn runtime_starts_with_redraw_requested() {
        let mut runtime = make_test_runtime();

        assert!(runtime.take_redraw_request());
        assert!(!runtime.take_redraw_request());
    }

    #[test]
    fn scheduler_coalesces_immediate_and_delayed_requests() {
        let now = Instant::now();
        let mut scheduler = TuiFrameScheduler {
            focused: true,
            next_deadline: None,
        };

        scheduler.request_delayed(now, Duration::from_secs(10));
        scheduler.request_delayed(now, Duration::from_secs(5));
        assert_eq!(
            scheduler.next_poll_timeout(now, Duration::from_secs(30)),
            Duration::from_secs(5)
        );

        scheduler.request_immediate(now + Duration::from_secs(1));
        assert_eq!(
            scheduler.next_poll_timeout(now, Duration::from_secs(30)),
            Duration::from_secs(1)
        );
        assert!(!scheduler.take_due(now));
        assert!(scheduler.take_due(now + Duration::from_secs(1)));
        assert!(!scheduler.take_due(now + Duration::from_secs(1)));
    }

    #[test]
    fn scheduler_reports_zero_timeout_when_draw_is_due() {
        let now = Instant::now();
        let scheduler = TuiFrameScheduler::new(now);

        assert_eq!(
            scheduler.next_poll_timeout(now, Duration::from_millis(100)),
            Duration::ZERO
        );
    }

    #[test]
    fn focus_lost_blocks_draw_until_focus_returns() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();
        let now = Instant::now();

        runtime.handle_terminal_event_at(Event::FocusLost, now);
        runtime.handle_terminal_event_at(Event::Resize(120, 40), now + Duration::from_millis(1));

        assert!(!runtime.take_due_draw_request(now + Duration::from_millis(1)));

        runtime.handle_terminal_event_at(Event::FocusGained, now + Duration::from_millis(2));

        assert!(runtime.take_due_draw_request(now + Duration::from_millis(2)));
    }

    #[test]
    fn idle_background_poll_does_not_request_redraw() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();

        runtime.poll_background_messages();

        assert!(!runtime.take_redraw_request());
    }

    #[test]
    fn live_activity_schedules_delayed_draw_without_immediate_redraw() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();
        let now = Instant::now();
        let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.input_state = ConversationInputState::StreamingTurn;
        conversation.active_turn_id = Some("turn-1".to_string());
        conversation.active_turn_started_at = Some(now - Duration::from_secs(5));
        runtime.last_live_activity_pulse = Some(5);

        runtime.poll_background_messages_at(now);

        assert!(!runtime.take_due_draw_request(now));
        assert_eq!(
            runtime.next_event_poll_timeout(now, Duration::from_secs(1)),
            Duration::from_millis(250)
        );
        assert!(runtime.take_due_draw_request(now + Duration::from_millis(250)));
    }

    #[test]
    fn stale_post_turn_evaluation_background_message_is_ignored() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();
        let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.thread_id = "thread-1".to_string();
        conversation.status_text = "session ready".to_string();
        conversation.turn_activity.last_completed_turn_id = Some("turn-2".to_string());

        runtime
            .app
            .tx
            .send(BackgroundMessage::PostTurnEvaluated {
                thread_id: "thread-1".to_string(),
                queued_from_turn_id: "turn-1".to_string(),
                evaluation: Box::new(ConversationPostTurnEvaluation {
                    planning_runtime_snapshot: crate::application::service::planning::PlanningRuntimeSnapshot::invalid(
                        "stale snapshot".to_string(),
                    ),
                    planning_repair_state: None,
                    runtime_notices: vec!["stale notice".to_string()],
                    action: crate::adapter::inbound::tui::app::conversation_runtime::ConversationPostTurnAction::SkipAutoFollowup {
                        reason: crate::adapter::inbound::tui::app::conversation_model::AutoFollowupSkipReason::PostTurnContinuationPaused,
                    },
                }),
                planner_worker_panel_state: crate::adapter::inbound::tui::app::PlannerWorkerPanelState {
                    status: crate::adapter::inbound::tui::app::PlannerWorkerStatus::RefreshSucceeded,
                    last_operation_label: None,
                    last_queue_summary: Some("next task: stale".to_string()),
                    last_summary: Some("stale".to_string()),
                    last_rejected_summary: None,
                    last_notice_detail: None,
                    last_prompt: None,
                    last_response: None,
                    last_host_detail: None,
                },
            })
            .expect("background message should enqueue");

        runtime.poll_background_messages();

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.status_text, "session ready");
        assert!(conversation.runtime_notices.is_empty());
        assert!(
            runtime
                .app()
                .planner_worker_panel_state
                .last_summary
                .is_none()
        );
    }

    #[test]
    fn plain_character_input_uses_empty_modifier_check() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::empty(),
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.input_buffer, "a");
        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn supersession_overlay_allows_prompt_input() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::empty(),
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.input_buffer, "a");
        assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn supersession_overlay_allows_enter_to_submit_prompt() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().startup_state = StartupState::Ready(sample_startup_diagnostics(
            &runtime.app().current_workspace_directory(),
        ));
        runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
        for character in "run next".chars() {
            runtime.app_mut().push_input_character(character);
        }
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert!(conversation.input_buffer.is_empty());
        assert!(conversation.has_running_turn());
        assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn enter_executes_selected_inline_command_palette_item() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().push_input_character(':');
        runtime.app_mut().push_input_character('d');
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(runtime.app().shell_overlay, ShellOverlay::Startup);
        assert!(conversation.input_buffer.is_empty());
        assert!(
            conversation
                .status_text
                .contains("opened diagnostics inspection")
        );
    }

    #[test]
    fn down_then_enter_on_palette_item_with_argument_inserts_completion() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().push_input_character(':');
        runtime.app_mut().push_input_character('r');
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.input_buffer, ":reset ");
        assert!(!conversation.inline_shell_command_palette_state.is_active());
        assert_eq!(runtime.app().shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn up_wraps_inline_command_palette_selection() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().push_input_character(':');
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(
            conversation
                .inline_shell_command_palette_state
                .selected_command(),
            Some(InlineShellCommand::Help)
        );
    }

    #[test]
    fn escape_dismisses_inline_command_palette_without_clearing_buffer() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().push_input_character(':');
        runtime.app_mut().push_input_character('p');
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.input_buffer, ":p");
        assert!(!conversation.inline_shell_command_palette_state.is_active());
    }

    #[test]
    fn page_navigation_keys_do_not_trigger_transcript_navigation() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));

        assert!(!runtime.take_redraw_request());
    }

    #[test]
    fn resize_event_requests_redraw() {
        let mut runtime = make_test_runtime();
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Resize(120, 40));

        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn resize_event_leaves_transcript_state_unchanged() {
        let mut runtime = make_test_runtime();
        let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "completed history stays committed".to_string(),
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        conversation.refresh_conversation_lines();
        conversation.input_buffer = "buffered prompt".to_string();
        let expected_lines = conversation.cached_conversation_lines.clone();

        runtime.take_redraw_request();
        runtime.handle_terminal_event(Event::Resize(120, 40));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.cached_conversation_lines, expected_lines);
        assert_eq!(conversation.input_buffer, "buffered prompt");
        assert_eq!(conversation.messages.len(), 1);
        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn ctrl_u_clears_buffered_input() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().push_input_character('s');
        runtime.app_mut().push_input_character('h');
        runtime.app_mut().push_input_character('i');
        runtime.app_mut().push_input_character('p');

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert!(conversation.input_buffer.is_empty());
    }

    #[test]
    fn ctrl_w_deletes_previous_buffered_word() {
        let mut runtime = make_test_runtime();
        for character in "ship this next".chars() {
            runtime.app_mut().push_input_character(character);
        }

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.input_buffer, "ship this ");
    }

    #[test]
    fn manual_turn_elapsed_pulse_requests_redraw() {
        let mut runtime = make_test_runtime();
        let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.input_state = ConversationInputState::StreamingTurn;
        conversation.active_turn_id = Some("turn-1".to_string());
        conversation.active_turn_started_at = Some(Instant::now() - Duration::from_secs(5));
        runtime.last_live_activity_pulse = Some(4);
        runtime.take_redraw_request();

        runtime.poll_background_messages();

        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn poll_background_messages_starts_github_review_polling_when_due() {
        let mut runtime = make_test_runtime();
        runtime.app_mut().configure_github_review_polling(
            super::super::github_polling::GithubReviewPollingBootstrap {
                service: Some(GithubReviewPollerService::new(Arc::new(
                    FakeGithubReviewPollerPort,
                ))),
                state: super::super::github_polling::GithubReviewPollingState::active(
                    super::super::github_polling::GithubReviewPollingConfig {
                        target: GithubPullRequestTarget::new("acme/widgets", 42),
                        interval: Duration::from_secs(30),
                    },
                    Instant::now(),
                ),
            },
        );

        runtime.poll_background_messages();
        thread::sleep(Duration::from_millis(20));
        runtime.poll_background_messages();

        let super::super::github_polling::GithubReviewPollingState::Active(polling_state) =
            &runtime.app().github_review_polling_state
        else {
            panic!("expected active github review polling state");
        };
        assert!(polling_state.snapshot.is_some());
        assert!(polling_state.last_error.is_none());
    }
}
