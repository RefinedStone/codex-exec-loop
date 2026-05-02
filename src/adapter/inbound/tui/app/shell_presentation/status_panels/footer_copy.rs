// 학습 주석: footer builder의 최종 출력은 ratatui `Line` 목록입니다. 문자열 조립은 여기서 끝나고,
// renderer는 Line을 그대로 status panel에 배치합니다.
use ratatui::text::Line;

// 학습 주석: ready conversation footer는 ConversationViewModel의 warning/runtime/auto-follow projection을 읽습니다.
use super::super::ConversationViewModel;
// 학습 주석: loading 상태에서는 아직 conversation detail이 없으므로 thread history loader copy를 별도 helper에서 가져옵니다.
use super::super::capability_copy::thread_history_loading_status_line;
// 학습 주석: footer는 shell context와 여러 copy helper를 모아 status/detail budget 안에서 한 줄 요약들을 만듭니다.
use super::super::{
    FOOTER_AUTO_FOLLOW_DETAIL_LIMIT, FOOTER_MODE_LABEL_LIMIT, FOOTER_NOTICE_DETAIL_LIMIT,
    FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT, FOOTER_STATUS_DETAIL_LIMIT, FOOTER_WARNING_DETAIL_LIMIT,
    ShellConversationState, ShellCorePresentationContext, build_working_line,
    compact_inline_detail, turn_status_label,
};
// 학습 주석: plan mode indicator는 footer 첫 줄 앞에 붙는 모드 신호입니다. footer copy 자체와 분리해
// plan indicator 스타일링을 재사용합니다.
use super::plan_indicator::{PlanModeIndicatorView, plan_mode_prefixed_spans};
// 학습 주석: tail_shared helper들은 inline tail과 footer가 같은 thread/auto-follow/operator notice 용어를 쓰게 합니다.
use super::tail_shared::{
    build_operator_notice_line, compact_auto_follow_status_summary, inline_thread_label,
};

// 학습 주석: footer는 shell context 외에도 planning, parallel, GitHub review 요약을 받아 한 panel에
// 합치는 통합 지점이라 인자가 많습니다. 구조체로 감싸기보다 call-site의 기존 projection들을 그대로 받습니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_shell_footer_lines_with_context(
    // 학습 주석: shell core가 app 상태에서 잘라낸 startup/action/conversation projection입니다.
    context: &ShellCorePresentationContext<'_>,
    // 학습 주석: footer 첫 줄에 plan mode prefix를 붙이기 위한 현재 plan mode 표시 모델입니다.
    plan_mode_indicator: PlanModeIndicatorView,
    // 학습 주석: parallel mode pool/dispatcher의 기본 summary line은 ready footer에 항상 표시됩니다.
    parallel_mode_summary_line: String,
    // 학습 주석: parallel mode가 operator attention이 필요한 상태면 summary 다음 줄에 alert를 추가합니다.
    parallel_mode_alert_line: Option<String>,
    // 학습 주석: GitHub review polling이 최근 변경을 감지했을 때 operator notice 후보로 전달됩니다.
    github_review_recent_changes_summary: Option<String>,
    // 학습 주석: planning runtime의 queue/proposal summary입니다. ready footer 하단에 추가됩니다.
    planning_summary_line: Option<String>,
    // 학습 주석: planning runtime에서 별도 attention notice가 있으면 summary와 분리해 표시합니다.
    planning_notice_line: Option<String>,
    // 학습 주석: planner panel의 추가 detail lines입니다. debug/normal visibility는 caller 쪽 projection에서 결정합니다.
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    // 학습 주석: footer copy는 conversation lifecycle에 따라 완전히 다른 정보 밀도를 가집니다.
    // loading/failed는 startup과 history 상태만, ready는 runtime details를 여러 줄로 압축합니다.
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            // 학습 주석: loading 첫 줄은 startup action, session catalog, GitHub polling 상태를 한 줄에 모읍니다.
            // conversation 자체는 아직 없으므로 thread/turn/input 대신 startup capability를 보여 줍니다.
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            // 학습 주석: 두 번째 줄은 conversation state machine이 아직 metadata load 중임을 명시합니다.
            Line::from("conversation state: loading thread metadata"),
            // 학습 주석: thread history loader copy는 capability layer와 공유해 startup footer와 loader UI가 같은 말을 합니다.
            Line::from(thread_history_loading_status_line()),
        ],
        ShellConversationState::Failed(message) => vec![
            // 학습 주석: failed 상태도 startup/session/github capability는 계속 유용하므로 loading과 같은 첫 줄을 유지합니다.
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            // 학습 주석: failure는 loader 진행 상태와 달리 terminal state라 별도 state line을 둡니다.
            Line::from("conversation state: failed"),
            // 학습 주석: 실패 message는 status 줄로 내려 operator가 원인을 바로 확인하게 합니다.
            Line::from(format!("status: {message}")),
        ],
        ShellConversationState::Ready(conversation) => {
            // 학습 주석: warning summary는 view model이 최신 warning과 count를 이미 고른 결과입니다.
            // footer는 detail budget 안으로 다시 compact해 status line에 끼워 넣습니다.
            let warning_summary = conversation.warning_summary(FOOTER_WARNING_DETAIL_LIMIT);
            // 학습 주석: runtime notice는 없을 수 있어 Option으로 유지합니다. 있으면 warning보다 뒤에 이어 붙입니다.
            let runtime_notice_summary =
                conversation.runtime_notice_summary(FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT);
            // 학습 주석: ready footer의 앞 두 줄은 항상 고정 위치를 갖습니다. 첫 줄은 thread/turn/input,
            // 둘째 줄은 startup/GitHub/auto-follow/progress/mode를 담아 반복 사용자가 빠르게 스캔하게 합니다.
            let mut lines = vec![
                // 학습 주석: thread label, turn status, input state는 conversation의 즉시 상호작용 상태입니다.
                Line::from(plan_mode_prefixed_spans(
                    format!(
                        "thread: {}  |  turn: {}  |  input: {}",
                        inline_thread_label(conversation),
                        turn_status_label(conversation),
                        conversation.input_state.label(),
                    ),
                    plan_mode_indicator,
                )),
                // 학습 주석: auto-follow는 상태 문자열이 길 수 있으므로 별도 detail limit으로 줄이고,
                // progress와 mode는 같은 자동화 흐름의 현재 위치를 보완합니다.
                Line::from(format!(
                    "startup: {}  |  gh: {}  |  auto: {}  |  progress: {}  |  mode: {}",
                    context.shell_action_availability.status_text(),
                    context.github_review_polling_status_label.as_str(),
                    compact_auto_follow_status_summary(
                        conversation,
                        FOOTER_AUTO_FOLLOW_DETAIL_LIMIT,
                    ),
                    conversation
                        .auto_follow_state
                        .compact_completed_progress_label(),
                    footer_mode_label(conversation),
                )),
            ];

            // 학습 주석: 세 번째 줄은 base status를 시작점으로 warning/runtime/session fallback을 붙이는
            // 압축 status row입니다. 너무 많은 panel line을 만들지 않기 위한 핵심 합성 지점입니다.
            let mut status_segments = vec![format!(
                "status: {}",
                compact_inline_detail(&conversation.status_text, FOOTER_STATUS_DETAIL_LIMIT)
            )];
            // 학습 주석: warning이 clear가 아니면 status row에 붙여 사용자가 하단을 한 번만 훑어도
            // 위험 신호를 볼 수 있게 합니다.
            if warning_summary != "clear" {
                status_segments.push(compact_inline_detail(
                    &warning_summary,
                    FOOTER_WARNING_DETAIL_LIMIT,
                ));
            }
            // 학습 주석: runtime notice가 있으면 session catalog 상태보다 우선합니다. 실행 중 문제나
            // 후속 안내가 단순 capability보다 더 즉시성이 높기 때문입니다.
            if let Some(runtime_notice_summary) = runtime_notice_summary.as_deref() {
                status_segments.push(compact_inline_detail(
                    runtime_notice_summary,
                    FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT,
                ));
            } else if warning_summary == "clear" {
                // 학습 주석: warning과 runtime notice가 모두 없을 때만 sessions 상태를 채워 status row의
                // 남는 공간을 유용한 capability signal로 사용합니다.
                status_segments.push(format!(
                    "sessions: {}",
                    context.recent_session_status_label.as_str()
                ));
            }
            lines.push(Line::from(status_segments.join("  |  ")));
            // 학습 주석: parallel mode summary는 ready footer의 기본 운영 상태라 항상 status row 다음에 둡니다.
            lines.push(Line::from(parallel_mode_summary_line));
            // 학습 주석: alert는 summary와 분리해 한 줄을 더 써서 blocked/cleanup 같은 operator action을 묻히지 않게 합니다.
            if let Some(parallel_mode_alert_line) = parallel_mode_alert_line {
                lines.push(Line::from(parallel_mode_alert_line));
            }
            // 학습 주석: working line은 active tool/agent activity가 있을 때만 추가되는 transient detail입니다.
            if let Some(working_line) = build_working_line(conversation, FOOTER_STATUS_DETAIL_LIMIT)
            {
                lines.push(working_line);
            }

            // 학습 주석: planning summary는 parallel/runtime 상태 뒤에 배치해 queue context를 별도 줄로 보존합니다.
            if let Some(planning_line) = planning_summary_line {
                lines.push(Line::from(planning_line));
            }
            // 학습 주석: planning notice는 summary보다 action-oriented detail이므로 바로 다음 줄에 둡니다.
            if let Some(planning_notice_line) = planning_notice_line {
                lines.push(Line::from(planning_notice_line));
            }
            // 학습 주석: planner panel lines는 caller가 이미 visibility와 ordering을 결정한 상세 row입니다.
            lines.extend(planner_panel_lines.into_iter().map(Line::from));

            // 학습 주석: operator notice는 마지막에 붙여 최신 GitHub review 변화나 conversation-specific
            // 안내를 footer의 결론처럼 보이게 합니다.
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

fn footer_mode_label(conversation: &ConversationViewModel) -> String {
    // 학습 주석: auto-follow mode label은 footer 두 번째 줄의 마지막 segment라 짧아야 합니다.
    // mode copy 자체는 auto_follow_state가 만들고, footer는 표시 폭에 맞게 compact만 수행합니다.
    compact_inline_detail(
        conversation.auto_follow_state.mode_label(),
        FOOTER_MODE_LABEL_LIMIT,
    )
}
