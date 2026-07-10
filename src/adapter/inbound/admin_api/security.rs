use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use askama::Template;
use axum::extract::{Form, State};
use axum::http::header::{AUTHORIZATION, HOST, ORIGIN, REFERER};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use time::Duration as CookieDuration;

use super::AdminAppState;

pub(super) const ADMIN_CAPABILITY_TOKEN_ENV: &str = "AKRA_ADMIN_TOKEN";
pub(super) const ADMIN_TOKEN_HEADER: &str = "x-akra-admin-token";
const ADMIN_SESSION_COOKIE: &str = "akra_admin_session";
const CAPABILITY_TOKEN_HEX_LENGTH: usize = 64;
const ADMIN_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const ADMIN_SESSION_ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(8 * 60 * 60);

#[derive(Clone)]
pub(super) struct AdminSecurityConfig {
    capability_digest: [u8; 32],
    session: Arc<Mutex<Option<AdminSession>>>,
    expected_host: Arc<str>,
    expected_port: u16,
}

struct AdminSession {
    digest: [u8; 32],
    issued_at: Instant,
    last_seen_at: Instant,
}

pub(super) struct AdminSecurityBootstrap {
    pub(super) config: AdminSecurityConfig,
    pub(super) generated_capability_token: Option<String>,
}

pub(super) enum AdminRequestAuthentication {
    Capability,
    Session(Cookie<'static>),
}

impl AdminRequestAuthentication {
    pub(super) fn refreshed_session_cookie(self) -> Option<Cookie<'static>> {
        match self {
            Self::Capability => None,
            Self::Session(cookie) => Some(cookie),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalAuthority {
    host: String,
    port: u16,
}

#[derive(Debug, Deserialize)]
pub(super) struct AdminLoginForm {
    token: String,
}

#[derive(Template)]
#[template(path = "admin/login.html")]
struct AdminLoginTemplate {
    error_message: Option<&'static str>,
}

impl AdminSecurityBootstrap {
    pub(super) fn from_env(expected_port: u16) -> Result<Self> {
        match std::env::var(ADMIN_CAPABILITY_TOKEN_ENV) {
            Ok(token) => {
                validate_capability_token(&token)?;
                Ok(Self {
                    config: AdminSecurityConfig::new(&token, expected_port)?,
                    generated_capability_token: None,
                })
            }
            Err(std::env::VarError::NotPresent) => {
                let token = generate_capability_token()?;
                Ok(Self {
                    config: AdminSecurityConfig::new(&token, expected_port)?,
                    generated_capability_token: Some(token),
                })
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                bail!("{ADMIN_CAPABILITY_TOKEN_ENV} must be valid UTF-8")
            }
        }
    }
}

impl AdminSecurityConfig {
    fn new(capability_token: &str, expected_port: u16) -> Result<Self> {
        let origin_nonce = generate_capability_token()?;
        let expected_host = format!(
            "akra-{}.{}.localhost",
            &origin_nonce[..CAPABILITY_TOKEN_HEX_LENGTH / 2],
            &origin_nonce[CAPABILITY_TOKEN_HEX_LENGTH / 2..]
        );
        Self::new_with_host(capability_token, expected_host, expected_port)
    }

    fn new_with_host(
        capability_token: &str,
        expected_host: impl Into<Arc<str>>,
        expected_port: u16,
    ) -> Result<Self> {
        validate_capability_token(capability_token)?;
        Ok(Self {
            capability_digest: token_digest(capability_token),
            session: Arc::new(Mutex::new(None)),
            expected_host: expected_host.into(),
            expected_port,
        })
    }

    #[cfg(test)]
    pub(super) fn for_test(capability_token: &str, expected_port: u16) -> Self {
        Self::new_with_host(capability_token, "akra-test.localhost", expected_port)
            .expect("test admin capability token should be valid")
    }

    pub(super) fn expected_port(&self) -> u16 {
        self.expected_port
    }

    pub(super) fn expected_host(&self) -> &str {
        self.expected_host.as_ref()
    }

    pub(super) fn public_origin(&self) -> String {
        format!("http://{}:{}", self.expected_host, self.expected_port)
    }

    pub(super) fn authenticate_request(
        &self,
        headers: &HeaderMap,
    ) -> Option<AdminRequestAuthentication> {
        self.authenticate_request_at(headers, Instant::now())
    }

    fn authenticate_request_at(
        &self,
        headers: &HeaderMap,
        now: Instant,
    ) -> Option<AdminRequestAuthentication> {
        if header_capability_tokens(headers)
            .into_iter()
            .any(|candidate| self.matches_capability_token(candidate))
        {
            return Some(AdminRequestAuthentication::Capability);
        }
        let session_token = CookieJar::from_headers(headers)
            .get(ADMIN_SESSION_COOKIE)?
            .value()
            .to_string();
        self.refresh_session_cookie_at(&session_token, now)
            .map(AdminRequestAuthentication::Session)
    }

    #[cfg(test)]
    pub(super) fn request_is_authenticated(&self, headers: &HeaderMap) -> bool {
        self.authenticate_request(headers).is_some()
    }

    pub(super) fn matches_capability_token(&self, candidate: &str) -> bool {
        constant_time_digest_eq(&self.capability_digest, &token_digest(candidate))
    }

    fn refresh_session_cookie_at(&self, candidate: &str, now: Instant) -> Option<Cookie<'static>> {
        let candidate_digest = token_digest(candidate);
        let mut session = self.session.lock().ok()?;
        let active = session.as_mut()?;
        let absolute_age = now.saturating_duration_since(active.issued_at);
        if absolute_age >= ADMIN_SESSION_ABSOLUTE_TIMEOUT
            || now.saturating_duration_since(active.last_seen_at) >= ADMIN_SESSION_IDLE_TIMEOUT
        {
            *session = None;
            return None;
        }
        if !constant_time_digest_eq(&active.digest, &candidate_digest) {
            return None;
        }
        active.last_seen_at = now;
        let remaining_absolute = ADMIN_SESSION_ABSOLUTE_TIMEOUT.saturating_sub(absolute_age);
        let refreshed_lifetime = ADMIN_SESSION_IDLE_TIMEOUT.min(remaining_absolute);
        if refreshed_lifetime.is_zero() {
            *session = None;
            return None;
        }
        Some(Self::session_cookie(
            candidate.to_string(),
            refreshed_lifetime,
        ))
    }

    fn issue_session_cookie(&self) -> Result<Cookie<'static>> {
        let token = generate_capability_token()?;
        let now = Instant::now();
        *self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("admin session lock was poisoned"))? =
            Some(AdminSession {
                digest: token_digest(&token),
                issued_at: now,
                last_seen_at: now,
            });
        Ok(Self::session_cookie(token, ADMIN_SESSION_IDLE_TIMEOUT))
    }

    fn session_cookie(token: String, lifetime: Duration) -> Cookie<'static> {
        Cookie::build((ADMIN_SESSION_COOKIE, token))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Strict)
            .max_age(CookieDuration::seconds(
                i64::try_from(lifetime.as_secs()).expect("admin session timeout should fit i64"),
            ))
            .build()
    }

    fn clear_session(&self) {
        if let Ok(mut session) = self.session.lock() {
            *session = None;
        }
    }

    fn removal_cookie(&self) -> Cookie<'static> {
        Cookie::build((ADMIN_SESSION_COOKIE, String::new()))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Strict)
            .max_age(CookieDuration::ZERO)
            .build()
    }
}

pub(super) async fn login_page() -> std::result::Result<Response, StatusCode> {
    render_login_page(StatusCode::OK, None)
}

pub(super) async fn login_submit(
    State(state): State<AdminAppState>,
    jar: CookieJar,
    Form(form): Form<AdminLoginForm>,
) -> std::result::Result<Response, StatusCode> {
    if !state.security.matches_capability_token(&form.token) {
        return render_login_page(
            StatusCode::UNAUTHORIZED,
            Some("Authentication failed. Check the capability token and try again."),
        );
    }
    let session_cookie = state
        .security
        .issue_session_cookie()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((jar.add(session_cookie), Redirect::to("/admin")).into_response())
}

pub(super) async fn logout_submit(State(state): State<AdminAppState>, jar: CookieJar) -> Response {
    state.security.clear_session();
    (
        jar.remove(state.security.removal_cookie()),
        Redirect::to("/admin/login"),
    )
        .into_response()
}

fn render_login_page(
    status: StatusCode,
    error_message: Option<&'static str>,
) -> std::result::Result<Response, StatusCode> {
    let body = AdminLoginTemplate { error_message }
        .render()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut response = (status, Html(body)).into_response();
    // Chromium serializes form POST origins as `null` under `no-referrer`.
    // Keep the relaxed policy scoped to this token exchange page so the
    // loopback Origin/Referer guard can authenticate a real browser submit.
    response.headers_mut().insert(
        "referrer-policy",
        axum::http::HeaderValue::from_static("same-origin"),
    );
    Ok(response)
}

pub(super) fn verify_local_admin_request(
    headers: &HeaderMap,
    security: &AdminSecurityConfig,
) -> std::result::Result<(), StatusCode> {
    let host = required_header_text(headers, HOST)?;
    let request_authority =
        parse_local_authority(host, security.expected_host(), security.expected_port())?;

    for header_name in [ORIGIN, REFERER] {
        let Some(value) = optional_header_text(headers, &header_name)? else {
            continue;
        };
        let uri = value.parse::<Uri>().map_err(|_| StatusCode::BAD_REQUEST)?;
        if uri.scheme_str() != Some("http") {
            return Err(StatusCode::FORBIDDEN);
        }
        let authority = uri.authority().ok_or(StatusCode::BAD_REQUEST)?;
        let source_authority = parse_local_authority(
            authority.as_str(),
            security.expected_host(),
            security.expected_port(),
        )?;
        if source_authority != request_authority {
            return Err(StatusCode::FORBIDDEN);
        }
        if header_name == ORIGIN
            && uri
                .path_and_query()
                .is_some_and(|path| path.as_str() != "/")
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    Ok(())
}

fn validate_capability_token(token: &str) -> Result<()> {
    if token.len() != CAPABILITY_TOKEN_HEX_LENGTH
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!(
            "{ADMIN_CAPABILITY_TOKEN_ENV} must be exactly {CAPABILITY_TOKEN_HEX_LENGTH} hexadecimal characters (generate one with `openssl rand -hex 32`)"
        );
    }
    Ok(())
}

fn generate_capability_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .context("failed to obtain OS randomness for admin capability token")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

pub(super) fn constant_time_token_eq(expected: &str, candidate: &str) -> bool {
    constant_time_digest_eq(&token_digest(expected), &token_digest(candidate))
}

fn constant_time_digest_eq(expected: &[u8; 32], candidate: &[u8; 32]) -> bool {
    let mut difference = 0_u8;
    for (expected_byte, candidate_byte) in expected.iter().zip(candidate) {
        difference |= expected_byte ^ candidate_byte;
    }
    std::hint::black_box(difference) == 0
}

fn header_capability_tokens(headers: &HeaderMap) -> Vec<&str> {
    let mut candidates = Vec::with_capacity(2);
    if let Some(value) = headers
        .get(ADMIN_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        candidates.push(value);
    }
    if let Some(value) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
    {
        candidates.push(value);
    }
    candidates
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && !token.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        Some(token)
    } else {
        None
    }
}

fn required_header_text(
    headers: &HeaderMap,
    name: axum::http::header::HeaderName,
) -> std::result::Result<&str, StatusCode> {
    optional_header_text(headers, &name)?.ok_or(StatusCode::BAD_REQUEST)
}

fn optional_header_text<'a>(
    headers: &'a HeaderMap,
    name: &axum::http::header::HeaderName,
) -> std::result::Result<Option<&'a str>, StatusCode> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let value = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?.trim();
    if value.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(Some(value))
}

fn parse_local_authority(
    value: &str,
    expected_host: &str,
    expected_port: u16,
) -> std::result::Result<LocalAuthority, StatusCode> {
    let authority = Authority::from_str(value).map_err(|_| StatusCode::BAD_REQUEST)?;
    if authority.as_str().contains('@') {
        return Err(StatusCode::BAD_REQUEST);
    }
    // HTTP authorities may omit the default port. Non-default admin ports still
    // require an explicit, exact port and therefore remain fail-closed.
    let port = authority.port_u16().unwrap_or(80);
    if port != expected_port {
        return Err(StatusCode::FORBIDDEN);
    }

    let raw_host = authority.host();
    let host = raw_host.to_ascii_lowercase();
    if host != expected_host || raw_host.ends_with('.') {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(LocalAuthority { host, port })
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn capability_token_requires_a_256_bit_hex_representation() {
        assert!(validate_capability_token(TEST_TOKEN).is_ok());
        for invalid in [
            "short",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde ",
        ] {
            assert!(validate_capability_token(invalid).is_err());
        }
    }

    #[test]
    fn production_admin_origin_uses_an_unpredictable_exact_localhost_name() {
        let first = AdminSecurityConfig::new(TEST_TOKEN, 18442)
            .expect("first production security config should build");
        let second = AdminSecurityConfig::new(TEST_TOKEN, 18442)
            .expect("second production security config should build");
        assert_ne!(first.expected_host(), second.expected_host());
        for host in [first.expected_host(), second.expected_host()] {
            let labels = host.split('.').collect::<Vec<_>>();
            assert_eq!(labels.len(), 3);
            assert!(labels[0].starts_with("akra-"));
            assert_eq!(labels[0].len(), 37);
            assert_eq!(labels[1].len(), 32);
            assert!(
                labels[..2]
                    .iter()
                    .flat_map(|label| label.trim_start_matches("akra-").bytes())
                    .all(|byte| byte.is_ascii_hexdigit())
            );
            assert_eq!(&labels[2..], &["localhost"]);
        }
    }

    #[test]
    fn request_auth_accepts_dedicated_header_bearer_and_session_cookie() {
        let security = AdminSecurityConfig::for_test(TEST_TOKEN, 18442);
        let mut headers = HeaderMap::new();
        headers.insert(ADMIN_TOKEN_HEADER, HeaderValue::from_static(TEST_TOKEN));
        assert!(security.request_is_authenticated(&headers));

        headers.clear();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {TEST_TOKEN}"))
                .expect("bearer token should be valid"),
        );
        assert!(security.request_is_authenticated(&headers));

        headers.clear();
        let session_cookie = security
            .issue_session_cookie()
            .expect("test session should be issued");
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(&format!(
                "{ADMIN_SESSION_COOKIE}={}",
                session_cookie.value()
            ))
            .expect("session cookie should be valid"),
        );
        assert!(security.request_is_authenticated(&headers));

        security.clear_session();
        assert!(!security.request_is_authenticated(&headers));
    }

    #[test]
    fn browser_session_expires_server_side_and_login_rotates_the_bearer() {
        let security = AdminSecurityConfig::for_test(TEST_TOKEN, 18442);
        let first = security
            .issue_session_cookie()
            .expect("first test session should be issued");
        let second = security
            .issue_session_cookie()
            .expect("second test session should be issued");
        assert_ne!(first.value(), second.value());

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(&format!("{ADMIN_SESSION_COOKIE}={}", first.value()))
                .expect("first session header should be valid"),
        );
        assert!(!security.request_is_authenticated(&headers));
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(&format!("{ADMIN_SESSION_COOKIE}={}", second.value()))
                .expect("second session header should be valid"),
        );
        assert!(security.request_is_authenticated(&headers));

        let idle_expired_at = security
            .session
            .lock()
            .expect("test session lock should remain healthy")
            .as_ref()
            .expect("active session should exist")
            .last_seen_at
            .checked_add(ADMIN_SESSION_IDLE_TIMEOUT + Duration::from_secs(1))
            .expect("idle timeout should fit the monotonic clock");
        assert!(
            security
                .authenticate_request_at(&headers, idle_expired_at)
                .is_none()
        );

        let absolute_security = AdminSecurityConfig::for_test(TEST_TOKEN, 18442);
        let absolute_cookie = absolute_security
            .issue_session_cookie()
            .expect("absolute-timeout session should issue");
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(&format!(
                "{ADMIN_SESSION_COOKIE}={}",
                absolute_cookie.value()
            ))
            .expect("absolute-timeout cookie header should be valid"),
        );
        let absolute_issued_at = absolute_security
            .session
            .lock()
            .expect("absolute-timeout session lock should remain healthy")
            .as_ref()
            .expect("active session should exist")
            .issued_at;
        let near_absolute_expiry = absolute_issued_at
            .checked_add(ADMIN_SESSION_ABSOLUTE_TIMEOUT - Duration::from_secs(10))
            .expect("absolute timeout should fit the monotonic clock");
        absolute_security
            .session
            .lock()
            .expect("absolute-timeout session lock should remain healthy")
            .as_mut()
            .expect("active session should exist")
            .last_seen_at = near_absolute_expiry;
        let refreshed = absolute_security
            .authenticate_request_at(&headers, near_absolute_expiry)
            .and_then(AdminRequestAuthentication::refreshed_session_cookie)
            .expect("session should refresh inside the absolute lifetime");
        let max_age = refreshed
            .max_age()
            .expect("refreshed session should have a bounded max age")
            .whole_seconds();
        assert!((1..=10).contains(&max_age));

        let absolute_expired_at = absolute_issued_at
            .checked_add(ADMIN_SESSION_ABSOLUTE_TIMEOUT + Duration::from_secs(1))
            .expect("absolute timeout should fit the monotonic clock");
        assert!(
            absolute_security
                .authenticate_request_at(&headers, absolute_expired_at)
                .is_none()
        );
    }

    #[test]
    fn local_authority_requires_host_and_exact_origin_port() {
        let security = AdminSecurityConfig::for_test(TEST_TOKEN, 18442);
        let mut headers = HeaderMap::new();
        assert_eq!(
            verify_local_admin_request(&headers, &security),
            Err(StatusCode::BAD_REQUEST)
        );

        headers.insert(HOST, HeaderValue::from_static("akra-test.localhost:18442"));
        assert!(verify_local_admin_request(&headers, &security).is_ok());

        headers.insert(
            ORIGIN,
            HeaderValue::from_static("http://akra-test.localhost:19000"),
        );
        assert_eq!(
            verify_local_admin_request(&headers, &security),
            Err(StatusCode::FORBIDDEN)
        );

        headers.insert(
            ORIGIN,
            HeaderValue::from_static("http://akra-test.localhost:18442"),
        );
        assert!(verify_local_admin_request(&headers, &security).is_ok());

        headers.clear();
        for host in [
            "127.0.0.1:18442",
            "localhost:18442",
            "akra-test.localhost.:18442",
        ] {
            headers.insert(
                HOST,
                HeaderValue::from_str(host).expect("test host should be valid"),
            );
            assert_eq!(
                verify_local_admin_request(&headers, &security),
                Err(StatusCode::FORBIDDEN)
            );
        }

        headers.insert(HOST, HeaderValue::from_static("akra-test.localhost:18442"));
        headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:18442"));
        assert_eq!(
            verify_local_admin_request(&headers, &security),
            Err(StatusCode::FORBIDDEN)
        );
    }
}
