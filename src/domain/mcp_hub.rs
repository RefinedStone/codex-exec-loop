/*
 * MCP hub 의 값 타입이다. hub 는 사용자당 하나인 MCP 서버 등록부와 그 서버들의 로그인을 가진다.
 * 세션은 서버 도구 스키마를 미리 싣지 않고, 필요할 때 `akra mcp` 로 검색하고 호출한다.
 * 이 모듈은 I/O 없이 이름 규칙과 결과 축약만 정한다.
 */
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 서버 이름은 Codex `config.toml` 의 `mcp_servers.<name>` 키와 CLI 인자로 그대로 쓰인다.
pub const MCP_HUB_SERVER_NAME_MAX_CHARS: usize = 64;
/// 검색 결과의 설명은 첫 줄만, 이 길이까지만 보여 준다. 전체 설명은 describe 가 준다.
pub const MCP_HUB_TOOL_SUMMARY_MAX_CHARS: usize = 160;

/*
 * Gateway 상주 엔진과 `akra mcp` 클라이언트 사이의 JSON-RPC 계약이다. 모든 요청 params 에 `hubHome` 을 싣는다.
 * Windows 의 Gateway pipe 이름은 사용자에 묶이므로 다른 `AKRA_HOME` 의 Gateway 가 받을 수 있다. 경로가 다르면
 * Gateway 는 `MCP_HUB_GATEWAY_OTHER_HUB` 로 답하고, 클라이언트는 직접 실행한다.
 */
pub const MCP_HUB_GATEWAY_METHOD_PREFIX: &str = "akra/mcp/";
/// params `{ hubHome, server? }` -> result `{ statuses: [McpHubServerStatus] }`
pub const MCP_HUB_GATEWAY_STATUSES: &str = "akra/mcp/statuses";
/// params `{ hubHome, server, tool, arguments }` -> result 는 MCP `tools/call` 결과이다.
pub const MCP_HUB_GATEWAY_CALL: &str = "akra/mcp/call";
/// params `{ hubHome }` -> result `{}`. 등록부나 로그인이 바뀐 뒤 상주 엔진을 내린다.
pub const MCP_HUB_GATEWAY_RELOAD: &str = "akra/mcp/reload";
pub const MCP_HUB_GATEWAY_OTHER_HUB: i64 = -32040;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpHubTransport {
    Http { url: String },
    Stdio { command: String, args: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHubServerSpec {
    pub name: String,
    pub transport: McpHubTransport,
}

impl McpHubServerSpec {
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Result<Self, String> {
        let name = validated_server_name(name.into())?;
        let url = url.into().trim().to_string();
        if !(url.starts_with("https://") || is_loopback_http_url(&url)) {
            return Err(format!(
                "MCP server `{name}` URL must use https (http is allowed only for localhost)"
            ));
        }
        Ok(Self {
            name,
            transport: McpHubTransport::Http { url },
        })
    }

    pub fn stdio(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
    ) -> Result<Self, String> {
        let name = validated_server_name(name.into())?;
        let command = command.into().trim().to_string();
        if command.is_empty() {
            return Err(format!("MCP server `{name}` needs a command"));
        }
        Ok(Self {
            name,
            transport: McpHubTransport::Stdio { command, args },
        })
    }

    /// 사람이 읽는 연결 대상이다. stdio 는 명령줄, http 는 URL 이다.
    pub fn target_label(&self) -> String {
        match &self.transport {
            McpHubTransport::Http { url } => url.clone(),
            McpHubTransport::Stdio { command, args } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

pub fn validated_server_name(name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    let valid_chars = name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-');
    if name.is_empty() || name.chars().count() > MCP_HUB_SERVER_NAME_MAX_CHARS || !valid_chars {
        return Err(format!(
            "MCP server name `{name}` must be 1-{MCP_HUB_SERVER_NAME_MAX_CHARS} characters of a-z, A-Z, 0-9, `_` or `-`"
        ));
    }
    Ok(name)
}

fn is_loopback_http_url(url: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|prefix| {
            url.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        })
}

/// backend 가 알려 준 서버 하나의 상태와 도구 목록이다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpHubServerStatus {
    pub name: String,
    pub auth: String,
    pub connection: String,
    pub tools: Vec<McpHubToolInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpHubToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpHubServerSummary {
    pub name: String,
    pub target: String,
    pub auth: String,
    pub connection: String,
    pub tool_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 사용자의 Codex 홈에 둔 hub skill 에 할 일이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpHubSharedSkillAction {
    Status,
    Install,
    Remove,
}

/// 사용자의 Codex 홈에 둔 hub skill 의 상태이다. `path` 는 설치되지 않았어도 설치될 자리를 알린다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpHubSharedSkill {
    pub installed: bool,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpHubToolSummary {
    pub server: String,
    pub tool: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct McpHubToolDetail {
    pub server: String,
    pub tool: String,
    pub description: String,
    pub input_schema: Value,
}

/// 검색어의 모든 단어가 서버 이름, 도구 이름, 설명 중 어디엔가 있으면 맞는 것으로 본다. 빈 검색어는 전부 맞다.
pub fn tool_matches_query(server: &str, tool: &McpHubToolInfo, query: &str) -> bool {
    let haystack = format!("{server} {} {}", tool.name, tool.description).to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

/// 설명의 첫 줄을 글자 수 제한까지 자른다. 도구가 수십 개여도 검색 결과가 짧게 유지된다.
pub fn summarized_description(description: &str) -> String {
    let first_line = description
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if first_line.chars().count() <= MCP_HUB_TOOL_SUMMARY_MAX_CHARS {
        return first_line.to_string();
    }
    let mut summary = first_line
        .chars()
        .take(MCP_HUB_TOOL_SUMMARY_MAX_CHARS - 1)
        .collect::<String>();
    summary.push('…');
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn server_names_follow_config_key_rules() {
        assert!(McpHubServerSpec::http("notion", "https://mcp.notion.com/mcp").is_ok());
        assert!(McpHubServerSpec::http("my-server_2", "https://example.com").is_ok());
        assert!(McpHubServerSpec::http("bad.name", "https://example.com").is_err());
        assert!(McpHubServerSpec::http("", "https://example.com").is_err());
        assert!(McpHubServerSpec::http("a".repeat(65), "https://example.com").is_err());
    }

    #[test]
    fn http_urls_must_be_https_or_loopback() {
        assert!(McpHubServerSpec::http("a", "http://example.com/mcp").is_err());
        assert!(McpHubServerSpec::http("a", "http://localhost:8080/mcp").is_ok());
        assert!(McpHubServerSpec::http("a", "http://127.0.0.1/mcp").is_ok());
        assert!(McpHubServerSpec::http("a", "http://localhost.evil.com/mcp").is_err());
    }

    #[test]
    fn stdio_needs_a_command_and_labels_the_command_line() {
        assert!(McpHubServerSpec::stdio("a", " ", vec![]).is_err());
        let spec = McpHubServerSpec::stdio("a", "node", vec!["server.js".into()]).unwrap();
        assert_eq!(spec.target_label(), "node server.js");
    }

    #[test]
    fn query_matches_every_word_across_server_tool_and_description() {
        let tool = McpHubToolInfo {
            name: "notion-create-pages".into(),
            description: "Creates one or more Notion pages".into(),
            input_schema: json!({}),
        };
        assert!(tool_matches_query("notion", &tool, ""));
        assert!(tool_matches_query("notion", &tool, "create PAGES"));
        assert!(!tool_matches_query("notion", &tool, "create database"));
    }

    #[test]
    fn descriptions_keep_the_first_line_within_the_limit() {
        assert_eq!(
            summarized_description("\n  First line.\nSecond"),
            "First line."
        );
        let long = "x".repeat(MCP_HUB_TOOL_SUMMARY_MAX_CHARS + 10);
        let summary = summarized_description(&long);
        assert_eq!(summary.chars().count(), MCP_HUB_TOOL_SUMMARY_MAX_CHARS);
        assert!(summary.ends_with('…'));
    }
}
