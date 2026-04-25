use std::time::{Duration, Instant};

use crate::domain::text::compact_whitespace_detail;

#[cfg(test)]
use super::Style;
use super::{
    AkraTheme, AutoFollowRuntimePhase, ConversationInputState, ConversationViewModel, Line,
    Modifier, Span,
};

pub(super) fn compact_inline_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
}

pub(super) fn turn_status_label(conversation: &ConversationViewModel) -> &'static str {
    if conversation.has_running_turn() || conversation.auto_follow_state.has_live_activity() {
        "working"
    } else {
        "idle"
    }
}

pub(super) fn build_working_line(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<Line<'static>> {
    let (started_at, detail) = if conversation.auto_follow_state.has_live_activity() {
        (
            conversation.auto_follow_state.active_started_at()?,
            auto_follow_working_detail(conversation),
        )
    } else {
        (
            conversation.active_turn_started_at?,
            manual_turn_working_detail(conversation)?,
        )
    };
    let detail = compact_inline_detail(&detail, max_detail_len);
    let elapsed = format_elapsed(Instant::now().saturating_duration_since(started_at));

    Some(Line::from(vec![
        Span::styled(
            "◦ Working".to_string(),
            AkraTheme::muted().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" ({elapsed} • {detail})"), AkraTheme::subtle()),
    ]))
}

fn manual_turn_working_detail(conversation: &ConversationViewModel) -> Option<String> {
    if !conversation.has_running_turn() {
        return None;
    }

    let interrupt_label = conversation.interrupt_support_label();
    match conversation.input_state {
        ConversationInputState::SubmittingTurn => {
            Some(format!("starting turn / interrupt {interrupt_label}"))
        }
        ConversationInputState::StreamingTurn => {
            if conversation.live_agent_message.is_some() {
                Some(format!("turn running / interrupt {interrupt_label}"))
            } else {
                Some(format!(
                    "waiting for response / interrupt {interrupt_label}"
                ))
            }
        }
        ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue => None,
    }
}

fn auto_follow_working_detail(conversation: &ConversationViewModel) -> String {
    let max_auto_turns = conversation.auto_follow_state.max_auto_turns_label();
    let interrupt_label = conversation.interrupt_support_label();
    match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => "idle".to_string(),
        AutoFollowRuntimePhase::Evaluating { .. } => "evaluating next auto follow-up".to_string(),
        AutoFollowRuntimePhase::Queued { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} queued for submission")
        }
        AutoFollowRuntimePhase::Submitting { turn_index, .. } => {
            format!(
                "auto turn {turn_index}/{max_auto_turns} starting / interrupt {interrupt_label}"
            )
        }
        AutoFollowRuntimePhase::Running { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} running / interrupt {interrupt_label}")
        }
    }
}

pub(super) fn auto_follow_prompt_status_line(
    conversation: &ConversationViewModel,
    inline: bool,
) -> Option<String> {
    let max_auto_turns = conversation.auto_follow_state.max_auto_turns_label();
    let detail = match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => return None,
        AutoFollowRuntimePhase::Evaluating { .. } => "auto follow-up evaluating".to_string(),
        AutoFollowRuntimePhase::Queued { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} queued")
        }
        AutoFollowRuntimePhase::Submitting { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} starting")
        }
        AutoFollowRuntimePhase::Running { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} running")
        }
    };

    Some(if inline {
        format!("prompt: {detail}  |  type now, Enter when idle")
    } else {
        detail
    })
}

#[cfg(test)]
pub(super) fn auto_follow_prompt_lines(
    conversation: &ConversationViewModel,
) -> Option<Vec<Line<'static>>> {
    let detail = auto_follow_prompt_status_line(conversation, false)?;
    Some(vec![
        Line::from(format!("Auto follow-up is {detail}.")),
        Line::from("Type now; press Enter after the shell returns idle."),
    ])
}

pub(in super::super) fn format_elapsed(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn inline_input_state_label(input_state: ConversationInputState) -> &'static str {
    match input_state {
        ConversationInputState::DraftReady => "draft",
        ConversationInputState::ReadyToContinue => "ready",
        ConversationInputState::SubmittingTurn => "sending",
        ConversationInputState::StreamingTurn => "streaming",
    }
}

#[cfg(test)]
pub(super) fn input_state_style(input_state: ConversationInputState) -> Style {
    match input_state {
        ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue => {
            AkraTheme::success()
        }
        ConversationInputState::SubmittingTurn => AkraTheme::warning(),
        ConversationInputState::StreamingTurn => AkraTheme::accent(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::INFINITE_AUTO_FOLLOW_MAX_TURNS;
    use crate::adapter::inbound::tui::app::{AutoFollowState, ConversationViewModel};

    #[test]
    fn auto_follow_status_lines_use_infinite_label() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        conversation.auto_follow_state = AutoFollowState::new();
        conversation
            .auto_follow_state
            .set_max_auto_turns(INFINITE_AUTO_FOLLOW_MAX_TURNS);
        conversation.auto_follow_state.runtime_phase = AutoFollowRuntimePhase::Running {
            started_at: Instant::now(),
            turn_index: 2,
        };

        assert_eq!(
            auto_follow_working_detail(&conversation),
            "auto turn 2/infinite running / interrupt unsupported"
        );
        assert_eq!(
            auto_follow_prompt_status_line(&conversation, true).as_deref(),
            Some("prompt: auto turn 2/infinite running  |  type now, Enter when idle")
        );
    }
}
