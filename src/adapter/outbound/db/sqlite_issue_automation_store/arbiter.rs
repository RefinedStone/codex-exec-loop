//! 예약·예산·첫 답 우선·알림을 SQLite의 한 전이로 확정한다. 모델은 여기서 호출하지 않는다.
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use super::{
    SqliteIssueAutomationStoreAdapter, decision, next_revision, sql_revision, transaction,
    validate_expected_revision, validate_reference_scope,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueArbiterClaimRequest, IssueArbiterClaimResult, IssueArbiterFinishCommit,
    IssueArbiterFinishRequest, IssueArbiterFinishResult, IssueArbiterWaitRequest,
    IssueDecisionFence, IssueDecisionSaveResult,
};
use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::arbiter::{
    IssueArbiterAttempt, IssueArbiterFinishedEvent, IssueArbiterFinishedOutcome,
    IssueArbiterResult, IssueArbiterSettings, IssueArbiterWaitEntry, IssueArbiterWaitReason,
    IssueArbiterWaitSnapshot, MAX_ISSUE_ARBITER_BATCH, arbiter_eligible,
};
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::decision::{
    IssueDecisionId, IssueDecisionRequest, IssueDecisionState, IssueDecisionTimeout,
    PersistedIssueDecisionRequest, StoredIssueDecision,
};
use crate::domain::issue_automation::triage::IssueTriageAssessment;

mod context;
mod queries;
mod storage;

pub(super) use queries::{acknowledge, pending, unfinished, waits};
pub(super) use storage::{create_schema, schema_objects};

pub(super) fn claim(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    request: &IssueArbiterClaimRequest,
) -> Result<IssueArbiterClaimResult> {
    request
        .input_basis
        .validate()
        .map_err(|reason| anyhow!(reason))?;
    request
        .settings
        .validate()
        .map_err(|reason| anyhow!(reason))?;
    validate_reference_scope(workspace, &request.current.basis().issue)?;
    validate_expected_revision(request.fence.decision_revision)?;
    timestamp(request.now_unix_ms)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(record) = decision::read(tx, workspace, &request.decision_id)? else {
                return Ok(IssueArbiterClaimResult::Conflict);
            };
            if record.request.state() != IssueDecisionState::Open {
                return Ok(IssueArbiterClaimResult::Resolved(Box::new(record)));
            }
            if let Some((_, commit)) = storage::read_event(tx, workspace, &request.decision_id)? {
                return Ok(IssueArbiterClaimResult::Deferred(Box::new(commit)));
            }
            if let Some(attempt) =
                storage::attempt_for_decision(tx, workspace, &request.decision_id)?
            {
                if attempt.finished_event_id.is_some() {
                    bail!("a finished arbiter attempt has no result");
                }
                return Ok(IssueArbiterClaimResult::Existing(Box::new(attempt)));
            }
            if record.revision != request.fence.decision_revision
                || request.input_basis.decision_revision != record.revision
                || request.input_basis.binding_run_revision
                    != request.fence.triage.binding_run_revision
                || request.input_basis.decision_basis != record.request.definition().basis
            {
                return Ok(IssueArbiterClaimResult::Conflict);
            }
            if !context::current_allowed(
                tx,
                workspace,
                request.fence,
                &request.current,
                &record,
                request.now_unix_ms,
            )? {
                return Ok(IssueArbiterClaimResult::Ineligible);
            }
            let day = storage::reserve_day(tx, workspace, request.now_unix_ms)?;
            if !storage::budget_available(tx, workspace, &request.settings, day)? {
                let commit = commit_event(
                    tx,
                    workspace,
                    record.revision,
                    &record,
                    IssueArbiterFinishedOutcome::Waiting {
                        reason: IssueArbiterWaitReason::BudgetExhausted,
                    },
                    None,
                    request.now_unix_ms,
                )?;
                return Ok(IssueArbiterClaimResult::Deferred(Box::new(commit)));
            }
            let deadline_at_unix_ms = request
                .settings
                .timeout_seconds
                .checked_mul(1000)
                .and_then(|duration| request.now_unix_ms.checked_add(duration))
                .filter(|value| *value <= i64::MAX as u64)
                .ok_or_else(|| anyhow!("the arbiter deadline is outside the supported range"))?;
            let attempt = IssueArbiterAttempt {
                attempt_id: storage::attempt_id(workspace, &request.decision_id, record.revision)?,
                decision_id: request.decision_id.clone(),
                input_basis: request.input_basis.clone(),
                settings: request.settings,
                claimed_at_unix_ms: request.now_unix_ms,
                deadline_at_unix_ms,
                utc_day: day,
                revision: 1,
                finished_event_id: None,
            };
            storage::insert_attempt(tx, workspace, &attempt)?;
            Ok(IssueArbiterClaimResult::Claimed(Box::new(attempt)))
        })
    })
}

pub(super) fn finish(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    request: &IssueArbiterFinishRequest,
) -> Result<IssueArbiterFinishResult> {
    validate_expected_revision(request.expected_revision)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(attempt) = storage::read_attempt(tx, workspace, &request.attempt_id)? else {
                return Ok(IssueArbiterFinishResult::Conflict);
            };
            if let Some(event_id) = &attempt.finished_event_id {
                let (_, commit) = storage::read_event(tx, workspace, &attempt.decision_id)?
                    .ok_or_else(|| anyhow!("the finished arbiter result is missing"))?;
                if &commit.event.event_id != event_id {
                    bail!("the finished arbiter event changed identity");
                }
                return Ok(IssueArbiterFinishResult::Saved(Box::new(commit)));
            }
            if attempt.revision != request.expected_revision {
                return Ok(IssueArbiterFinishResult::Conflict);
            }
            timestamp(request.now_unix_ms)?;
            let record = decision::read(tx, workspace, &attempt.decision_id)?
                .ok_or_else(|| anyhow!("the arbiter decision is missing"))?;
            let (outcome, next) = finish_outcome(tx, workspace, &attempt, &record, request)?;
            let commit = commit_event(
                tx,
                workspace,
                attempt.input_basis.decision_revision,
                &record,
                outcome,
                next.as_ref(),
                request.now_unix_ms,
            )?;
            storage::finish_attempt(tx, workspace, &attempt, &commit.event.event_id)?;
            Ok(IssueArbiterFinishResult::Saved(Box::new(commit)))
        })
    })
}

fn finish_outcome(
    tx: &Transaction<'_>,
    workspace: &str,
    attempt: &IssueArbiterAttempt,
    record: &StoredIssueDecision,
    request: &IssueArbiterFinishRequest,
) -> Result<(IssueArbiterFinishedOutcome, Option<IssueDecisionRequest>)> {
    // Human/다른 terminal 저장이 먼저면 입력 읽기 실패·마감·낡은 fence보다 우선한다.
    if record.request.state() != IssueDecisionState::Open {
        return Ok((IssueArbiterFinishedOutcome::Discarded {}, None));
    }
    if context::durable_basis_changed(tx, workspace, record)?
        || record.request.definition().basis != attempt.input_basis.decision_basis
        || record.revision != attempt.input_basis.decision_revision
    {
        let mut next = record.request.to_record();
        next.state = IssueDecisionState::Superseded;
        return Ok((
            IssueArbiterFinishedOutcome::Superseded {},
            Some(IssueDecisionRequest::hydrate(next)?),
        ));
    }
    let Some(current) = &request.current else {
        return waiting(IssueArbiterWaitReason::ContextUnavailable);
    };
    current
        .input_basis
        .validate()
        .map_err(|reason| anyhow!(reason))?;
    if !context::current_allowed(
        tx,
        workspace,
        current.fence,
        &current.current,
        record,
        request.now_unix_ms,
    )? || current.input_basis.decision_basis != record.request.definition().basis
        || current.input_basis.decision_revision != record.revision
        || current.input_basis.binding_run_revision != current.fence.triage.binding_run_revision
    {
        return waiting(IssueArbiterWaitReason::ContextUnavailable);
    }
    if current.input_basis.model_route_revision != attempt.input_basis.model_route_revision {
        return waiting(IssueArbiterWaitReason::ContextUnavailable);
    }
    if current.input_basis != attempt.input_basis {
        return waiting(IssueArbiterWaitReason::EvidenceChanged);
    }
    if request.now_unix_ms < attempt.claimed_at_unix_ms
        || request.now_unix_ms >= attempt.deadline_at_unix_ms
    {
        return waiting(IssueArbiterWaitReason::OutcomeUnknown);
    }
    match &request.result {
        IssueArbiterResult::Waiting(reason) => waiting(*reason),
        IssueArbiterResult::Proposal(proposal) => {
            if let Some(reason) = proposal.wait_reason(&attempt.settings) {
                return waiting(reason);
            }
            match record.request.answer_arbiter(
                &current.input_basis.decision_basis,
                &current.current,
                &attempt.settings,
                proposal,
                request.now_unix_ms,
            ) {
                Ok(resolution) => Ok((
                    IssueArbiterFinishedOutcome::Applied {},
                    Some(resolution.request),
                )),
                Err(_) => waiting(IssueArbiterWaitReason::InvalidResponse),
            }
        }
    }
}

fn waiting(
    reason: IssueArbiterWaitReason,
) -> Result<(IssueArbiterFinishedOutcome, Option<IssueDecisionRequest>)> {
    Ok((IssueArbiterFinishedOutcome::Waiting { reason }, None))
}

pub(super) fn record_wait(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    request: &IssueArbiterWaitRequest,
) -> Result<IssueArbiterFinishResult> {
    validate_reference_scope(workspace, &request.current.basis().issue)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            if let Some((_, commit)) = storage::read_event(tx, workspace, &request.decision_id)? {
                return Ok(IssueArbiterFinishResult::Saved(Box::new(commit)));
            }
            let Some(record) = decision::read(tx, workspace, &request.decision_id)? else {
                return Ok(IssueArbiterFinishResult::Conflict);
            };
            if storage::attempt_for_decision(tx, workspace, &request.decision_id)?.is_some()
                || !context::current_allowed(
                    tx,
                    workspace,
                    request.fence,
                    &request.current,
                    &record,
                    request.now_unix_ms,
                )?
            {
                return Ok(IssueArbiterFinishResult::Conflict);
            }
            let commit = commit_event(
                tx,
                workspace,
                record.revision,
                &record,
                IssueArbiterFinishedOutcome::Waiting {
                    reason: request.reason,
                },
                None,
                request.now_unix_ms,
            )?;
            Ok(IssueArbiterFinishResult::Saved(Box::new(commit)))
        })
    })
}

fn commit_event(
    tx: &Transaction<'_>,
    workspace: &str,
    source_revision: u64,
    previous: &StoredIssueDecision,
    outcome: IssueArbiterFinishedOutcome,
    next: Option<&IssueDecisionRequest>,
    now_unix_ms: u64,
) -> Result<IssueArbiterFinishCommit> {
    let (record, catalog_revision) = if let Some(next) = next {
        match decision::persist_resolution(tx, workspace, previous, next)? {
            IssueDecisionSaveResult::Saved(commit) => (commit.record, commit.catalog_revision),
            IssueDecisionSaveResult::Conflict => {
                bail!("the arbiter decision changed during its transaction")
            }
        }
    } else {
        (
            previous.clone(),
            super::triage::bump_catalog(tx, workspace)?,
        )
    };
    let event = IssueArbiterFinishedEvent {
        event_id: storage::event_id(workspace, record.request.id(), source_revision)?,
        fact: record.request.fact(),
        outcome,
        at: timestamp(now_unix_ms)?,
        catalog_revision,
    };
    storage::insert_event(tx, workspace, source_revision, &record, &event)?;
    Ok(IssueArbiterFinishCommit {
        record,
        event,
        changed: true,
    })
}

pub(super) fn applied_decision_current(
    connection: &Connection,
    workspace: &str,
    record: &StoredIssueDecision,
) -> Result<bool> {
    Ok(
        storage::read_event(connection, workspace, record.request.id())?.is_some_and(
            |(_, commit)| {
                commit.record == *record
                    && commit.event.outcome == IssueArbiterFinishedOutcome::Applied {}
            },
        ),
    )
}

fn timestamp(now_unix_ms: u64) -> Result<String> {
    i64::try_from(now_unix_ms)
        .ok()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|now| now.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| anyhow!("the arbiter time is outside the supported range"))
}
