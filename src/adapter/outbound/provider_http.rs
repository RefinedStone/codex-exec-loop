//! Gateway-only authenticated reads. Tokens never become a public request or diagnostic.
use crate::application::port::outbound::provider_credentials_port::ProviderAccountVerifierPort;
use crate::application::port::outbound::provider_work_port::ProviderWorkApiPort;
use crate::domain::provider_credentials::*;
use crate::domain::provider_work::*;
use base64::Engine;
use serde_json::{Value, json};
use std::io::Read;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

mod jira;
mod linear;

const RESPONSE_LIMIT: usize = 8 << 20;
const VERSION_RESPONSE_LIMIT: usize = 64 << 10;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const VERSION_TIMEOUT: Duration = Duration::from_secs(8);
const LINEAR_ENDPOINT: &str = "https://api.linear.app/graphql";

/// Internal only: callers cannot supply a URL, HTTP method, query or authorization through RPC.
struct HttpRequest {
    url: String,
    query: Vec<(String, String)>,
    body: Option<Value>,
    limit: usize,
    timeout: Duration,
}
struct JiraRead<'a> {
    relative: &'a str,
    query: Vec<(String, String)>,
    timeout: Duration,
    limit: usize,
}
impl<'a> JiraRead<'a> {
    fn normal(relative: &'a str, query: Vec<(String, String)>) -> Self {
        Self {
            relative,
            query,
            timeout: REQUEST_TIMEOUT,
            limit: RESPONSE_LIMIT,
        }
    }
    fn version(relative: &'a str) -> Self {
        Self {
            relative,
            query: vec![("fields".to_owned(), "updated".to_owned())],
            timeout: VERSION_TIMEOUT,
            limit: VERSION_RESPONSE_LIMIT,
        }
    }
}
trait HttpTransport: Send + Sync {
    fn read(
        &self,
        request: HttpRequest,
        authorization: &str,
        token: &SecretToken,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<Value>;
}
struct NativeHttpTransport;
impl HttpTransport for NativeHttpTransport {
    fn read(
        &self,
        request: HttpRequest,
        authorization: &str,
        token: &SecretToken,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<Value> {
        ensure_live(cancelled, deadline)?;
        let timeout = request
            .timeout
            .min(deadline.saturating_duration_since(Instant::now()));
        if timeout.is_zero() {
            return Err(ProviderCredentialError::Cancelled);
        }
        // Construction, send and drop all occur on the caller's blocking worker, never a Tokio async task.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(8)))
            .connection_verbose(false)
            .tls_sslkeylogfile(false)
            .tls_backend_rustls()
            .user_agent(concat!("Akra-Gateway/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        let mut authorization = reqwest::header::HeaderValue::from_bytes(authorization.as_bytes())
            .map_err(|_| ProviderCredentialError::InvalidRequest)?;
        authorization.set_sensitive(true);
        let builder = if let Some(body) = request.body {
            client.post(&request.url).json(&body)
        } else {
            client.get(&request.url)
        }
        .query(&request.query)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .header(reqwest::header::ACCEPT, "application/json");
        let mut response = builder
            .send()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        ensure_live(cancelled, deadline)?;
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ProviderCredentialError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ProviderCredentialError::Unavailable);
        }
        if response
            .content_length()
            .is_some_and(|size| size > request.limit as u64)
        {
            return Err(ProviderCredentialError::TooLarge);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        let mut chunk = Zeroizing::new([0u8; 8192]);
        loop {
            ensure_live(cancelled, deadline)?;
            let size = response
                .read(chunk.as_mut())
                .map_err(|_| ProviderCredentialError::Unavailable)?;
            if size == 0 {
                break;
            }
            if bytes.len().saturating_add(size) > request.limit {
                return Err(ProviderCredentialError::TooLarge);
            }
            bytes.extend_from_slice(&chunk[..size]);
        }
        ensure_live(cancelled, deadline)?;
        parse_response(&bytes, token, request.limit)
    }
}
fn ensure_live(cancelled: &AtomicBool, deadline: Instant) -> CredentialResult<()> {
    if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
        return Err(ProviderCredentialError::Cancelled);
    }
    Ok(())
}
fn parse_response(bytes: &[u8], token: &SecretToken, limit: usize) -> CredentialResult<Value> {
    if bytes.len() > limit {
        return Err(ProviderCredentialError::TooLarge);
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| ProviderCredentialError::Corrupt)?;
    // Reflected credentials, including JSON-escaped values, never reach a renderer or metadata ledger.
    if contains_secret(&value, token.as_str()) {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(value)
}
fn contains_secret(value: &Value, secret: &str) -> bool {
    match value {
        Value::String(text) => text.contains(secret),
        Value::Array(values) => values.iter().any(|value| contains_secret(value, secret)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(secret) || contains_secret(value, secret)),
        _ => false,
    }
}
pub(crate) struct ProviderHttpAdapter {
    transport: Arc<dyn HttpTransport>,
}
impl Default for ProviderHttpAdapter {
    fn default() -> Self {
        Self {
            transport: Arc::new(NativeHttpTransport),
        }
    }
}
impl ProviderHttpAdapter {
    fn jira_read(
        &self,
        config: &ConnectionConfiguration,
        token: &SecretToken,
        read: JiraRead<'_>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<Value> {
        let config = config.normalized(Provider::Jira)?;
        let site = config
            .site_url
            .as_deref()
            .ok_or(ProviderCredentialError::InvalidRequest)?;
        let version = if config.deployment == Some(JiraDeployment::Cloud) {
            "3"
        } else {
            "2"
        };
        let authorization = jira_authorization(&config, token)?;
        let value = self.transport.read(
            HttpRequest {
                url: format!("{site}/rest/api/{version}/{}", read.relative),
                query: read.query,
                body: None,
                limit: read.limit,
                timeout: read.timeout,
            },
            &authorization,
            token,
            cancelled,
            deadline,
        )?;
        // The native transport checks this too; keeping it at the adapter boundary covers injected transports.
        if contains_secret(&value, token.as_str())
            || contains_secret(&value, &authorization)
            || authorization
                .strip_prefix("Basic ")
                .is_some_and(|encoded| contains_secret(&value, encoded))
        {
            return Err(ProviderCredentialError::Corrupt);
        }
        Ok(value)
    }
    fn linear_read(
        &self,
        token: &SecretToken,
        query: &'static str,
        variables: Value,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<Value> {
        let value = self.transport.read(
            HttpRequest {
                url: LINEAR_ENDPOINT.to_owned(),
                query: vec![],
                body: Some(json!({"query":query,"variables":variables})),
                limit: RESPONSE_LIMIT,
                timeout: REQUEST_TIMEOUT,
            },
            token.as_str(),
            token,
            cancelled,
            deadline,
        )?;
        if contains_secret(&value, token.as_str()) {
            return Err(ProviderCredentialError::Corrupt);
        }
        if let Some(errors) = value.get("errors")
            && errors.as_array().is_none_or(|errors| !errors.is_empty())
        {
            return Err(ProviderCredentialError::Unavailable);
        }
        value
            .get("data")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or(ProviderCredentialError::Corrupt)
    }
}
fn jira_authorization(
    config: &ConnectionConfiguration,
    token: &SecretToken,
) -> CredentialResult<Zeroizing<String>> {
    match config.deployment {
        Some(JiraDeployment::Cloud) => {
            let email = config
                .email
                .as_deref()
                .ok_or(ProviderCredentialError::InvalidRequest)?;
            let mut plain =
                Zeroizing::new(Vec::with_capacity(email.len() + 1 + token.expose().len()));
            plain.extend_from_slice(email.as_bytes());
            plain.push(b':');
            plain.extend_from_slice(token.expose());
            let encoded =
                Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(plain.as_slice()));
            let mut header = Zeroizing::new(String::from("Basic "));
            header.push_str(&encoded);
            Ok(header)
        }
        Some(JiraDeployment::Server) => {
            let mut header = Zeroizing::new(String::from("Bearer "));
            header.push_str(token.as_str());
            Ok(header)
        }
        None => Err(ProviderCredentialError::InvalidRequest),
    }
}
impl ProviderAccountVerifierPort for ProviderHttpAdapter {
    fn verify(
        &self,
        request: &ConnectRequest,
        token: &SecretToken,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<ConnectionMetadata> {
        let configuration = request.configuration.normalized(request.provider)?;
        let (account_id, account_name) = match request.provider {
            Provider::Jira => {
                let value = self.jira_read(
                    &configuration,
                    token,
                    JiraRead::normal("myself", vec![]),
                    cancelled,
                    deadline,
                )?;
                let id = value
                    .get("accountId")
                    .or_else(|| value.get("key"))
                    .or_else(|| value.get("name"))
                    .ok_or(ProviderCredentialError::Corrupt)?;
                let name = value
                    .get("displayName")
                    .or_else(|| value.get("name"))
                    .ok_or(ProviderCredentialError::Corrupt)?;
                (bounded_text(id, 256)?, bounded_text(name, 1024)?)
            }
            Provider::Linear => {
                let value =
                    self.linear_read(token, linear::VIEWER_QUERY, json!({}), cancelled, deadline)?;
                let viewer = &value["viewer"];
                object(viewer)?;
                (
                    linear::id(&viewer["id"])?,
                    bounded_text(&viewer["name"], 1024)?,
                )
            }
        };
        ensure_live(cancelled, deadline)?;
        let metadata = ConnectionMetadata {
            configuration,
            account_id,
            account_name,
        };
        metadata.validate(request.provider)?;
        Ok(metadata)
    }
}
impl ProviderWorkApiPort for ProviderHttpAdapter {
    fn jira_summary(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
    ) -> CredentialResult<JiraSummary> {
        let config = &metadata.configuration;
        let relative = if config.deployment == Some(JiraDeployment::Cloud) {
            "search/jql"
        } else {
            "search"
        };
        let query = vec![
            (
                "jql".to_owned(),
                "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC"
                    .to_owned(),
            ),
            ("maxResults".to_owned(), ISSUE_LIMIT.to_string()),
            (
                "fields".to_owned(),
                "summary,status,issuetype,priority,updated".to_owned(),
            ),
        ];
        let value = self.jira_read(
            config,
            token,
            JiraRead::normal(relative, query),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )?;
        jira::summary(
            &value,
            config
                .site_url
                .as_deref()
                .ok_or(ProviderCredentialError::Corrupt)?,
        )
    }
    fn jira_issue_prompt(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
        key: &str,
    ) -> CredentialResult<String> {
        if !valid_jira_key(key) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let value = self.jira_read(
            &metadata.configuration,
            token,
            JiraRead::normal(
                &format!("issue/{key}"),
                vec![(
                    "fields".to_owned(),
                    "summary,status,issuetype,priority,updated,description,comment".to_owned(),
                )],
            ),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )?;
        if value["key"].as_str() != Some(key) {
            return Err(ProviderCredentialError::Corrupt);
        }
        jira::prompt(
            &value,
            metadata
                .configuration
                .site_url
                .as_deref()
                .ok_or(ProviderCredentialError::Corrupt)?,
        )
    }
    fn jira_current_version(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
        key: &str,
    ) -> CredentialResult<String> {
        if !valid_jira_key(key) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let value = self.jira_read(
            &metadata.configuration,
            token,
            JiraRead::version(&format!("issue/{key}")),
            &AtomicBool::new(false),
            Instant::now() + VERSION_TIMEOUT,
        )?;
        let updated = value["fields"]["updated"]
            .as_str()
            .ok_or(ProviderCredentialError::Corrupt)?;
        normalized_timestamp(updated)
    }
    fn linear_assigned_issues(&self, token: &SecretToken) -> CredentialResult<Vec<LinearIssue>> {
        let value = self.linear_read(
            token,
            linear::ISSUES_QUERY,
            json!({}),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )?;
        linear::assigned_issues(&value)
    }
    fn linear_issue_prompt(&self, token: &SecretToken, id: &str) -> CredentialResult<String> {
        if !valid_linear_id(id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let value = self.linear_read(
            token,
            linear::DETAIL_QUERY,
            json!({"id":id}),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )?;
        if value["issue"]["id"].as_str() != Some(id) {
            return Err(ProviderCredentialError::Corrupt);
        }
        linear::prompt(&value["issue"])
    }
}
fn object(value: &Value) -> CredentialResult<&serde_json::Map<String, Value>> {
    value.as_object().ok_or(ProviderCredentialError::Corrupt)
}
fn bounded_text(value: &Value, maximum: usize) -> CredentialResult<String> {
    let value = value.as_str().ok_or(ProviderCredentialError::Corrupt)?;
    if value.trim().is_empty()
        || value.chars().count() > maximum
        || value.chars().any(|ch| ch == '\0')
    {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(value.to_owned())
}
fn nullable_text(value: &Value, maximum: usize) -> CredentialResult<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let value = value.as_str().ok_or(ProviderCredentialError::Corrupt)?;
    if value.chars().count() > maximum || value.contains('\0') {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(Some(value.to_owned()))
}
fn clip(value: &str, limit: usize) -> String {
    const NOTICE: &str = "\n[Content omitted]";
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut result: String = value
        .chars()
        .take(limit.saturating_sub(NOTICE.chars().count()))
        .collect();
    result.push_str(NOTICE);
    result
}

#[cfg(test)]
mod tests;
