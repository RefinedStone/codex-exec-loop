use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, Row, params};

use super::super::{
    SqliteIssueAutomationStoreAdapter, enum_name, next_revision, read_revision, sql_revision,
    transaction, validate_expected_revision, validate_workspace,
};
use super::{decode, encode, state, validate_issue};
use crate::application::port::outbound::issue_feedback_store_port::{
    IssueFeedbackClaimResult, IssueFeedbackFence, IssueFeedbackRecordResult,
};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackIntent, IssueFeedbackIntentRecord, IssueFeedbackIntentState, validate_feedback_id,
};
use crate::domain::issue_work::IssueReference;

const COLUMNS: &str = "intent_id,provider,server_id,repository_id,issue_id,owner_workspace,status,record,revision,recorded_fence";
type LegacyFence = (
    u64,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    u64,
    u64,
    u64,
);
type CurrentFeedbackFenceTuple = (
    u64,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    u64,
    u64,
    u64,
    u64,
);

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum EncodedFence {
    Current(CurrentFeedbackFenceTuple),
    Legacy(LegacyFence),
}

struct StoredIntent {
    record: IssueFeedbackIntentRecord,
    recorded_fence: IssueFeedbackFence,
}

fn encode_fence(fence: IssueFeedbackFence) -> Result<String> {
    state::validate_fence(fence)?;
    // The original zero generation keeps the exact v13 JSON representation.
    if fence.binding_run_revision == 0 {
        return Ok(serde_json::to_string(&(
            fence.collection_epoch,
            fence.observation_revision,
            fence.tracked_revision,
            fence.work_revision,
            fence.execution_revision,
            fence.autonomy_revision,
            fence.authorization_epoch,
            fence.feedback_revision,
        ))?);
    }
    Ok(serde_json::to_string(&(
        fence.collection_epoch,
        fence.observation_revision,
        fence.tracked_revision,
        fence.work_revision,
        fence.execution_revision,
        fence.autonomy_revision,
        fence.authorization_epoch,
        fence.feedback_revision,
        fence.binding_run_revision,
    ))?)
}

fn decode_fence(encoded: String) -> Result<IssueFeedbackFence> {
    if encoded.len() > 1024 {
        bail!("the feedback recorded fence exceeds its bound");
    }
    let wire: EncodedFence = serde_json::from_str(&encoded)
        .map_err(|_| anyhow!("the feedback recorded fence is invalid"))?;
    let value = match wire {
        EncodedFence::Current(value) => value,
        EncodedFence::Legacy(value) => (
            value.0, value.1, value.2, value.3, value.4, value.5, value.6, value.7, 0,
        ),
    };
    let fence = IssueFeedbackFence {
        collection_epoch: value.0,
        observation_revision: value.1,
        tracked_revision: value.2,
        work_revision: value.3,
        execution_revision: value.4,
        autonomy_revision: value.5,
        authorization_epoch: value.6,
        feedback_revision: value.7,
        binding_run_revision: value.8,
    };
    state::validate_fence(fence)?;
    Ok(fence)
}

fn hydrate(row: &Row<'_>) -> Result<StoredIntent> {
    let record: IssueFeedbackIntentRecord = decode(row.get(7)?)?;
    record.validate().map_err(|reason| anyhow!(reason))?;
    validate_issue(&record.intent.issue)?;
    let issue = &record.intent.issue;
    if record.intent.intent_id != row.get::<_, String>(0)?
        || enum_name(&issue.provider)? != row.get::<_, String>(1)?
        || issue.server_id != row.get::<_, String>(2)?
        || issue.repository_id != row.get::<_, String>(3)?
        || issue.issue_id != row.get::<_, String>(4)?
        || issue.project_scope.workspace_dir != row.get::<_, String>(5)?
        || enum_name(&record.state)? != row.get::<_, String>(6)?
        || record.revision != read_revision(row, 8)?
    {
        bail!("the feedback intent identity or revision does not match");
    }
    Ok(StoredIntent {
        record,
        recorded_fence: decode_fence(row.get(9)?)?,
    })
}

fn read_stored(connection: &Connection, intent_id: &str) -> Result<Option<StoredIntent>> {
    validate_feedback_id(intent_id).map_err(|reason| anyhow!(reason))?;
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_feedback_intents WHERE intent_id=?1"
    ))?;
    let mut rows = statement.query([intent_id])?;
    rows.next()?.map(hydrate).transpose()
}

pub(super) fn read(
    connection: &Connection,
    intent_id: &str,
) -> Result<Option<IssueFeedbackIntentRecord>> {
    Ok(read_stored(connection, intent_id)?.map(|stored| stored.record))
}

pub(super) fn unresolved_in_transaction(
    connection: &Connection,
    issue: &IssueReference,
) -> Result<Vec<IssueFeedbackIntentRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_feedback_intents
         WHERE provider=?1 AND server_id=?2 AND repository_id=?3 AND issue_id=?4
           AND status IN ('recorded','sent','needs_confirmation') ORDER BY intent_id"
    ))?;
    let mut rows = statement.query(params![
        enum_name(&issue.provider)?,
        issue.server_id,
        issue.repository_id,
        issue.issue_id,
    ])?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        let record = hydrate(row)?.record;
        if !record.intent.issue.same_remote_issue(issue) || !record.state.is_unresolved() {
            bail!("the feedback pending intent belongs to another remote issue");
        }
        records.push(record);
    }
    if records.len() > 1 {
        bail!("the feedback remote issue has conflicting pending intents");
    }
    Ok(records)
}

pub(super) fn unresolved(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
) -> Result<Vec<IssueFeedbackIntentRecord>> {
    validate_issue(issue)?;
    adapter.with_connection(&issue.project_scope.workspace_dir, |connection, _| {
        unresolved_in_transaction(connection, issue)
    })
}

pub(super) fn pending(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<Vec<IssueFeedbackIntentRecord>> {
    validate_workspace(workspace)?;
    adapter.with_connection(workspace, |connection, _| {
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM issue_feedback_intents
             WHERE status IN ('recorded','sent','needs_confirmation')
             ORDER BY intent_id"
        ))?;
        let mut rows = statement.query([])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            let record = hydrate(row)?.record;
            if !record.state.is_unresolved() {
                bail!("the feedback pending intent belongs to another owner");
            }
            records.push(record);
        }
        Ok(records)
    })
}

pub(super) fn insert(
    connection: &Connection,
    record: &IssueFeedbackIntentRecord,
    recorded_fence: IssueFeedbackFence,
) -> Result<()> {
    record.validate().map_err(|reason| anyhow!(reason))?;
    let issue = &record.intent.issue;
    connection.execute(
        "INSERT INTO issue_feedback_intents
         (intent_id,provider,server_id,repository_id,issue_id,owner_workspace,status,record,revision,recorded_fence)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![record.intent.intent_id, enum_name(&issue.provider)?, issue.server_id, issue.repository_id,
            issue.issue_id, issue.project_scope.workspace_dir, enum_name(&record.state)?, encode(record)?,
            sql_revision(record.revision)?, encode_fence(recorded_fence)?],
    )?;
    Ok(())
}

pub(super) fn update(
    connection: &Connection,
    previous: &IssueFeedbackIntentRecord,
    next: &IssueFeedbackIntentRecord,
) -> Result<()> {
    next.validate().map_err(|reason| anyhow!(reason))?;
    if previous.intent != next.intent || next.revision != next_revision(previous.revision)? {
        bail!("the feedback intent identity or revision cannot change");
    }
    let changed = connection.execute(
        "UPDATE issue_feedback_intents SET status=?1,record=?2,revision=?3
         WHERE intent_id=?4 AND revision=?5 AND owner_workspace=?6",
        params![
            enum_name(&next.state)?,
            encode(next)?,
            sql_revision(next.revision)?,
            next.intent.intent_id,
            sql_revision(previous.revision)?,
            next.intent.issue.project_scope.workspace_dir
        ],
    )?;
    if changed != 1 {
        bail!("the feedback intent lost its revision fence");
    }
    Ok(())
}

pub(super) fn record(
    adapter: &SqliteIssueAutomationStoreAdapter,
    expected: IssueFeedbackFence,
    intent: &IssueFeedbackIntent,
) -> Result<IssueFeedbackRecordResult> {
    intent.validate().map_err(|reason| anyhow!(reason))?;
    validate_issue(&intent.issue)?;
    state::validate_fence(expected)?;
    adapter.with_connection(
        &intent.issue.project_scope.workspace_dir,
        |connection, guard| {
            transaction(connection, guard, |tx| {
                if let Some(existing) = read(tx, &intent.intent_id)? {
                    if existing.intent != *intent {
                        bail!("the feedback intent identifier was reused for another request");
                    }
                    state::read(tx, &intent.issue)?;
                    return Ok(IssueFeedbackRecordResult::Existing(existing));
                }
                if !unresolved_in_transaction(tx, &intent.issue)?.is_empty() {
                    return Ok(IssueFeedbackRecordResult::Blocked);
                }
                let Some(mut current) = state::snapshot_in_transaction(tx, &intent.issue)? else {
                    return Ok(IssueFeedbackRecordResult::Conflict);
                };
                if current.fence() != expected || !state::permits_new_write(&current, intent) {
                    return Ok(IssueFeedbackRecordResult::Conflict);
                }
                let record = IssueFeedbackIntentRecord {
                    intent: intent.clone(),
                    state: IssueFeedbackIntentState::Recorded,
                    revision: 1,
                    receipt: None,
                    acknowledged_comment_id: None,
                    contradictory_response: false,
                    closure_acknowledged: false,
                };
                state::bump(tx, &mut current.state)?;
                insert(tx, &record, expected)?;
                Ok(IssueFeedbackRecordResult::Created(record))
            })
        },
    )
}

pub(super) fn claim_sent(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
    intent_id: &str,
    expected_revision: u64,
    expected: IssueFeedbackFence,
) -> Result<IssueFeedbackClaimResult> {
    validate_issue(issue)?;
    validate_expected_revision(expected_revision)?;
    state::validate_fence(expected)?;
    adapter.with_connection(&issue.project_scope.workspace_dir, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(stored) = read_stored(tx, intent_id)? else {
                return Ok(IssueFeedbackClaimResult::Conflict);
            };
            let record = stored.record;
            if record.intent.issue != *issue {
                bail!("the feedback intent belongs to another owner");
            }
            if record.state != IssueFeedbackIntentState::Recorded {
                state::read(tx, issue)?;
                return Ok(IssueFeedbackClaimResult::Existing(record));
            }
            if record.revision != expected_revision {
                return Ok(IssueFeedbackClaimResult::Conflict);
            }
            let Some(mut current) = state::snapshot_in_transaction(tx, issue)? else {
                return Ok(IssueFeedbackClaimResult::Conflict);
            };
            let mut recorded_fence = stored.recorded_fence;
            recorded_fence.feedback_revision = next_revision(recorded_fence.feedback_revision)?;
            if current.fence() != expected
                || current.fence() != recorded_fence
                || !state::permits_new_write(&current, &record.intent)
            {
                return Ok(IssueFeedbackClaimResult::Conflict);
            }
            let pending = unresolved_in_transaction(tx, issue)?;
            if pending.len() != 1 || pending[0].intent != record.intent {
                bail!("the feedback claim does not own the pending intent");
            }
            let mut next = record.clone();
            next.state = IssueFeedbackIntentState::Sent;
            next.revision = next_revision(record.revision)?;
            update(tx, &record, &next)?;
            state::bump(tx, &mut current.state)?;
            Ok(IssueFeedbackClaimResult::Claimed(next))
        })
    })
}
