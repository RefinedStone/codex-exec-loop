// 학습 주석: startup probe는 app-server initialize/account 조회 실패를 application service로 올려야 하므로
// 공통 오류 타입인 `anyhow::Result`를 사용합니다. 실패는 TUI에서 `StartupState::Failed`로 줄어듭니다.
use anyhow::Result;

// 학습 주석: attachment profile은 app-server가 새로 launch 되었는지, 기존 runtime에 reattach 되었는지를
// domain vocabulary로 표현하는 값입니다. port contract에 이 domain 타입을 싣기 때문에 startup service와 TUI는
// outbound adapter의 프로토콜 세부사항을 몰라도 같은 표시 모델을 사용할 수 있습니다.
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

#[derive(Debug, Clone)]
// 학습 주석: `AppServerStartupContext`는 app-server 쪽 startup probe가 돌려주는 application-facing snapshot입니다.
// `StartupService`는 여기에 local checks(cwd, codex binary, git workspace)를 더해 최종 `StartupDiagnostics`를 만듭니다.
//
// 학습 주석: 이 구조체는 app-server protocol 응답을 그대로 노출하지 않습니다. adapter가 initialize response,
// account 상태, warning을 해석한 뒤 TUI startup overlay가 바로 쓸 수 있는 필드로 정규화한 경계 값입니다.
pub struct AppServerStartupContext {
    // 학습 주석: startup 중 app-server 연결 방식입니다. service는 이 값을 diagnostics로 옮기고,
    // rendering layer는 attachment summary line으로 표시합니다.
    pub attachment_profile: TerminalBridgeAttachmentProfile,
    // 학습 주석: initialize/probe 요청이 어떤 상태로 끝났는지 설명하는 사람이 읽는 문자열입니다.
    // 성공 여부는 이 context를 얻은 시점에 이미 보장되므로 service는 `initialize_ok: true`로 매핑합니다.
    pub initialize_detail: String,
    // 학습 주석: 로그인 계정, auth 상태, 계정 관련 안내를 담는 표시 문자열입니다.
    pub account_detail: String,
    // 학습 주석: account readiness flag입니다. `StartupDiagnostics::can_continue()` 같은 domain 판단에서
    // prompt submit과 recent-session 조회 가능 여부를 결정하는 축이 됩니다.
    pub account_ok: bool,
    // 학습 주석: startup을 막지는 않지만 사용자에게 노출해야 하는 app-server 쪽 경고 목록입니다.
    // 예를 들어 schema mismatch나 계정 관련 non-blocking 안내가 여기에 들어갈 수 있습니다.
    pub warnings: Vec<String>,
}

// 학습 주석: `StartupProbePort`는 application service가 outbound app-server adapter에 요구하는 startup 전용 계약입니다.
// interactive turn 실행이나 session catalog 조회와 분리된 port를 두면 startup overlay가 필요한 짧은 probe만
// 독립적으로 테스트하고 교체할 수 있습니다.
//
// 학습 주석: `Send + Sync`는 이 port가 background startup task로 넘어갈 수 있다는 의미입니다.
// TUI는 화면을 그리는 thread와 별도로 startup checks를 실행하므로, port 구현은 thread-safe 공유가 가능해야 합니다.
pub trait StartupProbePort: Send + Sync {
    // 학습 주석: app-server에 연결해 startup context를 읽습니다. 성공하면 정규화된 context를,
    // 실패하면 startup service가 `StartupState::Failed`로 바꿀 수 있는 오류를 반환합니다.
    fn load_startup_context(&self) -> Result<AppServerStartupContext>;
}
