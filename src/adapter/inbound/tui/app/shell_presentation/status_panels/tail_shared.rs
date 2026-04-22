use ratatui::text::Line;

use super::super::ConversationViewModel;
use super::super::{
    INLINE_LIVE_AGENT_DETAIL_LIMIT, INLINE_LIVE_AGENT_MAX_CONTENT_LINES,
    INLINE_TAIL_THREAD_LABEL_LIMIT, INLINE_TAIL_WARNING_DETAIL_LIMIT, NativeTuiApp,
    compact_inline_detail,
};
use crate::adapter::inbound::tui::conversation_text::conversation_message_kind_label;

pub(super) fn current_live_agent_lines(
    conversation: &ConversationViewModel,
) -> Option<Vec<Line<'static>>> {
    let message = conversation.live_agent_message.as_ref()?;
    let label = conversation_message_kind_label(message.kind, message.phase.as_deref());
    let content_lines = message.text.lines().collect::<Vec<_>>();
    let start_index = content_lines
        .len()
        .saturating_sub(INLINE_LIVE_AGENT_MAX_CONTENT_LINES);
    let mut lines = vec![Line::from(format!("live: {label}"))];

    for line in content_lines.into_iter().skip(start_index) {
        lines.push(Line::from(format!(
            "  {}",
            compact_live_agent_line(line, INLINE_LIVE_AGENT_DETAIL_LIMIT)
        )));
    }

    Some(lines)
}

pub(super) fn parallel_mode_summary_line(app: &NativeTuiApp) -> String {
    match app.parallel_mode_readiness_snapshot() {
        Some(snapshot) => {
            let supervisor_snapshot = app.parallel_mode_supervisor_snapshot();
            format!(
                "parallel: {}  |  mode: {}  |  pool: {}  |  agents: {}  |  queue: {}",
                snapshot.readiness_label(),
                if app.parallel_mode_enabled() {
                    "parallel"
                } else {
                    "normal"
                },
                supervisor_snapshot.pool.compact_summary(),
                supervisor_snapshot.roster.compact_summary(),
                supervisor_snapshot.distributor.compact_summary(),
            )
        }
        None if app.parallel_mode_enabled() => {
            "parallel: preparing  |  mode: parallel  |  pool: pending reconcile  |  agents: 0 active  |  queue: pending".to_string()
        }
        None => {
            "parallel: off  |  mode: normal  |  pool: inactive  |  agents: inactive  |  queue: inactive".to_string()
        }
    }
}

pub(super) fn parallel_mode_alert_line(app: &NativeTuiApp) -> Option<String> {
    app.parallel_mode_readiness_snapshot()
        .and_then(|snapshot| snapshot.top_alert.as_deref())
        .map(|alert| format!("parallel alert: {alert}"))
}

pub(super) fn build_operator_notice_line(
    github_review_recent_changes_summary: Option<&str>,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<String> {
    if let Some(github_review_summary) = github_review_recent_changes_summary {
        return Some(format!(
            "gh update: {}",
            compact_inline_detail(github_review_summary, max_detail_len)
        ));
    }

    let turn_running = conversation.has_running_turn();
    let activity_scope = conversation
        .turn_activity
        .activity_scope_label(turn_running);
    let activity_summary = conversation.turn_activity.activity_summary(turn_running);
    let activity_command_count = conversation
        .turn_activity
        .activity_command_count(turn_running);
    let activity_file_change_count = conversation
        .turn_activity
        .activity_file_change_count(turn_running);
    let has_tool_activity = (activity_summary != "idle" && activity_summary != "none")
        || activity_command_count > 0
        || activity_file_change_count > 0;
    if turn_running && has_tool_activity {
        let mut notice_line = format!(
            "tool activity: {}  |  {activity_scope} commands: {}  |  {activity_scope} file changes: {}",
            compact_inline_detail(activity_summary, max_detail_len),
            activity_command_count,
            activity_file_change_count,
        );
        if let Some(approval_summary) = conversation.approval_summary().as_deref() {
            notice_line.push_str(&format!(
                "  |  approval: {}",
                compact_inline_detail(approval_summary, max_detail_len)
            ));
        }
        return Some(notice_line);
    }

    if let Some(activity) = conversation.last_auto_followup_activity.as_ref() {
        return Some(format!(
            "auto: {}  |  detail: {}",
            activity.summary,
            compact_inline_detail(&activity.detail, max_detail_len)
        ));
    }

    if has_tool_activity {
        let mut notice_line = format!(
            "tool activity: {}  |  {activity_scope} commands: {}  |  {activity_scope} file changes: {}",
            compact_inline_detail(activity_summary, max_detail_len),
            activity_command_count,
            activity_file_change_count,
        );
        if let Some(approval_summary) = conversation.approval_summary().as_deref() {
            notice_line.push_str(&format!(
                "  |  approval: {}",
                compact_inline_detail(approval_summary, max_detail_len)
            ));
        }
        return Some(notice_line);
    }

    conversation.approval_summary().map(|approval_summary| {
        format!(
            "approval: {}",
            compact_inline_detail(&approval_summary, max_detail_len)
        )
    })
}

pub(super) fn compact_inline_summary_label(summary: &str) -> String {
    compact_inline_detail(
        &summary
            .replace("runtime warning:", "rt warn:")
            .replace("runtime warnings", "rt warns")
            .replace("warning:", "warn:")
            .replace("warnings:", "warn:")
            .replace("runtime notices", "notices")
            .replace("runtime:", "notice:"),
        INLINE_TAIL_WARNING_DETAIL_LIMIT,
    )
}

pub(super) fn compact_auto_follow_status_summary(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> String {
    let summary = if conversation.auto_follow_state.enabled {
        format!(
            "{}/{}",
            conversation.auto_follow_state.status_label(),
            conversation.auto_follow_state.activity_label()
        )
    } else {
        conversation.auto_follow_state.status_label().to_string()
    };
    compact_inline_detail(&summary, max_detail_len)
}

pub(super) fn inline_thread_label(conversation: &ConversationViewModel) -> String {
    if !conversation.has_active_thread() {
        return "new draft".to_string();
    }

    compact_inline_detail(&conversation.title, INLINE_TAIL_THREAD_LABEL_LIMIT)
}

fn compact_live_agent_line(text: &str, max_len: usize) -> String {
    let rendered = text.replace('\t', "    ");
    if rendered.chars().count() <= max_len {
        return rendered;
    }

    let keep = max_len.saturating_sub(3);
    let truncated = rendered.chars().take(keep).collect::<String>();
    format!("{truncated}...")
}
