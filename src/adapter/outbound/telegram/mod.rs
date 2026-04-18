use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
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
        let max_time_seconds = u32::from(timeout_seconds).saturating_add(15).to_string();
        let output = Command::new(&self.curl_path)
            .args([
                "--silent",
                "--show-error",
                "--connect-timeout",
                CURL_CONNECT_TIMEOUT_SECONDS,
                "--max-time",
                max_time_seconds.as_str(),
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                json_body.as_str(),
                url.as_str(),
            ])
            .output()
            .with_context(|| format!("failed to invoke curl for Telegram {method_name}"))?;

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
    _message_id: i64,
}

#[cfg(test)]
mod tests {
    use super::TelegramApiEnvelope;

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
}
