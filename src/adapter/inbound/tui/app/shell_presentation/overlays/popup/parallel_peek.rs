use ratatui::text::Line;

use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationSnapshot,
};
use crate::domain::parallel_mode::ParallelModeAgentRosterEntry;

use super::super::super::super::parallel_peek_overlay_ui::ParallelPeekConversationPreview;
use super::super::super::super::{AkraTheme, NativeTuiApp, ParallelPeekOverlayStep};
use super::ParallelPeekOverlayView;

pub(crate) fn build_parallel_peek_overlay_view(app: &NativeTuiApp) -> ParallelPeekOverlayView {
    let active_agents = app.active_parallel_peek_entries();
    let selected_index = app.parallel_peek_overlay_ui_state.selected_agent_index();
    let step = app.parallel_peek_overlay_ui_state.step();
    let preview = app.parallel_peek_overlay_ui_state.preview();

    let header_lines = build_header_lines(step, active_agents.len(), preview);
    let agent_lines = build_agent_lines(&active_agents, selected_index);
    let conversation_lines = build_conversation_lines(step, preview);
    let status_lines = build_status_lines(step, active_agents.len(), preview);
    let key_lines = build_key_lines(step);

    ParallelPeekOverlayView {
        header_lines,
        agent_lines,
        conversation_lines,
        status_lines,
        key_lines,
    }
}

fn build_header_lines(
    step: ParallelPeekOverlayStep,
    active_agent_count: usize,
    preview: Option<&ParallelPeekConversationPreview>,
) -> Vec<Line<'static>> {
    let mut lines = vec![AkraTheme::title_line("Parallel Peek", " / read-only")];
    match step {
        ParallelPeekOverlayStep::AgentList => {
            lines.push(Line::from(format!(
                "active agents: {active_agent_count} / choose one to inspect"
            )));
        }
        ParallelPeekOverlayStep::ConversationPreview => {
            if let Some(preview) = preview {
                lines.push(Line::from(format!(
                    "{} / {} / {}",
                    preview.agent_id,
                    preview.slot_id,
                    truncate_peek_text(&preview.task_title, 72)
                )));
            } else {
                lines.push(Line::from("conversation preview unavailable"));
            }
        }
    }
    lines
}

fn build_agent_lines(
    active_agents: &[ParallelModeAgentRosterEntry],
    selected_index: usize,
) -> Vec<Line<'static>> {
    if active_agents.is_empty() {
        return vec![
            Line::from("No active parallel agents are currently available."),
            Line::from("Run `:parallel` or wait for the pool to lease a slot."),
        ];
    }

    let mut lines = Vec::new();
    for (index, entry) in active_agents.iter().enumerate() {
        let prefix = if index == selected_index { ">" } else { " " };
        let thread_label = if entry.thread_id.is_some() {
            "thread ok"
        } else {
            "thread pending"
        };
        lines.push(Line::from(format!(
            "{prefix} {}. {} / {} / {} / {} / {}",
            index + 1,
            entry.agent_id,
            entry.slot_id,
            display_peek_state_label(&entry.state_label),
            thread_label,
            truncate_peek_text(&entry.task_title, 36)
        )));
    }
    lines
}

fn build_conversation_lines(
    step: ParallelPeekOverlayStep,
    preview: Option<&ParallelPeekConversationPreview>,
) -> Vec<Line<'static>> {
    if step == ParallelPeekOverlayStep::AgentList {
        return vec![Line::from(
            "Select an active agent and press Enter to peek at its conversation.",
        )];
    }

    let Some(preview) = preview else {
        return vec![Line::from("No conversation preview is loaded.")];
    };
    let mut lines = vec![
        Line::from(format!("agent: {}", preview.agent_id)),
        Line::from(format!("slot: {}", preview.slot_id)),
        Line::from(format!(
            "thread: {}",
            preview.thread_id.as_deref().unwrap_or("not captured yet")
        )),
        Line::from(format!(
            "task: {}",
            truncate_peek_text(&preview.task_title, 96)
        )),
        Line::from(format!("status: {}", preview.status_text)),
    ];

    match preview.snapshot.as_ref() {
        Some(snapshot) => lines.extend(build_snapshot_message_lines(snapshot)),
        None => lines.push(Line::from(
            "Conversation transcript is not available for this agent yet.",
        )),
    }
    lines
}

fn build_status_lines(
    step: ParallelPeekOverlayStep,
    active_agent_count: usize,
    preview: Option<&ParallelPeekConversationPreview>,
) -> Vec<Line<'static>> {
    match step {
        ParallelPeekOverlayStep::AgentList if active_agent_count == 0 => vec![Line::from(
            "Waiting for a leased or running parallel agent before peek can open a conversation.",
        )],
        ParallelPeekOverlayStep::AgentList => vec![Line::from(format!(
            "{active_agent_count} active parallel agent(s) ready for peek"
        ))],
        ParallelPeekOverlayStep::ConversationPreview => preview
            .map(|preview| vec![Line::from(preview.status_text.clone())])
            .unwrap_or_else(|| vec![Line::from("No preview is loaded.")]),
    }
}

fn build_key_lines(step: ParallelPeekOverlayStep) -> Vec<Line<'static>> {
    match step {
        ParallelPeekOverlayStep::AgentList => vec![AkraTheme::key_line(
            "Enter: open  |  Up/Down: select  |  Esc/Ctrl+O/Ctrl+C: close",
        )],
        ParallelPeekOverlayStep::ConversationPreview => vec![AkraTheme::key_line(
            "Esc/Left: back to agents  |  Ctrl+O/Ctrl+C: close  |  read-only",
        )],
    }
}

fn build_snapshot_message_lines(snapshot: &ConversationSnapshot) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!(
            "title: {}",
            truncate_peek_text(&snapshot.title, 96)
        )),
        Line::from(format!("cwd: {}", truncate_peek_text(&snapshot.cwd, 96))),
    ];
    if snapshot.messages.is_empty() {
        lines.push(Line::from("conversation: no messages captured yet"));
    } else {
        lines.push(Line::from("conversation:"));
        let skip_count = snapshot.messages.len().saturating_sub(80);
        lines.extend(
            snapshot
                .messages
                .iter()
                .skip(skip_count)
                .map(format_message_line),
        );
    }
    if !snapshot.warnings.is_empty() {
        lines.push(Line::from(format!(
            "warnings: {}",
            snapshot
                .warnings
                .iter()
                .map(|warning| truncate_peek_text(warning, 44))
                .collect::<Vec<_>>()
                .join(" / ")
        )));
    }
    if !snapshot.runtime_notices.is_empty() {
        lines.push(Line::from(format!(
            "runtime: {}",
            snapshot
                .runtime_notices
                .iter()
                .map(|notice| truncate_peek_text(notice, 44))
                .collect::<Vec<_>>()
                .join(" / ")
        )));
    }
    lines
}

fn format_message_line(message: &ConversationMessage) -> Line<'static> {
    let label = message
        .display_label
        .as_deref()
        .unwrap_or_else(|| message_kind_label(message.kind));
    Line::from(format!(
        "{}: {}",
        label,
        truncate_peek_text(&compact_peek_text(&message.text), 116)
    ))
}

fn message_kind_label(kind: ConversationMessageKind) -> &'static str {
    match kind {
        ConversationMessageKind::User => "User",
        ConversationMessageKind::Agent => "Agent",
        ConversationMessageKind::Tool => "Tool",
        ConversationMessageKind::Status => "Status",
    }
}

fn display_peek_state_label(state_label: &str) -> String {
    match state_label {
        "reported_complete" => "reported".to_string(),
        "commit_ready" => "official".to_string(),
        other => other.replace('_', " "),
    }
}

fn compact_peek_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_peek_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let keep = max_chars.saturating_sub(3);
    let mut truncated = trimmed.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}
