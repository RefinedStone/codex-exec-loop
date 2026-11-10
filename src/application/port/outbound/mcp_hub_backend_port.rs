use anyhow::Result;
use serde_json::Value;

use crate::domain::mcp_hub::{McpHubServerSpec, McpHubServerStatus};

/*
 * MCP hub backend 는 서버 등록부와 실제 MCP 연결을 가진 바깥 경계다.
 * production 구현은 hub 전용 `codex app-server` 를 필요할 때만 띄워 Codex 의 MCP 클라이언트,
 * OAuth, 토큰 보관을 그대로 쓴다. service 는 이 계약만 보고 검색, 검증, 축약을 맡는다.
 */
pub trait McpHubBackendPort: Send + Sync {
    /// 등록부에 적힌 서버들이다. 연결하지 않는다.
    fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>>;

    /// 같은 이름이 있으면 덮어쓴다.
    fn register_server(&self, spec: &McpHubServerSpec) -> Result<()>;

    /// 지웠으면 true, 없던 이름이면 false 이다.
    fn unregister_server(&self, name: &str) -> Result<bool>;

    /// `only` 가 있으면 그 서버에만 연결해서 상태와 도구 목록을 읽는다. 없으면 켜진 서버 모두에 연결한다.
    fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>>;

    /// `server` 만 연결해서 도구를 한 번 부른다. 결과는 MCP `tools/call` 결과 그대로이다.
    fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value>;

    /// OAuth 로그인을 시작한다. 인증 주소를 알려 준 뒤 사용자가 끝낼 때까지 기다린다.
    fn login(&self, server: &str, on_authorization_url: &mut dyn FnMut(&str)) -> Result<()>;

    /// 유지 중인 연결을 내린다. 다음 요청이 등록부와 로그인을 다시 읽는다. 연결을 유지하지 않으면 할 일이 없다.
    fn reload(&self) -> Result<()> {
        Ok(())
    }

    /// 오래 쓰지 않은 연결을 내린다. 요청을 처리하는 중이면 기다리지 않고 그냥 돌아온다.
    fn release_if_idle(&self) {}
}
