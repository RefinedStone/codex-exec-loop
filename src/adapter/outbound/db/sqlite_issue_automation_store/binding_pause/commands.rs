use super::*;

fn resume_basis_current(
    connection: &Connection,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
) -> Result<bool> {
    let Some(basis) = &intent.resume_basis else {
        return Ok(intent.paused);
    };
    let binding = read_binding(connection, workspace, &intent.binding_id)?
        .ok_or_else(|| anyhow!("the issue binding is missing"))?;
    let autonomy = super::super::autonomy::read(connection, workspace)?;
    // File policy and effective level are rechecked by the live service. This transaction owns
    // the persisted binding/autonomy comparison and never reads repository configuration files.
    Ok(binding.revision == basis.binding_revision
        && autonomy.revision == basis.autonomy_revision
        && autonomy.pending.is_none())
}

pub(in super::super) fn begin(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
) -> Result<StoredIssueBindingPauseCommand> {
    intent.validate().map_err(|reason| anyhow!(reason))?;
    adapter.with_connection(workspace, |connection, guard| transaction(connection, guard, |tx| {
        if let Some(stored) = read_command(tx, workspace, &intent.command_id)? {
            if stored.intent != *intent { bail!("binding pause command ID was reused"); }
            return Ok(stored);
        }
        let current = read_state(tx, workspace, &intent.binding_id)?;
        if current.pending.is_some() || current.revision != intent.expected_revision {
            bail!("the binding pause state changed; refresh before retrying");
        }
        if !intent.paused && !resume_basis_current(tx, workspace, intent)? {
            bail!("the binding resume basis changed");
        }
        let unchanged = current.paused == intent.paused;
        let accepted_revision = if unchanged { current.revision } else { next_revision(current.revision)? };
        tx.execute(
            "INSERT INTO issue_binding_pause_commands
             (workspace_dir,command_id,binding_id,intent_json,accepted_revision)
             VALUES (?1,?2,?3,?4,?5)",
            params![workspace, intent.command_id, intent.binding_id.as_str(),
                serde_json::to_string(intent)?, i64::try_from(accepted_revision)?],
        )?;
        let stored = StoredIssueBindingPauseCommand { intent: intent.clone(), accepted_revision, receipt: None };
        let stored = if unchanged {
            storage::persist_receipt(tx, workspace, &stored, IssueBindingPauseOutcome::Unchanged, current.paused)?
        } else {
            if tx.execute(
                "UPDATE issue_binding_pause_states SET revision=?3,pending_command_id=?4
                 WHERE workspace_dir=?1 AND binding_id=?2 AND revision=?5 AND pending_command_id IS NULL",
                params![workspace, intent.binding_id.as_str(), i64::try_from(accepted_revision)?,
                    intent.command_id, i64::try_from(current.revision)?],
            )? != 1 { bail!("the binding pause acceptance lost its generation"); }
            stored
        };
        super::super::triage::bump_catalog(tx, workspace)?;
        Ok(stored)
    }))
}

pub(in super::super) fn finish(
    adapter: &SqliteIssueAutomationStoreAdapter,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
    completion: IssueBindingPauseCompletion,
) -> Result<StoredIssueBindingPauseCommand> {
    intent.validate().map_err(|reason| anyhow!(reason))?;
    adapter.with_connection(workspace, |connection, guard| {
        transaction(connection, guard, |tx| {
            let stored = read_command(tx, workspace, &intent.command_id)?
                .ok_or_else(|| anyhow!("the binding pause intent is missing"))?;
            if stored.intent != *intent {
                bail!("binding pause command ID was reused");
            }
            if stored.receipt.is_some() {
                return Ok(stored);
            }
            let stored = require_pending(tx, workspace, intent)?;
            if intent.paused && completion == IssueBindingPauseCompletion::ResumeSuperseded {
                bail!("a pause cannot be completed as a superseded resume");
            }
            fences::require_completed(tx, workspace, intent)?;
            let outcome = if intent.paused {
                IssueBindingPauseOutcome::Paused
            } else if completion == IssueBindingPauseCompletion::ResumeSuperseded
                || !resume_basis_current(tx, workspace, intent)?
            {
                IssueBindingPauseOutcome::ResumeSuperseded
            } else {
                IssueBindingPauseOutcome::Resumed
            };
            let paused = outcome != IssueBindingPauseOutcome::Resumed;
            if tx.execute(
                "UPDATE issue_binding_pause_states SET paused=?3,pending_command_id=NULL
             WHERE workspace_dir=?1 AND binding_id=?2 AND revision=?4 AND pending_command_id=?5",
                params![
                    workspace,
                    intent.binding_id.as_str(),
                    paused,
                    i64::try_from(stored.accepted_revision)?,
                    intent.command_id
                ],
            )? != 1
            {
                bail!("the binding pause completion lost its generation");
            }
            let completed = storage::persist_receipt(tx, workspace, &stored, outcome, paused)?;
            super::super::triage::bump_catalog(tx, workspace)?;
            Ok(completed)
        })
    })
}
