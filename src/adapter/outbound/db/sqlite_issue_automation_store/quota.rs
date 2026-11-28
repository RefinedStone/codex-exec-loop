//! Durable one-shot quota retry revisions, separate from all work and arbiter reservations.
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::application::port::outbound::issue_account_quota_port::{
    IssueQuotaHold, IssueQuotaLane, IssueQuotaWait, IssueQuotaWaitChange,
};
use crate::domain::issue_automation::collection::IssueAccountQuotaState;

pub(super) const TABLE: &str = "issue_account_quota";
pub(super) const SCHEMA: &str = "
    CREATE TABLE issue_account_quota (
        workspace_dir TEXT NOT NULL,
        lane TEXT NOT NULL CHECK (lane IN ('start','arbiter')),
        revision INTEGER NOT NULL CHECK (revision > 0),
        hold TEXT CHECK (hold IN ('exhausted','unknown','retry_queued')),
        PRIMARY KEY (workspace_dir,lane)
    );";

pub(super) fn read(
    connection: &Connection,
    workspace: &str,
    lane: IssueQuotaLane,
) -> Result<Option<IssueQuotaWait>> {
    let row: Option<(i64, Option<String>)> = connection
        .query_row(
            "SELECT revision,hold FROM issue_account_quota WHERE workspace_dir=?1 AND lane=?2",
            params![workspace, super::enum_name(&lane)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(revision, hold)| {
        let hold = match hold.as_deref() {
            None => None,
            Some("exhausted") => Some(IssueQuotaHold::Exhausted),
            Some("unknown") => Some(IssueQuotaHold::Unknown),
            Some("retry_queued") => Some(IssueQuotaHold::RetryQueued),
            Some(_) => anyhow::bail!("invalid issue quota hold"),
        };
        Ok(IssueQuotaWait {
            revision: u64::try_from(revision)?,
            hold,
        })
    })
    .transpose()
}

pub(super) fn projection(
    connection: &Connection,
    workspace: &str,
) -> Result<IssueAccountQuotaState> {
    Ok(IssueAccountQuotaState {
        start: read(connection, workspace, IssueQuotaLane::Start)?
            .filter(|wait| wait.hold.is_some()),
        arbiter: read(connection, workspace, IssueQuotaLane::Arbiter)?
            .filter(|wait| wait.hold.is_some()),
    })
}

pub(super) fn change(
    tx: &Transaction<'_>,
    workspace: &str,
    change: IssueQuotaWaitChange,
) -> Result<bool> {
    let current = read(tx, workspace, change.lane)?;
    if current.map_or(0, |state| state.revision) != change.expected_revision {
        return Ok(false);
    }
    let revision = i64::try_from(change.expected_revision)?
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("issue quota revision exhausted"))?;
    tx.execute(
        "INSERT INTO issue_account_quota (workspace_dir,lane,revision,hold) VALUES (?1,?2,?3,?4)
         ON CONFLICT(workspace_dir,lane) DO UPDATE SET revision=excluded.revision,hold=excluded.hold",
        params![
            workspace,
            super::enum_name(&change.lane)?,
            revision,
            change.hold.as_ref().map(super::enum_name).transpose()?,
        ],
    )?;
    super::triage::bump_catalog(tx, workspace)?;
    Ok(true)
}
