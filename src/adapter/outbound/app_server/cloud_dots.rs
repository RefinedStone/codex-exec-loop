//! dot 목록을 ChatGPT 백엔드 REST 로 읽는다 (D-19, dots-integration §4). app-server WebSocket 에는 dot 목록이
//! 없다. `GET https://chatgpt.com/backend-api/tbo` 가 `{items, cursor}` 를 준다 (2026-11-14 실계정 확인).
//! dot 과의 대화는 메시지 방이다. `GET /messaging/rooms/{id}` 가 구성원을, `.../messages` 가 메시지를 준다.
//!
//! 인증은 클라우드 다리와 같은 `~/.codex/auth.json` 의 토큰이다. 요청은 `curl -q --config -` 로 보낸다. 토큰은
//! stdin 의 config 로만 넘기므로 argv·디스크·로그에 남지 않는다. 오류 글자에는 응답 본문을 넣지 않는다.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::cloud_dots_live::start_room_watch;
use super::cloud_upstream::{CloudCredentials, default_user_agent, load_credentials};
use crate::application::port::outbound::dots_port::{
    DotLiveListener, DotMessage, DotRoomMessages, DotSummary, DotsPort,
};
use crate::subprocess;

const DOTS_LIST_URL: &str = "https://chatgpt.com/backend-api/tbo";
const ROOMS_URL: &str = "https://chatgpt.com/backend-api/messaging/rooms";
/// 한 번에 읽는 최근 메시지 수이다. `limit` 은 최신 N개를 준다. 서비스는 30 까지 받고 40 부터 HTTP 422 로
/// 거절한다 (2026-11-14 실계정 확인).
// shortcut: 최근 30개만 읽는다. 더 오래된 대화가 필요해지면 `prev_cursor` 로 페이지를 넘긴다.
const ROOM_MESSAGE_LIMIT: u32 = 30;
const MAX_ROOM_ID_LENGTH: usize = 128;
/// 보내는 글의 상한이다 (글자 수). 서비스의 상한은 모른다. 실수로 큰 글을 보내지 않게 하는 값이다.
const MAX_MESSAGE_CHARS: usize = 8_000;
/// 오류 글자에 넣는 서비스의 이유 글자 상한이다.
const MAX_ERROR_DETAIL_CHARS: usize = 200;
const CONNECT_TIMEOUT_SECONDS: u32 = 10;
const MAX_TIME_SECONDS: u32 = 20;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;

#[derive(Clone)]
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
        parse_dot_list(&self.get(DOTS_LIST_URL, "dots list")?)
    }

    fn room_messages(&self, room_id: &str) -> Result<DotRoomMessages> {
        if !is_safe_room_id(room_id) {
            bail!("the message room id is not valid");
        }
        // 방(구성원, 읽음)과 메시지를 함께 묻는다. 차례로 물으면 요청 하나의 시간만큼 더 걸린다 (2026-11-14 실측:
        // 차례로 약 1.05초, 함께 약 0.64초).
        let room_url = format!("{ROOMS_URL}/{room_id}");
        let (room, messages) = std::thread::scope(|scope| {
            let room = scope.spawn(|| self.get(&room_url, "dots room"));
            let messages = self.get(
                &format!("{ROOMS_URL}/{room_id}/messages?limit={ROOM_MESSAGE_LIMIT}"),
                "dots room messages",
            );
            (room.join(), messages)
        });
        let room = room.map_err(|_| anyhow::anyhow!("the dots room request stopped"))??;
        parse_room_messages(&room, &messages?)
    }

    fn watch_room(&self, room_id: &str, on_state: DotLiveListener) -> Result<Box<dyn Send>> {
        if !is_safe_room_id(room_id) {
            bail!("the message room id is not valid");
        }
        Ok(start_room_watch(
            self.clone(),
            room_id.to_string(),
            on_state,
        ))
    }

    fn send_room_message(&self, room_id: &str, text: &str) -> Result<()> {
        if !is_safe_room_id(room_id) {
            bail!("the message room id is not valid");
        }
        let body = message_body(text, &new_request_id())?;
        // 답의 모양은 쓰지 않는다. HTTP 성공이면 보낸 것이다.
        self.request(
            &format!("{ROOMS_URL}/{room_id}/messages"),
            Some(&body),
            "dots send message",
        )?;
        Ok(())
    }
}

impl CloudDotsRestAdapter {
    /// 끝나지 않는 응답(SSE)을 받을 curl 명령과 그 stdin config 이다. 시간 제한과 상태 코드 출력이 없다. 실행과
    /// 정리는 부르는 쪽이 한다.
    pub(super) fn stream_command(&self, url: &str, json_body: &str) -> Result<(Command, String)> {
        let curl = match &self.curl {
            Ok(path) => path,
            Err(error) => bail!("trusted curl executable could not be pinned for dots: {error}"),
        };
        let credentials = load_credentials(&self.auth_path)?;
        let config = build_curl_config(
            url,
            Some(json_body),
            &credentials,
            &default_user_agent(),
            CurlMode::Stream,
        );
        let mut command = Command::new(curl);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        crate::trusted_executable::configure_credential_command_environment(
            &mut command,
            &cwd,
            false,
        )?;
        command.args(["-q", "--config", "-"]);
        Ok((command, config))
    }

    /// 인증된 GET 하나를 보내고 본문을 돌려준다. `label` 은 오류 글자에만 쓴다.
    pub(super) fn get(&self, url: &str, label: &str) -> Result<String> {
        self.request(url, None, label)
    }

    /// 인증된 요청 하나를 보내고 본문을 돌려준다. `json_body` 가 있으면 POST 이고 없으면 GET 이다.
    fn request(&self, url: &str, json_body: Option<&str>, label: &str) -> Result<String> {
        let curl = match &self.curl {
            Ok(path) => path,
            Err(error) => bail!("trusted curl executable could not be pinned for dots: {error}"),
        };
        let credentials = load_credentials(&self.auth_path)?;
        let config = build_curl_config(
            url,
            json_body,
            &credentials,
            &default_user_agent(),
            CurlMode::Request,
        );

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
            bail!("the {label} request failed ({})", output.status);
        }
        let stdout = String::from_utf8(output.stdout)
            .with_context(|| format!("the {label} response was not utf-8"))?;
        let (status, body) = split_http_response(&stdout)
            .with_context(|| format!("the {label} response had no HTTP status"))?;
        if status >= 400 {
            bail!("{}", http_error(label, status, body));
        }
        Ok(body.to_string())
    }
}

/// curl 의 출력은 `본문 + 줄바꿈 + 상태 코드` 이다 (`write-out`). 마지막 줄에서 상태 코드를 뗀다.
fn split_http_response(stdout: &str) -> Option<(u16, &str)> {
    let (body, status) = stdout.rsplit_once('\n')?;
    Some((status.trim().parse().ok()?, body))
}

/// HTTP 오류의 글자이다. 서비스가 준 짧은 이유(`detail` 글자)만 넣는다. 그 밖의 본문은 넣지 않는다. 이유는 서비스의
/// 글이고 자격증명이 아니다. 예: `422 Messaging with the user's dot requires a stable send identifier`.
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

/// 보내는 요청의 식별자이다 (UUID v4 모양). 서비스는 dot 에게 보내는 메시지마다 `request_id` 를 요구한다.
fn new_request_id() -> String {
    let bits = (rand::random::<u128>() & !(0xf << 76) & !(0x3 << 62)) | (0x4 << 76) | (0x2 << 62);
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// 보내는 메시지의 JSON 본문이다. 서비스의 모양은 `{"content": {"text": ...}, "request_id": ...}` 이다.
/// `content.text` 는 필수 글자이고 `attachments`·`message_metadata`·`reply_to` 는 선택이다 (2026-11-14 에 검증
/// 오류로 확인). `request_id` 가 없으면 서비스가 HTTP 422 "requires a stable send identifier" 로 거절한다
/// (같은 날 실제 전송으로 확인: `request_id` 를 넣으면 HTTP 200). 빈 글과 너무 긴 글은 거절한다.
fn message_body(text: &str, request_id: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        bail!("the message is empty");
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        bail!("the message is longer than {MAX_MESSAGE_CHARS} characters");
    }
    Ok(serde_json::json!({ "content": { "text": text }, "request_id": request_id }).to_string())
}

/// 방 id 는 주소의 한 조각으로 들어간다. 글자·숫자·`-`·`_` 만 받는다. 다른 경로나 query 를 만들 수 없다.
fn is_safe_room_id(room_id: &str) -> bool {
    !room_id.is_empty()
        && room_id.len() <= MAX_ROOM_ID_LENGTH
        && room_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// curl config 이다. stdin 으로만 넘긴다. 헤더는 클라우드 다리와 같은 공식 앱 신원이다.
/// curl 이 응답을 어떻게 받는지이다.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CurlMode {
    /// 끝나는 요청이다. 시간 제한을 두고, 본문 뒤에 상태 코드를 한 줄로 받는다.
    Request,
    /// 끝나지 않는 스트림(SSE)이다. 시간 제한이 없고 버퍼를 쓰지 않는다. HTTP 오류는 종료 코드로 받는다.
    Stream,
}

fn build_curl_config(
    url: &str,
    json_body: Option<&str>,
    credentials: &CloudCredentials,
    user_agent: &str,
    mode: CurlMode,
) -> String {
    let header = |name: &str, value: &str| {
        format!(
            "header = \"{}: {}\"\n",
            name,
            escape_curl_config_value(value)
        )
    };
    let transfer = match mode {
        CurlMode::Request => {
            format!("write-out = \"\\n%{{http_code}}\"\nmax-time = {MAX_TIME_SECONDS}\n")
        }
        CurlMode::Stream => "fail\nno-buffer\n".to_string(),
    };
    let mut config = format!(
        "silent\nshow-error\n{transfer}connect-timeout = {CONNECT_TIMEOUT_SECONDS}\nrequest = \"{}\"\nurl = \"{}\"\n",
        if json_body.is_some() { "POST" } else { "GET" },
        escape_curl_config_value(url),
    );
    if mode == CurlMode::Stream {
        config.push_str(&header("Accept", "text/event-stream"));
    }
    if let Some(body) = json_body {
        config.push_str(&header("Content-Type", "application/json"));
        config.push_str(&format!("data = \"{}\"\n", escape_curl_config_value(body)));
    }
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
    #[serde(default)]
    messaging_room_id: Option<String>,
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
            messaging_room_id: dot.messaging_room_id,
        })
        .collect())
}

#[derive(Deserialize)]
struct RoomResponse {
    #[serde(default)]
    members: Vec<RoomMemberResponse>,
    #[serde(default)]
    member_profile_snapshots: Vec<RoomProfileResponse>,
    #[serde(default)]
    read_receipts: Vec<RoomReadReceiptResponse>,
}

#[derive(Deserialize)]
struct RoomReadReceiptResponse {
    account_user_id: String,
    #[serde(default)]
    last_read_at: Option<String>,
}

#[derive(Deserialize)]
struct RoomMemberResponse {
    account_user_id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct RoomProfileResponse {
    account_user_id: String,
}

#[derive(Deserialize)]
struct RoomMessagesResponse {
    #[serde(default)]
    items: Vec<RoomMessageResponse>,
}

#[derive(Deserialize)]
struct RoomMessageResponse {
    id: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    account_user_id: Option<String>,
    #[serde(default)]
    content: Option<RoomMessageContentResponse>,
    #[serde(default)]
    deleted_at: Option<String>,
}

#[derive(Deserialize)]
struct RoomMessageContentResponse {
    #[serde(default)]
    text: Option<String>,
}

/// 방과 메시지 응답을 화면용 메시지로 옮긴다. 지운 메시지는 뺀다. 서비스는 모든 메시지의 `role` 을 `user` 로
/// 주므로 보낸 쪽은 `account_user_id` 로 가린다. dot 의 id 는 `member_profile_snapshots` 에 있다 (2026-11-14 에
/// 계정 하나에서 본 규칙이다. 틀리면 dot 의 메시지가 사람 쪽에 보인다).
fn parse_room_messages(room_body: &str, messages_body: &str) -> Result<DotRoomMessages> {
    let room: RoomResponse =
        serde_json::from_str(room_body).context("failed to parse the dots room response")?;
    let messages: RoomMessagesResponse = serde_json::from_str(messages_body)
        .context("failed to parse the dots room messages response")?;
    // dot 의 읽음 표시이다. dot 은 `member_profile_snapshots` 의 id 이다.
    let dot_read_at = room
        .read_receipts
        .iter()
        .filter(|receipt| {
            room.member_profile_snapshots
                .iter()
                .any(|profile| profile.account_user_id == receipt.account_user_id)
        })
        .filter_map(|receipt| receipt.last_read_at.clone())
        .max();
    let messages = messages
        .items
        .into_iter()
        .filter(|message| message.deleted_at.is_none())
        .map(|message| {
            let sender = message.account_user_id.unwrap_or_default();
            DotMessage {
                id: message.id,
                created_at: message.created_at.unwrap_or_default(),
                author: room
                    .members
                    .iter()
                    .find(|member| member.account_user_id == sender)
                    .and_then(|member| member.name.clone())
                    .unwrap_or_default(),
                from_dot: room
                    .member_profile_snapshots
                    .iter()
                    .any(|profile| profile.account_user_id == sender),
                text: message
                    .content
                    .and_then(|content| content.text)
                    .unwrap_or_default(),
            }
        })
        .collect();
    Ok(DotRoomMessages {
        messages,
        dot_read_at,
    })
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

    /// 실계정 읽기 확인이다 (조회만). 첫 dot 의 메시지 방을 읽는다. 개수만 찍는다.
    /// 수동 실행: `cargo test --lib cloud_dots::tests::live_room_messages -- --ignored --nocapture`.
    #[test]
    #[ignore = "real ChatGPT account and network; run manually"]
    fn live_room_messages() {
        let auth_path = super::super::cloud_upstream::codex_auth_path().expect("auth path");
        let adapter = CloudDotsRestAdapter::new(auth_path);
        let dots = adapter.list_dots().expect("the dot list must load");
        let Some(room_id) = dots.iter().find_map(|dot| dot.messaging_room_id.clone()) else {
            eprintln!("live: no dot has a message room");
            return;
        };
        let started = std::time::Instant::now();
        let messages = adapter
            .room_messages(&room_id)
            .expect("the room messages must load");
        eprintln!(
            "live: room messages took {} ms",
            started.elapsed().as_millis()
        );
        eprintln!(
            "live: dot read marker present: {}",
            messages.dot_read_at.is_some()
        );
        let messages = messages.messages;
        eprintln!(
            "live: {} messages, {} from the dot, {} without an author name",
            messages.len(),
            messages.iter().filter(|message| message.from_dot).count(),
            messages
                .iter()
                .filter(|message| message.author.is_empty())
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
            None,
            &credentials(),
            "Codex Desktop/1.2.3 (Windows; x86_64)",
            CurlMode::Request,
        );
        assert!(!config.contains("data ="));
        assert!(config.contains("request = \"GET\""));
        assert!(config.contains("url = \"https://example.test/tbo\""));
        // 상태 코드를 본문 뒤의 한 줄로 받는다. HTTP 오류의 본문도 읽어야 하므로 `fail` 은 쓰지 않는다.
        assert!(
            config
                .lines()
                .any(|line| line == r#"write-out = "\n%{http_code}""#)
        );
        assert!(!config.lines().any(|line| line == "fail"));
        assert!(config.contains("header = \"Authorization: Bearer header.payload.signature\""));
        assert!(config.contains("header = \"ChatGPT-Account-Id: acc-1\""));
        assert!(config.contains("header = \"originator: Codex Desktop\""));
        assert!(config.contains("header = \"User-Agent: Codex Desktop/1.2.3 (Windows; x86_64)\""));
    }

    #[test]
    fn a_stream_has_no_time_limit_and_no_buffer() {
        let config = build_curl_config(
            "https://example.test/rooms/r1/live",
            Some("{}"),
            &credentials(),
            "agent",
            CurlMode::Stream,
        );
        assert!(config.lines().any(|line| line == "no-buffer"));
        assert!(config.lines().any(|line| line == "fail"));
        assert!(!config.contains("max-time"));
        assert!(!config.contains("write-out"));
        assert!(config.contains("header = \"Accept: text/event-stream\""));
        assert!(config.contains("request = \"POST\""));
    }

    #[test]
    fn a_header_value_cannot_add_a_config_line() {
        let mut hostile = credentials();
        hostile.account_id = "acc\"\noutput = \"/tmp/x".to_string();
        let config = build_curl_config(
            "https://example.test/tbo",
            None,
            &hostile,
            "agent",
            CurlMode::Request,
        );
        assert!(!config.lines().any(|line| line.starts_with("output")));
    }

    #[test]
    fn the_dot_list_maps_to_summaries() {
        let body = r#"{"items":[
            {"id":"dot-1","display_name":"Mika","status":"active","aeon_kind":"orbit",
             "is_paused":false,"active_root_thread_id":"thread-1","messaging_room_id":"room-1",
             "color":"blue","unknown":1},
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
                messaging_room_id: Some("room-1".to_string()),
            }
        );
        // 이름이 없으면 id 를 보인다. thread 가 없으면 열 수 없다.
        assert_eq!(dots[1].display_name, "dot-2");
        assert_eq!(dots[1].root_thread_id, None);
    }

    #[test]
    fn room_messages_name_the_sender_and_mark_the_dot() {
        let room = r#"{"members":[
            {"role":"user","account_user_id":"user-1","name":"Me"},
            {"role":"user","account_user_id":"calpico-9","name":"Mika"}
        ],"member_profile_snapshots":[{"account_user_id":"calpico-9"}],
        "read_receipts":[
            {"account_user_id":"user-1","last_read_at":"2026-11-14T09:00:00Z"},
            {"account_user_id":"calpico-9","last_read_at":"2026-11-14T01:00:03Z"}
        ]}"#;
        let messages = r#"{"items":[
            {"id":"m1","created_at":"2026-11-14T01:00:00Z","role":"user","account_user_id":"user-1",
             "content":{"text":"hello","attachments":[]},"deleted_at":null},
            {"id":"m2","created_at":"2026-11-14T01:00:05Z","role":"user","account_user_id":"calpico-9",
             "content":{"text":"hi"},"deleted_at":null},
            {"id":"m3","account_user_id":"user-1","content":{"text":"gone"},"deleted_at":"2026-11-14T02:00:00Z"},
            {"id":"m4","account_user_id":"stranger"}
        ],"prev_cursor":null,"next_cursor":null}"#;
        let room_messages = parse_room_messages(room, messages).expect("messages parse");
        // 읽음 표시는 dot 의 것만 쓴다. 내 것은 쓰지 않는다.
        assert_eq!(
            room_messages.dot_read_at.as_deref(),
            Some("2026-11-14T01:00:03Z")
        );
        let parsed = room_messages.messages;
        assert_eq!(
            parsed[0],
            DotMessage {
                id: "m1".to_string(),
                created_at: "2026-11-14T01:00:00Z".to_string(),
                author: "Me".to_string(),
                from_dot: false,
                text: "hello".to_string(),
            }
        );
        assert_eq!(parsed[1].author, "Mika");
        assert!(parsed[1].from_dot);
        // 지운 메시지는 없다. 구성원에 없는 보낸 쪽은 이름이 비고 사람 쪽이다.
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[2].id, "m4");
        assert_eq!(parsed[2].author, "");
        assert!(!parsed[2].from_dot);
    }

    #[test]
    fn a_message_becomes_a_post_with_a_json_body() {
        let body = message_body("  hello \"dot\"\nline 2  ", "req-1").expect("body");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!({ "content": { "text": "hello \"dot\"\nline 2" }, "request_id": "req-1" })
        );

        let config = build_curl_config(
            "https://example.test/rooms/r1/messages",
            Some(&body),
            &credentials(),
            "agent",
            CurlMode::Request,
        );
        assert!(config.contains("request = \"POST\""));
        assert!(config.contains("header = \"Content-Type: application/json\""));
        // 본문은 config 의 한 줄이다. 글 안의 따옴표와 줄바꿈이 다른 줄을 만들지 않는다.
        let data_lines: Vec<&str> = config
            .lines()
            .filter(|line| line.starts_with("data = "))
            .collect();
        assert_eq!(data_lines.len(), 1);
        assert!(!config.lines().any(|line| line.starts_with("line 2")));
    }

    #[test]
    fn an_empty_or_huge_message_is_not_sent() {
        assert!(message_body("   \n ", "req-1").is_err());
        assert!(message_body(&"가".repeat(MAX_MESSAGE_CHARS), "req-1").is_ok());
        assert!(message_body(&"가".repeat(MAX_MESSAGE_CHARS + 1), "req-1").is_err());
    }

    #[test]
    fn every_send_has_its_own_uuid_shaped_request_id() {
        let first = new_request_id();
        let second = new_request_id();
        assert_ne!(first, second);
        let parts: Vec<&str> = first.split('-').collect();
        assert_eq!(
            parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(first.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
        // UUID v4 의 버전과 변형 자리이다.
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn the_http_status_is_the_last_line_and_an_error_keeps_only_the_reason() {
        assert_eq!(
            split_http_response("{\"items\":[]}\n200"),
            Some((200, "{\"items\":[]}"))
        );
        // 본문에 줄바꿈이 있어도 마지막 줄만 상태 코드이다.
        assert_eq!(split_http_response("a\nb\n404"), Some((404, "a\nb")));
        assert_eq!(split_http_response("no status"), None);

        let reason = http_error(
            "dots send message",
            422,
            r#"{"detail":"Messaging with the user's dot requires a stable send identifier"}"#,
        );
        assert_eq!(
            reason,
            "the dots send message request failed (HTTP 422): Messaging with the user's dot requires a stable send identifier"
        );
        // 이유가 글자가 아니면(검증 오류 목록 등) 상태 코드만 남긴다. 그 밖의 본문은 넣지 않는다.
        let list = http_error(
            "dots send message",
            422,
            r#"{"detail":[{"msg":"x"}],"other":"secret"}"#,
        );
        assert_eq!(list, "the dots send message request failed (HTTP 422)");
        assert_eq!(
            http_error("dots list", 502, "<html>secret page</html>"),
            "the dots list request failed (HTTP 502)"
        );
        let long = http_error(
            "dots list",
            400,
            &format!(r#"{{"detail":"{}"}}"#, "x".repeat(500)),
        );
        assert!(long.len() < 300);
    }

    #[test]
    fn a_room_id_cannot_leave_its_path_segment() {
        assert!(is_safe_room_id("room_1-Ab"));
        for hostile in [
            "",
            "a/b",
            "a?x=1",
            "..",
            "a b",
            "a%2Fb",
            "a#b",
            &"x".repeat(129),
        ] {
            assert!(!is_safe_room_id(hostile), "{hostile}");
        }
    }

    #[test]
    fn a_body_that_is_not_a_dot_list_is_an_error_without_the_body() {
        let error = parse_dot_list("<html>secret page</html>").expect_err("not json");
        assert!(!format!("{error:#}").contains("secret page"));
        assert!(parse_dot_list("{}").expect("empty object").is_empty());
    }
}
