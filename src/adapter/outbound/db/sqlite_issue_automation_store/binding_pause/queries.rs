use super::*;

pub(in super::super) fn list_states(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    after: Option<&IssueAutomationBindingId>,
    limit: usize,
) -> Result<Vec<IssueBindingPauseState>> {
    let limit = page_limit(limit)?;
    adapter.with_connection(workspace, |connection, _| {
        let mut query = connection.prepare(
            "SELECT binding_id FROM issue_automation_bindings
             WHERE workspace_dir=?1 AND (?2 IS NULL OR binding_id>?2)
             ORDER BY binding_id LIMIT ?3",
        )?;
        let ids = query
            .query_map(
                params![
                    workspace,
                    after.map(IssueAutomationBindingId::as_str),
                    limit
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| read_state(connection, workspace, &IssueAutomationBindingId::new(id)?))
            .collect()
    })
}

pub(in super::super) fn pending_commands(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    after: Option<&IssueAutomationBindingId>,
    limit: usize,
) -> Result<Vec<StoredIssueBindingPauseCommand>> {
    let limit = page_limit(limit)?;
    adapter.with_connection(workspace, |connection, _| {
        let mut query = connection.prepare(
            "SELECT binding_id FROM issue_binding_pause_states
             WHERE workspace_dir=?1 AND pending_command_id IS NOT NULL
               AND (?2 IS NULL OR binding_id>?2) ORDER BY binding_id LIMIT ?3",
        )?;
        let ids = query
            .query_map(
                params![
                    workspace,
                    after.map(IssueAutomationBindingId::as_str),
                    limit
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| {
                let state = read_state(connection, workspace, &IssueAutomationBindingId::new(id)?)?;
                let intent = state
                    .pending
                    .ok_or_else(|| anyhow!("the pending binding command disappeared"))?;
                read_command(connection, workspace, &intent.command_id)?
                    .ok_or_else(|| anyhow!("the pending binding command is missing"))
            })
            .collect()
    })
}

pub(super) fn work_by_id(
    connection: &Connection,
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<IssueWorkCheckpoint>> {
    if work_order_id.trim().is_empty()
        || work_order_id.len() > 512
        || work_order_id.chars().any(char::is_control)
    {
        bail!("invalid issue work-order identity");
    }
    let row: Option<(String, String, String, String)> = connection
        .query_row(
            "SELECT provider,server_id,repository_id,issue_id FROM issue_work_checkpoints
         WHERE workspace_dir=?1 AND work_order_id=?2",
            params![workspace, work_order_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((provider, server_id, repository_id, issue_id)) = row else {
        return Ok(None);
    };
    let issue = IssueReference {
        provider: serde_json::from_value(serde_json::Value::String(provider))?,
        server_id,
        repository_id,
        issue_id,
        project_scope: crate::domain::issue_work::IssueProjectScope {
            workspace_dir: workspace.into(),
        },
    };
    let work = super::super::coordinator::read(connection, workspace, &issue)?
        .ok_or_else(|| anyhow!("the issue work checkpoint disappeared"))?;
    if work.order.work_order_id != work_order_id {
        bail!("the issue work checkpoint has a different identity");
    }
    Ok(Some(work))
}

pub(in super::super) fn resolve_work(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<IssueAutomationBindingId>> {
    adapter.with_connection(workspace, |connection, _| {
        let transaction = connection.transaction()?;
        let result = work_by_id(&transaction, workspace, work_order_id)?
            .map(|work| {
                issue_state(&transaction, workspace, &work.order.issue)
                    .map(|state| state.binding_id)
            })
            .transpose()?;
        transaction.commit()?;
        Ok(result)
    })
}

pub(super) fn work_page(
    connection: &Connection,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
    after: Option<&str>,
    limit: usize,
    unfinished_only: bool,
) -> Result<Vec<IssueWorkCheckpoint>> {
    let limit = page_limit(limit)?;
    let binding = read_binding(connection, workspace, &intent.binding_id)?
        .ok_or_else(|| anyhow!("the paused binding is missing"))?;
    let mut query = connection.prepare(
        "SELECT w.work_order_id FROM issue_work_checkpoints w
         LEFT JOIN tracked_issues t USING(workspace_dir,provider,server_id,repository_id,issue_id)
         LEFT JOIN issue_source_observations o USING(workspace_dir,provider,server_id,repository_id,issue_id)
         LEFT JOIN issue_binding_pause_work_fences f ON f.workspace_dir=w.workspace_dir
           AND f.work_order_id=w.work_order_id AND f.pause_command_id=?3
         LEFT JOIN issue_intervention_commands c ON c.workspace_dir=f.workspace_dir
           AND c.command_id=f.control_command_id
         WHERE w.workspace_dir=?1 AND w.provider=?4 AND w.server_id=?5 AND w.repository_id=?6
           AND (t.binding_id=?2 OR o.binding_id=?2 OR (t.binding_id IS NULL AND o.binding_id IS NULL))
           AND (?7 IS NULL OR w.work_order_id>?7) AND (?8=0 OR c.receipt_json IS NULL)
         ORDER BY w.work_order_id LIMIT ?9",
    )?;
    let ids = query
        .query_map(
            params![
                workspace,
                intent.binding_id.as_str(),
                intent.command_id,
                super::super::enum_name(&binding.binding.provider())?,
                binding.binding.server_id(),
                binding.binding.repository_id(),
                after,
                unfinished_only,
                limit
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.into_iter()
        .map(|id| {
            let work = work_by_id(connection, workspace, &id)?
                .ok_or_else(|| anyhow!("the paused issue work disappeared"))?;
            if issue_state(connection, workspace, &work.order.issue)?.binding_id
                != intent.binding_id
            {
                bail!("the paused issue work belongs to a different binding");
            }
            Ok(work)
        })
        .collect()
}

pub(in super::super) fn pending_events(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    limit: usize,
) -> Result<Vec<IssueBindingPauseEvent>> {
    let limit = page_limit(limit)?;
    adapter.with_connection(workspace, |connection, _| {
        let mut query = connection.prepare(
            "SELECT command_id FROM issue_binding_pause_commands WHERE workspace_dir=?1
             AND event_json IS NOT NULL AND acknowledged=0 ORDER BY binding_id,command_id LIMIT ?2",
        )?;
        let ids = query
            .query_map(params![workspace, limit], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| {
                let stored = read_command(connection, workspace, &id)?
                    .ok_or_else(|| anyhow!("the completed binding pause command is missing"))?;
                storage::event(
                    workspace,
                    &stored
                        .receipt
                        .ok_or_else(|| anyhow!("the binding pause event has no receipt"))?,
                )
            })
            .collect()
    })
}

pub(in super::super) fn acknowledge(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    expected: &IssueBindingPauseEvent,
) -> Result<()> {
    adapter.with_connection(workspace, |connection, guard| transaction(connection, guard, |tx| {
        let stored = read_command(tx, workspace, &expected.receipt.command_id)?
            .ok_or_else(|| anyhow!("the binding pause event is missing"))?;
        let receipt = stored.receipt.ok_or_else(|| anyhow!("the binding pause command is incomplete"))?;
        if storage::event(workspace, &receipt)? != *expected {
            bail!("the binding pause ACK differs from the immutable event");
        }
        tx.execute("UPDATE issue_binding_pause_commands SET acknowledged=1 WHERE workspace_dir=?1 AND command_id=?2",
            params![workspace, expected.receipt.command_id])?;
        Ok(())
    }))
}
