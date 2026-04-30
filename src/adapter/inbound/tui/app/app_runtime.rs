use std::sync::mpsc;
use std::thread;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::PlanningServices;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::github_review::GithubPullRequestPollResult;
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;

use super::conversation_runtime::ConversationPostTurnEvaluation;
use super::{
    ConversationInputEvent, ConversationIntentEffect, ConversationIntentEvent,
    ConversationIntentMode, ConversationIntentState, ConversationLifecycleEffect,
    ConversationLifecycleEvent, ConversationLifecycleState, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, ConversationViewModel, ExitConfirmationState,
    FollowupControlEffect, FollowupControlEvent, FollowupOverlayUiEvent, FollowupOverlayUiState,
    NativeTuiApp, PlanningInitOverlayUiState, SESSION_PAGE_SIZE, SessionOverlayUiState,
    SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState, ShellOverlay,
    StartupState, reduce_conversation_input, reduce_conversation_intents,
    reduce_conversation_lifecycle, reduce_conversation_runtime, reduce_followup_controls,
    reduce_followup_overlay_ui, reduce_shell_chrome, startup_ascii_art_enabled_from_environment,
};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(super) enum BackgroundMessage {
    StartupLoaded(Result<StartupDiagnostics, String>),
    SessionsLoaded(Result<SessionCatalog, String>),
    ConversationLoaded(Result<ConversationSnapshot, String>),
    ConversationStream(ConversationStreamEvent),
    ConversationRuntimeNotice(String),
    InvalidateParallelModeSupervisorSnapshot,
    PostTurnEvaluated {
        thread_id: String,
        queued_from_turn_id: String,
        evaluation: Box<ConversationPostTurnEvaluation>,
        planner_worker_panel_state: super::PlannerWorkerPanelState,
    },
    GithubReviewPollLoaded(Result<GithubPullRequestPollResult, String>),
}

impl NativeTuiApp {
    pub(super) fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_mode_service: ParallelModeService,
        planning: PlanningServices,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let workspace_directory = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let turn_control_truth = conversation_service.runtime_control_truth();
        let mut initial_conversation = ConversationViewModel::new_draft_with_truth(
            workspace_directory.clone(),
            turn_control_truth,
        );
        initial_conversation.replace_planning_runtime_snapshot(
            planning
                .runtime
                .load_runtime_snapshot_or_invalid(&workspace_directory),
        );
        Self {
            shell_overlay: ShellOverlay::Hidden,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            parallel_mode_enabled: false,
            parallel_mode_readiness_snapshot: None,
            parallel_mode_supervisor_snapshot: None,
            conversation_state: ConversationState::ready(initial_conversation),
            selected_session_index: 0,
            session_overlay_ui_state: SessionOverlayUiState::new(SESSION_PAGE_SIZE),
            followup_overlay_ui_state: FollowupOverlayUiState::default(),
            directions_maintenance_overlay_ui_state:
                super::DirectionsMaintenanceOverlayUiState::default(),
            planning_init_overlay_ui_state: PlanningInitOverlayUiState::default(),
            planning_draft_editor_ui_state: super::PlanningDraftEditorUiState::default(),
            task_intake_overlay_ui_state: super::TaskIntakeOverlayUiState::default(),
            pending_task_intake_command: None,
            active_session: None,
            startup_service,
            session_service,
            conversation_service,
            turn_control_truth,
            parallel_mode_service,
            planning,
            active_turn_planning_capture: None,
            planner_worker_panel_state: super::PlannerWorkerPanelState::default(),
            planner_visibility: super::PlannerVisibility::from_environment(),
            github_review_poller_service: None,
            github_review_polling_state: super::GithubReviewPollingState::Disabled,
            inline_history_render_mode: super::InlineHistoryRenderMode::from_environment(),
            history_insert_mode: super::HistoryInsertionMode::from_environment(),
            show_startup_ascii_art: startup_ascii_art_enabled_from_environment(),
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
            ShellChromeEffect::LoadSessionCatalog {
                limit,
                current_workspace_directory,
            } => {
                let tx = self.tx.clone();
                let service = self.session_service.clone();
                let workspace_directory = current_workspace_directory
                    .unwrap_or_else(|| self.current_workspace_directory());
                let request = SessionCatalogRequest::for_workspace(limit, workspace_directory);
                thread::spawn(move || {
                    let result = service
                        .load_session_catalog(request)
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
            turn_control_truth: self.turn_control_truth,
        }
    }

    fn apply_conversation_lifecycle_state(&mut self, state: ConversationLifecycleState) {
        self.conversation_state = state.conversation_state;
        self.active_session = state.active_session;
    }

    pub(super) fn reset_planner_worker_panel_state(&mut self) {
        self.planner_worker_panel_state = super::PlannerWorkerPanelState::default();
    }

    pub(super) fn dispatch_conversation_lifecycle(&mut self, event: ConversationLifecycleEvent) {
        let reduction =
            reduce_conversation_lifecycle(self.take_conversation_lifecycle_state(), event);
        self.apply_conversation_lifecycle_state(reduction.state);
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

    pub(super) fn take_ready_conversation_state(&mut self) -> Option<ConversationViewModel> {
        let state = std::mem::replace(&mut self.conversation_state, ConversationState::Loading);
        match state {
            ConversationState::Ready(conversation) => Some(*conversation),
            other => {
                self.conversation_state = other;
                None
            }
        }
    }

    pub(super) fn dispatch_conversation_runtime(
        &mut self,
        event: ConversationRuntimeEvent,
    ) -> bool {
        let clear_turn_snapshot = matches!(
            &event,
            ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::Failed { .. })
        );
        let should_flush_pending_task_intake = matches!(
            &event,
            ConversationRuntimeEvent::PostTurnEvaluated { .. }
                | ConversationRuntimeEvent::StreamUpdated(ConversationStreamEvent::Failed { .. })
        );
        let Some(conversation) = self.take_ready_conversation_state() else {
            return false;
        };

        let reduction = reduce_conversation_runtime(conversation, event);
        let mut effects = reduction.effects;
        let started_stream = effects
            .iter()
            .any(|effect| matches!(effect, ConversationRuntimeEffect::StartStream { .. }));
        self.conversation_state = ConversationState::ready(reduction.state);
        if clear_turn_snapshot {
            self.active_turn_planning_capture = None;
        }
        if should_flush_pending_task_intake && self.execute_pending_task_intake_command_if_ready() {
            effects.retain(|effect| {
                !matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. })
            });
        }
        for effect in effects {
            self.execute_conversation_runtime_effect(effect);
        }
        started_stream
    }

    pub(super) fn should_apply_post_turn_evaluation(
        &self,
        thread_id: &str,
        queued_from_turn_id: &str,
    ) -> bool {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.accepts_post_turn_evaluation(thread_id, queued_from_turn_id)
            }
            ConversationState::Loading | ConversationState::Failed(_) => false,
        }
    }

    pub(super) fn dispatch_conversation_input(&mut self, event: ConversationInputEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_conversation_input(conversation, event);
        self.conversation_state = ConversationState::ready(reduction.state);
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
            interrupt_support: match &self.conversation_state {
                ConversationState::Ready(conversation) => {
                    conversation.turn_control_truth().interrupt
                }
                ConversationState::Loading | ConversationState::Failed(_) => {
                    self.turn_control_truth.interrupt
                }
            },
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
                self.reset_planner_worker_panel_state();
                let workspace_directory = self.current_workspace_directory();
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
                    workspace_directory: workspace_directory.clone(),
                });
                self.refresh_ready_conversation_planning_runtime_snapshot();
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                    max_auto_turns: self.current_max_auto_turns_label(),
                });
            }
            ConversationIntentEffect::OpenSession { session } => {
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.reset_planner_worker_panel_state();
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
                    session,
                });
            }
            ConversationIntentEffect::ShowExitConfirmation => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
            }
        }
    }

    pub(super) fn dispatch_followup_controls(&mut self, event: FollowupControlEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let reduction = reduce_followup_controls(conversation, event);
        self.conversation_state = ConversationState::ready(reduction.state);
        if !self.is_max_auto_turns_editing() {
            self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::MaxAutoTurnsValueSynced {
                value: self.current_max_auto_turns_label(),
            });
        }
        for effect in reduction.effects {
            self.execute_followup_control_effect(effect);
        }
    }

    fn execute_followup_control_effect(&mut self, effect: FollowupControlEffect) {
        match effect {
            FollowupControlEffect::OverlayUi => {
                self.dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                    max_auto_turns: self.current_max_auto_turns_label(),
                });
            }
            FollowupControlEffect::MaxAutoTurnsEditor { value } => {
                self.dispatch_followup_overlay_ui(
                    FollowupOverlayUiEvent::MaxAutoTurnsEditCommitted {
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
}
