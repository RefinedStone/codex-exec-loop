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
use crate::application::service::followup_template_service::{
    FollowupTemplateReloadResult, FollowupTemplateService,
};
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::planning_reconciliation_service::PlanningExecutionSnapshot;
use crate::application::service::planning_services::PlanningServices;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot, ConversationStreamEvent,
};
use crate::domain::followup_template::FollowupTemplateCatalogLoadResult;
use crate::domain::session_summary::SessionSummary;
use crate::application::service::planning_runtime_facade_service::PlanningTaskHandoff;

const SESSION_PAGE_SIZE: usize = 10;
const MAX_CONVERSATION_HISTORY_LINES: usize = 160;
const DEFAULT_AUTO_FOLLOW_MAX_TURNS: usize = 3;
const MAX_AUTO_FOLLOW_MAX_TURNS: usize = 50;
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
const MAX_INLINE_TAIL_HEIGHT: u16 = 10;
const INLINE_VIEWPORT_HEIGHT: u16 = 16;
const DEFAULT_TRANSCRIPT_PAGE_STEP: u16 = 6;
const ALT_SCREEN_ENV_VAR: &str = "CODEX_EXEC_LOOP_ALT_SCREEN";
const STARTUP_ASCII_ART_ENV_VAR: &str = "CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART";
const PLANNER_VISIBILITY_ENV_VAR: &str = "CODEX_EXEC_LOOP_PLANNER_VISIBILITY";

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
#[path = "app/github_polling.rs"]
mod github_polling;
#[path = "app/inline_shell_commands.rs"]
mod inline_shell_commands;
#[path = "app/planning_draft_editor_ui.rs"]
mod planning_draft_editor_ui;
#[path = "app/planning_init_overlay_ui.rs"]
mod planning_init_overlay_ui;
#[path = "app/planning_presentation.rs"]
mod planning_presentation;
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
#[cfg(test)]
#[path = "app/test_helpers.rs"]
pub(crate) mod test_helpers;
#[path = "app/transcript_viewport.rs"]
mod transcript_viewport;
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
#[allow(unused_imports)]
pub(super) use conversation_model::{
    AutoFollowState, AutoFollowupDecision, AutoFollowupSkipReason, ConversationInputState,
    ConversationState, ConversationViewModel, StopKeywordRule,
};
#[cfg(test)]
pub(super) use conversation_model::{RecordedAutoFollowupActivity, TurnActivityState};
use conversation_runtime::{
    ConversationRuntimeEffect, ConversationRuntimeEvent, reduce_conversation_runtime,
};
use followup_controls::{FollowupControlEffect, FollowupControlEvent, reduce_followup_controls};
use followup_overlay_ui::{
    FollowupOverlayUiEvent, FollowupOverlayUiState, reduce_followup_overlay_ui,
};
use github_polling::GithubReviewPollingState;
use inline_shell_commands::{InlineShellCommand, InlineShellCommandInput};
use planning_draft_editor_ui::PlanningDraftEditorUiState;
use planning_init_overlay_ui::{
    PlanningInitDetailSelection, PlanningInitModeSelection, PlanningInitOverlayStep,
    PlanningInitOverlayUiState,
};
use session_overlay_ui::SessionOverlayUiState;
pub(super) use shell_controller::ShellActionAvailability;
pub use shell_entrypoint::run;
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
    build_followup_template_status_lines, build_inline_tail_lines, build_input_title,
    build_planning_init_overlay_view, build_ready_input_lines, build_session_overlay_view,
    build_shell_footer_lines, build_startup_overlay_view, build_status_title,
    build_transcript_panel_view, build_transcript_title,
};
use transcript_viewport::TranscriptViewportState;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoFollowupSubmitContext {
    queued_from_turn_id: String,
    template_label: String,
    transcript_text: String,
    handoff_task: Option<PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanningRepairSubmitContext {
    queued_from_turn_id: String,
    attempt_number: usize,
    max_attempts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptOrigin {
    Manual,
    AutoFollow(AutoFollowupSubmitContext),
    PlanningRepair(PlanningRepairSubmitContext),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PlannerWorkerStatus {
    #[default]
    Idle,
    RefreshRunning,
    RefreshSucceeded,
    RefreshFailed,
    RepairRunning,
    RepairSucceeded,
    RepairFailed,
}

impl PlannerWorkerStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::RefreshRunning => "refresh running",
            Self::RefreshSucceeded => "refresh ok",
            Self::RefreshFailed => "refresh failed",
            Self::RepairRunning => "repair running",
            Self::RepairSucceeded => "repair ok",
            Self::RepairFailed => "repair failed",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PlannerVisibility {
    #[default]
    Normal,
    Debug,
}

impl PlannerVisibility {
    fn from_environment() -> Self {
        Self::from_env_value(std::env::var(PLANNER_VISIBILITY_ENV_VAR).ok().as_deref())
    }

    fn from_env_value(value: Option<&str>) -> Self {
        match value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            Some("debug") | Some("verbose") | Some("detailed") | Some("1") | Some("true") => {
                Self::Debug
            }
            _ => Self::Normal,
        }
    }

    fn toggle(self) -> Self {
        match self {
            Self::Normal => Self::Debug,
            Self::Debug => Self::Normal,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Debug => "debug",
        }
    }

    fn shows_debug_details(self) -> bool {
        matches!(self, Self::Debug)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PlannerWorkerPanelState {
    status: PlannerWorkerStatus,
    last_summary: Option<String>,
    last_rejected_summary: Option<String>,
    last_queue_summary: Option<String>,
    last_host_detail: Option<String>,
}

impl PlannerWorkerPanelState {
    fn has_content(&self) -> bool {
        !matches!(self.status, PlannerWorkerStatus::Idle)
            || self.last_summary.is_some()
            || self.last_rejected_summary.is_some()
            || self.last_queue_summary.is_some()
            || self.last_host_detail.is_some()
    }
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
    planning_init_overlay_ui_state: PlanningInitOverlayUiState,
    planning_draft_editor_ui_state: PlanningDraftEditorUiState,
    transcript_viewport_state: TranscriptViewportState,
    active_session: Option<SessionSummary>,
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    followup_template_service: FollowupTemplateService,
    planning_services: PlanningServices,
    active_turn_planning_capture: Option<ActiveTurnPlanningCapture>,
    planner_worker_panel_state: PlannerWorkerPanelState,
    planner_visibility: PlannerVisibility,
    github_review_poller_service: Option<GithubReviewPollerService>,
    github_review_polling_state: GithubReviewPollingState,
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
    use super::{PlannerVisibility, startup_ascii_art_enabled_from_value};

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
        assert_eq!(PlannerVisibility::from_env_value(None), PlannerVisibility::Normal);
        assert_eq!(PlannerVisibility::from_env_value(Some("")), PlannerVisibility::Normal);
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
}
