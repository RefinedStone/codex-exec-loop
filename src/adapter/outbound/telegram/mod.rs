use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde::Serialize;

use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};

const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
const CURL_CONNECT_TIMEOUT_SECONDS: &str = "10";
const DEFAULT_ALLOWED_UPDATES: [&str; 1] = ["message"];

pub struct CurlTelegramBotAdapter {
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
    format!(
        "silent\nshow-error\nconnect-timeout = {connect_timeout}\nmax-time = {max_time}\nrequest = \"POST\"\nheader = \"Content-Type: application/json\"\nurl = \"{url}\"\ndata = \"{body}\"\n",
        connect_timeout = CURL_CONNECT_TIMEOUT_SECONDS,
        max_time = max_time_seconds,
        url = escape_curl_config_value(url),
        body = escape_curl_config_value(body),
    )
}

fn escape_curl_config_value(value: &str) -> String {
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
        let envelope = serde_json::from_str::<TelegramApiEnvelope<Vec<serde_json::Value>>>(
            r#"{"ok":true,"result":[{"update_id":1}]}"#,
        )
        .expect("telegram envelope should parse");

        assert!(envelope.ok);
        assert_eq!(envelope.result.expect("result should exist").len(), 1);
    }

    #[test]
    fn telegram_envelope_parses_error_payload() {
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
        let envelope = serde_json::from_str::<TelegramApiEnvelope<TelegramSendMessageResponse>>(
            r#"{"ok":true,"result":{"message_id":42}}"#,
        )
        .expect("telegram sendMessage envelope should parse");

        assert!(envelope.ok);
        assert!(envelope.result.is_some());
    }

    #[test]
    fn build_curl_config_supports_stdin_delivery() {
        let config = build_curl_config(
            "https://api.telegram.org/bot123456:secret/getUpdates",
            r#"{"offset":1}"#,
            45,
        );

        assert!(config.contains("bot123456:secret"));
        assert!(config.contains("request = \"POST\""));
    }
}
