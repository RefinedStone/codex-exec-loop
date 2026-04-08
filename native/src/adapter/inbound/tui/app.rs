use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::adapter::inbound::tui::shell_chrome::{
    ExitConfirmationState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
    ShellOverlay, StartupState, reduce_shell_chrome,
};
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot, ConversationStreamEvent,
};
use crate::domain::followup_template::FollowupTemplateCatalogLoadResult;
use crate::domain::session_summary::SessionSummary;

const SESSION_PAGE_SIZE: usize = 10;
const MAX_CONVERSATION_HISTORY_LINES: usize = 160;
const DEFAULT_AUTO_FOLLOW_MAX_TURNS: usize = 3;
const DEFAULT_AUTO_FOLLOW_STOP_KEYWORD: &str = "AUTO_STOP";
const FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP: u16 = 6;
const SHELL_FRAME_MARGIN: u16 = 1;
const MIN_SHELL_HEADER_HEIGHT: u16 = 4;
const MAX_SHELL_HEADER_HEIGHT: u16 = 6;
const MIN_TRANSCRIPT_PANEL_HEIGHT: u16 = 12;
const MIN_SHELL_STATUS_HEIGHT: u16 = 5;
const MAX_SHELL_STATUS_HEIGHT: u16 = 8;
const MIN_COMPOSER_HEIGHT: u16 = 4;
const MAX_COMPOSER_HEIGHT: u16 = 8;
const DEFAULT_TRANSCRIPT_PAGE_STEP: u16 = 6;
const ALT_SCREEN_ENV_VAR: &str = "CODEX_EXEC_LOOP_ALT_SCREEN";

#[path = "app/app_runtime.rs"]
mod app_runtime;
#[path = "app/conversation_input.rs"]
mod conversation_input;
#[path = "app/conversation_intents.rs"]
mod conversation_intents;
#[path = "app/conversation_lifecycle.rs"]
mod conversation_lifecycle;
#[path = "app/conversation_model.rs"]
mod conversation_model;
#[path = "app/conversation_runtime.rs"]
mod conversation_runtime;
#[path = "app/followup_controls.rs"]
mod followup_controls;
#[path = "app/followup_overlay_ui.rs"]
mod followup_overlay_ui;
#[path = "app/inline_shell_commands.rs"]
mod inline_shell_commands;
#[path = "app/session_overlay_ui.rs"]
mod session_overlay_ui;
#[path = "app/shell_controller.rs"]
mod shell_controller;
#[path = "app/shell_frontend.rs"]
mod shell_frontend;
#[path = "app/shell_layout.rs"]
mod shell_layout;
#[path = "app/shell_presentation.rs"]
mod shell_presentation;
#[path = "app/shell_rendering.rs"]
mod shell_rendering;
#[path = "app/shell_runtime.rs"]
mod shell_runtime;
#[path = "app/transcript_viewport.rs"]
mod transcript_viewport;

use app_runtime::BackgroundMessage;
pub use app_runtime::run;
use conversation_input::{ConversationInputEvent, reduce_conversation_input};
use conversation_intents::{
    ConversationIntentEffect, ConversationIntentEvent, ConversationIntentMode,
    ConversationIntentState, reduce_conversation_intents,
};
use conversation_lifecycle::{
    ConversationLifecycleEffect, ConversationLifecycleEvent, ConversationLifecycleState,
    reduce_conversation_lifecycle,
};
pub(super) use conversation_model::{
    AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason, ConversationInputState,
    ConversationState, ConversationViewModel, StopKeywordRule,
};
#[cfg(test)]
pub(super) use conversation_model::{RecordedAutoFollowupSkip, TurnActivityState};
use conversation_runtime::{
    ConversationRuntimeEffect, ConversationRuntimeEvent, reduce_conversation_runtime,
};
use followup_controls::{FollowupControlEffect, FollowupControlEvent, reduce_followup_controls};
use followup_overlay_ui::{
    FollowupOverlayUiEvent, FollowupOverlayUiState, reduce_followup_overlay_ui,
};
use inline_shell_commands::InlineShellCommand;
use session_overlay_ui::SessionOverlayUiState;
pub(super) use shell_controller::ShellActionAvailability;
use shell_frontend::ShellFrontendMode;
use shell_layout::{
    block_height_for_lines, build_conversation_scroll_offset, build_input_block_height,
    build_shell_footer_height,
};
use shell_presentation::format_conversation_lines;
#[cfg(test)]
use shell_presentation::{
    build_conversation_shell_frame_view, build_conversation_shell_view,
    build_followup_template_overlay_view, build_followup_template_preview_lines,
    build_followup_template_status_lines, build_input_title, build_ready_input_lines,
    build_session_overlay_view, build_shell_footer_lines, build_startup_overlay_view,
    build_status_title, build_transcript_panel_view, build_transcript_title,
};
use transcript_viewport::TranscriptViewportState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptOrigin {
    Manual,
    AutoFollow,
}

struct NativeTuiApp {
    shell_overlay: ShellOverlay,
    exit_confirmation_state: ExitConfirmationState,
    startup_state: StartupState,
    session_state: SessionState,
    conversation_state: ConversationState,
    selected_session_index: usize,
    session_overlay_ui_state: SessionOverlayUiState,
    followup_overlay_ui_state: FollowupOverlayUiState,
    transcript_viewport_state: TranscriptViewportState,
    active_session: Option<SessionSummary>,
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    followup_template_service: FollowupTemplateService,
    tx: Sender<BackgroundMessage>,
    rx: Receiver<BackgroundMessage>,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use ratatui::text::Line;

    use super::{
        AutoFollowState, AutoFollowupSkipReason, BackgroundMessage, ConversationInputState,
        ConversationMessage, ConversationMessageKind, ConversationRuntimeEvent, ConversationState,
        ConversationViewModel, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD, ExitConfirmationState,
        FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP, InlineShellCommand, MAX_COMPOSER_HEIGHT,
        NativeTuiApp, PromptOrigin, RecordedAutoFollowupSkip, SessionState,
        ShellActionAvailability, ShellFrontendMode, ShellOverlay, StartupState, TurnActivityState,
        build_conversation_shell_frame_view, build_conversation_shell_view,
        build_followup_template_overlay_view, build_followup_template_preview_lines,
        build_followup_template_status_lines, build_input_title, build_ready_input_lines,
        build_session_overlay_view, build_shell_footer_lines, build_startup_overlay_view,
        build_status_title, build_transcript_panel_view, build_transcript_title,
        format_conversation_lines, shell_layout,
    };
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
    use crate::domain::followup_template::{
        FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
    };
    use crate::domain::recent_sessions::RecentSessions;
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::startup_diagnostics::StartupDiagnostics;

    #[derive(Default)]
    struct FakeCodexAppServerPort {
        new_thread_calls: Mutex<Vec<(String, String)>>,
        turn_calls: Mutex<Vec<(String, String)>>,
    }

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
            cwd: &str,
            prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            self.new_thread_calls
                .lock()
                .expect("new-thread call mutex poisoned")
                .push((cwd.to_string(), prompt.to_string()));
            Ok(())
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            self.turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .push((thread_id.to_string(), prompt.to_string()));
            Ok(())
        }
    }

    struct FakeFollowupTemplatePort;

    impl FollowupTemplatePort for FakeFollowupTemplatePort {
        fn load_workspace_templates(
            &self,
            workspace_dir: &str,
        ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
            if workspace_dir == "/tmp/root" {
                return Ok(vec![WorkspaceFollowupTemplateRecord {
                    name: "root-template".to_string(),
                    path: "/tmp/root/.codex-exec-loop/followups/root-template.md".to_string(),
                    body: "workspace template body".to_string(),
                }]);
            }

            Ok(Vec::new())
        }
    }

    fn make_test_app() -> (NativeTuiApp, Arc<FakeCodexAppServerPort>) {
        let codex_port = Arc::new(FakeCodexAppServerPort::default());
        let followup_port = Arc::new(FakeFollowupTemplatePort);
        let app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port.clone()),
            FollowupTemplateService::new(followup_port),
        );

        (app, codex_port)
    }

    fn sample_template_catalog() -> FollowupTemplateCatalog {
        FollowupTemplateCatalog {
            items: vec![
                FollowupTemplateDefinition {
                    id: "builtin-next-task".to_string(),
                    label: "builtin next-task".to_string(),
                    body: "대리인입니다.\n자동 후속 {auto_turn}/{max_auto_turns} 입니다.\n\n직전 답변:\n{last_message}\n{stop_keyword}".to_string(),
                    source: FollowupTemplateSource::Builtin,
                },
                FollowupTemplateDefinition {
                    id: "builtin-plan-queue".to_string(),
                    label: "builtin plan-queue".to_string(),
                    body: "plan_priority_queue.md\n{last_message}\n{stop_keyword}".to_string(),
                    source: FollowupTemplateSource::Builtin,
                },
                FollowupTemplateDefinition {
                    id: "workspace-custom-review".to_string(),
                    label: "workspace custom-review".to_string(),
                    body: "workspace custom body\n{last_message}".to_string(),
                    source: FollowupTemplateSource::WorkspaceFile {
                        path: "/tmp/workspace/.codex-exec-loop/followups/custom-review.md"
                            .to_string(),
                    },
                },
            ],
        }
    }

    fn sample_startup_diagnostics(workspace_path: &str, can_continue: bool) -> StartupDiagnostics {
        StartupDiagnostics {
            cwd: workspace_path.to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "ok".to_string(),
            workspace_ok: true,
            workspace_path: workspace_path.to_string(),
            workspace_detail: "ok".to_string(),
            initialize_ok: true,
            initialize_detail: "ok".to_string(),
            account_ok: can_continue,
            account_detail: if can_continue {
                "ok".to_string()
            } else {
                "needs login".to_string()
            },
            warnings: Vec::new(),
            schema_snapshot: "schema".to_string(),
        }
    }

    fn sample_session(id: &str) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            name: Some(id.to_string()),
            preview: "preview".to_string(),
            cwd: "/tmp/root".to_string(),
            source: "codex".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: format!("/tmp/root/{id}.json"),
            git_branch: Some("main".to_string()),
        }
    }

    fn ready_conversation() -> ConversationViewModel {
        ConversationViewModel {
            thread_id: "thread-1".to_string(),
            title: "Existing session".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            cached_conversation_lines: format_conversation_lines(&[]),
            warnings: Vec::new(),
            input_buffer: String::new(),
            active_turn_id: None,
            input_state: ConversationInputState::ReadyToContinue,
            auto_follow_state: AutoFollowState::new(sample_template_catalog()),
            turn_activity: TurnActivityState::default(),
            last_auto_followup_skip: None,
            status_text: "thread loaded".to_string(),
        }
    }

    #[test]
    fn running_turn_still_shows_buffered_prompt() {
        let mut conversation = ready_conversation();
        conversation.input_state = ConversationInputState::StreamingTurn;
        conversation.input_buffer = "Continue from the last change.".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Continue from the last change."));
        assert!(rendered.contains("Ctrl+j inserts a new line"));
    }

    #[test]
    fn empty_existing_session_prompts_for_next_message() {
        let conversation = ready_conversation();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Ready to continue this session."));
        assert!(rendered.contains("Ctrl+j for newline"));
        assert!(rendered.contains("Shell commands: :diag"));
    }

    #[test]
    fn empty_draft_prompts_for_first_message() {
        let mut conversation = ready_conversation();
        conversation.thread_id.clear();
        conversation.input_state = ConversationInputState::DraftReady;

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Ready to start a new thread."));
        assert!(rendered.contains("Ctrl+j for newline"));
    }

    #[test]
    fn multiline_buffer_renders_as_multiple_input_lines() {
        let mut conversation = ready_conversation();
        conversation.input_buffer = "first line\nsecond line".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line == "first line"));
        assert!(rendered.iter().any(|line| line == "second line"));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("Ctrl+j inserts a new line"))
        );
    }

    #[test]
    fn inline_shell_command_buffer_shows_command_hint() {
        let mut conversation = ready_conversation();
        conversation.input_buffer = ":templates".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(":templates"));
        assert!(rendered.contains("Press Enter to open the template overlay."));
    }

    #[test]
    fn multiline_buffer_expands_composer_height() {
        let mut conversation = ready_conversation();
        conversation.input_buffer = "one\ntwo\nthree\nfour\nfive\nsix".to_string();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready);

        assert_eq!(
            shell_layout::build_input_block_height(&rendered),
            MAX_COMPOSER_HEIGHT
        );
    }

    #[test]
    fn status_footer_height_expands_for_ready_shell_summary() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_shell_footer_lines(&app);

        assert_eq!(shell_layout::build_shell_footer_height(&rendered), 7);
    }

    #[test]
    fn startup_pending_prompts_wait_before_send() {
        let conversation = ready_conversation();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Pending)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Startup checks are still running."));
        assert!(rendered.contains("send once diagnostics turn ready"));
    }

    #[test]
    fn startup_blocked_prompt_guides_user_to_diagnostics_overlay() {
        let conversation = ready_conversation();

        let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Blocked)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Startup diagnostics need attention."));
        assert!(rendered.contains("Open Ctrl+d"));
    }

    #[test]
    fn draft_workspace_sync_preserves_buffered_input() {
        let (mut app, _) = make_test_app();

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a draft conversation");
        };
        conversation.cwd = "/tmp/subdir".to_string();
        conversation.input_buffer = "buffered prompt".to_string();

        app.sync_draft_shell_workspace("/tmp/root");

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("draft conversation should still be ready");
        };
        assert_eq!(conversation.cwd, "/tmp/root");
        assert_eq!(conversation.input_buffer, "buffered prompt");
        assert_eq!(conversation.auto_follow_state.template_count(), 5);
        assert!(conversation.status_text.contains("draft workspace synced"));
    }

    #[test]
    fn background_startup_message_updates_startup_state_and_syncs_draft_workspace() {
        let (app, _) = make_test_app();
        let mut runtime = super::shell_runtime::ShellRuntime::new(app);
        let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state
        else {
            panic!("app should start with a draft conversation");
        };
        conversation.cwd = "/tmp/subdir".to_string();

        runtime
            .app()
            .tx
            .send(BackgroundMessage::StartupLoaded(Ok(
                sample_startup_diagnostics("/tmp/root", false),
            )))
            .expect("background message should enqueue");

        runtime.poll_background_messages();

        let app = runtime.app();
        assert!(matches!(app.startup_state, StartupState::Ready(_)));
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(conversation.cwd, "/tmp/root");
        assert_eq!(conversation.auto_follow_state.template_count(), 5);
    }

    #[test]
    fn background_conversation_loaded_resets_followup_overlay_state() {
        let (app, _) = make_test_app();
        let mut runtime = super::shell_runtime::ShellRuntime::new(app);
        runtime.app_mut().followup_overlay_ui_state.preview_scroll = 12;
        runtime
            .app_mut()
            .followup_overlay_ui_state
            .list_state
            .select(Some(2));
        runtime
            .app_mut()
            .followup_overlay_ui_state
            .stop_keyword_editor
            .buffer = "STALE".to_string();

        runtime
            .app()
            .tx
            .send(BackgroundMessage::ConversationLoaded(Ok(
                ConversationSnapshot {
                    thread_id: "thread-123".to_string(),
                    title: "Loaded thread".to_string(),
                    cwd: "/tmp/root".to_string(),
                    messages: Vec::new(),
                    warnings: Vec::new(),
                },
            )))
            .expect("background message should enqueue");

        runtime.poll_background_messages();

        let app = runtime.app();
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should become ready");
        };
        assert_eq!(conversation.thread_id, "thread-123");
        assert_eq!(conversation.cwd, "/tmp/root");
        assert_eq!(app.followup_overlay_ui_state.preview_scroll, 0);
        assert_eq!(app.followup_overlay_ui_state.list_state.selected(), None);
        assert_eq!(
            app.followup_overlay_ui_state.stop_keyword_editor.buffer,
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
    }

    #[test]
    fn opening_new_draft_is_blocked_while_turn_is_streaming() {
        let (mut app, _) = make_test_app();

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a draft conversation");
        };
        conversation.thread_id = "thread-123".to_string();
        conversation.title = "Streaming thread".to_string();
        conversation.input_state = ConversationInputState::StreamingTurn;

        app.open_new_conversation_shell();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(conversation.thread_id, "thread-123");
        assert_eq!(conversation.title, "Streaming thread");
        assert_eq!(
            conversation.input_state,
            ConversationInputState::StreamingTurn
        );
        assert!(conversation.status_text.contains("turn still running"));
    }

    #[test]
    fn auto_follow_submission_respects_startup_gate() {
        let (mut app, codex_port) = make_test_app();
        app.startup_state = StartupState::Loading;

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a draft conversation");
        };
        conversation.thread_id = "thread-123".to_string();
        conversation.input_state = ConversationInputState::ReadyToContinue;

        app.submit_prompt("continue working".to_string(), PromptOrigin::AutoFollow);

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert!(
            codex_port
                .turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .is_empty()
        );
        assert!(conversation.status_text.contains("auto follow-up paused"));
    }

    #[test]
    fn inline_diag_command_opens_overlay_and_clears_input() {
        let (mut app, codex_port) = make_test_app();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a ready conversation");
        };
        conversation.input_buffer = ":diag".to_string();

        app.start_turn_submission();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(app.shell_overlay, ShellOverlay::Startup);
        assert!(conversation.input_buffer.is_empty());
        assert!(
            conversation
                .status_text
                .contains("opened diagnostics overlay")
        );
        assert!(
            codex_port
                .new_thread_calls
                .lock()
                .expect("new-thread call mutex poisoned")
                .is_empty()
        );
        assert!(
            codex_port
                .turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .is_empty()
        );
    }

    #[test]
    fn inline_templates_command_opens_overlay_while_turn_is_streaming() {
        let (mut app, codex_port) = make_test_app();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a ready conversation");
        };
        conversation.input_state = ConversationInputState::StreamingTurn;
        conversation.input_buffer = ":templates".to_string();

        app.start_turn_submission();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
        assert_eq!(
            conversation.input_state,
            ConversationInputState::StreamingTurn
        );
        assert!(conversation.input_buffer.is_empty());
        assert!(
            codex_port
                .turn_calls
                .lock()
                .expect("turn call mutex poisoned")
                .is_empty()
        );
    }

    #[test]
    fn inline_help_command_updates_status_and_clears_input() {
        let (mut app, _) = make_test_app();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("app should start with a ready conversation");
        };
        conversation.input_buffer = ":help".to_string();

        app.start_turn_submission();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert!(conversation.input_buffer.is_empty());
        assert!(
            conversation
                .status_text
                .contains(InlineShellCommand::command_list_line())
        );
    }

    #[test]
    fn transcript_title_includes_transcript_viewport_status() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.sync_transcript_viewport_metrics(18, 6);
        app.scroll_transcript_page_up();

        assert_eq!(
            build_transcript_title(&app, ShellFrontendMode::AlternateScreen).to_string(),
            "Transcript / manual 13/18 / PageUp PageDown / Home End"
        );
    }

    #[test]
    fn inline_history_title_prefers_scrollback_copy() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.sync_transcript_viewport_metrics(18, 6);
        app.scroll_transcript_page_up();

        assert_eq!(
            build_transcript_title(&app, ShellFrontendMode::InlineMainBuffer).to_string(),
            "History / manual 13/18 / scrollback-first"
        );
    }

    #[test]
    fn transcript_panel_view_collects_title_and_scroll_offset() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.sync_transcript_viewport_metrics(18, 6);
        app.scroll_transcript_page_up();
        let transcript_lines = (1..=24)
            .map(|index| Line::from(format!("line {index}")))
            .collect::<Vec<_>>();

        let view = build_transcript_panel_view(
            &mut app,
            ShellFrontendMode::AlternateScreen,
            transcript_lines,
            20,
            6,
        );

        assert_eq!(view.scroll_offset, 13);
        assert_eq!(
            view.title.to_string(),
            "Transcript / manual 13/18 / PageUp PageDown / Home End"
        );
        assert_eq!(view.lines.len(), 24);
    }

    #[test]
    fn composer_title_includes_submit_and_newline_hints() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_input_title(&app, ShellFrontendMode::AlternateScreen).to_string();

        assert!(rendered.contains("Composer / ready"));
        assert!(rendered.contains("Enter send"));
        assert!(rendered.contains("Ctrl+j newline"));
    }

    #[test]
    fn inline_prompt_title_uses_prompt_label() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_input_title(&app, ShellFrontendMode::InlineMainBuffer).to_string();

        assert!(rendered.contains("Prompt / ready"));
        assert!(rendered.contains("Enter send"));
    }

    #[test]
    fn composer_title_shows_readiness_gated_submit_hint() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Loading;
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_input_title(&app, ShellFrontendMode::AlternateScreen).to_string();

        assert!(rendered.contains("Enter send when ready"));
    }

    #[test]
    fn status_title_surfaces_overlay_and_followup_controls() {
        let rendered = build_status_title(ShellFrontendMode::AlternateScreen).to_string();

        assert!(rendered.contains("Ctrl+o sessions"));
        assert!(rendered.contains("Ctrl+d diag"));
        assert!(rendered.contains("Ctrl+p templ"));
        assert!(rendered.contains("Ctrl+a auto"));
    }

    #[test]
    fn inline_status_title_uses_inline_controls_copy() {
        let rendered = build_status_title(ShellFrontendMode::InlineMainBuffer).to_string();

        assert!(rendered.contains("Inline Controls"));
        assert!(rendered.contains("Ctrl+o sessions"));
    }

    #[test]
    fn conversation_shell_view_collects_inline_snapshot_content() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.sync_transcript_viewport_metrics(18, 6);

        let view = build_conversation_shell_view(&app, ShellFrontendMode::InlineMainBuffer);
        let header = view
            .header_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let transcript_title = build_transcript_title(&app, ShellFrontendMode::InlineMainBuffer);

        assert!(view.shell_title.to_string().contains("Inline Shell"));
        assert!(transcript_title.to_string().contains("History /"));
        assert!(view.status_title.to_string().contains("Inline Controls"));
        assert!(view.input_title.to_string().contains("Prompt / ready"));
        assert!(header.contains("thread: thread-1"));
        assert!(header.contains("startup: "));
        assert!(!view.conversation_lines.is_empty());
        assert!(!view.footer_lines.is_empty());
        assert!(!view.input_lines.is_empty());
    }

    #[test]
    fn conversation_shell_frame_view_collects_layout_and_transcript_panel() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let view = build_conversation_shell_frame_view(
            &mut app,
            ShellFrontendMode::AlternateScreen,
            Rect::new(0, 0, 100, 36),
        );

        assert!(view.shell_title.to_string().contains("Shell /"));
        assert_eq!(view.header_area.y, 1);
        assert!(view.transcript_area.height >= 12);
        assert!(view.footer_area.y > view.transcript_area.y);
        assert!(view.input_area.y > view.footer_area.y);
        assert!(
            view.transcript_view
                .title
                .to_string()
                .contains("Transcript /")
        );
        assert!(!view.transcript_view.lines.is_empty());
    }

    #[test]
    fn startup_overlay_view_collects_summary_checks_and_keys() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));

        let view = build_startup_overlay_view(&app);
        let summary = view
            .summary_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let keys = view
            .key_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            view.header_lines[0]
                .to_string()
                .contains("Startup Diagnostics")
        );
        assert!(summary.contains("status: ready"));
        assert!(summary.contains("/tmp/root"));
        assert!(!view.check_items.is_empty());
        assert!(keys.contains("rerun checks"));
    }

    #[test]
    fn session_overlay_view_collects_selected_session_detail_and_keys() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
        app.session_state = SessionState::Ready(RecentSessions {
            items: vec![sample_session("thread-1"), sample_session("thread-2")],
            warnings: Vec::new(),
            next_cursor: None,
        });
        app.selected_session_index = 1;

        let view = build_session_overlay_view(&app);
        let list = view
            .list_view
            .items
            .iter()
            .map(|item| {
                item.lines
                    .iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n---\n");
        let detail = view
            .detail_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let keys = view
            .key_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(view.header_lines[0].to_string().contains("Recent Sessions"));
        assert!(view.list_view.message_lines.is_none());
        assert_eq!(view.list_view.selected_index, Some(1));
        assert!(list.contains("thread-2"));
        assert!(detail.contains("id: thread-2"));
        assert!(detail.contains("/tmp/root/thread-2.json"));
        assert!(keys.contains("Enter: open thread"));
    }

    #[test]
    fn followup_template_overlay_view_collects_preview_status_and_keys() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        let view = build_followup_template_overlay_view(&app);
        let list = view
            .list_view
            .items
            .iter()
            .map(|item| {
                item.lines
                    .iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n---\n");
        let preview = view
            .preview_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let status = view
            .status_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let keys = view
            .key_lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            view.header_lines[0]
                .to_string()
                .contains("Follow-Up Templates")
        );
        assert!(view.list_view.message_lines.is_none());
        assert_eq!(view.list_view.selected_index, Some(0));
        assert!(list.contains("builtin next-task"));
        assert!(preview.contains("Rendered Preview"));
        assert!(status.contains("auto follow-up:"));
        assert!(keys.contains("change template"));
    }

    #[test]
    fn followup_template_preview_renders_selected_template_and_runtime_values() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        let rendered = build_followup_template_preview_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("selected: builtin next-task"));
        assert!(rendered.contains("preview thread id: thread-1"));
        assert!(rendered.contains("latest answer"));
        assert!(rendered.contains("AUTO_STOP"));
        assert!(rendered.contains("Rendered Preview"));
    }

    #[test]
    fn followup_template_preview_uses_placeholder_without_agent_reply() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        let rendered = build_followup_template_preview_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("preview last_message: placeholder"));
        assert!(rendered.contains("(waiting for next agent reply)"));
    }

    #[test]
    fn followup_template_overlay_navigation_updates_selection() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.show_followup_template_overlay();

        assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
        assert_eq!(app.followup_template_selection(), Some(0));

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE,)));

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should stay ready");
        };
        assert_eq!(
            conversation.auto_follow_state.template_label(),
            "builtin plan-queue"
        );
        assert!(conversation.status_text.contains("auto follow-up template"));
        assert_eq!(app.followup_template_selection(), Some(1));
        assert_eq!(app.followup_overlay_ui_state.preview_scroll, 0);
    }

    #[test]
    fn startup_overlay_ctrl_o_opens_sessions_overlay_and_starts_loading_when_ready() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
        app.show_startup_overlay();

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,))
        );

        assert_eq!(app.shell_overlay, ShellOverlay::Sessions);
        assert!(matches!(app.session_state, SessionState::Loading));
    }

    #[test]
    fn sessions_overlay_reload_is_gated_until_startup_ready() {
        let (mut app, _) = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", false));
        app.session_state = SessionState::Failed("load failed".to_string());
        app.show_session_overlay();

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE,))
        );

        assert_eq!(app.shell_overlay, ShellOverlay::Sessions);
        assert!(matches!(
            &app.session_state,
            SessionState::Failed(message) if message == "load failed"
        ));
    }

    #[test]
    fn sessions_overlay_enter_opens_selected_session_and_dismisses_chrome() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.exit_confirmation_state = ExitConfirmationState::Visible;
        app.session_state = SessionState::Ready(RecentSessions {
            items: vec![sample_session("thread-1"), sample_session("thread-2")],
            warnings: Vec::new(),
            next_cursor: None,
        });
        app.selected_session_index = 1;
        app.shell_overlay = ShellOverlay::Sessions;

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(app.exit_confirmation_state, ExitConfirmationState::Hidden);
        assert!(matches!(app.conversation_state, ConversationState::Loading));
        assert_eq!(
            app.active_session
                .as_ref()
                .map(|session| session.id.as_str()),
            Some("thread-2")
        );
    }

    #[test]
    fn sessions_overlay_enter_while_turn_is_streaming_keeps_overlay_visible() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.thread_id = "thread-current".to_string();
        conversation.title = "Streaming thread".to_string();
        conversation.input_state = ConversationInputState::StreamingTurn;
        app.conversation_state = ConversationState::Ready(conversation);
        app.session_state = SessionState::Ready(RecentSessions {
            items: vec![sample_session("thread-2")],
            warnings: Vec::new(),
            next_cursor: None,
        });
        app.shell_overlay = ShellOverlay::Sessions;

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        assert_eq!(app.shell_overlay, ShellOverlay::Sessions);
        assert!(app.active_session.is_none());
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should remain ready");
        };
        assert_eq!(conversation.thread_id, "thread-current");
        assert!(
            conversation
                .status_text
                .contains("wait for completion before switching sessions")
        );
    }

    #[test]
    fn followup_template_overlay_enter_closes_overlay() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.show_followup_template_overlay();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn followup_template_overlay_scroll_keys_update_preview_offset() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.show_followup_template_overlay();

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,))
        );
        assert_eq!(
            app.followup_overlay_ui_state.preview_scroll,
            FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP
        );

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL,))
        );
        assert_eq!(
            app.followup_overlay_ui_state.preview_scroll,
            FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP.saturating_mul(2)
        );

        assert!(
            app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL,))
        );
        assert_eq!(
            app.followup_overlay_ui_state.preview_scroll,
            FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP
        );
    }

    #[test]
    fn ctrl_g_starts_stop_keyword_edit_in_followup_overlay() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());

        app.start_stop_keyword_edit();

        assert_eq!(app.shell_overlay, ShellOverlay::FollowupTemplates);
        assert!(app.is_stop_keyword_editing());
        assert_eq!(
            app.followup_overlay_ui_state.stop_keyword_editor.buffer,
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
    }

    #[test]
    fn stop_keyword_edit_commit_updates_saved_value_and_preview() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);
        app.start_stop_keyword_edit();
        app.followup_overlay_ui_state.stop_keyword_editor.buffer = "DONE".to_string();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should stay ready");
        };
        assert_eq!(conversation.auto_follow_state.stop_keyword_value(), "DONE");
        assert!(!app.is_stop_keyword_editing());

        let rendered = build_followup_template_preview_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("DONE"));
    }

    #[test]
    fn invalid_stop_keyword_edit_keeps_editor_open() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.start_stop_keyword_edit();
        app.followup_overlay_ui_state.stop_keyword_editor.buffer = "two words".to_string();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)));

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("conversation should stay ready");
        };
        assert_eq!(
            conversation.auto_follow_state.stop_keyword_value(),
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
        assert!(app.is_stop_keyword_editing());
        assert!(
            conversation
                .status_text
                .contains("letters, numbers, or underscores")
        );
    }

    #[test]
    fn followup_template_status_lines_include_latest_status_text() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.status_text =
            "auto stop keyword must use only letters, numbers, or underscores".to_string();
        app.conversation_state = ConversationState::Ready(conversation);

        let rendered = build_followup_template_status_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered.contains(
                "status: auto stop keyword must use only letters, numbers, or underscores"
            )
        );
    }

    #[test]
    fn stop_keyword_edit_cancel_restores_saved_value() {
        let (mut app, _) = make_test_app();
        app.conversation_state = ConversationState::Ready(ready_conversation());
        app.start_stop_keyword_edit();
        app.followup_overlay_ui_state.stop_keyword_editor.buffer = "DONE".to_string();

        assert!(app.handle_shell_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)));

        assert!(!app.is_stop_keyword_editing());
        assert_eq!(
            app.followup_overlay_ui_state.stop_keyword_editor.buffer,
            DEFAULT_AUTO_FOLLOW_STOP_KEYWORD
        );
    }

    #[test]
    fn auto_followup_skip_reason_is_visible_in_status_footer() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation
            .auto_follow_state
            .stop_rules
            .stop_on_no_file_changes = true;
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
            },
        ));

        let rendered = build_shell_footer_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("last skip: no file changes"));
        assert!(rendered.contains("detail: the last completed turn changed 0 files"));
    }

    #[test]
    fn auto_followup_queue_clears_previous_skip_reason_from_status_footer() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.last_auto_followup_skip = Some(RecordedAutoFollowupSkip {
            reason: AutoFollowupSkipReason::Disabled,
            detail: "auto follow-up is off; toggle Ctrl+a to re-enable it".to_string(),
        });
        conversation
            .turn_activity
            .last_completed_turn_file_change_count = 2;
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-2".to_string(),
            },
        ));

        let rendered = build_shell_footer_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("last skip: none"));
        assert!(rendered.contains("detail: none"));
    }

    #[test]
    fn recorded_limit_skip_detail_stays_stable_after_progress_resets() {
        let (mut app, _) = make_test_app();
        let mut conversation = ready_conversation();
        conversation.auto_follow_state.completed_auto_turns =
            conversation.auto_follow_state.max_auto_turns;
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            "latest answer",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        ));
        app.conversation_state = ConversationState::Ready(conversation);

        app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-limit".to_string(),
            },
        ));

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("conversation should remain ready");
        };
        conversation.auto_follow_state.completed_auto_turns = 0;

        let rendered = build_shell_footer_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("last skip: turn limit reached"));
        assert!(rendered.contains("detail: reached the configured auto-turn budget (3/3)"));
        assert!(!rendered.contains("detail: reached the configured auto-turn budget (0/3)"));
    }
}
