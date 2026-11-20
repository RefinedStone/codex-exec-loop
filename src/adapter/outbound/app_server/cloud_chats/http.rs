//! ChatGPT history's HTTP transport via curl.
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::{Context, anyhow};

use super::super::cloud_upstream::{CloudCredentials, default_user_agent};
use super::{CloudChatsRestAdapter, Query, whole_seconds_rounded_up};
use crate::application::port::outbound::chats_port::ChatsError;
use crate::subprocess;

const MAX_ERROR_DETAIL_CHARS: usize = 200;
const CONNECT_TIMEOUT_SECONDS: u32 = 10;
const MAX_TIME_SECONDS: u32 = 20;
// 한 건의 응답은 모든 가지와 도구 출력을 담는다.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;

impl CloudChatsRestAdapter {
    /// 인증된 GET 하나를 보내고 본문을 돌려준다. 429 는 `RateLimited` 이다. `label` 은 오류 글자에만 쓴다.
    /// 자격증명은 부르는 쪽이 한 번 읽어 넘긴다: 요청을 보낸 계정과 캐시의 계정이 같아야 한다.
    pub(super) fn get(
        &self,
        query: Query<'_>,
        credentials: &CloudCredentials,
    ) -> Result<String, ChatsError> {
        let label = query.label();
        let curl = match &self.curl {
            Ok(path) => path,
            Err(error) => {
                return Err(ChatsError::Other(anyhow!(
                    "trusted curl executable could not be pinned for chats: {error}"
                )));
            }
        };
        let config = build_curl_config(&query.url(), credentials, &default_user_agent());

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
            &format!("curl -q --config - {label}"),
            config.as_bytes(),
            Duration::from_secs(u64::from(MAX_TIME_SECONDS) + 1),
            MAX_RESPONSE_BYTES,
            MAX_ERROR_BYTES,
        )
        .with_context(|| format!("failed to wait for curl during the {label} request"))?;
        if !output.status.success() {
            // 연결·시간 초과 같은 curl 자체의 실패이다. stderr 는 넣지 않는다.
            return Err(ChatsError::Other(anyhow!(
                "the {label} request failed ({})",
                output.status
            )));
        }
        let stdout = String::from_utf8(output.stdout)
            .with_context(|| format!("the {label} response was not utf-8"))?;
        let response = split_http_response(&stdout, SystemTime::now())
            .with_context(|| format!("the {label} response had no HTTP status"))?;
        if response.status == 429 {
            return Err(ChatsError::RateLimited {
                retry_after_seconds: response.retry_after_seconds,
            });
        }
        if response.status >= 400 {
            return Err(ChatsError::Other(anyhow!(
                "{}",
                http_error(label, response.status, response.body)
            )));
        }
        Ok(response.body.to_string())
    }
}

/// curl config 이다. stdin 으로만 넘긴다. 헤더는 공식 앱이 기록을 읽을 때와 같다 (`X-OpenAI-Product-Sku` 는 빼고).
/// 응답 헤더를 본문 앞에 붙인다. curl 7.84 이전에는 `%header{retry-after}` 를 쓸 수 없다.
fn build_curl_config(url: &str, credentials: &CloudCredentials, user_agent: &str) -> String {
    let header = |name: &str, value: &str| {
        format!(
            "header = \"{}: {}\"\n",
            name,
            escape_curl_config_value(value)
        )
    };
    let mut config = format!(
        "silent\nshow-error\ninclude\nsuppress-connect-headers\nwrite-out = \"\\n%{{http_code}}\"\nmax-time = {MAX_TIME_SECONDS}\nconnect-timeout = {CONNECT_TIMEOUT_SECONDS}\nrequest = \"GET\"\nurl = \"{}\"\n",
        escape_curl_config_value(url),
    );
    config.push_str(&header(
        "Authorization",
        &format!("Bearer {}", credentials.access_token),
    ));
    config.push_str(&header("ChatGPT-Account-Id", &credentials.account_id));
    config.push_str(&header("originator", "Codex Desktop"));
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

#[derive(Debug)]
pub(super) struct ChatsHttp<'a> {
    pub(super) status: u16,
    pub(super) retry_after_seconds: Option<u64>,
    pub(super) body: &'a str,
}

pub(super) fn split_http_response(stdout: &str, now: SystemTime) -> Option<ChatsHttp<'_>> {
    let (mut rest, status_line) = stdout.rsplit_once('\n')?;
    let status = status_line.trim().parse().ok()?;
    loop {
        let (headers, body) = ["\r\n\r\n", "\n\n"]
            .into_iter()
            .filter_map(|separator| rest.split_once(separator))
            .min_by_key(|(headers, _)| headers.len())?;
        let mut lines = headers.lines();
        let mut status_line = lines.next()?.split_whitespace();
        status_line.next()?.strip_prefix("HTTP/")?;
        let header_status: u16 = status_line.next()?.parse().ok()?;
        if (100..200).contains(&header_status) {
            rest = body;
            continue;
        }
        if status != header_status {
            return None;
        }
        let retry_after_seconds = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
            .and_then(|(_, value)| {
                let value = value.trim();
                value.parse().ok().or_else(|| {
                    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
                    let remaining = SystemTime::from(date)
                        .duration_since(now)
                        .unwrap_or_default();
                    Some(whole_seconds_rounded_up(remaining))
                })
            });
        return Some(ChatsHttp {
            status,
            retry_after_seconds,
            body,
        });
    }
}

/// HTTP 오류의 글자이다. 서비스가 준 짧은 이유(`detail` 글자)만 넣는다. 그 밖의 본문은 넣지 않는다.
fn http_error(label: &str, status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("detail")?.as_str().map(str::to_string));
    match detail {
        Some(detail) => {
            let detail: String = detail.chars().take(MAX_ERROR_DETAIL_CHARS).collect();
            format!("the {label} request failed (HTTP {status}): {detail}")
        }
        None => format!("the {label} request failed (HTTP {status})"),
    }
}
