use std::sync::Arc;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::application::port::inbound::pr_validation_query_port::{
    PrValidationAdminCheck, PrValidationAdminCheckStatus, PrValidationAdminCorrelation,
    PrValidationAdminPhase, PrValidationAdminProvider, PrValidationAdminRecord,
    PrValidationAdminSchedule, PrValidationAdminSeverity, PrValidationAdminWorkflow,
    PrValidationBoardCursorError, PrValidationBoardRequest, PrValidationBoardSnapshot,
    PrValidationBoardSummary, PrValidationDetailRequest, PrValidationQueryPort,
    PrValidationStatusRequest,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
    PrValidationAuthorityPagePosition, PrValidationAuthorityPageRequest,
    PrValidationAuthorityRecordSnapshot,
};
use crate::domain::parallel_mode::{
    PrValidationObservedCheck, PrValidationObservedCheckStatus,
    PrValidationObservedProviderLifecycle, PrValidationObservedProviderStatus,
    PrValidationObservedRunStatus, PrValidationPhase, PrValidationPollErrorClass,
    PrValidationRecord, PrValidationRecordKey,
};

const BOARD_CURSOR_VERSION: u8 = 1;
const RECENT_TERMINAL_DAYS: i64 = 30;
const BOARD_CURSOR_CUTOFF_TOLERANCE_DAYS: i64 = 1;
const STALE_AFTER_SECONDS: i64 = 120;

pub struct PrValidationQueryService {
    workspace_dir: String,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PrValidationBoardCursor {
    version: u8,
    revision: i64,
    terminal_since: String,
    terminal_rank: u8,
    updated_at: String,
    record_key: String,
}

impl PrValidationQueryService {
    pub fn new(
        workspace_dir: impl Into<String>,
        planning_authority: Arc<dyn PlanningAuthorityPort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            planning_authority,
        }
    }

    fn board_cursor(
        value: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Option<PrValidationBoardCursor>> {
        let Some(value) = value else {
            return Ok(None);
        };
        let invalid = || anyhow::Error::new(PrValidationBoardCursorError);
        let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
        let cursor =
            serde_json::from_slice::<PrValidationBoardCursor>(&bytes).map_err(|_| invalid())?;
        if cursor.version != BOARD_CURSOR_VERSION
            || cursor.revision < 0
            || cursor.terminal_rank > 1
            || cursor.record_key.trim().is_empty()
            || cursor.record_key.len() > 200
        {
            return Err(invalid());
        }
        let terminal_since = DateTime::parse_from_rfc3339(&cursor.terminal_since)
            .map_err(|_| invalid())?
            .with_timezone(&Utc);
        let earliest =
            now - Duration::days(RECENT_TERMINAL_DAYS + BOARD_CURSOR_CUTOFF_TOLERANCE_DAYS);
        let latest =
            now - Duration::days(RECENT_TERMINAL_DAYS - BOARD_CURSOR_CUTOFF_TOLERANCE_DAYS);
        if terminal_since < earliest || terminal_since > latest {
            return Err(invalid());
        }
        DateTime::parse_from_rfc3339(&cursor.updated_at).map_err(|_| invalid())?;
        Ok(Some(cursor))
    }

    fn encode_cursor(
        revision: i64,
        terminal_since: &str,
        position: PrValidationAuthorityPagePosition,
    ) -> Result<String> {
        let payload = serde_json::to_vec(&PrValidationBoardCursor {
            version: BOARD_CURSOR_VERSION,
            revision,
            terminal_since: terminal_since.to_string(),
            terminal_rank: position.terminal_rank,
            updated_at: position.updated_at,
            record_key: position.record_key,
        })
        .context("failed to serialize PR validation board cursor")?;
        Ok(URL_SAFE_NO_PAD.encode(payload))
    }

    fn runtime_projection(&self) -> Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        self.planning_authority
            .load_runtime_projections(&self.workspace_dir)
            .context("failed to load PR validation worker correlations")
    }
}

impl PrValidationQueryPort for PrValidationQueryService {
    fn status_for_pr(
        &self,
        request: PrValidationStatusRequest,
    ) -> Result<Option<crate::domain::parallel_mode::PrValidationOperatorSummary>> {
        if request.pull_request_number == 0 {
            bail!("pull request number must be positive");
        }
        self.planning_authority
            .load_runtime_pr_validation_record_for_pr(
                &self.workspace_dir,
                request.pull_request_number,
            )
            .map(|record| record.map(|record| record.operator_summary()))
    }

    fn load_board(&self, request: PrValidationBoardRequest) -> Result<PrValidationBoardSnapshot> {
        if !(1..=crate::application::port::inbound::pr_validation_query_port::PR_VALIDATION_BOARD_MAX_LIMIT)
            .contains(&request.limit)
        {
            bail!("PR validation board limit is outside the bounded contract");
        }
        let generated_at = Utc::now();
        let cursor = Self::board_cursor(request.cursor.as_deref(), generated_at)?;
        let terminal_since = cursor
            .as_ref()
            .map(|cursor| cursor.terminal_since.clone())
            .unwrap_or_else(|| (generated_at - Duration::days(RECENT_TERMINAL_DAYS)).to_rfc3339());
        let authority_page = self
            .planning_authority
            .load_runtime_pr_validation_page(
                &self.workspace_dir,
                &PrValidationAuthorityPageRequest {
                    limit: request.limit,
                    terminal_since: terminal_since.clone(),
                    after: cursor
                        .as_ref()
                        .map(|cursor| PrValidationAuthorityPagePosition {
                            terminal_rank: cursor.terminal_rank,
                            updated_at: cursor.updated_at.clone(),
                            record_key: cursor.record_key.clone(),
                        }),
                    expected_revision: cursor.as_ref().map(|cursor| cursor.revision),
                },
            )
            .context("failed to load bounded PR validation board")?;
        let runtime = self.runtime_projection()?;
        let records = authority_page
            .records
            .iter()
            .map(|snapshot| map_admin_record(snapshot, &runtime, generated_at))
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = authority_page
            .next_position
            .map(|position| Self::encode_cursor(authority_page.revision, &terminal_since, position))
            .transpose()?;
        Ok(PrValidationBoardSnapshot {
            revision: authority_page.revision,
            summary: PrValidationBoardSummary {
                active: authority_page.summary.active,
                integrated: authority_page.summary.integrated,
                verifying: authority_page.summary.verifying,
                remediation: authority_page.summary.remediation,
                verified: authority_page.summary.verified,
                blocked: authority_page.summary.blocked,
                failed: authority_page.summary.failed,
            },
            records,
            next_cursor,
            cursor_reset_required: authority_page.cursor_reset_required,
            generated_at: generated_at.to_rfc3339(),
        })
    }

    fn load_detail(
        &self,
        request: PrValidationDetailRequest,
    ) -> Result<Option<PrValidationAdminRecord>> {
        let record_key =
            PrValidationRecordKey::new(request.record_key).map_err(anyhow::Error::msg)?;
        let Some(snapshot) = self
            .planning_authority
            .load_runtime_pr_validation_record_snapshot(&self.workspace_dir, &record_key)
            .context("failed to load PR validation detail")?
        else {
            return Ok(None);
        };
        let runtime = self.runtime_projection()?;
        map_admin_record(&snapshot, &runtime, Utc::now()).map(Some)
    }
}

fn map_admin_record(
    snapshot: &PrValidationAuthorityRecordSnapshot,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
    now: DateTime<Utc>,
) -> Result<PrValidationAdminRecord> {
    let record = &snapshot.record;
    let operator = record.operator_summary();
    let attestation = record.integration_attestation();
    let phase = admin_phase(record);
    let checks = admin_checks(record);
    let required_checks_total = checks.iter().filter(|check| check.required).count();
    let required_checks_succeeded = checks
        .iter()
        .filter(|check| check.required && check.status == PrValidationAdminCheckStatus::Succeeded)
        .count();
    let provider_blocked = matches!(
        snapshot.last_error_class,
        Some(
            PrValidationPollErrorClass::AuthenticationBlocked
                | PrValidationPollErrorClass::RetryableProvider
        )
    );
    let stale_reference = snapshot
        .last_polled_at
        .as_deref()
        .unwrap_or(snapshot.updated_at.as_str());
    let stale_seconds = DateTime::parse_from_rfc3339(stale_reference)
        .with_context(|| {
            format!(
                "PR validation record `{}` contains an invalid poll timestamp",
                record.key().as_str()
            )
        })
        .map(|value| (now - value.with_timezone(&Utc)).num_seconds().max(0) as u64)?;
    let correlations = operator
        .correlations
        .iter()
        .map(|correlation| map_correlation(correlation, runtime))
        .collect();
    Ok(PrValidationAdminRecord {
        record_key: record.key().as_str().to_string(),
        akra_id: operator.akra_id,
        repository: record.target().repository().to_string(),
        canonical_pr_url: operator.canonical_pr_url,
        pull_request_number: operator.pull_request_number,
        phase,
        phase_label: admin_phase_label(phase).to_string(),
        severity: admin_severity(phase, snapshot.last_error_class, &checks),
        integrated: record.evidence_sha().is_some(),
        verified: phase == PrValidationAdminPhase::Verified,
        provider_blocked,
        stale: stale_seconds >= STALE_AFTER_SECONDS as u64
            && !matches!(phase, PrValidationAdminPhase::Verified),
        stale_seconds,
        target_sha: record.target_shas().source_sha().as_str().to_string(),
        base_before_sha: attestation
            .and_then(|value| value.base_before_sha())
            .map(|sha| sha.as_str().to_string()),
        evidence_sha: record.evidence_sha().map(|sha| sha.as_str().to_string()),
        merge_sha: record.merge_sha().map(|sha| sha.as_str().to_string()),
        target_short_sha: operator.target_short_sha,
        evidence_short_sha: operator.evidence_short_sha,
        merge_short_sha: operator.merge_short_sha,
        integration_method: operator
            .integration_method
            .map(|method| method.label().to_string()),
        integration_pull_request_number: attestation.and_then(|value| value.pull_request_number()),
        integrated_at: attestation.map(|value| value.integrated_at().to_rfc3339()),
        remote_verified_at: attestation.map(|value| value.remote_verified_at().to_rfc3339()),
        reason: operator.reason,
        blocker: operator.blocker,
        recovery_action: operator.recovery_action,
        checks,
        required_checks_succeeded,
        required_checks_total,
        workflows: record
            .observation_projection()
            .workflows()
            .iter()
            .map(|workflow| PrValidationAdminWorkflow {
                name: workflow.name().to_string(),
                status: observed_run_status_label(workflow.status()).to_string(),
                run_attempt: workflow.run_attempt(),
                started_at: workflow.started_at().map(str::to_string),
                updated_at: workflow.updated_at().map(str::to_string),
            })
            .collect(),
        providers: record
            .observation_projection()
            .providers()
            .iter()
            .map(|provider| PrValidationAdminProvider {
                key: provider.key().to_string(),
                lifecycle: match provider.lifecycle() {
                    PrValidationObservedProviderLifecycle::Finite => "finite",
                    PrValidationObservedProviderLifecycle::Watchable => "watchable",
                }
                .to_string(),
                status: match provider.status() {
                    PrValidationObservedProviderStatus::Pending => "pending",
                    PrValidationObservedProviderStatus::Complete => "complete",
                    PrValidationObservedProviderStatus::Failed => "failed",
                }
                .to_string(),
                page_complete: provider.page_complete(),
            })
            .collect(),
        finding_count: operator.finding_count,
        remediation_count: operator.remediation_count,
        correlations,
        schedule: PrValidationAdminSchedule {
            updated_at: snapshot.updated_at.clone(),
            last_polled_at: snapshot.last_polled_at.clone(),
            next_poll_at: snapshot.next_poll_at.clone(),
            poll_attempt: snapshot.poll_attempt,
            consecutive_error_count: snapshot.consecutive_error_count,
            error_class: snapshot
                .last_error_class
                .map(|class| class.label().to_string()),
            rate_limit_remaining: snapshot.rate_limit_remaining,
            rate_limit_reset_at: snapshot.rate_limit_reset_at.clone(),
        },
        observation_revision: operator.observation_revision,
        post_merge_checkpoint_observed: operator.post_merge_checkpoint_observed,
    })
}

fn admin_phase(record: &PrValidationRecord) -> PrValidationAdminPhase {
    match record.phase() {
        PrValidationPhase::Registered => PrValidationAdminPhase::Registered,
        PrValidationPhase::PreMergeObservation => PrValidationAdminPhase::PreMerge,
        PrValidationPhase::PostMergeObservation if record.has_post_merge_checkpoint() => {
            PrValidationAdminPhase::Verifying
        }
        PrValidationPhase::PostMergeObservation => PrValidationAdminPhase::Integrated,
        PrValidationPhase::RemediationQueued => PrValidationAdminPhase::RemediationQueued,
        PrValidationPhase::RemediationRunning => PrValidationAdminPhase::RemediationRunning,
        PrValidationPhase::Settled => PrValidationAdminPhase::Verified,
        PrValidationPhase::Blocked => PrValidationAdminPhase::Blocked,
        PrValidationPhase::Failed => PrValidationAdminPhase::Failed,
    }
}

fn admin_phase_label(phase: PrValidationAdminPhase) -> &'static str {
    match phase {
        PrValidationAdminPhase::Registered => "Registered",
        PrValidationAdminPhase::PreMerge => "Pre-merge",
        PrValidationAdminPhase::Integrated => "Integrated",
        PrValidationAdminPhase::Verifying => "Verifying",
        PrValidationAdminPhase::RemediationQueued => "Remediation queued",
        PrValidationAdminPhase::RemediationRunning => "Remediation running",
        PrValidationAdminPhase::Verified => "Verified",
        PrValidationAdminPhase::Blocked => "Blocked",
        PrValidationAdminPhase::Failed => "Failed",
    }
}

fn admin_severity(
    phase: PrValidationAdminPhase,
    error_class: Option<PrValidationPollErrorClass>,
    checks: &[PrValidationAdminCheck],
) -> PrValidationAdminSeverity {
    if matches!(
        error_class,
        Some(
            PrValidationPollErrorClass::AuthenticationBlocked
                | PrValidationPollErrorClass::PolicyBlocked
                | PrValidationPollErrorClass::IdentityFailed
                | PrValidationPollErrorClass::IntegrityFailed
        )
    ) || matches!(
        phase,
        PrValidationAdminPhase::Blocked | PrValidationAdminPhase::Failed
    ) || checks.iter().any(|check| {
        matches!(
            check.status,
            PrValidationAdminCheckStatus::ActionableFailure
                | PrValidationAdminCheckStatus::PolicyBlocked
        )
    }) {
        PrValidationAdminSeverity::Danger
    } else if matches!(
        error_class,
        Some(
            PrValidationPollErrorClass::RetryableProvider
                | PrValidationPollErrorClass::AdmissionRetryable
        )
    ) || matches!(
        phase,
        PrValidationAdminPhase::RemediationQueued | PrValidationAdminPhase::RemediationRunning
    ) {
        PrValidationAdminSeverity::Warning
    } else if phase == PrValidationAdminPhase::Verified {
        PrValidationAdminSeverity::Success
    } else if matches!(
        phase,
        PrValidationAdminPhase::Registered | PrValidationAdminPhase::PreMerge
    ) {
        PrValidationAdminSeverity::Muted
    } else {
        PrValidationAdminSeverity::Info
    }
}

fn admin_checks(record: &PrValidationRecord) -> Vec<PrValidationAdminCheck> {
    let projection = record.observation_projection();
    let mut checks = projection
        .required_checks()
        .iter()
        .map(|check| map_check(check, true))
        .chain(
            projection
                .optional_checks()
                .iter()
                .map(|check| map_check(check, false)),
        )
        .collect::<Vec<_>>();
    if checks.is_empty() {
        checks.extend(
            record
                .post_merge_validation_contract()
                .required_check_contexts()
                .iter()
                .map(|context| PrValidationAdminCheck {
                    context: context.context().to_string(),
                    app_slug: context.app_slug().map(str::to_string),
                    required: true,
                    status: PrValidationAdminCheckStatus::Missing,
                    latest_attempt: None,
                    started_at: None,
                    completed_at: None,
                }),
        );
        checks.extend(
            record
                .post_merge_validation_contract()
                .optional_check_contexts()
                .iter()
                .map(|context| PrValidationAdminCheck {
                    context: context.context().to_string(),
                    app_slug: context.app_slug().map(str::to_string),
                    required: false,
                    status: PrValidationAdminCheckStatus::Missing,
                    latest_attempt: None,
                    started_at: None,
                    completed_at: None,
                }),
        );
    }
    checks
}

fn map_check(check: &PrValidationObservedCheck, required: bool) -> PrValidationAdminCheck {
    PrValidationAdminCheck {
        context: check.context().context().to_string(),
        app_slug: check.context().app_slug().map(str::to_string),
        required,
        status: match check.status() {
            PrValidationObservedCheckStatus::Missing => PrValidationAdminCheckStatus::Missing,
            PrValidationObservedCheckStatus::Pending => PrValidationAdminCheckStatus::Pending,
            PrValidationObservedCheckStatus::Succeeded => PrValidationAdminCheckStatus::Succeeded,
            PrValidationObservedCheckStatus::ActionableFailure => {
                PrValidationAdminCheckStatus::ActionableFailure
            }
            PrValidationObservedCheckStatus::PolicyBlocked => {
                PrValidationAdminCheckStatus::PolicyBlocked
            }
        },
        latest_attempt: check.latest_attempt(),
        started_at: check.started_at().map(str::to_string),
        completed_at: check.completed_at().map(str::to_string),
    }
}

fn map_correlation(
    correlation: &crate::domain::parallel_mode::PrValidationOperatorCorrelation,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
) -> PrValidationAdminCorrelation {
    let lease = runtime
        .slot_leases
        .values()
        .find(|lease| lease.task_id == correlation.remediation_akra_id);
    let session = runtime
        .session_details
        .iter()
        .find(|session| session.task_id == correlation.remediation_akra_id);
    let queue = runtime
        .distributor_queue_records
        .iter()
        .find(|queue| queue.task_id == correlation.remediation_akra_id);
    PrValidationAdminCorrelation {
        finding: correlation.finding.clone(),
        remediation_akra_id: correlation.remediation_akra_id.clone(),
        task_state: lease
            .map(|lease| lease.state.label().to_string())
            .or_else(|| session.map(|session| session.state_label.clone()))
            .or_else(|| queue.map(|queue| queue.queue_state.label().to_string())),
        slot_id: lease
            .map(|lease| lease.slot_id.clone())
            .or_else(|| session.map(|session| session.slot_id.clone()))
            .or_else(|| {
                queue
                    .map(|queue| queue.slot_id.clone())
                    .filter(|value| !value.is_empty())
            }),
        session_key: session
            .map(|session| session.session_key.clone())
            .or_else(|| queue.map(|queue| queue.session_key.clone())),
        worker_state: session.map(|session| session.completion_state_label.clone()),
    }
}

fn observed_run_status_label(status: PrValidationObservedRunStatus) -> &'static str {
    match status {
        PrValidationObservedRunStatus::Queued => "queued",
        PrValidationObservedRunStatus::InProgress => "in_progress",
        PrValidationObservedRunStatus::Succeeded => "succeeded",
        PrValidationObservedRunStatus::Failed => "failed",
        PrValidationObservedRunStatus::Cancelled => "cancelled",
        PrValidationObservedRunStatus::Skipped => "skipped",
        PrValidationObservedRunStatus::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_cursor_rejects_forged_unbounded_cutoffs_and_negative_revisions() {
        let now = DateTime::parse_from_rfc3339("2026-08-10T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        for (revision, terminal_since) in [
            (3, "2020-01-01T00:00:00+00:00"),
            (-1, "2026-07-11T12:00:00+00:00"),
        ] {
            let encoded = URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&PrValidationBoardCursor {
                    version: BOARD_CURSOR_VERSION,
                    revision,
                    terminal_since: terminal_since.to_string(),
                    terminal_rank: 0,
                    updated_at: "2026-08-10T11:00:00+00:00".to_string(),
                    record_key: "record-1".to_string(),
                })
                .unwrap(),
            );
            let error = PrValidationQueryService::board_cursor(Some(&encoded), now).unwrap_err();
            assert!(
                error
                    .downcast_ref::<PrValidationBoardCursorError>()
                    .is_some()
            );
        }
    }
}
