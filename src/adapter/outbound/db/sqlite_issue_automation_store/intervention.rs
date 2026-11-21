//! 개입 intent를 먼저 기록한다. planning fence가 확정되기 전에는 같은 이슈를 다시 실행하지 않는다.
use super::{
    SqliteIssueAutomationStoreAdapter, next_revision, read_issue, transaction, update_issue_row,
    validate_reference_scope,
};
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest,
    IssueSteerDelivery, StoredIssueIntervention,
};
use crate::domain::issue_automation::{IssueAutomationFlag, IssueAutomationState, IssueTransition};
use crate::domain::issue_work::IssueReference;
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) const COMMAND_TABLE: &str = "issue_intervention_commands";
pub(super) const COMMAND_SCHEMA: &str = r#"
CREATE TABLE issue_intervention_commands (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    command_id TEXT COLLATE BINARY NOT NULL,
    issue_key TEXT COLLATE BINARY NOT NULL,
    request_json TEXT NOT NULL,
    receipt_json TEXT,
    PRIMARY KEY (workspace_dir, command_id)
) STRICT, WITHOUT ROWID
"#;
pub(super) const TRACKING_TABLE: &str = "issue_intervention_tracking";
pub(super) const TRACKING_SCHEMA: &str = r#"
CREATE TABLE issue_intervention_tracking (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    issue_key TEXT COLLATE BINARY NOT NULL,
    tracked INTEGER NOT NULL CHECK (tracked IN (0, 1)),
    PRIMARY KEY (workspace_dir, issue_key)
) STRICT, WITHOUT ROWID
"#;

fn issue_key(issue: &IssueReference) -> Result<String> {
    Ok(serde_json::to_string(issue)?)
}

pub(super) fn is_tracked(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
) -> Result<bool> {
    let tracked:Option<bool>=connection.query_row(
        "SELECT tracked FROM issue_intervention_tracking WHERE workspace_dir=?1 AND issue_key=?2",
        params![workspace,issue_key(issue)?],|row|row.get(0),
    ).optional()?;
    Ok(tracked.unwrap_or(true))
}

pub(super) fn has_pending(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM issue_intervention_commands WHERE workspace_dir=?1 AND issue_key=?2 AND receipt_json IS NULL)",
        params![workspace,issue_key(issue)?],|row|row.get(0),
    )?)
}

fn read(
    connection: &Connection,
    workspace: &str,
    command_id: &str,
) -> Result<Option<StoredIssueIntervention>> {
    let value:Option<(String,Option<String>)>=connection.query_row(
        "SELECT request_json,receipt_json FROM issue_intervention_commands WHERE workspace_dir=?1 AND command_id=?2",
        params![workspace,command_id],|row|Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    value
        .map(|(request, receipt)| {
            let request: IssueInterventionRequest = serde_json::from_str(&request)?;
            request.validate()?;
            Ok(StoredIssueIntervention {
                request,
                receipt: receipt
                    .map(|encoded| serde_json::from_str(&encoded))
                    .transpose()?,
            })
        })
        .transpose()
}

pub(super) fn get(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    command_id: &str,
) -> Result<Option<StoredIssueIntervention>> {
    adapter.with_connection(workspace, |connection, _guard| {
        read(connection, workspace, command_id)
    })
}

fn begin_in_transaction(
    connection: &Connection,
    workspace: &str,
    request: &IssueInterventionRequest,
) -> Result<StoredIssueIntervention> {
    if let Some(existing) = read(connection, workspace, &request.command_id)? {
        if existing.request != *request {
            bail!("issue intervention command_id was reused for different content");
        }
        return Ok(existing);
    }
    if super::collection::queries::item(connection, workspace, &request.issue)?.is_none()
        && read_issue(connection, workspace, &request.issue)?.is_none()
    {
        bail!("the issue has not been collected in this workspace");
    }
    if has_pending(connection, workspace, &request.issue)? {
        bail!("another intervention for this issue is still pending");
    }
    connection.execute(
        "INSERT INTO issue_intervention_commands (workspace_dir,command_id,issue_key,request_json) VALUES (?1,?2,?3,?4)",
        params![workspace, request.command_id, issue_key(&request.issue)?, serde_json::to_string(request)?],
    )?;
    Ok(StoredIssueIntervention {
        request: request.clone(),
        receipt: None,
    })
}

pub(super) fn begin(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    request: &IssueInterventionRequest,
) -> Result<StoredIssueIntervention> {
    request.validate()?;
    validate_reference_scope(workspace, &request.issue)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            begin_in_transaction(tx, workspace, request)
        })
    })
}

pub(super) fn claim_steer(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    request: &IssueInterventionRequest,
    receipt: &IssueInterventionReceipt,
) -> Result<bool> {
    request.validate()?;
    validate_reference_scope(workspace, &request.issue)?;
    let work_order_id =
        crate::domain::issue_automation::coordinator::issue_work_order_id(&request.issue);
    if !matches!(request.action, IssueInterventionAction::Steer { .. })
        || receipt.issue != request.issue
        || receipt.command_id != request.command_id
        || receipt.action != request.action.label()
        || receipt.work_order_id.as_deref() != Some(work_order_id.as_str())
        || receipt.delivery != Some(IssueSteerDelivery::Unknown)
        || receipt.authorization_epoch > i64::MAX as u64
        || !receipt.changed
        || receipt.handoff.is_some()
    {
        bail!("a steering claim requires its exact initial Unknown receipt");
    }
    adapter.with_connection(workspace, |connection, guard| transaction(connection, guard, |tx| {
        let stored = begin_in_transaction(tx, workspace, request)?;
        if stored.receipt.is_some() {
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE issue_intervention_commands SET receipt_json=?3 WHERE workspace_dir=?1 AND command_id=?2 AND receipt_json IS NULL",
            params![workspace, request.command_id, serde_json::to_string(receipt)?],
        )?;
        if changed != 1 { bail!("the steering intent changed before its claim"); }
        super::triage::bump_catalog(tx, workspace)?;
        Ok(true)
    }))
}

pub(super) fn finish(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    receipt: &IssueInterventionReceipt,
) -> Result<()> {
    validate_reference_scope(workspace, &receipt.issue)?;
    adapter.with_connection(workspace,|connection,guard|transaction(connection,guard,|tx|{
        let stored=read(tx,workspace,&receipt.command_id)?
            .ok_or_else(||anyhow::anyhow!("issue intervention intent is missing"))?;
        if stored.request.issue != receipt.issue || stored.request.action.label() != receipt.action {
            bail!("issue intervention receipt does not match its intent");
        }
        if let Some(previous) = stored.receipt {
            if previous == *receipt { return Ok(()); }
            // The first claimant may confirm queuing or acknowledgement; identities stay immutable.
            let mut expected = previous.clone();
            expected.delivery = receipt.delivery;
            if receipt.delivery == Some(IssueSteerDelivery::Queued) {
                expected.changed = receipt.changed;
            }
            if previous.delivery != Some(IssueSteerDelivery::Unknown)
                || !matches!(receipt.delivery, Some(IssueSteerDelivery::Queued | IssueSteerDelivery::Sent))
                || !matches!(stored.request.action, IssueInterventionAction::Steer { .. })
                || expected != *receipt
            {
                bail!("a completed issue intervention receipt cannot change");
            }
        }
        if let Some(mut issue)=read_issue(tx,workspace,&receipt.issue)? {
            let before=issue.revision();
            let changed=match stored.request.action {
                IssueInterventionAction::Hold|IssueInterventionAction::TakeOver=>issue.set_flag(IssueAutomationFlag::Held,true),
                IssueInterventionAction::Release=>issue.set_flag(IssueAutomationFlag::Held,false),

                IssueInterventionAction::Cancel{..} if issue.state()!=IssueAutomationState::Done=>issue.transition(IssueTransition::Cancel)?,
                _=>false,
            };
            if changed {
                let mut record=issue.to_record();
                record.revision=next_revision(before)?;
                update_issue_row(tx,before,&record)?;
            }
        }
        match stored.request.action {
            IssueInterventionAction::Track|IssueInterventionAction::Untrack=>{
                let tracked=matches!(stored.request.action,IssueInterventionAction::Track);
                tx.execute(
                    "INSERT INTO issue_intervention_tracking (workspace_dir,issue_key,tracked) VALUES (?1,?2,?3)
                     ON CONFLICT(workspace_dir,issue_key) DO UPDATE SET tracked=excluded.tracked",
                    params![workspace,issue_key(&receipt.issue)?,tracked],
                )?;
            },
            _=>{},
        }
        tx.execute(
            "UPDATE issue_intervention_commands SET receipt_json=?3 WHERE workspace_dir=?1 AND command_id=?2",
            params![workspace,receipt.command_id,serde_json::to_string(receipt)?],
        )?;
        super::triage::bump_catalog(tx,workspace)?;
        Ok(())
    }))
}

pub(super) fn pending(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<Vec<StoredIssueIntervention>> {
    adapter.with_connection(workspace,|connection,_|{
        let mut statement=connection.prepare(
            "SELECT command_id FROM issue_intervention_commands WHERE workspace_dir=?1 AND receipt_json IS NULL ORDER BY command_id",
        )?;
        let ids=statement.query_map(params![workspace],|row|row.get::<_,String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id|read(connection,workspace,&id)?
            .ok_or_else(||anyhow::anyhow!("pending issue intervention disappeared"))).collect()
    })
}

pub(super) fn authorization_epoch(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
) -> Result<u64> {
    let value:i64=connection.query_row(
        "SELECT COALESCE(MAX(json_extract(receipt_json,'$.authorizationEpoch')),0)
         FROM issue_intervention_commands WHERE workspace_dir=?1 AND issue_key=?2 AND receipt_json IS NOT NULL",
        params![workspace,issue_key(issue)?],|row|row.get(0),
    )?;
    Ok(u64::try_from(value)?)
}
