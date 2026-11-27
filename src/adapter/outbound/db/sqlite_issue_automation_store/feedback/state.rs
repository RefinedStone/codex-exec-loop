use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, params};

use super::super::{
    SqliteIssueAutomationStoreAdapter, autonomy, collection, coordinator, enum_name, execution,
    intervention, next_revision, read_revision, sql_revision, transaction,
};
use super::{decode, encode, intents, validate_issue};
use crate::application::port::outbound::issue_feedback_store_port::{
    IssueFeedbackFence, IssueFeedbackGrantResult, IssueFeedbackSaveResult, IssueFeedbackSnapshot,
};
use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::execution::IssueExecutionState;
use crate::domain::issue_automation::feedback::{
    IssueFeedbackDesired, IssueFeedbackEvidence, IssueFeedbackIntent, IssueFeedbackIntentState,
    IssueFeedbackOperation, IssueFeedbackState,
};
use crate::domain::issue_automation::source::{IssueSourceContent, IssueSourceRead};
use crate::domain::issue_work::IssueReference;

const COLUMNS: &str = "provider, server_id, repository_id, issue_id, owner_workspace, record, revision, desired_revision";

pub(super) fn read(connection: &Connection, issue: &IssueReference) -> Result<IssueFeedbackState> {
    validate_issue(issue)?;
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_feedback_state
         WHERE provider=?1 AND server_id=?2 AND repository_id=?3 AND issue_id=?4"
    ))?;
    let mut rows = statement.query(params![
        enum_name(&issue.provider)?,
        issue.server_id,
        issue.repository_id,
        issue.issue_id,
    ])?;
    let Some(row) = rows.next()? else {
        return Ok(IssueFeedbackState::new(issue.clone()));
    };
    let record: IssueFeedbackState = decode(row.get(5)?)?;
    record.validate().map_err(|reason| anyhow!(reason))?;
    let desired_revision = u64::try_from(row.get::<_, i64>(7)?)?;
    if record.issue != *issue
        || enum_name(&record.issue.provider)? != row.get::<_, String>(0)?
        || record.issue.server_id != row.get::<_, String>(1)?
        || record.issue.repository_id != row.get::<_, String>(2)?
        || record.issue.issue_id != row.get::<_, String>(3)?
        || record.issue.project_scope.workspace_dir != row.get::<_, String>(4)?
        || record.revision != read_revision(row, 6)?
        || record.desired_revision != desired_revision
        || record.revision < record.desired_revision
        || record.desired.is_some() != (record.desired_revision > 0)
    {
        bail!("the feedback state identity or revision does not match");
    }
    validate_owned_receipts(connection, &record)?;
    Ok(record)
}

fn validate_owned_receipts(connection: &Connection, state: &IssueFeedbackState) -> Result<()> {
    if let Some(owner) = &state.comment_owner {
        let record = intents::read(connection, &owner.latest_intent_id)?
            .ok_or_else(|| anyhow!("the feedback comment owner has no confirmed intent"))?;
        let Some(receipt) = &record.receipt else {
            bail!("the feedback comment owner has no confirmed receipt");
        };
        if record.state != IssueFeedbackIntentState::Confirmed
            || receipt.issue != state.issue
            || receipt.actor_id != owner.actor_id
            || record.intent.operation.comment_content_hash() != Some(owner.content_sha256.as_str())
            || !matches!(&receipt.evidence, IssueFeedbackEvidence::Comment { comment_id, body_sha256 }
                if *comment_id == owner.comment_id && *body_sha256 == owner.body_sha256)
        {
            bail!("the feedback comment owner does not match its receipt");
        }
    }
    if let Some(proof) = &state.label_proof {
        let record = intents::read(connection, &proof.intent_id)?
            .ok_or_else(|| anyhow!("the feedback label has no confirmed intent"))?;
        if record.state != IssueFeedbackIntentState::Confirmed
            || record.receipt.as_ref() != Some(proof)
            || !matches!(
                &proof.evidence,
                IssueFeedbackEvidence::Label { applied: true, .. }
            )
        {
            bail!("the feedback label does not match its receipt");
        }
    }
    if let Some(proof) = &state.board_proof {
        let record = intents::read(connection, &proof.intent_id)?
            .ok_or_else(|| anyhow!("the board field has no confirmed intent"))?;
        if record.state != IssueFeedbackIntentState::Confirmed
            || record.receipt.as_ref() != Some(proof)
            || !matches!(&proof.evidence, IssueFeedbackEvidence::BoardField { .. })
        {
            bail!("the board field does not match its receipt");
        }
    }
    if let Some(proof) = &state.close_proof {
        let record = intents::read(connection, &proof.intent_id)?
            .ok_or_else(|| anyhow!("the issue close has no confirmed intent"))?;
        if record.state != IssueFeedbackIntentState::Confirmed
            || record.receipt.as_ref() != Some(proof)
            || !matches!(&proof.evidence, IssueFeedbackEvidence::Closed { .. })
        {
            bail!("the issue close does not match its receipt");
        }
    }
    Ok(())
}

pub(super) fn write(
    connection: &Connection,
    expected_revision: u64,
    state: &IssueFeedbackState,
) -> Result<()> {
    state.validate().map_err(|reason| anyhow!(reason))?;
    validate_issue(&state.issue)?;
    if state.revision != next_revision(expected_revision)?
        || state.revision < state.desired_revision
        || state.desired.is_some() != (state.desired_revision > 0)
    {
        bail!("the feedback state revision cannot skip its predecessor");
    }
    let issue = &state.issue;
    let encoded = encode(state)?;
    let changed = if expected_revision == 0 {
        connection.execute(
            "INSERT INTO issue_feedback_state
             (provider,server_id,repository_id,issue_id,owner_workspace,record,revision,desired_revision)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![enum_name(&issue.provider)?, issue.server_id, issue.repository_id, issue.issue_id,
                issue.project_scope.workspace_dir, encoded, sql_revision(state.revision)?,
                sql_revision(state.desired_revision)?],
        )?
    } else {
        connection.execute(
            "UPDATE issue_feedback_state SET record=?1,revision=?2,desired_revision=?3
             WHERE provider=?4 AND server_id=?5 AND repository_id=?6 AND issue_id=?7
               AND owner_workspace=?8 AND revision=?9",
            params![
                encoded,
                sql_revision(state.revision)?,
                sql_revision(state.desired_revision)?,
                enum_name(&issue.provider)?,
                issue.server_id,
                issue.repository_id,
                issue.issue_id,
                issue.project_scope.workspace_dir,
                sql_revision(expected_revision)?
            ],
        )?
    };
    if changed != 1 {
        bail!("the feedback state lost its revision fence");
    }
    Ok(())
}

pub(super) fn bump(connection: &Connection, state: &mut IssueFeedbackState) -> Result<()> {
    let expected = state.revision;
    state.revision = next_revision(expected)?;
    write(connection, expected, state)
}

pub(super) fn grant_entered(connection: &Connection, issue: &IssueReference) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM issue_feedback_grant_entries
         WHERE provider=?1 AND server_id=?2 AND repository_id=?3 AND issue_id=?4)",
        params![
            enum_name(&issue.provider)?,
            issue.server_id,
            issue.repository_id,
            issue.issue_id
        ],
        |row| row.get(0),
    )?)
}

pub(super) fn publication_attempts(connection: &Connection, issue: &IssueReference) -> Result<u64> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM issue_feedback_intents
         WHERE provider=?1 AND server_id=?2 AND repository_id=?3 AND issue_id=?4
           AND (status IN ('sent','confirmed','needs_confirmation')
                OR (status = 'abandoned' AND json_extract(record, '$.jira_rejection') IS NOT NULL))",
        params![
            enum_name(&issue.provider)?,
            issue.server_id,
            issue.repository_id,
            issue.issue_id
        ],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(count)?)
}

pub(super) fn snapshot_in_transaction(
    connection: &Connection,
    issue: &IssueReference,
) -> Result<Option<IssueFeedbackSnapshot>> {
    let state = read(connection, issue)?;
    let workspace = &issue.project_scope.workspace_dir;
    let Some(collection) = collection::read_control(connection, workspace)? else {
        return Ok(None);
    };
    let Some(item) = collection::queries::item(connection, workspace, issue)? else {
        return Ok(None);
    };
    let binding_pause =
        super::super::binding_pause::read_state(connection, workspace, &item.binding_id)?;
    let work = coordinator::read(connection, workspace, issue)?;
    let execution = work
        .as_ref()
        .map(|work| execution::read(connection, workspace, &work.order.work_order_id))
        .transpose()?
        .flatten();
    let authorization_epoch = intervention::authorization_epoch(connection, workspace, issue)?;
    let close_approval = match (&work, &execution, &item.triage) {
        (Some(work), Some(execution), Some(triage)) if execution.progress.integration.is_some() => {
            use crate::domain::issue_automation::{
                closure::close_decision_id, decision::IssueDecisionBasis,
                policy::IssueAutomationGate,
            };
            let mut basis = IssueDecisionBasis::from_assessment(
                &triage.assessment,
                IssueAutomationGate::Close,
                Some(work.order.work_order_id.clone()),
            )?;
            basis.authorization_epoch = authorization_epoch;
            match close_decision_id(work, execution, &basis) {
                Ok(id) => super::super::decision::read(connection, workspace, &id)?,
                Err(_) => None,
            }
        }
        _ => None,
    };
    Ok(Some(IssueFeedbackSnapshot {
        collection,
        item,
        binding_pause,
        work,
        execution,
        autonomy: autonomy::read(connection, workspace)?,
        authorization_epoch,
        close_approval,
        pending_intervention: intervention::has_pending(connection, workspace, issue)?,
        state,
        first_grant_entered: grant_entered(connection, issue)?,
        publication_attempts: publication_attempts(connection, issue)?,
    }))
}

pub(super) fn snapshot(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
) -> Result<Option<IssueFeedbackSnapshot>> {
    validate_issue(issue)?;
    adapter.with_connection(&issue.project_scope.workspace_dir, |connection, guard| {
        let tx = connection.transaction()?;
        guard.verify_identity()?;
        let result = snapshot_in_transaction(&tx, issue)?;
        guard.verify_identity()?;
        tx.commit()?;
        Ok(result)
    })
}

pub(super) fn validate_fence(fence: IssueFeedbackFence) -> Result<()> {
    for value in [
        fence.collection_epoch,
        fence.binding_run_revision,
        fence.autonomy_revision,
        fence.authorization_epoch,
        fence.feedback_revision,
    ] {
        sql_revision(value)?;
    }
    if fence.collection_epoch == 0 {
        bail!("the feedback collection fence has no persisted epoch");
    }
    for value in [
        fence.observation_revision,
        fence.tracked_revision,
        fence.work_revision,
        fence.execution_revision,
    ]
    .into_iter()
    .flatten()
    {
        if value == 0 {
            bail!("the feedback fence contains an unpersisted revision");
        }
        sql_revision(value)?;
    }
    Ok(())
}

pub(super) fn save_desired(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
    expected: IssueFeedbackFence,
    desired: &IssueFeedbackDesired,
) -> Result<IssueFeedbackSaveResult> {
    validate_issue(issue)?;
    validate_fence(expected)?;
    desired.validate().map_err(|reason| anyhow!(reason))?;
    adapter.with_connection(&issue.project_scope.workspace_dir, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(mut current) = snapshot_in_transaction(tx, issue)? else {
                return Ok(IssueFeedbackSaveResult::Conflict);
            };
            if current.fence() != expected {
                return Ok(IssueFeedbackSaveResult::Conflict);
            }
            if current.state.desired.as_ref() == Some(desired) {
                return Ok(IssueFeedbackSaveResult::Saved(Box::new(current.state)));
            }
            current.state.desired = Some(desired.clone());
            current.state.desired_revision = next_revision(current.state.desired_revision)?;
            bump(tx, &mut current.state)?;
            Ok(IssueFeedbackSaveResult::Saved(Box::new(current.state)))
        })
    })
}

/// This checks persisted authority, not the file-policy grant owned by the service.
pub(super) fn permits_new_write(
    current: &IssueFeedbackSnapshot,
    intent: &IssueFeedbackIntent,
) -> bool {
    if current.collection.config.mode != IssueCollectionMode::Observe
        || !current.binding_pause.allows_automation()
        || current.pending_intervention
        || current.autonomy.pending.is_some()
        || current.state.desired_revision != intent.desired_revision
        || current.item.tracked.as_ref().is_none_or(|tracked| {
            tracked.flags.held || tracked.flags.stale || tracked.flags.unknown
        })
    {
        return false;
    }
    let Some(triage) = &current.item.triage else {
        return false;
    };
    if triage.assessment.basis().automation_policy_version != intent.automation_policy_version
        || triage.assessment.basis().runtime_selection_revision != current.autonomy.revision
        || triage
            .assessment
            .validate_current(&intent.issue, current.item.current.as_ref(), false)
            .is_err()
    {
        return false;
    }
    let Some(IssueSourceRead::Observed(observed)) = &current.item.current else {
        return false;
    };
    let IssueSourceContent::Shared(shared) = &observed.content else {
        return false;
    };
    let observed_connection = match intent.issue.provider {
        crate::domain::issue_work::IssueProvider::Jira => {
            observed.jira.as_ref().map(|jira| &jira.connection)
        }
        crate::domain::issue_work::IssueProvider::Linear => {
            observed.linear.as_ref().map(|linear| &linear.connection)
        }
        crate::domain::issue_work::IssueProvider::GitHub => None,
    };
    if intent.issue.provider != crate::domain::issue_work::IssueProvider::GitHub
        && !matches!((observed_connection, intent.connection.as_ref()), (Some(observed), Some(connection))
            if observed.profile_id == connection.profile_id
                && observed.connection_id == connection.connection_id
                && observed.revision == connection.revision)
    {
        return false;
    }
    if shared.policy_version != intent.disclosure_policy_version {
        return false;
    }
    match &intent.operation {
        IssueFeedbackOperation::CreateComment { content_sha256, .. } => {
            current.state.comment_owner.is_none()
                && current.state.desired.as_ref().is_some_and(|desired|
                    desired.comment_body_sha256 == *content_sha256)
        }
        IssueFeedbackOperation::UpdateComment { owner, content_sha256, .. } => {
            current.state.comment_owner.as_ref() == Some(owner.as_ref())
                && owner.content_sha256 != *content_sha256
                && current.state.desired.as_ref().is_some_and(|desired|
                    desired.comment_body_sha256 == *content_sha256)
        }
        IssueFeedbackOperation::CloseIssue { integration_record_id, integration_sha, decision_id, .. } => {
            current.state.close_proof.is_none()
                && observed.remote_state == crate::domain::issue_automation::source::IssueRemoteState::Open
                && current.authorized_close().is_some_and(|approval| approval.request.id().as_str() == decision_id)
                && current.execution.as_ref().and_then(|execution| execution.progress.integration.as_ref())
                    .is_some_and(|integration| integration.record_id == *integration_record_id && integration.integration_sha == *integration_sha)
        }
        IssueFeedbackOperation::AddLabel { .. } => {
            !current.first_grant_entered && current.state.label_proof.is_none()
        }
        IssueFeedbackOperation::RemoveLabel { label, owned_event_id } => {
            current.first_grant_entered
                && !current.state.label_removal_confirmed
                && (current.item.tracked.as_ref().is_some_and(|tracked| matches!(tracked.state,
                    IssueAutomationState::Done | IssueAutomationState::Cancelled | IssueAutomationState::Failed))
                    || current.execution.as_ref().is_some_and(|execution| matches!(execution.progress.state,
                        IssueExecutionState::Done | IssueExecutionState::Failed)))
                && current.state.label_proof.as_ref().is_some_and(|proof|
                    proof.issue == intent.issue && proof.actor_id == intent.actor_id
                        && matches!(&proof.evidence, IssueFeedbackEvidence::Label { label: observed, event_id, applied: true }
                            if observed == label && event_id == owned_event_id))
        }
        IssueFeedbackOperation::SetBoardField { project_id, item_id, field_id, from_option_id, origin_option_id, .. } => {
            current.state.board_origin(project_id, item_id, field_id, from_option_id.as_deref())
                == *origin_option_id
        }
    }
}

pub(super) fn mark_first_grant(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
    work_order_id: &str,
    expected: IssueFeedbackFence,
) -> Result<IssueFeedbackGrantResult> {
    validate_issue(issue)?;
    validate_fence(expected)?;
    adapter.with_connection(&issue.project_scope.workspace_dir, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(mut current) = snapshot_in_transaction(tx, issue)? else {
                return Ok(IssueFeedbackGrantResult::Conflict);
            };
            if current.fence() != expected
                || !current.binding_pause.allows_automation()
                || current
                    .work
                    .as_ref()
                    .is_none_or(|work| work.order.work_order_id != work_order_id)
            {
                return Ok(IssueFeedbackGrantResult::Conflict);
            }
            if current.first_grant_entered {
                return Ok(IssueFeedbackGrantResult::AlreadyEntered);
            }
            if current.pending_intervention
                || current.autonomy.pending.is_some()
                || !intents::unresolved_in_transaction(tx, issue)?.is_empty()
            {
                return Ok(IssueFeedbackGrantResult::Conflict);
            }
            tx.execute(
                "INSERT INTO issue_feedback_grant_entries
                 (provider,server_id,repository_id,issue_id) VALUES (?1,?2,?3,?4)",
                params![
                    enum_name(&issue.provider)?,
                    issue.server_id,
                    issue.repository_id,
                    issue.issue_id
                ],
            )?;
            bump(tx, &mut current.state)?;
            Ok(IssueFeedbackGrantResult::Marked)
        })
    })
}
