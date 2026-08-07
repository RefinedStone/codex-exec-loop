use std::path::PathBuf;

use sha2::{Digest, Sha256};

use super::ParallelModeService;
use crate::application::port::outbound::github_pr_validation_port::{
    GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
    GithubPrValidationSnapshot, GithubValidationRunStatus, GithubValidationSourceStatus,
};
use crate::application::port::outbound::pr_validation_remediation_port::{
    PrValidationRemediationKey, PrValidationRemediationPort, PrValidationRemediationRequest,
};
use crate::domain::github_review::{GithubCommitSha, GithubPullRequestTarget};
use crate::domain::parallel_mode::{
    PrValidationCatchUpState, PrValidationCheckKind, PrValidationCommitSha, PrValidationCompletion,
    PrValidationEvent, PrValidationFinding, PrValidationFindingKey, PrValidationFindingSource,
    PrValidationPhase, PrValidationProviderCompletion, PrValidationProviderKey,
    PrValidationRecordKey, PrValidationRemediationCorrelation, PrValidationRequiredCheck,
    PrValidationTargetShaSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationPollRequest {
    pub workspace_dir: String,
    pub pool_root: PathBuf,
    pub record_key: PrValidationRecordKey,
    pub target_shas: PrValidationTargetShaSnapshot,
    pub delivery_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrValidationPollResult {
    Waiting,
    RemediationRequested {
        idempotency_key: PrValidationRemediationKey,
    },
    Settled,
    StaleDeliveryIgnored,
}

impl ParallelModeService {
    /// Executes one poll/trigger delivery. Waiting is represented by returning `Waiting`; this
    /// service never acquires a pool mutation lock, worktree, or slot lease between deliveries.
    pub fn poll_pr_validation(
        &self,
        observation: &dyn GithubPrValidationPort,
        remediation: &dyn PrValidationRemediationPort,
        request: PrValidationPollRequest,
    ) -> Result<PrValidationPollResult, String> {
        let current = self
            .recover_pr_validation_record(
                &request.workspace_dir,
                &request.pool_root,
                &request.record_key,
            )?
            .ok_or_else(|| {
                format!(
                    "PR validation record `{}` is not registered",
                    request.record_key.as_str()
                )
            })?;
        if request.delivery_revision <= current.observation_revision() {
            return Ok(PrValidationPollResult::StaleDeliveryIgnored);
        }

        let mut next = current.clone();
        if next.target_shas() != &request.target_shas {
            next = next
                .transition(PrValidationEvent::TargetShaChanged(
                    request.target_shas.clone(),
                ))
                .map_err(transition_error)?;
        }
        if next.phase() == PrValidationPhase::Registered {
            next = next
                .transition(PrValidationEvent::BeginPreMergeObservation)
                .map_err(transition_error)?;
        }

        let target = GithubPullRequestTarget::new(
            next.target().repository(),
            next.target().pull_request_number(),
        );
        let target_sha = GithubCommitSha::new(next.target_shas().source_sha().as_str());
        let snapshot = observation
            .load_validation_snapshot(&GithubPrValidationObservationRequest::new(
                target.clone(),
                target_sha.clone(),
                next.observation_cursor()
                    .map(crate::application::port::outbound::github_pr_validation_port::GithubValidationCursor::new),
            ))
            .map_err(|error| format!("failed to obtain trusted PR validation snapshot: {error}"))?;
        validate_snapshot_identity(&snapshot, &target, &target_sha)?;

        let fingerprint = snapshot_fingerprint(&snapshot);
        let prior_fingerprint = next.evidence_fingerprint().map(str::to_string);
        let had_post_merge_checkpoint = next.has_post_merge_checkpoint();
        let mut requested_remediation = None;

        if matches!(
            next.phase(),
            PrValidationPhase::PreMergeObservation | PrValidationPhase::PostMergeObservation
        ) {
            for finding in actionable_findings(&snapshot)? {
                if next.finding_keys().contains(finding.key()) {
                    continue;
                }
                next = next
                    .transition(PrValidationEvent::FindingObserved(finding.clone()))
                    .map_err(transition_error)?;
                let idempotency_key = remediation_key(next.key(), &finding);
                remediation
                    .request_remediation(&PrValidationRemediationRequest {
                        idempotency_key: idempotency_key.clone(),
                        validation_record_key: next.key().clone(),
                        target: next.target().clone(),
                        target_sha: finding.target_sha().clone(),
                        finding_key: finding.key().clone(),
                        summary: finding.summary().to_string(),
                    })
                    .map_err(|error| {
                        format!("failed to request PR validation remediation: {error}")
                    })?;
                next = next
                    .transition(PrValidationEvent::RemediationQueued(
                        PrValidationRemediationCorrelation::new(
                            finding.key().clone(),
                            PrValidationRecordKey::new(idempotency_key.as_str())
                                .map_err(|error| error.to_string())?,
                        ),
                    ))
                    .map_err(transition_error)?;
                requested_remediation = Some(idempotency_key);
                break;
            }
        }

        let starting_post_merge = next.phase() == PrValidationPhase::PreMergeObservation
            && snapshot.merge_state == GithubPrMergeState::Merged;
        if starting_post_merge {
            let merge_sha = snapshot
                .merge_sha
                .as_ref()
                .ok_or_else(|| "merged PR snapshot omitted its merge SHA".to_string())?;
            next = next
                .transition(PrValidationEvent::MergeObserved(commit_sha(merge_sha)?))
                .and_then(|record| record.transition(PrValidationEvent::BeginPostMergeObservation))
                .map_err(transition_error)?;
        }

        next = next
            .transition(PrValidationEvent::ObservationCheckpointed {
                delivery_revision: request.delivery_revision,
                cursor: snapshot
                    .next_cursor
                    .as_ref()
                    .map(|cursor| cursor.as_str().to_string()),
                evidence_fingerprint: fingerprint.clone(),
            })
            .map_err(transition_error)?;

        let can_settle = requested_remediation.is_none()
            && !starting_post_merge
            && had_post_merge_checkpoint
            && prior_fingerprint.as_deref() == Some(fingerprint.as_str())
            && next.phase() == PrValidationPhase::PostMergeObservation
            && snapshot.is_terminally_complete();
        if can_settle {
            next = next
                .transition(PrValidationEvent::Settle(completion(&snapshot)?))
                .map_err(transition_error)?;
        }

        if next != current {
            self.persist_pr_validation_record(
                &request.workspace_dir,
                &request.pool_root,
                Some(&current),
                &next,
            )?;
        }

        if let Some(idempotency_key) = requested_remediation {
            Ok(PrValidationPollResult::RemediationRequested { idempotency_key })
        } else if can_settle {
            Ok(PrValidationPollResult::Settled)
        } else {
            Ok(PrValidationPollResult::Waiting)
        }
    }
}

fn validate_snapshot_identity(
    snapshot: &GithubPrValidationSnapshot,
    target: &GithubPullRequestTarget,
    target_sha: &GithubCommitSha,
) -> Result<(), String> {
    if &snapshot.target != target || &snapshot.target_sha != target_sha {
        return Err(
            "trusted PR validation snapshot did not match the requested target revision"
                .to_string(),
        );
    }
    if snapshot
        .check_runs
        .iter()
        .any(|run| &run.target_sha != target_sha)
        || snapshot
            .workflow_runs
            .iter()
            .any(|run| &run.target_sha != target_sha)
    {
        return Err(
            "trusted PR validation snapshot contained evidence for another revision".to_string(),
        );
    }
    Ok(())
}

fn actionable_findings(
    snapshot: &GithubPrValidationSnapshot,
) -> Result<Vec<PrValidationFinding>, String> {
    let target_sha = commit_sha(&snapshot.target_sha)?;
    let mut findings = Vec::new();
    for run in &snapshot.check_runs {
        if is_actionable_failure(&run.status) {
            findings.push(PrValidationFinding::new(
                PrValidationFindingKey::new(
                    PrValidationFindingSource::new("check_run")?,
                    run.id.as_str(),
                )?,
                target_sha.clone(),
                format!("required check `{}` ended with {:?}", run.name, run.status),
            )?);
        }
    }
    for run in &snapshot.workflow_runs {
        if is_actionable_failure(&run.status) {
            findings.push(PrValidationFinding::new(
                PrValidationFindingKey::new(
                    PrValidationFindingSource::new("workflow_run")?,
                    run.id.as_str(),
                )?,
                target_sha.clone(),
                format!("workflow `{}` ended with {:?}", run.name, run.status),
            )?);
        }
    }
    Ok(findings)
}

fn is_actionable_failure(status: &GithubValidationRunStatus) -> bool {
    matches!(
        status,
        GithubValidationRunStatus::Failed | GithubValidationRunStatus::Cancelled
    )
}

fn remediation_key(
    record_key: &PrValidationRecordKey,
    finding: &PrValidationFinding,
) -> PrValidationRemediationKey {
    let mut digest = Sha256::new();
    digest.update(record_key.as_str());
    digest.update([0]);
    digest.update(finding.target_sha().as_str());
    digest.update([0]);
    digest.update(finding.key().source().as_str());
    digest.update([0]);
    digest.update(finding.key().provider_event_id());
    PrValidationRemediationKey::new(format!("pr-remediation-{:x}", digest.finalize()))
        .expect("SHA-256 remediation identity is non-empty")
}

fn snapshot_fingerprint(snapshot: &GithubPrValidationSnapshot) -> String {
    let mut digest = Sha256::new();
    digest.update(snapshot.target_sha.as_str());
    digest.update(format!("{:?}", snapshot.merge_state));
    digest.update(
        snapshot
            .merge_sha
            .as_ref()
            .map(GithubCommitSha::as_str)
            .unwrap_or_default(),
    );
    for activity in snapshot.activities.iter().filter(|activity| {
        activity
            .commit_sha
            .as_ref()
            .is_none_or(|sha| sha == &snapshot.target_sha)
    }) {
        digest.update(format!(
            "activity:{:?}:{}:{}:{:?}",
            activity.kind,
            activity.id.as_str(),
            activity.observed_at,
            activity.commit_sha.as_ref().map(GithubCommitSha::as_str)
        ));
    }
    for run in &snapshot.check_runs {
        digest.update(format!(
            "check:{}:{}:{:?}",
            run.id.as_str(),
            run.name,
            run.status
        ));
    }
    for run in &snapshot.workflow_runs {
        digest.update(format!(
            "workflow:{}:{}:{:?}",
            run.id.as_str(),
            run.name,
            run.status
        ));
    }
    for source in &snapshot.sources {
        digest.update(format!(
            "source:{:?}:{:?}:{:?}",
            source.source,
            source.status,
            source.next_cursor.as_ref().map(|cursor| cursor.as_str())
        ));
    }
    digest.update(format!(
        "cursor:{:?}",
        snapshot.next_cursor.as_ref().map(|cursor| cursor.as_str())
    ));
    format!("{:x}", digest.finalize())
}

fn completion(snapshot: &GithubPrValidationSnapshot) -> Result<PrValidationCompletion, String> {
    let merge_sha = snapshot
        .merge_sha
        .as_ref()
        .ok_or_else(|| "post-merge completion requires a merge SHA".to_string())?;
    let providers = snapshot
        .sources
        .iter()
        .map(|source| {
            let provider = PrValidationProviderKey::new(format!("github:{:?}", source.source))?;
            Ok(match source.status {
                GithubValidationSourceStatus::Complete if source.next_cursor.is_none() => {
                    PrValidationProviderCompletion::terminal(provider)
                }
                _ => PrValidationProviderCompletion::pending(provider),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut checks = snapshot
        .check_runs
        .iter()
        .map(|run| {
            PrValidationRequiredCheck::new(
                PrValidationCheckKind::CheckRun,
                &run.name,
                run.status.is_terminal(),
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    checks.extend(
        snapshot
            .workflow_runs
            .iter()
            .map(|run| {
                PrValidationRequiredCheck::new(
                    PrValidationCheckKind::WorkflowRun,
                    &run.name,
                    run.status.is_terminal(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?,
    );
    Ok(PrValidationCompletion::new(
        commit_sha(merge_sha)?,
        providers,
        checks,
        PrValidationCatchUpState::NoUnseenRelevantEvents,
    ))
}

fn commit_sha(sha: &GithubCommitSha) -> Result<PrValidationCommitSha, String> {
    PrValidationCommitSha::new(sha.as_str())
}

fn transition_error(error: impl std::fmt::Debug) -> String {
    format!("PR validation transition rejected: {error:?}")
}
