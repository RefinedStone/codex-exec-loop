use super::*;
use std::collections::BTreeSet;

pub(in super::super) fn unfinished(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    after_id: Option<&str>,
    limit: usize,
) -> Result<Vec<IssueArbiterAttempt>> {
    if after_id.is_some_and(|id| id.len() > 128) {
        bail!("the arbiter cursor exceeds its bound");
    }
    let limit = limit.min(MAX_ISSUE_ARBITER_BATCH);
    adapter.with_connection(workspace, |connection, guard| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let ids = {
            let mut statement = tx.prepare(
                "SELECT attempt_id FROM issue_arbiter_attempts WHERE workspace_dir=?1
                 AND phase='pending' AND (?2 IS NULL OR attempt_id>?2)
                 ORDER BY attempt_id LIMIT ?3",
            )?;
            statement
                .query_map(params![workspace, after_id, limit as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut attempts = Vec::with_capacity(ids.len());
        for id in ids {
            let attempt = storage::read_attempt(&tx, workspace, &id)?
                .ok_or_else(|| anyhow!("the selected arbiter attempt is missing"))?;
            if attempt.finished_event_id.is_some() {
                bail!("the pending arbiter attempt is finished");
            }
            attempts.push(attempt);
        }
        guard.verify_identity()?;
        tx.commit()?;
        Ok(attempts)
    })
}

pub(in super::super) fn pending(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    limit: usize,
) -> Result<Vec<IssueArbiterFinishedEvent>> {
    let limit = limit.min(MAX_ISSUE_ARBITER_BATCH);
    adapter.with_connection(workspace, |connection, guard| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let ids = {
            let mut statement = tx.prepare(
                "SELECT decision_id FROM issue_arbiter_finished_events WHERE workspace_dir=?1
                 AND acknowledged=0 ORDER BY event_id LIMIT ?2",
            )?;
            statement
                .query_map(params![workspace, limit as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut events = Vec::with_capacity(ids.len());
        for id in ids {
            let (_, commit) = storage::read_event(&tx, workspace, &IssueDecisionId::new(id)?)?
                .ok_or_else(|| anyhow!("the selected arbiter event is missing"))?;
            events.push(commit.event);
        }
        guard.verify_identity()?;
        tx.commit()?;
        Ok(events)
    })
}

pub(in super::super) fn acknowledge(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    event_id: &str,
) -> Result<()> {
    if event_id.len() > 128 {
        bail!("the arbiter event ID exceeds its bound");
    }
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            tx.execute(
                "UPDATE issue_arbiter_finished_events SET acknowledged=1
             WHERE workspace_dir=?1 AND event_id=?2 AND acknowledged=0",
                params![workspace, event_id],
            )?;
            Ok(())
        })
    })
}

pub(in super::super) fn waits(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    ids: &[IssueDecisionId],
) -> Result<IssueArbiterWaitSnapshot> {
    if ids.len() > MAX_ISSUE_ARBITER_BATCH || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
    {
        bail!("arbiter wait queries require at most 256 unique decisions");
    }
    adapter.with_connection(workspace, |connection, guard| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        guard.verify_identity()?;
        let mut waits = Vec::new();
        for id in ids {
            let Some(record) = decision::read(&tx, workspace, id)? else {
                continue;
            };
            if record.request.state() != IssueDecisionState::Open
                || record.request.definition().on_timeout != IssueDecisionTimeout::Arbiter
            {
                continue;
            }
            let Some((source_revision, commit)) = storage::read_event(&tx, workspace, id)? else {
                continue;
            };
            let IssueArbiterFinishedOutcome::Waiting { reason } = commit.event.outcome else {
                continue;
            };
            if source_revision == record.revision
                && commit.record == record
                && commit.event.fact.basis == record.request.definition().basis
            {
                waits.push(IssueArbiterWaitEntry {
                    decision_id: id.clone(),
                    decision_revision: record.revision,
                    basis: record.request.definition().basis.clone(),
                    reason,
                });
            }
        }
        let catalog_revision = super::super::triage::catalog_revision(&tx, workspace)?;
        guard.verify_identity()?;
        tx.commit()?;
        Ok(IssueArbiterWaitSnapshot {
            catalog_revision,
            waits,
        })
    })
}
