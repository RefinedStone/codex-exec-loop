use super::*;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::source::{IssueSourceContent, IssueSourceRead};

pub(super) fn current_allowed(
    connection: &Connection,
    workspace: &str,
    fence: IssueDecisionFence,
    current: &IssueTriageAssessment,
    record: &StoredIssueDecision,
    now_unix_ms: u64,
) -> Result<bool> {
    if record.revision != fence.decision_revision
        || !arbiter_eligible(&record.request, current, now_unix_ms)
    {
        return Ok(false);
    }
    let Some(control) = super::super::collection::read_control(connection, workspace)? else {
        return Ok(false);
    };
    if control.config.mode != IssueCollectionMode::Observe
        || control.epoch != fence.triage.collection_epoch
        || super::super::intervention::has_pending(connection, workspace, record.request.issue())?
        || !super::super::intervention::is_tracked(connection, workspace, record.request.issue())?
    {
        return Ok(false);
    }
    let selection = super::super::autonomy::read(connection, workspace)?;
    if selection.pending.is_some()
        || selection.revision != current.basis().runtime_selection_revision
        || (matches!(
            current.effective_mode(),
            IssueAutomationAutonomy::Gated | IssueAutomationAutonomy::Auto
        ) && !super::super::autonomy::matches_automatic_mode(
            &selection,
            current.effective_mode(),
        ))
    {
        return Ok(false);
    }
    let Some(item) =
        super::super::collection::queries::item(connection, workspace, record.request.issue())?
    else {
        return Ok(false);
    };
    if item.binding_run_blocked
        || item.binding_run_revision != fence.triage.binding_run_revision
        || item.observation_revision != fence.triage.observation_revision
        || item.tracked_revision != fence.triage.tracked_revision
        || item.triage.as_ref().map(|triage| triage.revision) != fence.triage.assessment_revision
        || item
            .triage
            .as_ref()
            .is_none_or(|triage| triage.assessment != *current)
        || item.tracked.as_ref().is_some_and(|tracked| {
            tracked.flags.held
                || tracked.flags.stale
                || tracked.flags.unknown
                || matches!(
                    tracked.state,
                    IssueAutomationState::Done
                        | IssueAutomationState::Failed
                        | IssueAutomationState::Cancelled
                )
        })
    {
        return Ok(false);
    }
    if current
        .validate_current(
            record.request.issue(),
            item.current.as_ref(),
            item.tracked
                .as_ref()
                .is_some_and(|tracked| tracked.flags.held),
        )
        .is_err()
    {
        return Ok(false);
    }
    Ok(
        decision::current_item(connection, workspace, fence.triage, current)?.is_some()
            && record.request.definition().basis
                == decision::current_basis(connection, workspace, &record.request, current)?
            && decision::work_context_current(connection, workspace, &record.request)?,
    )
}

/// 현재 파일 정책을 읽지 못해도 장부가 확정한 epoch·필드·관문 변경은 낡은 요청을 닫는다.
pub(super) fn durable_basis_changed(
    connection: &Connection,
    workspace: &str,
    record: &StoredIssueDecision,
) -> Result<bool> {
    let basis = &record.request.definition().basis;
    if basis.authorization_epoch
        != super::super::intervention::authorization_epoch(
            connection,
            workspace,
            record.request.issue(),
        )?
    {
        return Ok(true);
    }
    let Some(item) =
        super::super::collection::queries::item(connection, workspace, record.request.issue())?
    else {
        return Ok(false);
    };
    if let Some(IssueSourceRead::Observed(observation)) = &item.current
        && let IssueSourceContent::Shared(snapshot) = &observation.content
        && basis.field_hash.as_ref() != Some(&snapshot.basis.field_hash)
    {
        return Ok(true);
    }
    if let Some(triage) = &item.triage {
        let current =
            decision::current_basis(connection, workspace, &record.request, &triage.assessment)?;
        if &current != basis {
            return Ok(true);
        }
    }
    Ok(false)
}
