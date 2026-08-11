use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

const BUNDLED_ASSET_CACHE_CONTROL: &str = "private, no-cache";

#[derive(Clone, Copy, Debug)]
pub(super) struct BundledAssetCachePolicy;

macro_rules! bundled_asset {
    ($bytes:ident, $etag:ident, $path:literal) => {
        const $bytes: &[u8] = include_bytes!($path);
        static $etag: LazyLock<HeaderValue> = LazyLock::new(|| bundled_asset_etag($bytes));
    };
}

bundled_asset!(
    AKRA_OPERATIONS_STUDIO_V3,
    AKRA_OPERATIONS_STUDIO_V3_ETAG,
    "../../../../assets/admin/graphics/akra-operations-studio-v3.png"
);
bundled_asset!(
    GAMEBALJEONGUK_ATLAS_64X96,
    GAMEBALJEONGUK_ATLAS_64X96_ETAG,
    "../../../../assets/admin/graphics/gamebaljeonguk_atlas_64x96.png"
);
bundled_asset!(
    GAMEBALJEONGUK_ATLAS_128X192,
    GAMEBALJEONGUK_ATLAS_128X192_ETAG,
    "../../../../assets/admin/graphics/gamebaljeonguk_atlas_128x192.png"
);
bundled_asset!(
    PR_APPROVER_ATLAS_64X96,
    PR_APPROVER_ATLAS_64X96_ETAG,
    "../../../../assets/admin/graphics/pr-approver-atlas-64x96.png"
);
bundled_asset!(
    PR_APPROVER_ATLAS_128X192,
    PR_APPROVER_ATLAS_128X192_ETAG,
    "../../../../assets/admin/graphics/pr-approver-atlas-128x192.png"
);

bundled_asset!(
    AKRA_DIORAMA_JS,
    AKRA_DIORAMA_JS_ETAG,
    "../../../../assets/admin/game/akra-diorama.js"
);
bundled_asset!(
    ADMIN_SHELL_JS,
    ADMIN_SHELL_JS_ETAG,
    "../../../../assets/admin/scripts/admin-shell.js"
);
bundled_asset!(
    AKRA_DASHBOARD_JS,
    AKRA_DASHBOARD_JS_ETAG,
    "../../../../assets/admin/scripts/akra-dashboard.js"
);
bundled_asset!(
    GALMURI11_WOFF2,
    GALMURI11_WOFF2_ETAG,
    "../../../../assets/admin/fonts/Galmuri11.woff2"
);
bundled_asset!(
    GALMURI11_BOLD_WOFF2,
    GALMURI11_BOLD_WOFF2_ETAG,
    "../../../../assets/admin/fonts/Galmuri11-Bold.woff2"
);

fn bundled_asset_etag(bytes: &[u8]) -> HeaderValue {
    let digest = Sha256::digest(bytes);
    let value = format!("\"sha256-{digest:x}\"");
    HeaderValue::from_bytes(value.as_bytes()).expect("SHA-256 ETag must be a valid header value")
}

fn if_none_match_matches(headers: &HeaderMap, etag: &HeaderValue) -> bool {
    let Ok(expected) = etag.to_str() else {
        return false;
    };

    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .any(|candidate| {
            candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == expected
        })
}

fn bundled_asset_response(
    request_headers: &HeaderMap,
    content_type: &'static str,
    bytes: &'static [u8],
    etag: &'static LazyLock<HeaderValue>,
) -> Response {
    let etag = LazyLock::force(etag);
    let mut response = if if_none_match_matches(request_headers, etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(BUNDLED_ASSET_CACHE_CONTROL),
    );
    response.headers_mut().insert(header::ETAG, etag.clone());
    response.extensions_mut().insert(BundledAssetCachePolicy);
    response
}

pub(super) async fn admin_graphic_asset(
    request_headers: HeaderMap,
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let (bytes, etag) = match asset_name.as_str() {
        "akra-operations-studio-v3.png" => {
            (AKRA_OPERATIONS_STUDIO_V3, &AKRA_OPERATIONS_STUDIO_V3_ETAG)
        }
        "gamebaljeonguk_atlas_64x96.png" => {
            (GAMEBALJEONGUK_ATLAS_64X96, &GAMEBALJEONGUK_ATLAS_64X96_ETAG)
        }
        "gamebaljeonguk_atlas_128x192.png" => (
            GAMEBALJEONGUK_ATLAS_128X192,
            &GAMEBALJEONGUK_ATLAS_128X192_ETAG,
        ),
        "pr-approver-atlas-64x96.png" => (PR_APPROVER_ATLAS_64X96, &PR_APPROVER_ATLAS_64X96_ETAG),
        "pr-approver-atlas-128x192.png" => {
            (PR_APPROVER_ATLAS_128X192, &PR_APPROVER_ATLAS_128X192_ETAG)
        }
        _ => return Err(StatusCode::NOT_FOUND),
    };

    Ok(bundled_asset_response(
        &request_headers,
        "image/png",
        bytes,
        etag,
    ))
}

pub(super) async fn admin_game_asset(
    request_headers: HeaderMap,
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let (content_type, bytes, etag) = match asset_name.as_str() {
        "akra-diorama.js" => (
            "text/javascript; charset=utf-8",
            AKRA_DIORAMA_JS,
            &AKRA_DIORAMA_JS_ETAG,
        ),
        _ => return Err(StatusCode::NOT_FOUND),
    };

    Ok(bundled_asset_response(
        &request_headers,
        content_type,
        bytes,
        etag,
    ))
}

pub(super) async fn admin_script_asset(
    request_headers: HeaderMap,
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let (bytes, etag) = match asset_name.as_str() {
        "admin-shell.js" => (ADMIN_SHELL_JS, &ADMIN_SHELL_JS_ETAG),
        "akra-dashboard.js" => (AKRA_DASHBOARD_JS, &AKRA_DASHBOARD_JS_ETAG),
        _ => return Err(StatusCode::NOT_FOUND),
    };

    Ok(bundled_asset_response(
        &request_headers,
        "text/javascript; charset=utf-8",
        bytes,
        etag,
    ))
}

pub(super) async fn admin_font_asset(
    request_headers: HeaderMap,
    Path(asset_name): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let (bytes, etag) = match asset_name.as_str() {
        "Galmuri11.woff2" => (GALMURI11_WOFF2, &GALMURI11_WOFF2_ETAG),
        "Galmuri11-Bold.woff2" => (GALMURI11_BOLD_WOFF2, &GALMURI11_BOLD_WOFF2_ETAG),
        _ => return Err(StatusCode::NOT_FOUND),
    };

    Ok(bundled_asset_response(
        &request_headers,
        "font/woff2",
        bytes,
        etag,
    ))
}
