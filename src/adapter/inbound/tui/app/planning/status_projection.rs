use super::super::{ConversationViewModel, NativeTuiApp};
use crate::application::service::planning::{
    PlanningRuntimeRepairAttempt, PlanningRuntimeSnapshot, PlanningRuntimeStatusProjectionRequest,
    PlanningRuntimeSummaryLineRequest,
};
use crate::domain::text::compact_whitespace_detail;
use ratatui::text::Line;

const RESUMED_SESSION_DETAIL_LIMIT: usize = 96;
const STATUS_SEGMENT_SEPARATOR: &str = "  |  ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanningStatusSurfaceProjection {
    pub(crate) summary_line: Option<String>,
    pub(crate) notice_line: Option<String>,
    pub(crate) queue_framing_lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanningFollowupSurfaceProjection {
    pub(crate) planning_status_line: String,
    pub(crate) repair_attempt_line: Option<String>,
    pub(crate) queue_head_line: Option<String>,
    pub(crate) proposal_line: Option<String>,
    pub(crate) failure_line: Option<String>,
    pub(crate) notice_line: Option<String>,
}

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
    PlanningStatusSurfaceProjection {
        summary_line: build_planning_summary_line(
            app,
            conversation,
            summary_detail_len,
            always_show,
        ),
        notice_line: build_planning_notice_line(conversation, supplemental_detail_len),
        queue_framing_lines: build_queue_framing_lines(conversation, supplemental_detail_len),
    }
}

pub(crate) fn build_planning_followup_surface_projection(
    app: &NativeTuiApp,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> PlanningFollowupSurfaceProjection {
    let followup_projection = app.planning.runtime.build_followup_status_projection(
        PlanningRuntimeStatusProjectionRequest {
            snapshot: &conversation.planning_runtime_snapshot,
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
            max_detail_len,
        },
    );

    PlanningFollowupSurfaceProjection {
        planning_status_line: followup_projection.planning_status_line,
        repair_attempt_line: followup_projection.repair_attempt_line,
        queue_head_line: followup_projection.queue_head_line,
        proposal_line: followup_projection.proposal_line,
        failure_line: followup_projection.failure_line,
        notice_line: build_planning_notice_line(conversation, max_detail_len),
    }
}

pub(crate) fn build_resumed_session_status_text(snapshot: &PlanningRuntimeSnapshot) -> String {
    let mut status_text = format!(
        "thread loaded / planning status: {}",
        snapshot.preview_status_label()
    );

    if let Some(queue_summary) =
        build_queue_framing_summary_from_snapshot(snapshot, RESUMED_SESSION_DETAIL_LIMIT)
    {
        status_text.push_str(" / queue summary: ");
        status_text.push_str(&queue_summary);
    } else if let Some(detail) = snapshot.preview_detail() {
        status_text.push_str(" / planning detail: ");
        status_text.push_str(&compact_whitespace_detail(
            detail,
            RESUMED_SESSION_DETAIL_LIMIT,
        ));
    }

    status_text
}

pub(crate) fn build_planning_summary_line(
    app: &NativeTuiApp,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
    always_show: bool,
) -> Option<String> {
    app.planning
        .runtime
        .build_summary_line(PlanningRuntimeSummaryLineRequest {
            snapshot: &conversation.planning_runtime_snapshot,
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
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Vec<Line<'static>> {
    build_queue_framing_lines_from_snapshot(&conversation.planning_runtime_snapshot, max_detail_len)
}

pub(crate) fn build_queue_framing_lines_from_snapshot(
    snapshot: &PlanningRuntimeSnapshot,
    max_detail_len: usize,
) -> Vec<Line<'static>> {
    build_queue_framing_details_from_snapshot(snapshot, max_detail_len)
        .map(|details| queue_framing_lines_from_details(&details))
        .unwrap_or_default()
}

pub(crate) fn build_queue_framing_summary_from_snapshot(
    snapshot: &PlanningRuntimeSnapshot,
    max_detail_len: usize,
) -> Option<String> {
    build_queue_framing_details_from_snapshot(snapshot, max_detail_len)
        .map(|details| queue_framing_summary_from_details(&details))
}

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

fn build_queue_framing_details_from_snapshot(
    snapshot: &PlanningRuntimeSnapshot,
    max_detail_len: usize,
) -> Option<QueueFramingDetails> {
    let queue_snapshot = snapshot.queue_snapshot();
    let has_queue_context = snapshot.workspace_present()
        || snapshot.queue_head().is_some()
        || snapshot.queue_summary().is_some()
        || snapshot.proposal_summary().is_some()
        || queue_snapshot.is_some();
    if !has_queue_context {
        return None;
    }

    let mut details = QueueFramingDetails {
        now_detail: "none".to_string(),
        next_detail: "none".to_string(),
        proposed_detail: "none".to_string(),
        blocked_detail: "none".to_string(),
    };

    if let Some(queue_snapshot) = queue_snapshot {
        let current_task = queue_snapshot
            .next_task
            .as_ref()
            .or_else(|| queue_snapshot.active_tasks.first())
            .or_else(|| snapshot.queue_head());
        let now_detail = current_task
            .map(|task| compact_queue_task_summary(task.task_title.as_str(), 1, 1, max_detail_len))
            .unwrap_or_else(|| "none".to_string());

        let remaining_tasks = current_task
            .map(|current| {
                queue_snapshot
                    .active_tasks
                    .iter()
                    .filter(|task| task.task_id != current.task_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| queue_snapshot.active_tasks.iter().collect::<Vec<_>>());
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

        let proposed_detail = queue_snapshot
            .proposed_tasks
            .first()
            .map(|task| {
                compact_queue_task_summary(
                    task.task_title.as_str(),
                    queue_snapshot.proposed_tasks.len(),
                    1,
                    max_detail_len,
                )
            })
            .or_else(|| {
                snapshot
                    .proposal_summary()
                    .map(|summary| compact_proposal_summary_detail(summary, max_detail_len))
            })
            .unwrap_or_else(|| "none".to_string());

        let blocked_detail = queue_snapshot
            .skipped_tasks
            .first()
            .map(|task| {
                let title = compact_whitespace_detail(task.task_title.as_str(), max_detail_len);
                let reason = compact_whitespace_detail(task.reason.as_str(), max_detail_len);
                let mut summary = format!("{title} ({reason})");
                let hidden_count = queue_snapshot.skipped_tasks.len().saturating_sub(1);
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

    if let Some(queue_head) = snapshot.queue_head() {
        details.now_detail =
            compact_queue_task_summary(queue_head.task_title.as_str(), 1, 1, max_detail_len);
    }

    if let Some(queue_summary) = snapshot.queue_summary() {
        if let Some(parsed_details) = parse_queue_framing_details(queue_summary, max_detail_len) {
            merge_queue_framing_details(&mut details, parsed_details);
        }
    }

    if let Some(proposal_summary) = snapshot.proposal_summary() {
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
    vec![
        Line::from(format!(
            "now: {}{STATUS_SEGMENT_SEPARATOR}next: {}",
            details.now_detail, details.next_detail
        )),
        Line::from(format!(
            "proposed: {}{STATUS_SEGMENT_SEPARATOR}blocked: {}",
            details.proposed_detail, details.blocked_detail
        )),
    ]
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
        build_queue_framing_summary_from_snapshot, build_resumed_session_status_text,
        compact_queue_framing_summary,
    };
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    use crate::domain::planning::{PriorityQueueSnapshot, PriorityQueueTask, TaskStatus};

    #[test]
    fn resumed_session_status_prefers_queue_summary_projection() {
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: Ship resume status  |  next: Review overlays  |  proposed: none  |  blocked: none"
                .to_string(),
            None,
            None,
        );

        let status_text = build_resumed_session_status_text(&snapshot);

        assert!(status_text.contains("thread loaded / planning status: ready"));
        assert!(status_text.contains("queue summary: now: Ship resume status"));
        assert!(status_text.contains("next: Review overlays"));
    }

    #[test]
    fn queue_framing_summary_skips_duplicate_next_when_snapshot_has_no_explicit_next_task() {
        let snapshot = PlanningRuntimeSnapshot::ready_with_queue_snapshot(
            "Planning Context".to_string(),
            "queue ready".to_string(),
            None,
            None,
            PriorityQueueSnapshot {
                next_task: None,
                active_tasks: vec![
                    queue_task("task-1", "Ship resume status", 1),
                    queue_task("task-2", "Review overlays", 2),
                ],
                proposed_tasks: Vec::new(),
                skipped_tasks: Vec::new(),
            },
        );

        let summary = build_queue_framing_summary_from_snapshot(&snapshot, 96)
            .expect("queue framing summary should exist");

        assert!(summary.contains("now: Ship resume status"));
        assert!(summary.contains("next: Review overlays"));
        assert!(!summary.contains("next: Ship resume status"));
    }

    #[test]
    fn queue_framing_summary_merges_partial_queue_summary_with_existing_details() {
        let snapshot = PlanningRuntimeSnapshot::ready_with_details(
            "Planning Context".to_string(),
            "now: Review overlays".to_string(),
            Some("Promote follow-up proposal".to_string()),
            None,
        )
        .with_workspace_present(true);

        let summary = build_queue_framing_summary_from_snapshot(&snapshot, 96)
            .expect("queue framing summary should exist");

        assert_eq!(
            summary,
            "now: Review overlays  |  next: none  |  proposed: Promote follow-up proposal  |  blocked: none"
        );
    }

    #[test]
    fn compact_queue_framing_summary_fills_missing_fields_with_none() {
        assert_eq!(
            compact_queue_framing_summary("now: Review overlays", 96),
            "now: Review overlays  |  next: none  |  proposed: none  |  blocked: none"
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
}
