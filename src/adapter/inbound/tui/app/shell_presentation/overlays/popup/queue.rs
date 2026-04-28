use super::super::super::{
    AkraTheme, ConversationState, Line, NativeTuiApp, PriorityQueueSkippedTask, PriorityQueueTask,
    QUEUE_INSPECTION_NOTE_DETAIL_LIMIT, QUEUE_INSPECTION_PROPOSAL_LIMIT,
    QUEUE_INSPECTION_TASK_LIMIT, QUEUE_INSPECTION_TITLE_DETAIL_LIMIT, compact_whitespace_detail,
};
use super::QueueOverlayView;

pub(crate) fn build_queue_overlay_view(app: &NativeTuiApp) -> QueueOverlayView {
    let header_lines = vec![
        AkraTheme::title_line("Planning Queue", " / shell inspection"),
        Line::from("Review the next actionable work without opening raw planning artifacts."),
    ];

    match &app.conversation_state {
        ConversationState::Loading => QueueOverlayView {
            header_lines,
            summary_lines: vec![Line::from("status: loading conversation planning state")],
            queue_lines: vec![Line::from(
                "Queue inspection becomes available after the thread loads.",
            )],
            proposal_lines: vec![Line::from("Proposal data is unavailable while loading.")],
            note_lines: vec![Line::from("No planner notes yet.")],
            key_lines: build_queue_overlay_key_lines(),
        },
        ConversationState::Failed(message) => QueueOverlayView {
            header_lines,
            summary_lines: vec![Line::from("status: conversation unavailable")],
            queue_lines: vec![Line::from(
                "Queue inspection is unavailable while the conversation failed to load.",
            )],
            proposal_lines: vec![Line::from(
                "Open a new draft or reload a session to restore planning state.",
            )],
            note_lines: vec![Line::from(format!(
                "conversation error: {}",
                compact_whitespace_detail(message, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            ))],
            key_lines: build_queue_overlay_key_lines(),
        },
        ConversationState::Ready(conversation) => {
            let snapshot = &conversation.planning_runtime_snapshot;
            let queue_projection = snapshot.queue_projection();
            let queue_lines = queue_projection
                .map(|queue_projection| {
                    build_queue_task_lines(
                        &queue_projection.active_tasks,
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
            let proposal_lines = queue_projection
                .map(|queue_projection| {
                    build_queue_task_lines(
                        &queue_projection.proposed_tasks,
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

            let mut summary_segments = Vec::new();
            if let Some(queue_head) = snapshot.queue_head() {
                summary_segments.push(format!(
                    "next: {}",
                    compact_whitespace_detail(
                        queue_head.task_title.trim(),
                        QUEUE_INSPECTION_TITLE_DETAIL_LIMIT
                    )
                ));
            }
            if let Some(queue_summary) = snapshot.queue_summary() {
                summary_segments.push(format!(
                    "queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
                if snapshot.queue_head().is_none() {
                    summary_segments
                        .push(format!("policy: {}", snapshot.queue_idle_policy().label()));
                }
            }
            if let Some(proposal_summary) = snapshot.proposal_summary() {
                summary_segments.push(format!(
                    "proposals: {}",
                    compact_whitespace_detail(proposal_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
            }
            if summary_segments.is_empty() {
                summary_segments.push(format!("status: {}", snapshot.preview_status_label()));
            }
            let summary_lines = vec![Line::from(summary_segments.join("  |  "))];

            let mut note_lines = Vec::new();
            if let Some(detail) = snapshot.auto_followup_pause_reason() {
                note_lines.push(Line::from(format!(
                    "pause: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            } else if let Some(detail) = snapshot.failure_reason() {
                note_lines.push(Line::from(format!(
                    "blocking issue: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(summary) =
                conversation.planning_notice_summary(QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            {
                note_lines.push(Line::from(format!("planning notice: {summary}")));
            }
            if let Some(queue_summary) =
                app.planner_worker_panel_state.last_queue_summary.as_deref()
            {
                note_lines.push(Line::from(format!(
                    "planner queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = app.planner_worker_panel_state.last_host_detail.as_deref() {
                note_lines.push(Line::from(format!(
                    "planner host detail: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = queue_projection.and_then(|queue_projection| {
                build_skipped_queue_note_line(&queue_projection.skipped_tasks)
            }) {
                note_lines.push(detail);
            }
            if note_lines.is_empty() {
                note_lines.push(Line::from("No planner notices or skipped queue items."));
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

fn build_queue_overlay_key_lines() -> Vec<Line<'static>> {
    vec![AkraTheme::key_line(
        "Esc/Ctrl+C: close  |  :planning: update files",
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
