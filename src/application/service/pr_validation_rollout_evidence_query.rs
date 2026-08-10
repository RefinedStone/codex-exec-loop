use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::application::port::inbound::pr_validation_rollout_evidence_query_port::{
    PR_VALIDATION_EVIDENCE_MAX_HISTORY_LIMIT, PrValidationCriterionSnapshot,
    PrValidationGateMetricSnapshot, PrValidationMetricLabel, PrValidationProductionCanarySnapshot,
    PrValidationQuotaSnapshot, PrValidationRolloutEvidenceCursorError,
    PrValidationRolloutEvidenceLimitError, PrValidationRolloutEvidencePage,
    PrValidationRolloutEvidenceQueryPort, PrValidationRolloutEvidenceRequest,
    PrValidationRolloutEvidenceSnapshot, PrValidationRolloutEvidenceStatus,
    PrValidationRolloutEvidenceSummary, PrValidationSampleWindowSnapshot,
};
use crate::application::port::outbound::pr_validation_rollout_evidence_port::{
    PrValidationRolloutEvidenceDocument, PrValidationRolloutEvidencePort,
};

const SUPPORTED_SCHEMA_VERSION: u64 = 1;
const CURSOR_VERSION: u8 = 1;
const DEFAULT_FRESHNESS_HOURS: i64 = 48;
const MAX_FUTURE_SKEW_MINUTES: i64 = 5;
const MAX_SAMPLE_ROWS: usize = 100;
const MAX_CRITERIA: usize = 32;
const MAX_METRIC_SECONDS: f64 = 86_400.0;
const MAX_STRING_LENGTH: usize = 512;
const LATEST_SUMMARY_CACHE_TTL: StdDuration = StdDuration::from_secs(5);

pub struct PrValidationRolloutEvidenceQueryService {
    workspace_dir: String,
    expected_repository: Option<String>,
    expected_base_branch: String,
    source: Arc<dyn PrValidationRolloutEvidencePort>,
    freshness: Duration,
    latest_summary_cache: Mutex<Option<(Instant, PrValidationRolloutEvidenceSummary)>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EvidenceCursor {
    version: u8,
    sort_at: String,
    artifact_sha: String,
}

#[derive(Debug)]
struct ProjectedDocument {
    sort_at: DateTime<Utc>,
    artifact_sha: String,
    snapshot: PrValidationRolloutEvidenceSnapshot,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEvidence {
    schema_version: u64,
    generated_at: String,
    repository: String,
    base_branch: String,
    decision: RawDecision,
    sample: RawSample,
    timings: RawTimings,
    #[serde(default)]
    actual_fast_gate_runs: Vec<RawActualFastGateRun>,
    post_merge: RawPostMerge,
    quota: RawQuota,
    criteria: BTreeMap<String, RawCriterion>,
    canaries: RawCanaries,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDecision {
    status: String,
    recommended_scheduler_mode: String,
    queue_admission_enabled: bool,
    ruleset_change: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSample {
    pull_request_count: u64,
    earliest_merged_at: String,
    latest_merged_at: String,
    window_hours: f64,
    rows: Vec<RawSampleRow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSampleRow {
    merge_sha: String,
    evidence_sha_matches_actions_target: bool,
    pre_merge: Option<RawRunSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRunSummary {
    fast_gate: Option<RawFastGate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFastGate {
    source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTimings {
    fast_gate: RawMetric,
    actual_fast_gate: RawMetric,
    ci_gate: RawMetric,
    post_merge_gate: RawMetric,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMetric {
    sample_count: u64,
    p50_seconds: Option<f64>,
    p95_seconds: Option<f64>,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawActualFastGateRun {
    id: serde_json::Value,
    head_sha: String,
    conclusion: String,
    seconds: f64,
    run_url: String,
    job_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPostMerge {
    sample_count: u64,
    failure_count: u64,
    failure_rate_percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawQuota {
    limit: u64,
    remaining: u64,
    used: u64,
    reset: i64,
    collector_requests: u64,
    used_percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawCriterion {
    status: String,
    source: String,
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCanaries {
    production_success: Option<RawProductionCanary>,
    failure: Option<RawFailureCanary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProductionCanary {
    status: String,
    pull_request_number: u64,
    merge_sha: String,
    run_url: String,
}

#[derive(Debug, Deserialize)]
struct RawFailureCanary {
    status: String,
}

impl PrValidationRolloutEvidenceQueryService {
    pub fn new(
        workspace_dir: impl Into<String>,
        expected_repository: Option<String>,
        expected_base_branch: impl Into<String>,
        source: Arc<dyn PrValidationRolloutEvidencePort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            expected_repository,
            expected_base_branch: expected_base_branch.into(),
            source,
            freshness: Duration::hours(DEFAULT_FRESHNESS_HOURS),
            latest_summary_cache: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_freshness(mut self, freshness: Duration) -> Self {
        self.freshness = freshness;
        self
    }

    fn load_page_at(
        &self,
        request: PrValidationRolloutEvidenceRequest,
        now: DateTime<Utc>,
    ) -> Result<PrValidationRolloutEvidencePage> {
        if !(1..=PR_VALIDATION_EVIDENCE_MAX_HISTORY_LIMIT).contains(&request.limit) {
            return Err(anyhow::Error::new(PrValidationRolloutEvidenceLimitError));
        }
        if self.expected_repository.is_none() {
            return Ok(unavailable_page(
                now,
                "active repository identity is unavailable",
            ));
        }
        let documents = match self.source.load_documents(&self.workspace_dir) {
            Ok(documents) => documents,
            Err(error) => {
                tracing::warn!(error = %error, "PR validation rollout evidence source is unavailable");
                return Ok(unavailable_page(
                    now,
                    "rollout evidence source is unavailable",
                ));
            }
        };
        if documents.is_empty() {
            return Ok(unavailable_page(
                now,
                "rollout evidence has not been collected",
            ));
        }

        let mut projected = documents
            .into_iter()
            .map(|document| self.project_document(document, now))
            .collect::<Vec<_>>();
        projected.sort_by(|left, right| {
            right
                .sort_at
                .cmp(&left.sort_at)
                .then_with(|| right.artifact_sha.cmp(&left.artifact_sha))
        });
        let mut seen = HashSet::new();
        projected.retain(|document| seen.insert(document.artifact_sha.clone()));

        let latest = projected
            .first()
            .map(|document| document.snapshot.clone())
            .unwrap_or_else(|| {
                unavailable_snapshot(now, "rollout evidence has not been collected")
            });
        let last_valid = projected
            .iter()
            .find(|document| document.snapshot.summary.status.is_structurally_valid())
            .map(|document| document.snapshot.clone());
        let start = decode_cursor(request.cursor.as_deref(), &projected)?;
        let end = (start + request.limit).min(projected.len());
        let history = projected[start..end]
            .iter()
            .map(|document| document.snapshot.clone())
            .collect::<Vec<_>>();
        let next_cursor = (end < projected.len())
            .then(|| encode_cursor(&projected[end - 1]))
            .transpose()?;
        Ok(PrValidationRolloutEvidencePage {
            latest,
            last_valid,
            history,
            next_cursor,
        })
    }

    fn project_document(
        &self,
        document: PrValidationRolloutEvidenceDocument,
        now: DateTime<Utc>,
    ) -> ProjectedDocument {
        let artifact_sha = hex_sha256(&document.content);
        let observed_at = match DateTime::parse_from_rfc3339(&document.observed_at) {
            Ok(value) => value.with_timezone(&Utc),
            Err(_) => {
                return ProjectedDocument {
                    sort_at: now,
                    artifact_sha: artifact_sha.clone(),
                    snapshot: invalid_snapshot(
                        now,
                        &artifact_sha,
                        "rollout evidence observation timestamp is invalid".to_string(),
                    ),
                };
            }
        };
        let snapshot = match serde_json::from_slice::<RawEvidence>(&document.content) {
            Ok(raw) => self
                .validate_and_project(raw, observed_at, now, &artifact_sha)
                .unwrap_or_else(|error| {
                    invalid_snapshot(observed_at, &artifact_sha, error.to_string())
                }),
            Err(_) => invalid_snapshot(
                observed_at,
                &artifact_sha,
                "rollout evidence JSON is malformed".to_string(),
            ),
        };
        let sort_at = snapshot
            .summary
            .generated_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or(observed_at);
        ProjectedDocument {
            sort_at,
            artifact_sha,
            snapshot,
        }
    }

    fn validate_and_project(
        &self,
        raw: RawEvidence,
        observed_at: DateTime<Utc>,
        now: DateTime<Utc>,
        artifact_sha: &str,
    ) -> Result<PrValidationRolloutEvidenceSnapshot> {
        if raw.schema_version != SUPPORTED_SCHEMA_VERSION {
            bail!("rollout evidence schema version is unsupported");
        }
        validate_text(&raw.repository, "repository")?;
        validate_text(&raw.base_branch, "base branch")?;
        let Some(expected_repository) = self.expected_repository.as_deref() else {
            bail!("expected repository identity is unavailable");
        };
        if raw.repository != expected_repository {
            bail!("rollout evidence repository does not match the active repository");
        }
        if raw.base_branch != self.expected_base_branch {
            bail!("rollout evidence base branch does not match the configured base");
        }
        let generated_at = parse_time(&raw.generated_at, "generated-at")?;
        if generated_at > now + Duration::minutes(MAX_FUTURE_SKEW_MINUTES) {
            bail!("rollout evidence generated-at is in the future");
        }
        if raw.sample.rows.len() > MAX_SAMPLE_ROWS
            || raw.sample.pull_request_count as usize != raw.sample.rows.len()
        {
            bail!("rollout evidence sample rows violate the bounded count contract");
        }
        if raw.criteria.is_empty() || raw.criteria.len() > MAX_CRITERIA {
            bail!("rollout evidence criteria violate the bounded contract");
        }
        if raw.decision.ruleset_change != "approval_required" {
            bail!("rollout evidence must preserve the Ruleset approval boundary");
        }
        validate_text(&raw.decision.reason, "decision reason")?;
        let recommended_scheduler_mode = match raw.decision.recommended_scheduler_mode.as_str() {
            "observe" | "remediate" => raw.decision.recommended_scheduler_mode.clone(),
            _ => bail!("rollout evidence recommends an unsupported scheduler mode"),
        };
        if !matches!(
            raw.decision.status.as_str(),
            "ready_for_remediate" | "hold_observe"
        ) {
            bail!("rollout evidence decision status is unsupported");
        }

        let historical_actual_count = raw
            .sample
            .rows
            .iter()
            .filter(|row| {
                row.pre_merge
                    .as_ref()
                    .and_then(|run| run.fast_gate.as_ref())
                    .is_some_and(|gate| gate.source == "actual")
            })
            .count();
        if raw.sample.rows.iter().any(|row| {
            row.pre_merge
                .as_ref()
                .and_then(|run| run.fast_gate.as_ref())
                .is_some_and(|gate| {
                    !matches!(
                        gate.source.as_str(),
                        "actual" | "projected" | "projected_from_existing_jobs"
                    )
                })
        }) {
            bail!("historical Fast Gate row source is unsupported");
        }
        if raw.timings.fast_gate.sample_count != raw.sample.pull_request_count {
            bail!("historical Fast Gate sample count is inconsistent");
        }
        let historical_fast_gate = project_metric(
            &raw.timings.fast_gate,
            MetricContract::Historical {
                actual_count: historical_actual_count,
            },
        )?;
        let actual_fast_gate = project_metric(
            &raw.timings.actual_fast_gate,
            MetricContract::Actual("independent_actual_runs"),
        )?;
        let ci_gate = project_metric(
            &raw.timings.ci_gate,
            MetricContract::Actual("pre_merge_runs"),
        )?;
        if raw.timings.ci_gate.sample_count != raw.sample.pull_request_count {
            bail!("CI Gate sample count is inconsistent");
        }
        let post_merge_gate = project_metric(
            &raw.timings.post_merge_gate,
            MetricContract::Actual("post_merge_push_runs"),
        )?;
        validate_actual_runs(&raw.actual_fast_gate_runs, &raw.repository)?;
        if raw.timings.actual_fast_gate.sample_count as usize != raw.actual_fast_gate_runs.len() {
            bail!("independent actual Fast Gate sample count is inconsistent");
        }
        validate_percentage(
            raw.post_merge.failure_rate_percent,
            "post-merge failure rate",
        )?;
        if raw.post_merge.failure_count > raw.post_merge.sample_count
            || raw.post_merge.sample_count != raw.timings.post_merge_gate.sample_count
            || raw.post_merge.sample_count > raw.sample.pull_request_count
        {
            bail!("post-merge sample counts are inconsistent");
        }
        let expected_failure_rate = if raw.post_merge.sample_count == 0 {
            None
        } else {
            Some((raw.post_merge.failure_count as f64 / raw.post_merge.sample_count as f64) * 100.0)
        };
        if !approximately_equal(raw.post_merge.failure_rate_percent, expected_failure_rate) {
            bail!("post-merge failure rate is inconsistent with its sample counts");
        }
        validate_percentage(raw.quota.used_percent, "quota usage")?;
        if raw.quota.used > raw.quota.limit
            || raw.quota.remaining > raw.quota.limit
            || raw.quota.used.checked_add(raw.quota.remaining) != Some(raw.quota.limit)
        {
            bail!("quota counts are inconsistent");
        }
        let expected_quota_percent = if raw.quota.limit == 0 {
            None
        } else {
            Some((raw.quota.used as f64 / raw.quota.limit as f64) * 100.0)
        };
        if !approximately_equal(raw.quota.used_percent, expected_quota_percent) {
            bail!("quota usage is inconsistent with its counts");
        }
        let reset_at = Utc
            .timestamp_opt(raw.quota.reset, 0)
            .single()
            .ok_or_else(|| anyhow::anyhow!("quota reset timestamp is invalid"))?;
        let earliest_merged_at = parse_time(&raw.sample.earliest_merged_at, "sample start")?;
        let latest_merged_at = parse_time(&raw.sample.latest_merged_at, "sample end")?;
        if latest_merged_at < earliest_merged_at
            || generated_at < latest_merged_at
            || !raw.sample.window_hours.is_finite()
            || raw.sample.window_hours < 0.0
        {
            bail!("sample window is invalid");
        }
        let expected_window_hours =
            (latest_merged_at - earliest_merged_at).num_seconds() as f64 / 3_600.0;
        if (raw.sample.window_hours - expected_window_hours).abs() > 0.02 {
            bail!("sample window duration is inconsistent with its timestamps");
        }

        let production_canary = raw
            .canaries
            .production_success
            .as_ref()
            .filter(|canary| canary.status == "pass" && canary.pull_request_number > 0);
        let evidence_sha = production_canary
            .map(|canary| canary.merge_sha.as_str())
            .ok_or_else(|| anyhow::anyhow!("production success evidence SHA is unavailable"))?;
        validate_git_sha(evidence_sha)?;
        if !raw
            .sample
            .rows
            .iter()
            .any(|row| row.merge_sha == evidence_sha && row.evidence_sha_matches_actions_target)
        {
            bail!("production evidence SHA does not match a verified Actions target");
        }
        let sha_criterion = raw
            .criteria
            .get("evidenceShaMismatch")
            .ok_or_else(|| anyhow::anyhow!("evidence SHA criterion is missing"))?;
        if sha_criterion.status != "pass" {
            bail!("evidence SHA mismatch criterion failed");
        }
        if raw
            .sample
            .rows
            .iter()
            .any(|row| !row.evidence_sha_matches_actions_target)
        {
            bail!("one or more sampled Actions targets have an evidence SHA mismatch");
        }

        let criteria = raw
            .criteria
            .iter()
            .map(|(key, criterion)| {
                validate_text(key, "criterion key")?;
                validate_text(&criterion.source, "criterion source")?;
                validate_text(&criterion.note, "criterion note")?;
                if !matches!(criterion.status.as_str(), "pass" | "fail") {
                    bail!("criterion status is unsupported");
                }
                Ok(PrValidationCriterionSnapshot {
                    key: key.clone(),
                    status: criterion.status.clone(),
                    source: criterion.source.clone(),
                    note: criterion.note.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let failed_criteria = criteria
            .iter()
            .filter(|criterion| criterion.status != "pass")
            .map(|criterion| format!("{} criterion did not pass", criterion.key))
            .collect::<Vec<_>>();
        let failure_canary_status = raw
            .canaries
            .failure
            .as_ref()
            .map(|canary| canary.status.clone());
        if failure_canary_status
            .as_deref()
            .is_some_and(|status| !matches!(status, "pass" | "fail"))
        {
            bail!("failure canary status is unsupported");
        }
        let decision_ready = raw.decision.status == "ready_for_remediate";
        if decision_ready
            && (!failed_criteria.is_empty()
                || failure_canary_status.as_deref() != Some("pass")
                || !raw.decision.queue_admission_enabled
                || recommended_scheduler_mode != "remediate")
        {
            bail!("ready rollout decision conflicts with its supporting evidence");
        }
        if !decision_ready
            && (raw.decision.queue_admission_enabled || recommended_scheduler_mode != "observe")
        {
            bail!("hold rollout decision must keep Queue admission disabled");
        }

        let stale = now.signed_duration_since(generated_at) > self.freshness;
        let status = if stale {
            PrValidationRolloutEvidenceStatus::Stale
        } else if decision_ready {
            PrValidationRolloutEvidenceStatus::Ready
        } else {
            PrValidationRolloutEvidenceStatus::Hold
        };
        let mut blockers = failed_criteria;
        if failure_canary_status.as_deref() != Some("pass") {
            blockers.push("failure canary did not pass".to_string());
        }
        if stale {
            blockers.push("rollout evidence exceeded the freshness window".to_string());
        }
        if blockers.is_empty() && !decision_ready {
            blockers.push("rollout decision keeps scheduler in observe mode".to_string());
        }
        let evidence_short_sha = evidence_sha.chars().take(8).collect::<String>();
        let production_success_canary = production_canary
            .map(|canary| -> Result<PrValidationProductionCanarySnapshot> {
                validate_canonical_actions_url(
                    &canary.run_url,
                    &raw.repository,
                    GithubActionsUrlKind::Run,
                )?;
                Ok(PrValidationProductionCanarySnapshot {
                    pull_request_number: canary.pull_request_number,
                    evidence_short_sha: evidence_short_sha.clone(),
                    run_url: Some(canary.run_url.clone()),
                })
            })
            .transpose()?;
        let summary = PrValidationRolloutEvidenceSummary {
            status,
            generated_at: Some(generated_at.to_rfc3339()),
            repository: Some(raw.repository),
            base_branch: Some(raw.base_branch),
            evidence_short_sha: Some(evidence_short_sha),
            recommended_scheduler_mode: Some(recommended_scheduler_mode),
            queue_admission_supported: status == PrValidationRolloutEvidenceStatus::Ready
                && raw.decision.queue_admission_enabled,
            sample_window: PrValidationSampleWindowSnapshot {
                pull_request_count: Some(raw.sample.pull_request_count),
                earliest_merged_at: Some(earliest_merged_at.to_rfc3339()),
                latest_merged_at: Some(latest_merged_at.to_rfc3339()),
                window_hours: Some(raw.sample.window_hours),
                source: "github_live_sample".to_string(),
            },
            historical_fast_gate,
            actual_fast_gate,
            ci_gate,
            post_merge_gate,
            post_merge_failure_rate_percent: raw.post_merge.failure_rate_percent,
            quota_used_percent: raw.quota.used_percent,
            blockers,
        };
        Ok(PrValidationRolloutEvidenceSnapshot {
            artifact_short_sha: Some(short_sha(artifact_sha)),
            observed_at: observed_at.to_rfc3339(),
            summary,
            quota: PrValidationQuotaSnapshot {
                limit: Some(raw.quota.limit),
                remaining: Some(raw.quota.remaining),
                used: Some(raw.quota.used),
                used_percent: raw.quota.used_percent,
                reset_at: Some(reset_at.to_rfc3339()),
                collector_requests: Some(raw.quota.collector_requests),
            },
            criteria,
            production_success_canary,
            failure_canary_status,
        })
    }
}

impl PrValidationRolloutEvidenceQueryPort for PrValidationRolloutEvidenceQueryService {
    fn load_latest_summary(&self) -> PrValidationRolloutEvidenceSummary {
        let now = Instant::now();
        if let Some(summary) = self
            .latest_summary_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .filter(|(cached_at, _)| now.duration_since(*cached_at) < LATEST_SUMMARY_CACHE_TTL)
            .map(|(_, summary)| summary.clone())
        {
            return summary;
        }
        let summary = self
            .load_page_at(PrValidationRolloutEvidenceRequest::default(), Utc::now())
            .map(|page| page.latest.summary)
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "PR validation rollout evidence projection failed");
                PrValidationRolloutEvidenceSummary::unavailable(
                    "rollout evidence projection is unavailable",
                )
            });
        *self
            .latest_summary_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((now, summary.clone()));
        summary
    }

    fn load_page(
        &self,
        request: PrValidationRolloutEvidenceRequest,
    ) -> Result<PrValidationRolloutEvidencePage> {
        self.load_page_at(request, Utc::now())
    }
}

enum MetricContract {
    Historical { actual_count: usize },
    Actual(&'static str),
}

fn project_metric(
    raw: &RawMetric,
    contract: MetricContract,
) -> Result<PrValidationGateMetricSnapshot> {
    if raw.sample_count == 0 {
        if raw.p50_seconds.is_some() || raw.p95_seconds.is_some() {
            bail!("empty metric must not synthesize percentile values");
        }
    } else {
        let p50 = valid_metric_seconds(raw.p50_seconds, "p50")?;
        let p95 = valid_metric_seconds(raw.p95_seconds, "p95")?;
        if p95 < p50 {
            bail!("metric p95 must not be lower than p50");
        }
    }
    let (label, source) = match contract {
        MetricContract::Historical { actual_count } => {
            let label = match raw.label.as_str() {
                "projected" if actual_count == 0 => PrValidationMetricLabel::Projected,
                "mixed_actual_and_projected" if actual_count > 0 => {
                    PrValidationMetricLabel::MixedActualAndProjected
                }
                "projected" => bail!("projected Fast Gate contains actual sample rows"),
                "mixed_actual_and_projected" => {
                    bail!("mixed Fast Gate has no actual sample row")
                }
                _ => bail!("historical Fast Gate label is unsupported"),
            };
            (label, "historical_pr_sample")
        }
        MetricContract::Actual(source) => {
            if raw.label != "actual" {
                bail!("actual gate metric label is unsupported");
            }
            (PrValidationMetricLabel::Actual, source)
        }
    };
    Ok(PrValidationGateMetricSnapshot {
        label,
        sample_count: Some(raw.sample_count),
        p50_seconds: raw.p50_seconds,
        p95_seconds: raw.p95_seconds,
        source: source.to_string(),
    })
}

fn valid_metric_seconds(value: Option<f64>, label: &str) -> Result<f64> {
    let value = value.ok_or_else(|| anyhow::anyhow!("metric {label} is unavailable"))?;
    if !value.is_finite() || !(0.0..=MAX_METRIC_SECONDS).contains(&value) {
        bail!("metric {label} is outside the bounded contract");
    }
    Ok(value)
}

fn validate_actual_runs(runs: &[RawActualFastGateRun], repository: &str) -> Result<()> {
    if runs.len() > MAX_SAMPLE_ROWS {
        bail!("independent actual Fast Gate runs exceed the bounded contract");
    }
    for run in runs {
        if !matches!(
            &run.id,
            serde_json::Value::Number(_) | serde_json::Value::String(_)
        ) {
            bail!("actual Fast Gate run identity is invalid");
        }
        validate_git_sha(&run.head_sha)?;
        validate_text(&run.conclusion, "actual Fast Gate conclusion")?;
        if !run.seconds.is_finite() || !(0.0..=MAX_METRIC_SECONDS).contains(&run.seconds) {
            bail!("actual Fast Gate duration is outside the bounded contract");
        }
        validate_canonical_actions_url(&run.run_url, repository, GithubActionsUrlKind::Run)?;
        validate_canonical_actions_url(&run.job_url, repository, GithubActionsUrlKind::Job)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GithubActionsUrlKind {
    Run,
    Job,
}

fn validate_canonical_actions_url(
    value: &str,
    repository: &str,
    kind: GithubActionsUrlKind,
) -> Result<()> {
    validate_text(value, "run URL")?;
    let expected_prefix = format!("https://github.com/{repository}/actions/runs/");
    let Some(suffix) = value.strip_prefix(&expected_prefix) else {
        bail!("run URL is outside the canonical repository");
    };
    let segments = suffix.split('/').collect::<Vec<_>>();
    let canonical = match (kind, segments.as_slice()) {
        (GithubActionsUrlKind::Run, [run_id]) => is_positive_decimal(run_id),
        (GithubActionsUrlKind::Job, [run_id, "job", job_id]) => {
            is_positive_decimal(run_id) && is_positive_decimal(job_id)
        }
        _ => false,
    };
    if !canonical {
        bail!("run URL is not a canonical GitHub Actions URL");
    }
    Ok(())
}

fn is_positive_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) && value != "0"
}

fn validate_text(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > MAX_STRING_LENGTH
        || value.chars().any(char::is_control)
    {
        bail!("rollout evidence {label} is invalid");
    }
    Ok(())
}

fn validate_git_sha(value: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64) || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("rollout evidence SHA is invalid");
    }
    Ok(())
}

fn validate_percentage(value: Option<f64>, label: &str) -> Result<()> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value)) {
        bail!("{label} is outside the bounded contract");
    }
    Ok(())
}

fn approximately_equal(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => (left - right).abs() <= 0.02,
        _ => false,
    }
}

fn parse_time(value: &str, label: &str) -> Result<DateTime<Utc>> {
    validate_text(value, label)?;
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| anyhow::anyhow!("rollout evidence {label} is invalid"))
}

fn hex_sha256(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn short_sha(value: &str) -> String {
    value.chars().take(12).collect()
}

fn invalid_snapshot(
    observed_at: DateTime<Utc>,
    artifact_sha: &str,
    reason: String,
) -> PrValidationRolloutEvidenceSnapshot {
    let mut summary = PrValidationRolloutEvidenceSummary::unavailable(reason);
    summary.status = PrValidationRolloutEvidenceStatus::Invalid;
    PrValidationRolloutEvidenceSnapshot {
        artifact_short_sha: Some(short_sha(artifact_sha)),
        observed_at: observed_at.to_rfc3339(),
        summary,
        quota: empty_quota(),
        criteria: Vec::new(),
        production_success_canary: None,
        failure_canary_status: None,
    }
}

fn unavailable_snapshot(now: DateTime<Utc>, reason: &str) -> PrValidationRolloutEvidenceSnapshot {
    PrValidationRolloutEvidenceSnapshot {
        artifact_short_sha: None,
        observed_at: now.to_rfc3339(),
        summary: PrValidationRolloutEvidenceSummary::unavailable(reason),
        quota: empty_quota(),
        criteria: Vec::new(),
        production_success_canary: None,
        failure_canary_status: None,
    }
}

fn unavailable_page(now: DateTime<Utc>, reason: &str) -> PrValidationRolloutEvidencePage {
    PrValidationRolloutEvidencePage {
        latest: unavailable_snapshot(now, reason),
        last_valid: None,
        history: Vec::new(),
        next_cursor: None,
    }
}

fn empty_quota() -> PrValidationQuotaSnapshot {
    PrValidationQuotaSnapshot {
        limit: None,
        remaining: None,
        used: None,
        used_percent: None,
        reset_at: None,
        collector_requests: None,
    }
}

fn decode_cursor(value: Option<&str>, projected: &[ProjectedDocument]) -> Result<usize> {
    let Some(value) = value else {
        return Ok(0);
    };
    let invalid = || anyhow::Error::new(PrValidationRolloutEvidenceCursorError);
    let payload = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    let cursor = serde_json::from_slice::<EvidenceCursor>(&payload).map_err(|_| invalid())?;
    if cursor.version != CURSOR_VERSION
        || cursor.artifact_sha.len() != 64
        || !cursor.artifact_sha.chars().all(|ch| ch.is_ascii_hexdigit())
        || DateTime::parse_from_rfc3339(&cursor.sort_at).is_err()
    {
        return Err(invalid());
    }
    projected
        .iter()
        .position(|document| {
            document.artifact_sha == cursor.artifact_sha
                && document.sort_at.to_rfc3339() == cursor.sort_at
        })
        .map(|position| position + 1)
        .ok_or_else(invalid)
}

fn encode_cursor(document: &ProjectedDocument) -> Result<String> {
    let payload = serde_json::to_vec(&EvidenceCursor {
        version: CURSOR_VERSION,
        sort_at: document.sort_at.to_rfc3339(),
        artifact_sha: document.artifact_sha.clone(),
    })?;
    Ok(URL_SAFE_NO_PAD.encode(payload))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};

    use super::*;

    const CHECKED_IN_EVIDENCE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/validation/artifacts/post-merge-validation-rollout-2026-08-10/evidence.json"
    ));

    #[derive(Default)]
    struct FakeEvidencePort {
        documents: Mutex<Vec<PrValidationRolloutEvidenceDocument>>,
        fail: bool,
    }

    impl PrValidationRolloutEvidencePort for FakeEvidencePort {
        fn load_documents(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<PrValidationRolloutEvidenceDocument>> {
            if self.fail {
                bail!("sensitive/path/that/must/not/escape")
            }
            Ok(self.documents.lock().expect("documents lock").clone())
        }
    }

    struct CountingEvidencePort {
        calls: AtomicUsize,
        document: PrValidationRolloutEvidenceDocument,
    }

    impl PrValidationRolloutEvidencePort for CountingEvidencePort {
        fn load_documents(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<PrValidationRolloutEvidenceDocument>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![self.document.clone()])
        }
    }

    fn valid_evidence(generated_at: &str, historical_label: &str, actual_in_sample: bool) -> Value {
        let merge_sha = "0123456789abcdef0123456789abcdef01234567";
        json!({
            "schemaVersion": 1,
            "generatedAt": generated_at,
            "repository": "acme/widgets",
            "baseBranch": "prerelease",
            "decision": {
                "status": "ready_for_remediate",
                "recommendedSchedulerMode": "remediate",
                "queueAdmissionEnabled": true,
                "rulesetChange": "approval_required",
                "reason": "validated rollout evidence"
            },
            "sample": {
                "pullRequestCount": 1,
                "earliestMergedAt": "2026-08-09T10:00:00Z",
                "latestMergedAt": "2026-08-10T10:00:00Z",
                "windowHours": 24.0,
                "rows": [{
                    "mergeSha": merge_sha,
                    "evidenceShaMatchesActionsTarget": true,
                    "preMerge": { "fastGate": { "source": if actual_in_sample { "actual" } else { "projected" } } }
                }]
            },
            "timings": {
                "fastGate": { "sampleCount": 1, "p50Seconds": 80, "p95Seconds": 90, "label": historical_label },
                "actualFastGate": { "sampleCount": 1, "p50Seconds": 70, "p95Seconds": 70, "label": "actual" },
                "ciGate": { "sampleCount": 1, "p50Seconds": 500, "p95Seconds": 500, "label": "actual" },
                "postMergeGate": { "sampleCount": 1, "p50Seconds": 510, "p95Seconds": 510, "label": "actual" }
            },
            "actualFastGateRuns": [{
                "id": 99,
                "headSha": merge_sha,
                "conclusion": "success",
                "seconds": 70,
                "runUrl": "https://github.com/acme/widgets/actions/runs/99",
                "jobUrl": "https://github.com/acme/widgets/actions/runs/99/job/1"
            }],
            "postMerge": { "sampleCount": 1, "failureCount": 0, "failureRatePercent": 0 },
            "quota": { "limit": 5000, "remaining": 4900, "used": 100, "reset": 1786388400, "collectorRequests": 3, "usedPercent": 2 },
            "criteria": {
                "sampleWindow": { "status": "pass", "source": "github_live_sample", "note": "window passed" },
                "evidenceShaMismatch": { "status": "pass", "source": "github_live_sample", "note": "SHA matched" }
            },
            "canaries": {
                "productionSuccess": {
                    "status": "pass",
                    "pullRequestNumber": 42,
                    "mergeSha": merge_sha,
                    "runUrl": "https://github.com/acme/widgets/actions/runs/99"
                },
                "failure": { "status": "pass" }
            }
        })
    }

    fn document(value: Value, observed_at: &str) -> PrValidationRolloutEvidenceDocument {
        PrValidationRolloutEvidenceDocument {
            observed_at: observed_at.to_string(),
            content: serde_json::to_vec(&value).expect("fixture should serialize"),
        }
    }

    fn service(
        documents: Vec<PrValidationRolloutEvidenceDocument>,
    ) -> PrValidationRolloutEvidenceQueryService {
        PrValidationRolloutEvidenceQueryService::new(
            "/workspace",
            Some("acme/widgets".to_string()),
            "prerelease",
            Arc::new(FakeEvidencePort {
                documents: Mutex::new(documents),
                fail: false,
            }),
        )
    }

    #[test]
    fn actual_and_projected_metrics_remain_independent() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let page = service(vec![document(
            valid_evidence("2026-08-10T23:00:00Z", "projected", false),
            "2026-08-10T23:01:00Z",
        )])
        .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
        .expect("evidence should project");

        assert_eq!(
            page.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Ready
        );
        assert_eq!(
            page.latest.summary.historical_fast_gate.label,
            PrValidationMetricLabel::Projected
        );
        assert_eq!(
            page.latest.summary.actual_fast_gate.label,
            PrValidationMetricLabel::Actual
        );
        assert_eq!(page.latest.summary.actual_fast_gate.sample_count, Some(1));
        assert_eq!(
            page.latest.summary.evidence_short_sha.as_deref(),
            Some("01234567")
        );
    }

    #[test]
    fn checked_in_rollout_artifact_matches_the_application_contract() {
        let generated_at = serde_json::from_slice::<Value>(CHECKED_IN_EVIDENCE)
            .expect("checked-in evidence should be JSON")["generatedAt"]
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .expect("checked-in generated-at should parse")
            .with_timezone(&Utc);
        let document = PrValidationRolloutEvidenceDocument {
            observed_at: generated_at.to_rfc3339(),
            content: CHECKED_IN_EVIDENCE.to_vec(),
        };
        let service = PrValidationRolloutEvidenceQueryService::new(
            "/workspace",
            Some("RefinedStone/codex-exec-loop".to_string()),
            "prerelease",
            Arc::new(FakeEvidencePort {
                documents: Mutex::new(vec![document]),
                fail: false,
            }),
        );

        let page = service
            .load_page_at(
                PrValidationRolloutEvidenceRequest::default(),
                generated_at + Duration::hours(1),
            )
            .expect("checked-in evidence should project");

        assert_eq!(
            page.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Ready
        );
        assert_eq!(
            page.latest.summary.historical_fast_gate.label,
            PrValidationMetricLabel::MixedActualAndProjected
        );
        assert_eq!(page.latest.summary.actual_fast_gate.sample_count, Some(1));
        assert_eq!(page.latest.summary.ci_gate.sample_count, Some(10));
        assert_eq!(page.latest.summary.post_merge_gate.sample_count, Some(6));
    }

    #[test]
    fn compact_dashboard_summary_uses_a_short_shared_cache() {
        let port = Arc::new(CountingEvidencePort {
            calls: AtomicUsize::new(0),
            document: document(
                valid_evidence("2026-08-10T23:00:00Z", "projected", false),
                "2026-08-10T23:01:00Z",
            ),
        });
        let service = PrValidationRolloutEvidenceQueryService::new(
            "/workspace",
            Some("acme/widgets".to_string()),
            "prerelease",
            port.clone(),
        );

        let first = service.load_latest_summary();
        let second = service.load_latest_summary();

        assert_eq!(first, second);
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn mixed_label_requires_an_actual_row_in_the_historical_sample() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let invalid = service(vec![document(
            valid_evidence("2026-08-10T23:00:00Z", "mixed_actual_and_projected", false),
            "2026-08-10T23:01:00Z",
        )])
        .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
        .expect("invalid evidence is a typed projection");
        assert_eq!(
            invalid.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Invalid
        );

        let mixed = service(vec![document(
            valid_evidence("2026-08-10T23:00:00Z", "mixed_actual_and_projected", true),
            "2026-08-10T23:01:00Z",
        )])
        .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
        .expect("mixed evidence should project");
        assert_eq!(
            mixed.latest.summary.historical_fast_gate.label,
            PrValidationMetricLabel::MixedActualAndProjected
        );
    }

    #[test]
    fn stale_schema_repository_and_sha_fail_closed_without_raw_payload_leaks() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let mut stale_value = valid_evidence("2026-08-01T12:00:00Z", "projected", false);
        stale_value["sample"]["earliestMergedAt"] = json!("2026-07-31T10:00:00Z");
        stale_value["sample"]["latestMergedAt"] = json!("2026-08-01T10:00:00Z");
        let stale = service(vec![document(stale_value, "2026-08-10T23:01:00Z")])
            .with_freshness(Duration::hours(24))
            .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
            .expect("stale evidence should project");
        assert_eq!(
            stale.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Stale
        );
        assert!(!stale.latest.summary.queue_admission_supported);

        for mutate in ["schema", "repository", "sha"] {
            let mut value = valid_evidence("2026-08-10T23:00:00Z", "projected", false);
            match mutate {
                "schema" => value["schemaVersion"] = json!(9),
                "repository" => value["repository"] = json!("other/repository"),
                "sha" => value["criteria"]["evidenceShaMismatch"]["status"] = json!("fail"),
                _ => unreachable!(),
            }
            let page = service(vec![document(value, "2026-08-10T23:01:00Z")])
                .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
                .expect("invalid evidence should be returned as typed state");
            assert_eq!(
                page.latest.summary.status,
                PrValidationRolloutEvidenceStatus::Invalid
            );
            let serialized = serde_json::to_string(&page).expect("page should serialize");
            assert!(!serialized.contains("hidden/evidence.json"));
        }
    }

    #[test]
    fn unavailable_source_does_not_propagate_sensitive_adapter_errors() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let service = PrValidationRolloutEvidenceQueryService::new(
            "/workspace",
            Some("acme/widgets".to_string()),
            "prerelease",
            Arc::new(FakeEvidencePort {
                documents: Mutex::new(Vec::new()),
                fail: true,
            }),
        );
        let page = service
            .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
            .expect("source errors should become unavailable state");
        assert_eq!(
            page.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Unavailable
        );
        assert!(
            !serde_json::to_string(&page)
                .unwrap()
                .contains("sensitive/path")
        );
    }

    #[test]
    fn history_is_bounded_stably_sorted_and_cursor_validated() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let documents = (0..3)
            .map(|offset| {
                document(
                    valid_evidence(
                        &format!("2026-08-10T2{}:00:00Z", offset),
                        "projected",
                        false,
                    ),
                    &format!("2026-08-10T2{}:01:00Z", offset),
                )
            })
            .collect();
        let service = service(documents);
        let first = service
            .load_page_at(
                PrValidationRolloutEvidenceRequest {
                    limit: 2,
                    cursor: None,
                },
                now,
            )
            .expect("first page should load");
        assert_eq!(first.history.len(), 2);
        let second = service
            .load_page_at(
                PrValidationRolloutEvidenceRequest {
                    limit: 2,
                    cursor: first.next_cursor,
                },
                now,
            )
            .expect("second page should load");
        assert_eq!(second.history.len(), 1);
        let error = service
            .load_page_at(
                PrValidationRolloutEvidenceRequest {
                    limit: 2,
                    cursor: Some("malformed".to_string()),
                },
                now,
            )
            .expect_err("malformed cursor must fail");
        assert!(
            error
                .downcast_ref::<PrValidationRolloutEvidenceCursorError>()
                .is_some()
        );

        let error = service
            .load_page_at(
                PrValidationRolloutEvidenceRequest {
                    limit: PR_VALIDATION_EVIDENCE_MAX_HISTORY_LIMIT + 1,
                    cursor: None,
                },
                now,
            )
            .expect_err("oversized history requests must fail");
        assert!(
            error
                .downcast_ref::<PrValidationRolloutEvidenceLimitError>()
                .is_some()
        );
    }

    #[test]
    fn generated_at_controls_history_order_independently_of_file_observation_time() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let page = service(vec![
            document(
                valid_evidence("2026-08-10T22:00:00Z", "projected", false),
                "2026-08-10T23:59:00Z",
            ),
            document(
                valid_evidence("2026-08-10T23:00:00Z", "projected", false),
                "2026-08-10T23:01:00Z",
            ),
        ])
        .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
        .expect("history should load");

        assert_eq!(
            page.latest.summary.generated_at.as_deref(),
            Some("2026-08-10T23:00:00+00:00")
        );
        assert_eq!(
            page.history[1].summary.generated_at.as_deref(),
            Some("2026-08-10T22:00:00+00:00")
        );
    }

    #[test]
    fn missing_repository_identity_and_invalid_observation_time_fail_closed() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let value = valid_evidence("2026-08-10T23:00:00Z", "projected", false);
        let unavailable = PrValidationRolloutEvidenceQueryService::new(
            "/workspace",
            None,
            "prerelease",
            Arc::new(FakeEvidencePort {
                documents: Mutex::new(vec![document(value.clone(), "2026-08-10T23:01:00Z")]),
                fail: false,
            }),
        )
        .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
        .expect("missing repository identity should be typed state");
        assert_eq!(
            unavailable.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Unavailable
        );

        let invalid = service(vec![document(value, "not-a-timestamp")])
            .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
            .expect("invalid observation time should be typed state");
        assert_eq!(
            invalid.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Invalid
        );
    }

    #[test]
    fn inconsistent_counts_and_noncanonical_links_are_invalid() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        for mutate in ["ci_count", "quota", "sample_time", "run_url"] {
            let mut value = valid_evidence("2026-08-10T23:00:00Z", "projected", false);
            match mutate {
                "ci_count" => value["timings"]["ciGate"]["sampleCount"] = json!(2),
                "quota" => value["quota"]["remaining"] = json!(4_899),
                "sample_time" => value["generatedAt"] = json!("2026-08-09T00:00:00Z"),
                "run_url" => {
                    value["canaries"]["productionSuccess"]["runUrl"] =
                        json!("https://github.com/acme/widgets/actions/runs/99?token=secret")
                }
                _ => unreachable!(),
            }
            let page = service(vec![document(value, "2026-08-10T23:01:00Z")])
                .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
                .expect("invalid evidence should remain a typed projection");
            assert_eq!(
                page.latest.summary.status,
                PrValidationRolloutEvidenceStatus::Invalid,
                "mutation {mutate} must fail closed"
            );
            assert!(
                !serde_json::to_string(&page)
                    .expect("page should serialize")
                    .contains("secret")
            );
        }
    }

    #[test]
    fn newer_invalid_observation_keeps_last_valid_snapshot_distinct() {
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let valid = document(
            valid_evidence("2026-08-10T21:00:00Z", "projected", false),
            "2026-08-10T21:01:00Z",
        );
        let invalid = PrValidationRolloutEvidenceDocument {
            observed_at: "2026-08-10T23:01:00Z".to_string(),
            content: b"not-json".to_vec(),
        };
        let page = service(vec![valid, invalid])
            .load_page_at(PrValidationRolloutEvidenceRequest::default(), now)
            .expect("page should load");
        assert_eq!(
            page.latest.summary.status,
            PrValidationRolloutEvidenceStatus::Invalid
        );
        assert_eq!(
            page.last_valid
                .as_ref()
                .map(|snapshot| snapshot.summary.status),
            Some(PrValidationRolloutEvidenceStatus::Ready)
        );
    }
}
