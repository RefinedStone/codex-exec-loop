#[derive(Debug, Clone)]
// ConversationSnapshot은 app-server 세션을 TUI view model로 넘길 때 쓰는 도메인 단위의
// "현재 대화 전체 상태"이다. adapter가 raw event stream을 줄여 만든 결과를 application/UI 경계에 전달한다.
pub struct ConversationSnapshot {
    // thread_id는 resume/attach와 session catalog가 같은 대화를 다시 찾는 외부 식별자이다.
    pub thread_id: String,
    // title은 TUI 목록과 shell header에 표시되는 사람이 읽는 대화 이름이다.
    pub title: String,
    // cwd는 대화가 실행되는 workspace 기준 경로이며, session browser와 shell context copy에 연결된다.
    pub cwd: String,
    // messages는 transcript를 구성하는 도메인 메시지 목록이다. TUI adapter가 kind/phase를 보고 스타일을 입힌다.
    pub messages: Vec<ConversationMessage>,
    // warnings는 상태 줄에 붙는 사용자 주의 신호이다. transcript message와 분리해 footer summary로 압축된다.
    pub warnings: Vec<String>,
    // runtime_notices는 실행 중 발생한 안내/복구 정보이다. warning보다 운영 상태에 가까워 별도 footer row로 처리된다.
    pub runtime_notices: Vec<String>,
}

#[derive(Debug, Clone)]
// ConversationMessage는 transcript의 한 항목이다. 텍스트뿐 아니라 phase, item_id,
// display_label을 함께 둬 streaming event와 최종 transcript rendering을 같은 타입으로 연결한다.
pub struct ConversationMessage {
    // kind는 user/agent/tool/status를 구분해 TUI 색상, prefix, grouping을 결정한다.
    pub kind: ConversationMessageKind,
    // text는 사용자가 보는 본문이다. debug_detail과 분리해 기본 transcript를 과하게 늘리지 않는다.
    pub text: String,
    // debug_detail은 planner/debug mode에서만 보이는 보조 설명으로, 기본 사용자 copy와 분리된다.
    pub debug_detail: Option<String>,
    // phase는 app-server item lifecycle 단계이다. streaming/reduction 계층이 상태 message를 묶는 단서로 쓴다.
    pub phase: Option<String>,
    // item_id는 app-server event item과 transcript row를 연결하는 식별자이다.
    pub item_id: Option<String>,
    // display_label은 kind보다 구체적인 표시 이름이 필요할 때 adapter가 덮어쓰는 label이다.
    pub display_label: Option<String>,
}

impl ConversationMessage {
    pub fn new(
        // transcript row의 기본 분류이다.
        kind: ConversationMessageKind,
        // String과 &str을 모두 받을 수 있게 해 stream reducer call-site를 간결하게 유지한다.
        text: impl Into<String>,
        // app-server phase를 보존할 때 전달한다.
        phase: Option<String>,
        // app-server item id를 보존할 때 전달한다.
        item_id: Option<String>,
    ) -> Self {
        // 생성 시점에는 debug_detail과 display_label을 비워 두고, 필요한 call-site가 builder
        // method로 명시적으로 붙이게 한다. 기본 transcript row는 최소 정보만 갖는다.
        Self {
            kind,
            text: text.into(),
            debug_detail: None,
            phase,
            item_id,
            display_label: None,
        }
    }

    pub fn with_display_label(mut self, label: impl Into<String>) -> Self {
        // builder style로 label을 붙여, message 생성 흐름은 유지하면서 renderer용 표시 이름만 보강한다.
        self.display_label = Some(label.into());
        self
    }

    pub fn with_debug_detail(mut self, detail: impl Into<String>) -> Self {
        // debug detail은 일반 text와 별도 필드라, debug mode가 꺼져 있을 때 transcript 노이즈를 만들지 않는다.
        self.debug_detail = Some(detail.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ConversationMessageKind는 transcript row의 1차 분류이다. adapter는 이 enum만 보고도
// user prompt, agent answer, tool/status row의 기본 스타일과 위치를 정할 수 있다.
pub enum ConversationMessageKind {
    User,
    Agent,
    Tool,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ConversationToolActivityKind는 live tail/working line이 tool activity를 사람이 읽는
// 범주로 묶을 때 쓰는 도메인 분류이다.
pub enum ConversationToolActivityKind {
    FileChange,
    CommandExecution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// ConversationToolActivity는 active tool 상태를 transcript message와 별도 요약으로 나타낸다.
// file changes와 command execution을 footer/inline tail에서 짧게 보여 주는 데 사용된다.
pub struct ConversationToolActivity {
    // activity의 분류이다.
    pub kind: ConversationToolActivityKind,
    // activity summary 문구이다.
    pub text: String,
    // file change activity에서 변경 파일 수를 별도 숫자로 보존해 compact copy가 쉽게 만들 수 있게 한다.
    pub file_change_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// ConversationApprovalReviewStatus는 app-server approval review의 진행/결과 상태를 보존한다.
// Unknown을 열어 둬 upstream이 새 status string을 보내도 정보를 잃지 않고 readable copy로 넘길 수 있다.
pub enum ConversationApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
// ConversationApprovalReview는 특정 tool/item approval 요청의 상태이다. TUI status line과
// approval summary가 target item, risk, rationale을 같은 domain record에서 읽는다.
pub struct ConversationApprovalReview {
    // approval 대상이 되는 app-server item id이다.
    pub target_item_id: String,
    // approval review의 현재 상태이다.
    pub status: ConversationApprovalReviewStatus,
    // 위험도는 upstream이 제공할 때만 표시되므로 Option으로 유지한다.
    pub risk_level: Option<String>,
    // rationale은 operator가 승인/거절 이유를 이해할 수 있는 보조 설명이다.
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ConversationControlSupport는 approval/interrupt 같은 runtime control을 현재 backend가
// 어떻게 지원하는지 표현한다. UI는 이 truth를 보고 버튼 copy를 native/manual/unsupported로 나눈다.
pub enum ConversationControlSupport {
    RuntimeNative,
    ManualHandoff,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ConversationRuntimeControlTruth는 conversation backend별 control capability matrix이다.
// approval과 interrupt를 분리해, 한쪽은 수동 handoff이고 다른 쪽은 unsupported인 상황을 정확히 표시한다.
pub struct ConversationRuntimeControlTruth {
    // approval review에 대한 사용자 action 지원 수준이다.
    pub approval: ConversationControlSupport,
    // running turn interrupt에 대한 사용자 action 지원 수준이다.
    pub interrupt: ConversationControlSupport,
}

impl ConversationRuntimeControlTruth {
    pub const fn new(
        // approval capability이다.
        approval: ConversationControlSupport,
        // interrupt capability이다.
        interrupt: ConversationControlSupport,
    ) -> Self {
        // const constructor라 default/static context에서도 backend capability truth를 만들 수 있다.
        Self {
            approval,
            interrupt,
        }
    }

    pub const fn codex_app_server() -> Self {
        // 현재 codex app-server flow는 approval을 native API로 직접 처리하지 않고 manual
        // handoff copy로 안내하며, interrupt는 지원하지 않는 truth로 둔다.
        Self::new(
            ConversationControlSupport::ManualHandoff,
            ConversationControlSupport::Unsupported,
        )
    }
}

impl Default for ConversationRuntimeControlTruth {
    fn default() -> Self {
        // 별도 backend truth가 주입되지 않으면 native-first app-server flow를 기본값으로 삼는다.
        Self::codex_app_server()
    }
}
