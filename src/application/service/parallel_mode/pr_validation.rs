use std::path::PathBuf;

#[cfg(test)]
use std::cell::RefCell;

use chrono::Utc;
use sha2::{Digest, Sha256};

use super::ParallelModeService;
use crate::application::port::outbound::github_pr_validation_port::{
    GithubExpectedCheckStatus, GithubPostMergeValidationDecision, GithubPrMergeState,
    GithubPrValidationObservationRequest, GithubPrValidationPort, GithubPrValidationSnapshot,
    GithubValidationSourceStatus,
};
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
};
use crate::application::port::outbound::pr_validation_remediation_port::{
    PrValidationRemediationKey, PrValidationRemediationPort, PrValidationRemediationRequest,
};
use crate::application::service::planning::{PlanningQueueUseCases, PlanningTaskCreateInput};
use crate::domain::github_review::{GithubCommitSha, GithubPullRequestTarget};
use crate::domain::parallel_mode::{
    IntegrationAttestation, IntegrationMethod, PrValidationCatchUpState, PrValidationCheckKind,
    PrValidationCommitSha, PrValidationCompletion, PrValidationEvent, PrValidationFinding,
    PrValidationFindingKey, PrValidationFindingSource, PrValidationPhase,
    PrValidationProviderCompletion, PrValidationProviderKey, PrValidationRecord,
    PrValidationRecordKey, PrValidationRemediationCorrelation, PrValidationRequiredCheck,
    PrValidationTarget, PrValidationTargetShaSnapshot, PrValidationTerminalReason,
    PrValidationTransitionRejection,
};
use crate::domain::planning::TaskStatus;

#[cfg(test)]
thread_local! {
    static BEFORE_DISTRIBUTOR_ATTESTATION_PERSIST_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_before_distributor_attestation_persist_hook(
    hook: impl FnOnce() + 'static,
) {
    BEFORE_DISTRIBUTOR_ATTESTATION_PERSIST_HOOK.with(|slot| {
        let previous = slot.borrow_mut().replace(Box::new(hook));
        assert!(
            previous.is_none(),
            "distributor attestation test hook already installed"
        );
    });
}

#[cfg(test)]
fn run_before_distributor_attestation_persist_hook() {
    BEFORE_DISTRIBUTOR_ATTESTATION_PERSIST_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_distributor_attestation_persist_hook() {}

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
    Blocked,
    Failed,
    StaleDeliveryIgnored,
}

pub struct PlanningQueuePrValidationRemediationPort<'a> {
    queue: &'a PlanningQueueUseCases,
    workspace_dir: &'a str,
}

impl<'a> PlanningQueuePrValidationRemediationPort<'a> {
    pub fn new(queue: &'a PlanningQueueUseCases, workspace_dir: &'a str) -> Self {
        Self {
            queue,
            workspace_dir,
        }
    }
}

impl PrValidationRemediationPort for PlanningQueuePrValidationRemediationPort<'_> {
    fn request_remediation(
        &self,
        request: &PrValidationRemediationRequest,
    ) -> anyhow::Result<PrValidationRecordKey> {
        let admission = self.queue.admit_system_task_once(
            self.workspace_dir,
            request.idempotency_key.as_str(),
            PlanningTaskCreateInput {
                direction_id: None,
                direction_relation_note: Some(format!(
                    "Remediates validation record {} for {}#{}",
                    request.validation_record_key.as_str(),
                    request.target.repository(),
                    request.target.pull_request_number()
                )),
                title: format!(
                    "Remediate PR #{} validation finding",
                    request.target.pull_request_number()
                ),
                description: Some(format!(
                    "Validation record: {}\nTarget SHA: {}\nFinding: {}",
                    request.validation_record_key.as_str(),
                    request.target_sha.as_str(),
                    request.summary
                )),
                status: Some(TaskStatus::Ready),
                base_priority: None,
                dynamic_priority_delta: None,
                priority_reason: None,
                depends_on: Vec::new(),
                blocked_by: Vec::new(),
            },
        )?;
        PrValidationRecordKey::new(admission.task_id).map_err(anyhow::Error::msg)
    }
}

pub(super) fn register_distributor_pr_validation_with_ports(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &std::path::Path,
    record: &PlanningAuthorityDistributorQueueRecord,
) -> Result<PrValidationRecordKey, String> {
    let target = record.delivery_target.as_ref().ok_or_else(|| {
        "cannot register PR validation without an immutable delivery target".to_string()
    })?;
    let pull_request_number = record.pull_request_number.ok_or_else(|| {
        "cannot register PR validation before a pull request is ensured".to_string()
    })?;
    let key = PrValidationRecordKey::new(&record.queue_item_id)?;
    let registered = PrValidationRecord::register(
        key.clone(),
        PrValidationTarget::new(&target.github_repository, pull_request_number)?,
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new(record.effective_source_commit_sha())?,
            PrValidationCommitSha::new(&record.source_base_commit_sha)?,
        ),
    );
    match super::pr_validation_store::recover_pr_validation_record_mirror(
        planning_authority,
        runtime,
        workspace_dir,
        pool_root,
        &key,
    )? {
        Some(existing) if existing == registered => Ok(key),
        Some(existing)
            if existing.target() == registered.target()
                && existing.target_shas() == registered.target_shas() =>
        {
            Ok(key)
        }
        Some(_) => Err(format!(
            "PR validation record `{}` is already registered with different immutable identity",
            key.as_str()
        )),
        None => {
            super::pr_validation_store::persist_pr_validation_record(
                planning_authority,
                runtime,
                workspace_dir,
                pool_root,
                None,
                &registered,
            )?;
            Ok(key)
        }
    }
}

pub(super) fn attest_distributor_pr_validation_with_ports(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &std::path::Path,
    record: &PlanningAuthorityDistributorQueueRecord,
) -> Result<(), String> {
    let key = PrValidationRecordKey::new(&record.queue_item_id)?;
    let mut current = super::pr_validation_store::recover_pr_validation_record_mirror(
        planning_authority,
        runtime,
        workspace_dir,
        pool_root,
        &key,
    )?;
    if current.is_none() {
        // Explicit direct-delivery compatibility has no PR identity to validate and must not
        // fabricate one. A preserved/ensured PR does have enough immutable identity, so recover
        // a missing registration before recording the verified distributor evidence.
        if record.pull_request_number.is_none() {
            return Ok(());
        }
        register_distributor_pr_validation_with_ports(
            planning_authority,
            runtime,
            workspace_dir,
            pool_root,
            record,
        )?;
        current = super::pr_validation_store::recover_pr_validation_record_mirror(
            planning_authority,
            runtime,
            workspace_dir,
            pool_root,
            &key,
        )?;
    }
    let mut current = current.ok_or_else(|| {
        format!(
            "PR validation record `{}` is missing after integration registration",
            key.as_str()
        )
    })?;
    let integration_base_sha = record
        .integration_base_commit_sha
        .as_deref()
        .ok_or_else(|| {
            "cannot attest distributor integration without its frozen base SHA".to_string()
        })?;
    let evidence_sha = record.integration_commit_sha.as_deref().ok_or_else(|| {
        "cannot attest distributor integration without its verified evidence SHA".to_string()
    })?;
    let observed_at = Utc::now();
    let attestation = IntegrationAttestation::new(
        IntegrationMethod::DistributorCherryPick,
        PrValidationCommitSha::new(record.effective_source_commit_sha())?,
        Some(PrValidationCommitSha::new(integration_base_sha)?),
        PrValidationCommitSha::new(evidence_sha)?,
        record.pull_request_number,
        None,
        observed_at,
        observed_at,
    )?;
    const MAX_ATTESTATION_CAS_ATTEMPTS: usize = 4;
    for attempt in 1..=MAX_ATTESTATION_CAS_ATTEMPTS {
        let next =
            match current.transition(PrValidationEvent::IntegrationAttested(attestation.clone())) {
                Ok(next) => next,
                Err(PrValidationTransitionRejection::IntegrationAuthorityConflict { .. }) => {
                    let failed = current
                        .transition(PrValidationEvent::Fail(
                            PrValidationTerminalReason::IntegrationAuthorityConflict,
                        ))
                        .map_err(transition_error)?;
                    super::pr_validation_store::persist_pr_validation_record(
                        planning_authority,
                        runtime,
                        workspace_dir,
                        pool_root,
                        Some(&current),
                        &failed,
                    )?;
                    return Err(
                        "distributor integration conflicts with persisted validation authority"
                            .to_string(),
                    );
                }
                Err(error) => return Err(transition_error(error)),
            };
        if next == current {
            return Ok(());
        }
        run_before_distributor_attestation_persist_hook();
        let persistence_error = match super::pr_validation_store::persist_pr_validation_record(
            planning_authority,
            runtime,
            workspace_dir,
            pool_root,
            Some(&current),
            &next,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        let latest = super::pr_validation_store::recover_pr_validation_record_mirror(
            planning_authority,
            runtime,
            workspace_dir,
            pool_root,
            &key,
        )?
        .ok_or_else(|| {
            format!(
                "PR validation record `{}` disappeared during integration attestation",
                key.as_str()
            )
        })?;
        if latest == next {
            return Ok(());
        }
        if latest == current {
            return Err(persistence_error);
        }
        if attempt == MAX_ATTESTATION_CAS_ATTEMPTS {
            return Err(format!(
                "PR validation record `{}` kept changing during integration attestation after {MAX_ATTESTATION_CAS_ATTEMPTS} attempts: {persistence_error}",
                key.as_str()
            ));
        }
        current = latest;
    }
    unreachable!("bounded distributor attestation loop always returns")
}

pub(super) fn transition_pr_validation_remediation_with_ports(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    workspace_dir: &str,
    pool_root: &std::path::Path,
    task_id: &str,
    complete: bool,
) -> Result<bool, String> {
    let Some(current) = planning_authority
        .load_runtime_pr_validation_record_for_remediation(workspace_dir, task_id)
        .map_err(|error| error.to_string())?
    else {
        return Ok(false);
    };
    let correlation = current
        .remediation_for_task(task_id)
        .cloned()
        .expect("persistence lookup returned the matching remediation");
    let next = match (current.phase(), complete) {
        (PrValidationPhase::RemediationQueued, false) => current
            .transition(PrValidationEvent::RemediationStarted {
                finding_key: correlation.finding_key().clone(),
            })
            .map_err(transition_error)?,
        (PrValidationPhase::RemediationQueued, true) => current
            .transition(PrValidationEvent::RemediationStarted {
                finding_key: correlation.finding_key().clone(),
            })
            .and_then(|record| {
                record.transition(PrValidationEvent::RemediationCompleted {
                    finding_key: correlation.finding_key().clone(),
                })
            })
            .map_err(transition_error)?,
        (PrValidationPhase::RemediationRunning, true) => current
            .transition(PrValidationEvent::RemediationCompleted {
                finding_key: correlation.finding_key().clone(),
            })
            .map_err(transition_error)?,
        (PrValidationPhase::RemediationRunning, false) => return Ok(false),
        _ => return Ok(false),
    };
    super::pr_validation_store::persist_pr_validation_record(
        planning_authority,
        runtime,
        workspace_dir,
        pool_root,
        Some(&current),
        &next,
    )?;
    Ok(true)
}

impl ParallelModeService {
    pub fn register_distributor_pr_validation(
        &self,
        workspace_dir: &str,
        pool_root: &std::path::Path,
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<PrValidationRecordKey, String> {
        register_distributor_pr_validation_with_ports(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            pool_root,
            record,
        )
    }

    /// Activates every durable, pollable validation record for one existing runtime tick.
    /// Revisions derive from persisted authority, so restarts and repeated ticks remain monotonic.
    pub fn poll_pr_validations_for_runtime_tick(
        &self,
        workspace_dir: &str,
        queue: &PlanningQueueUseCases,
    ) -> Result<Vec<PrValidationPollResult>, String> {
        let Some(observation) = self.pr_validation_observation.as_ref() else {
            return Ok(Vec::new());
        };
        let repo_root = self
            .parallel_runtime
            .detect_git_repo_root(workspace_dir)
            .ok_or_else(|| "git repository is unavailable for PR validation polling".to_string())?;
        let pool_root = super::derive_default_pool_root(std::path::Path::new(&repo_root));
        let records = self
            .planning_authority
            .load_runtime_pr_validation_records(workspace_dir)
            .map_err(|error| format!("failed to load runtime PR validation records: {error}"))?;
        let mut results = Vec::new();
        for record in records.into_iter().filter(|record| {
            matches!(
                record.phase(),
                PrValidationPhase::Registered
                    | PrValidationPhase::PreMergeObservation
                    | PrValidationPhase::PostMergeObservation
            )
        }) {
            let delivery_revision =
                record
                    .observation_revision()
                    .checked_add(1)
                    .ok_or_else(|| {
                        format!(
                            "PR validation record `{}` exhausted delivery revisions",
                            record.key().as_str()
                        )
                    })?;
            results.push(self.poll_pr_validation_into_normal_queue(
                observation.as_ref(),
                queue,
                PrValidationPollRequest {
                    workspace_dir: workspace_dir.to_string(),
                    pool_root: pool_root.clone(),
                    record_key: record.key().clone(),
                    target_shas: record.target_shas().clone(),
                    delivery_revision,
                },
            )?);
        }
        Ok(results)
    }

    /// Executes one poll/trigger delivery. Waiting is represented by returning `Waiting`; this
    /// service never acquires a pool mutation lock, worktree, or slot lease between deliveries.
    pub fn poll_pr_validation_into_normal_queue(
        &self,
        observation: &dyn GithubPrValidationPort,
        queue: &PlanningQueueUseCases,
        request: PrValidationPollRequest,
    ) -> Result<PrValidationPollResult, String> {
        let workspace_dir = request.workspace_dir.clone();
        let remediation = PlanningQueuePrValidationRemediationPort::new(queue, &workspace_dir);
        self.poll_pr_validation(observation, &remediation, request)
    }

    pub fn transition_pr_validation_remediation_started(
        &self,
        workspace_dir: &str,
        pool_root: &std::path::Path,
        task_id: &str,
    ) -> Result<bool, String> {
        self.transition_pr_validation_remediation(workspace_dir, pool_root, task_id, false)
    }

    pub fn transition_pr_validation_remediation_completed(
        &self,
        workspace_dir: &str,
        pool_root: &std::path::Path,
        task_id: &str,
    ) -> Result<bool, String> {
        self.transition_pr_validation_remediation(workspace_dir, pool_root, task_id, true)
    }

    fn transition_pr_validation_remediation(
        &self,
        workspace_dir: &str,
        pool_root: &std::path::Path,
        task_id: &str,
        complete: bool,
    ) -> Result<bool, String> {
        transition_pr_validation_remediation_with_ports(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            pool_root,
            task_id,
            complete,
        )
    }

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
        match current.phase() {
            PrValidationPhase::Settled => return Ok(PrValidationPollResult::Settled),
            PrValidationPhase::Blocked => return Ok(PrValidationPollResult::Blocked),
            PrValidationPhase::Failed => return Ok(PrValidationPollResult::Failed),
            _ => {}
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
        let observation_request = GithubPrValidationObservationRequest::new(
            target.clone(),
            target_sha.clone(),
            next.observation_cursor().map(
                crate::application::port::outbound::github_pr_validation_port::GithubValidationCursor::new,
            ),
        )
        .with_evidence_sha(
            next.evidence_sha()
                .map(|sha| GithubCommitSha::new(sha.as_str())),
        );
        let snapshot = match observation.load_validation_snapshot(&observation_request) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                next = next
                    .transition(PrValidationEvent::Fail(
                        PrValidationTerminalReason::ObservationFailed,
                    ))
                    .map_err(transition_error)?;
                self.persist_pr_validation_record(
                    &request.workspace_dir,
                    &request.pool_root,
                    Some(&current),
                    &next,
                )?;
                return Ok(PrValidationPollResult::Failed);
            }
        };
        if validate_snapshot_identity(&snapshot, &target, &target_sha, next.evidence_sha()).is_err()
        {
            next = next
                .transition(PrValidationEvent::Fail(
                    PrValidationTerminalReason::ObservationFailed,
                ))
                .map_err(transition_error)?;
            self.persist_pr_validation_record(
                &request.workspace_dir,
                &request.pool_root,
                Some(&current),
                &next,
            )?;
            return Ok(PrValidationPollResult::Failed);
        }
        if snapshot.merge_state == GithubPrMergeState::Closed && next.evidence_sha().is_none() {
            next = next
                .transition(PrValidationEvent::Block(
                    PrValidationTerminalReason::IntegrationEvidenceMissing,
                ))
                .map_err(transition_error)?;
            self.persist_pr_validation_record(
                &request.workspace_dir,
                &request.pool_root,
                Some(&current),
                &next,
            )?;
            return Ok(PrValidationPollResult::Blocked);
        }

        let was_post_merge = next.phase() == PrValidationPhase::PostMergeObservation;
        if snapshot.merge_state == GithubPrMergeState::Merged {
            let merge_sha = snapshot
                .merge_sha
                .as_ref()
                .ok_or_else(|| "merged PR snapshot omitted its merge SHA".to_string())?;
            let observed_at = Utc::now();
            let attestation = IntegrationAttestation::new(
                IntegrationMethod::GithubRebaseMerge,
                next.target_shas().source_sha().clone(),
                Some(next.target_shas().base_sha().clone()),
                commit_sha(merge_sha)?,
                Some(next.target().pull_request_number()),
                Some(commit_sha(merge_sha)?),
                observed_at,
                observed_at,
            )?;
            next = match next.transition(PrValidationEvent::IntegrationAttested(attestation)) {
                Ok(next) => next,
                Err(PrValidationTransitionRejection::IntegrationAuthorityConflict { .. }) => {
                    let failed = next
                        .transition(PrValidationEvent::Fail(
                            PrValidationTerminalReason::IntegrationAuthorityConflict,
                        ))
                        .map_err(transition_error)?;
                    self.persist_pr_validation_record(
                        &request.workspace_dir,
                        &request.pool_root,
                        Some(&current),
                        &failed,
                    )?;
                    return Ok(PrValidationPollResult::Failed);
                }
                Err(error) => return Err(transition_error(error)),
            };
        }
        let starting_post_merge =
            !was_post_merge && next.phase() == PrValidationPhase::PostMergeObservation;

        let fingerprint = snapshot_fingerprint(&snapshot);
        let contract_decision =
            snapshot.evaluate_post_merge_contract(next.post_merge_validation_contract());
        let prior_fingerprint = next.evidence_fingerprint().map(str::to_string);
        let had_post_merge_checkpoint = next.has_post_merge_checkpoint();
        let mut requested_remediation = None;
        let mut remediation_failed = false;

        if matches!(
            next.phase(),
            PrValidationPhase::PreMergeObservation | PrValidationPhase::PostMergeObservation
        ) {
            for finding in actionable_findings(&snapshot, &contract_decision)? {
                if next.finding_keys().contains(finding.key()) {
                    continue;
                }
                next = next
                    .transition(PrValidationEvent::FindingObserved(finding.clone()))
                    .map_err(transition_error)?;
                let idempotency_key = remediation_key(next.key(), &finding);
                let remediation_task_key =
                    remediation.request_remediation(&PrValidationRemediationRequest {
                        idempotency_key: idempotency_key.clone(),
                        validation_record_key: next.key().clone(),
                        target: next.target().clone(),
                        target_sha: finding.target_sha().clone(),
                        finding_key: finding.key().clone(),
                        summary: finding.summary().to_string(),
                    });
                match remediation_task_key {
                    Ok(remediation_task_key) => {
                        next = next
                            .transition(PrValidationEvent::RemediationQueued(
                                PrValidationRemediationCorrelation::new(
                                    finding.key().clone(),
                                    remediation_task_key,
                                ),
                            ))
                            .map_err(transition_error)?;
                        requested_remediation = Some(idempotency_key);
                    }
                    Err(_) => {
                        next = next
                            .transition(PrValidationEvent::Fail(
                                PrValidationTerminalReason::RemediationAdmissionFailed,
                            ))
                            .map_err(transition_error)?;
                        remediation_failed = true;
                    }
                }
                break;
            }
        }

        if !remediation_failed {
            let checkpoint_cursor = snapshot.next_cursor.as_ref().or_else(|| {
                (!starting_post_merge)
                    .then_some(observation_request.cursor.as_ref())
                    .flatten()
            });
            next = next
                .transition(PrValidationEvent::ObservationCheckpointed {
                    delivery_revision: request.delivery_revision,
                    cursor: checkpoint_cursor.map(|cursor| cursor.as_str().to_string()),
                    evidence_fingerprint: fingerprint.clone(),
                })
                .map_err(transition_error)?;
        }

        let settlement_evidence_sha = next.evidence_sha().cloned();
        let successful_settlement_evidence = settlement_evidence_sha.as_ref().is_some_and(|sha| {
            snapshot.is_successfully_complete(
                next.post_merge_validation_contract(),
                &GithubCommitSha::new(sha.as_str()),
            )
        });
        let can_finalize_contract = requested_remediation.is_none()
            && !remediation_failed
            && !starting_post_merge
            && had_post_merge_checkpoint
            && prior_fingerprint.as_deref() == Some(fingerprint.as_str())
            && next.phase() == PrValidationPhase::PostMergeObservation;
        let missing_context_is_final = !contract_decision.has_missing_required()
            || (!snapshot.workflow_runs.is_empty()
                && snapshot
                    .workflow_runs
                    .iter()
                    .all(|run| run.status.is_terminal()));
        let contract_blocked = can_finalize_contract
            && (contract_decision.has_explicit_policy_blocker()
                || (contract_decision.has_missing_required() && missing_context_is_final));
        if contract_blocked {
            next = next
                .transition(PrValidationEvent::Block(
                    PrValidationTerminalReason::PostMergeContractBlocked,
                ))
                .map_err(transition_error)?;
        }
        let can_settle =
            can_finalize_contract && !contract_blocked && successful_settlement_evidence;
        if can_settle {
            let evidence_sha = settlement_evidence_sha
                .as_ref()
                .expect("successful settlement evidence was checked above");
            next = next
                .transition(PrValidationEvent::Settle(completion(
                    &snapshot,
                    evidence_sha,
                    &contract_decision,
                )?))
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

        if remediation_failed {
            Ok(PrValidationPollResult::Failed)
        } else if let Some(idempotency_key) = requested_remediation {
            Ok(PrValidationPollResult::RemediationRequested { idempotency_key })
        } else if contract_blocked {
            Ok(PrValidationPollResult::Blocked)
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
    attested_evidence_sha: Option<&PrValidationCommitSha>,
) -> Result<(), String> {
    if &snapshot.target != target || &snapshot.target_sha != target_sha {
        return Err(
            "trusted PR validation snapshot did not match the requested target revision"
                .to_string(),
        );
    }
    let attested_evidence_sha = attested_evidence_sha.map(|sha| GithubCommitSha::new(sha.as_str()));
    let expected_evidence_sha = match snapshot.merge_state {
        GithubPrMergeState::Merged => {
            let merge_sha = snapshot
                .merge_sha
                .as_ref()
                .ok_or_else(|| "merged PR snapshot omitted its merge SHA".to_string())?;
            if attested_evidence_sha
                .as_ref()
                .is_some_and(|attested| attested != merge_sha)
            {
                return Err(
                    "GitHub merge SHA conflicts with attested integration evidence".to_string(),
                );
            }
            merge_sha
        }
        GithubPrMergeState::Open | GithubPrMergeState::Closed => {
            attested_evidence_sha.as_ref().unwrap_or(target_sha)
        }
        GithubPrMergeState::Unknown(_) => {
            return Err("trusted PR validation snapshot had an unknown merge state".to_string());
        }
    };
    if &snapshot.evidence_sha != expected_evidence_sha
        || snapshot
            .check_runs
            .iter()
            .any(|run| &run.target_sha != expected_evidence_sha)
        || snapshot
            .workflow_runs
            .iter()
            .any(|run| &run.target_sha != expected_evidence_sha)
    {
        return Err(
            "trusted PR validation snapshot contained evidence for another revision".to_string(),
        );
    }
    Ok(())
}

fn actionable_findings(
    snapshot: &GithubPrValidationSnapshot,
    contract_decision: &GithubPostMergeValidationDecision,
) -> Result<Vec<PrValidationFinding>, String> {
    let target_sha = commit_sha(&snapshot.target_sha)?;
    let mut findings = Vec::new();
    for activity in &snapshot.activities {
        if activity
            .commit_sha
            .as_ref()
            .is_some_and(|sha| sha != &snapshot.target_sha)
        {
            continue;
        }
        let (source, summary) = match activity.kind {
            crate::application::port::outbound::github_pr_validation_port::GithubValidationActivityKind::Review => {
                ("review", "pull request review requires remediation")
            }
            crate::application::port::outbound::github_pr_validation_port::GithubValidationActivityKind::IssueComment => {
                ("issue_comment", "pull request issue comment requires remediation")
            }
            crate::application::port::outbound::github_pr_validation_port::GithubValidationActivityKind::ReviewThread => {
                ("review_thread", "pull request review thread requires remediation")
            }
            crate::application::port::outbound::github_pr_validation_port::GithubValidationActivityKind::ReviewComment => {
                ("review_comment", "pull request review comment requires remediation")
            }
        };
        findings.push(PrValidationFinding::new(
            PrValidationFindingKey::new(
                PrValidationFindingSource::new(source)?,
                activity.id.as_str(),
            )?,
            target_sha.clone(),
            summary,
        )?);
    }
    for evaluation in contract_decision.actionable_failures() {
        let Some(run) = evaluation.selected_run.as_ref() else {
            continue;
        };
        findings.push(PrValidationFinding::new(
            PrValidationFindingKey::new(
                PrValidationFindingSource::new("check_run")?,
                sanitized_provider_id(run.id.as_str()),
            )?,
            target_sha.clone(),
            format!(
                "required check `{}` ended with {:?}",
                check_context_label(&evaluation.context),
                run.status
            ),
        )?);
    }
    Ok(findings)
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
    digest.update(snapshot.evidence_sha.as_str());
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
            "check:{}:{}:{:?}:{:?}:{:?}:{:?}:{:?}",
            run.id.as_str(),
            run.name,
            run.app_slug,
            run.check_suite_id.as_ref().map(|id| id.as_str()),
            run.started_at,
            run.completed_at,
            run.status
        ));
    }
    for run in &snapshot.workflow_runs {
        digest.update(format!(
            "workflow:{}:{}:{}:{:?}:{:?}:{:?}:{:?}:{:?}",
            run.id.as_str(),
            run.name,
            run.run_attempt,
            run.check_suite_id.as_ref().map(|id| id.as_str()),
            run.run_started_at,
            run.created_at,
            run.updated_at,
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

fn completion(
    snapshot: &GithubPrValidationSnapshot,
    evidence_sha: &PrValidationCommitSha,
    contract_decision: &GithubPostMergeValidationDecision,
) -> Result<PrValidationCompletion, String> {
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
    let checks = contract_decision
        .required
        .iter()
        .map(|evaluation| {
            PrValidationRequiredCheck::new(
                PrValidationCheckKind::CheckRun,
                check_context_label(&evaluation.context),
                evaluation.status == GithubExpectedCheckStatus::Succeeded,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(PrValidationCompletion::new(
        evidence_sha.clone(),
        providers,
        checks,
        PrValidationCatchUpState::NoUnseenRelevantEvents,
    ))
}

fn check_context_label(context: &crate::domain::parallel_mode::PrValidationCheckContext) -> String {
    context
        .app_slug()
        .map(|app| format!("{app}/{}", context.context()))
        .unwrap_or_else(|| context.context().to_string())
}

fn sanitized_provider_id(value: &str) -> String {
    const MAX_ID_LEN: usize = 96;
    const DIGEST_LEN: usize = 16;

    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if !sanitized.is_empty() && sanitized == value && sanitized.chars().count() <= MAX_ID_LEN {
        return sanitized;
    }

    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    let prefix_len = MAX_ID_LEN - DIGEST_LEN - 1;
    let prefix = sanitized.chars().take(prefix_len).collect::<String>();
    format!("{prefix}-{}", &digest[..DIGEST_LEN])
}

fn commit_sha(sha: &GithubCommitSha) -> Result<PrValidationCommitSha, String> {
    PrValidationCommitSha::new(sha.as_str())
}

fn transition_error(error: impl std::fmt::Debug) -> String {
    format!("PR validation transition rejected: {error:?}")
}
