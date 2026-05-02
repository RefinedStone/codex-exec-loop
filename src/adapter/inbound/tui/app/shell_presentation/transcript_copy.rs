// 학습 주석: conversation_text helper는 message kind와 content를 사람이 읽는 speaker label로 바꿉니다.
// transcript formatter는 label 문구를 직접 만들지 않고 shared helper를 써서 다른 conversation surfaces와 맞춥니다.
use crate::adapter::inbound::tui::conversation_text::conversation_message_label;

// 학습 주석: transcript copy는 shell presentation 내부 type만 소비합니다. message model을 styled Line/Span으로
// 변환하고, history cap과 theme style을 여기서 적용해 renderer가 단순히 lines를 그리게 합니다.
use super::{
    AkraTheme, ConversationMessage, ConversationMessageKind, Line, MAX_CONVERSATION_HISTORY_LINES,
    Span, Style,
};

// 학습 주석: 일반 transcript formatting entry입니다. debug detail은 기본 shell transcript에서는 숨기고,
// 별도 debug toggle path만 아래 `_with_debug` 함수를 직접 호출합니다.
pub(in super::super) fn format_conversation_lines(
    // 학습 주석: messages는 conversation state가 보관하는 logical message list입니다. 여기서 renderer용 line으로
    // projection하지만 원본 message는 변경하지 않습니다.
    messages: &[ConversationMessage],
) -> Vec<Line<'static>> {
    format_conversation_lines_with_debug(messages, false)
}

// 학습 주석: 이 함수는 conversation messages를 transcript panel에 들어갈 styled lines로 펼칩니다. message마다
// label line, indented body lines, optional debug lines, blank separator를 만들어 terminal transcript의 읽기
// 순서를 고정합니다.
pub(in super::super) fn format_conversation_lines_with_debug(
    // 학습 주석: logical transcript input입니다. 각 message는 kind, text, optional debug detail을 갖습니다.
    messages: &[ConversationMessage],
    // 학습 주석: true일 때만 debug_detail을 transcript에 섞습니다. 평상시 사용자 transcript와 내부 진단을 분리하는 gate입니다.
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    // 학습 주석: renderer가 받을 최종 line buffer입니다. message를 순회하며 push하고, 마지막에 empty/history
    // cap 처리를 적용합니다.
    let mut lines = Vec::new();

    // 학습 주석: message 순서를 그대로 유지해야 transcript 시간 순서가 보존됩니다.
    for message in messages {
        // 학습 주석: label line은 speaker/tool/status 경계를 눈으로 빠르게 구분하게 합니다.
        let label = conversation_message_label(message);
        lines.push(Line::from(Span::styled(
            format!("{label}:"),
            label_style(message.kind),
        )));
        // 학습 주석: message body는 원문 줄바꿈을 보존하되 두 칸 들여써 label과 구분합니다. tab은 terminal
        // width 차이를 줄이기 위해 spaces로 확장합니다.
        for text_line in message.text.lines() {
            lines.push(Line::from(format!("  {}", expand_tui_tabs(text_line))));
        }
        // 학습 주석: debug detail은 message body와 같은 위치에 붙지만 muted style을 적용합니다. 이 값은
        // operator/debug mode용이라 show_debug_details가 꺼져 있으면 렌더링하지 않습니다.
        if show_debug_details && let Some(debug_detail) = message.debug_detail.as_deref() {
            // 학습 주석: debug detail도 multiline일 수 있어 body와 같은 줄 단위 확장 규칙을 적용합니다.
            for detail_line in debug_detail.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", expand_tui_tabs(detail_line)),
                    // 학습 주석: muted style은 debug text가 primary conversation content보다 낮은 우선순위임을 보여 줍니다.
                    AkraTheme::muted(),
                )));
            }
        }
        // 학습 주석: blank line은 message block 간 간격입니다. cap 처리는 마지막에 하므로 이 separator도
        // transcript height 계산에 포함됩니다.
        lines.push(Line::from(""));
    }

    // 학습 주석: thread가 비어 있어도 transcript panel이 완전히 blank가 되지 않게 placeholder line을 제공합니다.
    if lines.is_empty() {
        lines.push(Line::from("No messages in this thread yet."));
    }

    // 학습 주석: transcript line buffer가 너무 커지면 rendering 비용과 scroll 계산이 커집니다. 가장 오래된
    // lines를 버리고 최근 history만 유지해 terminal frame을 안정적으로 렌더링합니다.
    if lines.len() > MAX_CONVERSATION_HISTORY_LINES {
        lines.drain(0..lines.len() - MAX_CONVERSATION_HISTORY_LINES);
    }

    lines
}

// 학습 주석: ratatui line layout은 tab width를 terminal/environment마다 다르게 해석할 수 있습니다. transcript
// projection에서 tab을 spaces로 고정해 message alignment가 흔들리지 않게 합니다.
fn expand_tui_tabs(text: &str) -> String {
    text.replace('\t', "    ")
}

// 학습 주석: message kind별 label style을 고르는 작은 presentation policy입니다. label만 styled하고 body는
// plain으로 두어 speaker identity가 강조되지만 content 색이 과하게 섞이지 않게 합니다.
fn label_style(kind: ConversationMessageKind) -> Style {
    // 학습 주석: enum match라 새 message kind가 추가되면 style 정책도 컴파일 시점에 업데이트를 요구합니다.
    match kind {
        // 학습 주석: user label은 shortcut/accent 색으로 표시해 operator input을 빠르게 찾게 합니다.
        ConversationMessageKind::User => AkraTheme::shortcut(),
        // 학습 주석: agent label은 brand 색으로 표시해 assistant response block을 구분합니다.
        ConversationMessageKind::Agent => AkraTheme::brand(),
        // 학습 주석: tool label은 tool-specific style로 표시해 command/tool output을 대화 text와 분리합니다.
        ConversationMessageKind::Tool => AkraTheme::tool(),
        // 학습 주석: status label은 muted style로 표시해 background 상태 message가 primary content를 압도하지 않게 합니다.
        ConversationMessageKind::Status => AkraTheme::muted(),
    }
}
