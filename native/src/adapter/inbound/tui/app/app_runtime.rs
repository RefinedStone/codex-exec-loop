use std::sync::mpsc;
use std::thread;

use crate::application::service::conversation_service::ConversationService;
use crate::application::service::followup_template_service::FollowupTemplateService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::github_review::GithubPullRequestPollResult;
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::startup_diagnostics::StartupDiagnostics;

use super::{
    AutoFollowupSubmitContext, ConversationInputEvent, ConversationIntentEffect,
    ConversationIntentEvent, ConversationIntentMode, ConversationIntentState,
    ConversationLifecycleEffect, ConversationLifecycleEvent, ConversationLifecycleState,
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, ConversationViewModel,
    ExitConfirmationState, FollowupControlEffect, FollowupControlEvent, FollowupOverlayUiEvent,
    FollowupOverlayUiState, InlineShellCommand, NativeTuiApp, PromptOrigin, SESSION_PAGE_SIZE,
    SessionOverlayUiState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
    ShellOverlay, StartupState, TranscriptViewportState, reduce_conversation_input,
    reduce_conversation_intents, reduce_conversation_lifecycle, reduce_conversation_runtime,
    reduce_followup_controls, reduce_followup_overlay_ui, reduce_shell_chrome,
};
use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};

#[derive(Debug, Clone)]
pub(super) enum BackgroundMessage {
    StartupLoaded(Result<StartupDiagnostics, String>),
    SessionsLoaded(Result<RecentSessions, String>),
    ConversationLoaded(Result<ConversationSnapshot, String>),
    ConversationStream(ConversationStreamEvent),
    GithubReviewPollLoaded(Result<GithubPullRequestPollResult, String>),
}

impl NativeTuiApp {
    pub(super) fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        followup_template_service: FollowupTemplateService,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let workspace_directory = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let initial_conversation = ConversationState::Ready(ConversationViewModel::new_draft(
            workspace_directory.clone(),
            followup_template_service.load_catalog(&workspace_directory),
        ));
        Self {
            shell_overlay: ShellOverlay::Hidden,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            conversation_state: initial_conversation,
            selected_session_index: 0,
            session_overlay_ui_state: SessionOverlayUiState::new(SESSION_PAGE_SIZE),
            followup_overlay_ui_state: FollowupOverlayUiState::default(),
            transcript_viewport_state: TranscriptViewportState::default(),
            active_session: None,
            startup_service,
            session_service,
            conversation_service,
            followup_template_service,
            github_review_poller_service: None,
            github_review_polling_state: super::GithubReviewPollingState::Disabled,
            tx,
            rx,
        }
    }

    fn take_shell_chrome_state(&mut self) -> ShellChromeState {
        ShellChromeState {
            shell_overlay: self.shell_overlay,
            exit_confirmation_state: self.exit_confirmation_state,
            startup_state: std::mem::replace(&mut self.startup_state, StartupState::Idle),
            session_state: std::mem::replace(&mut self.session_state, SessionState::Idle),
            selected_session_index: self.selected_session_index,
        }
    }

    fn apply_shell_chrome_state(&mut self, state: ShellChromeState) {
        self.shell_overlay = state.shell_overlay;
        self.exit_confirmation_state = state.exit_confirmation_state;
        self.startup_state = state.startup_state;
        self.session_state = state.session_state;
        self.selected_session_index = state.selected_session_index;
    }

    pub(super) fn dispatch_shell_chrome(&mut self, event: ShellChromeEvent) {
        let reduction = reduce_shell_chrome(self.take_shell_chrome_state(), event);
        self.apply_shell_chrome_state(reduction.state);
        for effect in reduction.effects {
            self.execute_shell_chrome_effect(effect);
        }
    }

    fn execute_shell_chrome_effect(&mut self, effect: ShellChromeEffect) {
        match effect {
            ShellChromeEffect::RunStartupChecks => {
                let tx = self.tx.clone();
                let service = self.startup_service.clone();
                thread::spawn(move || {
                    let result = service.run_checks().map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::StartupLoaded(result));
                });
            }
            ShellChromeEffect::LoadRecentSessions { limit } => {
                let tx = self.tx.clone();
                let service = self.session_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_recent_sessions(limit)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::SessionsLoaded(result));
                });
            }
        }
    }

    fn take_conversation_lifecycle_state(&mut self) -> ConversationLifecycleState {
        ConversationLifecycleState {
            conversation_state: std::mem::replace(
                &mut self.conversation_state,
                ConversationState::Loading,
            ),
            active_session: self.active_session.take(),
        }
    }

    fn apply_conversation_lifecycle_state(&mut self, state: ConversationLifecycleState) {
        self.conversation_state = state.conversation_state;
        self.active_session = state.active_session;
    }

    pub(super) fn dispatch_conversation_lifecycle(&mut self, event: ConversationLifecycleEvent) {
        let reduction =
            reduce_conversation_lifecycle(self.take_conversation_lifecycle_state(), event);
        self.apply_conversation_lifecycle_state(reduction.state);
        self.reset_transcript_viewport();
        for effect in reduction.effects {
            self.execute_conversation_lifecycle_effect(effect);
        }
    }

    fn execute_conversation_lifecycle_effect(&mut self, effect: ConversationLifecycleEffect) {
        match effect {
            ConversationLifecycleEffect::LoadConversation { thread_id } => {
                let tx = self.tx.clone();
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let result = service
                        .load_snapshot(&thread_id)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(BackgroundMessage::ConversationLoaded(result));
                });
            }
        }
    }

    pub(super) fn start_turn_submission(&mut self) {
        let inline_command = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommand::parse(&conversation.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            self.execute_inline_shell_command(command);
            return;
        }

        let prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.can_submit_prompt() => {
                conversation.input_buffer.trim().to_string()
            }
            _ => return,
        };
        if prompt.is_empty() {
            return;
        }
        self.submit_prompt(prompt, PromptOrigin::Manual);
    }

    fn take_ready_conversation_state(&mut self) -> Option<ConversationViewModel> {
        let state = std::mem::replace(&mut self.conversation_state, ConversationState::Loading);
        match state {
            ConversationState::Ready(conversation) => Some(conversation),
            other => {
                self.conversation_state = other;
                None
            }
        }
    }

    pub(super) fn dispatch_conversation_runtime(&mut self, event: ConversationRuntimeEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_conversation_runtime(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
        for effect in reduction.effects {
            self.execute_conversation_runtime_effect(effect);
        }
    }

    pub(super) fn dispatch_conversation_input(&mut self, event: ConversationInputEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_conversation_input(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
    }

    pub(super) fn clear_input_buffer(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::InputCleared);
    }

    fn conversation_intent_state(&self) -> ConversationIntentState {
        let mode = match &self.conversation_state {
            ConversationState::Loading => ConversationIntentMode::Loading,
            ConversationState::Failed(_) => ConversationIntentMode::Failed,
            ConversationState::Ready(conversation) if conversation.is_blank_draft() => {
                ConversationIntentMode::BlankDraft
            }
            ConversationState::Ready(_) => ConversationIntentMode::Ready,
        };

        ConversationIntentState {
            has_running_turn: self.conversation_has_running_turn(),
            mode,
        }
    }

    pub(super) fn dispatch_conversation_intent(&mut self, event: ConversationIntentEvent) {
        let reduction = reduce_conversation_intents(self.conversation_intent_state(), event);
        for effect in reduction.effects {
            self.execute_conversation_intent_effect(effect);
        }
    }

    fn execute_conversation_intent_effect(&mut self, effect: ConversationIntentEffect) {
        match effect {
            ConversationIntentEffect::ShowStatus { status_text } => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ConversationIntentEffect::OpenNewDraft => {
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                let workspace_directory = self.current_workspace_directory();
                let template_load_result =
                    self.load_followup_template_catalog(&workspace_directory);
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
                    workspace_directory: workspace_directory.clone(),
                    template_load_result,
                });
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                    stop_keyword: self.current_stop_keyword_value(),
                    max_auto_turns: self.current_max_auto_turns_value().to_string(),
                });
            }
            ConversationIntentEffect::OpenSession { session } => {
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
                    session,
                });
            }
            ConversationIntentEffect::ShowExitConfirmation => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
            }
        }
    }

    fn execute_conversation_runtime_effect(&mut self, effect: ConversationRuntimeEffect) {
        match effect {
            ConversationRuntimeEffect::StartStream {
                cwd,
                thread_id,
                prompt,
            } => {
                let outer_tx = self.tx.clone();
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let (event_tx, event_rx) = mpsc::channel();

                    let service_thread = thread::spawn(move || {
                        let result = match thread_id {
                            Some(thread_id) => {
                                service.run_turn_stream(&thread_id, &prompt, event_tx)
                            }
                            None => service.run_new_thread_stream(&cwd, &prompt, event_tx),
                        };
                        let _ = result;
                    });

                    while let Ok(event) = event_rx.recv() {
                        let should_stop = matches!(
                            event,
                            ConversationStreamEvent::TurnCompleted { .. }
                                | ConversationStreamEvent::Failed { .. }
                        );
                        let _ = outer_tx.send(BackgroundMessage::ConversationStream(event));
                        if should_stop {
                            break;
                        }
                    }

                    let _ = service_thread.join();
                });
            }
            ConversationRuntimeEffect::QueueAutoPrompt {
                prompt,
                queued_from_turn_id,
                template_label,
            } => {
                self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(AutoFollowupSubmitContext {
                        queued_from_turn_id,
                        template_label,
                    }),
                );
            }
        }
    }

    pub(super) fn dispatch_followup_controls(&mut self, event: FollowupControlEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_followup_controls(conversation, event);
        self.conversation_state = ConversationState::Ready(reduction.state);
        if !self.is_max_auto_turns_editing() {
            self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsValueSynced {
                value: self.current_max_auto_turns_value().to_string(),
            });
        }
        if !self.is_stop_keyword_editing() {
            self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::StopKeywordValueSynced {
                value: self.current_stop_keyword_value(),
            });
        }
        for effect in reduction.effects {
            self.execute_followup_control_effect(effect);
        }
    }

    fn execute_followup_control_effect(&mut self, effect: FollowupControlEffect) {
        match effect {
            FollowupControlEffect::SyncTemplateOverlayUi => {
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::TemplateChanged);
            }
            FollowupControlEffect::SyncMaxAutoTurnsEditor { value } => {
                self.dispatch_followup_overlay_ui(
                    FollowupOverlayUiEvent::MaxAutoTurnsEditCommitted {
                        current_value: value,
                    },
                );
            }
            FollowupControlEffect::SyncStopKeywordEditor { value } => {
                self.dispatch_followup_overlay_ui(
                    FollowupOverlayUiEvent::StopKeywordEditCommitted {
                        current_value: value,
                    },
                );
            }
        }
    }

    pub(super) fn dispatch_followup_overlay_ui(&mut self, event: FollowupOverlayUiEvent) {
        let state = std::mem::take(&mut self.followup_overlay_ui_state);
        self.followup_overlay_ui_state = reduce_followup_overlay_ui(state, event);
    }

    pub(super) fn resolve_startup_submit_queue(&mut self) {
        let (startup_submit_armed, prompt) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                conversation.startup_submit_armed,
                conversation.input_buffer.trim().to_string(),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };
        if !startup_submit_armed {
            return;
        }

        match self.shell_action_availability() {
            super::ShellActionAvailability::Ready if prompt.is_empty() => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: None,
                });
            }
            super::ShellActionAvailability::Ready => {
                self.submit_prompt(prompt, PromptOrigin::Manual);
            }
            super::ShellActionAvailability::Pending => {}
            super::ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: Some(format!(
                        "{}; queued prompt kept in buffer",
                        self.submission_blocked_status(PromptOrigin::Manual)
                    )),
                });
            }
        }
    }

    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) {
        if matches!(prompt_origin, PromptOrigin::Manual)
            && matches!(
                self.shell_action_availability(),
                super::ShellActionAvailability::Pending
            )
        {
            self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            });
            return;
        }

        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.submission_blocked_status(prompt_origin),
            });
            return;
        }

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            origin: prompt_origin,
        });
    }
}
