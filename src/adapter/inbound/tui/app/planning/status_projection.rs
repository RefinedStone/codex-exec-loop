use super::super::ConversationViewModel;
use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningRuntimeProjection, PlanningRuntimeRepairAttempt,
    PlanningRuntimeSummaryLineRequest, build_planning_runtime_summary_line,
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
    runtime_projection: &PlanningRuntimeProjection,
    conversation: &ConversationViewModel,
    summary_detail_len: usize,
    supplemental_detail_len: usize,
    always_show: bool,
) -> PlanningStatusSurfaceProjection {
    let queue_framing_details =
        build_queue_framing_details_from_projection(runtime_projection, supplemental_detail_len);
    let queue_framing_lines = queue_framing_details
        .as_ref()
        .map(queue_framing_lines_from_details)
        .unwrap_or_default();
    let mut summary_line = build_planning_summary_line(
        conversation,
        runtime_projection,
        summary_detail_len,
        always_show,
    );
    if queue_framing_details.is_some()
        && let Some(summary) = summary_line.take()
    {
        summary_line = remove_duplicate_queue_framing_segments(summary);
    }
    PlanningStatusSurfaceProjection {
        summary_line,
        notice_line: build_planning_notice_line(conversation, supplemental_detail_len),
        queue_framing_lines,
    }
}

// Resumed sessions need status text before the full inline shell has rendered.
// Prefer the queue summary because it gives operators immediate handoff context;
// fall back to the runtime detail only when no queue framing is available.
pub(crate) fn build_resumed_session_status_text(
    runtime_projection: &PlanningRuntimeProjection,
    resumed_thread_review_summary: Option<&str>,
    resumed_thread_manual_handoff_context: Option<&str>,
) -> String {
    let mut status_text = format!(
        "thread loaded / planning status: {}",
        runtime_projection.preview_status_label()
    );
    append_resumed_status_detail(&mut status_text, "review", resumed_thread_review_summary);
    append_resumed_status_detail(
        &mut status_text,
        "manual handoff",
        resumed_thread_manual_handoff_context,
    );
    let queue_framing_details = build_queue_framing_details_from_projection(
        runtime_projection,
        RESUMED_SESSION_DETAIL_LIMIT,
    );
    let queue_summary = queue_framing_details
        .as_ref()
        .map(queue_framing_lines_from_details)
        .map(|lines| {
            lines
                .into_iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join(STATUS_SEGMENT_SEPARATOR)
        })
        .filter(|summary| !summary.is_empty());
    if let Some(queue_summary) = queue_summary {
        status_text.push_str(" / queue summary: ");
        status_text.push_str(&queue_summary);
    } else if queue_framing_details.is_none()
        && let Some(detail) = runtime_projection.preview_detail()
    {
        status_text.push_str(" / planning detail: ");
        status_text.push_str(&compact_whitespace_detail(
            detail,
            RESUMED_SESSION_DETAIL_LIMIT,
        ));
    }

    status_text
}

fn append_resumed_status_detail(status_text: &mut String, label: &str, detail: Option<&str>) {
    let Some(detail) = detail.filter(|detail| !detail.trim().is_empty()) else {
        return;
    };
    status_text.push_str(" / ");
    status_text.push_str(label);
    status_text.push_str(": ");
    status_text.push_str(&compact_whitespace_detail(
        detail,
        RESUMED_SESSION_DETAIL_LIMIT,
    ));
}

// Summary generation stays delegated to the pure planning policy so the TUI
// does not duplicate readiness/repair wording. The adapter only contributes
// shell context: whether a turn is running, whether a repair is in flight,
// and whether a separate notice line already exists.
pub(crate) fn build_planning_summary_line(
    conversation: &ConversationViewModel,
    runtime_projection: &PlanningRuntimeProjection,
    max_detail_len: usize,
    always_show: bool,
) -> Option<String> {
    build_planning_runtime_summary_line(PlanningRuntimeSummaryLineRequest {
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
    let trimmed = summary_line.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn remove_duplicate_queue_framing_segments(summary_line: String) -> Option<String> {
    let retained = summary_line
        .split(STATUS_SEGMENT_SEPARATOR)
        .map(str::trim)
        .take_while(|segment| !segment.starts_with("queue:") && !segment.starts_with("proposals:"))
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    (!retained.is_empty()).then(|| retained.join(STATUS_SEGMENT_SEPARATOR))
}

pub(crate) fn build_planning_notice_line(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<String> {
    conversation
        .planning_notice_summary(max_detail_len)
        .map(|summary| format!("planning notice: {summary}"))
}

#[cfg(test)]
pub(crate) fn build_queue_framing_lines_from_projection(
    runtime_projection: &PlanningRuntimeProjection,
    max_detail_len: usize,
) -> Vec<Line<'static>> {
    build_queue_framing_details_from_projection(runtime_projection, max_detail_len)
        .map(|details| queue_framing_lines_from_details(&details))
        .unwrap_or_default()
}

#[cfg(test)]
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
            .or_else(|| {
                let has_other_actionable_detail = !projection.proposed_tasks.is_empty()
                    || projection
                        .skipped_tasks
                        .iter()
                        .any(|task| !task.status.is_terminal());
                if has_other_actionable_detail {
                    return None;
                }
                projection.queue_summary.as_deref().and_then(|summary| {
                    if let Some(parsed) = parse_queue_framing_details(summary, max_detail_len) {
                        return parsed.now_detail.filter(|detail| detail_has_signal(detail));
                    }
                    detail_has_signal(summary)
                        .then(|| compact_whitespace_detail(summary, max_detail_len))
                })
            })
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
        let blocked_tasks = projection
            .skipped_tasks
            .iter()
            .filter(|task| !task.status.is_terminal())
            .collect::<Vec<_>>();
        let blocked_detail = blocked_tasks
            .first()
            .map(|task| {
                let title = compact_whitespace_detail(task.task_title.as_str(), max_detail_len);
                let reason = compact_whitespace_detail(task.reason.as_str(), max_detail_len);
                let mut summary = format!("{title} ({reason})");
                let hidden_count = blocked_tasks.len().saturating_sub(1);
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
    if let Some(queue_summary) = projection.queue_summary.as_deref() {
        if let Some(parsed_details) = parse_queue_framing_details(queue_summary, max_detail_len) {
            merge_queue_framing_details(&mut details, parsed_details);
        } else if !detail_has_signal(&details.now_detail) {
            details.now_detail = compact_whitespace_detail(queue_summary, max_detail_len);
        }
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
    let current = [
        ("now", details.now_detail.as_str()),
        ("next", details.next_detail.as_str()),
    ]
    .into_iter()
    .filter(|(_, detail)| detail_has_signal(detail))
    .map(|(label, detail)| format!("{label}: {detail}"))
    .collect::<Vec<_>>();
    if !current.is_empty() {
        lines.push(Line::from(current.join(STATUS_SEGMENT_SEPARATOR)));
    }
    let follow_up = [
        ("proposed", details.proposed_detail.as_str()),
        ("blocked", details.blocked_detail.as_str()),
    ]
    .into_iter()
    .filter(|(_, detail)| detail_has_signal(detail))
    .map(|(label, detail)| format!("{label}: {detail}"))
    .collect::<Vec<_>>();
    if !follow_up.is_empty() {
        lines.push(Line::from(follow_up.join(STATUS_SEGMENT_SEPARATOR)));
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
        build_planning_status_surface_projection, build_queue_framing_lines_from_projection,
        build_queue_framing_summary_from_projection, build_resumed_session_status_text,
        compact_queue_framing_summary, remove_duplicate_queue_framing_segments,
        remove_legacy_valid_planning_summary_prefix,
    };
    use crate::adapter::inbound::tui::app::ConversationState;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
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
        let status_text = build_resumed_session_status_text(&runtime_projection, None, None);

        assert!(status_text.contains("thread loaded / planning status: ready"));
        assert!(status_text.contains("queue summary: now: Ship resume status"));
        assert!(status_text.contains("next: Review overlays"));
    }
    #[test]
    fn resumed_session_status_includes_repository_review_summary_and_manual_handoff_context() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "queue summary unavailable".to_string(),
            None,
            None,
        );
        let status_text = build_resumed_session_status_text(
            &runtime_projection,
            Some("manual handoff (waiting): operator follow-up required"),
            Some("operator: open review center inbox"),
        );

        assert!(
            status_text.contains("review: manual handoff (waiting): operator follow-up required")
        );
        assert!(status_text.contains("manual handoff: operator: open review center inbox"));
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
                "  planning: invalid  |  failure: missing result-output.md  ".to_string()
            )
            .as_deref(),
            Some("planning: invalid  |  failure: missing result-output.md")
        );
        assert_eq!(
            remove_legacy_valid_planning_summary_prefix("   ".to_string()),
            None
        );
    }

    #[test]
    fn duplicate_queue_segment_is_removed_without_hiding_planning_health() {
        assert_eq!(
            remove_duplicate_queue_framing_segments(
                "planning: stale  |  queue: now: task-1  |  next: task-2  |  proposed: none  |  blocked: none  |  proposals: 2 promotable".to_string()
            )
            .as_deref(),
            Some("planning: stale")
        );
        assert_eq!(
            remove_duplicate_queue_framing_segments(
                "queue: queue head: rank 1 / task-1  |  proposals: 2 promotable".to_string()
            ),
            None
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
        assert_eq!(blocked_lines, vec!["blocked: Follow blocked review"]);
    }

    #[test]
    fn planning_surface_hides_none_only_queue_summary_segments() {
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::ready_with_details(
                "Planning Context".to_string(),
                "now: none  |  next: none  |  proposed: none  |  blocked: none".to_string(),
                None,
                None,
            )
            .with_workspace_present(true),
        );
        let runtime_projection = app.planning_runtime_projection_snapshot();
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("test app should keep a ready conversation");
        };

        let surface = build_planning_status_surface_projection(
            &runtime_projection,
            conversation,
            96,
            96,
            true,
        );

        assert!(surface.queue_framing_lines.is_empty());
        assert!(
            surface.summary_line.as_deref().is_none_or(
                |summary| !summary.contains("queue:") && !summary.contains("proposals:")
            )
        );
    }

    #[test]
    fn planning_surface_preserves_unstructured_queue_status_in_framing() {
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::ready_with_details(
                "Planning Context".to_string(),
                "queue idle: no executable planning task".to_string(),
                Some("2 promotable follow-up proposals".to_string()),
                None,
            )
            .with_workspace_present(true),
        );
        let runtime_projection = app.planning_runtime_projection_snapshot();
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("test app should keep a ready conversation");
        };

        let surface = build_planning_status_surface_projection(
            &runtime_projection,
            conversation,
            96,
            96,
            true,
        );
        let framing = surface
            .queue_framing_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(framing.contains("now: queue idle: no executable planning task"));
        assert!(framing.contains("proposed: 2 promotable follow-up proposals"));
        assert!(
            surface.summary_line.as_deref().is_none_or(
                |summary| !summary.contains("queue:") && !summary.contains("proposals:")
            )
        );
    }

    #[test]
    fn planning_surface_preserves_structured_idle_status_after_terminal_skips() {
        let mut completed = skipped_task("done-1", "Completed task", "status done");
        completed.status = TaskStatus::Done;
        let mut app = test_native_tui_app();
        app.sync_ready_conversation_planning_runtime_projection(
            PlanningRuntimeProjection::ready_with_queue_projection(
                "Planning Context".to_string(),
                "queue idle: no executable planning task".to_string(),
                None,
                None,
                PriorityQueueProjection {
                    next_task: None,
                    active_tasks: Vec::new(),
                    proposed_tasks: Vec::new(),
                    skipped_tasks: vec![completed],
                },
            ),
        );
        let runtime_projection = app.planning_runtime_projection_snapshot();
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("test app should keep a ready conversation");
        };

        let surface = build_planning_status_surface_projection(
            &runtime_projection,
            conversation,
            96,
            96,
            true,
        );
        let framing = surface
            .queue_framing_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert_eq!(
            framing,
            vec!["now: queue idle: no executable planning task"]
        );
        assert!(
            surface
                .summary_line
                .as_deref()
                .is_none_or(|summary| !summary.contains("queue:"))
        );
    }

    #[test]
    fn compact_queue_framing_hides_terminal_skips_and_none_placeholders() {
        let mut completed = skipped_task("done-1", "Completed sushi task", "status done");
        completed.status = TaskStatus::Done;
        let mut cancelled = skipped_task("cancelled-1", "Cancelled task", "status cancelled");
        cancelled.status = TaskStatus::Cancelled;
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue head: rank 1 / task-1".to_string(),
            None,
            None,
            PriorityQueueProjection {
                next_task: Some(queue_task("task-1", "Current task", 1)),
                active_tasks: vec![queue_task("task-1", "Current task", 1)],
                proposed_tasks: Vec::new(),
                skipped_tasks: vec![completed, cancelled],
            },
        );
        let lines = build_queue_framing_lines_from_projection(&runtime_projection, 96)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert_eq!(lines, vec!["now: Current task"]);
    }

    #[test]
    fn compact_queue_framing_counts_only_actionable_blockers() {
        let mut completed = skipped_task("done-1", "Completed task", "status done");
        completed.status = TaskStatus::Done;
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue ready".to_string(),
            None,
            None,
            PriorityQueueProjection {
                next_task: None,
                active_tasks: Vec::new(),
                proposed_tasks: Vec::new(),
                skipped_tasks: vec![
                    skipped_task("blocked-1", "Real blocker", "dependency open"),
                    completed,
                ],
            },
        );
        let lines = build_queue_framing_lines_from_projection(&runtime_projection, 96)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert_eq!(lines, vec!["blocked: Real blocker (dependency open)"]);
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
