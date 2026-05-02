// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use ratatui::text::Line;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::ConversationViewModel;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::capability_copy::thread_history_loading_status_line;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::{
    FOOTER_AUTO_FOLLOW_DETAIL_LIMIT, FOOTER_MODE_LABEL_LIMIT, FOOTER_NOTICE_DETAIL_LIMIT,
    FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT, FOOTER_STATUS_DETAIL_LIMIT, FOOTER_WARNING_DETAIL_LIMIT,
    ShellConversationState, ShellCorePresentationContext, build_working_line,
    compact_inline_detail, turn_status_label,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::plan_indicator::{PlanModeIndicatorView, plan_mode_prefixed_spans};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::tail_shared::{
    build_operator_notice_line, compact_auto_follow_status_summary, inline_thread_label,
};

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[allow(clippy::too_many_arguments)]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_shell_footer_lines_with_context(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    context: &ShellCorePresentationContext<'_>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    plan_mode_indicator: PlanModeIndicatorView,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    parallel_mode_summary_line: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    parallel_mode_alert_line: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    github_review_recent_changes_summary: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_summary_line: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_notice_line: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match context.conversation_state {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ShellConversationState::Loading => vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from("conversation state: loading thread metadata"),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(thread_history_loading_status_line()),
        ],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ShellConversationState::Failed(message) => vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from("conversation state: failed"),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(format!("status: {message}")),
        ],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ShellConversationState::Ready(conversation) => {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let warning_summary = conversation.warning_summary(FOOTER_WARNING_DETAIL_LIMIT);
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let runtime_notice_summary =
                conversation.runtime_notice_summary(FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT);
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let mut lines = vec![
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Line::from(plan_mode_prefixed_spans(
                    format!(
                        "thread: {}  |  turn: {}  |  input: {}",
                        inline_thread_label(conversation),
                        turn_status_label(conversation),
                        conversation.input_state.label(),
                    ),
                    plan_mode_indicator,
                )),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Line::from(format!(
                    "startup: {}  |  gh: {}  |  auto: {}  |  progress: {}  |  mode: {}",
                    context.shell_action_availability.status_text(),
                    context.github_review_polling_status_label.as_str(),
                    compact_auto_follow_status_summary(
                        conversation,
                        FOOTER_AUTO_FOLLOW_DETAIL_LIMIT,
                    ),
                    conversation
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .auto_follow_state
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .compact_completed_progress_label(),
                    footer_mode_label(conversation),
                )),
            ];

            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let mut status_segments = vec![format!(
                "status: {}",
                compact_inline_detail(&conversation.status_text, FOOTER_STATUS_DETAIL_LIMIT)
            )];
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if warning_summary != "clear" {
                status_segments.push(compact_inline_detail(
                    &warning_summary,
                    FOOTER_WARNING_DETAIL_LIMIT,
                ));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(runtime_notice_summary) = runtime_notice_summary.as_deref() {
                status_segments.push(compact_inline_detail(
                    runtime_notice_summary,
                    FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT,
                ));
            } else if warning_summary == "clear" {
                status_segments.push(format!(
                    "sessions: {}",
                    context.recent_session_status_label.as_str()
                ));
            }
            lines.push(Line::from(status_segments.join("  |  ")));
            lines.push(Line::from(parallel_mode_summary_line));
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(parallel_mode_alert_line) = parallel_mode_alert_line {
                lines.push(Line::from(parallel_mode_alert_line));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(working_line) = build_working_line(conversation, FOOTER_STATUS_DETAIL_LIMIT)
            {
                lines.push(working_line);
            }

            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(planning_line) = planning_summary_line {
                lines.push(Line::from(planning_line));
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(planning_notice_line) = planning_notice_line {
                lines.push(Line::from(planning_notice_line));
            }
            lines.extend(planner_panel_lines.into_iter().map(Line::from));

            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if let Some(notice_line) = build_operator_notice_line(
                github_review_recent_changes_summary.as_deref(),
                conversation,
                FOOTER_NOTICE_DETAIL_LIMIT,
            ) {
                lines.push(Line::from(format!("notice: {notice_line}")));
            }

            lines
        }
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn footer_mode_label(conversation: &ConversationViewModel) -> String {
    compact_inline_detail(
        conversation.auto_follow_state.mode_label(),
        FOOTER_MODE_LABEL_LIMIT,
    )
}
