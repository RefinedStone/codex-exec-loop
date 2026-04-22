use ratatui::text::Line;

use super::super::ConversationViewModel;
use super::super::capability_copy::thread_history_loading_status_line;
use super::super::{
    FOOTER_AUTO_FOLLOW_DETAIL_LIMIT, FOOTER_MODE_LABEL_LIMIT, FOOTER_NOTICE_DETAIL_LIMIT,
    FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT, FOOTER_STATUS_DETAIL_LIMIT, FOOTER_WARNING_DETAIL_LIMIT,
    ShellConversationState, ShellCorePresentationContext, build_working_line,
    compact_inline_detail, turn_status_label,
};
use super::plan_indicator::{PlanModeIndicatorView, plan_mode_prefixed_spans};
use super::tail_shared::{
    build_operator_notice_line, compact_auto_follow_status_summary, inline_thread_label,
};

pub(super) fn build_shell_footer_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
    plan_mode_indicator: PlanModeIndicatorView,
    parallel_mode_summary_line: String,
    parallel_mode_alert_line: Option<String>,
    github_review_recent_changes_summary: Option<String>,
    planning_summary_line: Option<String>,
    planning_notice_line: Option<String>,
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            Line::from("conversation state: loading thread metadata"),
            Line::from(thread_history_loading_status_line()),
        ],
        ShellConversationState::Failed(message) => vec![
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            Line::from("conversation state: failed"),
            Line::from(format!("status: {message}")),
        ],
        ShellConversationState::Ready(conversation) => {
            let warning_summary = conversation.warning_summary(FOOTER_WARNING_DETAIL_LIMIT);
            let runtime_notice_summary =
                conversation.runtime_notice_summary(FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT);
            let mut lines = vec![
                Line::from(plan_mode_prefixed_spans(
                    format!(
                        "thread: {}  |  turn: {}  |  input: {}",
                        inline_thread_label(conversation),
                        turn_status_label(conversation),
                        conversation.input_state.label(),
                    ),
                    plan_mode_indicator,
                )),
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

            let mut status_segments = vec![format!(
                "status: {}",
                compact_inline_detail(&conversation.status_text, FOOTER_STATUS_DETAIL_LIMIT)
            )];
            if warning_summary != "clear" {
                status_segments.push(compact_inline_detail(
                    &warning_summary,
                    FOOTER_WARNING_DETAIL_LIMIT,
                ));
            }
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
            if let Some(parallel_mode_alert_line) = parallel_mode_alert_line {
                lines.push(Line::from(parallel_mode_alert_line));
            }
            if let Some(working_line) = build_working_line(conversation, FOOTER_STATUS_DETAIL_LIMIT)
            {
                lines.push(working_line);
            }

            if let Some(planning_line) = planning_summary_line {
                lines.push(Line::from(planning_line));
            }
            if let Some(planning_notice_line) = planning_notice_line {
                lines.push(Line::from(planning_notice_line));
            }
            lines.extend(planner_panel_lines.into_iter().map(Line::from));

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
    compact_inline_detail(
        conversation.auto_follow_state.mode_label(),
        FOOTER_MODE_LABEL_LIMIT,
    )
}
