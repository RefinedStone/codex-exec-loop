// Startup-time switch for exposing raw planning worker diagnostics in TUI surfaces.
// Runtime code reads it once so a session has stable visibility semantics.
const PLANNING_WORKER_VISIBILITY_ENV_VAR: &str = "CODEX_EXEC_LOOP_PLANNING_WORKER_VISIBILITY";
const LEGACY_PLANNING_WORKER_VISIBILITY_ENV_VAR: &str = "CODEX_EXEC_LOOP_PLANNER_VISIBILITY";

// Visibility policy for planning worker internals.
// Normal keeps repeated TUI usage compact; Debug exposes raw prompt/response and host-side details for diagnosis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum PlanningWorkerVisibility {
    #[default]
    Normal,
    Debug,
}

impl PlanningWorkerVisibility {
    // NativeTuiApp calls this during construction; later rendering only consults the stored enum.
    pub(in crate::adapter::inbound::tui::app) fn from_environment() -> Self {
        let value = std::env::var(PLANNING_WORKER_VISIBILITY_ENV_VAR)
            .or_else(|_| std::env::var(LEGACY_PLANNING_WORKER_VISIBILITY_ENV_VAR))
            .ok();
        Self::from_env_value(value.as_deref())
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
