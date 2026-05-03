// Startup-time switch for exposing raw planner worker diagnostics in TUI surfaces.
// Runtime code reads it once so a session has stable visibility semantics.
const PLANNER_VISIBILITY_ENV_VAR: &str = "CODEX_EXEC_LOOP_PLANNER_VISIBILITY";

// UI-facing lifecycle for the post-turn planning worker.
// It intentionally combines refresh and repair outcomes because footer/debug panels need one compact status lane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum PlannerWorkerStatus {
    // No observable worker interaction for the current draft/session.
    #[default]
    Idle,
    // Refresh recomputes queue head/proposal state after a turn.
    RefreshRunning,
    RefreshSucceeded,
    // Covers worker failure, repair request, repeated queue head, or invalid snapshot after refresh.
    RefreshFailed,
    // Repair attempts to bring invalid or changed planning files back to a usable runtime snapshot.
    RepairRunning,
    // Repair restored the runtime snapshot enough for follow-up decisions to resume.
    RepairSucceeded,
    // Repair failed or left a blocking reason that still requires operator attention.
    RepairFailed,
}

impl PlannerWorkerStatus {
    // Short operator copy shared by footer, debug preview, and planner panel rendering.
    pub(in crate::adapter::inbound::tui::app) fn label(self) -> &'static str {
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

// Visibility policy for planner worker internals.
// Normal keeps repeated TUI usage compact; Debug exposes raw prompt/response and host-side details for diagnosis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum PlannerVisibility {
    #[default]
    Normal,
    Debug,
}

impl PlannerVisibility {
    // NativeTuiApp calls this during construction; later rendering only consults the stored enum.
    pub(in crate::adapter::inbound::tui::app) fn from_environment() -> Self {
        Self::from_env_value(std::env::var(PLANNER_VISIBILITY_ENV_VAR).ok().as_deref())
    }

    // Testable parser for env syntax without mutating process environment in unit tests.
    pub(in crate::adapter::inbound::tui::app) fn from_env_value(value: Option<&str>) -> Self {
        match value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            // Accept both human-readable words and shell/CI-friendly booleans.
            Some("debug") | Some("verbose") | Some("detailed") | Some("1") | Some("true") => {
                Self::Debug
            }
            // Unknown values fail closed to avoid turning noisy debug surfaces on accidentally.
            _ => Self::Normal,
        }
    }

    // Presentation asks for the capability instead of matching variants, keeping future visibility tiers local.
    pub(in crate::adapter::inbound::tui::app) fn shows_debug_details(self) -> bool {
        matches!(self, Self::Debug)
    }
}

// Last observed planner worker interaction, cached for status panels after post-turn execution finishes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct PlannerWorkerPanelState {
    // Status drives high-level label and success/failure styling.
    pub(in crate::adapter::inbound::tui::app) status: PlannerWorkerStatus,
    // Operation label names the worker action, usually refresh or repair.
    pub(in crate::adapter::inbound::tui::app) last_operation_label: Option<String>,
    // Compact accepted result or failure summary shown in normal mode.
    pub(in crate::adapter::inbound::tui::app) last_summary: Option<String>,
    // Candidate or decision the host rejected, kept separate from the accepted summary.
    pub(in crate::adapter::inbound::tui::app) last_rejected_summary: Option<String>,
    // Queue state after worker application, not merely what the worker predicted.
    pub(in crate::adapter::inbound::tui::app) last_queue_summary: Option<String>,
    // Extra notices such as repair/block reasons after summary text has been trimmed.
    pub(in crate::adapter::inbound::tui::app) last_notice_detail: Option<String>,
    // Raw worker IO is stored for Debug visibility and omitted from normal compact surfaces.
    pub(in crate::adapter::inbound::tui::app) last_prompt: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_response: Option<String>,
    // Host-side postprocessing decisions are tracked apart from worker text for diagnosis of orchestration behavior.
    pub(in crate::adapter::inbound::tui::app) last_host_detail: Option<String>,
}

impl PlannerWorkerPanelState {
    // One predicate controls whether planner panels render at all, including debug-only fields.
    pub(in crate::adapter::inbound::tui::app) fn has_content(&self) -> bool {
        !matches!(self.status, PlannerWorkerStatus::Idle)
            || self.last_operation_label.is_some()
            || self.last_summary.is_some()
            || self.last_rejected_summary.is_some()
            || self.last_queue_summary.is_some()
            || self.last_notice_detail.is_some()
            || self.last_prompt.is_some()
            || self.last_response.is_some()
            || self.last_host_detail.is_some()
    }
}
