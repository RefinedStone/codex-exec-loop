/*
Telegram Bot API outbound adapter다.

application 계층은 polling과 sendMessage를 `TelegramBotPort`로만 호출한다. 이 파일은 그 port 요청을
curl 기반 HTTPS POST로 변환하고, Telegram의 `{ ok, result, description }` envelope를 내부 DTO에서
application DTO로 다시 매핑한다. HTTP client crate를 추가하지 않고 curl config를 stdin으로 넘기는 이유는
token이 argv/process listing에 직접 노출되는 면을 줄이고, 기존 운영 환경의 curl 의존성만 사용하기 위해서다.
*/
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde::Serialize;
use std::io::Write;
use std::process::{Command, Stdio};
const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
const CURL_CONNECT_TIMEOUT_SECONDS: &str = "10";
const DEFAULT_ALLOWED_UPDATES: [&str; 1] = ["message"];

pub struct CurlTelegramBotAdapter {
    // 테스트와 production이 같은 request path를 쓰되, curl binary와 API base URL은 adapter 상태로 둔다.
    curl_path: String,
    api_base_url: String,
    token: String,
}

impl CurlTelegramBotAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            curl_path: "curl".to_string(),
            api_base_url: TELEGRAM_API_BASE_URL.to_string(),
            token: token.into(),
        }
    }
    fn execute_json_request<TRequest, TResponse>(
        &self,
        method_name: &str,
        body: &TRequest,
        timeout_seconds: u16,
    ) -> Result<TResponse>
    where
        TRequest: Serialize,
        TResponse: for<'de> Deserialize<'de>,
    {
        /*
        Telegram API 호출의 공통 경계다.

        각 port method는 Telegram method 이름과 payload만 넘기고, 이 helper가 JSON 직렬화, curl 실행,
        envelope 검증, result payload 추출을 모두 처리한다. polling timeout보다 curl max-time을 15초
        길게 잡아 Telegram long polling 자체의 timeout과 네트워크 여유 시간을 분리한다.
        */
        let url = format!("{}/bot{}/{}", self.api_base_url, self.token, method_name);
        let json_body = serde_json::to_string(body).context("failed to serialize request body")?;
        let max_time_seconds = u32::from(timeout_seconds).saturating_add(15);
        let mut child = Command::new(&self.curl_path)
            .args(["--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to invoke curl for Telegram {method_name}"))?;
        let config = build_curl_config(url.as_str(), json_body.as_str(), max_time_seconds);
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open curl stdin for Telegram {method_name}"))?;
        stdin
            .write_all(config.as_bytes())
            .with_context(|| format!("failed to write curl config for Telegram {method_name}"))?;
        drop(stdin);
        // stdin을 닫아야 curl이 `--config -` 입력 종료를 감지하고 실제 요청을 시작한다.
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed to wait for curl during Telegram {method_name}"))?;
        if !output.status.success() {
            bail!(
                "telegram {method_name} request failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let body = String::from_utf8(output.stdout)
            .with_context(|| format!("telegram {method_name} response was not valid utf-8"))?;
        let envelope = serde_json::from_str::<TelegramApiEnvelope<TResponse>>(&body)
            .with_context(|| format!("failed to parse telegram {method_name} response"))?;
        if !envelope.ok {
            // Telegram은 HTTP 200에서도 ok=false를 반환할 수 있으므로 envelope 레벨 오류를 별도로 올린다.
            return Err(anyhow!(
                "telegram {method_name} rejected the request: {}",
                envelope
                    .description
                    .unwrap_or_else(|| "unknown telegram api error".to_string())
            ));
        }
        envelope.result.ok_or_else(|| {
            anyhow!("telegram {method_name} returned ok=true without a result payload")
        })
    }
}

impl TelegramBotPort for CurlTelegramBotAdapter {
    fn get_updates(&self, request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>> {
        // message update만 요청해 bot command 처리 경로가 다루지 않는 callback/query update를 upstream에서 걸러낸다.
        let response = self.execute_json_request::<_, Vec<TelegramUpdateResponse>>(
            "getUpdates",
            &TelegramGetUpdatesPayload {
                offset: request.offset,
                limit: request.limit,
                timeout: request.timeout_seconds,
                allowed_updates: DEFAULT_ALLOWED_UPDATES,
            },
            request.timeout_seconds,
        )?;
        Ok(response.into_iter().map(Into::into).collect())
    }
    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()> {
        // port 계약은 전송 성공 여부만 필요하므로 Telegram result의 message_id는 DTO parse 검증 후 버린다.
        let _: TelegramSendMessageResponse = self.execute_json_request(
            "sendMessage",
            &TelegramSendMessagePayload {
                chat_id: request.chat_id,
                text: request.text.as_str(),
            },
            15,
        )?;
        Ok(())
    }
}

/*
Telegram API envelope와 payload DTO들이다.

이 private 타입들은 Telegram JSON field를 그대로 반영하고, 아래 `From` 구현에서 application port 타입으로
정규화한다. 이렇게 하면 Telegram의 optional sender/text 같은 세부사항이 application 계층의 안정적인
`TelegramUpdate` 계약으로만 노출된다.
*/
#[derive(Debug, Deserialize)]
struct TelegramApiEnvelope<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}
#[derive(Debug, Serialize)]
struct TelegramGetUpdatesPayload<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<i64>,
    limit: u8,
    timeout: u16,
    allowed_updates: [&'a str; 1],
}
#[derive(Debug, Serialize)]
struct TelegramSendMessagePayload<'a> {
    chat_id: i64,
    text: &'a str,
}
#[derive(Debug, Deserialize)]
struct TelegramUpdateResponse {
    update_id: i64,
    message: Option<TelegramMessageResponse>,
}
impl From<TelegramUpdateResponse> for TelegramUpdate {
    fn from(value: TelegramUpdateResponse) -> Self {
        Self {
            update_id: value.update_id,
            message: value.message.map(Into::into),
        }
    }
}
#[derive(Debug, Deserialize)]
struct TelegramMessageResponse {
    message_id: i64,
    chat: TelegramChatResponse,
    text: Option<String>,
    from: Option<TelegramUserResponse>,
}
impl From<TelegramMessageResponse> for TelegramInboundMessage {
    fn from(value: TelegramMessageResponse) -> Self {
        Self {
            message_id: value.message_id,
            chat_id: value.chat.id,
            text: value.text,
            // operator-facing sender label은 username을 우선하고, 없으면 Telegram profile 이름으로 fallback한다.
            sender_display_name: value.from.and_then(|user| {
                user.username
                    .or(user.first_name)
                    .or(user.last_name)
                    .filter(|value| !value.trim().is_empty())
            }),
        }
    }
}
#[derive(Debug, Deserialize)]
struct TelegramChatResponse {
    id: i64,
}
#[derive(Debug, Deserialize)]
struct TelegramUserResponse {
    username: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}
#[derive(Debug, Deserialize)]
struct TelegramSendMessageResponse {
    #[serde(rename = "message_id")]
    _message_id: i64,
}
fn build_curl_config(url: &str, body: &str, max_time_seconds: u32) -> String {
    // curl config file format을 stdin으로 전달하면 JSON body와 token URL을 shell quoting 없이 안전하게 넘길 수 있다.
    format!(
        "silent\nshow-error\nconnect-timeout = {connect_timeout}\nmax-time = {max_time}\nrequest = \"POST\"\nheader = \"Content-Type: application/json\"\nurl = \"{url}\"\ndata = \"{body}\"\n",
        connect_timeout = CURL_CONNECT_TIMEOUT_SECONDS,
        max_time = max_time_seconds,
        url = escape_curl_config_value(url),
        body = escape_curl_config_value(body),
    )
}
fn escape_curl_config_value(value: &str) -> String {
    // curl config의 quoted value 안에서 의미를 갖는 문자만 escaping한다.
    value
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
#[cfg(test)]
mod tests {
    use super::{TelegramApiEnvelope, TelegramSendMessageResponse, build_curl_config};

    #[test]
    fn telegram_envelope_parses_success_payload() {
        // 성공 envelope는 result가 실제 payload이므로 adapter helper가 이 값을 application DTO로 매핑한다.
        let envelope = serde_json::from_str::<TelegramApiEnvelope<Vec<serde_json::Value>>>(
            r#"{"ok":true,"result":[{"update_id":1}]}"#,
        )
        .expect("telegram envelope should parse");

        assert!(envelope.ok);
        assert_eq!(envelope.result.expect("result should exist").len(), 1);
    }
    #[test]
    fn telegram_envelope_parses_error_payload() {
        // 실패 envelope는 HTTP 성공과 별개로 description을 error context에 실어야 한다.
        let envelope = serde_json::from_str::<TelegramApiEnvelope<serde_json::Value>>(
            r#"{"ok":false,"description":"Bad Request: chat not found"}"#,
        )
        .expect("telegram error envelope should parse");

        assert!(!envelope.ok);
        assert_eq!(
            envelope.description.expect("description should exist"),
            "Bad Request: chat not found"
        );
    }
    #[test]
    fn send_message_response_maps_message_id_field() {
        // sendMessage result는 adapter가 버리지만, Telegram response shape가 바뀌면 deserialize 단계에서 잡힌다.
        let envelope = serde_json::from_str::<TelegramApiEnvelope<TelegramSendMessageResponse>>(
            r#"{"ok":true,"result":{"message_id":42}}"#,
        )
        .expect("telegram sendMessage envelope should parse");

        assert!(envelope.ok);
        assert!(envelope.result.is_some());
    }
    #[test]
    fn build_curl_config_supports_stdin_delivery() {
        // config는 curl stdin으로 전달되므로 URL, POST method, JSON data가 한 문자열에 모두 들어가야 한다.
        let config = build_curl_config(
            "https://api.telegram.org/bot123456:secret/getUpdates",
            r#"{"offset":1}"#,
            45,
        );

        assert!(config.contains("bot123456:secret"));
        assert!(config.contains("request = \"POST\""));
    }
}
