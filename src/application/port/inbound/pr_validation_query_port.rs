use anyhow::Result;
use serde::Serialize;

use crate::domain::parallel_mode::{PrValidationOperatorSummary, PrValidationSchedulerMode};

pub const PR_VALIDATION_BOARD_DEFAULT_LIMIT: usize = 20;
pub const PR_VALIDATION_BOARD_MAX_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrValidationBoardCursorError;

impl std::fmt::Display for PrValidationBoardCursorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid PR validation board cursor")
    }
}

impl std::error::Error for PrValidationBoardCursorError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrValidationStatusRequest {
    pub pull_request_number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationBoardRequest {
    pub limit: usize,
    pub cursor: Option<String>,
}

impl PrValidationBoardRequest {
    pub fn new(limit: usize, cursor: Option<String>) -> Result<Self, String> {
        if !(1..=PR_VALIDATION_BOARD_MAX_LIMIT).contains(&limit) {
            return Err(format!(
                "PR validation board limit must be between 1 and {PR_VALIDATION_BOARD_MAX_LIMIT}"
            ));
        }
        let cursor = cursor
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if cursor.as_ref().is_some_and(|value| value.len() > 4_096) {
            return Err("PR validation board cursor is too large".to_string());
        }
        Ok(Self { limit, cursor })
    }
}

impl Default for PrValidationBoardRequest {
    fn default() -> Self {
        Self {
            limit: PR_VALIDATION_BOARD_DEFAULT_LIMIT,
            cursor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationDetailRequest {
    pub record_key: String,
}

impl PrValidationDetailRequest {
    pub fn new(record_key: impl Into<String>) -> Result<Self, String> {
        let record_key = record_key.into().trim().to_string();
        if record_key.is_empty() || record_key.len() > 200 {
            return Err("PR validation record key must contain 1 to 200 bytes".to_string());
        }
        Ok(Self { record_key })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAdminPhase {
    Registered,
    PreMerge,
    Integrated,
    Verifying,
    RemediationQueued,
    RemediationRunning,
    Verified,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAdminSeverity {
    Muted,
    Info,
    Success,
    Warning,
    Danger,
}

impl std::fmt::Display for PrValidationAdminSeverity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Muted => "muted",
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Danger => "danger",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAdminCheckStatus {
    Unobserved,
    Skipped,
    Missing,
    Pending,
    Succeeded,
    ActionableFailure,
    PolicyBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminCheck {
    pub context: String,
    pub app_slug: Option<String>,
    pub required: bool,
    pub status: PrValidationAdminCheckStatus,
    pub latest_attempt: Option<u64>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminWorkflow {
    pub name: String,
    pub status: String,
    pub run_attempt: u64,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminProvider {
    pub key: String,
    pub lifecycle: String,
    pub status: String,
    pub page_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminCorrelation {
    pub finding: String,
    pub remediation_akra_id: String,
    pub task_state: Option<String>,
    pub slot_id: Option<String>,
    pub session_key: Option<String>,
    pub worker_state: Option<String>,
    pub lease_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminSchedule {
    pub updated_at: String,
    pub last_polled_at: Option<String>,
    pub next_poll_at: Option<String>,
    pub poll_attempt: u64,
    pub consecutive_error_count: u32,
    pub error_class: Option<String>,
    pub rate_limit_remaining: Option<u64>,
    pub rate_limit_reset_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminCommandAvailability {
    pub action: String,
    pub label: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminTimelineEntry {
    pub kind: String,
    pub label: String,
    pub state: String,
    pub occurred_at: Option<String>,
    pub attempt: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminRecord {
    pub record_key: String,
    pub akra_id: String,
    pub repository: String,
    pub canonical_pr_url: String,
    pub pull_request_number: u64,
    pub phase: PrValidationAdminPhase,
    pub phase_label: String,
    pub severity: PrValidationAdminSeverity,
    pub integrated: bool,
    pub verified: bool,
    pub provider_blocked: bool,
    pub stale: bool,
    pub stale_seconds: u64,
    pub target_sha: String,
    pub base_before_sha: Option<String>,
    pub evidence_sha: Option<String>,
    pub merge_sha: Option<String>,
    pub target_short_sha: String,
    pub evidence_short_sha: Option<String>,
    pub merge_short_sha: Option<String>,
    pub integration_method: Option<String>,
    pub integration_pull_request_number: Option<u64>,
    pub integrated_at: Option<String>,
    pub remote_verified_at: Option<String>,
    pub reason: String,
    pub blocker: Option<String>,
    pub recovery_action: String,
    pub checks: Vec<PrValidationAdminCheck>,
    pub required_checks_succeeded: usize,
    pub required_checks_total: usize,
    pub workflows: Vec<PrValidationAdminWorkflow>,
    pub providers: Vec<PrValidationAdminProvider>,
    pub finding_count: usize,
    pub remediation_count: usize,
    pub correlations: Vec<PrValidationAdminCorrelation>,
    pub schedule: PrValidationAdminSchedule,
    pub paused: bool,
    pub acknowledged_at: Option<String>,
    pub last_command_id: Option<String>,
    pub commands: Vec<PrValidationAdminCommandAvailability>,
    pub timeline: Vec<PrValidationAdminTimelineEntry>,
    pub observation_revision: u64,
    pub post_merge_checkpoint_observed: bool,
}

impl PrValidationAdminRecord {
    pub fn latest_required_attempt(&self) -> u64 {
        self.checks
            .iter()
            .filter(|check| check.required)
            .filter_map(|check| check.latest_attempt)
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationRolloutStage {
    Off,
    Shadow,
    Remediation,
}

impl std::fmt::Display for PrValidationRolloutStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Off => "off",
            Self::Shadow => "shadow",
            Self::Remediation => "remediation",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationRolloutSnapshot {
    pub stage: PrValidationRolloutStage,
    pub observation_enabled: bool,
    pub queue_admission_enabled: bool,
    pub ruleset_change_requires_approval: bool,
    pub status_label: String,
    pub detail: String,
}

impl PrValidationRolloutSnapshot {
    pub fn from_scheduler_mode(mode: PrValidationSchedulerMode) -> Self {
        let (stage, observation_enabled, queue_admission_enabled, status_label, detail) = match mode
        {
            PrValidationSchedulerMode::Off => (
                PrValidationRolloutStage::Off,
                false,
                false,
                "OFF · 기록 보존",
                "provider polling은 중지되고 기존 validation history는 유지됩니다.",
            ),
            PrValidationSchedulerMode::Observe => (
                PrValidationRolloutStage::Shadow,
                true,
                false,
                "SHADOW · Queue 차단",
                "GitHub evidence만 관찰하며 remediation task는 자동 admission하지 않습니다.",
            ),
            PrValidationSchedulerMode::Remediate => (
                PrValidationRolloutStage::Remediation,
                true,
                true,
                "REMEDIATE · Queue 활성",
                "actionable finding만 일반 Planning Queue로 admission하며 Ruleset 변경은 별도 승인입니다.",
            ),
        };
        Self {
            stage,
            observation_enabled,
            queue_admission_enabled,
            ruleset_change_requires_approval: true,
            status_label: status_label.to_string(),
            detail: detail.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationBoardSummary {
    pub active: usize,
    pub integrated: usize,
    pub verifying: usize,
    pub remediation: usize,
    pub remediation_queued: usize,
    pub verified: usize,
    pub blocked: usize,
    pub failed: usize,
    pub stale: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationBoardSnapshot {
    pub revision: i64,
    pub scheduler_mode: String,
    pub rollout: PrValidationRolloutSnapshot,
    pub summary: PrValidationBoardSummary,
    pub records: Vec<PrValidationAdminRecord>,
    pub next_cursor: Option<String>,
    pub cursor_reset_required: bool,
    pub generated_at: String,
}

pub trait PrValidationQueryPort: Send + Sync {
    fn status_for_pr(
        &self,
        request: PrValidationStatusRequest,
    ) -> Result<Option<PrValidationOperatorSummary>>;

    fn load_board(&self, request: PrValidationBoardRequest) -> Result<PrValidationBoardSnapshot>;

    /// Monotonic revision scoped to validation records. Runtime events from unrelated workers
    /// must not invalidate a validation-board pagination cursor.
    fn load_board_revision(&self) -> Result<i64> {
        Ok(0)
    }

    fn load_detail(
        &self,
        request: PrValidationDetailRequest,
    ) -> Result<Option<PrValidationAdminRecord>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollout_projection_keeps_shadow_safe_and_ruleset_approval_explicit() {
        let shadow =
            PrValidationRolloutSnapshot::from_scheduler_mode(PrValidationSchedulerMode::Observe);
        assert_eq!(shadow.stage, PrValidationRolloutStage::Shadow);
        assert!(shadow.observation_enabled);
        assert!(!shadow.queue_admission_enabled);
        assert!(shadow.ruleset_change_requires_approval);

        let remediation =
            PrValidationRolloutSnapshot::from_scheduler_mode(PrValidationSchedulerMode::Remediate);
        assert_eq!(remediation.stage, PrValidationRolloutStage::Remediation);
        assert!(remediation.queue_admission_enabled);
        assert!(remediation.ruleset_change_requires_approval);

        let off = PrValidationRolloutSnapshot::from_scheduler_mode(PrValidationSchedulerMode::Off);
        assert_eq!(off.stage, PrValidationRolloutStage::Off);
        assert!(!off.observation_enabled);
        assert!(!off.queue_admission_enabled);
    }
}
