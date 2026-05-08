use crate::adapter::inbound::tui::shell_chrome::{
    ExitConfirmationState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
    ShellOverlay, StartupState, reduce_shell_chrome,
};
use crate::adapter::inbound::tui::supersession_mud::SupersessionMudUiState;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::PlanningExecutionSnapshot;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::PlanningTaskHandoff;
use crate::application::service::planning::PlanningTaskIntakeRequest;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationRuntimeControlTruth,
};
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot,
};
use crate::domain::session_summary::SessionSummary;
use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use std::sync::mpsc::{Receiver, Sender};

/*
 * This module is the native TUI composition root. The implementation is split
 * into focused reducers, presentation builders, runtime bridges, and renderers
 * under app/, while this file keeps the shared constants, module wiring, and
 * state container that those slices extend through sibling impl blocks.
 */

// These defaults define the shell's bounded presentation surface: session lists
// are paged, transcript history is capped, auto-follow is finite unless the
// operator opts into the explicit "infinite" token, and inline panels stay short
// enough to keep the prompt visible.
const SESSION_PAGE_SIZE: usize = 10;
const MAX_CONVERSATION_HISTORY_LINES: usize = 160;
const DEFAULT_AUTO_FOLLOW_MAX_TURNS: usize = 20;
const INFINITE_AUTO_FOLLOW_MAX_TURNS: usize = usize::MAX;
const INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN: &str = "infinite";
const DEFAULT_AUTO_FOLLOW_STOP_KEYWORD: &str = "AUTO_STOP";
const MIN_TRANSCRIPT_PANEL_HEIGHT: u16 = 12;
const MAX_INLINE_TAIL_HEIGHT: u16 = 10;
const INLINE_VIEWPORT_HEIGHT: u16 = 16;
const STARTUP_ASCII_ART_ENV_VAR: &str = "CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART";
const INLINE_HISTORY_RENDER_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_INLINE_HISTORY_MODE";
const HISTORY_INSERT_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_HISTORY_INSERT_MODE";

/*
 * The #[path] list is deliberately flat: each child file owns one reducer,
 * presentation contract, runtime adapter, or UI state machine, but all of them
 * compile as one app module so they can share NativeTuiApp internals without
 * turning every cross-slice field into a public API.
 */
#[path = "app/app_runtime.rs"]
mod app_runtime;
#[path = "app/auto_follow/mod.rs"]
mod auto_follow;
#[path = "app/auto_follow_controls.rs"]
mod auto_follow_controls;
#[path = "app/auto_follow_overlay_ui.rs"]
mod auto_follow_overlay_ui;
#[path = "app/conversation/mod.rs"]
mod conversation;
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
#[path = "app/directions_maintenance_ui.rs"]
mod directions_maintenance_ui;
#[path = "app/github_polling.rs"]
mod github_polling;
#[path = "app/history_insertion.rs"]
mod history_insertion;
#[path = "app/inline_shell_commands.rs"]
mod inline_shell_commands;
#[path = "app/inline_terminal_adapter.rs"]
mod inline_terminal_adapter;
#[path = "app/parallel_mode.rs"]
mod parallel_mode;
#[path = "app/parallel_mode/panel_controller.rs"]
mod parallel_panel_controller;
#[path = "app/planning/mod.rs"]
mod planning;
#[path = "app/planning_draft_editor_ui.rs"]
mod planning_draft_editor_ui;
#[path = "app/planning_init_overlay_ui.rs"]
mod planning_init_overlay_ui;
#[path = "app/planning_worker_debug_preview.rs"]
mod planning_worker_debug_preview;
#[path = "app/post_turn_automation.rs"]
mod post_turn_automation;
#[path = "app/ratatui_frontend.rs"]
mod ratatui_frontend;
#[path = "app/session_overlay_ui.rs"]
mod session_overlay_ui;
#[path = "app/session_shell_controller.rs"]
mod session_shell_controller;
#[path = "app/shell_controller.rs"]
mod shell_controller;
#[path = "app/shell_entrypoint.rs"]
mod shell_entrypoint;
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
#[path = "app/task_intake_overlay_ui.rs"]
mod task_intake_overlay_ui;
#[cfg(test)]
#[path = "app/test_helpers.rs"]
pub(crate) mod test_helpers;
#[path = "app/theme.rs"]
mod theme;
#[cfg(test)]
#[path = "app/tui_testkit.rs"]
mod tui_testkit;
#[path = "app/turn_submission_runtime.rs"]
mod turn_submission_runtime;

// Re-exports below are the narrow surface area sibling modules expect from this
// composition root. Keeping them here makes the dependency graph explicit: app
// slices consume reducer events/effects and presentation types without reaching
// around to unrelated files.
use app_runtime::BackgroundMessage;
use auto_follow_controls::{
    AutoFollowControlEffect, AutoFollowControlEvent, reduce_auto_follow_controls,
};
use auto_follow_overlay_ui::{
    AutoFollowOverlayUiEvent, AutoFollowOverlayUiState, reduce_auto_follow_overlay_ui,
};
use conversation_input::{ConversationInputEvent, reduce_conversation_input};
use conversation_intents::{
    ConversationIntentEffect, ConversationIntentEvent, ConversationIntentMode,
    ConversationIntentState, reduce_conversation_intents,
};
use conversation_lifecycle::{
    ConversationLifecycleEffect, ConversationLifecycleEvent, ConversationLifecycleState,
    reduce_conversation_lifecycle,
};
#[allow(unused_imports)]
pub(super) use conversation_model::{
    AutoFollowDecision, AutoFollowRuntimePhase, AutoFollowSkipReason, AutoFollowState,
    ConversationInputState, ConversationState, ConversationViewModel, StopKeywordRule,
};
use conversation_runtime::{
    ConversationRuntimeEffect, ConversationRuntimeEvent, reduce_conversation_runtime,
};
use directions_maintenance_ui::{
    DetailDocConfirmChoice, DirectionsMaintenanceOverlayStep, DirectionsMaintenanceOverlayUiState,
};
use github_polling::GithubReviewPollingState;
use history_insertion::HistoryInsertionMode;
use inline_shell_commands::{InlineShellCommand, InlineShellCommandInput};
use parallel_panel_controller::{
    ParallelPanelStateController, ParallelPanelUiEvent, ParallelPanelUiState,
};
use planning::{PlanningWorkerPanelState, PlanningWorkerStatus, PlanningWorkerVisibility};
use planning_draft_editor_ui::PlanningDraftEditorUiState;
use planning_init_overlay_ui::{
    PlanningInitDetailSelection, PlanningInitModeSelection, PlanningInitOverlayStep,
    PlanningInitOverlayUiState,
};
use session_overlay_ui::SessionOverlayUiState;
pub(super) use shell_controller::ShellActionAvailability;
pub use shell_entrypoint::run;
use shell_frontend::ShellFrontendMode;
#[cfg(test)]
use shell_presentation::format_conversation_lines;
#[cfg(test)]
use shell_presentation::{build_inline_tail_lines, build_planning_init_overlay_view};
use task_intake_overlay_ui::{TaskIntakeOverlayStep, TaskIntakeOverlayUiState};
use theme::AkraTheme;

// Auto-follow submission carries more than a generated prompt: it records the
// turn that produced the handoff, the transcript line shown to the operator, any
// debug detail, and the planning task identity needed by parallel-mode leasing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoFollowSubmitContext {
    completed_turn_id: String,
    mode_label: String,
    transcript_text: String,
    debug_detail: Option<String>,
    handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManualIntakeSubmitContext {
    transcript_text: String,
    handoff_task: Option<PlanningTaskHandoff>,
}

// Prompt origin is captured at submission time so later stream handling can
// distinguish a manual turn from a reducer-scheduled continuation without
// inferring intent from prompt text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptOrigin {
    Manual,
    ManualIntake(Box<ManualIntakeSubmitContext>),
    AutoFollow(Box<AutoFollowSubmitContext>),
}

// Post-turn reconciliation is tied to the execution snapshot captured for the
// active turn workspace. The capture can fail independently of the conversation
// stream, so the runtime stores either the snapshot or the failure message for
// status/reporting reducers to consume.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTurnExecutionSnapshotCapture {
    workspace_directory: String,
    snapshot: ActiveTurnExecutionSnapshotState,
}
impl ActiveTurnExecutionSnapshotCapture {
    fn ready(workspace_directory: impl Into<String>, snapshot: PlanningExecutionSnapshot) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            snapshot: ActiveTurnExecutionSnapshotState::Ready(snapshot),
        }
    }
    fn capture_failed(workspace_directory: impl Into<String>, message: String) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            snapshot: ActiveTurnExecutionSnapshotState::CaptureFailed(message),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveTurnExecutionSnapshotState {
    Ready(PlanningExecutionSnapshot),
    CaptureFailed(String),
}

// Inline history mode chooses where transcript history is rendered. Host
// scrollback is the normal terminal-friendly path; viewport replay mirrors
// recent transcript rows into the inline tail for environments that need a
// self-contained frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineHistoryRenderMode {
    HostScrollback,
    ViewportReplay,
}
impl InlineHistoryRenderMode {
    fn from_environment() -> Self {
        Self::from_env_values(
            std::env::var(INLINE_HISTORY_RENDER_MODE_ENV_VAR)
                .ok()
                .as_deref(),
        )
    }
    fn from_env_values(mode_value: Option<&str>) -> Self {
        let explicit_mode = mode_value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
        if let Some(explicit_mode) = explicit_mode {
            return match explicit_mode.as_str() {
                "viewport" | "viewport-replay" | "replay" | "mirror" => Self::ViewportReplay,
                _ => Self::HostScrollback,
            };
        }
        Self::HostScrollback
    }
    fn mirrors_recent_transcript_in_tail(self) -> bool {
        matches!(self, Self::ViewportReplay)
    }
    fn writes_host_scrollback(self) -> bool {
        matches!(self, Self::HostScrollback)
    }
}

/*
 * NativeTuiApp is intentionally state-heavy: reducers own the decisions, but
 * the app instance is the integration point that holds shell chrome, active
 * conversation state, service handles, planning state, parallel-mode state, and
 * background message channels. Sibling impl modules mutate this single
 * aggregate so renderers and runtime effects see a coherent snapshot.
 */
struct NativeTuiApp {
    shell_overlay: ShellOverlay,
    exit_confirmation_state: ExitConfirmationState,
    startup_state: StartupState,
    session_state: SessionState,
    parallel_mode_enabled: bool,
    parallel_mode_readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
    parallel_mode_supervisor_snapshot: Option<ParallelModeSupervisorSnapshot>,
    supersession_mud_ui_state: SupersessionMudUiState,
    parallel_mode_supervisor_refresh_in_flight: bool,
    parallel_mode_orchestrator_wake_in_flight: bool,
    parallel_mode_orchestrator_tick_in_flight: bool,
    last_parallel_mode_orchestrator_tick_signature: Option<String>,
    parallel_mode_automation_epoch_id: Option<u64>,
    next_parallel_mode_automation_epoch_id: u64,
    last_parallel_mode_automation_trigger: Option<ParallelModeAutomationTrigger>,
    last_parallel_mode_dispatch_withheld_reason: Option<String>,
    conversation_state: ConversationState,
    selected_session_index: usize,
    session_overlay_ui_state: SessionOverlayUiState,
    auto_follow_overlay_ui_state: AutoFollowOverlayUiState,
    directions_maintenance_overlay_ui_state: DirectionsMaintenanceOverlayUiState,
    planning_init_overlay_ui_state: PlanningInitOverlayUiState,
    planning_draft_editor_ui_state: PlanningDraftEditorUiState,
    task_intake_overlay_ui_state: TaskIntakeOverlayUiState,
    pending_task_intake_command: Option<InlineShellCommandInput>,
    active_session: Option<SessionSummary>,
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    parallel_agent_worker_port: std::sync::Arc<dyn ParallelAgentWorkerPort>,
    turn_control_truth: ConversationRuntimeControlTruth,
    parallel_mode_service: ParallelModeService,
    planning: PlanningServices,
    active_turn_execution_snapshot_capture: Option<ActiveTurnExecutionSnapshotCapture>,
    planning_worker_panel_state: PlanningWorkerPanelState,
    planning_worker_visibility: PlanningWorkerVisibility,
    github_review_poller_service: Option<GithubReviewPollerService>,
    github_review_polling_state: GithubReviewPollingState,
    inline_history_render_mode: InlineHistoryRenderMode,
    history_insert_mode: HistoryInsertionMode,
    show_startup_ascii_art: bool,
    tx: Sender<BackgroundMessage>,
    rx: Receiver<BackgroundMessage>,
}

// Startup ASCII art is opt-out because it is useful in an attached TUI, but it
// can pollute automated captures. Falsey env values disable only the art; they
// do not alter startup checks or shell readiness.
fn startup_ascii_art_enabled_from_environment() -> bool {
    startup_ascii_art_enabled_from_value(std::env::var(STARTUP_ASCII_ART_ENV_VAR).ok().as_deref())
}
fn startup_ascii_art_enabled_from_value(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };

    !matches!(
        value.to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

#[cfg(test)]
mod startup_ascii_art_env_tests {
    use super::{
        InlineHistoryRenderMode, PlanningWorkerVisibility, startup_ascii_art_enabled_from_value,
    };
    #[test]
    fn startup_ascii_art_defaults_to_enabled() {
        assert!(startup_ascii_art_enabled_from_value(None));
        assert!(startup_ascii_art_enabled_from_value(Some("")));
        assert!(startup_ascii_art_enabled_from_value(Some("true")));
        assert!(startup_ascii_art_enabled_from_value(Some("1")));
        assert!(startup_ascii_art_enabled_from_value(Some("yes")));
    }
    #[test]
    fn startup_ascii_art_turns_off_for_falsey_values() {
        assert!(!startup_ascii_art_enabled_from_value(Some("false")));
        assert!(!startup_ascii_art_enabled_from_value(Some("0")));
        assert!(!startup_ascii_art_enabled_from_value(Some("off")));
        assert!(!startup_ascii_art_enabled_from_value(Some("no")));
    }
    #[test]
    fn planning_worker_visibility_defaults_to_normal() {
        assert_eq!(
            PlanningWorkerVisibility::from_env_value(None),
            PlanningWorkerVisibility::Normal
        );
        assert_eq!(
            PlanningWorkerVisibility::from_env_value(Some("")),
            PlanningWorkerVisibility::Normal
        );
        assert_eq!(
            PlanningWorkerVisibility::from_env_value(Some("normal")),
            PlanningWorkerVisibility::Normal
        );
    }
    #[test]
    fn planning_worker_visibility_supports_debug_values() {
        assert_eq!(
            PlanningWorkerVisibility::from_env_value(Some("debug")),
            PlanningWorkerVisibility::Debug
        );
        assert_eq!(
            PlanningWorkerVisibility::from_env_value(Some("TRUE")),
            PlanningWorkerVisibility::Debug
        );
        assert_eq!(
            PlanningWorkerVisibility::from_env_value(Some("verbose")),
            PlanningWorkerVisibility::Debug
        );
    }
    #[test]
    fn inline_history_render_mode_defaults_to_host_scrollback() {
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(None),
            InlineHistoryRenderMode::HostScrollback
        );
    }
    #[test]
    fn inline_history_render_mode_keeps_host_scrollback_for_windows_by_default() {
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(None),
            InlineHistoryRenderMode::HostScrollback
        );
    }
    #[test]
    fn inline_history_render_mode_supports_explicit_override() {
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(Some("scrollback")),
            InlineHistoryRenderMode::HostScrollback
        );
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(Some("viewport-replay")),
            InlineHistoryRenderMode::ViewportReplay
        );
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(Some("mirror")),
            InlineHistoryRenderMode::ViewportReplay
        );
    }
    #[test]
    fn viewport_replay_does_not_write_host_scrollback() {
        assert!(InlineHistoryRenderMode::HostScrollback.writes_host_scrollback());
        assert!(!InlineHistoryRenderMode::ViewportReplay.writes_host_scrollback());
    }
}
