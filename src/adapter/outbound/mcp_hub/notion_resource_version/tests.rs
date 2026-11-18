use std::sync::Mutex;

use super::*;
use crate::domain::mcp_hub::{McpHubServerSpec, McpHubServerStatus, McpHubToolInfo};

const PAGE_ID: &str = "0123456789abcdef0123456789abcdef";

fn page(key: &str) -> ExternalResourceRef {
    ExternalResourceRef {
        kind: "notion".into(),
        key: key.into(),
        url: format!("https://www.notion.so/{key}"),
        title_hint: None,
    }
}

struct Backend {
    registered: bool,
    auth: String,
    tool: String,
    result: Value,
    statuses: Mutex<Vec<Option<String>>>,
    calls: Mutex<Vec<(String, String, Value)>>,
}

impl Backend {
    fn ready(result: Value) -> Self {
        Self {
            registered: true,
            auth: "oAuth".into(),
            tool: "notion-fetch".into(),
            result,
            statuses: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl McpHubBackendPort for Backend {
    fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
        Ok(if self.registered {
            vec![McpHubServerSpec::http(SERVER, "https://mcp.notion.com/mcp").unwrap()]
        } else {
            vec![]
        })
    }
    fn register_server(&self, _: &McpHubServerSpec) -> Result<()> {
        unreachable!()
    }
    fn unregister_server(&self, _: &str) -> Result<bool> {
        unreachable!()
    }
    fn login(&self, _: &str, _: &mut dyn FnMut(&str)) -> Result<()> {
        unreachable!()
    }
    fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
        self.statuses.lock().unwrap().push(only.map(str::to_owned));
        Ok(vec![McpHubServerStatus {
            name: SERVER.into(),
            auth: self.auth.clone(),
            connection: "connected".into(),
            tools: vec![McpHubToolInfo {
                name: self.tool.clone(),
                description: String::new(),
                input_schema: json!({}),
            }],
            error: None,
        }])
    }
    fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((server.into(), tool.into(), arguments));
        Ok(self.result.clone())
    }
}

#[test]
fn fetches_only_the_requested_page_through_the_registered_notion_server() {
    let backend = Arc::new(Backend::ready(json!({
        "structuredContent": {"page_last_edited_at":"2026-11-17T10:00:00+09:00"}
    })));
    let adapter = NotionResourceVersionAdapter::new(backend.clone());
    assert_eq!(
        adapter.current_version(&page(PAGE_ID)).unwrap(),
        "2026-11-17T01:00:00Z"
    );
    assert_eq!(*backend.statuses.lock().unwrap(), [Some(SERVER.into())]);
    assert_eq!(
        *backend.calls.lock().unwrap(),
        [(SERVER.into(), "notion-fetch".into(), json!({"id":PAGE_ID}))]
    );
}

#[test]
fn connector_fetch_alias_and_text_payload_are_supported() {
    let mut backend = Backend::ready(json!({"content":[
        {"type":"image","data":"ignored"},
        {"type":"text","text":"{\"page_last_edited_at\":\"2026-11-17T01:00:00.000Z\"}"}
    ]}));
    backend.tool = "fetch".into();
    let backend = Arc::new(backend);
    let adapter = NotionResourceVersionAdapter::new(backend.clone());
    assert_eq!(
        adapter.current_version(&page(PAGE_ID)).unwrap(),
        "2026-11-17T01:00:00Z"
    );
    assert_eq!(backend.calls.lock().unwrap()[0].1, "fetch");
}

#[test]
fn invalid_ids_are_rejected_before_discovery() {
    let backend = Arc::new(Backend::ready(json!({})));
    let adapter = NotionResourceVersionAdapter::new(backend.clone());
    for key in [
        "",
        "memory",
        "https://other.example",
        "0123456789abcdef0123456789abcdeg",
    ] {
        assert!(adapter.current_version(&page(key)).is_err());
    }
    assert!(backend.statuses.lock().unwrap().is_empty());
    assert!(backend.calls.lock().unwrap().is_empty());
}

#[test]
fn an_unregistered_server_is_not_contacted() {
    let mut backend = Backend::ready(json!({}));
    backend.registered = false;
    let backend = Arc::new(backend);
    assert!(
        NotionResourceVersionAdapter::new(backend.clone())
            .current_version(&page(PAGE_ID))
            .is_err()
    );
    assert!(backend.statuses.lock().unwrap().is_empty());
    assert!(backend.calls.lock().unwrap().is_empty());
}

#[test]
fn unavailable_login_or_missing_read_tool_does_not_call_a_tool() {
    for (auth, tool) in [
        ("notLoggedIn", "notion-fetch"),
        ("oAuth", "notion-update-page"),
    ] {
        let mut backend = Backend::ready(json!({}));
        backend.auth = auth.into();
        backend.tool = tool.into();
        let backend = Arc::new(backend);
        assert!(
            NotionResourceVersionAdapter::new(backend.clone())
                .current_version(&page(PAGE_ID))
                .is_err()
        );
        assert!(backend.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn errors_and_unknown_payloads_never_become_versions() {
    for result in [
        json!({"isError":true,"structuredContent":{"page_last_edited_at":"2026-11-17T00:00:00Z"}}),
        json!({"structuredContent":{"page_last_edited_at":"2026-02-30T00:00:00Z"}}),
        json!({"structuredContent":{"page_last_edited_at":null}}),
        json!({"structuredContent":{"text":"private page"}}),
        json!({"content":[{"type":"text","text":"not JSON"}]}),
        json!({"content":[]}),
    ] {
        assert!(version_from_result(result).is_err());
    }
}

#[test]
fn null_structured_payload_uses_text_and_edit_instants_compare_canonically() {
    let first = version_from_result(json!({
        "structuredContent":null,
        "content":[{"type":"text","text":"{\"page_last_edited_at\":\"2026-11-17T10:00:00+09:00\"}"}]
    }))
    .unwrap();
    let same = version_from_result(
        json!({"structuredContent":{"page_last_edited_at":"2026-11-17T01:00:00.000Z"}}),
    )
    .unwrap();
    let changed = version_from_result(
        json!({"structuredContent":{"page_last_edited_at":"2026-11-17T01:00:01Z"}}),
    )
    .unwrap();
    assert_eq!(first, same);
    assert_ne!(first, changed);
}

#[test]
fn nullable_error_flag_preserves_a_successful_version() {
    for error_flag in [Value::Null, json!(false)] {
        let version = version_from_result(json!({
            "isError":error_flag,
            "content":[],
            "structuredContent":{"page_last_edited_at":"2026-11-17T01:00:00Z"}
        }))
        .unwrap();
        assert_eq!(version, "2026-11-17T01:00:00Z");
    }
}
