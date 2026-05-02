// 학습 주석: terminal bridge attachment profile은 Codex app-server와 TUI terminal bridge가 어떤 방식으로
// 붙어 있는지를 설명하는 domain 값입니다. startup diagnostics는 이 값을 함께 보관해 startup overlay가
// 연결 형태를 사용자에게 설명할 수 있게 합니다.
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

// 학습 주석: bundled schema snapshot은 현재 binary가 기준으로 삼는 app-server protocol schema 파일입니다.
// startup banner에서 이 label을 보여 주면 실행 중인 binary가 어떤 schema snapshot으로 빌드됐는지 추적할 수 있습니다.
const BUNDLED_SCHEMA_SNAPSHOT_PATH: &str = "schema/codex_app_server_protocol.v2.schemas.json";
// 학습 주석: include_str은 schema snapshot 내용을 binary에 embed합니다. runtime filesystem에 schema 파일이
// 없어도 startup diagnostics는 빌드 시점 snapshot 크기/출처를 표시할 수 있습니다.
const BUNDLED_SCHEMA_SNAPSHOT_CONTENTS: &str =
    include_str!("../../schema/codex_app_server_protocol.v2.schemas.json");

// 학습 주석: StartupDiagnostics는 startup service가 수행한 readiness checks의 domain snapshot입니다.
// TUI는 이 struct로 startup overlay, prompt submit gating, recent session loading gate를 결정합니다.
#[derive(Debug, Clone)]
pub struct StartupDiagnostics {
    // 학습 주석: cwd는 process가 startup checks를 실행한 기준 directory입니다. workspace path와 다를 수 있어
    // 문제 진단 시 둘을 나눠 보여 줍니다.
    pub cwd: String,
    // 학습 주석: codex_binary_ok는 official Codex binary/probe가 사용 가능하다는 readiness flag입니다.
    pub codex_binary_ok: bool,
    // 학습 주석: codex_binary_detail은 binary path/version/probe failure 같은 사람이 읽을 진단 문구입니다.
    pub codex_binary_detail: String,
    // 학습 주석: workspace_ok는 current repo/workspace가 app-server/TUI flow를 실행할 수 있는 상태인지 나타냅니다.
    pub workspace_ok: bool,
    // 학습 주석: workspace_path는 검사된 workspace root입니다. startup UI와 session loading이 같은 root를 참조합니다.
    pub workspace_path: String,
    // 학습 주석: workspace_detail은 git/worktree/repo detection 결과를 설명하는 diagnostic text입니다.
    pub workspace_detail: String,
    // 학습 주석: attachment_profile은 terminal bridge 연결 방식입니다. startup summary가 inline/attached terminal
    // expectations를 사용자에게 설명할 때 사용합니다.
    pub attachment_profile: TerminalBridgeAttachmentProfile,
    // 학습 주석: initialize_ok는 app-server initialize handshake가 성공했는지 나타냅니다.
    pub initialize_ok: bool,
    // 학습 주석: initialize_detail은 initialize response/protocol mismatch/failure reason을 담는 문구입니다.
    pub initialize_detail: String,
    // 학습 주석: account_ok는 Codex account/auth readiness를 나타냅니다. prompt submission을 허용할지 판단하는
    // 필수 gate 중 하나입니다.
    pub account_ok: bool,
    // 학습 주석: account_detail은 로그인 상태, account probe failure, 권한 문제를 사용자에게 설명합니다.
    pub account_detail: String,
    // 학습 주석: warnings는 fatal gate는 아니지만 startup overlay에 노출해야 하는 degraded 상태들입니다.
    pub warnings: Vec<String>,
    // 학습 주석: schema_snapshot은 binary에 포함된 app-server protocol schema snapshot label입니다.
    pub schema_snapshot: String,
}

// 학습 주석: StartupDiagnostics methods는 startup snapshot 자체에 속한 작은 derived contract입니다.
impl StartupDiagnostics {
    // 학습 주석: 이 label은 startup service가 diagnostics를 만들 때 schema_snapshot field에 넣습니다. path와
    // byte length를 함께 표시해 어떤 embedded schema가 들어갔는지 build artifact만 보고도 확인할 수 있습니다.
    pub fn bundled_schema_snapshot_label() -> String {
        format!(
            "embedded {BUNDLED_SCHEMA_SNAPSHOT_PATH} ({} bytes)",
            BUNDLED_SCHEMA_SNAPSHOT_CONTENTS.len()
        )
    }

    // 학습 주석: can_continue는 startup 결과를 TUI 실행 허용 여부로 접는 domain predicate입니다. warning은
    // non-fatal로 남기고, codex binary/workspace/initialize/account 네 필수 gate만 모두 true여야 합니다.
    pub fn can_continue(&self) -> bool {
        self.codex_binary_ok && self.workspace_ok && self.initialize_ok && self.account_ok
    }
}
