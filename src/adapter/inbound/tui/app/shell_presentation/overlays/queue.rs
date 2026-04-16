use super::super::{
    Color, ConversationState, Line, Modifier, NativeTuiApp, PriorityQueueSkippedTask,
    PriorityQueueTask, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT, QUEUE_INSPECTION_PROPOSAL_LIMIT,
    QUEUE_INSPECTION_TASK_LIMIT, QUEUE_INSPECTION_TITLE_DETAIL_LIMIT, Span, Style,
    build_automation_key_lines, build_automation_list_view, build_automation_preview_lines,
    build_automation_status_lines, compact_whitespace_detail,
};
use super::{AutomationOverlayView, QueueOverlayView};
use crate::adapter::inbound::tui::app::planning::build_planning_notice_line;
use crate::application::service::planning::{
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjectionRequest,
};

pub(crate) fn build_queue_overlay_view(app: &NativeTuiApp) -> QueueOverlayView {
    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                "Planning Queue",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" / shell inspection"),
        ]),
        Line::from("Review the next actionable work without opening raw planning files."),
    ];

    match &app.conversation_state {
        ConversationState::Loading => QueueOverlayView {
            header_lines,
            summary_lines: queue_overlay_loading_summary_lines(),
            queue_lines: vec![Line::from(
                "Queued work appears after the thread finishes loading.",
            )],
            proposal_lines: vec![Line::from(
                "Proposal data appears after the thread finishes loading.",
            )],
            note_lines: vec![Line::from(
                "Planner updates appear after the thread finishes loading.",
            )],
            key_lines: build_queue_overlay_key_lines(),
        },
        ConversationState::Failed(message) => QueueOverlayView {
            header_lines,
            summary_lines: queue_overlay_failed_summary_lines(),
            queue_lines: vec![Line::from(
                "Queued work is unavailable because the conversation failed to load.",
            )],
            proposal_lines: vec![Line::from(
                "Reload the session or open a new draft to restore queued work.",
            )],
            note_lines: vec![Line::from(format!(
                "conversation error: {}",
                compact_whitespace_detail(message, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            ))],
            key_lines: build_queue_overlay_key_lines(),
        },
        ConversationState::Ready(conversation) => {
            let snapshot = &conversation.planning_runtime_snapshot;
            let queue_snapshot = snapshot.queue_snapshot();
            let planning_projection = app.planning.runtime.build_followup_status_projection(
                PlanningRuntimeStatusProjectionRequest {
                    snapshot,
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
                    max_detail_len: QUEUE_INSPECTION_NOTE_DETAIL_LIMIT,
                },
            );
            let queue_lines = queue_snapshot
                .map(|queue_snapshot| {
                    build_queue_task_lines(
                        &queue_snapshot.active_tasks,
                        "No executable tasks in the current planning queue.",
                        QUEUE_INSPECTION_TASK_LIMIT,
                    )
                })
                .unwrap_or_else(|| match snapshot.queue_head() {
                    Some(queue_head) => build_queue_task_lines(
                        std::slice::from_ref(queue_head),
                        "No executable tasks in the current planning queue.",
                        1,
                    ),
                    None => vec![Line::from(
                        "No executable tasks in the current planning queue.",
                    )],
                });
            let proposal_lines = queue_snapshot
                .map(|queue_snapshot| {
                    build_queue_task_lines(
                        &queue_snapshot.proposed_tasks,
                        "No promotable proposals are queued right now.",
                        QUEUE_INSPECTION_PROPOSAL_LIMIT,
                    )
                })
                .unwrap_or_else(|| {
                    if let Some(summary) = snapshot.proposal_summary() {
                        vec![Line::from(format!(
                            "proposals: {}",
                            compact_whitespace_detail(summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                        ))]
                    } else {
                        vec![Line::from("No promotable proposals are queued right now.")]
                    }
                });
            let summary_lines = vec![
                Line::from(planning_projection.current_state_line),
                Line::from(planning_projection.cause_line),
                Line::from(planning_projection.next_action_line),
            ];

            let mut note_lines = Vec::new();
            if let Some(planning_notice_line) =
                build_planning_notice_line(conversation, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            {
                note_lines.push(Line::from(planning_notice_line));
            }
            if let Some(queue_summary) =
                app.planner_worker_panel_state.last_queue_summary.as_deref()
            {
                note_lines.push(Line::from(format!(
                    "queued work: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = app.planner_worker_panel_state.last_host_detail.as_deref() {
                note_lines.push(Line::from(format!(
                    "operator action: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = queue_snapshot.and_then(|queue_snapshot| {
                build_skipped_queue_note_line(&queue_snapshot.skipped_tasks)
            }) {
                note_lines.push(detail);
            }
            if note_lines.is_empty() {
                note_lines.push(Line::from("No planner updates or skipped queue items."));
            } else {
                note_lines.truncate(2);
            }

            QueueOverlayView {
                header_lines,
                summary_lines,
                queue_lines,
                proposal_lines,
                note_lines,
                key_lines: build_queue_overlay_key_lines(),
            }
        }
    }
}

pub(crate) fn build_automation_overlay_view(app: &NativeTuiApp) -> AutomationOverlayView {
    AutomationOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Automation Controls",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / shell inspection"),
            ]),
            Line::from("Inspect planning-driven automation before the next auto-follow turn."),
        ],
        list_view: build_automation_list_view(app),
        preview_lines: build_automation_preview_lines(app),
        status_lines: build_automation_status_lines(app),
        key_lines: build_automation_key_lines(app),
    }
}

fn build_queue_overlay_key_lines() -> Vec<Line<'static>> {
    vec![Line::from(
        "Esc/Ctrl+C: close  |  :planning: update files  |  Ctrl+f/Ctrl+a: automation controls",
    )]
}

fn build_queue_task_lines(
    tasks: &[PriorityQueueTask],
    empty_message: &str,
    max_visible_tasks: usize,
) -> Vec<Line<'static>> {
    if tasks.is_empty() {
        return vec![Line::from(empty_message.to_string())];
    }

    let mut lines = Vec::new();
    for task in tasks.iter().take(max_visible_tasks) {
        lines.push(Line::from(format!(
            "#{} [{} / p{}] {}",
            task.rank,
            task.status.label(),
            task.combined_priority,
            compact_whitespace_detail(task.task_title.trim(), QUEUE_INSPECTION_TITLE_DETAIL_LIMIT)
        )));
    }

    let hidden_count = tasks.len().saturating_sub(max_visible_tasks);
    if hidden_count > 0 {
        lines.push(Line::from(format!(
            "+{hidden_count} more queue item{} hidden for readability",
            if hidden_count == 1 { "" } else { "s" }
        )));
    }

    lines
}

fn queue_overlay_loading_summary_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("current state: waiting"),
        Line::from("cause: conversation planning state is still loading"),
        Line::from("next action: wait for the thread to finish loading"),
    ]
}

fn queue_overlay_failed_summary_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("current state: blocked"),
        Line::from(
            "cause: conversation planning state is unavailable because the thread failed to load",
        ),
        Line::from("next action: reload the session or open a new draft"),
    ]
}

fn build_skipped_queue_note_line(
    skipped_tasks: &[PriorityQueueSkippedTask],
) -> Option<Line<'static>> {
    let first_skipped = skipped_tasks.first()?;
    Some(Line::from(format!(
        "skipped tasks: {} / {}",
        skipped_tasks.len(),
        compact_whitespace_detail(
            first_skipped.reason.as_str(),
            QUEUE_INSPECTION_NOTE_DETAIL_LIMIT
        )
    )))
}
