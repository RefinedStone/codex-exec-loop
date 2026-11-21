//! 1이슈 1작업 연결의 영속 CAS다. 실행기 DB와 transaction을 공유하지 않는다.
use super::{
    SqliteIssueAutomationStoreAdapter, enum_name, next_revision, sql_revision, transaction,
    validate_reference_scope,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueTriageFence, IssueWorkSaveResult,
};
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::coordinator::{IssueWorkCheckpoint, human_start};
use crate::domain::issue_automation::triage::IssueTriageAssessment;
use crate::domain::issue_work::IssueReference;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, params};

pub(super) const TABLE: &str = "issue_work_checkpoints";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_work_checkpoints (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    work_order_id TEXT COLLATE BINARY NOT NULL,
    checkpoint TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (workspace_dir, provider, server_id, repository_id, issue_id),
    UNIQUE (workspace_dir, work_order_id)
) STRICT, WITHOUT ROWID
"#;

pub(super) fn get(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    issue: &IssueReference,
) -> Result<Option<IssueWorkCheckpoint>> {
    validate_reference_scope(workspace, issue)?;
    adapter.with_connection(workspace, |connection, _| {
        read(connection, workspace, issue)
    })
}

pub(super) fn read(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
) -> Result<Option<IssueWorkCheckpoint>> {
    let mut statement = connection.prepare(
        "SELECT checkpoint, revision, work_order_id FROM issue_work_checkpoints
         WHERE workspace_dir=?1 AND provider=?2 AND server_id=?3 AND repository_id=?4 AND issue_id=?5")?;
    let mut rows = statement.query(params![
        workspace,
        enum_name(&issue.provider)?,
        issue.server_id,
        issue.repository_id,
        issue.issue_id
    ])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let encoded: String = row.get(0)?;
    if encoded.len() > 512 * 1024 {
        bail!("the issue work checkpoint exceeds its bound");
    }
    let record: IssueWorkCheckpoint =
        serde_json::from_str(&encoded).map_err(|_| anyhow!("invalid issue work checkpoint"))?;
    record.validate().map_err(|reason| anyhow!(reason))?;
    if record.order.issue != *issue
        || record.revision != super::read_revision(row, 1)?
        || record.order.work_order_id != row.get::<_, String>(2)?
    {
        bail!("the issue work checkpoint identity does not match");
    }
    Ok(Some(record))
}

pub(super) fn save(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    fence: IssueTriageFence,
    assessment: &IssueTriageAssessment,
    candidate: &IssueWorkCheckpoint,
) -> Result<IssueWorkSaveResult> {
    candidate.validate().map_err(|reason| anyhow!(reason))?;
    validate_reference_scope(workspace, &candidate.order.issue)?;
    adapter.with_connection(workspace, |connection, guard| transaction(connection, guard, |tx| {
        let Some(control) = super::collection::read_control(tx, workspace)? else { return Ok(IssueWorkSaveResult::Conflict) };
        if control.config.mode != IssueCollectionMode::Observe
            || assessment.basis().issue != candidate.order.issue
            || super::decision::current_item(tx, workspace, fence, assessment)?.is_none() {
            return Ok(IssueWorkSaveResult::Conflict);
        }
        let existing = read(tx, workspace, &candidate.order.issue)?;
        if let Some(current) = &existing {
            if current.order != candidate.order || current.decision_id != candidate.decision_id
                || current.approved_basis != candidate.approved_basis {
                bail!("a registered issue work identity cannot change");
            }
            if current.revision != candidate.revision { return Ok(IssueWorkSaveResult::Conflict); }
            if current == candidate {
                return Ok(IssueWorkSaveResult::Saved { record: Box::new(current.clone()),
                    catalog_revision: super::triage::catalog_revision(tx, workspace)?, changed: false });
            }
            if (!current.task_ids.is_empty() && current.task_ids != candidate.task_ids)
                || candidate.permit_revision < current.permit_revision {
                bail!("an issue work checkpoint cannot move backwards");
            }
        } else {
            if candidate.revision != 0 { return Ok(IssueWorkSaveResult::Conflict); }
            let Some(decision) = super::decision::read(tx, workspace, &candidate.decision_id)? else {
                bail!("the issue work approval was not found");
            };
            let basis = crate::domain::issue_automation::decision::IssueDecisionBasis::from_assessment(
                assessment, crate::domain::issue_automation::policy::IssueAutomationGate::Start, None)?;
            if !human_start(&decision.request) || decision.request.issue() != &candidate.order.issue
                || decision.request.definition().basis != candidate.approved_basis
                || basis != candidate.approved_basis {
                bail!("the issue work does not have a current human start approval");
            }
        }
        let mut record = candidate.clone();
        record.revision = next_revision(candidate.revision)?;
        let encoded = serde_json::to_string(&record)?;
        if encoded.len() > 512 * 1024 { bail!("the issue work checkpoint exceeds its bound"); }
        tx.execute("INSERT INTO issue_work_checkpoints
            (workspace_dir, provider, server_id, repository_id, issue_id, work_order_id, checkpoint, revision)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
            ON CONFLICT(workspace_dir, provider, server_id, repository_id, issue_id)
            DO UPDATE SET checkpoint=excluded.checkpoint, revision=excluded.revision",
            params![workspace, enum_name(&record.order.issue.provider)?, record.order.issue.server_id,
                record.order.issue.repository_id, record.order.issue.issue_id, record.order.work_order_id, encoded, sql_revision(record.revision)?])?;
        let catalog_revision = super::triage::bump_catalog(tx, workspace)?;
        Ok(IssueWorkSaveResult::Saved { record: Box::new(record), catalog_revision, changed: true })
    }))
}
