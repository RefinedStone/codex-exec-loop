use anyhow::Result;
use serde_json::Value;

use crate::domain::mcp_hub::{
    McpHubServerSpec, McpHubServerSummary, McpHubSharedSkill, McpHubSharedSkillAction,
    McpHubToolDetail, McpHubToolSummary,
};

/*
 * MCP hub inbound 계약이다. `akra mcp` CLI 와 이후의 Desktop 패널이 같은 계약으로 들어온다.
 * 세션은 도구 스키마를 미리 싣지 않고 search -> describe -> call 순서로 필요한 도구만 쓴다.
 */
pub trait McpHubPort: Send + Sync {
    fn servers(&self) -> Result<Vec<McpHubServerSummary>>;
    fn add_server(&self, spec: McpHubServerSpec) -> Result<()>;
    fn remove_server(&self, name: &str) -> Result<()>;
    fn login(&self, name: &str, on_authorization_url: &mut dyn FnMut(&str)) -> Result<()>;
    fn search(&self, query: &str) -> Result<Vec<McpHubToolSummary>>;
    fn describe(&self, server: &str, tool: &str) -> Result<McpHubToolDetail>;
    fn call(&self, server: &str, tool: &str, arguments: Value) -> Result<Value>;
    /// hub skill 을 사용자의 Codex 홈에 두거나 지운다. Akra 가 열지 않은 Codex 세션도 hub 를 쓰게 된다.
    fn shared_skill(&self, action: McpHubSharedSkillAction) -> Result<McpHubSharedSkill>;
}
