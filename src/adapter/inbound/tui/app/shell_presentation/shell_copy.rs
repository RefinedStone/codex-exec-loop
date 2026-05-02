// 학습 주석: loading 상태의 header copy는 capability_copy와 공유합니다. thread history를 불러오는 동안
// header와 footer가 서로 다른 표현을 쓰지 않도록 같은 helper를 사용합니다.
use super::capability_copy::thread_history_loading_header_line;
// 학습 주석: shell_copy는 shell_presentation 모듈의 공통 Line/Span/Style, theme, state projection 타입을
// 폭넓게 사용하는 copy 조립 계층이라 부모 prelude를 가져옵니다.
use super::*;

pub(super) fn build_shell_header_lines_with_context(
    // 학습 주석: NativeTuiApp에서 렌더링에 필요한 조각만 잘라낸 shell presentation context입니다.
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    // 학습 주석: header는 conversation lifecycle을 가장 먼저 보여 주는 영역입니다. loading/failed는
    // thread detail이 없거나 신뢰할 수 없고, ready만 실제 conversation metadata를 표시합니다.
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            // 학습 주석: title line은 shell 자체가 살아 있지만 thread metadata는 아직 loading 중임을 표시합니다.
            Line::from(vec![
                Span::styled("Conversation Shell", AkraTheme::title()),
                Span::raw(" / loading thread"),
            ]),
            // 학습 주석: 두 번째 line은 thread history loading 세부 상태를 capability helper에서 가져옵니다.
            Line::from(thread_history_loading_header_line()),
        ],
        ShellConversationState::Ready(conversation) => vec![
            // 학습 주석: ready title은 conversation title을 shell brand 뒤에 붙여 현재 session을 즉시 식별하게 합니다.
            Line::from(vec![
                Span::styled("Conversation Shell", AkraTheme::title()),
                Span::raw(" / "),
                Span::raw(conversation.title.clone()),
            ]),
            // 학습 주석: ready second line은 thread id, input state, startup action availability를 한 줄에 묶습니다.
            // header만 봐도 "어느 thread인가", "입력이 가능한가", "startup action이 막혔는가"를 판단할 수 있습니다.
            Line::from(vec![
                Span::raw(format!(
                    "thread: {}  |  input: ",
                    // 학습 주석: 새 draft는 아직 app-server thread가 없을 수 있어, 빈 id 대신 명시적 placeholder를 둡니다.
                    if conversation.has_active_thread() {
                        conversation.thread_id.as_str()
                    } else {
                        "not started yet"
                    }
                )),
                // 학습 주석: input state는 label뿐 아니라 style도 바꿔 armed/running/ready 상태를 시각적으로 구분합니다.
                Span::styled(
                    conversation.input_state.label(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw("  |  startup: "),
                // 학습 주석: startup action availability는 action gating 결과이므로 success/warning/danger style을 입힙니다.
                Span::styled(
                    context.shell_action_availability.status_text(),
                    startup_state_style_for_availability(context.shell_action_availability),
                ),
            ]),
        ],
        ShellConversationState::Failed(message) => vec![
            // 학습 주석: failed header는 title style 자체를 danger로 바꿔 conversation load failure가
            // 단순 status message가 아니라 shell의 현재 주 상태임을 드러냅니다.
            Line::from(vec![
                Span::styled("Conversation Shell", AkraTheme::danger()),
                Span::raw(" / failed"),
            ]),
            // 학습 주석: failure message는 header에도 올려 transcript/footer를 보지 않아도 원인을 알 수 있게 합니다.
            Line::from(message.to_string()),
        ],
    }
}

pub(super) fn build_shell_title() -> Line<'static> {
    // 학습 주석: outer shell block title은 전역 navigation shortcut만 담습니다. thread별 상태는
    // header/footer가 담당하므로 여기서는 chrome의 고정 affordance를 유지합니다.
    Line::from("Shell / Ctrl+t new draft / Ctrl+C back / Ctrl+q quit")
}

pub(super) fn build_transcript_title_with_context(
    // 학습 주석: 현재는 transcript title이 state를 반영하지 않지만, call-site가 context를 넘기는
    // 형태를 유지해 startup/inspection title 분기가 필요해질 때 signature를 바꾸지 않게 합니다.
    _context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    // 학습 주석: transcript panel은 app-server output과 host scrollback tail을 보여 주는 영역임을 고정 copy로 표시합니다.
    Line::from("Transcript / live scrollback")
}

pub(in super::super) fn build_status_title() -> Line<'static> {
    // 학습 주석: status panel title은 footer content가 shortcut guide와 live runtime status를 함께 담는다는 계약입니다.
    Line::from("Controls / shell shortcuts and live status")
}

pub(super) fn build_input_title_with_context(
    // 학습 주석: input title은 conversation lifecycle과 input state를 동시에 반영합니다.
    context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    // 학습 주석: loading/failed에서는 prompt가 실제 submit target을 갖지 않으므로 unavailable copy를 보여 주고,
    // ready에서만 submit hint와 newline hint를 표시합니다.
    match context.conversation_state {
        ShellConversationState::Loading => {
            // 학습 주석: metadata loading 중에도 prompt block 위치는 유지하되, 아직 submit할 수 없음을 title에 표시합니다.
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / loading")])
        }
        ShellConversationState::Failed(_) => {
            // 학습 주석: conversation load failure에서는 prompt submit이 의미 없으므로 unavailable 상태를 표시합니다.
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / unavailable")])
        }
        ShellConversationState::Ready(conversation) => {
            // 학습 주석: submit hint는 startup armed, running turn, action availability를 반영하므로
            // input title의 핵심 행동 안내가 됩니다.
            let submit_hint = build_primary_submit_hint_with_context(context);
            Line::from(vec![
                Span::raw("Prompt"),
                Span::raw(" / "),
                // 학습 주석: input state label은 composer가 받을 입력의 의미를 보여 줍니다.
                Span::styled(
                    conversation.input_state.label(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw(" / "),
                // 학습 주석: Enter가 지금 즉시 전송인지, 대기인지, 준비 후 전송인지 명시합니다.
                Span::raw(submit_hint),
                // 학습 주석: newline shortcut은 submit hint와 함께 있어 multi-line prompt 작성 방식을 잊지 않게 합니다.
                Span::raw(" / Ctrl+j newline"),
            ])
        }
    }
}

pub(super) fn build_frontend_summary_line() -> Line<'static> {
    // 학습 주석: frontend summary는 현재 TUI가 inline main buffer 방식으로 렌더링한다는 운영 모드 copy입니다.
    // snapshot/contract test에서 host scrollback과 prompt anchoring 기대값을 고정하는 역할도 합니다.
    Line::from(
        "frontend: inline main buffer  |  history: host terminal scrollback  |  tail: prompt anchored",
    )
}

fn build_primary_submit_hint_with_context(
    // 학습 주석: shell action availability와 conversation runtime 상태를 함께 가진 context입니다.
    context: &ShellCorePresentationContext<'_>,
) -> &'static str {
    // 학습 주석: Enter 안내는 사용자 행동과 직접 연결되므로 가장 구체적인 상태부터 검사합니다.
    // startup submit armed가 running/action blocked보다 우선해 "이미 대기열에 들어갔다"는 사실을 먼저 보여 줍니다.
    match context.conversation_state {
        ShellConversationState::Ready(conversation) if conversation.startup_submit_armed => {
            "queued until ready"
        }
        // 학습 주석: running turn 중에는 draft가 있어도 즉시 submit할 수 없으므로 idle 이후 전송 copy를 씁니다.
        ShellConversationState::Ready(conversation) if conversation.has_running_turn() => {
            "Enter send when idle"
        }
        // 학습 주석: startup/action gate가 아직 pending/blocked이면 ready conversation이라도 전송 조건을 제한합니다.
        ShellConversationState::Ready(_) if !context.shell_action_availability.allows_actions() => {
            "Enter send when ready"
        }
        // 학습 주석: 모든 gate가 열려 있고 running turn도 없으면 Enter가 즉시 submit 동작입니다.
        ShellConversationState::Ready(_) => "Enter send",
        // 학습 주석: loading/failed 상태에서는 input title에서 submit hint 자리를 비워 둡니다.
        _ => "",
    }
}

fn startup_state_style_for_availability(
    // 학습 주석: startup checks, session loading, capability gate를 합친 shell action availability입니다.
    shell_action_availability: ShellActionAvailability,
) -> Style {
    // 학습 주석: header의 startup segment는 action 가능 여부를 색으로 압축합니다. Ready는 성공,
    // Pending은 대기, Blocked는 operator action이 필요한 상태로 읽히게 합니다.
    match shell_action_availability {
        ShellActionAvailability::Ready => AkraTheme::success(),
        ShellActionAvailability::Pending => AkraTheme::warning(),
        ShellActionAvailability::Blocked => AkraTheme::danger(),
    }
}
