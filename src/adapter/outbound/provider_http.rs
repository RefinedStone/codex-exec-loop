//! Gateway-only typed provider operations. Tokens never become a public request or diagnostic.
use crate::application::port::outbound::provider_credential_mutation_port::ProviderMutationAuthorizationPort;
use crate::application::port::outbound::provider_credentials_port::ProviderAccountVerifierPort;
use crate::application::port::outbound::provider_work_port::ProviderWorkApiPort;
use crate::domain::issue_automation::feedback::JiraFeedbackRejection;
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

mod feedback;
mod jira;
mod jira_draft;
mod jira_source;
mod linear;
mod linear_issue;

const RESPONSE_LIMIT: usize = 8 << 20;
const VERSION_RESPONSE_LIMIT: usize = 64 << 10;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const VERSION_TIMEOUT: Duration = Duration::from_secs(8);
const LINEAR_ENDPOINT: &str = "https://api.linear.app/graphql";

/// Internal only: callers cannot supply a URL, HTTP method, query or authorization through RPC.
struct HttpRequest {
    url: String,
    method: HttpMethod,
    expected: ExpectedResponse,
    send_authorization: Option<Arc<dyn ProviderMutationAuthorizationPort>>,
    preserve_client_errors: bool,
    report_jira_feedback_rejection: bool,
    query: Vec<(String, String)>,
    body: Option<Value>,
    limit: usize,
    timeout: Duration,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpMethod {
    Get,
    Post,
    Put,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedResponse {
    Json(u16),
    NoContent,
}
enum HttpResponse {
    Json(Value),
    NoContent,
    JiraRejected(JiraFeedbackRejection),
}
impl HttpResponse {
    fn json(self) -> CredentialResult<Value> {
        match self {
            Self::Json(value) => Ok(value),
            Self::NoContent | Self::JiraRejected(_) => Err(ProviderCredentialError::Corrupt),
        }
    }
}

enum JiraCloudRoute<'a> {
    Rest(&'a str),
    TenantInfo,
}
struct JiraCloudRead<'a> {
    route: JiraCloudRoute<'a>,
    query: Vec<(String, String)>,
    body: Option<Value>,
}
impl<'a> JiraCloudRead<'a> {
    fn get_rest(relative: &'a str, query: Vec<(String, String)>) -> Self {
        Self {
            route: JiraCloudRoute::Rest(relative),
            query,
            body: None,
        }
    }
    fn post_rest(relative: &'a str, body: Value) -> Self {
        Self {
            route: JiraCloudRoute::Rest(relative),
            query: vec![],
            body: Some(body),
        }
    }
    fn tenant_info() -> Self {
        Self {
            route: JiraCloudRoute::TenantInfo,
            query: vec![],
            body: None,
        }
    }
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
    ) -> CredentialResult<HttpResponse>;
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
    ) -> CredentialResult<HttpResponse> {
        ensure_live(cancelled, deadline)?;
        if (request.method == HttpMethod::Put
            || matches!(
                request.expected,
                ExpectedResponse::Json(201) | ExpectedResponse::NoContent
            ))
            && request.send_authorization.is_none()
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
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
        let builder = match request.method {
            HttpMethod::Get => client.get(&request.url),
            HttpMethod::Post => client.post(&request.url),
            HttpMethod::Put => client.put(&request.url),
        };
        let builder = if let Some(body) = request.body {
            builder.json(&body)
        } else {
            builder
        }
        .query(&request.query)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .header(reqwest::header::ACCEPT, "application/json");
        // No network request precedes this one-shot check. Keep its owned guard through
        // response parsing, errors and Unknown; redirects and automatic retries are disabled.
        ensure_live(cancelled, deadline)?;
        let _send_guard = request
            .send_authorization
            .as_ref()
            .map(|authorization| authorization.begin_send())
            .transpose()?;
        ensure_live(cancelled, deadline)?;
        let mut response = builder
            .timeout(timeout.min(deadline.saturating_duration_since(Instant::now())))
            .send()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        ensure_live(cancelled, deadline)?;
        let status = response.status();
        if let Some(rejection) = jira_feedback_rejection(
            status.as_u16(),
            request.report_jira_feedback_rejection,
            request.send_authorization.is_some(),
        ) {
            // Preserve only the definitive status, never the potentially sensitive body.
            return Ok(HttpResponse::JiraRejected(rejection));
        }
        check_response_status(
            status.as_u16(),
            request.expected,
            request.preserve_client_errors,
        )?;
        if request.expected == ExpectedResponse::NoContent
            && response.content_length().is_some_and(|size| size != 0)
        {
            return Err(ProviderCredentialError::Corrupt);
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
        parse_expected_response(&bytes, token, request.expected, request.limit)
    }
}
fn jira_feedback_rejection(
    status: u16,
    enabled: bool,
    authorized: bool,
) -> Option<JiraFeedbackRejection> {
    if enabled && authorized {
        JiraFeedbackRejection::from_http_status(status)
    } else {
        None
    }
}

fn check_response_status(
    status: u16,
    expected: ExpectedResponse,
    preserve_client_errors: bool,
) -> CredentialResult<()> {
    if matches!(status, 401 | 403) {
        return Err(ProviderCredentialError::Unauthorized);
    }
    if preserve_client_errors {
        match status {
            400 => return Err(ProviderCredentialError::InvalidRequest),
            404 => return Err(ProviderCredentialError::NotFound),
            _ => {}
        }
    }
    let expected_status = match expected {
        ExpectedResponse::Json(status) => status,
        ExpectedResponse::NoContent => 204,
    };
    if status != expected_status {
        return Err(ProviderCredentialError::Unavailable);
    }
    Ok(())
}
fn parse_expected_response(
    bytes: &[u8],
    token: &SecretToken,
    expected: ExpectedResponse,
    limit: usize,
) -> CredentialResult<HttpResponse> {
    match expected {
        ExpectedResponse::Json(_) => parse_response(bytes, token, limit).map(HttpResponse::Json),
        ExpectedResponse::NoContent if bytes.is_empty() => Ok(HttpResponse::NoContent),
        ExpectedResponse::NoContent => Err(ProviderCredentialError::Corrupt),
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
    fn jira_cloud_read(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
        request: JiraCloudRead<'_>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<Value> {
        let configuration = metadata.configuration.normalized(Provider::Jira)?;
        if configuration.deployment != Some(JiraDeployment::Cloud) {
            return Err(ProviderCredentialError::Unsupported);
        }
        let site = configuration
            .site_url
            .as_deref()
            .ok_or(ProviderCredentialError::InvalidRequest)?;
        let url = match request.route {
            JiraCloudRoute::TenantInfo => format!("{site}/_edge/tenant_info"),
            JiraCloudRoute::Rest(relative) => {
                validate_relative(relative)?;
                format!("{site}/rest/api/3/{relative}")
            }
        };
        let authorization = jira_authorization(&configuration, token)?;
        let response = self
            .transport
            .read(
                HttpRequest {
                    url,
                    method: if request.body.is_some() {
                        HttpMethod::Post
                    } else {
                        HttpMethod::Get
                    },
                    expected: ExpectedResponse::Json(200),
                    send_authorization: None,
                    preserve_client_errors: true,
                    report_jira_feedback_rejection: false,
                    query: request.query,
                    body: request.body,
                    limit: RESPONSE_LIMIT,
                    timeout: REQUEST_TIMEOUT,
                },
                &authorization,
                token,
                cancelled,
                deadline,
            )?
            .json()?;
        reject_reflected_credentials(&response, token, &authorization)?;
        Ok(response)
    }
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
        let value = self
            .transport
            .read(
                HttpRequest {
                    url: format!("{site}/rest/api/{version}/{}", read.relative),
                    method: HttpMethod::Get,
                    expected: ExpectedResponse::Json(200),
                    send_authorization: None,
                    preserve_client_errors: false,
                    report_jira_feedback_rejection: false,
                    query: read.query,
                    body: None,
                    limit: read.limit,
                    timeout: read.timeout,
                },
                &authorization,
                token,
                cancelled,
                deadline,
            )?
            .json()?;
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
        self.linear_graphql(token, query, variables, None, cancelled, deadline)
    }
    /// A mutation passes its one-shot admission; the transport checks it at the sole send.
    fn linear_graphql(
        &self,
        token: &SecretToken,
        query: &'static str,
        variables: Value,
        send_authorization: Option<Arc<dyn ProviderMutationAuthorizationPort>>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<Value> {
        let value = self
            .transport
            .read(
                HttpRequest {
                    url: LINEAR_ENDPOINT.to_owned(),
                    method: HttpMethod::Post,
                    expected: ExpectedResponse::Json(200),
                    send_authorization,
                    preserve_client_errors: false,
                    report_jira_feedback_rejection: false,
                    query: vec![],
                    body: Some(json!({"query":query,"variables":variables})),
                    limit: if query == linear::VERSION_QUERY {
                        VERSION_RESPONSE_LIMIT
                    } else {
                        RESPONSE_LIMIT
                    },
                    timeout: if query == linear::VERSION_QUERY {
                        VERSION_TIMEOUT
                    } else {
                        REQUEST_TIMEOUT
                    },
                },
                token.as_str(),
                token,
                cancelled,
                deadline,
            )?
            .json()?;
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
fn validate_relative(relative: &str) -> CredentialResult<()> {
    if relative.is_empty()
        || relative.len() > 2048
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !relative
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
    {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    Ok(())
}
fn reject_reflected_credentials(
    value: &Value,
    token: &SecretToken,
    authorization: &str,
) -> CredentialResult<()> {
    if contains_secret(value, token.as_str())
        || contains_secret(value, authorization)
        || authorization
            .strip_prefix("Basic ")
            .is_some_and(|encoded| contains_secret(value, encoded))
    {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(())
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
    fn jira_draft_ids(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
        key: &str,
    ) -> CredentialResult<(String, String, String)> {
        if !valid_jira_key(key) || metadata.configuration.deployment != Some(JiraDeployment::Cloud)
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let cancelled = AtomicBool::new(false);
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let tenant = self.jira_cloud_read(
            metadata,
            token,
            JiraCloudRead::tenant_info(),
            &cancelled,
            deadline,
        )?;
        let cloud = bounded_text(&tenant["cloudId"], 64)?;
        let relative = format!("issue/{key}");
        let issue = self.jira_cloud_read(
            metadata,
            token,
            JiraCloudRead::get_rest(&relative, vec![("fields".into(), "project".into())]),
            &cancelled,
            deadline,
        )?;
        if issue["key"].as_str() != Some(key) {
            return Err(ProviderCredentialError::Conflict);
        }
        let project = bounded_text(&issue["fields"]["project"]["id"], 32)?;
        let id = bounded_text(&issue["id"], 32)?;
        if ![&project, &id]
            .iter()
            .all(|v| v.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(ProviderCredentialError::Corrupt);
        }
        Ok((cloud, project, id))
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
    fn linear_current_version(
        &self,
        token: &SecretToken,
        resource_key: &str,
    ) -> CredentialResult<String> {
        use crate::domain::work_context::linear_resource_key_for_url;
        let (workspace, identifier) = resource_key
            .split_once('/')
            .ok_or(ProviderCredentialError::InvalidRequest)?;
        // Never interpret an arbitrary URL or query from the ledger. Validate the key before I/O.
        if !valid_linear_identifier(identifier)
            || linear_resource_key_for_url(&format!(
                "https://linear.app/{workspace}/issue/{identifier}"
            ))
            .as_deref()
                != Some(resource_key)
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let value = self.linear_read(
            token,
            linear::VERSION_QUERY,
            json!({"id":identifier}),
            &AtomicBool::new(false),
            Instant::now() + VERSION_TIMEOUT,
        )?;
        let issue = &value["issue"];
        // Identifiers can overlap across workspaces. A returned URL must match the requested ledger key.
        if issue["identifier"].as_str() != Some(identifier)
            || issue["url"]
                .as_str()
                .and_then(linear_resource_key_for_url)
                .as_deref()
                != Some(resource_key)
        {
            return Err(ProviderCredentialError::Corrupt);
        }
        linear::version(&issue["updatedAt"])
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
