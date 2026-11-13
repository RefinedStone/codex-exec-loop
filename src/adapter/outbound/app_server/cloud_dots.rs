//! dot 목록을 ChatGPT 백엔드 REST 로 읽는다 (D-19, dots-integration §4). app-server WebSocket 에는 dot 목록이
//! 없다. `GET https://chatgpt.com/backend-api/tbo` 가 `{items, cursor}` 를 준다 (2026-11-14 실계정 확인).
//!
//! 인증은 클라우드 다리와 같은 `~/.codex/auth.json` 의 토큰이다. 요청은 `curl -q --config -` 로 보낸다. 토큰은
//! stdin 의 config 로만 넘기므로 argv·디스크·로그에 남지 않는다. 오류 글자에는 응답 본문을 넣지 않는다.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::cloud_upstream::{CloudCredentials, default_user_agent, load_credentials};
use crate::application::port::outbound::dots_port::{DotSummary, DotsPort};
use crate::subprocess;

const DOTS_LIST_URL: &str = "https://chatgpt.com/backend-api/tbo";
const CONNECT_TIMEOUT_SECONDS: u32 = 10;
const MAX_TIME_SECONDS: u32 = 20;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;

pub(crate) struct CloudDotsRestAdapter {
    auth_path: PathBuf,
    /// 믿을 수 있는 curl 의 절대 경로이다. 찾지 못했으면 그 이유이다.
    curl: std::result::Result<PathBuf, String>,
}

impl CloudDotsRestAdapter {
    pub(crate) fn new(auth_path: PathBuf) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let curl = crate::trusted_executable::resolve_native_from_current_path("curl", &cwd)
            .map_err(|error| error.to_string());
        Self { auth_path, curl }
    }
}

impl DotsPort for CloudDotsRestAdapter {
    fn list_dots(&self) -> Result<Vec<DotSummary>> {
        let curl = match &self.curl {
            Ok(path) => path,
            Err(error) => bail!("trusted curl executable could not be pinned for dots: {error}"),
        };
        let credentials = load_credentials(&self.auth_path)?;
        let config = build_curl_config(DOTS_LIST_URL, &credentials, &default_user_agent());

        let mut command = Command::new(curl);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        crate::trusted_executable::configure_credential_command_environment(
            &mut command,
            &cwd,
            false,
        )?;
        // `-q` 는 argv[1] 이어야 한다. 아니면 curl 이 ~/.curlrc 를 먼저 읽어 trace·proxy 로 토큰이 샐 수 있다.
        command.args(["-q", "--config", "-"]);
        let output = subprocess::command_output_with_input_timeout_and_limits(
            &mut command,
            "curl -q --config - dots list",
            config.as_bytes(),
            Duration::from_secs(u64::from(MAX_TIME_SECONDS) + 1),
            MAX_RESPONSE_BYTES,
            MAX_ERROR_BYTES,
        )
        .context("failed to wait for curl during the dots list request")?;
        if !output.status.success() {
            // curl 22 는 HTTP 400 이상이다 (`fail`). 본문과 stderr 는 넣지 않는다.
            bail!("the dots list request failed ({})", output.status);
        }
        let body =
            String::from_utf8(output.stdout).context("the dots list response was not utf-8")?;
        parse_dot_list(&body)
    }
}

/// curl config 이다. stdin 으로만 넘긴다. 헤더는 클라우드 다리와 같은 공식 앱 신원이다.
fn build_curl_config(url: &str, credentials: &CloudCredentials, user_agent: &str) -> String {
    let header = |name: &str, value: &str| {
        format!(
            "header = \"{}: {}\"\n",
            name,
            escape_curl_config_value(value)
        )
    };
    let mut config = format!(
        "silent\nshow-error\nfail\nconnect-timeout = {CONNECT_TIMEOUT_SECONDS}\nmax-time = {MAX_TIME_SECONDS}\nrequest = \"GET\"\nurl = \"{}\"\n",
        escape_curl_config_value(url),
    );
    config.push_str(&header(
        "Authorization",
        &format!("Bearer {}", credentials.access_token),
    ));
    config.push_str(&header("ChatGPT-Account-Id", &credentials.account_id));
    config.push_str(&header("originator", "Codex Desktop"));
    config.push_str(&header("X-OpenAI-Product-Sku", "codex"));
    config.push_str(&header("User-Agent", user_agent));
    config
}

/// curl config 의 따옴표 값 안에서 뜻이 있는 글자만 escape 한다.
fn escape_curl_config_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[derive(Deserialize)]
struct DotListResponse {
    #[serde(default)]
    items: Vec<DotResponse>,
}

/// 서비스의 dot 하나이다. 쓰는 필드만 읽는다. 없는 필드는 기본값이다 (경로·필드가 자주 바뀐다).
#[derive(Deserialize)]
struct DotResponse {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    aeon_kind: Option<String>,
    #[serde(default)]
    is_paused: bool,
    #[serde(default)]
    active_root_thread_id: Option<String>,
}

// shortcut: 첫 페이지만 읽는다 (`cursor` 무시). dot 이 한 페이지를 넘는 계정이 나오면 cursor 를 따른다.
fn parse_dot_list(body: &str) -> Result<Vec<DotSummary>> {
    let response: DotListResponse =
        serde_json::from_str(body).context("failed to parse the dots list response")?;
    Ok(response
        .items
        .into_iter()
        .map(|dot| DotSummary {
            display_name: dot.display_name.unwrap_or_else(|| dot.id.clone()),
            id: dot.id,
            status: dot.status.unwrap_or_default(),
            kind: dot.aeon_kind.unwrap_or_default(),
            paused: dot.is_paused,
            root_thread_id: dot.active_root_thread_id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실계정 읽기 확인이다 (조회만). 네트워크와 실토큰이 필요하므로 `#[ignore]` 이다. 개수만 찍는다.
    /// 수동 실행: `cargo test --lib cloud_dots::tests::live_dot_list -- --ignored --nocapture`.
    #[test]
    #[ignore = "real ChatGPT account and network; run manually"]
    fn live_dot_list() {
        let auth_path = super::super::cloud_upstream::codex_auth_path().expect("auth path");
        let dots = CloudDotsRestAdapter::new(auth_path)
            .list_dots()
            .expect("the dot list must load");
        eprintln!(
            "live: {} dots, {} with a work thread",
            dots.len(),
            dots.iter()
                .filter(|dot| dot.root_thread_id.is_some())
                .count()
        );
    }

    fn credentials() -> CloudCredentials {
        CloudCredentials {
            access_token: "header.payload.signature".to_string(),
            account_id: "acc-1".to_string(),
        }
    }

    #[test]
    fn the_curl_config_carries_the_login_and_the_desktop_identity() {
        let config = build_curl_config(
            "https://example.test/tbo",
            &credentials(),
            "Codex Desktop/1.2.3 (Windows; x86_64)",
        );
        assert!(config.contains("request = \"GET\""));
        assert!(config.contains("url = \"https://example.test/tbo\""));
        // HTTP 오류를 종료 코드로 받는다.
        assert!(config.lines().any(|line| line == "fail"));
        assert!(config.contains("header = \"Authorization: Bearer header.payload.signature\""));
        assert!(config.contains("header = \"ChatGPT-Account-Id: acc-1\""));
        assert!(config.contains("header = \"originator: Codex Desktop\""));
        assert!(config.contains("header = \"User-Agent: Codex Desktop/1.2.3 (Windows; x86_64)\""));
    }

    #[test]
    fn a_header_value_cannot_add_a_config_line() {
        let mut hostile = credentials();
        hostile.account_id = "acc\"\noutput = \"/tmp/x".to_string();
        let config = build_curl_config("https://example.test/tbo", &hostile, "agent");
        assert!(!config.lines().any(|line| line.starts_with("output")));
    }

    #[test]
    fn the_dot_list_maps_to_summaries() {
        let body = r#"{"items":[
            {"id":"dot-1","display_name":"Mika","status":"active","aeon_kind":"orbit",
             "is_paused":false,"active_root_thread_id":"thread-1","color":"blue","unknown":1},
            {"id":"dot-2"}
        ],"cursor":null}"#;
        let dots = parse_dot_list(body).expect("list parses");
        assert_eq!(
            dots[0],
            DotSummary {
                id: "dot-1".to_string(),
                display_name: "Mika".to_string(),
                status: "active".to_string(),
                kind: "orbit".to_string(),
                paused: false,
                root_thread_id: Some("thread-1".to_string()),
            }
        );
        // 이름이 없으면 id 를 보인다. thread 가 없으면 열 수 없다.
        assert_eq!(dots[1].display_name, "dot-2");
        assert_eq!(dots[1].root_thread_id, None);
    }

    #[test]
    fn a_body_that_is_not_a_dot_list_is_an_error_without_the_body() {
        let error = parse_dot_list("<html>secret page</html>").expect_err("not json");
        assert!(!format!("{error:#}").contains("secret page"));
        assert!(parse_dot_list("{}").expect("empty object").is_empty());
    }
}
