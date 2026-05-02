// 학습 주석: shell core의 view DTO들은 주로 contract test가 renderer 결과를 구조적으로 확인할 때만 필요합니다.
// production drawing path는 같은 입력을 바로 ratatui frame에 그리지만, 테스트는 Line/Rect를 꺼내 비교합니다.
#[cfg(test)]
use super::Line;
// 학습 주석: Rect도 테스트 전용 DTO에만 노출됩니다. shell layout이 나눈 header/transcript/footer/input 영역을
// snapshot 문자열이 아니라 좌표 단위로 검증하기 위한 타입입니다.
#[cfg(test)]
use super::Rect;
// 학습 주석: 최근 session 상태는 NativeTuiApp의 여러 capability flag를 읽어 사람이 읽는 짧은 label로 접습니다.
use super::capability_projection::recent_session_status_label;
// 학습 주석: shell core context는 app 전체를 직접 들고 다니지 않고, 렌더링에 필요한 conversation/startup/action 상태만
// 이 모듈의 작은 projection 타입으로 잘라냅니다.
use super::{
    ConversationState, ConversationViewModel, NativeTuiApp, ShellActionAvailability, StartupState,
};

// 학습 주석: ConversationShellView는 Rect 없이 "무엇을 그릴지"만 담는 테스트 view입니다.
// layout 좌표가 관심사가 아닌 contract test에서 shell chrome copy와 각 panel line을 직접 확인합니다.
#[cfg(test)]
pub(in super::super) struct ConversationShellView {
    // 학습 주석: outer shell frame title입니다. startup, ready, overlay 상태의 chrome 이름을 검증합니다.
    pub(in super::super) shell_title: Line<'static>,
    // 학습 주석: 상단 header에 표시되는 capability/session/action summary line 목록입니다.
    pub(in super::super) header_lines: Vec<Line<'static>>,
    // 학습 주석: transcript panel 본문입니다. live tail, history, startup inspection이 최종적으로 들어오는 영역입니다.
    pub(in super::super) conversation_lines: Vec<Line<'static>>,
    // 학습 주석: footer/status panel title입니다. renderer가 status block을 어떤 용도로 표시하는지 고정합니다.
    pub(in super::super) status_title: Line<'static>,
    // 학습 주석: runtime notices, warnings, approval/control status가 압축되어 내려오는 footer line 목록입니다.
    pub(in super::super) footer_lines: Vec<Line<'static>>,
    // 학습 주석: prompt/composer block title입니다. input mode가 바뀌어도 shell chrome copy를 검증할 수 있습니다.
    pub(in super::super) input_title: Line<'static>,
    // 학습 주석: 현재 draft input, command palette prompt, attachment mode copy가 렌더링되는 composer line 목록입니다.
    pub(in super::super) input_lines: Vec<Line<'static>>,
}

// 학습 주석: ConversationShellFrameView는 copy뿐 아니라 각 panel의 Rect까지 포함합니다. viewport 회귀가
// 문자열 snapshot에만 숨어 있지 않게, header/transcript/footer/input 분할을 구조적으로 확인합니다.
#[cfg(test)]
// 학습 주석: 일부 contract test는 전체 frame DTO 중 몇 필드만 보므로 dead_code를 허용합니다.
#[allow(dead_code)]
pub(in super::super) struct ConversationShellFrameView {
    // 학습 주석: 전체 shell frame의 title line입니다.
    pub(in super::super) shell_title: Line<'static>,
    // 학습 주석: header content line입니다. 아래 header_area와 짝을 이뤄 "무엇을 어디에" 그렸는지 보여 줍니다.
    pub(in super::super) header_lines: Vec<Line<'static>>,
    // 학습 주석: header가 차지한 terminal 좌표입니다. narrow/wide viewport layout test의 기준점입니다.
    pub(in super::super) header_area: Rect,
    // 학습 주석: transcript title, lines, scroll offset을 한데 묶은 nested view입니다.
    pub(in super::super) transcript_view: TranscriptPanelView,
    // 학습 주석: transcript panel 좌표입니다. inline tail과 overlays가 서로 겹치지 않는지 확인하는 핵심 영역입니다.
    pub(in super::super) transcript_area: Rect,
    // 학습 주석: status/footer panel title입니다.
    pub(in super::super) status_title: Line<'static>,
    // 학습 주석: footer에 실제로 표시될 status lines입니다.
    pub(in super::super) footer_lines: Vec<Line<'static>>,
    // 학습 주석: footer 좌표입니다. composer 높이 변화가 footer를 밀어내지 않는지 검증합니다.
    pub(in super::super) footer_area: Rect,
    // 학습 주석: composer title입니다.
    pub(in super::super) input_title: Line<'static>,
    // 학습 주석: composer 본문 line입니다.
    pub(in super::super) input_lines: Vec<Line<'static>>,
    // 학습 주석: composer 좌표입니다. cursor placement test가 이 영역을 기준으로 입력 위치를 계산합니다.
    pub(in super::super) input_area: Rect,
}

// 학습 주석: TranscriptPanelView는 transcript content와 scroll decision을 함께 담습니다. line 목록만 보면
// viewport가 tail을 따라갔는지 알 수 없기 때문에 scroll_offset을 별도 필드로 둡니다.
#[cfg(test)]
pub(in super::super) struct TranscriptPanelView {
    // 학습 주석: transcript panel title입니다. startup/inspection mode에서는 title copy가 상태를 드러냅니다.
    pub(in super::super) title: Line<'static>,
    // 학습 주석: rendered transcript lines입니다.
    pub(in super::super) lines: Vec<Line<'static>>,
    // 학습 주석: ratatui Paragraph에 넘길 vertical scroll offset입니다.
    pub(in super::super) scroll_offset: u16,
}

#[derive(Clone, Copy)]
// 학습 주석: shell renderer는 ConversationState 전체를 알 필요 없이 loading, failed, ready 세 단계만 필요합니다.
// 이 enum은 app state를 presentation-friendly reference로 줄여 shell copy/layout 함수의 입력을 단순하게 만듭니다.
pub(super) enum ShellConversationState<'a> {
    Loading,
    Failed(&'a str),
    Ready(&'a ConversationViewModel),
}

// 학습 주석: ShellCorePresentationContext는 NativeTuiApp에서 shell chrome을 그리는 데 필요한 읽기 전용 조각만
// 모은 DTO입니다. 렌더링 함수가 app 전체에 의존하지 않게 해 테스트에서 작은 context를 만들 수 있습니다.
pub(super) struct ShellCorePresentationContext<'a> {
    // 학습 주석: startup screen에서 ASCII banner를 노출할지 결정하는 feature/runtime flag입니다.
    pub(super) show_startup_ascii_art: bool,
    // 학습 주석: command palette, attachment mode, recovery anchor 같은 startup overlay 상태의 원본입니다.
    pub(super) startup_state: &'a StartupState,
    // 학습 주석: 현재 shell에서 가능한 action set입니다. header/help copy가 실행 가능 명령만 노출하게 합니다.
    pub(super) shell_action_availability: ShellActionAvailability,
    // 학습 주석: recent session capability를 이미 label로 접은 값입니다. downstream copy 함수가 app을 다시 읽지 않습니다.
    pub(super) recent_session_status_label: String,
    // 학습 주석: GitHub review polling 상태 label입니다. footer/header copy가 polling runtime detail을 표시합니다.
    pub(super) github_review_polling_status_label: String,
    // 학습 주석: debug detail 표시 여부는 production DTO에 필요 없고, rendering contract test에서만
    // planning detail line 노출을 검증하기 위해 보관합니다.
    #[cfg(test)]
    pub(super) planner_shows_debug_details: bool,
    // 학습 주석: conversation 상태는 loading/failed/ready projection으로 보관해 shell이 각 상태별
    // transcript placeholder와 startup 조건을 일관되게 계산하게 합니다.
    pub(super) conversation_state: ShellConversationState<'a>,
}

impl<'a> ShellCorePresentationContext<'a> {
    pub(super) fn from_app(app: &'a NativeTuiApp) -> Self {
        // 학습 주석: NativeTuiApp은 runtime, input, planning, session catalog 상태까지 들고 있는 큰 객체입니다.
        // 이 constructor는 shell core가 필요한 값만 즉시 projection해 렌더링 계층의 결합도를 낮춥니다.
        Self {
            // 학습 주석: startup banner flag는 app runtime state에서 직접 복사합니다.
            show_startup_ascii_art: app.show_startup_ascii_art,
            // 학습 주석: startup_state는 copy helper가 여러 필드를 읽으므로 참조로 보관합니다.
            startup_state: &app.startup_state,
            // 학습 주석: action availability는 method를 통해 계산해 app 내부 조건을 한곳에 캡슐화합니다.
            shell_action_availability: app.shell_action_availability(),
            // 학습 주석: recent session label은 capability_projection에 위임해 header copy와 status logic을 분리합니다.
            recent_session_status_label: recent_session_status_label(app),
            // 학습 주석: GitHub polling label은 app이 가진 adapter/runtime 상태를 짧은 문자열로 노출한 결과입니다.
            github_review_polling_status_label: app.github_review_polling_status_label(),
            // 학습 주석: 테스트 빌드에서만 debug detail 노출 여부를 context에 실어 snapshot helper가 참조합니다.
            #[cfg(test)]
            planner_shows_debug_details: app.planner_shows_debug_details(),
            // 학습 주석: app conversation state를 shell 전용 enum으로 변환합니다. ready 상태는 view model을
            // 빌리지 않고 참조만 넘겨 transcript/footer projection이 원본을 읽게 합니다.
            conversation_state: match &app.conversation_state {
                ConversationState::Loading => ShellConversationState::Loading,
                ConversationState::Failed(message) => ShellConversationState::Failed(message),
                ConversationState::Ready(conversation) => {
                    ShellConversationState::Ready(conversation)
                }
            },
        }
    }

    pub(super) fn ready_conversation(&self) -> Option<&'a ConversationViewModel> {
        // 학습 주석: ready-only renderer branch가 loading/failed 상태를 매번 직접 match하지 않도록
        // Option helper로 좁혀 줍니다.
        match self.conversation_state {
            ShellConversationState::Ready(conversation) => Some(conversation),
            _ => None,
        }
    }

    pub(super) fn startup_screen_is_active(&self) -> bool {
        // 학습 주석: startup screen은 ready conversation에서만 의미가 있습니다. loading/failed는 각각
        // 별도 placeholder를 그리므로 startup shell로 오인하면 안 됩니다.
        let Some(conversation) = self.ready_conversation() else {
            return false;
        };

        // 학습 주석: active thread, history message, live turn, live agent output이 모두 없어야 "초기 화면"입니다.
        // 하나라도 있으면 startup inspection 대신 실제 conversation transcript를 유지해야 합니다.
        !conversation.has_active_thread()
            && conversation.messages.is_empty()
            && conversation.active_turn_id.is_none()
            && conversation.live_agent_message.is_none()
    }

    pub(super) fn startup_banner_is_active(&self) -> bool {
        // 학습 주석: banner는 startup screen 조건과 별도 feature flag를 모두 만족해야 합니다. 이렇게 분리하면
        // startup screen은 유지하되 ASCII art만 끄는 설정을 renderer가 쉽게 표현할 수 있습니다.
        self.show_startup_ascii_art && self.startup_screen_is_active()
    }
}
