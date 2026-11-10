//! MCP hub backend 이다. 사용자당 하나인 hub 폴더(`$AKRA_HOME/mcp-hub`)를 가진다.
//!
//! - 등록부: hub 폴더의 `config.toml` 에 있는 `[mcp_servers.<name>]` 이다. Codex 설정 형식 그대로라서
//!   `env`, `bearer_token_env_var` 같은 Codex 옵션을 손으로 더 적어도 된다.
//! - 연결: 명령마다 hub 전용 `codex app-server`(`CODEX_HOME` = hub 폴더)를 띄운다. OAuth, 토큰 보관과
//!   갱신, stdio/HTTP 전송은 Codex 가 한다. 그래서 로그인 한 번을 모든 세션과 Akra 가 같이 쓴다.
//! - 호출: 대상 서버만 켠 ephemeral 스레드를 열고 `mcpServer/tool/call` 을 부른다. 기록은 남지 않는다.
//! - skill: 서버가 하나라도 있으면 `agent-skills/akra-mcp-hub/SKILL.md` 를 쓰고, app-server 연결이 그 폴더를
//!   skill root 로 등록한다. 세션은 skill 설명 한 줄만 싣고, 필요할 때 `akra mcp` 를 쓴다.

mod session;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use toml_edit::{Array, DocumentMut, Item, Table, value};

use self::session::HubAppServerSession;
use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::domain::mcp_hub::{
    McpHubServerSpec, McpHubServerStatus, McpHubToolInfo, McpHubTransport,
};

const HUB_DIRECTORY: &str = "mcp-hub";
const CONFIG_FILE: &str = "config.toml";
// Codex 는 CODEX_HOME/skills 에 자체 시스템 skill 을 설치한다. 그 폴더와 섞이지 않게 따로 둔다.
const SKILLS_DIRECTORY: &str = "agent-skills";
const SKILL_NAME: &str = "akra-mcp-hub";
const SKILL_TEXT: &str =
    include_str!("../../../../assets/app-server/hub-skills/akra-mcp-hub/SKILL.md");
// thread/start 와 상태 조회는 MCP 서버의 handshake 를 기다린다. 원격 서버의 첫 연결을 고려한 값이다.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const STATUS_PAGE_LIMIT: u64 = 100;

pub(crate) struct CodexMcpHubBackend {
    home: PathBuf,
}

impl CodexMcpHubBackend {
    pub(crate) fn from_environment() -> Result<Self> {
        Ok(Self { home: hub_home()? })
    }

    fn with_home(home: PathBuf) -> Self {
        Self { home }
    }

    fn config_path(&self) -> PathBuf {
        self.home.join(CONFIG_FILE)
    }

    fn read_document(&self) -> Result<DocumentMut> {
        let path = self.config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => text
                .parse::<DocumentMut>()
                .with_context(|| format!("failed to parse `{}`", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to read `{}`", path.display()))
            }
        }
    }

    fn write_document(&self, document: &DocumentMut) -> Result<()> {
        crate::private_directory::ensure_private_directory(&self.home)?;
        let path = self.config_path();
        std::fs::write(&path, document.to_string())
            .with_context(|| format!("failed to write `{}`", path.display()))
    }

    fn open_session(&self) -> Result<HubAppServerSession> {
        crate::private_directory::ensure_private_directory(&self.home)?;
        let codex = crate::trusted_executable::pinned_codex_command()
            .context("the MCP hub needs the `codex` CLI")?;
        HubAppServerSession::open(&codex, &self.home)
    }

    /// 대상 서버만 켠 ephemeral 스레드를 연다. 다른 서버는 접속하지 않는다.
    fn start_thread_for(&self, session: &mut HubAppServerSession, server: &str) -> Result<String> {
        let mut overrides = Map::new();
        for spec in self.registered_servers()? {
            if spec.name != server {
                overrides.insert(format!("mcp_servers.{}.enabled", spec.name), json!(false));
            }
        }
        let response = session.request(
            "thread/start",
            json!({
                "ephemeral": true,
                "cwd": self.home,
                "config": Value::Object(overrides),
            }),
            REQUEST_TIMEOUT,
        )?;
        response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("thread/start returned no thread id")
    }
}

impl McpHubBackendPort for CodexMcpHubBackend {
    fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
        Ok(registered_servers_in(&self.read_document()?))
    }

    fn register_server(&self, spec: &McpHubServerSpec) -> Result<()> {
        let mut document = self.read_document()?;
        let servers = document
            .entry("mcp_servers")
            .or_insert_with(|| {
                let mut table = Table::new();
                table.set_implicit(true);
                Item::Table(table)
            })
            .as_table_mut()
            .context("`mcp_servers` in the MCP hub config is not a table")?;
        servers.insert(&spec.name, Item::Table(server_table(spec)));
        self.write_document(&document)?;
        write_agent_skill(&self.home)
    }

    fn unregister_server(&self, name: &str) -> Result<bool> {
        let mut document = self.read_document()?;
        let removed = document
            .get_mut("mcp_servers")
            .and_then(Item::as_table_mut)
            .and_then(|servers| servers.remove(name))
            .is_some();
        if removed {
            self.write_document(&document)?;
        }
        Ok(removed)
    }

    fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
        let mut session = self.open_session()?;
        let mut statuses = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = json!({ "detail": "full", "limit": STATUS_PAGE_LIMIT });
            if let Some(name) = only {
                params["serverName"] = json!(name);
            }
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let page = session.request("mcpServerStatus/list", params, REQUEST_TIMEOUT)?;
            if let Some(entries) = page.get("data").and_then(Value::as_array) {
                statuses.extend(entries.iter().map(server_status_from_wire));
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(statuses)
    }

    fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
        let mut session = self.open_session()?;
        let thread_id = self.start_thread_for(&mut session, server)?;
        session.request(
            "mcpServer/tool/call",
            json!({ "threadId": thread_id, "server": server, "tool": tool, "arguments": arguments }),
            REQUEST_TIMEOUT,
        )
    }

    fn login(&self, server: &str, on_authorization_url: &mut dyn FnMut(&str)) -> Result<()> {
        let mut session = self.open_session()?;
        let response = session.request(
            "mcpServer/oauth/login",
            json!({ "name": server }),
            REQUEST_TIMEOUT,
        )?;
        let url = response
            .get("authorizationUrl")
            .and_then(Value::as_str)
            .context("mcpServer/oauth/login returned no authorization URL")?;
        on_authorization_url(url);
        let completed = session.wait_for_notification(
            "mcpServer/oauthLogin/completed",
            LOGIN_TIMEOUT,
            |params| params.get("name").and_then(Value::as_str) == Some(server),
        )?;
        if completed.get("success").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        let detail = completed
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the login did not finish");
        bail!("MCP server `{server}` login failed: {detail}")
    }
}

/// app-server 연결이 등록할 skill root 이다. hub 에 서버가 없으면 비어 있어서 세션 컨텍스트가 늘지 않는다.
pub(crate) fn agent_skill_roots() -> Vec<PathBuf> {
    let Ok(home) = hub_home() else {
        return Vec::new();
    };
    agent_skill_roots_in(&home)
}

fn agent_skill_roots_in(home: &Path) -> Vec<PathBuf> {
    let backend = CodexMcpHubBackend::with_home(home.to_path_buf());
    let has_servers = backend
        .registered_servers()
        .is_ok_and(|servers| !servers.is_empty());
    if !has_servers || write_agent_skill(home).is_err() {
        return Vec::new();
    }
    vec![home.join(SKILLS_DIRECTORY)]
}

/// 바이너리에 든 skill 문서를 hub 폴더에 쓴다. 내용이 같으면 쓰지 않는다.
fn write_agent_skill(home: &Path) -> Result<()> {
    let directory = home.join(SKILLS_DIRECTORY).join(SKILL_NAME);
    let path = directory.join("SKILL.md");
    if std::fs::read_to_string(&path).is_ok_and(|current| current == SKILL_TEXT) {
        return Ok(());
    }
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create `{}`", directory.display()))?;
    std::fs::write(&path, SKILL_TEXT)
        .with_context(|| format!("failed to write `{}`", path.display()))
}

fn hub_home() -> Result<PathBuf> {
    Ok(crate::configuration::akra_home_dir()?.join(HUB_DIRECTORY))
}

fn registered_servers_in(document: &DocumentMut) -> Vec<McpHubServerSpec> {
    let Some(servers) = document.get("mcp_servers").and_then(Item::as_table_like) else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|(name, item)| {
            let table = item.as_table_like()?;
            if let Some(url) = table.get("url").and_then(Item::as_str) {
                return McpHubServerSpec::http(name, url).ok();
            }
            let command = table.get("command").and_then(Item::as_str)?;
            let args = table
                .get("args")
                .and_then(Item::as_array)
                .map(|args| {
                    args.iter()
                        .filter_map(|arg| arg.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            McpHubServerSpec::stdio(name, command, args).ok()
        })
        .collect()
}

fn server_table(spec: &McpHubServerSpec) -> Table {
    let mut table = Table::new();
    match &spec.transport {
        McpHubTransport::Http { url } => {
            table.insert("url", value(url.as_str()));
        }
        McpHubTransport::Stdio { command, args } => {
            table.insert("command", value(command.as_str()));
            if !args.is_empty() {
                table.insert("args", value(args.iter().collect::<Array>()));
            }
        }
    }
    table
}

fn server_status_from_wire(entry: &Value) -> McpHubServerStatus {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let tools = entry
        .get("tools")
        .and_then(Value::as_object)
        .map(|tools| {
            tools
                .values()
                .filter_map(|tool| {
                    Some(McpHubToolInfo {
                        name: tool.get("name")?.as_str()?.to_string(),
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        input_schema: tool.get("inputSchema").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let connection = text("runtimeStatus").unwrap_or_else(|| {
        if text("toolsError").is_some() {
            "failed".into()
        } else {
            "reachable".into()
        }
    });
    McpHubServerStatus {
        name: text("name").unwrap_or_default(),
        auth: text("authStatus").unwrap_or_else(|| "unknown".into()),
        connection,
        tools,
        error: text("toolsError"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(label: &str) -> PathBuf {
        let home = crate::test_utils::platform_safe_temp_dir()
            .join(format!("akra-mcp-hub-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn registry_round_trips_codex_server_tables_and_keeps_extra_keys() {
        let home = temp_home("registry");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(CONFIG_FILE),
            "[mcp_servers.notion]\nurl = \"https://mcp.notion.com/mcp\"\nstartup_timeout_sec = 30\n",
        )
        .unwrap();
        let backend = CodexMcpHubBackend::with_home(home.clone());
        backend
            .register_server(
                &McpHubServerSpec::stdio("local", "node", vec!["server.js".into()]).unwrap(),
            )
            .unwrap();

        let specs = backend.registered_servers().unwrap();
        assert_eq!(specs.len(), 2);
        assert!(
            specs
                .contains(&McpHubServerSpec::http("notion", "https://mcp.notion.com/mcp").unwrap())
        );
        assert!(specs.contains(
            &McpHubServerSpec::stdio("local", "node", vec!["server.js".into()]).unwrap()
        ));
        let text = std::fs::read_to_string(home.join(CONFIG_FILE)).unwrap();
        assert!(text.contains("startup_timeout_sec = 30"), "{text}");

        assert!(backend.unregister_server("local").unwrap());
        assert!(!backend.unregister_server("local").unwrap());
        assert_eq!(backend.registered_servers().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn skill_root_appears_only_when_servers_exist() {
        let home = temp_home("skill");
        assert!(agent_skill_roots_in(&home).is_empty());
        assert!(!home.join(SKILLS_DIRECTORY).exists());

        CodexMcpHubBackend::with_home(home.clone())
            .register_server(
                &McpHubServerSpec::http("notion", "https://mcp.notion.com/mcp").unwrap(),
            )
            .unwrap();
        assert_eq!(
            agent_skill_roots_in(&home),
            vec![home.join(SKILLS_DIRECTORY)]
        );
        let skill = std::fs::read_to_string(
            home.join(SKILLS_DIRECTORY)
                .join(SKILL_NAME)
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(skill.starts_with("---\nname: akra-mcp-hub\n"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn status_mapping_reads_tools_and_falls_back_without_runtime_status() {
        let status = server_status_from_wire(&json!({
            "name": "notion",
            "authStatus": "oAuth",
            "runtimeStatus": null,
            "tools": { "search": { "name": "search", "description": "Find", "inputSchema": { "type": "object" } } },
            "toolsError": null
        }));
        assert_eq!(status.connection, "reachable");
        assert_eq!(status.tools[0].name, "search");
        assert_eq!(status.tools[0].input_schema["type"], "object");

        let failed =
            server_status_from_wire(&json!({ "name": "x", "toolsError": "auth required" }));
        assert_eq!(failed.connection, "failed");
        assert_eq!(failed.auth, "unknown");
        assert_eq!(failed.error.as_deref(), Some("auth required"));
    }

    #[test]
    fn skill_text_documents_the_cli_contract() {
        for command in [
            "akra mcp search",
            "akra mcp describe",
            "akra mcp call",
            "akra mcp login",
        ] {
            assert!(
                SKILL_TEXT.contains(command),
                "skill should mention `{command}`"
            );
        }
    }
}
