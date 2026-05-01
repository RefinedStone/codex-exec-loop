use super::*;

pub(super) fn planning_manual_editor_close_warning_status(
    risk: PlanningDraftEditorCloseRisk,
) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "planning draft editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "planning draft editor close pending".to_string(),
    }
}

pub(super) fn planning_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "planning draft editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "planning draft editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "planning draft editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "planning draft editor closed".to_string(),
    }
}

pub(super) fn directions_manual_editor_close_warning_status(
    risk: PlanningDraftEditorCloseRisk,
) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "directions editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "directions editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "directions editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "directions editor close pending".to_string(),
    }
}

pub(super) fn directions_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "directions editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "directions editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "directions editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "directions editor closed".to_string(),
    }
}

pub(super) fn planning_doctor_status_text(report: &PlanningDoctorReport) -> String {
    let mut parts = vec![format!(
        "planning state: {}",
        report.planning_state().label()
    )];

    if let Some(queue_idle_policy) = report.queue_idle_policy() {
        parts.push(format!("queue-idle: {queue_idle_policy}"));
    }
    if let Some(queue_summary) = report.queue_summary() {
        parts.push(format!("queue: {queue_summary}"));
    }
    if let Some(proposal_summary) = report.proposal_summary() {
        parts.push(format!("proposals: {proposal_summary}"));
    }
    if let Some(issue) = report.issue() {
        parts.push(format!("issue: {issue}"));
    } else if let Some(health) = report.health() {
        parts.push(format!("health: {health}"));
    }
    if let Some(note) = report.note() {
        parts.push(format!("note: {note}"));
    }
    if report.planning_state() == PlanningDoctorState::Absent {
        parts.push("next action: run :init to stage the default planning scaffold".to_string());
    }

    parts.join(" / ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParsedResetShellCommand {
    pub(super) target: PlanningResetTarget,
    pub(super) confirmed: bool,
}

pub(super) fn parse_reset_shell_argument(
    argument: Option<&str>,
) -> Result<ParsedResetShellCommand, String> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(
            "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                .to_string(),
        );
    };
    let mut parts = argument.split_whitespace();
    let Some(target) = parts.next() else {
        return Err(
            "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                .to_string(),
        );
    };
    let confirmation = parts.next();
    let confirmed = match confirmation {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("confirm") => true,
        Some(_) => {
            return Err(
                "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                    .to_string(),
            );
        }
    };
    if parts.next().is_some() {
        return Err(
            "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                .to_string(),
        );
    }
    let target = match target.to_ascii_lowercase().as_str() {
        "queue" => PlanningResetTarget::Queue,
        "directions" => PlanningResetTarget::Directions,
        "all" => PlanningResetTarget::All,
        _ => {
            return Err(
                "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all"
                    .to_string(),
            );
        }
    };
    Ok(ParsedResetShellCommand { target, confirmed })
}

pub(super) fn planning_reset_preview_text(target: PlanningResetTarget) -> String {
    match target {
        PlanningResetTarget::Queue => {
            "reset queue preview: rewrites DB task authority and clears derived queue state"
                .to_string()
        }
        PlanningResetTarget::Directions => "reset directions preview: rewrites DB direction authority, recreates the default queue-idle prompt, removes direction detail docs and prompt artifacts, and clears derived queue state / rerun `:reset directions confirm` to continue".to_string(),
        PlanningResetTarget::All => "reset all preview: replaces the full active planning scaffold, clears derived queue state, and refreshes the planning authority / rerun `:reset all confirm` to continue".to_string(),
    }
}

pub(super) fn planning_reset_status_text(result: &PlanningWorkspaceResetResult) -> String {
    format!(
        "planning reset applied / target: {} / rewritten: {} / removed: {}",
        result.target.label(),
        result.rewritten_paths.len(),
        result.removed_paths.len(),
    )
}
