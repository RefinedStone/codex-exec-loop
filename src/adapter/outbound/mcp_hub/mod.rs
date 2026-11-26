//! MCP hub backend 이다. 사용자당 하나인 hub 폴더(`$AKRA_HOME/mcp-hub`)를 가진다.
//!
//! - 등록부: hub 폴더의 `config.toml` 에 있는 `[mcp_servers.<name>]` 이다. Codex 설정 형식 그대로라서
//!   `env`, `bearer_token_env_var` 같은 Codex 옵션을 손으로 더 적어도 된다.
//! - 연결: 명령마다 hub 전용 `codex app-server`(`CODEX_HOME` = hub 폴더)를 띄운다. OAuth, 토큰 보관과
//!   갱신, stdio/HTTP 전송은 Codex 가 한다. 그래서 로그인 한 번을 모든 세션과 Akra 가 같이 쓴다.
//! - 호출: 대상 서버만 켠 ephemeral 스레드를 열고 `mcpServer/tool/call` 을 부른다. 기록은 남지 않는다.
//! - skill: 서버가 하나라도 있으면 `agent-skills/akra-mcp-hub/SKILL.md` 를 쓰고, app-server 연결이 그 폴더를
//!   skill root 로 등록한다. 세션은 skill 설명 한 줄만 싣고, 필요할 때 `akra mcp` 를 쓴다.

mod catalog;
mod gateway_client;
mod notion_resource_version;
mod registry;
#[cfg(test)]
mod registry_tests;
mod session;
mod tool_cache;

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use toml_edit::{Array, DocumentMut, Item, Table, value};

pub(crate) use self::gateway_client::GatewayRoutedMcpHubBackend;
pub(crate) use self::notion_resource_version::NotionResourceVersionAdapter;
use self::session::HubAppServerSession;
use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
use crate::application::port::outbound::mcp_write_approval_port::{
    McpWriteApproval, McpWriteApprovalPort, McpWriteApprovalRequest,
};
use crate::domain::mcp_hub::{
    McpHubServerSpec, McpHubServerStatus, McpHubSharedSkill, McpHubSharedSkillAction,
    McpHubToolCatalog, McpHubToolInfo, McpHubTransport,
};
use crate::domain::mcp_write_policy::{MCP_WRITE_POLICY_FILE, McpToolAccess, McpWritePolicy};

const HUB_DIRECTORY: &str = "mcp-hub";
const CONFIG_FILE: &str = "config.toml";
// Never unlink this file: separate processes must always lock the same inode.
const CONFIG_LOCK_FILE: &str = ".config.toml.lock";
// Codex 는 CODEX_HOME/skills 에 자체 시스템 skill 을 설치한다. 그 폴더와 섞이지 않게 따로 둔다.
const SKILLS_DIRECTORY: &str = "agent-skills";
const SKILL_NAME: &str = "akra-mcp-hub";
const SKILL_TEXT: &str =
    include_str!("../../../../assets/app-server/hub-skills/akra-mcp-hub/SKILL.md");
// thread/start 와 상태 조회는 MCP 서버의 handshake 를 기다린다. 원격 서버의 첫 연결을 고려한 값이다.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const STATUS_PAGE_LIMIT: u64 = 100;

/// 상주 엔진이 이 시간 동안 쓰이지 않으면 Gateway 가 내린다. 커널의 한가한 시간과 같다.
const RESIDENT_IDLE_LIMIT: Duration = Duration::from_secs(600);

pub(crate) struct CodexMcpHubBackend {
    home: PathBuf,
    // None 이면 명령마다 app-server 를 띄우고 끝낸다. Some 이면 Gateway 처럼 app-server 를 유지한다.
    resident: Option<Mutex<Option<ResidentEngine>>>,
    sync_registry_directory: fn(&Path) -> std::io::Result<()>,
    /// 쓰기 도구를 물어볼 곳이다 (#2830). Gateway 만 Desktop 에 묻는다. 없으면 쓰기 도구를 실행하지 않는다.
    write_approver: Option<Arc<dyn McpWriteApprovalPort>>,
}

/// Gateway 가 유지하는 app-server 와 서버별 ephemeral 스레드이다. 스레드를 다시 쓰면 MCP 연결도 유지된다.
struct ResidentEngine {
    session: HubAppServerSession,
    threads: HashMap<String, String>,
    last_used: Instant,
    catalog_identity: Option<String>,
}

impl CodexMcpHubBackend {
    pub(crate) fn from_environment() -> Result<Self> {
        Ok(Self::with_home(hub_home()?))
    }

    /// Gateway 용이다. 첫 요청에 app-server 를 띄우고, reload 나 한가한 시간이 지날 때까지 유지한다.
    pub(crate) fn resident_from_environment() -> Result<Self> {
        Ok(Self {
            home: hub_home()?,
            resident: Some(Mutex::new(None)),
            sync_registry_directory: registry::sync_directory,
            write_approver: None,
        })
    }

    fn with_home(home: PathBuf) -> Self {
        Self {
            home,
            resident: None,
            sync_registry_directory: registry::sync_directory,
            write_approver: None,
        }
    }

    pub(crate) fn with_write_approver(mut self, approver: Arc<dyn McpWriteApprovalPort>) -> Self {
        self.write_approver = Some(approver);
        self
    }

    /// 쓰기 정책(`write-policy.toml`)을 보고, 쓰기 도구이면 승인을 받는다. 정책을 읽지 못하면 어떤 도구도 실행하지
    /// 않는다: 어느 도구가 쓰기인지 모르기 때문이다.
    fn authorize_call(&self, server: &str, tool: &str, arguments: &Value) -> Result<()> {
        let path = self.home.join(MCP_WRITE_POLICY_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "cannot read the MCP write policy `{}`; no tool call runs",
                        path.display()
                    )
                });
            }
        };
        let policy = McpWritePolicy::parse(text.as_deref()).with_context(|| {
            format!(
                "fix the MCP write policy `{}`; no tool call runs until it is valid",
                path.display()
            )
        })?;
        if policy.access(server, tool) == McpToolAccess::Read {
            return Ok(());
        }
        let Some(approver) = &self.write_approver else {
            bail!(
                "MCP tool `{server}/{tool}` writes; it runs only after approval in Akra Desktop through the Akra Gateway"
            )
        };
        let request = McpWriteApprovalRequest {
            server: server.to_string(),
            tool: tool.to_string(),
            arguments: arguments.clone(),
        };
        match approver.request_approval(&request) {
            McpWriteApproval::Approved => Ok(()),
            McpWriteApproval::Declined => {
                bail!("MCP tool `{server}/{tool}` was declined in Akra Desktop")
            }
            McpWriteApproval::TimedOut => bail!(
                "MCP tool `{server}/{tool}` got no answer in Akra Desktop in time; it did not run"
            ),
            McpWriteApproval::NoApprover => bail!(
                "MCP tool `{server}/{tool}` writes; open Akra Desktop on this Gateway to approve it"
            ),
        }
    }

    pub(crate) fn home(&self) -> &Path {
        &self.home
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

    /// Hold a process-wide filesystem lock for the entire read-modify-replace operation.
    /// An in-process mutex alone would lose updates from separate Akra/Gateway processes.
    fn lock_registry(&self) -> Result<File> {
        crate::private_directory::ensure_private_directory(&self.home)?;
        let path = self.home.join(CONFIG_LOCK_FILE);
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(&path)
            .with_context(|| format!("failed to open MCP registry lock `{}`", path.display()))?;
        lock.lock()
            .with_context(|| format!("failed to lock MCP registry `{}`", path.display()))?;
        Ok(lock)
    }

    fn update_document<T>(
        &self,
        update: impl FnOnce(&mut DocumentMut) -> Result<(T, bool)>,
    ) -> Result<T> {
        let _lock = self.lock_registry()?;
        // Read only after the lock is held, or two processes can lose one update.
        let mut document = self.read_document()?;
        let (result, changed) = update(&mut document)?;
        if changed {
            self.write_document(&document)?;
        }
        Ok(result)
    }

    /// A replaced registry remains installed even if directory sync fails.
    fn write_document(&self, document: &DocumentMut) -> Result<()> {
        registry::write_document(&self.home, document, self.sync_registry_directory)
    }

    fn finish_registry_change<T>(&self, mut result: Result<T>, refresh_skill: bool) -> Result<T> {
        let skill = if refresh_skill {
            write_agent_skill(&self.home).context("failed to refresh the MCP hub skill")
        } else {
            Ok(())
        };
        // Attempt every reconciliation step even if an earlier one fails. In
        // particular, a cache failure must not leave the resident on old config.
        let catalog = self
            .invalidate_tool_catalogs()
            .context("failed to invalidate the MCP tool catalog");
        let reload = self
            .reload()
            .context("failed to reload the MCP hub resident");
        for follow_up in [skill, catalog, reload] {
            if let Err(error) = follow_up {
                result = Err(match result {
                    Ok(_) => error.context(registry::RegistryChangeFailure::FollowUp),
                    Err(primary) => {
                        primary.context(format!("registry follow-up also failed: {error:#}"))
                    }
                });
            }
        }
        result
    }

    fn open_session(&self) -> Result<HubAppServerSession> {
        crate::private_directory::ensure_private_directory(&self.home)?;
        let codex = crate::trusted_executable::pinned_codex_command()
            .context("the MCP hub needs the `codex` CLI")?;
        HubAppServerSession::open(&codex, &self.home)
    }

    /*
     * app-server 세션 하나로 작업한다. 상주 모드는 세션과 스레드를 다시 쓰고, 오류가 나면 세션을 버려서 다음 요청이
     * 새로 시작하게 한다. 일회 모드는 매번 새 세션을 열고 끝나면 닫는다.
     */
    fn with_engine<T>(
        &self,
        work: impl FnOnce(&mut HubAppServerSession, &mut HashMap<String, String>) -> Result<T>,
    ) -> Result<T> {
        self.with_engine_for_catalog(None, work)
    }

    fn with_engine_for_catalog<T>(
        &self,
        identity: Option<&str>,
        work: impl FnOnce(&mut HubAppServerSession, &mut HashMap<String, String>) -> Result<T>,
    ) -> Result<T> {
        let Some(resident) = &self.resident else {
            let mut session = self.open_session()?;
            return work(&mut session, &mut HashMap::new());
        };
        let mut guard = resident
            .lock()
            .map_err(|_| anyhow::anyhow!("the MCP hub engine lock is poisoned"))?;
        if identity.is_some()
            && guard
                .as_ref()
                .is_some_and(|engine| engine.catalog_identity.as_deref() != identity)
        {
            *guard = None;
        }
        if guard.is_none() {
            *guard = Some(ResidentEngine {
                session: self.open_session()?,
                threads: HashMap::new(),
                last_used: Instant::now(),
                catalog_identity: identity.map(str::to_owned),
            });
        }
        let Some(engine) = guard.as_mut() else {
            bail!("the MCP hub engine did not start")
        };
        let result = work(&mut engine.session, &mut engine.threads);
        match result {
            Ok(_) => engine.last_used = Instant::now(),
            Err(_) => *guard = None,
        }
        result
    }

    fn thread_for_server(
        &self,
        session: &mut HubAppServerSession,
        threads: &mut HashMap<String, String>,
        server: &str,
    ) -> Result<String> {
        if let Some(thread) = threads.get(server) {
            return Ok(thread.clone());
        }
        let thread = self.start_thread_for(session, server)?;
        threads.insert(server.to_string(), thread.clone());
        Ok(thread)
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
    fn tool_catalogs(&self) -> Result<Vec<McpHubToolCatalog>> {
        self.cached_tool_catalogs()
    }

    fn invalidate_tool_catalogs(&self) -> Result<()> {
        tool_cache::ToolCatalogCache::new(&self.home).invalidate()
    }
    fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
        Ok(registered_servers_in(&self.read_document()?))
    }

    fn register_server(&self, spec: &McpHubServerSpec) -> Result<()> {
        let result = self.update_document(|document| {
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
            Ok(((), true))
        });
        if result.is_ok() || result.as_ref().is_err_and(registry::was_replaced) {
            self.finish_registry_change(result, true)
        } else {
            result
        }
    }

    fn unregister_server(&self, name: &str) -> Result<bool> {
        let result = self.update_document(|document| {
            let removed = document
                .get_mut("mcp_servers")
                .and_then(Item::as_table_mut)
                .and_then(|servers| servers.remove(name))
                .is_some();
            Ok((removed, removed))
        });
        if result.as_ref().is_ok_and(|removed| *removed)
            || result.as_ref().is_err_and(registry::was_replaced)
        {
            self.finish_registry_change(result, false)
        } else {
            result
        }
    }

    fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
        self.with_engine(|session, threads| {
            let thread = only
                .map(|server| self.thread_for_server(session, threads, server))
                .transpose()?;
            statuses_from(session, thread.as_deref())
        })
    }

    fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
        self.authorize_call(server, tool, &arguments)?;
        self.with_engine(|session, threads| {
            let thread_id = self.thread_for_server(session, threads, server)?;
            session.request(
                "mcpServer/tool/call",
                json!({ "threadId": thread_id, "server": server, "tool": tool, "arguments": arguments }),
                REQUEST_TIMEOUT,
            )
        })
    }

    fn shared_skill(&self, action: McpHubSharedSkillAction) -> Result<McpHubSharedSkill> {
        let akra = std::env::current_exe().context("failed to find the akra executable path")?;
        shared_skill_in(&codex_skills_dir()?, &akra, action)
    }

    /// 로그인은 사용자를 오래 기다리므로 상주 엔진을 쓰지 않고 따로 연다. 끝나면 상주 엔진을 내린다.
    fn login(&self, server: &str, on_authorization_url: &mut dyn FnMut(&str)) -> Result<()> {
        self.invalidate_tool_catalogs()?;
        let result = self
            .open_session()
            .and_then(|mut session| oauth_login(&mut session, server, on_authorization_url));
        self.reload()?;
        self.invalidate_tool_catalogs()?;
        result
    }

    fn reload(&self) -> Result<()> {
        if let Some(resident) = &self.resident {
            let engine = resident
                .lock()
                .map_err(|_| anyhow::anyhow!("the MCP hub engine lock is poisoned"))?
                .take();
            // app-server 정리는 잠금 밖에서 한다.
            drop(engine);
        }
        Ok(())
    }

    fn release_if_idle(&self) {
        let Some(resident) = &self.resident else {
            return;
        };
        let Ok(mut guard) = resident.try_lock() else {
            return;
        };
        let idle = guard
            .as_ref()
            .is_some_and(|engine| is_idle(engine.last_used, Instant::now()));
        let engine = if idle { guard.take() } else { None };
        drop(guard);
        drop(engine);
    }
}

fn is_idle(last_used: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_used) >= RESIDENT_IDLE_LIMIT
}

fn statuses_from(
    session: &mut HubAppServerSession,
    thread_id: Option<&str>,
) -> Result<Vec<McpHubServerStatus>> {
    let mut statuses = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut params = json!({ "detail": "full", "limit": STATUS_PAGE_LIMIT });
        if let Some(thread_id) = thread_id {
            params["threadId"] = json!(thread_id);
            params["detail"] = json!("toolsAndAuthOnly");
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
            return Ok(statuses);
        }
    }
}

/// app-server 연결이 등록할 skill root 이다. hub 에 서버가 없으면 비어 있어서 세션 컨텍스트가 늘지 않는다.
pub(crate) fn agent_skill_roots() -> Vec<PathBuf> {
    let Ok(home) = hub_home() else {
        return Vec::new();
    };
    // Codex 홈에 설치한 skill 은 app-server 가 직접 읽는다. 같은 skill 을 두 번 싣지 않는다.
    if codex_skills_dir().is_ok_and(|skills| shared_skill_path(&skills).exists()) {
        return Vec::new();
    }
    agent_skill_roots_in(&home)
}

/// Codex 가 사용자 skill 을 읽는 폴더이다. Akra 가 열지 않은 Codex 세션도 이 폴더를 읽는다.
fn codex_skills_dir() -> Result<PathBuf> {
    let home = match std::env::var_os("CODEX_HOME").filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => {
            let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            let user_home = std::env::var_os(variable)
                .filter(|path| !path.is_empty())
                .with_context(|| {
                    format!("`{variable}` is not set, so the Codex home is unknown")
                })?;
            PathBuf::from(user_home).join(".codex")
        }
    };
    Ok(home.join("skills"))
}

fn shared_skill_path(codex_skills: &Path) -> PathBuf {
    codex_skills.join(SKILL_NAME).join("SKILL.md")
}

/*
 * 설치본은 `akra` 의 전체 경로를 함께 적는다. Akra 가 열지 않은 세션의 셸은 `akra` 를 PATH 에서 찾지 못할 수
 * 있기 때문이다. 지울 때는 이 skill 의 파일과 빈 폴더만 지운다.
 */
fn shared_skill_in(
    codex_skills: &Path,
    akra: &Path,
    action: McpHubSharedSkillAction,
) -> Result<McpHubSharedSkill> {
    let path = shared_skill_path(codex_skills);
    let directory = codex_skills.join(SKILL_NAME);
    match action {
        McpHubSharedSkillAction::Status => {}
        McpHubSharedSkillAction::Install => {
            std::fs::create_dir_all(&directory)
                .with_context(|| format!("failed to create `{}`", directory.display()))?;
            let text = format!(
                "{SKILL_TEXT}\n## Command location\n\nIf the shell cannot find `akra`, run it by \
                 its full path:\n\n```text\n{}\n```\n",
                akra.display()
            );
            std::fs::write(&path, text)
                .with_context(|| format!("failed to write `{}`", path.display()))?;
        }
        McpHubSharedSkillAction::Remove => {
            match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error)
                        .with_context(|| format!("failed to remove `{}`", path.display()));
                }
                _ => {}
            }
            // 사용자가 폴더에 다른 파일을 두었으면 폴더는 남는다.
            let _ = std::fs::remove_dir(&directory);
        }
    }
    Ok(McpHubSharedSkill {
        installed: path.exists(),
        path: path.display().to_string(),
    })
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

/// app-server 의 로그인 대기를 Akra 의 대기(`LOGIN_TIMEOUT`)와 맞춘다. 빠지면 app-server 가 먼저 끝난다.
fn oauth_login(
    session: &mut HubAppServerSession,
    server: &str,
    on_authorization_url: &mut dyn FnMut(&str),
) -> Result<()> {
    let (response, completed) = session.request_with_notification(
        "mcpServer/oauth/login",
        json!({ "name": server, "timeoutSecs": LOGIN_TIMEOUT.as_secs() }),
        REQUEST_TIMEOUT,
        "mcpServer/oauthLogin/completed",
        |params| params.get("name").and_then(Value::as_str) == Some(server),
    )?;
    let url = response
        .get("authorizationUrl")
        .and_then(Value::as_str)
        .context("mcpServer/oauth/login returned no authorization URL")?;
    on_authorization_url(url);
    let completed = match completed {
        Some(completed) => completed,
        None => session.wait_for_notification(
            "mcpServer/oauthLogin/completed",
            LOGIN_TIMEOUT,
            |params| params.get("name").and_then(Value::as_str) == Some(server),
        )?,
    };
    if completed.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let detail = completed
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("the login did not finish");
    bail!("MCP server `{server}` login failed: {detail}")
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
    fn oauth_login_asks_app_server_to_wait_as_long_as_akra() {
        let home = temp_home("oauth-login-timeout");
        std::fs::create_dir_all(&home).unwrap();
        let script = home.join("fake-app-server.cjs");
        // 로그인 요청의 timeoutSecs 가 600 일 때만 성공을 알린다.
        std::fs::write(
            &script,
            r#"
const lines = require('node:readline').createInterface({input: process.stdin});
const send = message => process.stdout.write(JSON.stringify(message) + '\n');
lines.on('line', line => {
  const message = JSON.parse(line);
  if (message.id === undefined || !message.method) return;
  if (message.method !== 'mcpServer/oauth/login') return send({id: message.id, result: {}});
  send({id: message.id, result: {authorizationUrl: 'https://example.invalid/oauth'}});
  const timeout = message.params.timeoutSecs;
  send({method: 'mcpServer/oauthLogin/completed', params: {
    name: message.params.name, success: timeout === 600, error: `timeoutSecs=${timeout}`
  }});
});
"#,
        )
        .unwrap();
        let node = which::which("node").unwrap();
        let command = crate::trusted_executable::TrustedCommand {
            program: node.clone(),
            source_executable: node,
            prefix_args: vec![script.into_os_string()],
            trust_warning: None,
        };
        let mut session = HubAppServerSession::open(&command, &home).unwrap();
        let mut urls = Vec::new();
        let result = oauth_login(&mut session, "notion", &mut |url| urls.push(url.to_owned()));
        drop(session);
        let _ = std::fs::remove_dir_all(&home);

        result.unwrap();
        assert_eq!(LOGIN_TIMEOUT.as_secs(), 600);
        assert_eq!(urls, ["https://example.invalid/oauth"]);
    }

    struct FixedApprover(McpWriteApproval, std::sync::atomic::AtomicUsize);

    impl McpWriteApprovalPort for FixedApprover {
        fn request_approval(&self, request: &McpWriteApprovalRequest) -> McpWriteApproval {
            assert_eq!(
                (request.server.as_str(), request.tool.as_str()),
                ("notion", "update")
            );
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.0
        }
    }

    /// #2830: a write tool runs only after approval; a broken policy stops every call before the engine starts.
    #[test]
    fn write_tools_need_approval_and_a_broken_policy_stops_every_call() {
        let home = temp_home("write-policy");
        std::fs::create_dir_all(&home).unwrap();
        let backend = CodexMcpHubBackend::with_home(home.clone());
        let arguments = json!({"page": "p1"});
        // No policy file: every tool is a read, as before.
        backend
            .authorize_call("notion", "update", &arguments)
            .unwrap();

        std::fs::write(
            home.join(MCP_WRITE_POLICY_FILE),
            "[servers.notion]
write = [\"update\"]
",
        )
        .unwrap();
        backend
            .authorize_call("notion", "search", &arguments)
            .unwrap();
        let error = backend
            .call_tool("notion", "update", arguments.clone())
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("runs only after approval"),
            "{error:#}"
        );

        for (answer, allowed) in [
            (McpWriteApproval::Approved, true),
            (McpWriteApproval::Declined, false),
            (McpWriteApproval::TimedOut, false),
            (McpWriteApproval::NoApprover, false),
        ] {
            let approver = Arc::new(FixedApprover(answer, Default::default()));
            let backend =
                CodexMcpHubBackend::with_home(home.clone()).with_write_approver(approver.clone());
            assert_eq!(
                backend
                    .authorize_call("notion", "update", &arguments)
                    .is_ok(),
                allowed,
                "{answer:?}"
            );
            assert_eq!(approver.1.load(std::sync::atomic::Ordering::SeqCst), 1);
            backend
                .authorize_call("notion", "search", &arguments)
                .unwrap();
            assert_eq!(
                approver.1.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "reads are not asked"
            );
        }

        std::fs::write(
            home.join(MCP_WRITE_POLICY_FILE),
            "[servers.notion]
write = \"update\"
",
        )
        .unwrap();
        let error = backend
            .call_tool("notion", "search", arguments)
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("no tool call runs"),
            "{error:#}"
        );
        let _ = std::fs::remove_dir_all(&home);
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

    // Spawned by the parent test: separate OS processes must coordinate on
    // the same registry lock, not just a Rust Mutex in one process.
    #[test]
    fn registry_subprocess_register_helper() {
        let Ok(home) = std::env::var("AKRA_TEST_MCP_REGISTRY_HOME") else {
            return;
        };
        let name = std::env::var("AKRA_TEST_MCP_REGISTRY_NAME").unwrap();
        let home = PathBuf::from(home);
        std::fs::write(home.join(format!("{name}.ready")), b"ready").unwrap();
        CodexMcpHubBackend::with_home(home)
            .register_server(&McpHubServerSpec::http(&name, "https://example.test/mcp").unwrap())
            .unwrap();
    }

    #[test]
    fn registry_interprocess_registers_preserve_both_updates() {
        use std::process::{Command, Stdio};
        use std::thread;

        let home = temp_home("interprocess-registry");
        std::fs::create_dir_all(&home).unwrap();
        // Ensure the test also preserves hand-edited options in the original.
        let initial = r#"[mcp_servers.original]
url = "https://original.test/mcp"
startup_timeout_sec = 41
"#;
        std::fs::write(home.join(CONFIG_FILE), initial).unwrap();
        write_agent_skill(&home).unwrap();

        let backend = CodexMcpHubBackend::with_home(home.clone());
        let lock = backend.lock_registry().unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut children = Vec::new();
        for name in ["parallel_a", "parallel_b"] {
            children.push(
                Command::new(&executable)
                    .args([
                        "--exact",
                        "adapter::outbound::mcp_hub::tests::registry_subprocess_register_helper",
                        "--nocapture",
                    ])
                    .env("AKRA_TEST_MCP_REGISTRY_HOME", &home)
                    .env("AKRA_TEST_MCP_REGISTRY_NAME", name)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !["parallel_a", "parallel_b"]
            .iter()
            .all(|name| home.join(format!("{name}.ready")).exists())
        {
            assert!(
                Instant::now() < deadline,
                "child registry processes did not start"
            );
            thread::sleep(Duration::from_millis(10));
        }
        // Both subprocesses reached the registration call, but neither may
        // change the registry while the parent owns the filesystem lock.
        thread::sleep(Duration::from_millis(60));
        assert_eq!(
            std::fs::read_to_string(home.join(CONFIG_FILE)).unwrap(),
            initial
        );
        drop(lock);

        for mut child in children {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "registry subprocess failed: {status}");
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("registry subprocess was blocked after lock release");
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        let registry = std::fs::read_to_string(home.join(CONFIG_FILE)).unwrap();
        for name in ["original", "parallel_a", "parallel_b"] {
            assert!(
                registry.contains(&format!("[mcp_servers.{name}]")),
                "{registry}"
            );
        }
        assert!(registry.contains("startup_timeout_sec = 41"), "{registry}");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn registry_atomic_replace_ignores_orphaned_temporary_file() {
        let home = temp_home("atomic-registry");
        let backend = CodexMcpHubBackend::with_home(home.clone());
        backend
            .register_server(&McpHubServerSpec::http("old", "https://old.test/mcp").unwrap())
            .unwrap();
        // A crashed writer may leave an incomplete temporary file behind;
        // it must not corrupt the installed registry or block later updates.
        let orphan = home.join(".config.toml.abandoned.tmp");
        std::fs::write(&orphan, b"[mcp_servers.truncated").unwrap();
        backend
            .register_server(&McpHubServerSpec::http("new", "https://new.test/mcp").unwrap())
            .unwrap();
        let registry = backend.read_document().unwrap();
        assert!(registry["mcp_servers"]["old"].is_table());
        assert!(registry["mcp_servers"]["new"].is_table());
        assert_eq!(
            std::fs::read_to_string(&orphan).unwrap(),
            "[mcp_servers.truncated"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::metadata(home.join(CONFIG_FILE))
                .unwrap()
                .permissions();
            assert_eq!(permissions.mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn registry_readers_never_observe_partially_written_documents() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let home = temp_home("atomic-reader");
        std::fs::create_dir_all(&home).unwrap();
        let first = format!(
            r#"[mcp_servers.alpha]
url = "https://alpha.test/mcp"
{}"#,
            "# payload\\n".repeat(20_000)
        );
        let second = format!(
            r#"[mcp_servers.beta]
url = "https://beta.test/mcp"
{}"#,
            "# alternate\\n".repeat(20_000)
        );
        std::fs::write(home.join(CONFIG_FILE), &first).unwrap();

        let finished = Arc::new(AtomicBool::new(false));
        let child_finished = finished.clone();
        let writer_home = home.clone();
        let writer_first = first.clone();
        let writer_second = second.clone();
        let writer = std::thread::spawn(move || {
            let backend = CodexMcpHubBackend::with_home(writer_home);
            for i in 0..40 {
                let text = if i % 2 == 0 {
                    &writer_second
                } else {
                    &writer_first
                };
                backend.write_document(&text.parse().unwrap()).unwrap();
            }
            child_finished.store(true, Ordering::Release);
        });
        while !finished.load(Ordering::Acquire) {
            let text = std::fs::read_to_string(home.join(CONFIG_FILE)).unwrap();
            assert!(
                text == first || text == second,
                "observed a partial registry"
            );
        }
        writer.join().unwrap();
        let _ = std::fs::remove_dir_all(home);
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
    fn shared_skill_installs_with_the_command_path_and_removes_only_its_own_file() {
        let skills = temp_home("shared-skill").join("skills");
        let akra = Path::new("/opt/akra/bin/akra");
        let status = |action| shared_skill_in(&skills, akra, action).unwrap();

        assert!(!status(McpHubSharedSkillAction::Status).installed);
        assert!(!skills.exists());

        let installed = status(McpHubSharedSkillAction::Install);
        assert!(installed.installed);
        let text = std::fs::read_to_string(&installed.path).unwrap();
        assert!(text.starts_with("---\nname: akra-mcp-hub\n"));
        assert!(text.contains(&akra.display().to_string()));

        // 사용자가 둔 다른 파일과 폴더는 지우지 않는다.
        let other = skills.join(SKILL_NAME).join("notes.md");
        std::fs::write(&other, "keep").unwrap();
        assert!(!status(McpHubSharedSkillAction::Remove).installed);
        assert!(other.exists());
        assert!(!status(McpHubSharedSkillAction::Remove).installed);
        let _ = std::fs::remove_dir_all(skills.parent().unwrap());
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
    fn resident_engine_becomes_idle_at_the_limit() {
        let start = Instant::now();
        assert!(!is_idle(start, start));
        assert!(!is_idle(
            start,
            start + RESIDENT_IDLE_LIMIT - Duration::from_secs(1)
        ));
        assert!(is_idle(start, start + RESIDENT_IDLE_LIMIT));
    }

    #[test]
    fn one_shot_backend_has_no_engine_to_reload() {
        let backend = CodexMcpHubBackend::with_home(temp_home("reload"));
        backend.reload().unwrap();
        backend.release_if_idle();
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
