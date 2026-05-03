use std::collections::{BTreeSet, HashSet};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};

use super::{PlanningTaskMutationSource, TASK_ID_HASH_CHARS};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningFileKind,
    PlanningValidationReport, TaskAuthorityDocument, TaskStatus,
};

/*
 * These helpers are shared by preview and commit paths for task mutations.  The
 * service layer should decide what operation is being applied, while this file
 * keeps the cross-cutting invariants local: active direction selection,
 * reference integrity, priority bounds, stable task IDs, and normalized user
 * input before a task authority document is persisted.
 */
pub(super) fn select_direction<'a>(
    requested_direction_id: Option<&str>,
    directions: &'a DirectionCatalogDocument,
) -> Result<&'a DirectionDefinition> {
    // Explicit direction choice is authoritative but cannot target paused/done
    // directions.  Without an explicit choice, prefer the default lane and then
    // any active lane so older catalogs remain usable.
    if let Some(requested_direction_id) = requested_direction_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let direction = find_direction(requested_direction_id, directions)?;
        if direction.state != DirectionState::Active {
            bail!(
                "direction `{}` is not active; task mutations can only create tasks for active directions",
                direction.id.trim()
            );
        }
        return Ok(direction);
    }
    if let Some(direction) = directions.directions.iter().find(|direction| {
        direction.id.trim() == "general-workstream" && direction.state == DirectionState::Active
    }) {
        return Ok(direction);
    }

    directions
        .directions
        .iter()
        .find(|direction| direction.state == DirectionState::Active)
        .ok_or_else(|| anyhow!("task mutation requires an active planning direction"))
}

pub(super) fn find_direction<'a>(
    direction_id: &str,
    directions: &'a DirectionCatalogDocument,
) -> Result<&'a DirectionDefinition> {
    // Direction IDs are also file/command identifiers, so validate their shape
    // before searching the catalog.  This prevents a caller from smuggling path
    // fragments into diagnostics or later authority records.
    let direction_id = required_id(direction_id, "direction id")?;
    directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == direction_id)
        .ok_or_else(|| anyhow!("direction `{direction_id}` does not exist"))
}

pub(super) fn direction_title(
    directions: &DirectionCatalogDocument,
    direction_id: &str,
) -> Option<String> {
    directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == direction_id.trim())
        .map(|direction| direction.title.trim().to_string())
}

pub(super) fn default_relation_note(
    raw_note: Option<&str>,
    direction: &DirectionDefinition,
) -> String {
    // Relation notes are required by the authority schema.  When a caller does
    // not provide a stronger explanation, tie the task back to the direction
    // summary so audits can still see why the task belongs in that lane.
    raw_note
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "Task supports direction `{}`: {}",
                direction.id.trim(),
                direction.summary.trim()
            )
        })
}

pub(super) fn validate_task_reference(
    link_kind: &'static str,
    task_id: &str,
    target_task_id: &str,
    task_ids: &HashSet<String>,
) -> Result<()> {
    let normalized = target_task_id.trim();
    // Dependency/blocker links are graph edges inside the same authority
    // document.  Blank, self, or unknown edges would make queue projection
    // ambiguous, so mutation rejects them before validation is re-run.
    if normalized.is_empty() {
        bail!("task `{task_id}` contains a blank {link_kind}");
    }
    if normalized == task_id {
        bail!("task `{task_id}` cannot reference itself as a {link_kind}");
    }
    if !task_ids.contains(normalized) {
        bail!("task `{task_id}` references unknown {link_kind} `{normalized}`");
    }
    Ok(())
}

pub(super) fn validate_priorities(task_authority: &TaskAuthorityDocument) -> Result<()> {
    // The domain computes combined priority from base plus dynamic delta.  Both
    // inputs and the final projection must stay within UI/ranking bounds.
    for task in &task_authority.tasks {
        if !(0..=100).contains(&task.base_priority) {
            bail!(
                "task `{}` base_priority must be within 0..100",
                task.id.trim()
            );
        }
        if !(-100..=100).contains(&task.dynamic_priority_delta) {
            bail!(
                "task `{}` dynamic_priority_delta must be within -100..100",
                task.id.trim()
            );
        }
        if !(0..=100).contains(&task.combined_priority()) {
            bail!(
                "task `{}` combined priority must stay within 0..100",
                task.id.trim()
            );
        }
    }
    Ok(())
}

pub(super) fn reject_task_validation_errors(report: &PlanningValidationReport) -> Result<()> {
    // Mutation services are allowed to coexist with direction/result-output
    // warnings, but task-authority errors mean the proposed mutation would
    // persist an invalid ledger.  Collapse all task errors into one operator
    // message so preview and commit paths report the same failure.
    let errors = report
        .errors()
        .into_iter()
        .filter(|issue| issue.file_kind == PlanningFileKind::TaskAuthority)
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        return Ok(());
    }
    bail!(
        "planning task mutation failed validation: {}",
        errors.join("; ")
    )
}

pub(super) fn build_task_id(
    source: PlanningTaskMutationSource,
    generated_at: DateTime<Utc>,
    title: &str,
    collision_suffix: Option<u32>,
) -> String {
    let timestamp = generated_at.format("%Y%m%dT%H%M%SZ");
    // IDs need to be readable in task ledgers but deterministic enough for
    // preview/retry flows.  Source + timestamp + title hash forms the base;
    // collision suffix is appended only after the repository reports a clash.
    let base = format!(
        "task-{}-{timestamp}-{}",
        source.id_slug(),
        stable_short_hash(title)
    );
    match collision_suffix {
        Some(suffix) => format!("{base}-{suffix}"),
        None => base,
    }
}

fn stable_short_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..TASK_ID_HASH_CHARS].to_string()
}

pub(super) fn increment_suffix(suffix: Option<u32>) -> Option<u32> {
    Some(suffix.unwrap_or(0) + 1)
}

pub(super) fn task_id_exists(task_authority: &TaskAuthorityDocument, task_id: &str) -> bool {
    task_authority
        .tasks
        .iter()
        .any(|task| task.id.trim() == task_id.trim())
}

pub(super) fn required_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    if value.contains(char::is_whitespace) || value.contains('/') || value.contains('\\') {
        bail!("{label} `{value}` must not contain whitespace or path separators");
    }
    Ok(value)
}

pub(super) fn required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

pub(super) fn normalize_references(values: &[String]) -> Vec<String> {
    // Reference arrays are semantic sets, not user-visible ordered lists.  Trim,
    // drop blanks, de-duplicate, and sort so repeated previews produce stable
    // authority JSON and easier diffs.
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn terminal_status(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Cancelled)
}

pub(super) fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}
