#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * `TerminalBridgeAttachmentMode`는 TUI가 terminal 실행 환경에 붙은 방식을 나타내는
 * 도메인 언어다. outbound adapter는 Codex app-server 프로세스를 새로 띄우거나 기존
 * runtime에 다시 붙는 절차를 수행하지만, application/TUI 경계에는 그 절차 대신 이 분류만
 * 전달한다. 덕분에 화면은 provider process lifecycle을 직접 해석하지 않고도 연결 방식을
 * 안정적으로 설명할 수 있다.
 *
 * 현재 제품 경로는 app-server 중심이라 `ProviderLaunch`와 `ProviderReattach`가 주로
 * 방출된다. 나머지 값은 local terminal, 관리 wrapper, 원격 attach, proxy 경유 attach가 같은
 * UI/diagnostics 계약에 들어올 수 있게 남겨 둔 확장 vocabulary다. 새 bridge adapter는 provider
 * 전용 분기를 늘리기보다 이 enum에 맞는 실제 attachment truth를 선택해야 한다.
 */
pub enum TerminalBridgeAttachmentMode {
    ProviderLaunch,
    ProviderReattach,
    LocalAttach,
    ManagedWrapper,
    RemoteAttach,
    ProxyMediated,
}

impl TerminalBridgeAttachmentMode {
    /*
     * label은 shell chrome, startup overlay, conversation runtime notice가 공유하는 표시
     * 문자열이다. 문자열을 렌더링 계층마다 직접 만들면 launch/reattach 표기가 어긋날 수
     * 있으므로, 도메인 값 옆에서 kebab-case copy를 고정한다.
     */
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProviderLaunch => "provider-launched",
            Self::ProviderReattach => "provider-reattach",
            Self::LocalAttach => "local-attach",
            Self::ManagedWrapper => "managed-wrapper",
            Self::RemoteAttach => "remote-attach",
            Self::ProxyMediated => "proxy-mediated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * recovery anchor는 attachment가 끊기거나 화면이 복구될 때 어떤 식별자를 기준으로 같은
 * 실행 문맥을 다시 찾을 수 있는지를 나타낸다. attachment mode가 "어떤 통로로 붙었나"라면,
 * recovery anchor는 "다시 붙을 때 무엇을 붙잡나"를 정의한다.
 *
 * app-server provider 경로는 thread id를 중심으로 복구된다. session handle이나 terminal session
 * 값은 다른 bridge 방식이 들어와도 startup diagnostics와 렌더링 copy가 같은 recovery 슬롯을
 * 계속 쓸 수 있게 남겨 둔 확장점이다.
 */
pub enum TerminalBridgeRecoveryAnchor {
    None,
    ProviderThreadId,
    SessionHandle,
    TerminalSession,
}

impl TerminalBridgeRecoveryAnchor {
    /*
     * startup 요약과 대화 로그는 recovery anchor를 pass/fail probe가 아니라 선택된 bridge
     * profile의 속성으로 보여 준다. 이 label이 그 공통 표시 계약이며, UI는 enum variant 이름을
     * 직접 문자열화하지 않는다.
     */
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ProviderThreadId => "provider-thread-id",
            Self::SessionHandle => "session-handle",
            Self::TerminalSession => "terminal-session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * attachment profile은 runtime adapter에서 관찰한 terminal bridge 상태를 domain 값으로 압축한
 * 결과다. `StartupDiagnostics`, `ConversationStreamEvent::AttachmentObserved`, shell capability
 * copy, conversation notice가 모두 이 구조체를 공유한다. 이 구조체가 application 경계의 공통
 * 언어가 되므로 UI 계층은 provider process lifecycle을 직접 알 필요 없이 `mode`와
 * `recovery_anchor`만 렌더링하면 된다.
 */
pub struct TerminalBridgeAttachmentProfile {
    // 사용자가 보는 연결 방식의 축이다. launch/reattach 구분은 이 값에서 나온다.
    pub mode: TerminalBridgeAttachmentMode,
    // 같은 bridge 문맥을 복구하거나 설명할 때 기준이 되는 식별자 종류다.
    pub recovery_anchor: TerminalBridgeRecoveryAnchor,
}

impl TerminalBridgeAttachmentProfile {
    /*
     * `new`는 테스트와 future adapter가 mode/recovery 조합을 명시적으로 만들 때 쓰는 가장 얇은
     * 생성자다. 정책 판단은 여기서 하지 않는다. 어떤 조합이 표준인지에 대한 이름은 아래의
     * `codex_app_server_*` 생성자에 붙여 호출부가 bridge 의도를 드러내게 한다.
     */
    pub const fn new(
        mode: TerminalBridgeAttachmentMode,
        recovery_anchor: TerminalBridgeRecoveryAnchor,
    ) -> Self {
        Self {
            mode,
            recovery_anchor,
        }
    }

    /*
     * 새 Codex app-server provider를 시작한 경로의 표준 profile이다. runtime adapter가 initialize를
     * 끝내고 startup diagnostics나 stream event를 만들 때 이 값을 넣으면, TUI는 첫 연결을
     * `provider-launched / provider-thread-id`로 일관되게 표시한다.
     */
    pub const fn codex_app_server_launch() -> Self {
        Self::new(
            TerminalBridgeAttachmentMode::ProviderLaunch,
            TerminalBridgeRecoveryAnchor::ProviderThreadId,
        )
    }

    /*
     * 이미 존재하는 Codex app-server provider thread에 다시 붙은 경로의 표준 profile이다. launch와
     * recovery anchor는 같을 수 있지만 mode를 따로 두어 사용자가 새 프로세스 시작과 기존 세션
     * 재사용을 구분해서 볼 수 있게 한다.
     */
    pub const fn codex_app_server_reattach() -> Self {
        Self::new(
            TerminalBridgeAttachmentMode::ProviderReattach,
            TerminalBridgeRecoveryAnchor::ProviderThreadId,
        )
    }

    /*
     * 현재 제품 기본값은 Codex app-server를 provider로 launch하는 native-first 흐름이다. 기본
     * profile 이름을 별도로 두면 호출부는 "현재 기본 app-server attachment"만 요청하고, 실제 기본
     * 모드가 바뀌어도 이 파일의 정책만 수정하면 된다.
     */
    pub const fn codex_app_server() -> Self {
        Self::codex_app_server_launch()
    }
}

impl Default for TerminalBridgeAttachmentProfile {
    // 테스트 fixture와 startup fallback이 명시 profile을 생략하면 현재 app-server launch 정책을 따른다.
    fn default() -> Self {
        Self::codex_app_server()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalBridgeAttachmentMode, TerminalBridgeAttachmentProfile, TerminalBridgeRecoveryAnchor,
    };

    #[test]
    fn codex_launch_profile_reports_provider_launch_and_thread_id_recovery() {
        // launch helper는 runtime adapter와 startup probe가 새 provider 시작을 보고할 때의 단일 계약이다.
        assert_eq!(
            TerminalBridgeAttachmentProfile::codex_app_server_launch(),
            TerminalBridgeAttachmentProfile::new(
                TerminalBridgeAttachmentMode::ProviderLaunch,
                TerminalBridgeRecoveryAnchor::ProviderThreadId,
            )
        );
    }

    #[test]
    fn codex_reattach_profile_reports_provider_reattach_and_thread_id_recovery() {
        // reattach helper는 기존 provider thread 재사용을 launch와 같은 recovery anchor로 표현한다.
        assert_eq!(
            TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
            TerminalBridgeAttachmentProfile::new(
                TerminalBridgeAttachmentMode::ProviderReattach,
                TerminalBridgeRecoveryAnchor::ProviderThreadId,
            )
        );
    }

    #[test]
    fn attachment_labels_stay_kebab_case() {
        // label 문자열은 snapshot 성격의 UI copy라서 kebab-case가 깨지면 shell/test fixture가 함께 흔들린다.
        assert_eq!(
            TerminalBridgeAttachmentMode::ProviderLaunch.label(),
            "provider-launched"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::ProviderReattach.label(),
            "provider-reattach"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::LocalAttach.label(),
            "local-attach"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::ManagedWrapper.label(),
            "managed-wrapper"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::RemoteAttach.label(),
            "remote-attach"
        );
        assert_eq!(
            TerminalBridgeAttachmentMode::ProxyMediated.label(),
            "proxy-mediated"
        );
        assert_eq!(
            TerminalBridgeRecoveryAnchor::ProviderThreadId.label(),
            "provider-thread-id"
        );
        assert_eq!(
            TerminalBridgeRecoveryAnchor::SessionHandle.label(),
            "session-handle"
        );
        assert_eq!(
            TerminalBridgeRecoveryAnchor::TerminalSession.label(),
            "terminal-session"
        );
    }
}
