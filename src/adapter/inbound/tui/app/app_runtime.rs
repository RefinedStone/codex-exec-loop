use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::{
    ParallelModeService,
    control_plane::{
        ParallelModeControlPlaneEffectId, ParallelModeControlPlaneRuntime,
        ParallelModeControlPlaneWorkerEvent,
    },
};
use crate::application::service::planning::PlanningServices;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::github_review::GithubPullRequestPollResult;
use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeDispatchOutcome, ParallelModeReadinessSnapshot,
    ParallelModeSupervisorSnapshot,
};
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;

use super::conversation_runtime::ConversationPostTurnEvaluation;
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
    StartupLoaded(Result<StartupDiagnostics, String>),
    SessionsLoaded(Result<SessionCatalog, String>),
    ConversationLoaded(Result<ConversationSnapshot, String>),
    ConversationStream(ConversationStreamEvent),
    ConversationRuntimeNotice(String),
    OperatorAlert(OperatorAlert),
    InvalidateParallelModeSupervisorSnapshot,
    WakeParallelModeOrchestrator {
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    },
    ParallelModeEnterProgress {
        workspace_directory: String,
        readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
        supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
        status_text: String,
    },
    ParallelModeEntered {
        workspace_directory: String,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
        status_text: String,
        initial_pool_reset_completed: bool,
    },
    ParallelModeSupervisorSnapshotRefreshed {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
    },
    ParallelModeOrchestratorWakeCompleted {
        workspace_directory: String,
        effect_id: ParallelModeControlPlaneEffectId,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
        outcome: ParallelModeDispatchOutcome,
    },
    ParallelModeWorkerEvent(ParallelModeControlPlaneWorkerEvent),
    ParallelModeOrchestratorTickCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        blocked: bool,
        notices: Vec<String>,
    },
    PostTurnEvaluated {
        thread_id: String,
        completed_turn_id: String,
        evaluation: Box<ConversationPostTurnEvaluation>,
        planning_worker_panel_state: super::PlanningWorkerPanelState,
    },
    GithubReviewPollLoaded(Result<GithubPullRequestPollResult, String>),
}

impl NativeTuiApp {
    pub(super) fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
        parallel_mode_service: ParallelModeService,
        planning: PlanningServices,
    ) -> Self {
        let (tx, rx) = mpsc::channel();

        // The first draft is tied to the process working directory so startup can
        // render planning/runtime context before any session is selected.
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
            parallel_mode_initial_pool_reset_completed: false,
            parallel_mode_readiness_snapshot: None,
            parallel_mode_supervisor_snapshot: None,
            supersession_mud_ui_state: super::SupersessionMudUiState::default(),
            parallel_mode_control_plane_runtime: ParallelModeControlPlaneRuntime::new(),
            last_parallel_mode_automation_trigger: None,
            last_parallel_mode_dispatch_withheld_reason: None,
            conversation_state: ConversationState::ready(initial_conversation),
            selected_session_index: 0,
            session_overlay_ui_state: SessionOverlayUiState::new(SESSION_PAGE_SIZE),
            auto_follow_overlay_ui_state: AutoFollowOverlayUiState::default(),
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
            parallel_agent_worker_port,
            turn_control_truth,
            parallel_mode_service,
            planning,
            active_turn_execution_snapshot_capture: None,
            planning_worker_panel_state: super::PlanningWorkerPanelState::default(),
            planning_worker_visibility: super::PlanningWorkerVisibility::from_environment(),
            github_review_poller_service: None,
            github_review_polling_state: super::GithubReviewPollingState::Disabled,
            inline_history_render_mode: super::InlineHistoryRenderMode::from_environment(),
            history_insert_mode: super::HistoryInsertionMode::from_environment(),
            show_startup_ascii_art: startup_ascii_art_enabled_from_environment(),
            tx,
            rx,
        }
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
                // Session overlay requests are scoped to the visible conversation
                // workspace unless the reducer explicitly supplied another root.
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
        let automation_context = self.conversation_runtime_automation_context(&event);
        let Some(conversation) = self.take_ready_conversation_state() else {
            return false;
        };

        let reduction = reduce_conversation_runtime(conversation, event);
        let mut effects = reduction.effects;
        let started_stream = effects
            .iter()
            .any(|effect| matches!(effect, ConversationRuntimeEffect::StartStream { .. }));
        self.conversation_state = ConversationState::ready(reduction.state);
        self.route_conversation_runtime_automation_effects(automation_context, &mut effects);
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
                self.refresh_ready_conversation_planning_runtime_snapshot();
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
