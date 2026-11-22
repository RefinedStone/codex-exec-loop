//! 결정 요청은 실행 권한을 만들지 않는다. 현재 관찰과 첫 답변을 한 장부 transaction으로 검사한다.
use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, Row, TransactionBehavior, params};

use crate::application::port::outbound::issue_automation_store_port::{
    IssueDecisionCommit, IssueDecisionFence, IssueDecisionSaveResult, IssueTriageFence,
};
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::decision::{
    IssueDecisionBasis, IssueDecisionId, IssueDecisionRequest, IssueDecisionState,
    PersistedIssueDecisionRequest, StoredIssueDecision,
};
use crate::domain::issue_automation::triage::IssueTriageAssessment;
use crate::domain::issue_work::IssueReference;

use super::collection::{queries, read_control};
use super::triage::{bump_catalog, catalog_revision};
use super::{
    SqliteIssueAutomationStoreAdapter, enum_name, next_revision, read_revision, sql_revision,
    transaction, validate_expected_revision, validate_reference_scope,
};

pub(super) const TABLE: &str = "issue_decision_requests";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_decision_requests (
    decision_id TEXT COLLATE BINARY NOT NULL,
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    binding_id TEXT COLLATE BINARY NOT NULL,
    uniqueness_key TEXT COLLATE BINARY NOT NULL CHECK (length(uniqueness_key) = 64),
    state TEXT NOT NULL CHECK (state IN ('open', 'answered', 'expired', 'withdrawn', 'superseded')),
    request TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (workspace_dir, decision_id),
    UNIQUE (workspace_dir, uniqueness_key),
    FOREIGN KEY (binding_id, provider, server_id, repository_id, workspace_dir)
        REFERENCES issue_automation_bindings
            (binding_id, provider, server_id, repository_id, workspace_dir)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

const MAX_REQUEST_BYTES: usize = 512 * 1024;
const COLUMNS: &str = "decision_id, workspace_dir, provider, server_id, repository_id, issue_id, uniqueness_key, state, request, revision";

pub(super) fn get(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    id: &IssueDecisionId,
) -> Result<Option<StoredIssueDecision>> {
    adapter.with_connection(workspace, |connection, _guard| {
        read(connection, workspace, id)
    })
}

pub(super) fn get_commit(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    id: &IssueDecisionId,
) -> Result<Option<IssueDecisionCommit>> {
    adapter.with_connection(workspace, |connection, guard| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let record = read(&transaction, workspace, id)?;
        let catalog_revision = catalog_revision(&transaction, workspace)?;
        guard.verify_identity()?;
        transaction.commit()?;
        Ok(record.map(|record| IssueDecisionCommit {
            record,
            catalog_revision,
            changed: false,
        }))
    })
}

pub(super) fn read(
    connection: &Connection,
    workspace: &str,
    id: &IssueDecisionId,
) -> Result<Option<StoredIssueDecision>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_decision_requests WHERE workspace_dir = ?1 AND decision_id = ?2"
    ))?;
    let mut rows = statement.query(params![workspace, id.as_str()])?;
    rows.next()?.map(hydrate).transpose()
}

fn read_unique(
    connection: &Connection,
    workspace: &str,
    uniqueness_key: &str,
) -> Result<Option<StoredIssueDecision>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_decision_requests WHERE workspace_dir = ?1 AND uniqueness_key = ?2"
    ))?;
    let mut rows = statement.query(params![workspace, uniqueness_key])?;
    rows.next()?.map(hydrate).transpose()
}

fn hydrate(row: &Row<'_>) -> Result<StoredIssueDecision> {
    let encoded: String = row.get(8)?;
    if encoded.len() > MAX_REQUEST_BYTES {
        bail!("the stored decision exceeds its storage bound")
    }
    let persisted: PersistedIssueDecisionRequest =
        serde_json::from_str(&encoded).map_err(|_| anyhow!("the stored decision is malformed"))?;
    let request = IssueDecisionRequest::hydrate(persisted)
        .map_err(|_| anyhow!("the stored decision is invalid"))?;
    let reference = request.issue();
    if row.get::<_, String>(0)? != request.id().as_str()
        || row.get::<_, String>(1)? != reference.project_scope.workspace_dir
        || row.get::<_, String>(2)? != enum_name(&reference.provider)?
        || row.get::<_, String>(3)? != reference.server_id
        || row.get::<_, String>(4)? != reference.repository_id
        || row.get::<_, String>(5)? != reference.issue_id
        || row.get::<_, String>(6)? != request.uniqueness_key()
        || row.get::<_, String>(7)? != enum_name(&request.state())?
    {
        bail!("the stored decision identity does not match its validated record")
    }
    Ok(StoredIssueDecision {
        request,
        revision: read_revision(row, 9)?,
    })
}

/// collection 조회와 동일한 읽기 transaction을 사용한다.
pub(super) fn open_for_issue(
    connection: &Connection,
    workspace: &str,
    reference: &IssueReference,
) -> Result<Vec<StoredIssueDecision>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM issue_decision_requests
         WHERE workspace_dir = ?1 AND provider = ?2 AND server_id = ?3 AND repository_id = ?4
           AND issue_id = ?5 AND state = 'open' ORDER BY decision_id"
    ))?;
    let mut rows = statement.query(params![
        workspace,
        enum_name(&reference.provider)?,
        reference.server_id,
        reference.repository_id,
        reference.issue_id,
    ])?;
    let mut requests = Vec::new();
    while let Some(row) = rows.next()? {
        let stored = hydrate(row)?;
        if stored.request.issue() != reference || stored.request.state() != IssueDecisionState::Open
        {
            bail!("the open decision query returned an inconsistent record")
        }
        requests.push(stored);
    }
    Ok(requests)
}

/// 호출자는 질문 좌표 변경을 고정하는 실행 관측 transaction을 사용한다.
/// catalog는 해당 실행 관측의 변경 및 outbox와 함께 한 번만 증가시킨다.
pub(super) fn supersede_work_questions(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
    work_order_id: &str,
) -> Result<bool> {
    use crate::domain::issue_automation::decision::IssueDecisionKind;
    let mut changed = false;
    for existing in open_for_issue(connection, workspace, issue)? {
        let definition = existing.request.definition();
        if definition.kind != IssueDecisionKind::AgentQuestion
            || definition.basis.work_order_id.as_deref() != Some(work_order_id)
        {
            continue;
        }
        let mut next = existing.request.to_record();
        next.state = IssueDecisionState::Superseded;
        let next = IssueDecisionRequest::hydrate(next)?;
        next.validate_successor(&existing.request)?;
        let revision = next_revision(existing.revision)?;
        let affected = connection.execute(
            "UPDATE issue_decision_requests SET state='superseded', request=?1, revision=?2
             WHERE workspace_dir=?3 AND decision_id=?4 AND revision=?5 AND state='open'",
            params![
                encode(&next)?,
                sql_revision(revision)?,
                workspace,
                next.id().as_str(),
                sql_revision(existing.revision)?,
            ],
        )?;
        if affected != 1 {
            bail!("the worker question changed during its execution observation");
        }
        changed = true;
    }
    Ok(changed)
}

pub(super) fn ensure(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueTriageFence,
    assessment: &IssueTriageAssessment,
    request: &IssueDecisionRequest,
) -> Result<IssueDecisionSaveResult> {
    validate_reference_scope(workspace, request.issue())?;
    if request.state() != IssueDecisionState::Open {
        bail!("only a new open decision may be inserted")
    }
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            if let Some(existing) = read(transaction, workspace, request.id())? {
                if existing.request.definition() != request.definition() {
                    bail!("the decision ID already belongs to a different request")
                }
                return saved(existing, catalog_revision(transaction, workspace)?, false);
            }
            // Polling the same immutable definition with a new ID reuses even a terminal request.
            if let Some(existing) = read_unique(transaction, workspace, &request.uniqueness_key())?
            {
                return saved(existing, catalog_revision(transaction, workspace)?, false);
            }
            let Some(item) = current_item(transaction, workspace, expected, assessment)? else {
                return Ok(IssueDecisionSaveResult::Conflict);
            };
            // A durable triage supplies the previous gate revision for the next evaluation.
            // Without it metadata-only changes could repeatedly start at revision one.
            if item
                .triage
                .as_ref()
                .is_none_or(|triage| triage.assessment != *assessment)
            {
                return Ok(IssueDecisionSaveResult::Conflict);
            }
            if request.definition().basis
                != current_basis(transaction, workspace, request, assessment)?
                || !work_context_current(transaction, workspace, request)?
            {
                return Ok(IssueDecisionSaveResult::Conflict);
            }
            let encoded = encode(request)?;
            let reference = request.issue();
            transaction.execute(
                "INSERT INTO issue_decision_requests
                    (decision_id, workspace_dir, provider, server_id, repository_id, issue_id,
                     binding_id, uniqueness_key, state, request, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9, 1)",
                params![
                    request.id().as_str(),
                    workspace,
                    enum_name(&reference.provider)?,
                    reference.server_id,
                    reference.repository_id,
                    reference.issue_id,
                    item.binding_id.as_str(),
                    request.uniqueness_key(),
                    encoded,
                ],
            )?;
            let revision = bump_catalog(transaction, workspace)?;
            saved(
                StoredIssueDecision {
                    request: request.clone(),
                    revision: 1,
                },
                revision,
                true,
            )
        })
    })
}

pub(super) fn save(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: IssueDecisionFence,
    assessment: &IssueTriageAssessment,
    request: &IssueDecisionRequest,
) -> Result<IssueDecisionSaveResult> {
    validate_reference_scope(workspace, request.issue())?;
    validate_expected_revision(expected.decision_revision)?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |transaction| {
            let Some(existing) = read(transaction, workspace, request.id())? else {
                return Ok(IssueDecisionSaveResult::Conflict);
            };
            // Every later writer receives the first terminal record, without a second event.
            if existing.request.state() != IssueDecisionState::Open {
                return saved(existing, catalog_revision(transaction, workspace)?, false);
            }
            if existing.revision != expected.decision_revision {
                return Ok(IssueDecisionSaveResult::Conflict);
            }
            let Some(_item) = current_item(transaction, workspace, expected.triage, assessment)? else {
                return Ok(IssueDecisionSaveResult::Conflict);
            };
            request.validate_successor(&existing.request)?;
            let basis_matches = existing.request.definition().basis == current_basis(transaction, workspace, request, assessment)?
                && work_context_current(transaction, workspace, request)?;
            if basis_matches && (assessment.gates().iter().any(|gate|
                gate.gate == existing.request.definition().gate
                    && gate.action == crate::domain::issue_automation::policy::IssueAutomationGateAction::Human)
                || (request.definition().basis.work_order_id.is_some()
                    && matches!(
                        request.definition().kind,
                        crate::domain::issue_automation::decision::IssueDecisionKind::AgentQuestion
                            | crate::domain::issue_automation::decision::IssueDecisionKind::FailureRecovery
                    )))
                && request.answer().is_none_or(|answer|
                    answer.by != crate::domain::issue_automation::decision::IssueDecisionAnswerBy::Human
                        || answer.confirmation.is_none())
            {
                bail!("the current issue gate requires a verified human answer")
            }
            if basis_matches
                && request.definition().basis.work_order_id.is_some()
                && request.definition().kind == crate::domain::issue_automation::decision::IssueDecisionKind::AgentQuestion
                && request.answer().is_some_and(|answer| answer.option == "reply" && answer.note.is_none())
            {
                bail!("the worker question requires a shared answer note");
            }
            let expected_state = if basis_matches {
                IssueDecisionState::Answered
            } else {
                IssueDecisionState::Superseded
            };
            if request.state() != expected_state {
                bail!("the decision resolution does not match the current basis")
            }
            let revision = next_revision(existing.revision)?;
            let changed = transaction.execute(
                "UPDATE issue_decision_requests SET state = ?1, request = ?2, revision = ?3
                 WHERE workspace_dir = ?4 AND decision_id = ?5 AND revision = ?6 AND state = 'open'",
                params![
                    enum_name(&request.state())?, encode(request)?, sql_revision(revision)?,
                    workspace, request.id().as_str(), sql_revision(expected.decision_revision)?,
                ],
            )?;
            if changed != 1 {
                bail!("the decision changed during its transaction")
            }
            let catalog = bump_catalog(transaction, workspace)?;
            saved(StoredIssueDecision { request: request.clone(), revision }, catalog, true)
        })
    })
}

pub(super) fn current_item(
    connection: &Connection,
    workspace: &str,
    expected: IssueTriageFence,
    assessment: &IssueTriageAssessment,
) -> Result<Option<IssueCollectionItem>> {
    if !super::autonomy::permits_assessment(
        connection,
        workspace,
        assessment.basis().runtime_selection_revision,
    )? {
        return Ok(None);
    }
    validate_expected_revision(expected.collection_epoch)?;
    for revision in [
        expected.observation_revision,
        expected.tracked_revision,
        expected.assessment_revision,
    ]
    .into_iter()
    .flatten()
    {
        validate_expected_revision(revision)?;
    }
    let Some(control) = read_control(connection, workspace)? else {
        return Ok(None);
    };
    // An operator may answer while collection is paused. Epoch is still a CAS boundary.
    if control.epoch != expected.collection_epoch {
        return Ok(None);
    }
    let reference = &assessment.basis().issue;
    validate_reference_scope(workspace, reference)?;
    let Some(item) = queries::item(connection, workspace, reference)? else {
        return Ok(None);
    };
    if item.observation_revision != expected.observation_revision
        || item.tracked_revision != expected.tracked_revision
        || item.triage.as_ref().map(|triage| triage.revision) != expected.assessment_revision
    {
        return Ok(None);
    }
    assessment.validate_for(reference)?;
    assessment.validate_current(
        reference,
        item.current.as_ref(),
        item.tracked
            .as_ref()
            .is_some_and(|tracked| tracked.flags.held),
    )?;
    assessment.validate_successor(item.triage.as_ref().map(|triage| &triage.assessment))?;
    Ok(Some(item))
}

fn current_basis(
    connection: &Connection,
    workspace: &str,
    request: &IssueDecisionRequest,
    assessment: &IssueTriageAssessment,
) -> Result<IssueDecisionBasis> {
    if request.issue() != &assessment.basis().issue {
        bail!("the current assessment belongs to a different decision subject")
    }
    let mut basis = IssueDecisionBasis::from_assessment(
        assessment,
        request.definition().gate,
        request.definition().basis.work_order_id.clone(),
    )?;
    basis.authorization_epoch =
        super::intervention::authorization_epoch(connection, workspace, request.issue())?;
    Ok(basis)
}

pub(super) fn work_context_current(
    connection: &Connection,
    workspace: &str,
    request: &IssueDecisionRequest,
) -> Result<bool> {
    use crate::domain::issue_automation::decision::IssueDecisionKind;
    let definition = request.definition();
    let Some(work_order_id) = &definition.basis.work_order_id else {
        return Ok(true);
    };
    let Some(work) = super::coordinator::read(connection, workspace, request.issue())? else {
        return Ok(false);
    };
    if &work.order.work_order_id != work_order_id {
        return Ok(false);
    }
    let Some(execution) = super::execution::read(connection, workspace, work_order_id)? else {
        return Ok(false);
    };
    Ok(match definition.kind {
        IssueDecisionKind::AgentQuestion => {
            work.current_authorization_basis().authorization_epoch
                == definition.basis.authorization_epoch
                && definition.basis.field_hash.as_ref() == Some(&work.order.field_hash)
                && execution.question.as_ref().is_some_and(|question| {
                    definition.evidence == [format!("question:{}", question.question_id)]
                })
        }
        IssueDecisionKind::FailureRecovery => {
            execution
                .progress
                .terminal
                .as_ref()
                .is_some_and(|terminal| {
                    definition.evidence == [format!("terminal:{}", terminal.record_id)]
                })
        }
        _ => false,
    })
}

fn encode(request: &IssueDecisionRequest) -> Result<String> {
    let encoded = serde_json::to_string(&request.to_record())
        .context("the shared decision could not be encoded")?;
    if encoded.len() > MAX_REQUEST_BYTES {
        bail!("the shared decision exceeds its storage bound")
    }
    Ok(encoded)
}

fn saved(
    record: StoredIssueDecision,
    catalog_revision: u64,
    changed: bool,
) -> Result<IssueDecisionSaveResult> {
    Ok(IssueDecisionSaveResult::Saved(Box::new(
        IssueDecisionCommit {
            record,
            catalog_revision,
            changed,
        },
    )))
}

#[cfg(test)]
mod tests;

/// 최신 인가/관문 판 순으로 제한된 종료 카드만 반환한다. 원문 공개 판단은 service가 한다.
pub(super) fn closed(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    issue: &IssueReference,
    limit: u16,
) -> Result<Vec<StoredIssueDecision>> {
    validate_reference_scope(workspace, issue)?;
    if limit == 0 || limit > 20 {
        bail!("invalid decision history bound");
    }
    adapter.with_connection(workspace,|connection,_|{
        let mut statement=connection.prepare(&format!(
            "SELECT {COLUMNS} FROM issue_decision_requests WHERE workspace_dir=?1 AND provider=?2 AND server_id=?3 AND repository_id=?4 AND issue_id=?5 AND state!='open'
             ORDER BY COALESCE(json_extract(request,'$.definition.basis.authorizationEpoch'),0) DESC,
             COALESCE(json_extract(request,'$.definition.basis.gateRevision'),0) DESC, revision DESC, decision_id LIMIT ?6"))?;
        let mut rows=statement.query(params![workspace,enum_name(&issue.provider)?,issue.server_id,issue.repository_id,issue.issue_id,limit])?;
        let mut result=Vec::new();
        while let Some(row)=rows.next()? { result.push(hydrate(row)?); }
        Ok(result)
    })
}
