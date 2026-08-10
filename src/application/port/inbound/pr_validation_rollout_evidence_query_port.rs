use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const PR_VALIDATION_EVIDENCE_DEFAULT_HISTORY_LIMIT: usize = 10;
pub const PR_VALIDATION_EVIDENCE_MAX_HISTORY_LIMIT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationRolloutEvidenceStatus {
    Ready,
    Hold,
    Stale,
    Unavailable,
    Invalid,
}

impl PrValidationRolloutEvidenceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Hold => "hold",
            Self::Stale => "stale",
            Self::Unavailable => "unavailable",
            Self::Invalid => "invalid",
        }
    }

    pub fn is_structurally_valid(self) -> bool {
        matches!(self, Self::Ready | Self::Hold | Self::Stale)
    }
}

impl std::fmt::Display for PrValidationRolloutEvidenceStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationMetricLabel {
    Projected,
    MixedActualAndProjected,
    Actual,
    Unavailable,
}

impl PrValidationMetricLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Projected => "projected",
            Self::MixedActualAndProjected => "mixed_actual_and_projected",
            Self::Actual => "actual",
            Self::Unavailable => "unavailable",
        }
    }
}

impl std::fmt::Display for PrValidationMetricLabel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationGateMetricSnapshot {
    pub label: PrValidationMetricLabel,
    pub sample_count: Option<u64>,
    pub p50_seconds: Option<f64>,
    pub p95_seconds: Option<f64>,
    pub source: String,
}

impl PrValidationGateMetricSnapshot {
    pub fn unavailable(source: &str) -> Self {
        Self {
            label: PrValidationMetricLabel::Unavailable,
            sample_count: None,
            p50_seconds: None,
            p95_seconds: None,
            source: source.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationSampleWindowSnapshot {
    pub pull_request_count: Option<u64>,
    pub earliest_merged_at: Option<String>,
    pub latest_merged_at: Option<String>,
    pub window_hours: Option<f64>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationQuotaSnapshot {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub used: Option<u64>,
    pub used_percent: Option<f64>,
    pub reset_at: Option<String>,
    pub collector_requests: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationCriterionSnapshot {
    pub key: String,
    pub status: String,
    pub source: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationProductionCanarySnapshot {
    pub pull_request_number: u64,
    pub evidence_short_sha: String,
    pub run_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationRolloutEvidenceSummary {
    pub status: PrValidationRolloutEvidenceStatus,
    pub generated_at: Option<String>,
    pub repository: Option<String>,
    pub base_branch: Option<String>,
    pub evidence_short_sha: Option<String>,
    pub recommended_scheduler_mode: Option<String>,
    pub queue_admission_supported: bool,
    pub sample_window: PrValidationSampleWindowSnapshot,
    pub historical_fast_gate: PrValidationGateMetricSnapshot,
    pub actual_fast_gate: PrValidationGateMetricSnapshot,
    pub ci_gate: PrValidationGateMetricSnapshot,
    pub post_merge_gate: PrValidationGateMetricSnapshot,
    pub post_merge_failure_rate_percent: Option<f64>,
    pub quota_used_percent: Option<f64>,
    pub blockers: Vec<String>,
}

impl PrValidationRolloutEvidenceSummary {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: PrValidationRolloutEvidenceStatus::Unavailable,
            generated_at: None,
            repository: None,
            base_branch: None,
            evidence_short_sha: None,
            recommended_scheduler_mode: None,
            queue_admission_supported: false,
            sample_window: PrValidationSampleWindowSnapshot {
                pull_request_count: None,
                earliest_merged_at: None,
                latest_merged_at: None,
                window_hours: None,
                source: "unavailable".to_string(),
            },
            historical_fast_gate: PrValidationGateMetricSnapshot::unavailable("unavailable"),
            actual_fast_gate: PrValidationGateMetricSnapshot::unavailable("unavailable"),
            ci_gate: PrValidationGateMetricSnapshot::unavailable("unavailable"),
            post_merge_gate: PrValidationGateMetricSnapshot::unavailable("unavailable"),
            post_merge_failure_rate_percent: None,
            quota_used_percent: None,
            blockers: vec![reason.into()],
        }
    }
}

impl Default for PrValidationRolloutEvidenceSummary {
    fn default() -> Self {
        Self::unavailable("rollout evidence has not been collected")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationRolloutEvidenceSnapshot {
    pub artifact_short_sha: Option<String>,
    pub observed_at: String,
    pub summary: PrValidationRolloutEvidenceSummary,
    pub quota: PrValidationQuotaSnapshot,
    pub criteria: Vec<PrValidationCriterionSnapshot>,
    pub production_success_canary: Option<PrValidationProductionCanarySnapshot>,
    pub failure_canary_status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationEvidenceWarningSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationEvidenceWarningKind {
    InsufficientActualSamples,
    LatencySpike,
    StaleEvidence,
    EvidenceShaMismatch,
    PostMergeFailure,
    DuplicateRemediation,
    PhaseMismatch,
    CollectionUnavailable,
    StaleLeaseRecovered,
    IdentityConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationEvidenceWarningSnapshot {
    pub kind: PrValidationEvidenceWarningKind,
    pub severity: PrValidationEvidenceWarningSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationEvidenceMetricTrendSnapshot {
    pub metric: String,
    pub current_sample_count: Option<u64>,
    pub previous_sample_count: Option<u64>,
    pub sample_count_delta: Option<i64>,
    pub current_p95_seconds: Option<f64>,
    pub previous_p95_seconds: Option<f64>,
    pub p95_delta_seconds: Option<f64>,
    pub p95_delta_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationEvidenceTrendSnapshot {
    pub comparisons: Vec<PrValidationEvidenceMetricTrendSnapshot>,
    pub actual_minus_projected_p95_seconds: Option<f64>,
    pub actual_to_projected_p95_ratio: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationEvidenceCollectionSnapshot {
    pub state: String,
    pub revision: Option<i64>,
    pub last_collected_at: Option<String>,
    pub last_outcome: Option<String>,
    pub last_error_class: Option<String>,
    pub next_collect_at_epoch_millis: Option<i64>,
    pub stale_lease_recovery_count: u64,
    pub identity_conflict_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationRolloutEvidenceRequest {
    pub limit: usize,
    pub cursor: Option<String>,
}

impl Default for PrValidationRolloutEvidenceRequest {
    fn default() -> Self {
        Self {
            limit: PR_VALIDATION_EVIDENCE_DEFAULT_HISTORY_LIMIT,
            cursor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationRolloutEvidencePage {
    pub latest: PrValidationRolloutEvidenceSnapshot,
    pub last_valid: Option<PrValidationRolloutEvidenceSnapshot>,
    pub history: Vec<PrValidationRolloutEvidenceSnapshot>,
    pub next_cursor: Option<String>,
    pub revision: Option<i64>,
    pub trends: PrValidationEvidenceTrendSnapshot,
    pub warnings: Vec<PrValidationEvidenceWarningSnapshot>,
    pub collection: PrValidationEvidenceCollectionSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrValidationRolloutEvidenceCursorError;

impl std::fmt::Display for PrValidationRolloutEvidenceCursorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid PR validation rollout evidence cursor")
    }
}

impl std::error::Error for PrValidationRolloutEvidenceCursorError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrValidationRolloutEvidenceLimitError;

impl std::fmt::Display for PrValidationRolloutEvidenceLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid PR validation rollout evidence history limit")
    }
}

impl std::error::Error for PrValidationRolloutEvidenceLimitError {}

pub trait PrValidationRolloutEvidenceQueryPort: Send + Sync {
    fn load_latest_summary(&self) -> PrValidationRolloutEvidenceSummary;

    fn load_page(
        &self,
        request: PrValidationRolloutEvidenceRequest,
    ) -> Result<PrValidationRolloutEvidencePage>;

    fn load_revision(&self) -> Result<Option<i64>> {
        Ok(None)
    }
}
