use crate::application::service::planning::validate_planning_draft_name;
use askama::Template;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::RngCore;

// browser page와 JSON mutation이 함께 쓰는 double-submit token cookie 이름이다.
const CSRF_COOKIE_NAME: &str = "akra_admin_csrf";

// caller가 URI escaping을 기억한다고 믿지 않고, operator notice를 담은 redirect target을 여기서 조립한다.
pub(super) fn notice_location(path: &str, notice: &str) -> String {
    format!("{path}?notice={}", encode_uri_component(notice))
}

// notice에는 path, 공백, service error text가 들어갈 수 있으므로 conservative component escaping을 적용한다.
pub(super) fn encode_uri_component(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/*
 * local admin session에 CSRF token이 있는지 보장하고, template/JSON bootstrap이 쓸 값을 반환한다.
 * admin API에는 server-side session store가 없으므로 page, HTMX call, JSON handler 모두 per-form server state 대신
 * 같은 cookie-backed double-submit token을 사용한다.
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
        // Lax는 local admin navigation/form에 충분하고, cross-site subrequest에 cookie가 붙는 범위를 줄인다.
        .same_site(SameSite::Lax)
        // JSON/HTMX client가 cookie 값을 x-csrf-token으로 mirror해야 하므로 JavaScript가 읽을 수 있어야 한다.
        .http_only(false)
        .build();
    (jar.add(cookie), token)
}

// classic HTML form mutation을 shared cookie 값과 비교해 검증한다.
pub(super) fn verify_form_csrf(
    jar: &CookieJar,
    token: &str,
) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or(StatusCode::FORBIDDEN)?;
    // 이 local-only admin surface는 direct comparison을 사용한다. internet-facing 배포라면 timing-safe 검토가 필요하다.
    if cookie_value == token {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

// cookie token을 x-csrf-token header로 반사하는 JSON/HTMX mutation을 검증한다.
pub(super) fn verify_header_csrf(
    jar: &CookieJar,
    headers: &HeaderMap,
) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or(StatusCode::FORBIDDEN)?;
    // missing, non-UTF8, mismatch header 값은 모두 forbidden으로 접어 실패 세부를 노출하지 않는다.
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

pub(super) fn verify_draft_name_path(draft_name: &str) -> std::result::Result<(), StatusCode> {
    validate_planning_draft_name(draft_name).map_err(|_| StatusCode::BAD_REQUEST)
}

// cookie, hidden form field, request header를 통해 echo해도 되는 opaque token을 만든다.
fn new_csrf_token() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// full-page response 대신 fragment rendering을 선택하기 전에 정확한 HTMX request marker를 확인한다.
pub(super) fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        // HTMX의 표준 lowercase true만 fragment path로 들어간다. 다른 변형은 full page로 fallback한다.
        .is_some_and(|value| value == HeaderValue::from_static("true"))
}

// full admin page를 render하고, 새로 발급된 CSRF cookie 같은 CookieJar mutation을 함께 전달한다.
pub(super) fn render_html<T: Template>(
    jar: CookieJar,
    template: T,
) -> std::result::Result<Response, StatusCode> {
    let body = template.render().map_err(internal_server_error)?;
    Ok((jar, Html(body)).into_response())
}

// HTMX fragment는 cookie mutation 없이 render한다. caller는 page bootstrap이 CSRF를 설정한 뒤 이 helper를 쓴다.
pub(super) fn render_fragment<T: Template>(
    template: T,
) -> std::result::Result<Response, StatusCode> {
    let body = template.render().map_err(internal_server_error)?;
    Ok(Html(body).into_response())
}

// 공통 inbound error adapter다. 자세한 diagnostic은 server-side에 남기고 외부에는 500 response만 노출한다.
pub(super) fn internal_server_error(error: impl Into<anyhow::Error>) -> StatusCode {
    eprintln!("admin server error: {:#}", error.into());
    StatusCode::INTERNAL_SERVER_ERROR
}
