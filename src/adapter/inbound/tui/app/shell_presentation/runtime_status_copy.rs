use std::time::{Duration, Instant};

use crate::domain::text::compact_whitespace_detail;

use super::{
    AkraTheme, AutoFollowRuntimePhase, ConversationInputState, ConversationViewModel, Line,
    Modifier, Span,
};

// shell presentation의 런타임 상태 문구를 한곳에 모아 둔다. 컨트롤러 상태를 다시
// 해석하지 않고 `ConversationViewModel`의 projection만 읽어, 화면 조각들이 같은
// working/idle 판단과 auto-follow 문구를 공유하게 한다.
pub(super) fn compact_inline_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
}

pub(super) fn turn_status_label(conversation: &ConversationViewModel) -> &'static str {
    // auto-follow가 평가/큐/실행 중이면 사용자의 수동 turn이 없어도 하단 상태는
    // working이어야 한다. prompt 입력은 가능하더라도 런타임은 아직 살아 있기 때문이다.
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
    // auto-follow activity가 있으면 manual turn보다 우선해 표시한다. 자동 후속 작업은
    // 내부적으로 turn을 만들기 전 평가/큐 단계도 있으므로 별도 시작 시각을 사용한다.
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
    // status line은 terminal 폭을 가장 먼저 잃는 영역이라 detail을 whitespace 단위로
    // 접어 두고, elapsed는 monotonic Instant 기준으로만 계산한다.
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
        // submission 단계는 아직 streaming payload가 없으므로 시작 중임을 명확히 표시한다.
        ConversationInputState::SubmittingTurn => {
            Some(format!("starting turn / interrupt {interrupt_label}"))
        }
        ConversationInputState::StreamingTurn => {
            // live message가 생긴 뒤에는 agent가 실제 응답을 생산 중이고, 그 전에는
            // 서버 응답을 기다리는 상태로 분리해 사용자가 멈춤처럼 보지 않게 한다.
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
        // Idle은 보통 호출되지 않지만, projection 조합 실수에도 빈 문자열 대신 진단 가능한
        // 라벨을 남긴다.
        AutoFollowRuntimePhase::Idle => "idle".to_string(),
        AutoFollowRuntimePhase::Evaluating { .. } => "evaluating next auto-follow".to_string(),
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
    // prompt 영역 문구는 working line보다 짧다. interrupt 가능 여부는 이미 working
    // line에 있으므로 여기서는 사용자가 지금 입력해도 되는지에 초점을 둔다.
    let detail = match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => return None,
        AutoFollowRuntimePhase::Evaluating { .. } => "auto-follow evaluating".to_string(),
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
        // inline 모드는 입력 줄 옆에 붙기 때문에 "idle이 되면 Enter"라는 행동 단서까지
        // 한 줄로 압축한다.
        format!("prompt: {detail}  |  type now, Enter when idle")
    } else {
        detail
    })
}

pub(in super::super) fn format_elapsed(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    // 초 단위 상태는 즉시성을, 분/시간 단위 상태는 큰 흐름을 보여주도록 가장 큰 두
    // 단위까지만 노출한다.
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn inline_input_state_label(input_state: ConversationInputState) -> &'static str {
    // 좁은 footer에 들어가는 machine-readable에 가까운 라벨이다. 상세한 사용자 문장은
    // working line과 prompt notice에서만 만든다.
    match input_state {
        ConversationInputState::DraftReady => "draft",
        ConversationInputState::ReadyToContinue => "ready",
        ConversationInputState::SubmittingTurn => "sending",
        ConversationInputState::StreamingTurn => "streaming",
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

        // infinite 설정은 working line과 prompt notice 모두에서 같은 max-turn label을
        // 써야 사용자가 자동 후속 실행 한계를 다르게 읽지 않는다.
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
