use super::super::super::{
    AkraTheme, ConversationState, Line, NativeTuiApp, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT,
    QUEUE_INSPECTION_PROPOSAL_LIMIT, QUEUE_INSPECTION_TASK_LIMIT,
    QUEUE_INSPECTION_TITLE_DETAIL_LIMIT, compact_whitespace_detail,
};
use super::QueueOverlayView;
use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningApplicationQueueTask, PlanningApplicationSkippedTask,
};

pub(crate) fn build_queue_overlay_view(app: &NativeTuiApp) -> QueueOverlayView {
    /*
     * Queue overlay는 PlanningApplicationProjection을 popup renderer가 바로 배치할 수 있는
     * header/summary/queue/proposal/note/key section으로 낮춘다. PriorityQueueService가 이미
     * active/proposed/skipped 분류와 rank를 계산했으므로, 이 파일은 queue 의미를 재판단하지 않고
     * 좁은 popup 폭에 맞춰 title/detail을 압축하는 presentation adapter로 남는다.
     */
    let header_lines = vec![
        AkraTheme::title_line("Planning Queue", " / shell inspection"),
        Line::from("Review the next actionable work without opening raw planning artifacts."),
    ];

    match &app.conversation_state {
        ConversationState::Loading => QueueOverlayView {
            header_lines,
            /*
             * Conversation이 아직 load 중이면 planning runtime projection 자체가 없다. 이 상태에서 queue/proposal
             * section을 추측하지 않고 "thread load 뒤 가능" copy로 고정해 stale planning data처럼 보이지 않게 한다.
             */
            summary_lines: vec![Line::from("status: loading conversation planning state")],
            queue_lines: vec![Line::from(
                "Queue inspection becomes available after the thread loads.",
            )],
            proposal_lines: vec![Line::from("Proposal data is unavailable while loading.")],
            note_lines: vec![Line::from("No planning worker notes yet.")],
            key_lines: build_queue_overlay_key_lines(),
        },
        ConversationState::Failed(message) => QueueOverlayView {
            header_lines,
            /*
             * Conversation load 실패는 planning queue failure와 다르다. queue projection을 만들 수 없는 상태라
             * queue 자체를 empty로 오해시키지 않고 load error와 recovery action만 보여 준다.
             */
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
            // Ready conversation state only gates availability. The runtime read model itself comes from core.
            let runtime_projection = app.planning_runtime_projection_snapshot();
            let projection =
                PlanningApplicationProjection::from_runtime_projection(&runtime_projection);
            /*
             * 새 queue projection이 있으면 active task preview 전체를 보여 준다. 오래된 projection이나
             * compatibility path처럼 projection이 없을 때만 legacy queue_head 한 줄로 fallback한다.
             */
            let queue_lines = if projection.has_structured_queue_projection {
                build_queue_task_lines(
                    &projection.visible_tasks,
                    "No executable tasks in the current planning queue.",
                    QUEUE_INSPECTION_TASK_LIMIT,
                )
            } else {
                match projection.queue_head.as_ref() {
                    Some(queue_head) => build_queue_task_lines(
                        std::slice::from_ref(queue_head),
                        "No executable tasks in the current planning queue.",
                        1,
                    ),
                    None => vec![Line::from(
                        "No executable tasks in the current planning queue.",
                    )],
                }
            };
            /*
             * Proposed tasks는 실행 가능한 active queue가 아니라 operator가 promote할 수 있는 lane이다.
             * 별도 section으로 분리해 "다음 실행"과 "승격 후보"가 같은 우선순위처럼 읽히지 않게 한다.
             */
            let proposal_lines = if projection.has_structured_queue_projection {
                build_queue_task_lines(
                    &projection.proposed_tasks,
                    "No promotable proposals are queued right now.",
                    QUEUE_INSPECTION_PROPOSAL_LIMIT,
                )
            } else if let Some(summary) = projection.proposal_summary.as_deref() {
                vec![Line::from(format!(
                    "proposals: {}",
                    compact_whitespace_detail(summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ))]
            } else {
                vec![Line::from("No promotable proposals are queued right now.")]
            };
            /*
             * Summary는 popup 첫 시선에 필요한 queue-head/queue/proposal 상태만 한 줄로 합친다. 상세 row를
             * 읽기 전에 현재 queue head, queue health, proposal lane 유무를 빠르게 확인하게 하는 headline이다.
             */
            let mut summary_segments = Vec::new();
            if let Some(queue_head) = projection.queue_head.as_ref() {
                summary_segments.push(format!(
                    "next: {}",
                    compact_whitespace_detail(
                        queue_head.task_title.trim(),
                        QUEUE_INSPECTION_TITLE_DETAIL_LIMIT
                    )
                ));
            }
            if let Some(queue_summary) = projection.queue_summary.as_deref() {
                summary_segments.push(format!(
                    "queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
                if projection.queue_head.is_none() {
                    summary_segments
                        .push(format!("policy: {}", projection.queue_idle_policy.label()));
                }
            }
            if let Some(proposal_summary) = projection.proposal_summary.as_deref() {
                summary_segments.push(format!(
                    "proposals: {}",
                    compact_whitespace_detail(proposal_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
            }
            if summary_segments.is_empty() {
                // queue/proposal 요약이 모두 없을 때는 projection의 preview status가 가장 압축된 상태 설명이다.
                summary_segments.push(format!("status: {}", projection.status_label));
            }
            let summary_lines = vec![Line::from(summary_segments.join("  |  "))];

            /*
             * Note section은 actionability 순서로 채운다. auto-follow pause와 failure reason은 queue row보다
             * 먼저 operator가 봐야 하는 blocker이고, planning notice와 planning worker host detail은 그 다음 진단이다.
             */
            let mut note_lines = Vec::new();
            if let Some(detail) = runtime_projection.auto_follow_pause_reason() {
                note_lines.push(Line::from(format!(
                    "pause: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            } else if let Some(detail) = runtime_projection.failure_reason() {
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
            if let Some(queue_summary) = app
                .planning_worker_panel_state
                .last_queue_summary
                .as_deref()
            {
                note_lines.push(Line::from(format!(
                    "planning worker queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = app.planning_worker_panel_state.last_host_detail.as_deref() {
                note_lines.push(Line::from(format!(
                    "planning worker host detail: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = build_skipped_queue_note_line(&projection.skipped_tasks) {
                note_lines.push(detail);
            }
            if note_lines.is_empty() {
                note_lines.push(Line::from(
                    "No planning worker notices or skipped queue items.",
                ));
            } else {
                // popup height를 보호하기 위해 가장 중요한 두 줄만 남긴다. 상세 진단은 shell status/notice panel에 남아 있다.
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
    tasks: &[PlanningApplicationQueueTask],
    empty_message: &str,
    max_visible_tasks: usize,
) -> Vec<Line<'static>> {
    if tasks.is_empty() {
        return vec![Line::from(empty_message.to_string())];
    }

    /*
     * Popup row는 rank, status, combined priority, title만 남긴다. dependency/blocker 설명은 application
     * projection의 rank_reasons에 있지만 popup에서는 한 줄 scan 비용이 더 중요해 상세 원인은 생략한다.
     */
    let mut lines = Vec::new();
    for task in tasks.iter().take(max_visible_tasks) {
        lines.push(Line::from(format!(
            "#{} [{} / p{}] {}",
            task.rank,
            task.status_label,
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
    skipped_tasks: &[PlanningApplicationSkippedTask],
) -> Option<Line<'static>> {
    /*
     * Skipped tasks는 active queue에서 빠졌기 때문에 전체 목록보다 "왜 줄어든 것처럼 보이는가"가 중요하다.
     * 첫 reason과 총 개수만 note로 보여 주고, 자세한 개별 skip 원인은 full planning projection에 남긴다.
     */
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
