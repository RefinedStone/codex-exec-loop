use super::super::super::terminal_text::truncate_end_to_cells;
use super::super::super::{
    AkraTheme, ConversationState, Line, NativeTuiApp, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT,
    QUEUE_INSPECTION_PROPOSAL_LIMIT, QUEUE_INSPECTION_TASK_LIMIT,
    QUEUE_INSPECTION_TITLE_DETAIL_LIMIT, TuiLanguage, compact_whitespace_detail,
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
    let header_lines = vec![AkraTheme::title_line(
        "Planning Queue",
        " / shell inspection",
    )];

    let pending_operation_id = app.pending_queue_mutation_operation_id();
    let authority_refresh_required = app.queue_mutation_requires_authority_refresh();
    let selected_task_id = app.queue_overlay_ui_state.selected_task_id();
    let latest_registration_undo_available = matches!(
        &app.conversation_state,
        ConversationState::Ready(conversation)
            if conversation
                .latest_queue_mutation_receipt
                .as_ref()
                .is_some_and(|receipt| {
                    receipt.created_batch_is_cancellable()
                        && app.queue_overlay_ui_state.authority_revision()
                            == Some(receipt.planning_revision)
                })
    );

    match &app.conversation_state {
        ConversationState::Loading => QueueOverlayView {
            header_lines,
            /*
             * Conversation이 아직 load 중이면 planning runtime projection 자체가 없다. 이 상태에서 queue/proposal
             * section을 추측하지 않고 "thread load 뒤 가능" copy로 고정해 stale planning data처럼 보이지 않게 한다.
             */
            summary_lines: build_queue_overlay_summary_lines(
                "status: loading".to_string(),
                pending_operation_id,
                authority_refresh_required,
                app.tui_language,
            ),
            queue_lines: Vec::new(),
            proposal_lines: Vec::new(),
            note_lines: Vec::new(),
            selected_content_line_index: None,
            key_lines: build_queue_overlay_key_lines(
                false,
                pending_operation_id,
                authority_refresh_required,
                app.tui_language,
            ),
        },
        ConversationState::Failed(message) => QueueOverlayView {
            header_lines,
            /*
             * Conversation load 실패는 planning queue failure와 다르다. queue projection을 만들 수 없는 상태라
             * queue 자체를 empty로 오해시키지 않고 load error와 recovery action만 보여 준다.
             */
            summary_lines: build_queue_overlay_summary_lines(
                "status: unavailable".to_string(),
                pending_operation_id,
                authority_refresh_required,
                app.tui_language,
            ),
            queue_lines: vec![Line::from("Reload the session or open a new draft.")],
            proposal_lines: Vec::new(),
            note_lines: vec![Line::from(format!(
                "conversation error: {}",
                compact_whitespace_detail(message, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            ))],
            selected_content_line_index: None,
            key_lines: build_queue_overlay_key_lines(
                false,
                pending_operation_id,
                authority_refresh_required,
                app.tui_language,
            ),
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
            let queue_section = if projection.has_structured_queue_projection {
                build_queue_task_lines(
                    &projection.visible_tasks,
                    QUEUE_INSPECTION_TASK_LIMIT,
                    selected_task_id,
                )
            } else {
                match projection.queue_head.as_ref() {
                    Some(queue_head) => build_queue_task_lines(
                        std::slice::from_ref(queue_head),
                        1,
                        selected_task_id,
                    ),
                    None => QueueTaskLines::unselected(Vec::new()),
                }
            };
            /*
             * Proposed tasks는 실행 가능한 active queue가 아니라 operator가 promote할 수 있는 lane이다.
             * 별도 section으로 분리해 "다음 실행"과 "승격 후보"가 같은 우선순위처럼 읽히지 않게 한다.
             */
            let proposal_section = if projection.has_structured_queue_projection {
                build_queue_task_lines(
                    &projection.proposed_tasks,
                    QUEUE_INSPECTION_PROPOSAL_LIMIT,
                    selected_task_id,
                )
            } else if let Some(summary) = projection.proposal_summary.as_deref() {
                QueueTaskLines::unselected(vec![Line::from(format!(
                    "proposals: {}",
                    compact_whitespace_detail(summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ))])
            } else {
                QueueTaskLines::unselected(Vec::new())
            };
            let queue_lines = queue_section.lines;
            let queue_selected_line_index = queue_section.selected_line_index;
            let proposal_lines = proposal_section.lines;
            let proposal_selected_line_index = proposal_section.selected_line_index;
            // Rows own task titles. The summary keeps only global state and lane counts.
            let queued_count = if projection.has_structured_queue_projection {
                projection.visible_tasks.len()
            } else {
                usize::from(projection.queue_head.is_some())
            };
            let mut summary_segments = vec![
                format!("status: {}", projection.status_label),
                format!("queued: {queued_count}"),
            ];
            if projection.has_structured_queue_projection {
                summary_segments.push(format!("proposed: {}", projection.proposed_tasks.len()));
            }
            if queued_count == 0 {
                summary_segments.push(format!("idle: {}", projection.queue_idle_policy.label()));
            }
            let summary_lines = build_queue_overlay_summary_lines(
                summary_segments.join("  |  "),
                pending_operation_id,
                authority_refresh_required,
                app.tui_language,
            );

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
            let selected_skipped_line =
                build_selected_skipped_queue_line(&projection.skipped_tasks, selected_task_id);
            let selected_skipped_visible = selected_skipped_line.is_some();
            let selected_skipped_note_index = selected_skipped_line.map(|line| {
                let index = note_lines.len();
                note_lines.push(line);
                index
            });
            if let Some(feedback) = app.queue_overlay_ui_state.feedback() {
                note_lines.push(Line::from(feedback.to_string()));
            }
            if let Some(summary) =
                conversation.planning_notice_summary(QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            {
                note_lines.push(Line::from(format!("planning notice: {summary}")));
            }
            if let Some(detail) = app.planning_worker_panel_state.last_host_detail.as_deref() {
                note_lines.push(Line::from(format!(
                    "worker: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if !selected_skipped_visible
                && let Some(detail) = build_skipped_queue_note_line(&projection.skipped_tasks)
            {
                note_lines.push(detail);
            }
            if !note_lines.is_empty() {
                // popup height를 보호하기 위해 가장 중요한 두 줄만 남긴다. 상세 진단은 shell status/notice panel에 남아 있다.
                note_lines.truncate(2);
            }

            let proposal_heading_index = queue_lines.len();
            let notes_heading_index = proposal_heading_index
                + usize::from(!proposal_lines.is_empty())
                + proposal_lines.len();
            let selected_content_line_index = queue_selected_line_index
                .or_else(|| {
                    proposal_selected_line_index.map(|index| proposal_heading_index + 1 + index)
                })
                .or_else(|| {
                    selected_skipped_note_index
                        .map(|index| notes_heading_index.saturating_add(1).saturating_add(index))
                });

            QueueOverlayView {
                header_lines,
                summary_lines,
                queue_lines,
                proposal_lines,
                note_lines,
                selected_content_line_index,
                key_lines: build_queue_overlay_key_lines(
                    latest_registration_undo_available,
                    pending_operation_id,
                    authority_refresh_required,
                    app.tui_language,
                ),
            }
        }
    }
}

fn build_queue_overlay_summary_lines(
    summary: String,
    pending_operation_id: Option<u64>,
    authority_refresh_required: bool,
    tui_language: TuiLanguage,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(summary)];
    if let Some(operation_id) = pending_operation_id {
        lines.push(
            Line::from(tui_language.queue_mutation_pending_summary(operation_id))
                .style(AkraTheme::warning()),
        );
    } else if authority_refresh_required {
        lines.push(
            Line::from(tui_language.queue_mutation_refresh_required_summary())
                .style(AkraTheme::warning()),
        );
    }
    lines
}

fn build_queue_overlay_key_lines(
    latest_registration_undo_available: bool,
    pending_operation_id: Option<u64>,
    authority_refresh_required: bool,
    tui_language: TuiLanguage,
) -> Vec<Line<'static>> {
    if let Some(operation_id) = pending_operation_id {
        // Navigation and dismissal stay live while the authority gate owns every destructive action.
        return vec![
            AkraTheme::key_line(tui_language.queue_overlay_select_key_line()),
            Line::from(tui_language.queue_mutation_pending_disabled_key_line(operation_id))
                .style(AkraTheme::warning()),
            AkraTheme::key_line(tui_language.queue_overlay_close_key_line()),
        ];
    }
    if authority_refresh_required {
        return vec![
            AkraTheme::key_line(tui_language.queue_overlay_select_key_line()),
            Line::from(tui_language.queue_mutation_refresh_disabled_key_line())
                .style(AkraTheme::warning()),
            AkraTheme::key_line(tui_language.queue_overlay_close_key_line()),
        ];
    }

    let mut lines = vec![
        AkraTheme::key_line(tui_language.queue_overlay_select_key_line()),
        AkraTheme::key_line(
            tui_language.queue_overlay_remove_key_line(latest_registration_undo_available),
        ),
    ];
    lines.push(AkraTheme::key_line(
        tui_language.queue_overlay_close_key_line(),
    ));
    lines
}

struct QueueTaskLines {
    lines: Vec<Line<'static>>,
    selected_line_index: Option<usize>,
}

impl QueueTaskLines {
    fn unselected(lines: Vec<Line<'static>>) -> Self {
        Self {
            lines,
            selected_line_index: None,
        }
    }
}

fn build_queue_task_lines(
    tasks: &[PlanningApplicationQueueTask],
    max_visible_tasks: usize,
    selected_task_id: Option<&str>,
) -> QueueTaskLines {
    if tasks.is_empty() {
        return QueueTaskLines::unselected(Vec::new());
    }

    // Rank is the stable visible discriminator when compacted titles collide.
    let max_visible_tasks = max_visible_tasks.max(1);
    let selected_index = selected_task_id
        .and_then(|selected_task_id| {
            tasks
                .iter()
                .position(|task| task.task_id == selected_task_id)
        })
        .unwrap_or(0);
    let window_start = selected_index
        .saturating_add(1)
        .saturating_sub(max_visible_tasks);
    let mut lines = Vec::new();
    let mut selected_line_index = None;
    for task in tasks.iter().skip(window_start).take(max_visible_tasks) {
        let selected = selected_task_id == Some(task.task_id.as_str());
        let marker = if selected { ">" } else { " " };
        let line = Line::from(format!(
            "{marker} #{} [{}] {}",
            task.rank,
            task.status_label,
            compact_queue_title(task.task_title.as_str())
        ));
        if selected {
            selected_line_index = Some(lines.len());
        }
        lines.push(if selected {
            line.style(AkraTheme::selected())
        } else {
            line
        });
    }

    let hidden_before = window_start;
    let hidden_after = tasks
        .len()
        .saturating_sub(window_start.saturating_add(max_visible_tasks));
    match (hidden_before, hidden_after) {
        (0, 0) => {}
        (0, hidden_after) => lines.push(Line::from(format!("+{hidden_after} more"))),
        (hidden_before, 0) => lines.push(Line::from(format!("+{hidden_before} earlier"))),
        (hidden_before, hidden_after) => lines.push(Line::from(format!(
            "+{hidden_before} earlier / +{hidden_after} later"
        ))),
    }

    QueueTaskLines {
        lines,
        selected_line_index,
    }
}

fn build_selected_skipped_queue_line(
    skipped_tasks: &[PlanningApplicationSkippedTask],
    selected_task_id: Option<&str>,
) -> Option<Line<'static>> {
    let selected = skipped_tasks.iter().find(|task| {
        selected_task_id == Some(task.task_id.as_str())
            && matches!(
                task.status,
                crate::domain::planning::TaskStatus::Ready
                    | crate::domain::planning::TaskStatus::Proposed
            )
    })?;
    Some(
        Line::from(format!(
            "> [{} / skipped] {} - {}",
            selected.status_label,
            compact_queue_title(selected.task_title.as_str()),
            compact_whitespace_detail(selected.reason.as_str(), QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
        ))
        .style(AkraTheme::selected()),
    )
}

fn compact_queue_title(title: &str) -> String {
    let compact = compact_whitespace_detail(title, usize::MAX);
    truncate_end_to_cells(&compact, QUEUE_INSPECTION_TITLE_DETAIL_LIMIT)
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

#[cfg(test)]
mod tests {
    use super::super::super::super::terminal_text::display_width;
    use super::{
        TuiLanguage, build_queue_overlay_key_lines, build_queue_overlay_summary_lines,
        compact_queue_title,
    };

    #[test]
    fn queue_titles_compact_whitespace_and_respect_terminal_cell_budget() {
        let compact = compact_queue_title(&format!("  {}\n  tail  ", "긴 한국어 작업 ".repeat(12)));

        assert!(!compact.contains('\n'));
        assert!(!compact.contains("  "));
        assert!(compact.ends_with('…'));
        assert!(display_width(&compact) <= 56);
    }

    #[test]
    fn queue_keys_only_show_undo_when_the_latest_batch_is_cancellable() {
        let normal = build_queue_overlay_key_lines(false, None, false, TuiLanguage::English)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let cancellable = build_queue_overlay_key_lines(true, None, false, TuiLanguage::English)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(normal.contains("Up/Down, j/k: select"));
        assert!(normal.contains("x/Delete: remove"));
        assert!(normal.contains("Esc/Ctrl+C: close"));
        assert!(!normal.contains("u: undo"));
        assert!(cancellable.contains("u: undo added"));
    }

    #[test]
    fn pending_queue_mutation_keeps_navigation_and_hides_destructive_shortcuts() {
        let summary = build_queue_overlay_summary_lines(
            "status: ready".to_string(),
            Some(17),
            false,
            TuiLanguage::English,
        )
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let keys = build_queue_overlay_key_lines(true, Some(17), false, TuiLanguage::English)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(summary.contains("op-17 | authority acknowledgement pending"));
        assert!(keys.contains("Up/Down, j/k: select"));
        assert!(keys.contains("op-17 pending: remove/undo disabled"));
        assert!(keys.contains("Esc/Ctrl+C: close"));
        assert!(!keys.contains("x/Delete"));
        assert!(!keys.contains("u: undo"));

        let korean = build_queue_overlay_summary_lines(
            "status: ready".to_string(),
            Some(17),
            false,
            TuiLanguage::Korean,
        )
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        assert!(korean.contains("op-17 | 권한 확인 대기 중"));
        let korean_keys = build_queue_overlay_key_lines(true, Some(17), false, TuiLanguage::Korean)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(korean_keys.contains("Up/Down, j/k: 선택"));
        assert!(korean_keys.contains("제거/되돌리기 비활성화"));
        assert!(korean_keys.contains("Esc/Ctrl+C: 닫기"));
    }

    #[test]
    fn required_authority_refresh_disables_mutation_until_reopen() {
        let summary = build_queue_overlay_summary_lines(
            "status: ready".to_string(),
            None,
            true,
            TuiLanguage::English,
        )
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let keys = build_queue_overlay_key_lines(true, None, true, TuiLanguage::English)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(summary.contains("queue authority refresh required"));
        assert!(keys.contains("close and reopen to refresh"));
        assert!(!keys.contains("x/Delete"));
        assert!(!keys.contains("u: undo"));

        let korean = build_queue_overlay_key_lines(true, None, true, TuiLanguage::Korean)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(korean.contains("닫았다가 다시 열어 새로고침"));
    }
}
