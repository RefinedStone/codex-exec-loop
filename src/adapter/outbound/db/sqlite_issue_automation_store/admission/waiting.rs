use super::super::{enum_name, intervention, triage};
use crate::domain::issue_automation::IssueAutomationState;
use crate::domain::issue_automation::collection::IssueTriageProjection;
use crate::domain::issue_automation::coordinator::IssueStartBudgetWait;
use crate::domain::issue_automation::triage::IssueTriageAssessment;
use crate::domain::issue_work::IssueReference;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

type WaitRow = (String, i64, i64, String);

fn read(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
) -> Result<Option<WaitRow>> {
    Ok(connection.query_row(
        "SELECT input_fingerprint,authorization_epoch,collection_epoch,waiting FROM issue_start_budget_waits
         WHERE workspace_dir=?1 AND provider=?2 AND server_id=?3 AND repository_id=?4 AND issue_id=?5",
        params![workspace, enum_name(&issue.provider)?, issue.server_id, issue.repository_id, issue.issue_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?)
}

pub(in super::super) fn projection(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
    triage: Option<&IssueTriageProjection>,
) -> Result<Option<IssueStartBudgetWait>> {
    let Some(triage) = triage.filter(|triage| !triage.stale) else {
        return Ok(None);
    };
    let Some((fingerprint, epoch, collection_epoch, encoded)) = read(connection, workspace, issue)?
    else {
        return Ok(None);
    };
    let Some(control) = super::super::collection::read_control(connection, workspace)? else {
        return Ok(None);
    };
    if control.config.mode
        != crate::domain::issue_automation::collection::IssueCollectionMode::Observe
        || control.epoch != u64::try_from(collection_epoch)?
        || fingerprint != triage.assessment.input_fingerprint()
        || u64::try_from(epoch)? != intervention::authorization_epoch(connection, workspace, issue)?
        || !super::super::autonomy::permits_assessment(
            connection,
            workspace,
            triage.assessment.basis().runtime_selection_revision,
        )?
        || !intervention::is_tracked(connection, workspace, issue)?
    {
        return Ok(None);
    }
    let Some(tracked) = super::super::read_issue(connection, workspace, issue)? else {
        return Ok(None);
    };
    if tracked.state() != IssueAutomationState::Ready
        || tracked.flags().held
        || tracked.flags().stale
        || tracked.flags().unknown
    {
        return Ok(None);
    }
    if encoded.len() > 1024 {
        bail!("invalid stored issue budget wait");
    }
    let wait: IssueStartBudgetWait =
        serde_json::from_str(&encoded).map_err(|_| anyhow!("invalid stored issue budget wait"))?;
    if wait.exhausted.is_empty()
        || wait.exhausted.len() > 3
        || wait
            .exhausted
            .iter()
            .enumerate()
            .any(|(i, limit)| wait.exhausted[..i].contains(limit))
        || wait.utc_day > i64::MAX as u64 / super::MILLIS_PER_UTC_DAY
    {
        bail!("invalid stored issue budget wait");
    }
    Ok(Some(wait))
}

pub(super) fn save(
    connection: &Transaction<'_>,
    workspace: &str,
    issue: &IssueReference,
    assessment: &IssueTriageAssessment,
    wait: &IssueStartBudgetWait,
) -> Result<(u64, bool)> {
    let fingerprint = assessment.input_fingerprint();
    let epoch = i64::try_from(intervention::authorization_epoch(
        connection, workspace, issue,
    )?)?;
    let collection_epoch = i64::try_from(
        super::super::collection::read_control(connection, workspace)?
            .ok_or_else(|| anyhow!("issue collection control is missing"))?
            .epoch,
    )?;
    let encoded = serde_json::to_string(wait)?;
    if read(connection, workspace, issue)?
        == Some((
            fingerprint.clone(),
            epoch,
            collection_epoch,
            encoded.clone(),
        ))
    {
        return Ok((triage::catalog_revision(connection, workspace)?, false));
    }
    connection.execute(
        "INSERT INTO issue_start_budget_waits
         (workspace_dir,provider,server_id,repository_id,issue_id,input_fingerprint,authorization_epoch,collection_epoch,waiting)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(workspace_dir,provider,server_id,repository_id,issue_id)
         DO UPDATE SET input_fingerprint=excluded.input_fingerprint,authorization_epoch=excluded.authorization_epoch,collection_epoch=excluded.collection_epoch,waiting=excluded.waiting",
        params![workspace, enum_name(&issue.provider)?, issue.server_id, issue.repository_id, issue.issue_id, fingerprint, epoch, collection_epoch, encoded],
    )?;
    Ok((triage::bump_catalog(connection, workspace)?, true))
}

pub(super) fn clear(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
) -> Result<bool> {
    Ok(connection.execute(
        "DELETE FROM issue_start_budget_waits
         WHERE workspace_dir=?1 AND provider=?2 AND server_id=?3 AND repository_id=?4 AND issue_id=?5",
        params![workspace, enum_name(&issue.provider)?, issue.server_id, issue.repository_id, issue.issue_id],
    )? != 0)
}
