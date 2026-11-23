//! 1이슈 1작업 연결의 영속 CAS다. 실행기 DB와 transaction을 공유하지 않는다.
use super::{
    SqliteIssueAutomationStoreAdapter, enum_name, next_revision, sql_revision, transaction,
    validate_reference_scope,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueTriageFence, IssueWorkSaveResult,
};
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::coordinator::{
    IssueWorkBlockReason, IssueWorkCheckpoint, IssueWorkStartAuthority, human_start, policy_start,
    ready_for_start, timeout_arbiter_start, timeout_default_start,
};
use crate::domain::issue_automation::decision::{IssueDecisionBasis, IssueDecisionId};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::triage::IssueTriageAssessment;
use crate::domain::issue_work::IssueReference;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, Transaction, params};

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
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            if !validate_current(tx, workspace, fence, assessment, candidate)? {
                return Ok(IssueWorkSaveResult::Conflict);
            }
            let Some(current) = read(tx, workspace, &candidate.order.issue)? else {
                bail!("a new issue work checkpoint requires budget admission");
            };
            validate_identity(&current, candidate)?;
            if current.authorization != candidate.authorization {
                let Some(authorization) = &candidate.authorization else {
                    bail!("work authorization cannot be removed");
                };
                if authorization.basis.authorization_epoch
                    <= current.current_authorization_basis().authorization_epoch
                {
                    bail!("renewed work authorization requires a newer intervention epoch");
                }
                validate_authorization(
                    tx,
                    workspace,
                    assessment,
                    &candidate.order.issue,
                    authorization.start_authority,
                    authorization.decision_id.as_ref(),
                    &authorization.basis,
                )?;
                validate_handoff(tx, workspace, candidate)?;
            }
            if current.revision != candidate.revision {
                return Ok(IssueWorkSaveResult::Conflict);
            }
            if !validate_delivery_change(tx, workspace, assessment, &current, candidate)? {
                return Ok(IssueWorkSaveResult::Conflict);
            }
            if current == *candidate {
                return Ok(IssueWorkSaveResult::Saved {
                    record: Box::new(current),
                    catalog_revision: super::triage::catalog_revision(tx, workspace)?,
                    changed: false,
                });
            }
            if (!current.task_ids.is_empty() && current.task_ids != candidate.task_ids)
                || candidate.permit_revision < current.permit_revision
            {
                bail!("an issue work checkpoint cannot move backwards");
            }
            persist(tx, workspace, Some(&current), candidate)
        })
    })
}

/// 권한을 늘릴 candidate를 받지 않고 최신 저장 work에서 취소만 구성한다.
pub(super) fn clear_delivery(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    issue: &IssueReference,
    expected_work_revision: u64,
    reason: IssueWorkBlockReason,
) -> Result<IssueWorkSaveResult> {
    validate_reference_scope(workspace, issue)?;
    super::validate_expected_revision(expected_work_revision)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let Some(current) = read(tx, workspace, issue)? else {
                return Ok(IssueWorkSaveResult::Conflict);
            };
            if current.revision != expected_work_revision {
                return Ok(IssueWorkSaveResult::Conflict);
            }
            let mut candidate = current.clone();
            candidate.delivery_assessment = None;
            candidate.delivery_granted = false;
            candidate.blocked_reason = Some(reason);
            if candidate == current {
                return Ok(IssueWorkSaveResult::Saved {
                    record: Box::new(current),
                    catalog_revision: super::triage::catalog_revision(tx, workspace)?,
                    changed: false,
                });
            }
            candidate.validate().map_err(|reason| anyhow!(reason))?;
            persist(tx, workspace, Some(&current), &candidate)
        })
    })
}

fn validate_delivery_change(
    tx: &Connection,
    workspace: &str,
    assessment: &IssueTriageAssessment,
    current: &IssueWorkCheckpoint,
    candidate: &IssueWorkCheckpoint,
) -> Result<bool> {
    let changed = current.delivery_assessment != candidate.delivery_assessment;
    if candidate.delivery_revision < current.delivery_revision {
        bail!("the delivery assessment revision cannot move backwards");
    }
    if changed && candidate.delivery_granted {
        bail!("a changed delivery assessment requires suspending its permit first");
    }
    let Some(delivery) = &candidate.delivery_assessment else {
        if candidate.delivery_revision != current.delivery_revision {
            bail!("clearing a delivery assessment must preserve its revision");
        }
        if candidate.delivery_granted
            && (!current.delivery_granted
                || candidate.permit_source != current.permit_source
                || candidate.permit_revision != current.permit_revision
                || candidate.permit_authorization_epoch != current.permit_authorization_epoch)
        {
            bail!("a new delivery permit requires a current diff assessment");
        }
        return Ok(true);
    };
    if changed || candidate.delivery_granted {
        let epoch =
            super::intervention::authorization_epoch(tx, workspace, &candidate.order.issue)?;
        if !delivery.matches_work(candidate) || !delivery.matches_triage(assessment, epoch) {
            return Ok(false);
        }
    }
    if changed {
        let floor = current
            .delivery_revision
            .max(current.permit_revision)
            .max(delivery.triage_gate_revision);
        if delivery.basis.gate_revision <= floor {
            bail!("a changed delivery assessment requires a newer gate revision");
        }
    }
    if candidate.delivery_granted {
        let range = delivery
            .basis
            .delivery
            .as_ref()
            .expect("validated delivery basis");
        if candidate.permit_source.as_deref() != Some(&range.source_commit_sha)
            || candidate.permit_revision < delivery.basis.gate_revision
            || candidate.permit_authorization_epoch != delivery.basis.authorization_epoch
        {
            bail!("the delivery permit does not match its assessed source and revision");
        }
    }
    Ok(true)
}

pub(super) fn validate_current(
    tx: &Connection,
    workspace: &str,
    fence: IssueTriageFence,
    assessment: &IssueTriageAssessment,
    candidate: &IssueWorkCheckpoint,
) -> Result<bool> {
    let Some(control) = super::collection::read_control(tx, workspace)? else {
        return Ok(false);
    };
    if control.config.mode != IssueCollectionMode::Observe
        || assessment.basis().issue != candidate.order.issue
    {
        return Ok(false);
    }
    let Some(item) = super::decision::current_item(tx, workspace, fence, assessment)? else {
        return Ok(false);
    };
    // Automatic starts have no human decision whose insertion could supply this fence.
    // Require the exact durable assessment for every start authority.
    Ok(!item.binding_run_blocked
        && item
            .triage
            .as_ref()
            .is_some_and(|triage| triage.assessment == *assessment)
        && !super::intervention::has_pending(tx, workspace, &candidate.order.issue)?
        && super::intervention::is_tracked(tx, workspace, &candidate.order.issue)?)
}

pub(super) fn validate_identity(
    current: &IssueWorkCheckpoint,
    candidate: &IssueWorkCheckpoint,
) -> Result<()> {
    if current.order != candidate.order
        || current.start_authority != candidate.start_authority
        || current.decision_id != candidate.decision_id
        || current.approved_basis != candidate.approved_basis
    {
        bail!("a registered issue work identity cannot change");
    }
    Ok(())
}

pub(super) fn validate_authorization(
    tx: &Connection,
    workspace: &str,
    assessment: &IssueTriageAssessment,
    issue: &IssueReference,
    authority: IssueWorkStartAuthority,
    decision_id: Option<&IssueDecisionId>,
    approved: &IssueDecisionBasis,
) -> Result<()> {
    authority
        .validate(decision_id)
        .map_err(|reason| anyhow!(reason))?;
    let mut basis =
        IssueDecisionBasis::from_assessment(assessment, IssueAutomationGate::Start, None)?;
    basis.authorization_epoch = super::intervention::authorization_epoch(tx, workspace, issue)?;
    if basis != *approved || !ready_for_start(assessment) {
        bail!("the issue work requires a current start authorization");
    }
    let tracked = super::read_issue(tx, workspace, issue)?
        .ok_or_else(|| anyhow!("the issue work is not tracked"))?;
    if tracked.flags().held
        || tracked.flags().stale
        || tracked.flags().unknown
        || matches!(
            tracked.state(),
            crate::domain::issue_automation::IssueAutomationState::Done
                | crate::domain::issue_automation::IssueAutomationState::Failed
                | crate::domain::issue_automation::IssueAutomationState::Cancelled
        )
    {
        bail!("the issue work is not eligible for start authorization");
    }
    match authority {
        IssueWorkStartAuthority::Human => {
            let decision = super::decision::read(
                tx,
                workspace,
                decision_id.expect("validated human authority"),
            )?
            .ok_or_else(|| anyhow!("the issue work approval was not found"))?;
            if !human_start(&decision.request)
                || decision.request.issue() != issue
                || decision.request.definition().basis != *approved
            {
                bail!("the issue work does not have a current human start approval");
            }
        }
        IssueWorkStartAuthority::Policy => {
            if !policy_start(assessment) {
                bail!("the issue work requires a current automatic start gate");
            }
            validate_automatic_selection(tx, workspace, assessment)?;
        }
        IssueWorkStartAuthority::TimeoutDefault => {
            let decision = super::decision::read(
                tx,
                workspace,
                decision_id.expect("validated timeout-default authority"),
            )?
            .ok_or_else(|| anyhow!("the issue work default start decision was not found"))?;
            if !timeout_default_start(assessment, &decision.request)
                || decision.request.issue() != issue
                || decision.request.definition().basis != *approved
            {
                bail!("the issue work does not have a current default start decision");
            }
            validate_automatic_selection(tx, workspace, assessment)?;
        }
        IssueWorkStartAuthority::TimeoutArbiter => {
            let decision = super::decision::read(
                tx,
                workspace,
                decision_id.expect("validated timeout-arbiter authority"),
            )?
            .ok_or_else(|| anyhow!("the issue work arbiter start decision was not found"))?;
            if !timeout_arbiter_start(assessment, &decision.request)
                || decision.request.issue() != issue
                || decision.request.definition().basis != *approved
                || !super::arbiter::applied_decision_current(tx, workspace, &decision)?
            {
                bail!("the issue work does not have a current arbiter start decision");
            }
            validate_automatic_selection(tx, workspace, assessment)?;
        }
    }
    Ok(())
}

fn validate_automatic_selection(
    tx: &Connection,
    workspace: &str,
    assessment: &IssueTriageAssessment,
) -> Result<()> {
    let selection = super::autonomy::read(tx, workspace)?;
    if !super::autonomy::matches_automatic_mode(&selection, assessment.effective_mode())
        || selection.pending.is_some()
        || selection.revision != assessment.basis().runtime_selection_revision
    {
        bail!("the issue work requires an explicit current automatic selection");
    }
    Ok(())
}

pub(super) fn validate_handoff(
    tx: &Connection,
    workspace: &str,
    candidate: &IssueWorkCheckpoint,
) -> Result<()> {
    let item = super::collection::queries::item(tx, workspace, &candidate.order.issue)?
        .ok_or_else(|| anyhow!("the issue work source is missing"))?;
    let Some(crate::domain::issue_automation::source::IssueSourceRead::Observed(observation)) =
        item.current
    else {
        bail!("the issue work source is unknown");
    };
    let crate::domain::issue_automation::source::IssueSourceContent::Shared(snapshot) =
        observation.content
    else {
        bail!("the issue work handoff is withheld");
    };
    if snapshot.shared_copy != candidate.order.handoff
        || snapshot.basis.field_hash != candidate.order.field_hash
        || crate::domain::issue_automation::triage::issue_acceptance_criteria(
            &snapshot.shared_copy.body,
        )
        .map_err(|reason| anyhow!(reason))?
            != candidate.order.acceptance
    {
        bail!("the issue work does not contain the current authorized shared handoff");
    }
    Ok(())
}

pub(super) fn persist(
    tx: &Transaction<'_>,
    workspace: &str,
    existing: Option<&IssueWorkCheckpoint>,
    candidate: &IssueWorkCheckpoint,
) -> Result<IssueWorkSaveResult> {
    let mut record = candidate.clone();
    record.revision = next_revision(candidate.revision)?;
    let encoded = serde_json::to_string(&record)?;
    if encoded.len() > 512 * 1024 {
        bail!("the issue work checkpoint exceeds its bound");
    }
    tx.execute("INSERT INTO issue_work_checkpoints
        (workspace_dir, provider, server_id, repository_id, issue_id, work_order_id, checkpoint, revision)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
        ON CONFLICT(workspace_dir, provider, server_id, repository_id, issue_id)
        DO UPDATE SET checkpoint=excluded.checkpoint, revision=excluded.revision",
        params![workspace, enum_name(&record.order.issue.provider)?, record.order.issue.server_id,
            record.order.issue.repository_id, record.order.issue.issue_id, record.order.work_order_id, encoded, sql_revision(record.revision)?])?;
    let catalog_revision = super::triage::bump_catalog(tx, workspace)?;
    super::events::append(tx, workspace, existing, &record, catalog_revision)?;
    Ok(IssueWorkSaveResult::Saved {
        record: Box::new(record),
        catalog_revision,
        changed: true,
    })
}

pub(super) fn list(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
) -> Result<Vec<IssueWorkCheckpoint>> {
    adapter.with_connection(workspace, |connection,_| {
        let mut statement=connection.prepare("SELECT checkpoint, revision, work_order_id, provider, server_id, repository_id, issue_id FROM issue_work_checkpoints WHERE workspace_dir=?1 ORDER BY work_order_id")?;
        let mut rows=statement.query([workspace])?;
        let mut result=Vec::new();
        while let Some(row)=rows.next()? {
            let encoded:String=row.get(0)?;
            if encoded.len()>512*1024 { bail!("the issue work checkpoint exceeds its bound"); }
            let record:IssueWorkCheckpoint=serde_json::from_str(&encoded).map_err(|_|anyhow!("invalid issue work checkpoint"))?;
            record.validate().map_err(|reason|anyhow!(reason))?;
            validate_reference_scope(workspace,&record.order.issue)?;
            if record.revision!=super::read_revision(row,1)?
                || record.order.work_order_id!=row.get::<_,String>(2)?
                || enum_name(&record.order.issue.provider)?!=row.get::<_,String>(3)?
                || record.order.issue.server_id!=row.get::<_,String>(4)?
                || record.order.issue.repository_id!=row.get::<_,String>(5)?
                || record.order.issue.issue_id!=row.get::<_,String>(6)? {
                bail!("the issue work checkpoint identity does not match");
            }
            result.push(record);
        }
        Ok(result)
    })
}
