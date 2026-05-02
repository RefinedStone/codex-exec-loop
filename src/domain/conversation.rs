#[derive(Debug, Clone)]
// 학습 주석: ConversationSnapshot은 app-server 세션을 TUI view model로 넘길 때 쓰는 도메인 단위의
// "현재 대화 전체 상태"입니다. adapter가 raw event stream을 줄여 만든 결과를 application/UI 경계에 전달합니다.
pub struct ConversationSnapshot {
    // 학습 주석: thread_id는 resume/attach와 session catalog가 같은 대화를 다시 찾는 외부 식별자입니다.
    pub thread_id: String,
    // 학습 주석: title은 TUI 목록과 shell header에 표시되는 사람이 읽는 대화 이름입니다.
    pub title: String,
    // 학습 주석: cwd는 대화가 실행되는 workspace 기준 경로이며, session browser와 shell context copy에 연결됩니다.
    pub cwd: String,
    // 학습 주석: messages는 transcript를 구성하는 도메인 메시지 목록입니다. TUI adapter가 kind/phase를 보고 스타일을 입힙니다.
    pub messages: Vec<ConversationMessage>,
    // 학습 주석: warnings는 상태 줄에 붙는 사용자 주의 신호입니다. transcript message와 분리해 footer summary로 압축됩니다.
    pub warnings: Vec<String>,
    // 학습 주석: runtime_notices는 실행 중 발생한 안내/복구 정보입니다. warning보다 운영 상태에 가까워 별도 footer row로 처리됩니다.
    pub runtime_notices: Vec<String>,
}

#[derive(Debug, Clone)]
// 학습 주석: ConversationMessage는 transcript의 한 항목입니다. 텍스트뿐 아니라 phase, item_id,
// display_label을 함께 둬 streaming event와 최종 transcript rendering을 같은 타입으로 연결합니다.
pub struct ConversationMessage {
    // 학습 주석: kind는 user/agent/tool/status를 구분해 TUI 색상, prefix, grouping을 결정합니다.
    pub kind: ConversationMessageKind,
    // 학습 주석: text는 사용자가 보는 본문입니다. debug_detail과 분리해 기본 transcript를 과하게 늘리지 않습니다.
    pub text: String,
    // 학습 주석: debug_detail은 planner/debug mode에서만 보이는 보조 설명으로, 기본 사용자 copy와 분리됩니다.
    pub debug_detail: Option<String>,
    // 학습 주석: phase는 app-server item lifecycle 단계입니다. streaming/reduction 계층이 상태 message를 묶는 단서로 씁니다.
    pub phase: Option<String>,
    // 학습 주석: item_id는 app-server event item과 transcript row를 연결하는 식별자입니다.
    pub item_id: Option<String>,
    // 학습 주석: display_label은 kind보다 구체적인 표시 이름이 필요할 때 adapter가 덮어쓰는 label입니다.
    pub display_label: Option<String>,
}

impl ConversationMessage {
    pub fn new(
        // 학습 주석: transcript row의 기본 분류입니다.
        kind: ConversationMessageKind,
        // 학습 주석: String과 &str을 모두 받을 수 있게 해 stream reducer call-site를 간결하게 유지합니다.
        text: impl Into<String>,
        // 학습 주석: app-server phase를 보존할 때 전달합니다.
        phase: Option<String>,
        // 학습 주석: app-server item id를 보존할 때 전달합니다.
        item_id: Option<String>,
    ) -> Self {
        // 학습 주석: 생성 시점에는 debug_detail과 display_label을 비워 두고, 필요한 call-site가 builder
        // method로 명시적으로 붙이게 합니다. 기본 transcript row는 최소 정보만 갖습니다.
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
        // 학습 주석: builder style로 label을 붙여, message 생성 흐름은 유지하면서 renderer용 표시 이름만 보강합니다.
        self.display_label = Some(label.into());
        self
    }

    pub fn with_debug_detail(mut self, detail: impl Into<String>) -> Self {
        // 학습 주석: debug detail은 일반 text와 별도 필드라, debug mode가 꺼져 있을 때 transcript 노이즈를 만들지 않습니다.
        self.debug_detail = Some(detail.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: ConversationMessageKind는 transcript row의 1차 분류입니다. adapter는 이 enum만 보고도
// user prompt, agent answer, tool/status row의 기본 스타일과 위치를 정할 수 있습니다.
pub enum ConversationMessageKind {
    User,
    Agent,
    Tool,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: ConversationToolActivityKind는 live tail/working line이 tool activity를 사람이 읽는
// 범주로 묶을 때 쓰는 도메인 분류입니다.
pub enum ConversationToolActivityKind {
    FileChange,
    CommandExecution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: ConversationToolActivity는 active tool 상태를 transcript message와 별도 요약으로 나타냅니다.
// file changes와 command execution을 footer/inline tail에서 짧게 보여 주는 데 사용됩니다.
pub struct ConversationToolActivity {
    // 학습 주석: activity의 분류입니다.
    pub kind: ConversationToolActivityKind,
    // 학습 주석: activity summary 문구입니다.
    pub text: String,
    // 학습 주석: file change activity에서 변경 파일 수를 별도 숫자로 보존해 compact copy가 쉽게 만들 수 있게 합니다.
    pub file_change_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: ConversationApprovalReviewStatus는 app-server approval review의 진행/결과 상태를 보존합니다.
// Unknown을 열어 둬 upstream이 새 status string을 보내도 정보를 잃지 않고 readable copy로 넘길 수 있습니다.
pub enum ConversationApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: ConversationApprovalReview는 특정 tool/item approval 요청의 상태입니다. TUI status line과
// approval summary가 target item, risk, rationale을 같은 domain record에서 읽습니다.
pub struct ConversationApprovalReview {
    // 학습 주석: approval 대상이 되는 app-server item id입니다.
    pub target_item_id: String,
    // 학습 주석: approval review의 현재 상태입니다.
    pub status: ConversationApprovalReviewStatus,
    // 학습 주석: 위험도는 upstream이 제공할 때만 표시되므로 Option으로 유지합니다.
    pub risk_level: Option<String>,
    // 학습 주석: rationale은 operator가 승인/거절 이유를 이해할 수 있는 보조 설명입니다.
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: ConversationControlSupport는 approval/interrupt 같은 runtime control을 현재 backend가
// 어떻게 지원하는지 표현합니다. UI는 이 truth를 보고 버튼 copy를 native/manual/unsupported로 나눕니다.
pub enum ConversationControlSupport {
    RuntimeNative,
    ManualHandoff,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: ConversationRuntimeControlTruth는 conversation backend별 control capability matrix입니다.
// approval과 interrupt를 분리해, 한쪽은 수동 handoff이고 다른 쪽은 unsupported인 상황을 정확히 표시합니다.
pub struct ConversationRuntimeControlTruth {
    // 학습 주석: approval review에 대한 사용자 action 지원 수준입니다.
    pub approval: ConversationControlSupport,
    // 학습 주석: running turn interrupt에 대한 사용자 action 지원 수준입니다.
    pub interrupt: ConversationControlSupport,
}

impl ConversationRuntimeControlTruth {
    pub const fn new(
        // 학습 주석: approval capability입니다.
        approval: ConversationControlSupport,
        // 학습 주석: interrupt capability입니다.
        interrupt: ConversationControlSupport,
    ) -> Self {
        // 학습 주석: const constructor라 default/static context에서도 backend capability truth를 만들 수 있습니다.
        Self {
            approval,
            interrupt,
        }
    }

    pub const fn codex_app_server() -> Self {
        // 학습 주석: 현재 codex app-server flow는 approval을 native API로 직접 처리하지 않고 manual
        // handoff copy로 안내하며, interrupt는 지원하지 않는 truth로 둡니다.
        Self::new(
            ConversationControlSupport::ManualHandoff,
            ConversationControlSupport::Unsupported,
        )
    }
}

impl Default for ConversationRuntimeControlTruth {
    fn default() -> Self {
        // 학습 주석: 별도 backend truth가 주입되지 않으면 native-first app-server flow를 기본값으로 삼습니다.
        Self::codex_app_server()
    }
}
