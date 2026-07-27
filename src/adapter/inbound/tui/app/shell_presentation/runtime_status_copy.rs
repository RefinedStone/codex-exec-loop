use std::time::{Duration, Instant};

use crate::core::app::AutoFollowPhase;
use crate::domain::text::compact_whitespace_detail;

use super::{
    AkraTheme, ConversationInputState, ConversationRuntimeStatusScreenModel, Line, Modifier, Span,
};

// shell presentation의 런타임 상태 문구를 한곳에 모아 둔다. 컨트롤러 상태를 다시
// 해석하지 않고 사전에 캡처한 runtime-status projection만 읽어, 화면 조각들이 같은
// working/idle 판단과 auto-follow 문구를 공유하게 한다.
pub(super) fn compact_inline_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
}

pub(super) fn build_working_line(
    runtime_status: &ConversationRuntimeStatusScreenModel,
    max_detail_len: usize,
    rendered_at: Instant,
) -> Option<Line<'static>> {
    // auto-follow activity가 있으면 manual turn보다 우선해 표시한다. 자동 후속 작업은
    // 내부적으로 turn을 만들기 전 평가/큐 단계도 있으므로 별도 시작 시각을 사용한다.
    let detail = if runtime_status.post_turn_settlement_in_flight {
        "settling planning queue".to_string()
    } else if !matches!(runtime_status.auto_follow_phase, AutoFollowPhase::Idle) {
        auto_follow_working_detail(runtime_status)
    } else {
        manual_turn_working_detail(runtime_status)?
    };
    let started_at = runtime_status.working_started_at?;
    // status line은 terminal 폭을 가장 먼저 잃는 영역이라 detail을 whitespace 단위로
    // 접어 두고, elapsed는 monotonic Instant 기준으로만 계산한다.
    let detail = compact_inline_detail(&detail, max_detail_len);
    let elapsed = format_elapsed(rendered_at.saturating_duration_since(started_at));

    Some(Line::from(vec![
        Span::styled(
            "◦ Working".to_string(),
            AkraTheme::muted().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" ({elapsed} • {detail})"), AkraTheme::subtle()),
    ]))
}

fn manual_turn_working_detail(
    runtime_status: &ConversationRuntimeStatusScreenModel,
) -> Option<String> {
    let interrupt_label = runtime_status.interrupt_support_label;
    match runtime_status.input_state {
        // submission 단계는 아직 streaming payload가 없으므로 시작 중임을 명확히 표시한다.
        ConversationInputState::SubmittingTurn => {
            Some(format!("starting turn / interrupt {interrupt_label}"))
        }
        ConversationInputState::StreamingTurn => {
            // live message가 생긴 뒤에는 agent가 실제 응답을 생산 중이고, 그 전에는
            // 서버 응답을 기다리는 상태로 분리해 사용자가 멈춤처럼 보지 않게 한다.
            if runtime_status.live_agent_message_present {
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

fn auto_follow_working_detail(runtime_status: &ConversationRuntimeStatusScreenModel) -> String {
    let max_auto_turns = &runtime_status.auto_follow_max_turns_label;
    let interrupt_label = runtime_status.interrupt_support_label;
    match &runtime_status.auto_follow_phase {
        // Idle은 보통 호출되지 않지만, projection 조합 실수에도 빈 문자열 대신 진단 가능한
        // 라벨을 남긴다.
        AutoFollowPhase::Idle => "idle".to_string(),
        AutoFollowPhase::Queued { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} queued for submission")
        }
        AutoFollowPhase::Submitting { turn_index, .. } => {
            format!(
                "auto turn {turn_index}/{max_auto_turns} starting / interrupt {interrupt_label}"
            )
        }
        AutoFollowPhase::Running { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} running / interrupt {interrupt_label}")
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_line_preserves_runtime_priority_and_copy() {
        let started_at = Instant::now();
        let rendered_at = started_at + Duration::from_secs(2);
        let mut runtime_status = ConversationRuntimeStatusScreenModel {
            working_started_at: Some(started_at),
            post_turn_settlement_in_flight: false,
            auto_follow_phase: AutoFollowPhase::Idle,
            auto_follow_max_turns_label: "infinite".to_string(),
            input_state: ConversationInputState::DraftReady,
            live_agent_message_present: false,
            interrupt_support_label: "runtime-native",
        };
        let rendered = |runtime_status: &ConversationRuntimeStatusScreenModel| {
            build_working_line(runtime_status, 80, rendered_at).map(|line| line.to_string())
        };

        assert!(rendered(&runtime_status).is_none());

        runtime_status.input_state = ConversationInputState::SubmittingTurn;
        assert!(rendered(&runtime_status).is_some_and(|line| line.contains("starting turn")));

        runtime_status.input_state = ConversationInputState::StreamingTurn;
        assert!(
            rendered(&runtime_status).is_some_and(|line| line.contains("waiting for response"))
        );
        runtime_status.live_agent_message_present = true;
        assert!(rendered(&runtime_status).is_some_and(|line| line.contains("turn running")));

        runtime_status.auto_follow_phase = AutoFollowPhase::Queued {
            started_at,
            turn_index: 2,
        };
        assert!(
            rendered(&runtime_status)
                .is_some_and(|line| line.contains("auto turn 2/infinite queued for submission"))
        );

        runtime_status.post_turn_settlement_in_flight = true;
        let settlement = rendered(&runtime_status).expect("settlement should project working copy");
        assert!(settlement.contains("settling planning queue"));
        assert!(!settlement.contains("auto turn"));
    }
}
