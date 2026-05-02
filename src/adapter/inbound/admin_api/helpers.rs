// 학습 주석: admin pages는 Askama template을 렌더링해 HTML response나 HTMX fragment로 돌려줍니다.
use askama::Template;
// 학습 주석: helper는 Axum handler들이 공통으로 쓰는 header/status/response 타입을 모아 inbound
// adapter의 HTTP 계약을 한곳에서 낮춥니다.
use axum::http::{HeaderMap, HeaderValue, StatusCode};
// 학습 주석: full page와 fragment 모두 최종적으로 Axum `Response`가 되어 handler에서 그대로 반환됩니다.
use axum::response::{Html, IntoResponse, Response};
// 학습 주석: CookieJar는 admin browser session에 CSRF token을 보관하는 통로입니다.
use axum_extra::extract::CookieJar;
// 학습 주석: cookie builder와 SameSite policy를 직접 지정해 admin form submit과 fetch/HTMX 요청의
// CSRF 기준을 일관되게 만듭니다.
use axum_extra::extract::cookie::{Cookie, SameSite};
// 학습 주석: notice redirect query는 사람이 읽는 문장을 그대로 담으므로 percent-encoding이 필요합니다.
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
// 학습 주석: CSRF token은 request마다 예측 불가능해야 하므로 OS-backed thread RNG로 바이트를 채웁니다.
use rand::RngCore;

// 학습 주석: admin UI와 JSON API가 같은 cookie 이름을 공유합니다. pages.rs의 hidden form token과
// api.rs의 x-csrf-token header가 모두 이 cookie 값과 비교됩니다.
const CSRF_COOKIE_NAME: &str = "akra_admin_csrf";

// 학습 주석: mutation handler가 성공 후 redirect할 때 notice를 query string에 붙입니다. 직접 문자열
// 결합을 이 helper로 모아 notice escaping 누락과 path별 구현 차이를 줄입니다.
pub(super) fn notice_location(path: &str, notice: &str) -> String {
    format!("{path}?notice={}", encode_uri_component(notice))
}

// 학습 주석: HTML form/redirect notice에 넣는 임의 문자열을 URI component로 인코딩합니다. admin notice는
// 공백, 슬래시, 한글, 에러 문구를 포함할 수 있어 전체 non-alphanumeric set을 보수적으로 escape합니다.
pub(super) fn encode_uri_component(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

// 학습 주석: page handler 진입 시 CSRF cookie를 보장하고 template에 넣을 token 문자열을 돌려줍니다.
// 이미 cookie가 있으면 재발급하지 않아 열린 admin form들과 HTMX 요청이 같은 token을 계속 사용할 수 있습니다.
pub(super) fn ensure_csrf_cookie(jar: CookieJar) -> (CookieJar, String) {
    if let Some(existing) = jar
        // 학습 주석: cookie value를 String으로 소유해 반환합니다. CookieJar 자체도 response에 다시
        // 실어야 하므로 `(jar, token)` 형태로 caller에게 넘깁니다.
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
    {
        return (jar, existing);
    }

    // 학습 주석: cookie가 없을 때만 새 token을 만듭니다. 이 token은 hidden form field와 response cookie에
    // 같은 값으로 들어가 double-submit CSRF 패턴을 구성합니다.
    let token = new_csrf_token();
    // 학습 주석: path "/"는 admin 하위 모든 endpoint에서 같은 CSRF cookie를 읽게 합니다. http_only=false는
    // HTMX/JS가 header token을 구성해야 하는 API 요청에서도 값을 읽을 수 있게 하기 위한 선택입니다.
    let cookie = Cookie::build((CSRF_COOKIE_NAME, token.clone()))
        .path("/")
        // 학습 주석: Lax는 일반 admin navigation/form 흐름은 허용하되 cross-site subrequest 전송을 줄입니다.
        .same_site(SameSite::Lax)
        .http_only(false)
        .build();
    (jar.add(cookie), token)
}

// 학습 주석: classic HTML form mutation은 hidden `csrf_token` field를 cookie value와 비교합니다. 실패는
// 인증/권한 문제가 아니라 request 무결성 문제이므로 handler가 403으로 즉시 응답하게 합니다.
pub(super) fn verify_form_csrf(
    // 학습 주석: jar는 browser가 보낸 cookie source입니다. server-side session store 없이 double-submit
    // token만 비교하므로 cookie가 없으면 form도 신뢰할 수 없습니다.
    jar: &CookieJar,
    // 학습 주석: token은 form body에서 파싱된 hidden field 값입니다.
    token: &str,
) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        // 학습 주석: cookie가 없으면 token 비교 자체가 불가능하므로 forbidden으로 접습니다.
        .ok_or(StatusCode::FORBIDDEN)?;
    // 학습 주석: 현재는 constant-time compare가 아니라 단순 비교입니다. 로컬 admin surface의 실용성과
    // 단순성을 우선하지만, public internet 노출 전에는 timing-safe compare를 고려할 지점입니다.
    if cookie_value == token {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

// 학습 주석: JSON/admin API mutation은 hidden form field가 없으므로 `x-csrf-token` header를 cookie와
// 비교합니다. `api.rs`의 write handlers가 이 helper를 통해 page form과 같은 CSRF policy를 공유합니다.
pub(super) fn verify_header_csrf(
    jar: &CookieJar,
    // 학습 주석: headers는 Axum request headers입니다. HTMX/API client가 cookie token을 이 header로
    // 반사해 보내야 mutation이 허용됩니다.
    headers: &HeaderMap,
) -> std::result::Result<(), StatusCode> {
    let cookie_value = jar
        .get(CSRF_COOKIE_NAME)
        .map(|cookie| cookie.value().to_string())
        .ok_or(StatusCode::FORBIDDEN)?;
    // 학습 주석: invalid UTF-8 header도 missing token과 같은 forbidden으로 취급합니다. handler는
    // 보안 실패와 parsing 실패를 구분해 자세한 정보를 노출하지 않습니다.
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

// 학습 주석: 16 random bytes를 32자리 hex 문자열로 바꿔 CSRF token으로 씁니다. URL/form/header에
// 안전한 alphabet만 쓰므로 별도 escaping 없이 hidden field와 header 값으로 전달할 수 있습니다.
fn new_csrf_token() -> String {
    let mut bytes = [0_u8; 16];
    // 학습 주석: thread_rng가 token entropy를 채웁니다. deterministic test hook을 두지 않은 이유는
    // helper 자체보다 verify 경로가 중요한 계약이고 token 값은 opaque하기 때문입니다.
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// 학습 주석: HTMX 요청 여부를 `HX-Request: true` header로 감지합니다. pages.rs의 draft save/promote
// 경로는 이 값에 따라 전체 page redirect 대신 status fragment만 렌더링합니다.
pub(super) fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        // 학습 주석: header value를 static true와 직접 비교해 `"True"`나 `"1"` 같은 변형은 일반
        // full-page 요청처럼 처리합니다. HTMX가 보내는 표준 값만 fragment 경로로 들어갑니다.
        .is_some_and(|value| value == HeaderValue::from_static("true"))
}

// 학습 주석: full page render helper입니다. CookieJar를 response tuple에 포함해 ensure_csrf_cookie에서
// 추가된 cookie가 실제 Set-Cookie header로 내려가게 합니다.
pub(super) fn render_html<T: Template>(
    // 학습 주석: jar는 page response와 함께 반환할 cookie mutations를 담습니다.
    jar: CookieJar,
    // 학습 주석: template은 이미 handler에서 service 결과와 csrf token을 담아 만든 Askama view입니다.
    template: T,
) -> std::result::Result<Response, StatusCode> {
    // 학습 주석: template render 실패는 server bug나 view data mismatch입니다. public response는 500만
    // 돌리고, 자세한 오류는 `internal_server_error`가 stderr에 남깁니다.
    let body = template.render().map_err(internal_server_error)?;
    Ok((jar, Html(body)).into_response())
}

// 학습 주석: HTMX fragment render helper입니다. fragment response는 새 cookie를 설정하지 않으므로
// CookieJar를 받지 않고, caller가 status panel/body fragment만 교체할 수 있는 HTML 조각을 반환합니다.
pub(super) fn render_fragment<T: Template>(
    template: T,
) -> std::result::Result<Response, StatusCode> {
    let body = template.render().map_err(internal_server_error)?;
    Ok(Html(body).into_response())
}

// 학습 주석: inbound admin layer의 공통 error adapter입니다. application/service error detail은 stderr에
// 남기되 browser/API에는 500 status만 반환해 내부 경로와 stack 정보를 노출하지 않습니다.
pub(super) fn internal_server_error(error: impl Into<anyhow::Error>) -> StatusCode {
    eprintln!("admin server error: {:#}", error.into());
    StatusCode::INTERNAL_SERVER_ERROR
}
