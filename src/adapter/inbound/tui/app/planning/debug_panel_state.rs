const PLANNER_VISIBILITY_ENV_VAR: &str = "CODEX_EXEC_LOOP_PLANNER_VISIBILITY";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum PlannerWorkerStatus {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum PlannerVisibility {
    #[default]
    Normal,
    Debug,
}

impl PlannerVisibility {
    pub(in crate::adapter::inbound::tui::app) fn from_environment() -> Self {
        Self::from_env_value(std::env::var(PLANNER_VISIBILITY_ENV_VAR).ok().as_deref())
    }

    pub(in crate::adapter::inbound::tui::app) fn from_env_value(value: Option<&str>) -> Self {
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

    pub(in crate::adapter::inbound::tui::app) fn toggle(self) -> Self {
        match self {
            Self::Normal => Self::Debug,
            Self::Debug => Self::Normal,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Debug => "debug",
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn shows_debug_details(self) -> bool {
        matches!(self, Self::Debug)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct PlannerWorkerPanelState {
    pub(in crate::adapter::inbound::tui::app) status: PlannerWorkerStatus,
    pub(in crate::adapter::inbound::tui::app) last_operation_label: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_rejected_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_queue_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_notice_detail: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_prompt: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_response: Option<String>,
    pub(in crate::adapter::inbound::tui::app) last_host_detail: Option<String>,
}

impl PlannerWorkerPanelState {
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
