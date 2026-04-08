use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::*;

pub(super) struct ShellRuntime {
    app: NativeTuiApp,
    should_quit: bool,
}

impl ShellRuntime {
    pub(super) fn new(app: NativeTuiApp) -> Self {
        Self {
            app,
            should_quit: false,
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

    pub(super) fn poll_background_messages(&mut self) {
        while let Ok(message) = self.app.rx.try_recv() {
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
                }
                BackgroundMessage::SessionsLoaded(result) => {
                    self.app
                        .dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(result));
                    self.app.session_overlay_ui_state.reset();
                }
                BackgroundMessage::ConversationLoaded(result) => {
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
                        },
                    );
                    self.app
                        .dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                            stop_keyword: self.app.current_stop_keyword_value(),
                        });
                }
                BackgroundMessage::ConversationStream(event) => {
                    self.app.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::StreamUpdated(event),
                    );
                }
            }
        }
    }

    pub(super) fn handle_terminal_event(&mut self, event: Event) {
        let Event::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }

        self.handle_key_press(key);
    }

    fn handle_key_press(&mut self, key: KeyEvent) {
        if let Some(confirmed_exit) = self.app.handle_exit_confirmation_key(key) {
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
            return;
        }

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.app.handle_ctrl_c();
            return;
        }

        match key.code {
            KeyCode::PageUp if key.modifiers.is_empty() => self.app.scroll_transcript_page_up(),
            KeyCode::PageDown if key.modifiers.is_empty() => self.app.scroll_transcript_page_down(),
            KeyCode::Home if key.modifiers.is_empty() => self.app.scroll_transcript_to_top(),
            KeyCode::End if key.modifiers.is_empty() => self.app.scroll_transcript_to_tail(),
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_auto_followup()
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
            KeyCode::Backspace => self.app.pop_input_character(),
            KeyCode::Enter => self.app.start_turn_submission(),
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.app.push_input_character(character);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use crossterm::event::KeyEventState;

    use super::*;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::port::outbound::followup_template_port::{
        FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::followup_template_service::FollowupTemplateService;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
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

    fn make_test_runtime() -> ShellRuntime {
        let codex_port = Arc::new(FakeCodexAppServerPort);
        let followup_port = Arc::new(FakeFollowupTemplatePort);
        let app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            FollowupTemplateService::new(followup_port),
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
    fn startup_background_message_updates_app_state() {
        let mut runtime = make_test_runtime();
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
    fn plain_character_input_uses_empty_modifier_check() {
        let mut runtime = make_test_runtime();

        runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::empty(),
        )));

        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        assert_eq!(conversation.input_buffer, "a");
    }
}
