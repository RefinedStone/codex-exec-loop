use super::super::{ConversationViewModel, NativeTuiApp};
use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningRuntimeProjection, PlanningRuntimeRepairAttempt,
    PlanningRuntimeSummaryLineRequest,
};
use crate::domain::text::compact_whitespace_detail;
use ratatui::text::Line;

// Planning status appears in several shell surfaces with different space
// budgets. Resume status is a single line, inline tail gets a compact summary
// plus optional queue framing, and diagnostics can ask for longer details.
const RESUMED_SESSION_DETAIL_LIMIT: usize = 96;
const STATUS_SEGMENT_SEPARATOR: &str = "  |  ";

// The surface projection is the presentation boundary for planning runtime
// state. It deliberately separates persistent status, transient notices, and
// queue framing so renderers can place each part without reinterpreting the
// planning runtime projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanningStatusSurfaceProjection {
    pub(crate) summary_line: Option<String>,
    pub(crate) notice_line: Option<String>,
    pub(crate) queue_framing_lines: Vec<Line<'static>>,
}

// Queue framing normalizes multiple planning sources into the four labels the
// shell repeats everywhere: current work, next executable task, promotable
// proposals, and blocked/skipped work.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueFramingDetails {
    now_detail: String,
    next_detail: String,
    proposed_detail: String,
    blocked_detail: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PartialQueueFramingDetails {
    now_detail: Option<String>,
    next_detail: Option<String>,
    proposed_detail: Option<String>,
    blocked_detail: Option<String>,
}

pub(crate) fn build_planning_status_surface_projection(
    app: &NativeTuiApp,
    conversation: &ConversationViewModel,
    summary_detail_len: usize,
    supplemental_detail_len: usize,
    always_show: bool,
) -> PlanningStatusSurfaceProjection {
    let runtime_projection = app.planning_runtime_projection_snapshot();
    PlanningStatusSurfaceProjection {
        summary_line: build_planning_summary_line(
            app,
            conversation,
            &runtime_projection,
            summary_detail_len,
            always_show,
        ),
        notice_line: build_planning_notice_line(conversation, supplemental_detail_len),
        queue_framing_lines: build_queue_framing_lines(
            &runtime_projection,
            supplemental_detail_len,
        ),
    }
}

// Resumed sessions need status text before the full inline shell has rendered.
// Prefer the queue summary because it gives operators immediate handoff context;
// fall back to the runtime detail only when no queue framing is available.
pub(crate) fn build_resumed_session_status_text(
    runtime_projection: &PlanningRuntimeProjection,
) -> String {
    let mut status_text = format!(
        "thread loaded / planning status: {}",
        runtime_projection.preview_status_label()
    );
    if let Some(queue_summary) = build_queue_framing_summary_from_projection(
        runtime_projection,
        RESUMED_SESSION_DETAIL_LIMIT,
    ) {
        status_text.push_str(" / queue summary: ");
        status_text.push_str(&queue_summary);
    } else if let Some(detail) = runtime_projection.preview_detail() {
        status_text.push_str(" / planning detail: ");
        status_text.push_str(&compact_whitespace_detail(
            detail,
            RESUMED_SESSION_DETAIL_LIMIT,
        ));
    }

    status_text
}

// Summary generation stays delegated to the planning service so the TUI does
// not duplicate readiness/repair wording. The adapter only contributes shell
// context that the service cannot know: whether a turn is running, whether a
// repair is in flight, and whether a separate notice line already exists.
pub(crate) fn build_planning_summary_line(
    app: &NativeTuiApp,
    conversation: &ConversationViewModel,
    runtime_projection: &PlanningRuntimeProjection,
    max_detail_len: usize,
    always_show: bool,
) -> Option<String> {
    app.application
        .planning()
        .runtime()
        .build_summary_line(PlanningRuntimeSummaryLineRequest {
            projection: runtime_projection,
            has_running_turn: conversation.has_running_turn(),
            is_repairing: conversation.planning_repair_state.is_some(),
            repair_failure_summary: conversation
                .planning_repair_state
                .as_ref()
                .map(|state| state.latest_request.failure_summary.as_str()),
            repair_attempt: conversation.planning_repair_state.as_ref().map(|state| {
                PlanningRuntimeRepairAttempt {
                    attempts_used: state.attempts_used,
                    max_attempts: state.max_attempts,
                }
            }),
            has_notice: conversation
                .planning_notice_summary(max_detail_len)
                .is_some(),
            max_detail_len,
            always_show,
        })
        .and_then(remove_legacy_valid_planning_summary_prefix)
}

fn remove_legacy_valid_planning_summary_prefix(summary_line: String) -> Option<String> {
    const LEGACY_VALID_PREFIX: &str = "planning: valid";
    const LEGACY_VALID_SEGMENT_PREFIX: &str = "planning: valid  |  ";

    if summary_line == LEGACY_VALID_PREFIX {
        return None;
    }
    if let Some(rest) = summary_line.strip_prefix(LEGACY_VALID_SEGMENT_PREFIX) {
        let trimmed = rest.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }
    Some(summary_line)
}

pub(crate) fn build_planning_notice_line(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<String> {
    conversation
        .planning_notice_summary(max_detail_len)
        .map(|summary| format!("planning notice: {summary}"))
}

pub(crate) fn build_queue_framing_lines(
    runtime_projection: &PlanningRuntimeProjection,
    max_detail_len: usize,
) -> Vec<Line<'static>> {
    build_queue_framing_lines_from_projection(runtime_projection, max_detail_len)
}

pub(crate) fn build_queue_framing_lines_from_projection(
    runtime_projection: &PlanningRuntimeProjection,
    max_detail_len: usize,
) -> Vec<Line<'static>> {
    build_queue_framing_details_from_projection(runtime_projection, max_detail_len)
        .map(|details| queue_framing_lines_from_details(&details))
        .unwrap_or_default()
}

pub(crate) fn build_queue_framing_summary_from_projection(
    runtime_projection: &PlanningRuntimeProjection,
    max_detail_len: usize,
) -> Option<String> {
    build_queue_framing_details_from_projection(runtime_projection, max_detail_len)
        .map(|details| queue_framing_summary_from_details(&details))
}

// Queue summaries may come from older persisted strings or from fresh queue
// projections. This compactor upgrades partial strings into the current four
// field shape so restored sessions and live sessions read the same.
pub(crate) fn compact_queue_framing_summary(summary: &str, max_detail_len: usize) -> String {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return queue_framing_summary_from_parts("none", "none", "none", "none");
    }
    if let Some(parsed_details) = parse_queue_framing_details(trimmed, max_detail_len) {
        let mut details = QueueFramingDetails {
            now_detail: "none".to_string(),
            next_detail: "none".to_string(),
            proposed_detail: "none".to_string(),
            blocked_detail: "none".to_string(),
        };
        merge_queue_framing_details(&mut details, parsed_details);
        return queue_framing_summary_from_details(&details);
    }

    compact_whitespace_detail(trimmed, max_detail_len)
}

fn parse_queue_framing_details(
    summary: &str,
    max_detail_len: usize,
) -> Option<PartialQueueFramingDetails> {
    let mut details = PartialQueueFramingDetails::default();
    let mut matched = false;
    for segment in summary.split(STATUS_SEGMENT_SEPARATOR) {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("now: ") {
            details.now_detail = Some(compact_whitespace_detail(detail, max_detail_len));
            matched = true;
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("next: ") {
            details.next_detail = Some(compact_whitespace_detail(detail, max_detail_len));
            matched = true;
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("proposed: ") {
            details.proposed_detail = Some(compact_whitespace_detail(detail, max_detail_len));
            matched = true;
            continue;
        }
        if let Some(detail) = trimmed.strip_prefix("blocked: ") {
            details.blocked_detail = Some(compact_whitespace_detail(detail, max_detail_len));
            matched = true;
            continue;
        }
    }

    matched.then_some(details)
}

fn merge_queue_framing_details(
    details: &mut QueueFramingDetails,
    parsed: PartialQueueFramingDetails,
) {
    if let Some(now_detail) = parsed.now_detail {
        details.now_detail = now_detail;
    }
    if let Some(next_detail) = parsed.next_detail {
        details.next_detail = next_detail;
    }
    if let Some(proposed_detail) = parsed.proposed_detail {
        details.proposed_detail = proposed_detail;
    }
    if let Some(blocked_detail) = parsed.blocked_detail {
        details.blocked_detail = blocked_detail;
    }
}

// Prefer the structured application projection when it exists: it can distinguish
// the active task from remaining active work and skipped tasks. Older projections
// only have free-form summaries, so the fallback parser merges any available
// fields with queue-head and proposal data.
fn build_queue_framing_details_from_projection(
    runtime_projection: &PlanningRuntimeProjection,
    max_detail_len: usize,
) -> Option<QueueFramingDetails> {
    let application_projection =
        PlanningApplicationProjection::from_runtime_projection(runtime_projection);
    build_queue_framing_details_from_application_projection(&application_projection, max_detail_len)
}

fn build_queue_framing_details_from_application_projection(
    projection: &PlanningApplicationProjection,
    max_detail_len: usize,
) -> Option<QueueFramingDetails> {
    let has_queue_context = projection.workspace_present
        || projection.queue_head.is_some()
        || projection.queue_summary.is_some()
        || projection.proposal_summary.is_some()
        || projection.has_structured_queue_projection;
    if !has_queue_context {
        return None;
    }
    let mut details = QueueFramingDetails {
        now_detail: "none".to_string(),
        next_detail: "none".to_string(),
        proposed_detail: "none".to_string(),
        blocked_detail: "none".to_string(),
    };
    if projection.has_structured_queue_projection {
        let current_task = projection
            .queue_head
            .as_ref()
            .or_else(|| projection.visible_tasks.first());
        let now_detail = current_task
            .map(|task| compact_queue_task_summary(task.task_title.as_str(), 1, 1, max_detail_len))
            .unwrap_or_else(|| "none".to_string());
        let remaining_tasks = current_task
            .map(|current| {
                projection
                    .visible_tasks
                    .iter()
                    .filter(|task| task.task_id != current.task_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| projection.visible_tasks.iter().collect::<Vec<_>>());
        let next_detail = remaining_tasks
            .first()
            .map(|task| {
                compact_queue_task_summary(
                    task.task_title.as_str(),
                    remaining_tasks.len(),
                    1,
                    max_detail_len,
                )
            })
            .unwrap_or_else(|| "none".to_string());
        let proposed_detail = projection
            .proposed_tasks
            .first()
            .map(|task| {
                compact_queue_task_summary(
                    task.task_title.as_str(),
                    projection.proposed_tasks.len(),
                    1,
                    max_detail_len,
                )
            })
            .or_else(|| {
                projection
                    .proposal_summary
                    .as_deref()
                    .map(|summary| compact_proposal_summary_detail(summary, max_detail_len))
            })
            .unwrap_or_else(|| "none".to_string());
        let blocked_detail = projection
            .skipped_tasks
            .first()
            .map(|task| {
                let title = compact_whitespace_detail(task.task_title.as_str(), max_detail_len);
                let reason = compact_whitespace_detail(task.reason.as_str(), max_detail_len);
                let mut summary = format!("{title} ({reason})");
                let hidden_count = projection.skipped_tasks.len().saturating_sub(1);
                if hidden_count > 0 {
                    summary.push_str(&format!(" (+{hidden_count} more)"));
                }
                summary
            })
            .unwrap_or_else(|| "none".to_string());
        return Some(QueueFramingDetails {
            now_detail,
            next_detail,
            proposed_detail,
            blocked_detail,
        });
    }
    if let Some(queue_head) = projection.queue_head.as_ref() {
        details.now_detail =
            compact_queue_task_summary(queue_head.task_title.as_str(), 1, 1, max_detail_len);
    }
    if let Some(queue_summary) = projection.queue_summary.as_deref()
        && let Some(parsed_details) = parse_queue_framing_details(queue_summary, max_detail_len)
    {
        merge_queue_framing_details(&mut details, parsed_details);
    }
    if let Some(proposal_summary) = projection.proposal_summary.as_deref() {
        details.proposed_detail = compact_proposal_summary_detail(proposal_summary, max_detail_len);
    }

    Some(details)
}

fn compact_queue_task_summary(
    task_title: &str,
    total_count: usize,
    shown_count: usize,
    max_detail_len: usize,
) -> String {
    let mut summary = compact_whitespace_detail(task_title.trim(), max_detail_len);
    let hidden_count = total_count.saturating_sub(shown_count);
    if hidden_count > 0 {
        summary.push_str(&format!(" (+{hidden_count} more)"));
    }
    summary
}

fn compact_proposal_summary_detail(summary: &str, max_detail_len: usize) -> String {
    compact_whitespace_detail(summary, max_detail_len)
}

fn queue_framing_lines_from_details(details: &QueueFramingDetails) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if detail_has_signal(&details.now_detail) || detail_has_signal(&details.next_detail) {
        lines.push(Line::from(format!(
            "now: {}{STATUS_SEGMENT_SEPARATOR}next: {}",
            details.now_detail, details.next_detail
        )));
    }
    if detail_has_signal(&details.proposed_detail) || detail_has_signal(&details.blocked_detail) {
        lines.push(Line::from(format!(
            "proposed: {}{STATUS_SEGMENT_SEPARATOR}blocked: {}",
            details.proposed_detail, details.blocked_detail
        )));
    }
    lines
}

fn detail_has_signal(detail: &str) -> bool {
    detail.trim() != "none"
}

fn queue_framing_summary_from_details(details: &QueueFramingDetails) -> String {
    queue_framing_summary_from_parts(
        details.now_detail.as_str(),
        details.next_detail.as_str(),
        details.proposed_detail.as_str(),
        details.blocked_detail.as_str(),
    )
}

fn queue_framing_summary_from_parts(
    now_detail: &str,
    next_detail: &str,
    proposed_detail: &str,
    blocked_detail: &str,
) -> String {
    format!(
        "now: {now_detail}{STATUS_SEGMENT_SEPARATOR}next: {next_detail}{STATUS_SEGMENT_SEPARATOR}proposed: {proposed_detail}{STATUS_SEGMENT_SEPARATOR}blocked: {blocked_detail}"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        build_queue_framing_lines_from_projection, build_queue_framing_summary_from_projection,
        build_resumed_session_status_text, compact_queue_framing_summary,
        remove_legacy_valid_planning_summary_prefix,
    };
    use crate::application::service::planning::PlanningRuntimeProjection;
    use crate::domain::planning::{
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
    };
    #[test]
    fn resumed_session_status_prefers_queue_summary_projection() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "now: Ship resume status  |  next: Review overlays  |  proposed: none  |  blocked: none"
                .to_string(),
            None,
            None,
        );
        let status_text = build_resumed_session_status_text(&runtime_projection);

        assert!(status_text.contains("thread loaded / planning status: ready"));
        assert!(status_text.contains("queue summary: now: Ship resume status"));
        assert!(status_text.contains("next: Review overlays"));
    }
    #[test]
    fn queue_framing_summary_skips_duplicate_next_when_projection_has_no_explicit_next_task() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue ready".to_string(),
            None,
            None,
            PriorityQueueProjection {
                next_task: None,
                active_tasks: vec![
                    queue_task("task-1", "Ship resume status", 1),
                    queue_task("task-2", "Review overlays", 2),
                ],
                proposed_tasks: Vec::new(),
                skipped_tasks: Vec::new(),
            },
        );
        let summary = build_queue_framing_summary_from_projection(&runtime_projection, 96)
            .expect("queue framing summary should exist");

        assert!(summary.contains("now: Ship resume status"));
        assert!(summary.contains("next: Review overlays"));
        assert!(!summary.contains("next: Ship resume status"));
    }
    #[test]
    fn queue_framing_summary_merges_partial_queue_summary_with_existing_details() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "now: Review overlays".to_string(),
            Some("Promote follow-up proposal".to_string()),
            None,
        )
        .with_workspace_present(true);
        let summary = build_queue_framing_summary_from_projection(&runtime_projection, 96)
            .expect("queue framing summary should exist");

        assert_eq!(
            summary,
            "now: Review overlays  |  next: none  |  proposed: Promote follow-up proposal  |  blocked: none"
        );
    }
    #[test]
    fn queue_framing_summary_uses_structured_projection_for_proposals_and_blocked_work() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "Planning Context".to_string(),
            "legacy queue summary should not override structured projection".to_string(),
            Some("legacy proposal summary".to_string()),
            None,
            PriorityQueueProjection {
                next_task: Some(queue_task("task-1", "Current task", 1)),
                active_tasks: vec![
                    queue_task("task-1", "Current task", 1),
                    queue_task("task-2", "Next task", 2),
                    queue_task("task-3", "Later task", 3),
                ],
                proposed_tasks: vec![
                    queue_task("proposal-1", "First proposal", 1),
                    queue_task("proposal-2", "Second proposal", 2),
                ],
                skipped_tasks: vec![
                    skipped_task("blocked-1", "Blocked task", "dependency-open(ready)"),
                    skipped_task(
                        "blocked-2",
                        "Paused task",
                        "direction direction-b is paused",
                    ),
                ],
            },
        );
        let summary = build_queue_framing_summary_from_projection(&runtime_projection, 96)
            .expect("queue framing summary should exist");

        assert_eq!(
            summary,
            "now: Current task  |  next: Next task (+1 more)  |  proposed: First proposal (+1 more)  |  blocked: Blocked task (dependency-open(ready)) (+1 more)"
        );
    }
    #[test]
    fn compact_queue_framing_summary_fills_missing_fields_with_none() {
        assert_eq!(
            compact_queue_framing_summary("now: Review overlays", 96),
            "now: Review overlays  |  next: none  |  proposed: none  |  blocked: none"
        );
    }

    #[test]
    fn legacy_valid_planning_summary_prefix_is_removed_for_tui_surfaces() {
        assert_eq!(
            remove_legacy_valid_planning_summary_prefix("planning: valid".to_string()),
            None
        );
        assert_eq!(
            remove_legacy_valid_planning_summary_prefix(
                "planning: valid  |  queue: queue head: rank 1 / task-1".to_string()
            )
            .as_deref(),
            Some("queue: queue head: rank 1 / task-1")
        );
        assert_eq!(
            remove_legacy_valid_planning_summary_prefix("planning: valid  |  ".to_string()),
            None
        );
        assert_eq!(
            remove_legacy_valid_planning_summary_prefix(
                "planning: invalid  |  failure: missing result-output.md".to_string()
            )
            .as_deref(),
            Some("planning: invalid  |  failure: missing result-output.md")
        );
    }

    #[test]
    fn queue_framing_lines_hide_empty_none_only_rows() {
        let idle_projection = PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "now: none  |  next: none  |  proposed: none  |  blocked: none".to_string(),
            None,
            None,
        )
        .with_workspace_present(true);
        let idle_lines = build_queue_framing_lines_from_projection(&idle_projection, 96);
        assert!(idle_lines.is_empty());

        let blocked_projection = PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "now: none  |  next: none  |  proposed: none  |  blocked: Follow blocked review"
                .to_string(),
            None,
            None,
        )
        .with_workspace_present(true);
        let blocked_lines = build_queue_framing_lines_from_projection(&blocked_projection, 96)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            blocked_lines,
            vec!["proposed: none  |  blocked: Follow blocked review"]
        );
    }
    fn queue_task(task_id: &str, title: &str, rank: usize) -> PriorityQueueTask {
        PriorityQueueTask {
            rank,
            task_id: task_id.to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction".to_string(),
            task_title: title.to_string(),
            status: TaskStatus::Ready,
            combined_priority: 100,
            updated_at: "2026-04-17T00:00:00Z".to_string(),
            rank_reasons: vec!["test".to_string()],
        }
    }
    fn skipped_task(task_id: &str, title: &str, reason: &str) -> PriorityQueueSkippedTask {
        PriorityQueueSkippedTask {
            task_id: task_id.to_string(),
            task_title: title.to_string(),
            direction_id: "direction-1".to_string(),
            status: TaskStatus::Blocked,
            reason: reason.to_string(),
        }
    }
}
