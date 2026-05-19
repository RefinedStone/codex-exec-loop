use std::sync::mpsc;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneComposition,
    ParallelModeControlPlaneEventSink, ParallelModeControlPlaneHandle,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
#[cfg(test)]
use crate::application::service::planning::PlanningTaskToolUseCases;
use crate::application::service::planning::{
    PlanningRuntimeUseCases, PlanningServices, PlanningTurnExecutionSnapshotCapture,
    PlanningWorkspaceUseCases,
};
#[cfg(test)]
use crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::core_effect_runner::CoreEffectRunner;
#[cfg(test)]
use crate::core::app::StartupReadySnapshot;
use crate::core::app::{
    AppCommand, AppEvent, ConversationSnapshot as CoreConversationSnapshot, CoreDispatchOutcome,
    CoreInput, SessionCatalogSnapshot, StartupSnapshot, TurnStreamEvent,
};
use crate::core::runtime::CoreRuntime;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::github_review::GithubPullRequestPollResult;
use crate::domain::operator_alert::OperatorAlert;

use super::{
    AutoFollowControlEffect, AutoFollowControlEvent, AutoFollowOverlayUiEvent,
    AutoFollowOverlayUiState, ConversationInputEvent, ConversationIntentEffect,
    ConversationIntentEvent, ConversationIntentMode, ConversationIntentState,
    ConversationLifecycleEffect, ConversationLifecycleEvent, ConversationLifecycleState,
    ConversationRuntimeEffect, ConversationRuntimeEvent, ConversationState, ConversationViewModel,
    ExitConfirmationState, NativeTuiApp, PlanningInitOverlayUiState, SESSION_PAGE_SIZE,
    SessionOverlayUiState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
    ShellOverlay, StartupState, reduce_auto_follow_controls, reduce_auto_follow_overlay_ui,
    reduce_conversation_input, reduce_conversation_intents, reduce_conversation_lifecycle,
    reduce_conversation_runtime, reduce_shell_chrome, startup_ascii_art_enabled_from_environment,
};

/* NativeTuiApp is assembled as reducer-owned state plus outbound service handles.
 * Runtime files keep pure reducers away from threads and ports: reducers return
 * effects, this module turns those effects into background messages, and
 * ShellRuntime later drains those messages back into reducers.
 */
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(super) enum BackgroundMessage {
    #[cfg(test)]
    StartupLoaded(Result<Box<StartupReadySnapshot>, String>),
    #[cfg(test)]
    ConversationLoaded(Result<ConversationSnapshot, String>),
    ConversationStream(ConversationStreamEvent),
    ConversationTurnCompleted {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: PlanningTurnExecutionSnapshotCapture,
    },
    ConversationRuntimeNotice(String),
    OperatorAlert(OperatorAlert),
    InvalidateParallelModeSupervisorSnapshot,
    ParallelModeControlPlaneEvent(ParallelModeControlPlaneBackgroundEvent),
    #[cfg(test)]
    PostTurnEvaluationCompleted(Box<PostTurnEvaluationExecution>),
    GithubReviewPollLoaded(Result<GithubPullRequestPollResult, String>),
}

#[derive(Clone)]
pub(super) struct TuiParallelModeControlPlaneEventSink {
    tx: mpsc::Sender<BackgroundMessage>,
}

impl ParallelModeControlPlaneEventSink for TuiParallelModeControlPlaneEventSink {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent) {
        let _ = self
            .tx
            .send(BackgroundMessage::ParallelModeControlPlaneEvent(event));
    }
}

pub(super) struct NativeTuiAppRuntimeChannels {
    tx: mpsc::Sender<BackgroundMessage>,
    rx: mpsc::Receiver<BackgroundMessage>,
}

impl NativeTuiAppRuntimeChannels {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    pub(super) fn parallel_mode_event_sink(&self) -> TuiParallelModeControlPlaneEventSink {
        TuiParallelModeControlPlaneEventSink {
            tx: self.tx.clone(),
        }
    }
}

pub(super) fn core_turn_stream_event_from_application(
    event: ConversationStreamEvent,
) -> TurnStreamEvent {
    match event {
        ConversationStreamEvent::AttachmentObserved { profile } => {
            TurnStreamEvent::AttachmentObserved { profile }
        }
        ConversationStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
        } => TurnStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
        },
        ConversationStreamEvent::TurnStarted { turn_id } => {
            TurnStreamEvent::TurnStarted { turn_id }
        }
        ConversationStreamEvent::StatusUpdated { text } => TurnStreamEvent::StatusUpdated { text },
        ConversationStreamEvent::AgentMessageDelta {
            item_id,
            phase,
            delta,
        } => TurnStreamEvent::AgentMessageDelta {
            item_id,
            phase,
            delta,
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        } => TurnStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        },
        ConversationStreamEvent::ToolActivity { activity } => {
            TurnStreamEvent::ToolActivity { activity }
        }
        ConversationStreamEvent::ApprovalReviewUpdated { review } => {
            TurnStreamEvent::ApprovalReviewUpdated { review }
        }
        ConversationStreamEvent::TurnCompleted {
            turn_id,
            changed_planning_file_paths,
        } => TurnStreamEvent::TurnCompleted {
            turn_id,
            changed_planning_file_paths,
        },
        ConversationStreamEvent::Failed { message } => TurnStreamEvent::Failed { message },
    }
}

#[derive(Clone)]
pub(super) struct NativeTuiApplicationHandle {
    conversations: NativeTuiConversationHandle,
    planning_feature: NativeTuiPlanningHandle,
}

impl NativeTuiApplicationHandle {
    fn new(conversations: ConversationService, planning_feature: PlanningServices) -> Self {
        Self {
            conversations: NativeTuiConversationHandle::new(conversations),
            planning_feature: NativeTuiPlanningHandle::new(planning_feature),
        }
    }

    pub(super) fn planning(&self) -> &NativeTuiPlanningHandle {
        &self.planning_feature
    }

    pub(super) fn runtime_control_truth(&self) -> super::ConversationRuntimeControlTruth {
        self.conversations.runtime_control_truth()
    }

    pub(super) fn request_stop_all_sessions(&self) -> Result<(), String> {
        self.conversations.request_stop_all_sessions()
    }

    pub(super) fn load_conversation_snapshot(
        &self,
        thread_id: &str,
    ) -> Result<ConversationSnapshot, String> {
        self.conversations.load_snapshot(thread_id)
    }
}

#[derive(Clone)]
pub(super) struct NativeTuiConversationHandle {
    service: ConversationService,
}

impl NativeTuiConversationHandle {
    fn new(service: ConversationService) -> Self {
        Self { service }
    }

    pub(super) fn runtime_control_truth(&self) -> super::ConversationRuntimeControlTruth {
        self.service.runtime_control_truth()
    }

    pub(super) fn request_stop_all_sessions(&self) -> Result<(), String> {
        self.service
            .request_stop_all_sessions()
            .map_err(|error| error.to_string())
    }

    pub(super) fn load_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot, String> {
        self.service
            .load_snapshot(thread_id)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
pub(super) struct NativeTuiPlanningHandle {
    services: PlanningServices,
}

impl NativeTuiPlanningHandle {
    fn new(services: PlanningServices) -> Self {
        Self { services }
    }

    pub(super) fn workspace(&self) -> &PlanningWorkspaceUseCases {
        &self.services.workspace
    }

    pub(super) fn runtime(&self) -> &PlanningRuntimeUseCases {
        &self.services.runtime
    }

    #[cfg(test)]
    pub(super) fn task_tool(&self) -> &PlanningTaskToolUseCases {
        &self.services.task_tool
    }
}

pub(crate) struct NativeTuiParallelModeBinding {
    parallel_turns: ParallelModeTurnService,
    planning_feature: PlanningServices,
    parallel_mode_control_plane:
        ParallelModeControlPlaneHandle<TuiParallelModeControlPlaneEventSink>,
    runtime_channels: NativeTuiAppRuntimeChannels,
}

impl NativeTuiParallelModeBinding {
    pub(crate) fn from_composition(
        composition: ParallelModeControlPlaneComposition,
    ) -> NativeTuiParallelModeBinding {
        let runtime_channels = NativeTuiAppRuntimeChannels::new();
        let parallel_mode_control_plane =
            composition.bind_event_sink(runtime_channels.parallel_mode_event_sink());
        NativeTuiParallelModeBinding {
            parallel_turns: composition.parallel_mode_turn_service(),
            planning_feature: composition.planning().clone(),
            parallel_mode_control_plane,
            runtime_channels,
        }
    }
}

impl NativeTuiApp {
    pub(super) fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_mode_binding: NativeTuiParallelModeBinding,
    ) -> Self {
        let NativeTuiParallelModeBinding {
            parallel_turns,
            planning_feature,
            parallel_mode_control_plane,
            runtime_channels,
        } = parallel_mode_binding;
        let (core_input_sender, core_input_receiver) = mpsc::channel();
        let core_effect_runner = CoreEffectRunner::new(
            startup_service.clone(),
            session_service.clone(),
            conversation_service.clone(),
            planning_feature.clone(),
            parallel_turns.clone(),
            PostTurnEvaluationService::new(planning_feature.clone(), parallel_turns.clone()),
            core_input_sender,
        );
        let core_runtime = CoreRuntime::new(core_effect_runner, core_input_receiver);
        let application = NativeTuiApplicationHandle::new(conversation_service, planning_feature);

        // The first draft is tied to the process working directory so startup can
        // render planning/runtime context before any session is selected.
        let workspace_directory = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let turn_control_truth = application.runtime_control_truth();
        let mut initial_conversation = ConversationViewModel::new_draft_with_truth(
            workspace_directory.clone(),
            turn_control_truth,
        );
        let initial_planning_runtime_projection = application
            .planning()
            .runtime()
            .load_runtime_projection_or_invalid(&workspace_directory);
        initial_conversation
            .replace_reducer_event_projection_cache(initial_planning_runtime_projection.clone());
        let mut app = Self {
            shell_overlay: ShellOverlay::Hidden,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            supersession_mud_ui_state: super::SupersessionMudUiState::default(),
            parallel_peek_overlay_ui_state: super::ParallelPeekOverlayUiState::default(),
            parallel_supervisor_event_log: super::ParallelSupervisorEventLog::default(),
            pending_manual_prompt_preparation: None,
            parallel_mode_control_plane,
            conversation_state: ConversationState::ready(initial_conversation),
            selected_session_index: 0,
            session_overlay_ui_state: SessionOverlayUiState::new(SESSION_PAGE_SIZE),
            tui_language: super::TuiLanguage::default(),
            language_selection_overlay_ui_state: super::LanguageSelectionOverlayUiState::default(),
            model_selection_overlay_ui_state: super::ModelSelectionOverlayUiState::default(),
            view_selection_overlay_ui_state: super::ViewSelectionOverlayUiState::default(),
            auto_follow_overlay_ui_state: AutoFollowOverlayUiState::default(),
            directions_maintenance_overlay_ui_state:
                super::DirectionsMaintenanceOverlayUiState::default(),
            planning_init_overlay_ui_state: PlanningInitOverlayUiState::default(),
            planning_draft_editor_ui_state: super::PlanningDraftEditorUiState::default(),
            active_session: None,
            application,
            core_runtime,
            turn_control_truth,
            turn_options: Default::default(),
            conversation_view_mode: super::ConversationViewMode::default(),
            planning_worker_panel_state: super::PlanningWorkerPanelState::default(),
            planning_worker_visibility: super::PlanningWorkerVisibility::from_environment(),
            github_review_poller_service: None,
            github_review_polling_state: super::GithubReviewPollingState::Disabled,
            inline_history_render_mode: super::InlineHistoryRenderMode::from_environment(),
            history_insert_mode: super::HistoryInsertionMode::from_environment(),
            show_startup_ascii_art: startup_ascii_art_enabled_from_environment(),
            tx: runtime_channels.tx,
            rx: runtime_channels.rx,
        };
        app.sync_core_planning_runtime_projection(initial_planning_runtime_projection);
        app
    }

    // Shell chrome state is split across NativeTuiApp fields for ergonomic access by
    // renderers, then reassembled here so the reducer still owns one coherent value.
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

    pub(super) fn poll_core_runtime_inputs(&mut self, max_inputs: usize) -> bool {
        let outcomes = self.core_runtime.drain_pending_inputs(max_inputs);
        let changed = !outcomes.is_empty();
        for outcome in outcomes {
            self.apply_core_dispatch_outcome(outcome);
        }
        changed
    }

    fn apply_core_dispatch_outcome(&mut self, outcome: CoreDispatchOutcome) {
        for event in outcome.events {
            self.apply_core_event(event);
        }
    }

    fn apply_core_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::StartupChanged(StartupSnapshot::Idle) => {
                self.startup_state = StartupState::Idle;
            }
            AppEvent::StartupChanged(StartupSnapshot::Loading) => {
                self.startup_state = StartupState::Loading;
            }
            AppEvent::StartupChanged(StartupSnapshot::Ready(ready)) => {
                let workspace_directory = ready.workspace_path.clone();
                self.dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                    result: Ok(ready),
                    session_page_size: SESSION_PAGE_SIZE,
                });
                self.sync_draft_shell_workspace(&workspace_directory);
                self.resolve_startup_submit_queue();
            }
            AppEvent::StartupChanged(StartupSnapshot::Failed { message }) => {
                self.dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                    result: Err(message),
                    session_page_size: SESSION_PAGE_SIZE,
                });
                self.resolve_startup_submit_queue();
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Idle) => {
                self.session_state = SessionState::Idle;
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Loading) => {
                self.session_state = SessionState::Loading;
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Ready(ready)) => {
                self.dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(Ok(*ready.catalog)));
                self.session_overlay_ui_state.reset();
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Failed { message }) => {
                self.dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(Err(message)));
                self.session_overlay_ui_state.reset();
            }
            AppEvent::ConversationChanged(snapshot) => {
                self.apply_core_conversation_snapshot(snapshot);
            }
            AppEvent::TurnStreamSnapshotChanged(stream_snapshot) => {
                self.dispatch_conversation_runtime(
                    ConversationRuntimeEvent::StreamSnapshotApplied(Box::new(stream_snapshot)),
                );
            }
            AppEvent::ManualPromptPrepared(result) => {
                self.apply_manual_prompt_preparation(*result);
            }
            AppEvent::PostTurnEvaluationCompleted(execution) => {
                self.apply_post_turn_evaluation_execution(*execution);
            }
            AppEvent::ConversationTurnWorkspaceChanged {
                workspace_directory,
            } => {
                self.sync_active_turn_workspace_directory(&workspace_directory);
            }
            AppEvent::ParallelModeSupervisorSnapshotInvalidated => {
                self.invalidate_parallel_mode_supervisor_snapshot();
            }
            AppEvent::SnapshotChanged(_) => {}
        }
    }

    pub(super) fn dispatch_core_command(&mut self, command: AppCommand) {
        let outcome = self.core_runtime.dispatch_command(command);
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn dispatch_core_input(&mut self, input: CoreInput) {
        let outcome = self.core_runtime.dispatch_input(input);
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn apply_core_conversation_snapshot(&mut self, snapshot: CoreConversationSnapshot) {
        let loaded_successfully = matches!(&snapshot, CoreConversationSnapshot::Ready(_));
        let load_finished = matches!(
            &snapshot,
            CoreConversationSnapshot::Ready(_) | CoreConversationSnapshot::Failed { .. }
        );
        if matches!(&snapshot, CoreConversationSnapshot::Loading) {
            self.reset_planning_worker_panel_state();
        }
        let draft_workspace_directory = self.current_workspace_directory();
        self.dispatch_conversation_lifecycle(
            ConversationLifecycleEvent::CoreConversationSnapshotApplied {
                snapshot,
                draft_workspace_directory,
            },
        );
        if !load_finished {
            return;
        }
        self.refresh_ready_conversation_planning_runtime_projection();
        if loaded_successfully {
            self.surface_resumed_session_planning_context();
        }
        // A loaded conversation resets follow-up copy because auto-turn affordances
        // belong to the active thread, not the previous shell contents.
        self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::ContentReset {
            max_auto_turns: self.current_max_auto_turns_label(),
        });
    }

    fn execute_shell_chrome_effect(&mut self, effect: ShellChromeEffect) {
        match effect {
            ShellChromeEffect::RunStartupChecks => {
                self.dispatch_core_command(AppCommand::RunStartupChecks);
            }
            ShellChromeEffect::LoadSessionCatalog {
                limit,
                current_workspace_directory,
            } => {
                // Session overlay requests are scoped to the visible conversation
                // workspace unless the reducer explicitly supplied another root.
                let workspace_directory = current_workspace_directory
                    .unwrap_or_else(|| self.current_workspace_directory());
                self.dispatch_core_command(AppCommand::LoadSessionCatalog {
                    limit,
                    workspace_directory,
                });
            }
        }
    }

    // Moving the conversation out prevents accidental partial mutation when lifecycle
    // reducers decide between loading, failed, and ready session states.
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

    pub(super) fn reset_planning_worker_panel_state(&mut self) {
        self.planning_worker_panel_state = super::PlanningWorkerPanelState::default();
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
                self.dispatch_core_command(AppCommand::LoadConversation { thread_id });
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
        let post_turn_context = self.post_turn_continuation_context(&event);
        let Some(conversation) = self.take_ready_conversation_state() else {
            return false;
        };

        let previous_planning_runtime_projection =
            conversation.reducer_event_projection_cache().clone();
        let reduction = reduce_conversation_runtime(conversation, event);
        let next_planning_runtime_projection = (previous_planning_runtime_projection
            != *reduction.state.reducer_event_projection_cache())
        .then(|| reduction.state.reducer_event_projection_cache().clone());
        let mut effects = reduction.effects;
        let started_stream = effects
            .iter()
            .any(|effect| matches!(effect, ConversationRuntimeEffect::StartStream { .. }));
        self.conversation_state = ConversationState::ready(reduction.state);
        if let Some(runtime_projection) = next_planning_runtime_projection {
            self.sync_core_planning_runtime_projection(runtime_projection);
        }
        self.route_post_turn_continuation_effects(post_turn_context, &mut effects);
        for effect in effects {
            self.execute_conversation_runtime_effect(effect);
        }
        started_stream
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
                // New drafts must leave transient chrome and planning worker context behind;
                // otherwise the blank prompt can inherit stale session-side affordances.
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.reset_planning_worker_panel_state();
                let workspace_directory = self.current_workspace_directory();
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
                    workspace_directory: workspace_directory.clone(),
                });
                self.refresh_ready_conversation_planning_runtime_projection();
                self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::ContentReset {
                    max_auto_turns: self.current_max_auto_turns_label(),
                });
            }
            ConversationIntentEffect::OpenSession { session } => {
                // Session selection is a lifecycle transition, not just a transcript swap.
                // Reset planning side panels before the async load result returns.
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.reset_planning_worker_panel_state();
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
                    session,
                });
            }
            ConversationIntentEffect::ShowExitConfirmation => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
            }
        }
    }

    pub(super) fn dispatch_auto_follow_controls(&mut self, event: AutoFollowControlEvent) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };
        let reduction = reduce_auto_follow_controls(conversation, event);
        self.conversation_state = ConversationState::ready(reduction.state);
        if !self.is_max_auto_turns_editing() {
            self.dispatch_auto_follow_overlay_ui(
                AutoFollowOverlayUiEvent::MaxAutoTurnsValueSynced {
                    value: self.current_max_auto_turns_label(),
                },
            );
        }
        for effect in reduction.effects {
            self.execute_auto_follow_control_effect(effect);
        }
    }

    fn execute_auto_follow_control_effect(&mut self, effect: AutoFollowControlEffect) {
        match effect {
            AutoFollowControlEffect::OverlayUi => {
                self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::ContentReset {
                    max_auto_turns: self.current_max_auto_turns_label(),
                });
            }
            AutoFollowControlEffect::MaxAutoTurnsEditor { value } => {
                self.dispatch_auto_follow_overlay_ui(
                    AutoFollowOverlayUiEvent::MaxAutoTurnsEditCommitted {
                        current_value: value,
                    },
                );
            }
        }
    }

    pub(super) fn dispatch_auto_follow_overlay_ui(&mut self, event: AutoFollowOverlayUiEvent) {
        let state = std::mem::take(&mut self.auto_follow_overlay_ui_state);
        self.auto_follow_overlay_ui_state = reduce_auto_follow_overlay_ui(state, event);
    }
}
