use super::{SqliteIssueAutomationStoreAdapter, next_revision, transaction};
use crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION;
use crate::domain::issue_automation::autonomy::{
    IssueAutonomyCommit, IssueAutonomyIntent, IssueAutonomySelection, StoredIssueAutonomyCommand,
};
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, params};

mod fences;

pub(super) const FENCE_TABLE: &str = "issue_autonomy_work_fences";
pub(super) const FENCE_SCHEMA: &str = r#"
CREATE TABLE issue_autonomy_work_fences (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    autonomy_command_id TEXT COLLATE BINARY NOT NULL,
    work_order_id TEXT COLLATE BINARY NOT NULL,
    control_command_id TEXT COLLATE BINARY NOT NULL,
    PRIMARY KEY (workspace_dir, autonomy_command_id, work_order_id),
    UNIQUE (workspace_dir, control_command_id),
    FOREIGN KEY (workspace_dir, autonomy_command_id)
        REFERENCES issue_autonomy_commands (workspace_dir, command_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (workspace_dir, work_order_id)
        REFERENCES issue_work_checkpoints (workspace_dir, work_order_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (workspace_dir, control_command_id)
        REFERENCES issue_intervention_commands (workspace_dir, command_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

pub(super) const TABLE: &str = "issue_autonomy_selection";
pub(super) const COMMAND_TABLE: &str = "issue_autonomy_commands";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_autonomy_selection (
    workspace_dir TEXT COLLATE BINARY NOT NULL PRIMARY KEY,
    requested_level TEXT CHECK (requested_level IS NULL OR requested_level IN ('off','observe','propose')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    pending_command_id TEXT
) STRICT, WITHOUT ROWID
"#;
pub(super) const COMMAND_SCHEMA: &str = r#"
CREATE TABLE issue_autonomy_commands (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    command_id TEXT COLLATE BINARY NOT NULL,
    intent TEXT NOT NULL,
    completed_revision INTEGER CHECK (completed_revision IS NULL OR completed_revision >= 1),
    PRIMARY KEY (workspace_dir, command_id)
) STRICT, WITHOUT ROWID
"#;

pub(super) fn command(
    connection: &Connection,
    workspace: &str,
    id: &str,
) -> Result<Option<StoredIssueAutonomyCommand>> {
    let row: Option<(String,Option<i64>)> = connection.query_row(
        "SELECT intent,completed_revision FROM issue_autonomy_commands WHERE workspace_dir=?1 AND command_id=?2",
        params![workspace,id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    row.map(|(encoded, completed)| {
        if encoded.len() > 4096 {
            bail!("invalid stored autonomy intent");
        }
        let intent: IssueAutonomyIntent = serde_json::from_str(&encoded)
            .map_err(|_| anyhow!("invalid stored autonomy intent"))?;
        intent.validate().map_err(|reason| anyhow!(reason))?;
        if intent.command_id != id {
            bail!("autonomy command identity mismatch");
        }
        let completed_revision = completed.map(u64::try_from).transpose()?;
        if completed_revision.is_some_and(|revision| {
            revision == 0
                || revision > MAX_ISSUE_AUTOMATION_REVISION
                || (revision != intent.expected_revision
                    && revision != intent.expected_revision + 1)
        }) {
            bail!("invalid autonomy command receipt");
        }
        Ok(StoredIssueAutonomyCommand {
            intent,
            completed_revision,
        })
    })
    .transpose()
}
pub(super) fn read(connection: &Connection, workspace: &str) -> Result<IssueAutonomySelection> {
    let row: Option<(Option<String>,i64,Option<String>)> = connection.query_row(
        "SELECT requested_level,revision,pending_command_id FROM issue_autonomy_selection WHERE workspace_dir=?1",
        [workspace], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((level, revision, pending)) = row else {
        return Ok(Default::default());
    };
    let requested_level = level
        .map(|level| serde_json::from_value(serde_json::Value::String(level)))
        .transpose()?;
    let revision = u64::try_from(revision)?;
    if (revision == 0) != requested_level.is_none() || revision > MAX_ISSUE_AUTOMATION_REVISION {
        bail!("invalid autonomy selection revision");
    }
    let pending = pending
        .map(|id| {
            let stored = command(connection, workspace, &id)?
                .ok_or_else(|| anyhow!("autonomy pending command is missing"))?;
            if stored.completed_revision.is_some() || stored.intent.expected_revision != revision {
                bail!("invalid pending autonomy intent");
            }
            Ok(stored.intent)
        })
        .transpose()?;
    Ok(IssueAutonomySelection {
        requested_level,
        revision,
        pending,
    })
}
pub(super) fn permits_assessment(
    connection: &Connection,
    workspace: &str,
    revision: u64,
) -> Result<bool> {
    let current = read(connection, workspace)?;
    Ok(current.pending.is_none() && current.revision == revision)
}
pub(super) fn begin(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueAutonomyIntent,
) -> Result<StoredIssueAutonomyCommand> {
    intent.validate().map_err(|reason| anyhow!(reason))?;
    adapter.with_connection(workspace,|connection,guard|transaction(connection,guard,|tx|{
        if let Some(stored)=command(tx,workspace,&intent.command_id)? {
            if stored.intent!=*intent { bail!("autonomy command ID was reused"); }
            return Ok(stored);
        }
        let current=read(tx,workspace)?;
        if current.pending.is_some() || current.revision!=intent.expected_revision { bail!("autonomy selection changed; refresh before trying again"); }
        if current.requested_level == Some(intent.level) {
            tx.execute("INSERT INTO issue_autonomy_commands(workspace_dir,command_id,intent,completed_revision) VALUES (?1,?2,?3,?4)",
                params![workspace,intent.command_id,serde_json::to_string(intent)?,i64::try_from(current.revision)?])?;
            return Ok(StoredIssueAutonomyCommand{intent:intent.clone(),completed_revision:Some(current.revision)});
        }
        tx.execute("INSERT INTO issue_autonomy_commands(workspace_dir,command_id,intent) VALUES (?1,?2,?3)",
            params![workspace,intent.command_id,serde_json::to_string(intent)?])?;
        tx.execute("INSERT INTO issue_autonomy_selection(workspace_dir,revision,pending_command_id) VALUES (?1,0,?2)
            ON CONFLICT(workspace_dir) DO UPDATE SET pending_command_id=excluded.pending_command_id",
            params![workspace,intent.command_id])?;
        super::triage::bump_catalog(tx,workspace)?;
        Ok(StoredIssueAutonomyCommand{intent:intent.clone(),completed_revision:None})
    }))
}
pub(super) fn claim_fence(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueAutonomyIntent,
    work: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint,
) -> Result<crate::domain::issue_automation::intervention::StoredIssueIntervention> {
    fences::claim(adapter, workspace, intent, work)
}

pub(super) fn finish(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueAutonomyIntent,
) -> Result<IssueAutonomyCommit> {
    adapter.with_connection(workspace,|connection,guard|transaction(connection,guard,|tx|{
        let stored=command(tx,workspace,&intent.command_id)?.ok_or_else(||anyhow!("autonomy intent is missing"))?;
        if stored.intent!=*intent { bail!("autonomy command ID was reused"); }
        if stored.completed_revision.is_some() { return Ok(IssueAutonomyCommit{command:stored,changed:false}); }
        let current=read(tx,workspace)?;
        if current.pending.as_ref()!=Some(intent) || current.revision!=intent.expected_revision { bail!("autonomy completion lost its revision fence"); }
        fences::require_completed(tx,workspace,intent)?;
        let revision=next_revision(current.revision)?;
        tx.execute("UPDATE issue_autonomy_selection SET requested_level=?2,revision=?3,pending_command_id=NULL WHERE workspace_dir=?1",
            params![workspace,super::enum_name(&intent.level)?,i64::try_from(revision)?])?;
        tx.execute("UPDATE issue_autonomy_commands SET completed_revision=?3 WHERE workspace_dir=?1 AND command_id=?2",
            params![workspace,intent.command_id,i64::try_from(revision)?])?;
        super::triage::bump_catalog(tx,workspace)?;
        Ok(IssueAutonomyCommit{command:StoredIssueAutonomyCommand{intent:intent.clone(),completed_revision:Some(revision)},changed:true})
    }))
}

#[cfg(test)]
mod tests;
