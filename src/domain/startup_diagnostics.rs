// terminal bridge attachment profile은 runtime adapter가 관찰한 연결 형태를 startup 도메인에 싣는 값이다.
// startup overlay와 capability copy는 이 값으로 provider launch, reattach, future local attach를 같은
// 화면 계약에서 설명한다.
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

// bundled schema snapshot은 현재 binary가 기준으로 삼는 app-server protocol schema 파일이다.
// startup banner가 이 label을 보여 주면 실행 중인 binary가 어떤 schema snapshot으로 빌드됐는지
// build artifact만 보고 추적할 수 있다.
const BUNDLED_SCHEMA_SNAPSHOT_PATH: &str = "schema/codex_app_server_protocol.v2.schemas.json";
// `include_str!`은 schema snapshot 내용을 binary에 embed한다. runtime filesystem에 schema 파일이 없어도
// startup diagnostics는 빌드 시점 snapshot 크기와 출처를 표시할 수 있다.
const BUNDLED_SCHEMA_SNAPSHOT_CONTENTS: &str =
    include_str!("../../schema/codex_app_server_protocol.v2.schemas.json");

/*
 * `StartupDiagnostics`는 startup service가 수행한 readiness checks의 domain snapshot이다. application
 * service는 local checks(cwd, workspace, Codex binary)와 app-server startup context(initialize,
 * account, attachment profile)를 이 구조체로 합친다.
 *
 * TUI는 이 값을 startup overlay, prompt submit gating, recent session loading gate에 함께 사용한다.
 * 그래서 각 field는 단순한 표시 문자열이 아니라, "계속 진행해도 되는가"와 "사용자에게 어떤 degradation을
 * 설명해야 하는가"를 분리하는 경계 계약이다.
 */
#[derive(Debug, Clone)]
pub struct StartupDiagnostics {
    // startup checks를 실행한 process 기준 directory다. workspace root와 다를 수 있어 진단 화면에서 나눠 보여 준다.
    pub cwd: String,
    // official Codex binary/probe가 사용 가능하다는 readiness flag다.
    pub codex_binary_ok: bool,
    // binary path, version, probe failure를 사람이 읽을 수 있게 담는 진단 문구다.
    pub codex_binary_detail: String,
    // current repo/workspace가 app-server/TUI flow를 실행할 수 있는 상태인지 나타낸다.
    pub workspace_ok: bool,
    // 검사된 workspace root다. startup UI와 session loading은 이 경로를 같은 기준점으로 사용한다.
    pub workspace_path: String,
    // git/worktree/repo detection 결과를 설명하는 diagnostic text다.
    pub workspace_detail: String,
    // terminal bridge 연결 방식이다. startup summary는 이 값으로 inline/attached terminal expectation을 설명한다.
    pub attachment_profile: TerminalBridgeAttachmentProfile,
    // app-server initialize handshake가 성공했는지 나타낸다.
    pub initialize_ok: bool,
    // initialize response, protocol mismatch, failure reason을 담는 문구다.
    pub initialize_detail: String,
    // Codex account/auth readiness를 나타낸다. prompt submission을 허용할지 판단하는 필수 gate 중 하나다.
    pub account_ok: bool,
    // 로그인 상태, account probe failure, 권한 문제를 사용자에게 설명한다.
    pub account_detail: String,
    // fatal gate는 아니지만 startup overlay에 노출해야 하는 degraded 상태들이다.
    pub warnings: Vec<String>,
    // binary에 포함된 app-server protocol schema snapshot label이다.
    pub schema_snapshot: String,
}

// `StartupDiagnostics` methods는 startup snapshot 자체에 속한 작은 derived contract다.
impl StartupDiagnostics {
    /*
     * 이 label은 startup service가 diagnostics를 만들 때 `schema_snapshot` field에 넣는다. path와 byte
     * length를 함께 표시해 어떤 embedded schema가 들어갔는지 startup 화면과 로그만 보고도 확인할 수 있다.
     */
    pub fn bundled_schema_snapshot_label() -> String {
        format!(
            "embedded {BUNDLED_SCHEMA_SNAPSHOT_PATH} ({} bytes)",
            BUNDLED_SCHEMA_SNAPSHOT_CONTENTS.len()
        )
    }

    /*
     * `can_continue`는 startup 결과를 TUI 실행 허용 여부로 접는 domain predicate다. warning은 non-fatal로
     * 남기고, codex binary/workspace/initialize/account 네 필수 gate만 모두 true여야 한다.
     */
    pub fn can_continue(&self) -> bool {
        self.codex_binary_ok && self.workspace_ok && self.initialize_ok && self.account_ok
    }
}
