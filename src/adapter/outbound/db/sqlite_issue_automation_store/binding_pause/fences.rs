use super::*;

fn owned(
    connection: &Connection,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
    work: &IssueWorkCheckpoint,
) -> Result<Option<StoredIssueIntervention>> {
    let id: Option<String> = connection
        .query_row(
            "SELECT control_command_id FROM issue_binding_pause_work_fences
         WHERE workspace_dir=?1 AND pause_command_id=?2 AND work_order_id=?3",
            params![workspace, intent.command_id, work.order.work_order_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(None);
    };
    if !id.starts_with(ISSUE_BINDING_PAUSE_CONTROL_PREFIX) {
        bail!("the owned binding pause fence identifier is invalid");
    }
    let stored = super::super::intervention::read(connection, workspace, &id)?
        .ok_or_else(|| anyhow!("the owned binding pause fence is missing"))?;
    let expected = IssueInterventionRequest {
        command_id: id,
        issue: work.order.issue.clone(),
        action: IssueInterventionAction::Retriage,
    };
    if stored.request != expected
        || stored.receipt.as_ref().is_some_and(|receipt| {
            receipt.command_id != expected.command_id
                || receipt.issue != expected.issue
                || receipt.action != expected.action.label()
                || receipt.work_order_id.as_deref() != Some(work.order.work_order_id.as_str())
                || receipt.authorization_epoch > MAX_ISSUE_AUTOMATION_REVISION
                || receipt.delivery.is_some()
                || receipt.handoff.is_some()
        })
    {
        bail!("the owned binding pause fence differs from its intent");
    }
    Ok(Some(stored))
}

pub(in super::super) fn claim_fence(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
    work: &IssueWorkCheckpoint,
) -> Result<StoredIssueIntervention> {
    work.validate().map_err(|reason| anyhow!(reason))?;
    validate_reference_scope(workspace, &work.order.issue)?;
    adapter.with_connection(workspace, |connection, guard| transaction(connection, guard, |tx| {
        require_pending(tx, workspace, intent)?;
        let current = queries::work_by_id(tx, workspace, &work.order.work_order_id)?
            .ok_or_else(|| anyhow!("the binding pause work is missing"))?;
        if current.order.issue != work.order.issue
            || issue_state(tx, workspace, &current.order.issue)?.binding_id != intent.binding_id
        { bail!("the binding pause fence belongs to another work or binding"); }
        if let Some(stored) = owned(tx, workspace, intent, &current)? { return Ok(stored); }
        let base = format!("{}{:x}", ISSUE_BINDING_PAUSE_CONTROL_PREFIX,
            Sha256::digest(serde_json::to_vec(&(workspace, &intent.binding_id, &intent.command_id, &current.order.work_order_id))?));
        let mut collision = 0_u64;
        let command_id = loop {
            let candidate = if collision == 0 { base.clone() } else { format!("{base}-{collision}") };
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM issue_intervention_commands WHERE workspace_dir=?1 AND command_id=?2)",
                params![workspace, candidate], |row| row.get(0),
            )?;
            if !exists { break candidate; }
            if collision >= 64 { bail!("binding pause fence identifier collision limit reached"); }
            collision = collision.checked_add(1).ok_or_else(|| anyhow!("binding pause fence identifier exhausted"))?;
        };
        let request = IssueInterventionRequest { command_id, issue: current.order.issue.clone(), action: IssueInterventionAction::Retriage };
        let stored = super::super::intervention::begin_in_transaction(tx, workspace, &request)?;
        if stored.receipt.is_some() { bail!("a new binding pause fence already has a receipt"); }
        tx.execute(
            "INSERT INTO issue_binding_pause_work_fences(workspace_dir,pause_command_id,work_order_id,control_command_id)
             VALUES (?1,?2,?3,?4)",
            params![workspace, intent.command_id, current.order.work_order_id, request.command_id],
        )?;
        Ok(stored)
    }))
}

pub(in super::super) fn pending_work(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
    after: Option<&str>,
    limit: usize,
) -> Result<Vec<IssueWorkCheckpoint>> {
    page_limit(limit)?;
    adapter.with_connection(workspace, |connection, _| {
        let tx = connection.transaction()?;
        require_pending(&tx, workspace, intent)?;
        // A generic recoverer can commit Retriage before Pause explicitly revokes permits.
        // Keep completed receipts in these pages; the live keyset cursor avoids repetition.
        let work = queries::work_page(&tx, workspace, intent, after, limit, false)?;
        tx.commit()?;
        Ok(work)
    })
}

pub(super) fn require_completed(
    connection: &Connection,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
) -> Result<()> {
    let mut after = None;
    loop {
        let page = queries::work_page(
            connection,
            workspace,
            intent,
            after.as_deref(),
            ISSUE_BINDING_PAUSE_PAGE_LIMIT,
            false,
        )?;
        if page.is_empty() {
            return Ok(());
        }
        for work in &page {
            if owned(connection, workspace, intent, work)?
                .is_none_or(|stored| stored.receipt.is_none())
            {
                bail!("every binding work must complete its owned pause fence");
            }
        }
        after = page.last().map(|work| work.order.work_order_id.clone());
    }
}
