//! 바인딩의 실행 세대, 소유한 Retriage와 첫 완료 영수증을 저장한다.
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

use super::{
    SqliteIssueAutomationStoreAdapter, next_revision, read_binding, transaction,
    validate_reference_scope,
};
use crate::domain::issue_automation::binding_pause::*;
use crate::domain::issue_automation::coordinator::IssueWorkCheckpoint;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest, StoredIssueIntervention,
};
use crate::domain::issue_automation::{IssueAutomationBindingId, MAX_ISSUE_AUTOMATION_REVISION};
use crate::domain::issue_work::IssueReference;

mod commands;
mod fences;
mod queries;
mod schema;
mod storage;

pub(super) use commands::{begin, finish};
pub(super) use fences::{claim_fence, pending_work};
pub(super) use queries::{
    acknowledge, list_states, pending_commands, pending_events, resolve_work,
};
pub(super) use schema::{create_schema, initialize_binding, schema_objects};
pub(super) use storage::{read_command, read_state};

pub(super) fn issue_state(
    connection: &Connection,
    workspace: &str,
    issue: &IssueReference,
) -> Result<IssueBindingPauseState> {
    validate_reference_scope(workspace, issue)?;
    let item = super::collection::queries::item(connection, workspace, issue)?
        .ok_or_else(|| anyhow!("the issue has no stored binding"))?;
    let binding = read_binding(connection, workspace, &item.binding_id)?
        .ok_or_else(|| anyhow!("the issue binding is missing"))?;
    if !binding.binding.matches_reference(issue) {
        bail!("the issue binding does not match its stored source");
    }
    read_state(connection, workspace, &item.binding_id)
}

fn require_pending(
    connection: &Connection,
    workspace: &str,
    intent: &IssueBindingPauseIntent,
) -> Result<StoredIssueBindingPauseCommand> {
    intent.validate().map_err(|reason| anyhow!(reason))?;
    let stored = read_command(connection, workspace, &intent.command_id)?
        .ok_or_else(|| anyhow!("the binding pause intent is missing"))?;
    let state = read_state(connection, workspace, &intent.binding_id)?;
    if stored.intent != *intent
        || stored.receipt.is_some()
        || state.pending.as_ref() != Some(intent)
        || state.revision != stored.accepted_revision
    {
        bail!("the binding pause intent no longer owns its pending generation");
    }
    Ok(stored)
}

fn page_limit(limit: usize) -> Result<i64> {
    if !(1..=ISSUE_BINDING_PAUSE_PAGE_LIMIT).contains(&limit) {
        bail!("the binding pause page limit is invalid");
    }
    Ok(limit as i64)
}

#[cfg(test)]
mod tests;
