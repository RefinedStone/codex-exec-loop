use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::*;

pub(super) struct ShellRuntime {
    app: NativeTuiApp,
    frontend_mode: ShellFrontendMode,
    should_quit: bool,
    redraw_requested: bool,
}

impl ShellRuntime {
    pub(super) fn new(app: NativeTuiApp, frontend_mode: ShellFrontendMode) -> Self {
        Self {
            app,
            frontend_mode,
            should_quit: false,
            redraw_requested: true,
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

    pub(super) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.redraw_requested)
    }

    fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }

    pub(super) fn poll_background_messages(&mut self) {
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
                    let draft_workspace_directory = self.app.current_workspace_directory();
                    let template_load_result = match &result {
                        Ok(snapshot) => {
                            Some(self.app.load_followup_template_catalog(&snapshot.cwd))
                        }
                        Err(_) => None,
                    };
                    self.app.dispatch_conversation_lifecycle(
                        ConversationLifecycleEvent::ConversationLoaded {
                            result,
                            template_load_result,
                            draft_workspace_directory,
                        },
                    );
                    self.app
                        .refresh_ready_conversation_planning_runtime_snapshot();
                    self.app
                        .dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                            stop_keyword: self.app.current_stop_keyword_value(),
                            max_auto_turns: self.app.current_max_auto_turns_value().to_string(),
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
                BackgroundMessage::GithubReviewPollLoaded(result) => self
                    .app
                    .record_github_review_poll_result(Instant::now(), result),
            }
        }

        redraw_requested |= self.app.maybe_start_github_review_poll(Instant::now());
        if redraw_requested {
            self.request_redraw();
        }
    }

    pub(super) fn handle_terminal_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }

                self.handle_key_press(key);
            }
            Event::Resize(_, _) => self.request_redraw(),
            _ => {}
        }
    }

    fn handle_key_press(&mut self, key: KeyEvent) {
        if let Some(confirmed_exit) = self.app.handle_exit_confirmation_key(key) {
            if !confirmed_exit {
                self.request_redraw();
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
            self.request_redraw();
            return;
        }

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.app.handle_ctrl_c();
            self.request_redraw();
            return;
        }

        match key.code {
            KeyCode::PageUp
                if key.modifiers.is_empty()
                    && self.frontend_mode == ShellFrontendMode::AlternateScreen =>
            {
                self.app.scroll_transcript_page_up()
            }
            KeyCode::PageDown
                if key.modifiers.is_empty()
                    && self.frontend_mode == ShellFrontendMode::AlternateScreen =>
            {
                self.app.scroll_transcript_page_down()
            }
            KeyCode::Home
                if key.modifiers.is_empty()
                    && self.frontend_mode == ShellFrontendMode::AlternateScreen =>
            {
                self.app.scroll_transcript_to_top()
            }
            KeyCode::End
                if key.modifiers.is_empty()
                    && self.frontend_mode == ShellFrontendMode::AlternateScreen =>
            {
                self.app.scroll_transcript_to_tail()
            }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_auto_followup()
            }
            KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.start_max_auto_turns_edit()
            }
            KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.start_stop_keyword_edit()
            }
            KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.cycle_auto_followup_template()
            }
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_stop_keyword()
            }
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_no_file_change_stop()
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_startup_overlay()
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_session_overlay()
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_followup_template_overlay()
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

        self.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use anyhow::Result;
    use crossterm::event::KeyEventState;

    use super::*;
    use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::port::outbound::followup_template_port::{
        FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
    };
    use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::followup_template_service::FollowupTemplateService;
    use crate::application::service::github_review_poller_service::GithubReviewPollerService;
    use crate::application::service::planning_services::PlanningServices;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
    use crate::domain::github_review::{
        GithubPullRequestActivitySnapshot, GithubPullRequestTarget,
    };
    use crate::domain::recent_sessions::RecentSessions;
    use crate::domain::startup_diagnostics::StartupDiagnostics;

    #[derive(Default)]
    struct FakeCodexAppServerPort;

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }

        fn load_recent_sessions(&self, _limit: usize) -> Result<RecentSessions> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            })
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

    struct FakeFollowupTemplatePort;

    impl FollowupTemplatePort for FakeFollowupTemplatePort {
        fn load_workspace_templates(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
            Ok(Vec::new())
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

    fn make_test_runtime(frontend_mode: ShellFrontendMode) -> ShellRuntime {
        let codex_port = Arc::new(FakeCodexAppServerPort);
        let followup_port = Arc::new(FakeFollowupTemplatePort);
        let app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            FollowupTemplateService::new(followup_port),
            PlanningServices::from_workspace_port(Arc::new(
                FilesystemPlanningWorkspaceAdapter::new(),
            )),
        );

        ShellRuntime::new(app, frontend_mode)
    }

    fn sample_startup_diagnostics(workspace_path: &str) -> StartupDiagnostics {
        StartupDiagnostics {
            cwd: workspace_path.to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "ok".to_string(),
            workspace_ok: true,
            workspace_path: workspace_path.to_string(),
            workspace_detail: "ok".to_string(),
            initialize_ok: true,
            initialize_detail: "ok".to_string(),
            account_ok: true,
            account_detail: "ok".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "schema".to_string(),
        }
    }

    #[test]
    fn ctrl_q_requests_quit() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )));

        assert!(runtime.should_quit());
    }

    #[test]
    fn non_press_key_events_are_ignored() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);

        runtime.handle_terminal_event(Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        }));

        assert!(!runtime.should_quit());
    }

    #[test]
    fn startup_background_message_updates_app_state() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
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
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);

        assert!(runtime.take_redraw_request());
        assert!(!runtime.take_redraw_request());
    }

    #[test]
    fn idle_background_poll_does_not_request_redraw() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
        runtime.take_redraw_request();

        runtime.poll_background_messages();

        assert!(!runtime.take_redraw_request());
    }

    #[test]
    fn plain_character_input_uses_empty_modifier_check() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
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
    fn inline_mode_ignores_transcript_navigation_keys() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
        runtime.take_redraw_request();
        runtime.app_mut().sync_transcript_viewport_metrics(24, 6);

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));

        assert_eq!(runtime.app().transcript_viewport_status_label(), "tail");
        assert!(!runtime.take_redraw_request());
    }

    #[test]
    fn alternate_screen_keeps_transcript_navigation_keys() {
        let mut runtime = make_test_runtime(ShellFrontendMode::AlternateScreen);
        runtime.take_redraw_request();
        runtime.app_mut().sync_transcript_viewport_metrics(24, 6);

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));

        assert_eq!(
            runtime.app().transcript_viewport_status_label(),
            "manual 19/24"
        );
        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn resize_event_requests_redraw() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
        runtime.take_redraw_request();

        runtime.handle_terminal_event(Event::Resize(120, 40));

        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn ctrl_l_starts_max_auto_turns_editing() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
        runtime.app_mut().conversation_state =
            ConversationState::Ready(ConversationViewModel::new_draft(
                "/tmp/root".to_string(),
                runtime.app().load_followup_template_catalog("/tmp/root"),
            ));

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL,
        )));

        assert!(runtime.app().is_max_auto_turns_editing());
        assert_eq!(runtime.app().shell_overlay, ShellOverlay::FollowupTemplates);
    }

    #[test]
    fn ctrl_u_clears_buffered_input() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
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
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
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
    fn poll_background_messages_starts_github_review_polling_when_due() {
        let mut runtime = make_test_runtime(ShellFrontendMode::InlineMainBuffer);
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
