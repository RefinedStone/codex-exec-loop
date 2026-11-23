use super::*;

type BindingPauseCommandRow = (String, String, i64, Option<String>, Option<String>);

fn decode<T: serde::de::DeserializeOwned>(json: &str) -> Result<T> {
    if json.len() > 16 * 1024 {
        bail!("the binding pause record exceeds its bound");
    }
    serde_json::from_str(json).map_err(|_| anyhow!("invalid binding pause record"))
}

pub(super) fn event(
    workspace: &str,
    receipt: &IssueBindingPauseReceipt,
) -> Result<IssueBindingPauseEvent> {
    Ok(IssueBindingPauseEvent {
        event_id: format!(
            "issue-binding-pause:{:x}",
            Sha256::digest(serde_json::to_vec(&(workspace, receipt))?),
        ),
        receipt: receipt.clone(),
    })
}

fn validate_receipt(stored: &StoredIssueBindingPauseCommand) -> Result<()> {
    let Some(receipt) = &stored.receipt else {
        if stored.accepted_revision != stored.intent.expected_revision + 1 {
            bail!("invalid pending binding pause generation");
        }
        return Ok(());
    };
    let intent = &stored.intent;
    let expected = match receipt.outcome {
        IssueBindingPauseOutcome::Paused if intent.paused => {
            (true, true, intent.expected_revision + 1)
        }
        IssueBindingPauseOutcome::Resumed if !intent.paused => {
            (false, true, intent.expected_revision + 1)
        }
        IssueBindingPauseOutcome::ResumeSuperseded if !intent.paused => {
            (true, false, intent.expected_revision + 1)
        }
        IssueBindingPauseOutcome::Unchanged => (intent.paused, false, intent.expected_revision),
        _ => bail!("the binding pause outcome differs from its intent"),
    };
    if receipt.command_id != intent.command_id
        || receipt.binding_id != intent.binding_id
        || receipt.revision != stored.accepted_revision
        || (receipt.paused, receipt.changed, receipt.revision) != expected
    {
        bail!("the binding pause receipt differs from its intent");
    }
    Ok(())
}

pub(in super::super) fn read_command(
    connection: &Connection,
    workspace: &str,
    command_id: &str,
) -> Result<Option<StoredIssueBindingPauseCommand>> {
    crate::domain::issue_automation::autonomy::validate_command_id(command_id)
        .map_err(|reason| anyhow!(reason))?;
    let row: Option<BindingPauseCommandRow> = connection
        .query_row(
            "SELECT binding_id,intent_json,accepted_revision,receipt_json,event_json
         FROM issue_binding_pause_commands WHERE workspace_dir=?1 AND command_id=?2",
            params![workspace, command_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((binding, json, revision, receipt, event_json)) = row else {
        return Ok(None);
    };
    let intent: IssueBindingPauseIntent = decode(&json)?;
    intent.validate().map_err(|reason| anyhow!(reason))?;
    if intent.command_id != command_id
        || intent.binding_id.as_str() != binding
        || read_binding(connection, workspace, &intent.binding_id)?.is_none()
    {
        bail!("the binding pause command has a different owner");
    }
    let stored = StoredIssueBindingPauseCommand {
        intent,
        accepted_revision: u64::try_from(revision)?,
        receipt: receipt.as_deref().map(decode).transpose()?,
    };
    if stored.accepted_revision > MAX_ISSUE_AUTOMATION_REVISION {
        bail!("invalid binding pause generation");
    }
    validate_receipt(&stored)?;
    match (&stored.receipt, event_json.as_deref()) {
        (None, None) => {}
        (Some(receipt), Some(json))
            if decode::<IssueBindingPauseEvent>(json)? == event(workspace, receipt)? => {}
        _ => bail!("the binding pause event differs from its first receipt"),
    }
    Ok(Some(stored))
}

pub(in super::super) fn read_state(
    connection: &Connection,
    workspace: &str,
    binding_id: &IssueAutomationBindingId,
) -> Result<IssueBindingPauseState> {
    // Stored binding IDs keep their pre-Pause contract. The stricter bounds apply to new
    // command input, not to migration, collection reads, or live-runtime recovery.
    if read_binding(connection, workspace, binding_id)?.is_none() {
        bail!("the issue binding is missing");
    }
    let row: Option<(i64, bool, Option<String>)> = connection
        .query_row(
            "SELECT revision,paused,pending_command_id FROM issue_binding_pause_states
         WHERE workspace_dir=?1 AND binding_id=?2",
            params![workspace, binding_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (revision, paused, pending) =
        row.ok_or_else(|| anyhow!("the binding execution state is missing"))?;
    let revision = u64::try_from(revision)?;
    if revision > MAX_ISSUE_AUTOMATION_REVISION || (revision == 0 && (paused || pending.is_some()))
    {
        bail!("invalid binding execution state revision");
    }
    let pending = pending
        .map(|id| {
            let command = read_command(connection, workspace, &id)?
                .ok_or_else(|| anyhow!("the pending binding pause command is missing"))?;
            if command.intent.binding_id != *binding_id
                || command.accepted_revision != revision
                || command.receipt.is_some()
                || command.intent.paused == paused
            {
                bail!("the pending binding pause command does not match its state");
            }
            Ok(command.intent)
        })
        .transpose()?;
    Ok(IssueBindingPauseState {
        binding_id: binding_id.clone(),
        revision,
        paused,
        pending,
    })
}

pub(super) fn persist_receipt(
    transaction: &Transaction<'_>,
    workspace: &str,
    stored: &StoredIssueBindingPauseCommand,
    outcome: IssueBindingPauseOutcome,
    paused: bool,
) -> Result<StoredIssueBindingPauseCommand> {
    let mut completed = stored.clone();
    completed.receipt = Some(IssueBindingPauseReceipt {
        command_id: stored.intent.command_id.clone(),
        binding_id: stored.intent.binding_id.clone(),
        revision: stored.accepted_revision,
        paused,
        outcome,
        changed: matches!(
            outcome,
            IssueBindingPauseOutcome::Paused | IssueBindingPauseOutcome::Resumed
        ),
    });
    validate_receipt(&completed)?;
    let receipt = completed.receipt.as_ref().expect("completed receipt");
    if transaction.execute(
        "UPDATE issue_binding_pause_commands SET receipt_json=?3,event_json=?4
         WHERE workspace_dir=?1 AND command_id=?2 AND receipt_json IS NULL",
        params![
            workspace,
            stored.intent.command_id,
            serde_json::to_string(receipt)?,
            serde_json::to_string(&event(workspace, receipt)?)?
        ],
    )? != 1
    {
        bail!("the binding pause receipt was already completed");
    }
    Ok(completed)
}
