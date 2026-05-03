use askama::Template;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::RngCore;

// Shared double-submit token name for both browser pages and JSON mutations.
const CSRF_COOKIE_NAME: &str = "akra_admin_csrf";

// Build redirect targets that carry operator notices without trusting callers to remember URI escaping.
pub(super) fn notice_location(path: &str, notice: &str) -> String {
    format!("{path}?notice={}", encode_uri_component(notice))
}

// Conservative component escaping for notices that may include paths, spaces, or service error text.
pub(super) fn encode_uri_component(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/*
 * Ensure the local admin session has a CSRF token and return the value for templates or JSON bootstrap.
 * The admin API has no server-side session store, so pages, HTMX calls, and JSON handlers all use the
 * same cookie-backed double-submit token instead of maintaining per-form server state.
 */
pub(super) fn ensure_csrf_cookie(jar: CookieJar) -> (CookieJar, String) {
    if let Some(existing) = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
    {
        return (jar, existing);
    }

    let token = new_csrf_token();
    let cookie = Cookie::build((CSRF_COOKIE_NAME, token.clone()))
        .path("/")
        // Lax fits local admin navigation/forms while reducing cross-site subrequest attachment.
        .same_site(SameSite::Lax)
        // JSON/HTMX clients mirror the cookie into x-csrf-token, so JavaScript must be able to read it.
        .http_only(false)
        .build();
    (jar.add(cookie), token)
}

// Verify classic HTML form mutations against the shared cookie value.
pub(super) fn verify_form_csrf(
    jar: &CookieJar,
    token: &str,
) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or(StatusCode::FORBIDDEN)?;
    // This local-only admin surface uses a direct comparison; internet-facing deployment would need timing-safe review.
    if cookie_value == token {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

// Verify JSON/HTMX mutations that reflect the cookie token through x-csrf-token.
pub(super) fn verify_header_csrf(
    jar: &CookieJar,
    headers: &HeaderMap,
) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or(StatusCode::FORBIDDEN)?;
    // Missing, non-UTF8, and mismatched header values all collapse to forbidden to avoid leaking failure detail.
    let header_value = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;
    if cookie_value == header_value {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

// Generate an opaque token that is safe to echo through cookie, hidden form field, and request header.
fn new_csrf_token() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Detect the exact HTMX request marker before choosing fragment rendering over full-page responses.
pub(super) fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        // Only HTMX's standard lowercase true enters the fragment path; variants fall back to full pages.
        .is_some_and(|value| value == HeaderValue::from_static("true"))
}

// Render full admin pages and propagate CookieJar mutations such as a freshly issued CSRF cookie.
pub(super) fn render_html<T: Template>(
    jar: CookieJar,
    template: T,
) -> std::result::Result<Response, StatusCode> {
    let body = template.render().map_err(internal_server_error)?;
    Ok((jar, Html(body)).into_response())
}

// Render HTMX fragments without cookie mutations; callers use this after page bootstrap established CSRF.
pub(super) fn render_fragment<T: Template>(
    template: T,
) -> std::result::Result<Response, StatusCode> {
    let body = template.render().map_err(internal_server_error)?;
    Ok(Html(body).into_response())
}

// Common inbound error adapter: keep detailed diagnostics server-side and expose only a 500 response.
pub(super) fn internal_server_error(error: impl Into<anyhow::Error>) -> StatusCode {
    eprintln!("admin server error: {:#}", error.into());
    StatusCode::INTERNAL_SERVER_ERROR
}
