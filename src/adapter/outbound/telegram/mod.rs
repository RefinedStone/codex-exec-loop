/*
Telegram Bot API outbound adapter다.

application 계층은 polling과 sendMessage를 `TelegramBotPort`로만 호출한다. 이 파일은 그 port 요청을
curl 기반 HTTPS POST로 변환하고, Telegram의 `{ ok, result, description }` envelope를 내부 DTO에서
application DTO로 다시 매핑한다. HTTP client crate를 추가하지 않고 curl config를 stdin으로 넘기는 이유는
token이 argv/process listing에 직접 노출되는 면을 줄이고, 기존 운영 환경의 curl 의존성만 사용하기 위해서다.
*/
use crate::application::port::outbound::telegram_bot_port::{
    TELEGRAM_LONG_POLL_TRANSPORT_MARGIN_SECONDS, TelegramBotIdentity, TelegramBotPort,
    TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest, TelegramUpdate,
};
use crate::subprocess;
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde::Serialize;
#[cfg(all(test, unix))]
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
const CURL_CONNECT_TIMEOUT_SECONDS: &str = "10";
const DEFAULT_ALLOWED_UPDATES: [&str; 1] = ["message"];
const MAX_TELEGRAM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TELEGRAM_ERROR_BYTES: usize = 64 * 1024;

pub struct CurlTelegramBotAdapter {
    // 테스트와 production이 같은 request path를 쓰되, curl binary와 API base URL은 adapter 상태로 둔다.
    curl_path: String,
    curl_resolution_error: Option<String>,
    api_base_url: String,
    token: String,
    request_wait_timeout_override: Option<std::time::Duration>,
}

impl CurlTelegramBotAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new_for_workspace(token, &cwd)
    }

    fn new_for_workspace(token: impl Into<String>, workspace: &std::path::Path) -> Self {
        Self::new_for_workspace_with_curl_resolution(
            token,
            crate::trusted_executable::resolve_native_from_current_path("curl", workspace),
        )
    }

    #[cfg(all(test, unix))]
    fn new_for_workspace_with_path(
        token: impl Into<String>,
        workspace: &std::path::Path,
        path: &OsStr,
    ) -> Self {
        Self::new_for_workspace_with_curl_resolution(
            token,
            crate::trusted_executable::resolve_native_from_path("curl", path, workspace),
        )
    }

    fn new_for_workspace_with_curl_resolution(
        token: impl Into<String>,
        resolution: Result<PathBuf>,
    ) -> Self {
        let (curl_path, curl_resolution_error) = match resolution {
            Ok(path) => (path.display().to_string(), None),
            Err(error) => (
                unresolved_curl_executable_path().display().to_string(),
                Some(error.to_string()),
            ),
        };
        Self {
            curl_path,
            curl_resolution_error,
            api_base_url: TELEGRAM_API_BASE_URL.to_string(),
            token: token.into(),
            request_wait_timeout_override: None,
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
        if let Some(error) = &self.curl_resolution_error {
            bail!("trusted curl executable could not be pinned for Telegram: {error}")
        }
        let url = format!("{}/bot{}/{}", self.api_base_url, self.token, method_name);
        let json_body = serde_json::to_string(body).context("failed to serialize request body")?;
        let max_time_seconds = u32::from(timeout_seconds).saturating_add(
            u32::try_from(TELEGRAM_LONG_POLL_TRANSPORT_MARGIN_SECONDS)
                .expect("Telegram transport margin must fit u32"),
        );
        let wait_timeout = self.request_wait_timeout_override.unwrap_or_else(|| {
            std::time::Duration::from_secs(u64::from(max_time_seconds).saturating_add(1))
        });

        let mut command = Command::new(&self.curl_path);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        crate::trusted_executable::configure_credential_command_environment(
            &mut command,
            &cwd,
            false,
        )?;
        command
            // `-q` must be argv[1]. Otherwise curl loads ~/.curlrc first, where
            // trace/proxy/output directives could disclose the token-bearing URL.
            .args(["-q", "--config", "-"]);
        let config = build_curl_config(url.as_str(), json_body.as_str(), max_time_seconds);
        let output = subprocess::command_output_with_input_timeout_and_limits(
            &mut command,
            &format!("curl -q --config - Telegram {method_name}"),
            config.as_bytes(),
            wait_timeout,
            MAX_TELEGRAM_RESPONSE_BYTES,
            MAX_TELEGRAM_ERROR_BYTES,
        )
        .with_context(|| format!("failed to wait for curl during Telegram {method_name}"))?;

        if !output.status.success() {
            bail!("telegram {method_name} request failed ({})", output.status);
        }
        let body = String::from_utf8(output.stdout)
            .with_context(|| format!("telegram {method_name} response was not valid utf-8"))?;
        let envelope = serde_json::from_str::<TelegramApiEnvelope<TResponse>>(&body)
            .with_context(|| format!("failed to parse telegram {method_name} response"))?;
        if !envelope.ok {
            // Telegram은 HTTP 200에서도 ok=false를 반환할 수 있으므로 envelope 레벨 오류를 별도로 올린다.
            return Err(anyhow!(
                "telegram {method_name} rejected the request (response description redacted)"
            ));
        }
        envelope.result.ok_or_else(|| {
            anyhow!("telegram {method_name} returned ok=true without a result payload")
        })
    }
}

#[cfg(unix)]
fn unresolved_curl_executable_path() -> PathBuf {
    PathBuf::from("/__akra_unresolved_telegram_curl_executable__")
}

#[cfg(windows)]
fn unresolved_curl_executable_path() -> PathBuf {
    PathBuf::from(r"C:\__akra_unresolved_telegram_curl_executable__.exe")
}

impl TelegramBotPort for CurlTelegramBotAdapter {
    fn get_me(&self) -> Result<TelegramBotIdentity> {
        let response: TelegramGetMeResponse =
            self.execute_json_request("getMe", &serde_json::json!({}), 15)?;
        if response.id == 0 {
            bail!("telegram getMe returned a non-positive bot id");
        }
        Ok(TelegramBotIdentity {
            bot_id: response.id,
        })
    }

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
        let sender_user_id = value.from.as_ref().map(|user| user.id);
        Self {
            message_id: value.message_id,
            chat_id: value.chat.id,
            text: value.text,
            sender_user_id,
            // operator-facing sender label은 username을 우선하고, 없으면 Telegram profile 이름으로 fallback한다.
            sender_display_name: value.from.and_then(sender_display_name),
        }
    }
}
#[derive(Debug, Deserialize)]
struct TelegramChatResponse {
    id: i64,
}
#[derive(Debug, Deserialize)]
struct TelegramUserResponse {
    id: i64,
    username: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}
fn sender_display_name(user: TelegramUserResponse) -> Option<String> {
    [user.username, user.first_name, user.last_name]
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}
#[derive(Debug, Deserialize)]
struct TelegramSendMessageResponse {
    #[serde(rename = "message_id")]
    _message_id: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramGetMeResponse {
    id: u64,
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
    #[cfg(unix)]
    use super::CurlTelegramBotAdapter;
    use super::{
        TelegramApiEnvelope, TelegramChatResponse, TelegramGetMeResponse, TelegramMessageResponse,
        TelegramSendMessageResponse, TelegramUpdateResponse, TelegramUserResponse,
        build_curl_config, escape_curl_config_value,
    };
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

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
        assert!(envelope.result.is_none());
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
    fn get_me_response_requires_a_numeric_bot_identity() {
        let envelope = serde_json::from_str::<TelegramApiEnvelope<TelegramGetMeResponse>>(
            r#"{"ok":true,"result":{"id":123456}}"#,
        )
        .expect("Telegram getMe response should parse");

        assert_eq!(
            envelope.result.expect("getMe result should exist").id,
            123_456
        );
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

    #[test]
    fn curl_config_escaping_preserves_json_and_config_boundaries() {
        // curl config의 quoted value 안에서는 JSON quote와 개행이 config syntax로 새지 않아야 한다.
        assert_eq!(escape_curl_config_value("a\\b\"c\n\r"), "a\\\\b\\\"c\\n\\r");

        let config = build_curl_config(
            "https://telegram.example/bot\"token\\x/sendMessage",
            "{\"text\":\"line 1\nline 2\"}",
            30,
        );

        assert!(
            config.contains("url = \"https://telegram.example/bot\\\"token\\\\x/sendMessage\"")
        );
        assert!(config.contains("data = \"{\\\"text\\\":\\\"line 1\\nline 2\\\"}\""));
    }

    #[test]
    fn update_response_maps_message_identity_and_sender_fallbacks() {
        // 공백 username은 표시 가능한 이름이 아니므로 first_name, last_name 순서로 fallback해야 한다.
        let update = TelegramUpdateResponse {
            update_id: 7,
            message: Some(TelegramMessageResponse {
                message_id: 42,
                chat: TelegramChatResponse { id: -100 },
                text: Some("/status".to_string()),
                from: Some(TelegramUserResponse {
                    id: 9001,
                    username: Some("   ".to_string()),
                    first_name: Some("Akra".to_string()),
                    last_name: Some("Operator".to_string()),
                }),
            }),
        };

        let mapped: crate::application::port::outbound::telegram_bot_port::TelegramUpdate =
            update.into();
        let message = mapped.message.expect("message should map");

        assert_eq!(mapped.update_id, 7);
        assert_eq!(message.message_id, 42);
        assert_eq!(message.chat_id, -100);
        assert_eq!(message.sender_user_id, Some(9001));
        assert_eq!(message.text.as_deref(), Some("/status"));
        assert_eq!(message.sender_display_name.as_deref(), Some("Akra"));
    }

    #[test]
    fn update_response_preserves_non_message_updates_for_cursor_progress() {
        // Telegram update는 message가 없어도 update_id로 cursor를 전진시켜 중복 polling을 피해야 한다.
        let mapped: crate::application::port::outbound::telegram_bot_port::TelegramUpdate =
            TelegramUpdateResponse {
                update_id: 99,
                message: None,
            }
            .into();

        assert_eq!(mapped.update_id, 99);
        assert!(mapped.message.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn execute_json_request_times_out_hung_curl_process() {
        let root = unique_temp_dir("telegram-curl-timeout");
        fs::create_dir_all(&root).expect("fixture root should be created");
        let script = write_executable_script(
            &root,
            "fake-curl",
            r#"#!/bin/sh
set -eu
cat >/dev/null
sleep 2
"#,
        );
        let adapter = CurlTelegramBotAdapter {
            curl_path: script.display().to_string(),
            curl_resolution_error: None,
            api_base_url: "https://api.test".to_string(),
            token: "secret-token".to_string(),
            request_wait_timeout_override: Some(Duration::from_millis(50)),
        };

        let error = adapter
            .execute_json_request::<_, serde_json::Value>(
                "getUpdates",
                &serde_json::json!({"offset": 1}),
                0,
            )
            .expect_err("hung curl should time out");
        let error_chain = error
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | ");

        assert!(error_chain.contains("timed out after"));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn execute_json_request_redacts_token_from_curl_and_api_failures() {
        let root = unique_temp_dir("telegram-redacted-failures");
        fs::create_dir_all(&root).expect("fixture root should be created");
        let secret = "123456:private-token";
        let script = write_executable_script(
            &root,
            "fake-curl-failure",
            &format!(
                r#"#!/bin/sh
set -eu
cat >/dev/null
printf 'curl failed for https://api.test/bot{secret}/getUpdates' >&2
exit 22
"#
            ),
        );
        let mut adapter = CurlTelegramBotAdapter {
            curl_path: script.display().to_string(),
            curl_resolution_error: None,
            api_base_url: "https://api.test".to_string(),
            token: secret.to_string(),
            request_wait_timeout_override: Some(Duration::from_secs(1)),
        };

        let curl_error = adapter
            .execute_json_request::<_, serde_json::Value>(
                "getUpdates",
                &serde_json::json!({"offset": 1}),
                0,
            )
            .expect_err("curl failure should surface");
        assert!(curl_error.to_string().contains("exit status: 22"));
        assert!(!curl_error.to_string().contains(secret));
        assert!(!curl_error.to_string().contains("/bot"));

        let api_script = write_executable_script(
            &root,
            "fake-curl-api-error",
            &format!(
                r#"#!/bin/sh
set -eu
cat >/dev/null
printf '{{"ok":false,"description":"bad token {secret}"}}'
"#
            ),
        );
        adapter.curl_path = api_script.display().to_string();
        let api_error = adapter
            .execute_json_request::<_, serde_json::Value>(
                "getUpdates",
                &serde_json::json!({"offset": 1}),
                0,
            )
            .expect_err("API rejection should surface");
        assert!(api_error.to_string().contains("description redacted"));
        assert!(!api_error.to_string().contains(secret));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn execute_json_request_disables_the_user_curl_config_before_stdin_config() {
        let root = unique_temp_dir("telegram-curl-argv");
        fs::create_dir_all(&root).expect("fixture root should be created");
        let argv_path = root.join("argv.txt");
        let script = write_executable_script(
            &root,
            "fake-curl-argv",
            &format!(
                r#"#!/bin/sh
set -eu
printf '%s\n' "$@" > '{}'
cat >/dev/null
printf '{{"ok":true,"result":{{}}}}'
"#,
                argv_path.display()
            ),
        );
        let adapter = CurlTelegramBotAdapter {
            curl_path: script.display().to_string(),
            curl_resolution_error: None,
            api_base_url: "https://api.test".to_string(),
            token: "secret-token".to_string(),
            request_wait_timeout_override: Some(Duration::from_secs(1)),
        };

        let _: serde_json::Value = adapter
            .execute_json_request("getUpdates", &serde_json::json!({"offset": 1}), 0)
            .expect("fake curl should return a valid envelope");

        assert_eq!(
            fs::read_to_string(&argv_path).expect("curl argv should be captured"),
            "-q\n--config\n-\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn repository_path_cannot_replace_telegram_curl() {
        let repo_root = unique_temp_dir("telegram-hostile-path-repo");
        let repo_bin = repo_root.join("bin");
        fs::create_dir_all(repo_root.join(".git")).expect("repository marker should be created");
        fs::create_dir_all(&repo_bin).expect("repository bin should be created");
        let marker_path = repo_root.join("hostile-curl-ran");
        write_executable_script(
            &repo_bin,
            "curl",
            &format!(
                "#!/bin/sh\nset -eu\n: > '{}'\nprintf '{{\"ok\":true,\"result\":{{}}}}'\n",
                marker_path.display()
            ),
        );
        let original_path = std::env::var_os("PATH").expect("PATH should be available");
        let hostile_path = std::env::join_paths(
            std::iter::once(repo_bin).chain(std::env::split_paths(&original_path)),
        )
        .expect("hostile PATH should join");
        let adapter = CurlTelegramBotAdapter::new_for_workspace_with_path(
            "secret-token",
            &repo_root,
            &hostile_path,
        );
        let error = adapter
            .execute_json_request::<_, serde_json::Value>(
                "getUpdates",
                &serde_json::json!({"offset": 1}),
                0,
            )
            .expect_err("repository curl must make Telegram requests fail closed");

        assert!(error.to_string().contains("trusted curl executable"));
        assert!(!marker_path.exists(), "repository curl was executed");
        let _ = fs::remove_dir_all(&repo_root);
    }

    #[cfg(unix)]
    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"))
    }

    #[cfg(unix)]
    fn write_executable_script(root: &Path, name: &str, body: &str) -> PathBuf {
        let script_path = root.join(name);
        fs::write(&script_path, body).expect("script fixture should be written");
        let mut permissions = fs::metadata(&script_path)
            .expect("script metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions)
            .expect("script fixture should be executable");
        script_path
    }
}
