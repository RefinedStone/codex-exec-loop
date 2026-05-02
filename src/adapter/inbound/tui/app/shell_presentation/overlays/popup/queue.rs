// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::super::{
    AkraTheme, ConversationState, Line, NativeTuiApp, PriorityQueueSkippedTask, PriorityQueueTask,
    QUEUE_INSPECTION_NOTE_DETAIL_LIMIT, QUEUE_INSPECTION_PROPOSAL_LIMIT,
    QUEUE_INSPECTION_TASK_LIMIT, QUEUE_INSPECTION_TITLE_DETAIL_LIMIT, compact_whitespace_detail,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::QueueOverlayView;

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(crate) fn build_queue_overlay_view(app: &NativeTuiApp) -> QueueOverlayView {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let header_lines = vec![
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        AkraTheme::title_line("Planning Queue", " / shell inspection"),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Line::from("Review the next actionable work without opening raw planning artifacts."),
    ];

    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match &app.conversation_state {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ConversationState::Loading => QueueOverlayView {
            header_lines,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            summary_lines: vec![Line::from("status: loading conversation planning state")],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            queue_lines: vec![Line::from(
                "Queue inspection becomes available after the thread loads.",
            )],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            proposal_lines: vec![Line::from("Proposal data is unavailable while loading.")],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            note_lines: vec![Line::from("No planner notes yet.")],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            key_lines: build_queue_overlay_key_lines(),
        },
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ConversationState::Failed(message) => QueueOverlayView {
            header_lines,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            summary_lines: vec![Line::from("status: conversation unavailable")],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            queue_lines: vec![Line::from(
                "Queue inspection is unavailable while the conversation failed to load.",
            )],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            proposal_lines: vec![Line::from(
                "Open a new draft or reload a session to restore planning state.",
            )],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            note_lines: vec![Line::from(format!(
                "conversation error: {}",
                compact_whitespace_detail(message, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            ))],
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            key_lines: build_queue_overlay_key_lines(),
        },
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ConversationState::Ready(conversation) => {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let snapshot = &conversation.planning_runtime_snapshot;
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let queue_projection = snapshot.queue_projection();
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let queue_lines = queue_projection
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .map(|queue_projection| {
                    build_queue_task_lines(
                        &queue_projection.active_tasks,
                        "No executable tasks in the current planning queue.",
                        QUEUE_INSPECTION_TASK_LIMIT,
                    )
                })
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .unwrap_or_else(|| match snapshot.queue_head() {
                    // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                    Some(queue_head) => build_queue_task_lines(
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        std::slice::from_ref(queue_head),
                        "No executable tasks in the current planning queue.",
                        1,
                    ),
                    // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                    None => vec![Line::from(
                        "No executable tasks in the current planning queue.",
                    )],
                });
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let proposal_lines = queue_projection
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .map(|queue_projection| {
                    build_queue_task_lines(
                        &queue_projection.proposed_tasks,
                        "No promotable proposals are queued right now.",
                        QUEUE_INSPECTION_PROPOSAL_LIMIT,
                    )
                })
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .unwrap_or_else(|| {
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if let Some(summary) = snapshot.proposal_summary() {
                        vec![Line::from(format!(
                            "proposals: {}",
                            compact_whitespace_detail(summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                        ))]
                    } else {
                        vec![Line::from("No promotable proposals are queued right now.")]
                    }
                });

            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let mut summary_segments = Vec::new();
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(queue_head) = snapshot.queue_head() {
                summary_segments.push(format!(
                    "next: {}",
                    compact_whitespace_detail(
                        queue_head.task_title.trim(),
                        QUEUE_INSPECTION_TITLE_DETAIL_LIMIT
                    )
                ));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(queue_summary) = snapshot.queue_summary() {
                summary_segments.push(format!(
                    "queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
                // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                if snapshot.queue_head().is_none() {
                    summary_segments
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .push(format!("policy: {}", snapshot.queue_idle_policy().label()));
                }
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(proposal_summary) = snapshot.proposal_summary() {
                summary_segments.push(format!(
                    "proposals: {}",
                    compact_whitespace_detail(proposal_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if summary_segments.is_empty() {
                summary_segments.push(format!("status: {}", snapshot.preview_status_label()));
            }
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let summary_lines = vec![Line::from(summary_segments.join("  |  "))];

            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let mut note_lines = Vec::new();
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
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
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(summary) =
                conversation.planning_notice_summary(QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            {
                note_lines.push(Line::from(format!("planning notice: {summary}")));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(queue_summary) =
                app.planner_worker_panel_state.last_queue_summary.as_deref()
            {
                note_lines.push(Line::from(format!(
                    "planner queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(detail) = app.planner_worker_panel_state.last_host_detail.as_deref() {
                note_lines.push(Line::from(format!(
                    "planner host detail: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(detail) = queue_projection.and_then(|queue_projection| {
                build_skipped_queue_note_line(&queue_projection.skipped_tasks)
            }) {
                note_lines.push(detail);
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
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
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                key_lines: build_queue_overlay_key_lines(),
            }
        }
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_queue_overlay_key_lines() -> Vec<Line<'static>> {
    vec![AkraTheme::key_line(
        "Esc/Ctrl+C: close  |  :planning: update files",
    )]
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_queue_task_lines(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    tasks: &[PriorityQueueTask],
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    empty_message: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    max_visible_tasks: usize,
) -> Vec<Line<'static>> {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if tasks.is_empty() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return vec![Line::from(empty_message.to_string())];
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut lines = Vec::new();
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for task in tasks.iter().take(max_visible_tasks) {
        lines.push(Line::from(format!(
            "#{} [{} / p{}] {}",
            task.rank,
            task.status.label(),
            task.combined_priority,
            compact_whitespace_detail(task.task_title.trim(), QUEUE_INSPECTION_TITLE_DETAIL_LIMIT)
        )));
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let hidden_count = tasks.len().saturating_sub(max_visible_tasks);
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if hidden_count > 0 {
        lines.push(Line::from(format!(
            "+{hidden_count} more queue item{} hidden for readability",
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if hidden_count == 1 { "" } else { "s" }
        )));
    }

    lines
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_skipped_queue_note_line(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    skipped_tasks: &[PriorityQueueSkippedTask],
) -> Option<Line<'static>> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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
