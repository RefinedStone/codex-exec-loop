use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;

use crate::adapter::inbound::tui::shell_chrome::{
    ExitConfirmationState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
    ShellOverlay, StartupState, reduce_shell_chrome,
};
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
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::session_summary::SessionSummary;

const SESSION_PAGE_SIZE: usize = 10;
const MAX_CONVERSATION_HISTORY_LINES: usize = 160;
const DEFAULT_AUTO_FOLLOW_MAX_TURNS: usize = 20;
const INFINITE_AUTO_FOLLOW_MAX_TURNS: usize = usize::MAX;
const INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN: &str = "infinite";
const DEFAULT_AUTO_FOLLOW_STOP_KEYWORD: &str = "AUTO_STOP";
const FOLLOWUP_TEMPLATE_PREVIEW_SCROLL_STEP: u16 = 6;
#[cfg(test)]
const SHELL_FRAME_MARGIN: u16 = 1;
#[cfg(test)]
const MIN_SHELL_HEADER_HEIGHT: u16 = 4;
#[cfg(test)]
const MAX_SHELL_HEADER_HEIGHT: u16 = 6;
const MIN_TRANSCRIPT_PANEL_HEIGHT: u16 = 12;
#[cfg(test)]
const MIN_SHELL_STATUS_HEIGHT: u16 = 5;
#[cfg(test)]
const MAX_SHELL_STATUS_HEIGHT: u16 = 8;
#[cfg(test)]
const MIN_COMPOSER_HEIGHT: u16 = 4;
#[cfg(test)]
const MAX_COMPOSER_HEIGHT: u16 = 8;
const MAX_INLINE_TAIL_HEIGHT: u16 = 10;
const INLINE_VIEWPORT_HEIGHT: u16 = 16;
const STARTUP_ASCII_ART_ENV_VAR: &str = "CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART";
const INLINE_HISTORY_RENDER_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_INLINE_HISTORY_MODE";

#[path = "app/app_runtime.rs"]
mod app_runtime;
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
#[path = "app/followup/mod.rs"]
mod followup;
#[path = "app/followup_controls.rs"]
mod followup_controls;
#[path = "app/followup_overlay_ui.rs"]
mod followup_overlay_ui;
#[path = "app/github_polling.rs"]
mod github_polling;
#[path = "app/inline_shell_commands.rs"]
mod inline_shell_commands;
#[path = "app/parallel_mode.rs"]
mod parallel_mode;
#[path = "app/planner_debug_preview.rs"]
mod planner_debug_preview;
#[path = "app/planning/mod.rs"]
mod planning;
#[path = "app/planning_draft_editor_ui.rs"]
mod planning_draft_editor_ui;
#[path = "app/planning_init_overlay_ui.rs"]
mod planning_init_overlay_ui;
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
#[path = "app/turn_submission_runtime.rs"]
mod turn_submission_runtime;

use app_runtime::BackgroundMessage;
use conversation_input::{ConversationInputEvent, reduce_conversation_input};
use conversation_intents::{
    ConversationIntentEffect, ConversationIntentEvent, ConversationIntentMode,
    ConversationIntentState, reduce_conversation_intents,
};
use conversation_lifecycle::{
    ConversationLifecycleEffect, ConversationLifecycleEvent, ConversationLifecycleState,
    reduce_conversation_lifecycle,
};
#[cfg(test)]
pub(super) use conversation_model::TurnActivityState;
#[allow(unused_imports)]
pub(super) use conversation_model::{
    AutoFollowRuntimePhase, AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason,
    ConversationInputState, ConversationState, ConversationViewModel, StopKeywordRule,
};
use conversation_runtime::{
    ConversationRuntimeEffect, ConversationRuntimeEvent, reduce_conversation_runtime,
};
use directions_maintenance_ui::{
    DetailDocConfirmChoice, DirectionsMaintenanceOverlayStep, DirectionsMaintenanceOverlayUiState,
};
use followup_controls::{FollowupControlEffect, FollowupControlEvent, reduce_followup_controls};
use followup_overlay_ui::{
    FollowupOverlayUiEvent, FollowupOverlayUiState, reduce_followup_overlay_ui,
};
use github_polling::GithubReviewPollingState;
use inline_shell_commands::{InlineShellCommand, InlineShellCommandInput};
use planning::{PlannerVisibility, PlannerWorkerPanelState, PlannerWorkerStatus};
use planning_draft_editor_ui::PlanningDraftEditorUiState;
use planning_init_overlay_ui::{
    PlanningInitDetailSelection, PlanningInitModeSelection, PlanningInitOverlayStep,
    PlanningInitOverlayUiState,
};
use session_overlay_ui::SessionOverlayUiState;
pub(super) use shell_controller::ShellActionAvailability;
pub use shell_entrypoint::run;
use shell_frontend::ShellFrontendMode;
use shell_layout::build_conversation_scroll_offset;
#[cfg(test)]
use shell_layout::{block_height_for_lines, build_input_block_height, build_shell_footer_height};
#[cfg(test)]
use shell_presentation::format_conversation_lines;
#[cfg(test)]
use shell_presentation::{
    build_automation_overlay_view, build_automation_preview_lines, build_automation_status_lines,
    build_inline_tail_lines, build_planning_init_overlay_view, build_ready_input_lines,
};
use task_intake_overlay_ui::{TaskIntakeOverlayStep, TaskIntakeOverlayUiState};

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoFollowupSubmitContext {
    queued_from_turn_id: String,
    mode_label: String,
    transcript_text: String,
    debug_detail: Option<String>,
    handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptOrigin {
    Manual,
    AutoFollow(Box<AutoFollowupSubmitContext>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTurnPlanningCapture {
    workspace_directory: String,
    snapshot: ActiveTurnPlanningSnapshot,
}

impl ActiveTurnPlanningCapture {
    fn ready(workspace_directory: impl Into<String>, snapshot: PlanningExecutionSnapshot) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            snapshot: ActiveTurnPlanningSnapshot::Ready(snapshot),
        }
    }

    fn capture_failed(workspace_directory: impl Into<String>, message: String) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            snapshot: ActiveTurnPlanningSnapshot::CaptureFailed(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveTurnPlanningSnapshot {
    Ready(PlanningExecutionSnapshot),
    CaptureFailed(String),
}

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
            std::env::var("WT_SESSION").ok().as_deref(),
            cfg!(windows),
        )
    }

    fn from_env_values(
        mode_value: Option<&str>,
        _windows_terminal_session: Option<&str>,
        _running_on_windows: bool,
    ) -> Self {
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

struct NativeTuiApp {
    shell_overlay: ShellOverlay,
    exit_confirmation_state: ExitConfirmationState,
    startup_state: StartupState,
    session_state: SessionState,
    parallel_mode_enabled: bool,
    parallel_mode_readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
    parallel_mode_supervisor_snapshot: Option<ParallelModeSupervisorSnapshot>,
    conversation_state: ConversationState,
    selected_session_index: usize,
    session_overlay_ui_state: SessionOverlayUiState,
    followup_overlay_ui_state: FollowupOverlayUiState,
    directions_maintenance_overlay_ui_state: DirectionsMaintenanceOverlayUiState,
    planning_init_overlay_ui_state: PlanningInitOverlayUiState,
    planning_draft_editor_ui_state: PlanningDraftEditorUiState,
    task_intake_overlay_ui_state: TaskIntakeOverlayUiState,
    active_session: Option<SessionSummary>,
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    turn_control_truth: ConversationRuntimeControlTruth,
    parallel_mode_service: ParallelModeService,
    planning: PlanningServices,
    active_turn_planning_capture: Option<ActiveTurnPlanningCapture>,
    planner_worker_panel_state: PlannerWorkerPanelState,
    planner_visibility: PlannerVisibility,
    github_review_poller_service: Option<GithubReviewPollerService>,
    github_review_polling_state: GithubReviewPollingState,
    inline_history_render_mode: InlineHistoryRenderMode,
    show_startup_ascii_art: bool,
    tx: Sender<BackgroundMessage>,
    rx: Receiver<BackgroundMessage>,
}

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
#[path = "app/app_tests.rs"]
mod tests;

#[cfg(test)]
mod startup_ascii_art_env_tests {
    use super::{InlineHistoryRenderMode, PlannerVisibility, startup_ascii_art_enabled_from_value};

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
    fn planner_visibility_defaults_to_normal() {
        assert_eq!(
            PlannerVisibility::from_env_value(None),
            PlannerVisibility::Normal
        );
        assert_eq!(
            PlannerVisibility::from_env_value(Some("")),
            PlannerVisibility::Normal
        );
        assert_eq!(
            PlannerVisibility::from_env_value(Some("normal")),
            PlannerVisibility::Normal
        );
    }

    #[test]
    fn planner_visibility_supports_debug_values() {
        assert_eq!(
            PlannerVisibility::from_env_value(Some("debug")),
            PlannerVisibility::Debug
        );
        assert_eq!(
            PlannerVisibility::from_env_value(Some("TRUE")),
            PlannerVisibility::Debug
        );
        assert_eq!(
            PlannerVisibility::from_env_value(Some("verbose")),
            PlannerVisibility::Debug
        );
    }

    #[test]
    fn inline_history_render_mode_defaults_to_host_scrollback() {
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(None, None, false),
            InlineHistoryRenderMode::HostScrollback
        );
    }

    #[test]
    fn inline_history_render_mode_keeps_host_scrollback_for_windows_by_default() {
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(None, Some("wt-session"), false),
            InlineHistoryRenderMode::HostScrollback
        );
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(None, None, true),
            InlineHistoryRenderMode::HostScrollback
        );
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(None, None, false),
            InlineHistoryRenderMode::HostScrollback
        );
    }

    #[test]
    fn inline_history_render_mode_supports_explicit_override() {
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(Some("scrollback"), Some("wt-session"), true),
            InlineHistoryRenderMode::HostScrollback
        );
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(Some("viewport-replay"), None, false),
            InlineHistoryRenderMode::ViewportReplay
        );
        assert_eq!(
            InlineHistoryRenderMode::from_env_values(Some("mirror"), None, false),
            InlineHistoryRenderMode::ViewportReplay
        );
    }

    #[test]
    fn viewport_replay_does_not_write_host_scrollback() {
        assert!(InlineHistoryRenderMode::HostScrollback.writes_host_scrollback());
        assert!(!InlineHistoryRenderMode::ViewportReplay.writes_host_scrollback());
    }
}
