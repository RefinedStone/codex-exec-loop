// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: `enum`은 가능한 상태나 명령을 정해진 선택지로 제한해 패턴 매칭으로 안전하게 처리하게 해줍니다.
pub enum TerminalBridgeAttachmentMode {
    ProviderLaunch,
    ProviderReattach,
    LocalAttach,
    ManagedWrapper,
    RemoteAttach,
    ProxyMediated,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl TerminalBridgeAttachmentMode {
    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    pub const fn label(self) -> &'static str {
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::ProviderLaunch => "provider-launched",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::ProviderReattach => "provider-reattach",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::LocalAttach => "local-attach",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::ManagedWrapper => "managed-wrapper",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::RemoteAttach => "remote-attach",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::ProxyMediated => "proxy-mediated",
        }
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: `enum`은 가능한 상태나 명령을 정해진 선택지로 제한해 패턴 매칭으로 안전하게 처리하게 해줍니다.
pub enum TerminalBridgeRecoveryAnchor {
    None,
    ProviderThreadId,
    SessionHandle,
    TerminalSession,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl TerminalBridgeRecoveryAnchor {
    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    pub const fn label(self) -> &'static str {
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::None => "none",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::ProviderThreadId => "provider-thread-id",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::SessionHandle => "session-handle",
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Self::TerminalSession => "terminal-session",
        }
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct TerminalBridgeAttachmentProfile {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub mode: TerminalBridgeAttachmentMode,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub recovery_anchor: TerminalBridgeRecoveryAnchor,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl TerminalBridgeAttachmentProfile {
    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    pub const fn new(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        mode: TerminalBridgeAttachmentMode,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        recovery_anchor: TerminalBridgeRecoveryAnchor,
    ) -> Self {
        Self {
            mode,
            recovery_anchor,
        }
    }

    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    pub const fn codex_app_server_launch() -> Self {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::new(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::ProviderLaunch,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeRecoveryAnchor::ProviderThreadId,
        )
    }

    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    pub const fn codex_app_server_reattach() -> Self {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::new(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::ProviderReattach,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeRecoveryAnchor::ProviderThreadId,
        )
    }

    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    pub const fn codex_app_server() -> Self {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::codex_app_server_launch()
    }
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl Default for TerminalBridgeAttachmentProfile {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn default() -> Self {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::codex_app_server()
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(test)]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod tests {
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use super::{
        TerminalBridgeAttachmentMode, TerminalBridgeAttachmentProfile, TerminalBridgeRecoveryAnchor,
    };

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn codex_launch_profile_reports_provider_launch_and_thread_id_recovery() {
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentProfile::codex_app_server_launch(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentProfile::new(
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                TerminalBridgeAttachmentMode::ProviderLaunch,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                TerminalBridgeRecoveryAnchor::ProviderThreadId,
            )
        );
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn codex_reattach_profile_reports_provider_reattach_and_thread_id_recovery() {
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentProfile::new(
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                TerminalBridgeAttachmentMode::ProviderReattach,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                TerminalBridgeRecoveryAnchor::ProviderThreadId,
            )
        );
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn attachment_labels_stay_kebab_case() {
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::ProviderLaunch.label(),
            "provider-launched"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::ProviderReattach.label(),
            "provider-reattach"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::LocalAttach.label(),
            "local-attach"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::ManagedWrapper.label(),
            "managed-wrapper"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::RemoteAttach.label(),
            "remote-attach"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeAttachmentMode::ProxyMediated.label(),
            "proxy-mediated"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeRecoveryAnchor::ProviderThreadId.label(),
            "provider-thread-id"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeRecoveryAnchor::SessionHandle.label(),
            "session-handle"
        );
        assert_eq!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            TerminalBridgeRecoveryAnchor::TerminalSession.label(),
            "terminal-session"
        );
    }
}
