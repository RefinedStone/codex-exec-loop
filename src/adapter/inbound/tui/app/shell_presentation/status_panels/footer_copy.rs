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

/*
 * Footer copy is the last textual projection before ratatui rendering. Earlier layers keep
 * conversation, planning, GitHub polling, and parallel-mode state in separate view models; this
 * module decides which of those signals deserve one of the few persistent rows at the bottom of
 * the shell. Keeping that decision here prevents overlays and render tests from each inventing a
 * slightly different footer vocabulary.
 */
#[allow(clippy::too_many_arguments)]
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
            /*
             * While the conversation model is absent, startup capabilities are still available.
             * The first row therefore mirrors the ready footer's environment row, but replaces
             * thread/turn/input copy with loader-independent session and GitHub polling signals.
             */
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
            /*
             * The history loader owns detailed copy for why metadata is not ready yet. Reusing
             * that line keeps the footer aligned with the startup/loading UI instead of leaking
             * transport-specific state names into this presentation layer.
             */
            Line::from(thread_history_loading_status_line()),
        ],
        ShellConversationState::Failed(message) => vec![
            /*
             * Failure is terminal for the selected conversation, but shell-level capabilities can
             * still be healthy. Keeping the same first-row shape as loading lets operators compare
             * "app can act" with "conversation failed" without scanning a different layout.
             */
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
            /*
             * Warning and runtime notice summaries are already curated by ConversationViewModel:
             * newest warning first, optional runtime notice, and stable "clear" copy. Footer copy
             * only applies row-specific width budgets before deciding whether those signals
             * should displace lower-priority session catalog status.
             */
            let warning_summary = conversation.warning_summary(FOOTER_WARNING_DETAIL_LIMIT);
            let runtime_notice_summary =
                conversation.runtime_notice_summary(FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT);
            /*
             * Ready footers begin with two stable rows. Row one is the immediate interaction
             * state the user edits against; row two is the surrounding automation/capability
             * state that explains whether background systems will keep moving after the turn.
             */
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
            /*
             * The status row is deliberately a compact segment list rather than independent
             * lines. In normal operation it is "status + sessions"; under degradation it becomes
             * "status + warning/runtime notice" so the scarce footer height favors actionable
             * runtime context over passive capability inventory.
             */
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
            /*
             * Parallel mode is always shown for ready conversations because it describes work that
             * may happen outside the selected thread. Alerts get their own row to avoid hiding
             * dirty worktree, blocked queue, or cleanup-required states inside the summary string.
             */
            lines.push(Line::from(parallel_mode_summary_line));
            if let Some(parallel_mode_alert_line) = parallel_mode_alert_line {
                lines.push(Line::from(parallel_mode_alert_line));
            }
            /*
             * Working, planning, and planner-panel rows form a bottom-up activity stack:
             * live tool/agent work first, then queue/proposal context, then caller-curated detail
             * rows. The caller owns planner visibility, so this function preserves its ordering.
             */
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
            /*
             * Operator notice is the final row because tail_shared has already chosen the single
             * highest-priority human-attention item: GitHub review changes, live tool activity,
             * auto-follow detail, or approval status. Rendering it last makes it read like the
             * footer's conclusion instead of another background capability metric.
             */
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
    /*
     * The auto-follow mode label sits at the far right of the environment row. It is useful for
     * distinguishing queue/manual/internal pause modes, but it should not crowd out startup,
     * GitHub, or progress copy, so the generic inline compactor is applied at the boundary.
     */
    compact_inline_detail(
        conversation.auto_follow_state.mode_label(),
        FOOTER_MODE_LABEL_LIMIT,
    )
}
