use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::application::port::inbound::mcp_hub_port::McpHubPort;
use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::domain::mcp_hub::{
    McpHubServerSpec, McpHubServerSummary, McpHubToolDetail, McpHubToolSummary,
    summarized_description, tool_matches_query, validated_server_name,
};

/*
 * MCP hub service 는 등록부 확인, 검색 필터, 결과 축약을 맡는다.
 * 연결과 로그인은 backend 가 한다. 등록되지 않은 서버 이름은 연결 전에 여기서 거절한다.
 */
pub struct McpHubService {
    backend: Arc<dyn McpHubBackendPort>,
}

impl McpHubService {
    pub fn new(backend: Arc<dyn McpHubBackendPort>) -> Self {
        Self { backend }
    }

    fn require_registered(&self, name: &str) -> Result<String> {
        let name = validated_server_name(name.to_string()).map_err(anyhow::Error::msg)?;
        if self
            .backend
            .registered_servers()?
            .iter()
            .any(|spec| spec.name == name)
        {
            return Ok(name);
        }
        bail!("MCP server `{name}` is not registered. Run `akra mcp servers` to list servers")
    }
}

impl McpHubPort for McpHubService {
    fn servers(&self) -> Result<Vec<McpHubServerSummary>> {
        let specs = self.backend.registered_servers()?;
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        let statuses = self.backend.server_statuses(None)?;
        Ok(specs
            .into_iter()
            .map(|spec| {
                let status = statuses.iter().find(|status| status.name == spec.name);
                McpHubServerSummary {
                    target: spec.target_label(),
                    auth: status.map_or("unknown".into(), |status| status.auth.clone()),
                    connection: status.map_or("unknown".into(), |status| status.connection.clone()),
                    tool_count: status.map_or(0, |status| status.tools.len()),
                    error: status.and_then(|status| status.error.clone()),
                    name: spec.name,
                }
            })
            .collect())
    }

    fn add_server(&self, spec: McpHubServerSpec) -> Result<()> {
        self.backend.register_server(&spec)
    }

    fn remove_server(&self, name: &str) -> Result<()> {
        let name = validated_server_name(name.to_string()).map_err(anyhow::Error::msg)?;
        if !self.backend.unregister_server(&name)? {
            bail!("MCP server `{name}` is not registered")
        }
        Ok(())
    }

    fn login(&self, name: &str, on_authorization_url: &mut dyn FnMut(&str)) -> Result<()> {
        let name = self.require_registered(name)?;
        self.backend.login(&name, on_authorization_url)
    }

    fn search(&self, query: &str) -> Result<Vec<McpHubToolSummary>> {
        if self.backend.registered_servers()?.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .backend
            .server_statuses(None)?
            .into_iter()
            .flat_map(|status| {
                let server = status.name;
                status
                    .tools
                    .into_iter()
                    .filter(|tool| tool_matches_query(&server, tool, query))
                    .map(|tool| McpHubToolSummary {
                        server: server.clone(),
                        description: summarized_description(&tool.description),
                        tool: tool.name,
                    })
                    .collect::<Vec<_>>()
            })
            .collect())
    }

    fn describe(&self, server: &str, tool: &str) -> Result<McpHubToolDetail> {
        let server = self.require_registered(server)?;
        let status = self
            .backend
            .server_statuses(Some(&server))?
            .into_iter()
            .find(|status| status.name == server);
        let Some(status) = status else {
            bail!("MCP server `{server}` returned no status")
        };
        let Some(found) = status
            .tools
            .into_iter()
            .find(|candidate| candidate.name == tool)
        else {
            match status.error {
                Some(error) => bail!("MCP server `{server}` has no tool `{tool}`: {error}"),
                None => bail!(
                    "MCP server `{server}` has no tool `{tool}`. Run `akra mcp search` to find tools"
                ),
            }
        };
        Ok(McpHubToolDetail {
            server,
            tool: found.name,
            description: found.description,
            input_schema: found.input_schema,
        })
    }

    fn call(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
        let server = self.require_registered(server)?;
        if !arguments.is_object() {
            bail!("MCP tool arguments must be a JSON object")
        }
        self.backend.call_tool(&server, tool, arguments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mcp_hub::{McpHubServerStatus, McpHubToolInfo};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeBackend {
        specs: Mutex<Vec<McpHubServerSpec>>,
        statuses: Vec<McpHubServerStatus>,
        status_requests: Mutex<Vec<Option<String>>>,
        calls: Mutex<Vec<(String, String, Value)>>,
    }

    impl McpHubBackendPort for FakeBackend {
        fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
            Ok(self.specs.lock().unwrap().clone())
        }
        fn register_server(&self, spec: &McpHubServerSpec) -> Result<()> {
            self.specs.lock().unwrap().push(spec.clone());
            Ok(())
        }
        fn unregister_server(&self, name: &str) -> Result<bool> {
            let mut specs = self.specs.lock().unwrap();
            let before = specs.len();
            specs.retain(|spec| spec.name != name);
            Ok(specs.len() != before)
        }
        fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
            self.status_requests
                .lock()
                .unwrap()
                .push(only.map(str::to_string));
            Ok(self.statuses.clone())
        }
        fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((server.into(), tool.into(), arguments));
            Ok(json!({ "content": [] }))
        }
        fn login(&self, _server: &str, on_url: &mut dyn FnMut(&str)) -> Result<()> {
            on_url("https://auth.example/authorize");
            Ok(())
        }
    }

    fn notion_backend() -> Arc<FakeBackend> {
        Arc::new(FakeBackend {
            specs: Mutex::new(vec![
                McpHubServerSpec::http("notion", "https://mcp.notion.com/mcp").unwrap(),
            ]),
            statuses: vec![McpHubServerStatus {
                name: "notion".into(),
                auth: "oAuth".into(),
                connection: "connected".into(),
                tools: vec![
                    McpHubToolInfo {
                        name: "search".into(),
                        description: "Searches Notion only.\nLong details".into(),
                        input_schema: json!({ "type": "object" }),
                    },
                    McpHubToolInfo {
                        name: "notion-create-pages".into(),
                        description: "Creates pages".into(),
                        input_schema: json!({ "type": "object", "required": ["pages"] }),
                    },
                ],
                error: None,
            }],
            ..FakeBackend::default()
        })
    }

    #[test]
    fn servers_join_the_registry_with_live_status() {
        let service = McpHubService::new(notion_backend());
        let servers = service.servers().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].target, "https://mcp.notion.com/mcp");
        assert_eq!(servers[0].auth, "oAuth");
        assert_eq!(servers[0].tool_count, 2);
    }

    #[test]
    fn empty_registry_never_starts_the_backend() {
        let backend = Arc::new(FakeBackend::default());
        let service = McpHubService::new(backend.clone());
        assert!(service.servers().unwrap().is_empty());
        assert!(service.search("x").unwrap().is_empty());
        assert!(backend.status_requests.lock().unwrap().is_empty());
    }

    #[test]
    fn search_filters_and_shortens_descriptions() {
        let service = McpHubService::new(notion_backend());
        let found = service.search("search").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tool, "search");
        assert_eq!(found[0].description, "Searches Notion only.");
        assert_eq!(service.search("").unwrap().len(), 2);
    }

    #[test]
    fn describe_connects_only_the_named_server() {
        let backend = notion_backend();
        let service = McpHubService::new(backend.clone());
        let detail = service.describe("notion", "notion-create-pages").unwrap();
        assert_eq!(detail.input_schema["required"][0], "pages");
        assert_eq!(
            *backend.status_requests.lock().unwrap(),
            vec![Some("notion".to_string())]
        );
        assert!(service.describe("notion", "missing").is_err());
    }

    #[test]
    fn call_rejects_unregistered_servers_and_non_object_arguments() {
        let backend = notion_backend();
        let service = McpHubService::new(backend.clone());
        assert!(service.call("jira", "search", json!({})).is_err());
        assert!(service.call("notion", "search", json!(["x"])).is_err());
        service
            .call("notion", "search", json!({ "query": "x" }))
            .unwrap();
        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "notion");
        assert_eq!(calls[0].2["query"], "x");
    }

    #[test]
    fn remove_reports_unknown_names() {
        let service = McpHubService::new(notion_backend());
        assert!(service.remove_server("jira").is_err());
        service.remove_server("notion").unwrap();
        assert!(service.servers().unwrap().is_empty());
    }

    #[test]
    fn login_requires_a_registered_server() {
        let service = McpHubService::new(notion_backend());
        let mut seen = Vec::new();
        service
            .login("notion", &mut |url| seen.push(url.to_string()))
            .unwrap();
        assert_eq!(seen, vec!["https://auth.example/authorize".to_string()]);
        assert!(service.login("jira", &mut |_| {}).is_err());
    }
}
