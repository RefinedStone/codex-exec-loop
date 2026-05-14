#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
// ConversationMessage는 transcript의 한 항목이다. 텍스트뿐 아니라 phase, item_id,
// display_label을 함께 둬 streaming event와 최종 transcript rendering을 같은 타입으로 연결한다.
pub struct ConversationMessage {
    // kind는 user/agent/tool/status를 구분해 TUI 색상, prefix, grouping을 결정한다.
    pub kind: ConversationMessageKind,
    // text는 사용자가 보는 본문이다. debug_detail과 분리해 기본 transcript를 과하게 늘리지 않는다.
    pub text: String,
    // debug_detail은 debug visibility mode에서만 보이는 보조 설명으로, 기본 사용자 copy와 분리된다.
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
        /*
         * The base constructor keeps transcript rows intentionally small: kind, text,
         * phase, and item_id are the cross-layer contract shared by live stream
         * reduction and snapshot replay. Debug details and display labels are opt-in
         * builder additions so ordinary transcript rendering does not accidentally
         * inherit adapter-specific decoration.
         */
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
        /*
         * Display labels are renderer hints, not new message kinds. Keeping them as a
         * builder field lets adapters label tool/status rows more precisely while the
         * TUI can still fall back to ConversationMessageKind for baseline styling.
         */
        self.display_label = Some(label.into());
        self
    }

    pub fn with_debug_detail(mut self, detail: impl Into<String>) -> Self {
        /*
         * Debug detail is separated from text because it is diagnostic context, not
         * transcript content. This prevents replayed sessions and normal shell output
         * from growing extra rows when debug mode is disabled.
         */
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
// ConversationReasoningEffort is the UI-neutral turn option that Akra carries
// from operator selection to the outbound runtime. The app-server adapter maps
// this onto its wire enum at the boundary.
pub enum ConversationReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl ConversationReasoningEffort {
    pub const SUPPORTED_LABELS: &'static str = "none, minimal, low, medium, high, xhigh, default";

    pub fn parse(value: &str) -> Option<Self> {
        match normalize_turn_option_value(value).as_deref() {
            Some("none") | Some("off") => Some(Self::None),
            Some("minimal") => Some(Self::Minimal),
            Some("low") => Some(Self::Low),
            Some("medium") => Some(Self::Medium),
            Some("high") => Some(Self::High),
            Some("xhigh") | Some("extra-high") | Some("x-high") => Some(Self::XHigh),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// ConversationTurnOptions is the per-turn override bundle for interactive user
// sessions. The default is Akra's project-level model policy, not the provider
// fallback. None still means "let app-server use its current/default setting"
// when a caller explicitly needs that boundary behavior.
pub struct ConversationTurnOptions {
    pub model: Option<String>,
    pub reasoning_effort: Option<ConversationReasoningEffort>,
}

impl ConversationTurnOptions {
    pub const DEFAULT_MODEL: &'static str = "gpt-5.5";
    pub const DEFAULT_REASONING_EFFORT: ConversationReasoningEffort =
        ConversationReasoningEffort::High;

    pub fn app_server_default() -> Self {
        Self {
            model: None,
            reasoning_effort: None,
        }
    }

    pub fn is_default(&self) -> bool {
        self.model.as_deref() == Some(Self::DEFAULT_MODEL)
            && self.reasoning_effort == Some(Self::DEFAULT_REASONING_EFFORT)
    }

    pub fn summary_label(&self) -> String {
        format!(
            "model: {}  |  think: {}",
            self.model.as_deref().unwrap_or("default"),
            self.reasoning_effort
                .map(ConversationReasoningEffort::label)
                .unwrap_or("default")
        )
    }
}

impl Default for ConversationTurnOptions {
    fn default() -> Self {
        Self {
            model: Some(Self::DEFAULT_MODEL.to_string()),
            reasoning_effort: Some(Self::DEFAULT_REASONING_EFFORT),
        }
    }
}

fn normalize_turn_option_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_ascii_lowercase().replace(['_', ' '], "-"))
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
        /*
         * The constructor is const so backend capability truth can live in defaults and
         * static-style contexts. Approval and interrupt are deliberately independent
         * because backends may support one control path natively while requiring manual
         * handoff or no support for the other.
         */
        Self {
            approval,
            interrupt,
        }
    }

    pub const fn codex_app_server() -> Self {
        /*
         * The current codex app-server integration exposes conversation streaming and
         * manual approval handoff copy, but it does not yet surface a native approval
         * action in this domain truth. Interrupt is also marked unsupported here so UI
         * controls do not promise a stop capability unless the adapter changes the
         * contract explicitly.
         */
        Self::new(
            ConversationControlSupport::ManualHandoff,
            ConversationControlSupport::Unsupported,
        )
    }
}

impl Default for ConversationRuntimeControlTruth {
    fn default() -> Self {
        /*
         * Defaulting to codex_app_server keeps older call sites aligned with the native
         * app-server path. Alternative backends must opt in by supplying their own
         * truth instead of silently inheriting unsupported control affordances.
         */
        Self::codex_app_server()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_message_constructor_preserves_core_fields() {
        let message = ConversationMessage::new(
            ConversationMessageKind::Tool,
            "edited file",
            Some("completed".to_string()),
            Some("item-7".to_string()),
        );

        assert_eq!(message.kind, ConversationMessageKind::Tool);
        assert_eq!(message.text, "edited file");
        assert_eq!(message.phase.as_deref(), Some("completed"));
        assert_eq!(message.item_id.as_deref(), Some("item-7"));
        assert_eq!(message.debug_detail, None);
        assert_eq!(message.display_label, None);
    }

    #[test]
    fn conversation_message_builders_add_optional_rendering_metadata() {
        let message =
            ConversationMessage::new(ConversationMessageKind::Status, "running", None, None)
                .with_display_label("status")
                .with_debug_detail("raw event");

        assert_eq!(message.kind, ConversationMessageKind::Status);
        assert_eq!(message.text, "running");
        assert_eq!(message.phase, None);
        assert_eq!(message.item_id, None);
        assert_eq!(message.display_label.as_deref(), Some("status"));
        assert_eq!(message.debug_detail.as_deref(), Some("raw event"));
    }

    #[test]
    fn reasoning_effort_parse_accepts_supported_user_spellings() {
        let cases = [
            (" none ", ConversationReasoningEffort::None),
            ("OFF", ConversationReasoningEffort::None),
            ("minimal", ConversationReasoningEffort::Minimal),
            ("Low", ConversationReasoningEffort::Low),
            ("medium", ConversationReasoningEffort::Medium),
            ("HIGH", ConversationReasoningEffort::High),
            ("xhigh", ConversationReasoningEffort::XHigh),
            ("extra high", ConversationReasoningEffort::XHigh),
            ("extra_high", ConversationReasoningEffort::XHigh),
            ("x-high", ConversationReasoningEffort::XHigh),
        ];

        for (input, expected) in cases {
            assert_eq!(ConversationReasoningEffort::parse(input), Some(expected));
        }
    }

    #[test]
    fn reasoning_effort_parse_rejects_empty_default_and_unknown_values() {
        for input in ["", "   ", "default", "auto", "maximum"] {
            assert_eq!(ConversationReasoningEffort::parse(input), None);
        }
    }

    #[test]
    fn reasoning_effort_label_returns_canonical_values() {
        let cases = [
            (ConversationReasoningEffort::None, "none"),
            (ConversationReasoningEffort::Minimal, "minimal"),
            (ConversationReasoningEffort::Low, "low"),
            (ConversationReasoningEffort::Medium, "medium"),
            (ConversationReasoningEffort::High, "high"),
            (ConversationReasoningEffort::XHigh, "xhigh"),
        ];

        for (effort, label) in cases {
            assert_eq!(effort.label(), label);
        }
    }

    #[test]
    fn turn_options_default_to_akra_project_model_policy() {
        let options = ConversationTurnOptions::default();

        assert_eq!(options.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            options.reasoning_effort,
            Some(ConversationReasoningEffort::High)
        );
        assert!(options.is_default());
        assert_eq!(options.summary_label(), "model: gpt-5.5  |  think: high");
    }

    #[test]
    fn app_server_default_is_explicit_and_not_project_default() {
        let options = ConversationTurnOptions::app_server_default();

        assert_eq!(options.model, None);
        assert_eq!(options.reasoning_effort, None);
        assert!(!options.is_default());
        assert_eq!(options.summary_label(), "model: default  |  think: default");
    }

    #[test]
    fn turn_options_default_detection_requires_exact_policy_values() {
        let wrong_model = ConversationTurnOptions {
            model: Some("GPT-5.5".to_string()),
            reasoning_effort: Some(ConversationReasoningEffort::High),
        };
        let wrong_effort = ConversationTurnOptions {
            model: Some(ConversationTurnOptions::DEFAULT_MODEL.to_string()),
            reasoning_effort: Some(ConversationReasoningEffort::Medium),
        };
        let missing_model = ConversationTurnOptions {
            model: None,
            reasoning_effort: Some(ConversationReasoningEffort::High),
        };

        assert!(!wrong_model.is_default());
        assert!(!wrong_effort.is_default());
        assert!(!missing_model.is_default());
    }

    #[test]
    fn turn_options_summary_uses_default_placeholders_independently() {
        let default_model_only = ConversationTurnOptions {
            model: None,
            reasoning_effort: Some(ConversationReasoningEffort::XHigh),
        };
        let default_effort_only = ConversationTurnOptions {
            model: Some("gpt-5.4".to_string()),
            reasoning_effort: None,
        };

        assert_eq!(
            default_model_only.summary_label(),
            "model: default  |  think: xhigh"
        );
        assert_eq!(
            default_effort_only.summary_label(),
            "model: gpt-5.4  |  think: default"
        );
    }

    #[test]
    fn runtime_control_truth_defaults_to_codex_app_server_contract() {
        let truth = ConversationRuntimeControlTruth::default();

        assert_eq!(truth, ConversationRuntimeControlTruth::codex_app_server());
        assert_eq!(truth.approval, ConversationControlSupport::ManualHandoff);
        assert_eq!(truth.interrupt, ConversationControlSupport::Unsupported);
    }

    #[test]
    fn runtime_control_truth_constructor_keeps_capabilities_independent() {
        let truth = ConversationRuntimeControlTruth::new(
            ConversationControlSupport::RuntimeNative,
            ConversationControlSupport::ManualHandoff,
        );

        assert_eq!(truth.approval, ConversationControlSupport::RuntimeNative);
        assert_eq!(truth.interrupt, ConversationControlSupport::ManualHandoff);
    }
}
