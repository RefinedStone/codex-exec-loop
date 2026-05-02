use super::super::super::{
    AkraTheme, ConversationState, Line, NativeTuiApp, PriorityQueueSkippedTask, PriorityQueueTask,
    QUEUE_INSPECTION_NOTE_DETAIL_LIMIT, QUEUE_INSPECTION_PROPOSAL_LIMIT,
    QUEUE_INSPECTION_TASK_LIMIT, QUEUE_INSPECTION_TITLE_DETAIL_LIMIT, compact_whitespace_detail,
};
use super::QueueOverlayView;

pub(crate) fn build_queue_overlay_view(app: &NativeTuiApp) -> QueueOverlayView {
    /*
     * 학습 주석: queue overlay는 planning runtime snapshot을 사람이 훑기 쉬운 5개 column(header/summary/queue/proposal/note)
     * view model로 접습니다. 이 파일은 도메인 판단을 다시 하지 않고, 이미 계산된 queue projection과 snapshot label을
     * shell popup의 제한된 줄 수와 폭에 맞게 압축하는 presentation adapter입니다.
     */
    let header_lines = vec![
        AkraTheme::title_line("Planning Queue", " / shell inspection"),
        Line::from("Review the next actionable work without opening raw planning artifacts."),
    ];

    match &app.conversation_state {
        ConversationState::Loading => QueueOverlayView {
            header_lines,
            // 학습 주석: Loading 상태에서는 planning snapshot이 아직 없으므로 모든 section을 "준비 전" copy로 고정합니다.
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
            // 학습 주석: conversation load 실패는 planning failure와 다르므로 queue 자체를 추측하지 않고 load error만 노출합니다.
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
            // 학습 주석: Ready conversation만 planning snapshot을 갖고, popup은 이 snapshot의 read model만 소비합니다.
            let snapshot = &conversation.planning_runtime_snapshot;
            let queue_projection = snapshot.queue_projection();
            // 학습 주석: 새 projection이 있으면 active queue 전체 preview를 쓰고, 없으면 legacy queue_head로 한 줄 fallback합니다.
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
            // 학습 주석: proposals는 실행 queue와 다른 lane이라 별도 section으로 두어 promote intent를 유도합니다.
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

            // 학습 주석: summary는 popup 첫 시선에 필요한 next/queue/proposal 상태만 한 줄로 합칩니다.
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
                // 학습 주석: queue/proposal 요약이 모두 없으면 preview status가 popup의 가장 압축된 상태 설명입니다.
                summary_segments.push(format!("status: {}", snapshot.preview_status_label()));
            }
            let summary_lines = vec![Line::from(summary_segments.join("  |  "))];

            // 학습 주석: note section은 operator가 바로 조치할 수 있는 pause/failure/planner host 정보를 우선 배치합니다.
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
                // 학습 주석: popup height를 보호하기 위해 note는 가장 먼저 모은 두 줄만 보여 줍니다.
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

    // 학습 주석: queue/proposal row는 rank, 상태, combined priority, title만 남겨 popup scan 비용을 낮춥니다.
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
    // 학습 주석: skipped list 전체 대신 첫 reason과 개수만 보여 queue가 왜 줄어든 것처럼 보이는지 설명합니다.
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
