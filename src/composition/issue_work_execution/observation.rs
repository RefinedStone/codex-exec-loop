//! Akra의 영속 사실만 중립적인 이슈 실행 관찰로 옮긴다.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, bail};
use sha2::{Digest, Sha256};

use super::AkraIssueWorkExecutionAdapter;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderRecord;
use crate::domain::issue_automation::execution::{
    IssueExecutionIntegration, IssueExecutionPhase, IssueExecutionQuestion,
    IssueExecutionTerminalClass, IssueExecutionTerminalRecord, IssueWorkExecutionObservation,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeQueueItemState,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState, PrValidationFinding,
    PrValidationPhase, PrValidationRecord, PrValidationRecordKey, PrValidationTerminalReason,
};
use crate::domain::planning::{TaskAuthorityDocument, TaskDefinition, TaskStatus};

struct ValidationEvidence {
    source_task_id: String,
    record: PrValidationRecord,
    // Historical terminals remain receipt inputs, but a durable successor owns current state.
    superseded: bool,
    remediation_origin_key: Option<String>,
}

pub(super) struct ObservedExecution {
    pub(super) observation: IssueWorkExecutionObservation,
    pub(super) runtime: PlanningAuthorityRuntimeProjectionSnapshot,
    pub(super) active_task_status: Option<TaskStatus>,
}

pub(super) fn observe(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<IssueWorkExecutionObservation>> {
    Ok(load(adapter, workspace, work_order_id)?.map(|loaded| loaded.observation))
}

pub(super) fn load(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<ObservedExecution>> {
    let Some(order) = adapter
        .repository
        .load_work_order(workspace, work_order_id)?
    else {
        return Ok(None);
    };
    order.validate()?;
    let documents = adapter
        .authority
        .load_planning_authority_documents(workspace)?
        .ok_or_else(|| anyhow!("the planning execution documents are unavailable"))?;
    let runtime = adapter.authority.load_runtime_projections(workspace)?;
    let started = adapter
        .repository
        .started_work_order_task_ids(workspace, work_order_id)?;
    let mut records = Vec::new();
    let mut recovery_leaves = BTreeMap::new();
    // 계보 연결은 queue 보존 기간과 무관하다. PR 번호로 작업 소유권을 추측하지 않는다.
    for key in adapter
        .repository
        .work_order_validation_record_keys(workspace, work_order_id)?
    {
        let link = adapter
            .repository
            .load_work_order_validation_link(workspace, &key)?
            .ok_or_else(|| anyhow!("the work validation link disappeared"))?;
        if link.validation_record_key != key
            || link.work_order_id != work_order_id
            || !order.task_ids.contains(&link.source_task_id)
        {
            bail!("the work validation link belongs to a different lineage");
        }
        let key = PrValidationRecordKey::new(key).map_err(anyhow::Error::msg)?;
        if let Some(record) = adapter
            .authority
            .load_runtime_pr_validation_record(workspace, &key)?
        {
            if record.key() != &key {
                bail!("the validation record identity does not match its work link");
            }
            let remediation_origin_key = adapter
                .repository
                .load_work_order_validation_recovery_origin(workspace, key.as_str())?;
            let origin = PrValidationRecordKey::new(
                remediation_origin_key.as_deref().unwrap_or(key.as_str()),
            )
            .map_err(anyhow::Error::msg)?;
            let successor_key = cached_recovery_leaf(&mut recovery_leaves, &origin, |origin| {
                adapter
                    .authority
                    .resolve_runtime_pr_validation_successor_key(workspace, origin)
            })?;
            records.push(ValidationEvidence {
                source_task_id: link.source_task_id,
                record,
                superseded: successor_key != key,
                remediation_origin_key,
            });
        }
    }
    let observation = project(
        &order,
        &documents.task_authority,
        &runtime,
        &records,
        &started,
    )?;
    observation.validate().map_err(anyhow::Error::msg)?;
    let active_task_status = observation.active_task_id.as_ref().and_then(|id| {
        documents
            .task_authority
            .tasks
            .iter()
            .find(|task| &task.id == id)
            .map(|task| task.status)
    });
    Ok(Some(ObservedExecution {
        observation,
        runtime,
        active_task_status,
    }))
}

/// The cache belongs to exactly one load. New ticks must observe newly registered successors.
fn cached_recovery_leaf(
    cache: &mut BTreeMap<PrValidationRecordKey, PrValidationRecordKey>,
    origin: &PrValidationRecordKey,
    resolve: impl FnOnce(&PrValidationRecordKey) -> Result<PrValidationRecordKey>,
) -> Result<PrValidationRecordKey> {
    if let Some(current) = cache.get(origin) {
        return Ok(current.clone());
    }
    let current = resolve(origin)?;
    cache.insert(origin.clone(), current.clone());
    Ok(current)
}

fn project(
    order: &PlanningWorkOrderRecord,
    tasks: &TaskAuthorityDocument,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
    validations: &[ValidationEvidence],
    started: &[String],
) -> Result<IssueWorkExecutionObservation> {
    let owned = |id: &str| order.task_ids.iter().any(|task_id| task_id == id);
    if order
        .task_ids
        .iter()
        .any(|id| !tasks.tasks.iter().any(|task| &task.id == id))
        || validations
            .iter()
            .any(|evidence| !owned(&evidence.source_task_id))
    {
        bail!("the execution observation does not have a complete work lineage");
    }
    let running: BTreeSet<_> = runtime
        .slot_leases
        .values()
        .filter(|lease| owned(&lease.task_id) && worker_is_running(lease, runtime))
        .map(|lease| lease.task_id.as_str())
        .collect();
    let waiting: Vec<_> = tasks
        .tasks
        .iter()
        .filter(|task| owned(&task.id) && task.status == TaskStatus::AwaitingUser)
        .collect();
    let active_task_id = active_task_id(order, tasks, &running, &waiting);
    let active_status = active_task_id.as_ref().and_then(|id| {
        tasks
            .tasks
            .iter()
            .find(|task| &task.id == id)
            .map(|task| task.status)
    });
    let active_record = active_validation(validations, active_task_id.as_deref());
    let mut terminals = Vec::new();
    for evidence in validations {
        if let Some(terminal) = validation_terminal(&evidence.record) {
            terminals.push(terminal);
        }
        // 이미 큐에 들어갔거나 끝난 보완도 같은 지적의 영속 기록이다.
        for key in evidence.record.finding_keys() {
            let Some(finding) = evidence.record.finding(&key) else {
                continue;
            };
            let belongs = evidence
                .record
                .remediation_for(&key)
                .is_none_or(|correlation| owned(correlation.remediation_key().as_str()));
            if belongs {
                terminals.push(rework_terminal(
                    &evidence.record,
                    finding,
                    evidence.remediation_origin_key.as_deref(),
                ));
            }
        }
    }
    let mut current_terminal = active_record.and_then(|evidence| {
        current_validation_terminal(
            &evidence.record,
            active_task_id.as_deref(),
            evidence.remediation_origin_key.as_deref(),
        )
    });
    let integration =
        integration_evidence(validations, &runtime.distributor_queue_records, &owned)?;
    let question = match waiting.as_slice() {
        [task]
            if integration.is_none()
                && active_task_id.as_deref() == Some(task.id.as_str())
                && current_terminal.as_ref().is_none_or(|terminal| {
                    terminal.classification == IssueExecutionTerminalClass::ReworkNeeded
                }) =>
        {
            question_target(task, runtime)
        }
        _ => None,
    };
    let active_has_pr = active_record.is_some_and(|evidence| {
        active_task_id.as_deref() == Some(evidence.source_task_id.as_str())
    }) || runtime.distributor_queue_records.iter().any(|record| {
        Some(record.task_id.as_str()) == active_task_id.as_deref()
            && record.pull_request_number.is_some()
    });
    let has_pr = runtime
        .distributor_queue_records
        .iter()
        .any(|record| owned(&record.task_id) && record.pull_request_number.is_some())
        || !validations.is_empty();
    let phase = if integration.is_some() {
        IssueExecutionPhase::Integrated
    } else if current_terminal.as_ref().is_some_and(|terminal| {
        terminal.classification != IssueExecutionTerminalClass::ReworkNeeded
    }) {
        IssueExecutionPhase::Terminal
    } else if !running.is_empty() || !waiting.is_empty() {
        // 보완 실행 중인 이전 지적은 현재 종료 원인이 아니다. 기록은 그대로 보존한다.
        current_terminal = None;
        IssueExecutionPhase::Running
    } else if !active_has_pr
        && matches!(
            active_status,
            Some(TaskStatus::Ready | TaskStatus::Proposed)
        )
    {
        IssueExecutionPhase::Queued
    } else if current_terminal.is_some() {
        IssueExecutionPhase::Terminal
    } else if has_pr {
        IssueExecutionPhase::InReview
    } else {
        IssueExecutionPhase::Queued
    };
    terminals.sort_by(|left, right| left.record_id.cmp(&right.record_id));
    terminals.dedup();
    Ok(IssueWorkExecutionObservation {
        phase,
        task_ids: order.task_ids.clone(),
        active_task_id,
        execution_ids: execution_ids(runtime, started, &owned),
        terminals,
        current_terminal_id: current_terminal.map(|record| record.record_id),
        integration,
        question,
    })
}

fn active_task_id(
    order: &PlanningWorkOrderRecord,
    tasks: &TaskAuthorityDocument,
    running: &BTreeSet<&str>,
    waiting: &[&TaskDefinition],
) -> Option<String> {
    if running.len() == 1 {
        return running.first().map(|id| (*id).to_owned());
    }
    if !running.is_empty() {
        return None;
    }
    if let [task] = waiting {
        return Some(task.id.clone());
    }
    if !waiting.is_empty() {
        return None;
    }
    order
        .task_ids
        .iter()
        .rev()
        .find(|id| {
            tasks
                .tasks
                .iter()
                .any(|task| &task.id == *id && !task.status.is_terminal())
        })
        .cloned()
        .or_else(|| order.task_ids.last().cloned())
}

fn session_matches_lease(
    detail: &ParallelModeAgentSessionDetailSnapshot,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> bool {
    detail.task_id == lease.task_id
        && detail.session_key == lease.session_key()
        && detail.slot_id == lease.slot_id
        && detail.branch_name == lease.branch_name
        && detail.worktree_path == lease.worktree_path
        && detail.agent_id == lease.agent_id
        && detail.lease_started_at == lease.leased_at
}

fn worker_is_running(
    lease: &ParallelModeSlotLeaseSnapshot,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
) -> bool {
    if lease.state != ParallelModeSlotLeaseState::Running
        || lease.running_started_at.is_none()
        || runtime
            .distributor_queue_records
            .iter()
            .any(|record| record.session_key == lease.session_key())
    {
        return false;
    }
    // Running lease는 배포/정리가 끝날 때까지 남는다. session의 실제 실행 구간만 인정한다.
    runtime
        .session_details
        .iter()
        .find(|detail| session_matches_lease(detail, lease))
        .is_none_or(|detail| matches!(detail.state_label.as_str(), "starting" | "running"))
}

fn execution_ids(
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
    started: &[String],
    owned: &impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut ids: BTreeSet<String> = started.iter().filter(|id| owned(id)).cloned().collect();
    ids.extend(
        runtime
            .slot_leases
            .values()
            .filter(|lease| owned(&lease.task_id) && lease.running_started_at.is_some())
            .map(|lease| lease.task_id.clone()),
    );
    // thread/prepared만으로는 turn이 시작됐다고 할 수 없다. 이전 버전의 기록도 실행 증거가 있어야 한다.
    ids.extend(
        runtime
            .session_details
            .iter()
            .filter(|detail| {
                owned(&detail.task_id)
                    && (detail.state_label == "running"
                        || detail.history.iter().any(|event| {
                            matches!(event.state_label.as_str(), "running" | "reported_complete")
                        }))
            })
            .map(|detail| detail.task_id.clone()),
    );
    ids.extend(
        runtime
            .distributor_queue_records
            .iter()
            .filter(|record| owned(&record.task_id))
            .map(|record| record.task_id.clone()),
    );
    ids.into_iter().collect()
}

fn active_validation<'a>(
    records: &'a [ValidationEvidence],
    active_task_id: Option<&str>,
) -> Option<&'a ValidationEvidence> {
    let active = active_task_id?;
    // 연결은 영속 삽입 순서다. 보완 task 자체의 새 검증이 부모의 이전 지적보다 우선한다.
    records
        .iter()
        .rev()
        .find(|evidence| !evidence.superseded && evidence.source_task_id == active)
        .or_else(|| {
            records.iter().rev().find(|evidence| {
                !evidence.superseded && evidence.record.remediation_for_task(active).is_some()
            })
        })
}

fn current_validation_terminal(
    record: &PrValidationRecord,
    active_task_id: Option<&str>,
    remediation_origin_key: Option<&str>,
) -> Option<IssueExecutionTerminalRecord> {
    if let Some(terminal) = validation_terminal(record) {
        return Some(terminal);
    }
    let finding = if matches!(
        record.phase(),
        PrValidationPhase::RemediationQueued | PrValidationPhase::RemediationRunning
    ) {
        let correlation = record.remediation_for_task(active_task_id?)?;
        record.finding(correlation.finding_key())?
    } else if matches!(
        record.phase(),
        PrValidationPhase::PreMergeObservation | PrValidationPhase::PostMergeObservation
    ) {
        record.first_unremediated_finding()?
    } else {
        return None;
    };
    Some(rework_terminal(record, finding, remediation_origin_key))
}

fn terminal_class(
    reason: &PrValidationTerminalReason,
) -> Option<(IssueExecutionTerminalClass, &'static str)> {
    Some(match reason {
        PrValidationTerminalReason::AllConfiguredSourcesComplete => return None,
        PrValidationTerminalReason::PullRequestClosedWithoutMerge => (
            IssueExecutionTerminalClass::PullRequestClosed,
            "pull_request_closed",
        ),
        PrValidationTerminalReason::IntegrationEvidenceMissing => (
            IssueExecutionTerminalClass::ContractRecovery,
            "integration_evidence_missing",
        ),
        PrValidationTerminalReason::IntegrationAuthorityConflict => (
            IssueExecutionTerminalClass::IdentityConflict,
            "integration_authority_conflict",
        ),
        PrValidationTerminalReason::PostMergeContractBlocked => (
            IssueExecutionTerminalClass::ContractRecovery,
            "post_merge_contract_blocked",
        ),
        PrValidationTerminalReason::IdentityViolation => (
            IssueExecutionTerminalClass::IdentityConflict,
            "identity_violation",
        ),
        PrValidationTerminalReason::ObservationFailed => (
            IssueExecutionTerminalClass::ObserveAgain,
            "observation_failed",
        ),
        PrValidationTerminalReason::RemediationAdmissionFailed => (
            IssueExecutionTerminalClass::QueueRecovery,
            "remediation_admission_failed",
        ),
    })
}

pub(super) fn validation_terminal(
    record: &PrValidationRecord,
) -> Option<IssueExecutionTerminalRecord> {
    let (classification, reason) = terminal_class(record.terminal_reason()?)?;
    // revision은 FSM이 영속화한 관측 회차다. 조회 시각이나 현재 프로세스 식별자를 사용하지 않는다.
    let revision = record.observation_revision().to_string();
    Some(IssueExecutionTerminalRecord {
        record_id: stable_id(
            "validation-terminal-v1",
            &[
                record.key().as_str(),
                &revision,
                record.target_shas().source_sha().as_str(),
                record.target_shas().base_sha().as_str(),
                reason,
            ],
        ),
        classification,
    })
}

fn rework_terminal(
    record: &PrValidationRecord,
    finding: &PrValidationFinding,
    remediation_origin_key: Option<&str>,
) -> IssueExecutionTerminalRecord {
    // 같은 지적을 기다리며 polling하는 동안 observation_revision이 올라가도 보완 원인은 하나다.
    IssueExecutionTerminalRecord {
        record_id: stable_id(
            "validation-rework-v1",
            &[
                remediation_origin_key.unwrap_or_else(|| record.key().as_str()),
                finding.key().source().as_str(),
                finding.key().provider_event_id(),
                finding.target_sha().as_str(),
            ],
        ),
        classification: IssueExecutionTerminalClass::ReworkNeeded,
    }
}

fn integration_evidence(
    validations: &[ValidationEvidence],
    queue: &[PlanningAuthorityDistributorQueueRecord],
    owned: &impl Fn(&str) -> bool,
) -> Result<Option<IssueExecutionIntegration>> {
    for evidence in validations {
        let Some(attestation) = evidence.record.integration_attestation() else {
            continue;
        };
        if attestation.source_sha() != evidence.record.target_shas().source_sha()
            || attestation
                .pull_request_number()
                .is_some_and(|number| number != evidence.record.target().pull_request_number())
        {
            bail!("the integration evidence does not match the validated source");
        }
        return Ok(Some(integration_record(
            attestation.source_sha().as_str().to_owned(),
            attestation.evidence_sha().as_str().to_owned(),
            Some(evidence.record.target().canonical_url()),
        )));
    }
    // PR 없는 직접 전달도 성공이 영속화된 queue 결과만 통합 증거로 인정한다.
    for record in queue {
        if !owned(&record.task_id)
            || record.pull_request_number.is_some()
            || record.queue_state != ParallelModeQueueItemState::Done
            || record.integration_state != "done"
        {
            continue;
        }
        let Some(integration_sha) = record.integration_commit_sha.as_ref() else {
            continue;
        };
        return Ok(Some(integration_record(
            record.effective_source_commit_sha(),
            integration_sha.clone(),
            None,
        )));
    }
    Ok(None)
}

fn integration_record(
    source_sha: String,
    integration_sha: String,
    pull_request_url: Option<String>,
) -> IssueExecutionIntegration {
    // 새 검증 record가 동일한 통합을 다시 확인해도 이미 기록한 통합 사실의 신원은 바뀌지 않는다.
    let record_id = stable_id(
        "issue-integration-v1",
        &[
            &source_sha,
            &integration_sha,
            pull_request_url.as_deref().unwrap_or(""),
        ],
    );
    IssueExecutionIntegration {
        record_id,
        source_sha,
        integration_sha,
        pull_request_url,
    }
}

fn question_target(
    task: &TaskDefinition,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
) -> Option<IssueExecutionQuestion> {
    let mut leases = runtime
        .slot_leases
        .values()
        .filter(|lease| lease.task_id == task.id);
    let lease = leases.next()?;
    if leases.next().is_some() || lease.state != ParallelModeSlotLeaseState::Running {
        return None;
    }
    let generation = lease.lease_generation.as_deref()?;
    if !lease.has_valid_lease_generation() {
        return None;
    }
    let mut details = runtime
        .session_details
        .iter()
        .filter(|detail| session_matches_lease(detail, lease));
    let detail = details.next()?;
    if details.next().is_some()
        || !(detail.completion_state_label == "awaiting_user"
            || (detail.state_label == "awaiting_user"
                && detail.completion_state_label == "in_progress"))
    {
        // 재개 claim의 새 generation은 ACK 전 continuation_unknown이다.
        // task 상태만 기다림으로 남아 있더라도 같은 답을 위한 새 질문을 만들지 않는다.
        return None;
    }
    let thread_id = detail.thread_id.as_ref().filter(|id| {
        !id.is_empty() && !id.chars().any(char::is_whitespace) && !id.chars().any(char::is_control)
    })?;
    let question_id = stable_id(
        "issue-question-v1",
        &[
            &task.id,
            &detail.session_key,
            thread_id,
            &lease.slot_id,
            &lease.worktree_path,
            &lease.branch_name,
            generation,
            &task.updated_at,
        ],
    );
    Some(IssueExecutionQuestion {
        question_id,
        task_id: task.id.clone(),
        session_key: detail.session_key.clone(),
        thread_id: thread_id.clone(),
        slot_id: lease.slot_id.clone(),
        worktree_path: lease.worktree_path.clone(),
        branch_name: lease.branch_name.clone(),
        lease_generation: generation.to_owned(),
        awaiting_task_updated_at: task.updated_at.clone(),
    })
}

fn stable_id(namespace: &str, parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{namespace}-{:x}", hash.finalize())
}

#[cfg(test)]
mod tests;
