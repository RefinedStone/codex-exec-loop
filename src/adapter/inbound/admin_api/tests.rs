use super::helpers::{
    encode_uri_component, ensure_csrf_cookie, internal_server_error, is_htmx_request,
    notice_location, render_fragment, render_html, verify_form_csrf, verify_header_csrf,
};
use super::pages::{draft_mutation_path, extract_file_updates, nav_for_kind};
use super::security::{ADMIN_TOKEN_HEADER, AdminSecurityConfig, verify_local_admin_request};
use super::views::{EditorActionPaths, EditorTemplate};
use super::{
    build_admin_state, build_admin_state_with_debug_harness, build_router, harden_admin_response,
    parse_args, parse_reset_target,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterInboxItem, ReviewCenterRepositoryPort, ReviewCenterThreadProjection,
};
use crate::application::service::planning::admin::{
    PlanningAdminDraftFileView, PlanningAdminValidationView,
};
use crate::application::service::planning::{
    PlanningAdminDraftKind, PlanningAdminFileKey, PlanningAdminSessionView, PlanningResetTarget,
};
use crate::domain::parallel_mode::{
    IntegrationAttestation, IntegrationMethod, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, PrValidationCatchUpState, PrValidationCheckContext,
    PrValidationCommitSha, PrValidationCompletion, PrValidationEvent, PrValidationFinding,
    PrValidationFindingKey, PrValidationFindingSource, PrValidationObservationProjection,
    PrValidationObservedCheck, PrValidationObservedCheckStatus, PrValidationObservedProvider,
    PrValidationObservedProviderLifecycle, PrValidationObservedProviderStatus,
    PrValidationObservedRunStatus, PrValidationObservedWorkflow, PrValidationProviderCompletion,
    PrValidationProviderKey, PrValidationRecord, PrValidationRecordKey,
    PrValidationRemediationCorrelation, PrValidationTarget, PrValidationTargetShaSnapshot,
};
use askama::Template;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::response::Response;
use axum_extra::extract::CookieJar;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_stream::StreamExt;
use tower::ServiceExt;

struct BrokenTemplate;

impl std::fmt::Display for BrokenTemplate {
    fn fmt(&self, _formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Err(std::fmt::Error)
    }
}

impl askama::Template for BrokenTemplate {
    const EXTENSION: Option<&'static str> = Some("html");
    const SIZE_HINT: usize = 0;
    const MIME_TYPE: &'static str = "text/html";

    fn render_into(&self, _writer: &mut (impl std::fmt::Write + ?Sized)) -> askama::Result<()> {
        Err(std::fmt::Error.into())
    }
}

/*
 * admin_api tests는 service 내부가 아니라 inbound HTML/form boundary를 보호한다.
 * pages.rs가 form field를 어떤 application request로 인정하는지, template이 destructive POST 앞에서 어떤 browser guard를
 * 제공하는지 같은 adapter contract를 고정한다. template 파일은 compile-time fixture로 포함해 마크업 변경이 Rust test와
 * 함께 review되게 한다.
 */
const BASE_TEMPLATE: &str = include_str!("../../../../templates/admin/base.html");
const CONTROLS_TEMPLATE: &str = include_str!("../../../../templates/admin/controls.html");
const DIRECTIONS_TEMPLATE: &str = include_str!("../../../../templates/admin/directions.html");
const EDITOR_TEMPLATE: &str = include_str!("../../../../templates/admin/editor.html");
const TASKS_TEMPLATE: &str = include_str!("../../../../templates/admin/tasks.html");
const APP_SERVER_PROMPTS_TEMPLATE: &str =
    include_str!("../../../../templates/admin/app_server_prompts.html");
const DASHBOARD_TEMPLATE: &str = include_str!("../../../../templates/admin/dashboard.html");
const AKRA_DASHBOARD_TEMPLATE: &str =
    include_str!("../../../../templates/admin/akra_dashboard.html");
const AKRA_METRICS_TEMPLATE: &str = include_str!("../../../../templates/admin/akra_metrics.html");
const ADMIN_GRAPHIC_VISUAL_SCRIPT: &str =
    include_str!("../../../../scripts/check_admin_graphic_visual.sh");
const ADMIN_GRAPHIC_CAPTURE_SCRIPT: &str =
    include_str!("../../../../scripts/capture_admin_graphic.mjs");
const GAMEBALJEONGUK_SPRITE_PACK_README: &str =
    include_str!("../../../../templates/admin/resources/gamebaljeonguk_sprite_pack/README.txt");
const GAMEBALJEONGUK_SPRITE_METADATA: &str = include_str!(
    "../../../../templates/admin/resources/gamebaljeonguk_sprite_pack/gamebaljeonguk_sprite_metadata.json"
);
const AKRA_DIORAMA_JS: &str = include_str!("../../../../assets/admin/game/akra-diorama.js");
const AKRA_DIORAMA_TS: &str = include_str!("../../../../assets/admin/game/src/akra-diorama.ts");
const AKRA_AGENT_WORLD_TS: &str = include_str!("../../../../assets/admin/game/src/agent-world.ts");
const AKRA_AGENT_ATLAS_TS: &str = include_str!("../../../../assets/admin/game/src/agent-atlas.ts");
const AKRA_CAMERA_CONTROLLER_TS: &str =
    include_str!("../../../../assets/admin/game/src/camera-controller.ts");
const AKRA_GAME_TYPES_TS: &str = include_str!("../../../../assets/admin/game/src/game-types.ts");
const AKRA_SCENE_CONFIG_TS: &str =
    include_str!("../../../../assets/admin/game/src/scene-config.ts");
const AKRA_SCENE_STORE_TS: &str = include_str!("../../../../assets/admin/game/src/scene-store.ts");
const ADMIN_SHELL_JS: &str = include_str!("../../../../assets/admin/scripts/admin-shell.js");
const AKRA_DASHBOARD_JS: &str = include_str!("../../../../assets/admin/scripts/akra-dashboard.js");
const ADMIN_GAME_PACKAGE_JSON: &str = include_str!("../../../../assets/admin/game/package.json");
const ADMIN_GAME_VITE_CONFIG: &str = include_str!("../../../../assets/admin/game/vite.config.ts");
const ADMIN_GAME_PROMOTE_BUILD: &str =
    include_str!("../../../../assets/admin/game/scripts/promote-build.mjs");
const ADMIN_API: &str = include_str!("api.rs");
const AKRA_DASHBOARD_RS: &str = include_str!("akra_dashboard.rs");
const ADMIN_MOD: &str = include_str!("mod.rs");
const ADMIN_PAGES: &str = include_str!("pages.rs");
const ADMIN_STATIC_ASSETS: &str = include_str!("static_assets.rs");
const TEST_ADMIN_HOST: &str = "akra-test.localhost:18442";
const TEST_ADMIN_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const BUNDLED_ASSET_CACHE_CONTROL: &str = "private, no-cache";

fn source_contains(source: &str, needle: &str) -> bool {
    source.contains(needle) || source.replace("\r\n", "\n").contains(needle)
}

fn admin_game_source_contains(needle: &str) -> bool {
    [
        AKRA_DIORAMA_TS,
        AKRA_AGENT_WORLD_TS,
        AKRA_AGENT_ATLAS_TS,
        AKRA_CAMERA_CONTROLLER_TS,
        AKRA_GAME_TYPES_TS,
        AKRA_SCENE_CONFIG_TS,
        AKRA_SCENE_STORE_TS,
    ]
    .iter()
    .any(|source| source_contains(source, needle))
}

fn assert_bundled_asset_cache_headers(response: &axum::response::Response) -> HeaderValue {
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static(BUNDLED_ASSET_CACHE_CONTROL))
    );
    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("bundled assets should include a content ETag")
        .clone();
    let etag_text = etag.to_str().expect("content ETag should be valid text");
    assert!(etag_text.starts_with("\"sha256-"), "{etag_text}");
    assert!(etag_text.ends_with('"'), "{etag_text}");
    assert_eq!(etag_text.len(), 73, "{etag_text}");
    etag
}

#[test]
fn admin_response_hardening_overrides_untrusted_cache_policy() {
    let mut response = Response::new(Body::empty());
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000"),
    );

    let hardened = harden_admin_response(response);

    assert_eq!(
        hardened.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store, max-age=0"))
    );
}

struct TempAdminWorkspace {
    path: String,
}

impl TempAdminWorkspace {
    fn new(prefix: &str) -> Self {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "akra-admin-api-{prefix}-{}-{unique_suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp admin workspace should be created");
        Self {
            path: path.display().to_string(),
        }
    }

    fn new_git(prefix: &str) -> Self {
        let workspace = Self::new(prefix);
        let output = std::process::Command::new("git")
            .args(["init", "-q", workspace.path.as_str()])
            .output()
            .expect("git fixture initialization should run");
        assert!(output.status.success());
        workspace
    }
}

impl Drop for TempAdminWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn admin_test_router(workspace: &TempAdminWorkspace) -> Router {
    build_router(build_admin_state(
        workspace.path.clone(),
        AdminSecurityConfig::for_test(TEST_ADMIN_TOKEN, 18442),
    ))
}

fn write_rollout_evidence_fixture(workspace: &TempAdminWorkspace) {
    let remote = std::process::Command::new("git")
        .args([
            "-C",
            workspace.path.as_str(),
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widgets.git",
        ])
        .output()
        .expect("fixture remote command should run");
    assert!(remote.status.success());
    let generated_at = chrono::Utc::now();
    let merge_sha = "0123456789abcdef0123456789abcdef01234567";
    let evidence = json!({
        "schemaVersion": 1,
        "generatedAt": generated_at.to_rfc3339(),
        "repository": "acme/widgets",
        "baseBranch": "prerelease",
        "decision": {
            "status": "ready_for_remediate",
            "recommendedSchedulerMode": "remediate",
            "queueAdmissionEnabled": true,
            "rulesetChange": "approval_required",
            "reason": "fixture evidence passed"
        },
        "sample": {
            "pullRequestCount": 1,
            "earliestMergedAt": (generated_at - chrono::Duration::hours(24)).to_rfc3339(),
            "latestMergedAt": generated_at.to_rfc3339(),
            "windowHours": 24,
            "rows": [{
                "mergeSha": merge_sha,
                "evidenceShaMatchesActionsTarget": true,
                "preMerge": { "fastGate": { "source": "projected" } }
            }]
        },
        "timings": {
            "fastGate": { "sampleCount": 1, "p50Seconds": 88, "p95Seconds": 88, "label": "projected" },
            "actualFastGate": { "sampleCount": 1, "p50Seconds": 70, "p95Seconds": 70, "label": "actual" },
            "ciGate": { "sampleCount": 1, "p50Seconds": 500, "p95Seconds": 500, "label": "actual" },
            "postMergeGate": { "sampleCount": 1, "p50Seconds": 510, "p95Seconds": 510, "label": "actual" }
        },
        "actualFastGateRuns": [{
            "id": 99,
            "headSha": merge_sha,
            "conclusion": "success",
            "seconds": 70,
            "runUrl": "https://github.com/acme/widgets/actions/runs/99",
            "jobUrl": "https://github.com/acme/widgets/actions/runs/99/job/1"
        }],
        "postMerge": { "sampleCount": 1, "failureCount": 0, "failureRatePercent": 0 },
        "quota": {
            "limit": 5000,
            "remaining": 4900,
            "used": 100,
            "reset": (generated_at + chrono::Duration::hours(1)).timestamp(),
            "collectorRequests": 3,
            "usedPercent": 2
        },
        "criteria": {
            "sampleWindow": { "status": "pass", "source": "github_live_sample", "note": "window passed" },
            "evidenceShaMismatch": { "status": "pass", "source": "github_live_sample", "note": "SHA matched" }
        },
        "canaries": {
            "productionSuccess": {
                "status": "pass",
                "pullRequestNumber": 42,
                "mergeSha": merge_sha,
                "runUrl": "https://github.com/acme/widgets/actions/runs/99"
            },
            "failure": { "status": "pass" }
        }
    });
    let artifact_dir = std::path::Path::new(&workspace.path)
        .join("docs/validation/artifacts/post-merge-validation-rollout-fixture");
    fs::create_dir_all(&artifact_dir).expect("artifact directory should create");
    fs::write(
        artifact_dir.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence).expect("evidence should serialize"),
    )
    .expect("evidence fixture should write");
}

fn admin_debug_harness_test_router(workspace: &TempAdminWorkspace) -> Router {
    build_router(build_admin_state_with_debug_harness(
        workspace.path.clone(),
        AdminSecurityConfig::for_test(TEST_ADMIN_TOKEN, 18442),
        true,
    ))
}

fn admin_request_builder() -> axum::http::request::Builder {
    Request::builder()
        .header(header::HOST, TEST_ADMIN_HOST)
        .header(ADMIN_TOKEN_HEADER, TEST_ADMIN_TOKEN)
}

async fn json_body(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body).expect("response body should be JSON")
}

async fn text_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    String::from_utf8(body.to_vec()).expect("response body should be UTF-8")
}

async fn first_sse_json_frame(response: axum::response::Response) -> (i64, Value) {
    let mut chunks = response.into_body().into_data_stream();
    let mut text = String::new();
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(3), chunks.next())
            .await
            .expect("SSE frame should arrive before timeout")
            .expect("SSE response should remain open")
            .expect("SSE body chunk should be readable");
        text.push_str(std::str::from_utf8(&chunk).expect("SSE frame should be UTF-8"));
        let normalized = text.replace("\r\n", "\n");
        if let Some((frame, _)) = normalized.split_once("\n\n") {
            let event_id = frame
                .lines()
                .find_map(|line| line.strip_prefix("id:"))
                .expect("SSE update should carry an event id")
                .trim()
                .parse::<i64>()
                .expect("SSE event id should be numeric");
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            return (
                event_id,
                serde_json::from_str(&data).expect("SSE data should be JSON"),
            );
        }
        assert!(text.len() < 1_000_000, "SSE first frame must stay bounded");
    }
}

async fn bytes_body(response: axum::response::Response) -> Vec<u8> {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable")
        .to_vec()
}

async fn bootstrap_admin_json_session(router: &Router) -> (String, String) {
    let response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/planning/summary")
                .body(Body::empty())
                .expect("summary request should build"),
        )
        .await
        .expect("summary request should be served");

    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("summary should set CSRF cookie")
        .to_str()
        .expect("set-cookie should be valid text")
        .to_string();
    assert!(set_cookie.contains("akra_admin_csrf="));
    assert!(set_cookie.to_ascii_lowercase().contains("samesite=strict"));

    let body = json_body(response).await;
    let csrf_token = body["csrf_token"]
        .as_str()
        .expect("summary should expose CSRF token")
        .to_string();
    assert_eq!(csrf_token.len(), 32);

    (format!("akra_admin_csrf={csrf_token}"), csrf_token)
}

async fn bootstrap_admin_html_session(router: &Router) -> (String, String, String) {
    let response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/admin")
                .body(Body::empty())
                .expect("admin page request should build"),
        )
        .await
        .expect("admin page request should be served");

    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("admin page should set CSRF cookie")
        .to_str()
        .expect("set-cookie should be valid text")
        .to_string();
    let csrf_token = csrf_token_from_set_cookie(&set_cookie);
    let body = text_body(response).await;
    assert!(body.contains(&format!("value=\"{csrf_token}\"")));

    (format!("akra_admin_csrf={csrf_token}"), csrf_token, body)
}

fn csrf_token_from_set_cookie(set_cookie: &str) -> String {
    set_cookie
        .split("akra_admin_csrf=")
        .nth(1)
        .and_then(|value| value.split(';').next())
        .expect("set-cookie should include CSRF value")
        .to_string()
}

fn json_request(
    method: Method,
    uri: &str,
    body: Value,
    cookie: Option<&str>,
    csrf_token: Option<&str>,
) -> Request<Body> {
    let mut builder = admin_request_builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(csrf_token) = csrf_token {
        builder = builder.header("x-csrf-token", csrf_token);
    }
    builder
        .body(Body::from(body.to_string()))
        .expect("JSON request should build")
}

fn encoded_form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(key, percent_encoding::NON_ALPHANUMERIC),
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn html_form_request(uri: &str, body: String, cookie: Option<&str>, htmx: bool) -> Request<Body> {
    let mut builder = admin_request_builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if htmx {
        builder = builder.header("HX-Request", "true");
    }
    builder
        .body(Body::from(body))
        .expect("HTML form request should build")
}

/*
 * 제거된 raw-authority field는 stale browser tab이나 오래된 bookmark/form replay에서 여전히 들어올 수 있다.
 * extract_file_updates는 그런 이름을 application-level file mutation으로 승격하지 않아야 한다.
 * 이 테스트는 inbound adapter의 allow-list가 old transport vocabulary를 조용히 drop하는지 검증한다.
 */
#[test]
fn page_mutation_ignores_removed_raw_authority_file_updates() {
    // 현재 지원되는 field를 함께 넣어 parser가 전체 실패가 아니라 selective filtering을 수행한다는 점을 증명한다.
    let updates = extract_file_updates(HashMap::from([
        ("file_task_authority".to_string(), "{}".to_string()),
        ("file_directions".to_string(), "version = 1".to_string()),
        (
            "file_queue_idle_prompt".to_string(),
            "# Queue prompt".to_string(),
        ),
    ]));

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].key, PlanningAdminFileKey::QueueIdlePrompt);
}

/*
 * draft-kind navigation은 adapter policy다.
 * service enum이 어떤 admin tab 아래에서 editor를 열지 결정하는 것은 HTML navigation surface의 책임이다.
 * raw task authority draft kind가 visible navigation에서 제거된 상태도 여기서 고정한다.
 */
#[test]
fn nav_no_longer_has_raw_task_authority_draft_kind() {
    assert_eq!(
        nav_for_kind(PlanningAdminDraftKind::FullPlanning),
        "dashboard"
    );
    assert_eq!(
        nav_for_kind(PlanningAdminDraftKind::QueueIdlePrompt),
        "directions"
    );
}

#[test]
fn reset_form_and_json_spelling_maps_to_shared_application_target() {
    /*
     * HTML forms and JSON callers share parse_reset_target in admin_api::mod.
     * Keep the accepted labels mapped directly to PlanningResetTarget so admin
     * never grows a surface-specific destructive reset vocabulary.
     */
    for (raw, expected) in [
        ("queue", PlanningResetTarget::Queue),
        ("directions", PlanningResetTarget::Directions),
        ("all", PlanningResetTarget::All),
    ] {
        assert_eq!(parse_reset_target(raw).unwrap(), expected);
    }
    assert!(parse_reset_target("tasks").is_err());
}

#[test]
fn admin_server_arg_parser_accepts_default_and_port_only_surface() {
    let default_args = parse_args(Vec::<String>::new()).expect("default args should parse");
    assert_eq!(default_args.port, 18442);
    assert!(!default_args.debug_harness);

    let args = parse_args(["--port".to_string(), "19000".to_string()])
        .expect("explicit port should parse");
    assert_eq!(args.port, 19000);
    assert!(!args.debug_harness);

    let debug_args =
        parse_args(["--debug-harness".to_string()]).expect("debug harness flag should parse");
    assert!(debug_args.debug_harness);

    for (args, expected) in [
        (
            vec!["--port".to_string()],
            "--port requires a value".to_string(),
        ),
        (
            vec!["--port".to_string(), "abc".to_string()],
            "invalid port: abc".to_string(),
        ),
        (
            vec!["--host".to_string(), "0.0.0.0".to_string()],
            "unsupported argument: --host".to_string(),
        ),
    ] {
        let error = parse_args(args).expect_err("admin args should fail");
        assert!(
            error.to_string().contains(&expected),
            "expected `{expected}`, got `{error:#}`"
        );
    }
}

#[test]
fn admin_http_helpers_cover_csrf_redirect_htmx_and_render_failures() {
    assert_eq!(encode_uri_component("queue reset ok"), "queue%20reset%20ok");
    assert_eq!(
        notice_location("/admin/controls", "reset: queue"),
        "/admin/controls?notice=reset%3A%20queue"
    );

    let (jar, generated_token) = ensure_csrf_cookie(CookieJar::new());
    assert_eq!(generated_token.len(), 32);
    assert!(
        generated_token
            .chars()
            .all(|value| value.is_ascii_hexdigit())
    );
    assert!(verify_form_csrf(&jar, &generated_token).is_ok());
    assert_eq!(
        verify_form_csrf(&CookieJar::new(), &generated_token),
        Err(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        verify_form_csrf(&jar, "wrong-token"),
        Err(StatusCode::FORBIDDEN)
    );

    let (same_jar, existing_token) = ensure_csrf_cookie(jar);
    assert_eq!(existing_token, generated_token);

    let mut headers = HeaderMap::new();
    assert_eq!(
        verify_header_csrf(&same_jar, &headers),
        Err(StatusCode::FORBIDDEN)
    );
    headers.insert(
        "x-csrf-token",
        HeaderValue::from_str(&existing_token).expect("token should be a header value"),
    );
    assert!(verify_header_csrf(&same_jar, &headers).is_ok());
    headers.insert("x-csrf-token", HeaderValue::from_static("wrong-token"));
    assert_eq!(
        verify_header_csrf(&same_jar, &headers),
        Err(StatusCode::FORBIDDEN)
    );

    let mut htmx_headers = HeaderMap::new();
    assert!(!is_htmx_request(&htmx_headers));
    htmx_headers.insert("hx-request", HeaderValue::from_static("TRUE"));
    assert!(!is_htmx_request(&htmx_headers));
    htmx_headers.insert("hx-request", HeaderValue::from_static("true"));
    assert!(is_htmx_request(&htmx_headers));

    let security = AdminSecurityConfig::for_test(TEST_ADMIN_TOKEN, 18442);
    let local_headers = HeaderMap::new();
    assert_eq!(
        verify_local_admin_request(&local_headers, &security),
        Err(StatusCode::BAD_REQUEST)
    );

    let mut remote_host_headers = HeaderMap::new();
    remote_host_headers.insert(header::HOST, HeaderValue::from_static("evil.example:18442"));
    assert_eq!(
        verify_local_admin_request(&remote_host_headers, &security),
        Err(StatusCode::FORBIDDEN)
    );

    let mut local_host_headers = HeaderMap::new();
    local_host_headers.insert(header::HOST, HeaderValue::from_static(TEST_ADMIN_HOST));
    local_host_headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://akra-test.localhost:18442"),
    );
    local_host_headers.insert(
        header::REFERER,
        HeaderValue::from_static("http://akra-test.localhost:18442/admin"),
    );
    assert!(verify_local_admin_request(&local_host_headers, &security).is_ok());

    local_host_headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://akra-test.localhost:19000"),
    );
    assert_eq!(
        verify_local_admin_request(&local_host_headers, &security),
        Err(StatusCode::FORBIDDEN)
    );

    local_host_headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://evil.example:18442"),
    );
    assert_eq!(
        verify_local_admin_request(&local_host_headers, &security),
        Err(StatusCode::FORBIDDEN)
    );

    assert_eq!(
        render_fragment(BrokenTemplate).expect_err("fragment render should fail"),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        render_html(CookieJar::new(), BrokenTemplate).expect_err("page render should fail"),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        internal_server_error(anyhow::anyhow!("boom")),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn admin_json_summary_and_runtime_bootstrap_csrf_session() {
    let workspace = TempAdminWorkspace::new("summary-runtime");
    let router = admin_test_router(&workspace);

    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;
    let response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/planning/runtime")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("runtime request should build"),
        )
        .await
        .expect("runtime request should be served");

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(
        body["preview_status_label"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "runtime API should return the application projection"
    );
    assert_eq!(csrf_token.len(), 32);
}

#[tokio::test]
async fn admin_router_rejects_non_local_host_on_read_only_routes() {
    let workspace = TempAdminWorkspace::new("host-guard");
    let router = admin_test_router(&workspace);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/planning/summary")
                .header(header::HOST, "evil.example:18442")
                .header(ADMIN_TOKEN_HEADER, TEST_ADMIN_TOKEN)
                .body(Body::empty())
                .expect("summary request should build"),
        )
        .await
        .expect("summary request should be served");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_router_requires_capability_and_exact_loopback_authority() {
    let workspace = TempAdminWorkspace::new("auth-guard");
    let router = admin_test_router(&workspace);

    let unauthenticated_api = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/planning/summary")
                .header(header::HOST, TEST_ADMIN_HOST)
                .body(Body::empty())
                .expect("unauthenticated API request should build"),
        )
        .await
        .expect("unauthenticated API request should be served");
    assert_eq!(unauthenticated_api.status(), StatusCode::UNAUTHORIZED);

    let unauthenticated_page = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/admin")
                .header(header::HOST, TEST_ADMIN_HOST)
                .body(Body::empty())
                .expect("unauthenticated page request should build"),
        )
        .await
        .expect("unauthenticated page request should be served");
    assert_eq!(unauthenticated_page.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        unauthenticated_page.headers().get(header::LOCATION),
        Some(&HeaderValue::from_static("/admin/login"))
    );

    let missing_host = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/planning/summary")
                .header(ADMIN_TOKEN_HEADER, TEST_ADMIN_TOKEN)
                .body(Body::empty())
                .expect("missing Host request should build"),
        )
        .await
        .expect("missing Host request should be served");
    assert_eq!(missing_host.status(), StatusCode::BAD_REQUEST);

    let wrong_port = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/planning/summary")
                .header(header::HOST, "127.0.0.1:19000")
                .header(ADMIN_TOKEN_HEADER, TEST_ADMIN_TOKEN)
                .body(Body::empty())
                .expect("wrong-port request should build"),
        )
        .await
        .expect("wrong-port request should be served");
    assert_eq!(wrong_port.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_login_exchanges_capability_for_strict_http_only_session_cookie() {
    let workspace = TempAdminWorkspace::new("login-session");
    let router = admin_test_router(&workspace);

    let login_page = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/admin/login")
                .header(header::HOST, TEST_ADMIN_HOST)
                .body(Body::empty())
                .expect("login page request should build"),
        )
        .await
        .expect("login page request should be served");
    assert_eq!(login_page.status(), StatusCode::OK);
    let content_security_policy = login_page
        .headers()
        .get("content-security-policy")
        .expect("admin responses should set a content security policy")
        .to_str()
        .expect("content security policy should be valid text");
    for directive in [
        "default-src 'self'",
        "connect-src 'self'",
        "font-src 'self'",
        "frame-ancestors 'none'",
        "img-src 'self'",
        "script-src 'self'",
        "style-src 'self' 'unsafe-inline'",
    ] {
        assert!(content_security_policy.contains(directive));
    }
    assert!(!content_security_policy.contains("script-src 'self' 'unsafe-inline'"));
    assert!(!content_security_policy.contains("'unsafe-eval'"));
    assert_eq!(
        login_page.headers().get("cache-control"),
        Some(&HeaderValue::from_static("no-store, max-age=0"))
    );
    assert_eq!(
        login_page.headers().get("cross-origin-opener-policy"),
        Some(&HeaderValue::from_static("same-origin"))
    );
    assert_eq!(
        login_page.headers().get("cross-origin-resource-policy"),
        Some(&HeaderValue::from_static("same-origin"))
    );
    assert!(login_page
        .headers()
        .get("permissions-policy")
        .is_some_and(|value| value
            == "camera=(), display-capture=(), geolocation=(), microphone=(), payment=(), usb=()"));
    assert_eq!(
        login_page.headers().get("referrer-policy"),
        Some(&HeaderValue::from_static("same-origin")),
        "the login document must let real browser form submissions retain their loopback origin"
    );
    let login_page_body = text_body(login_page).await;
    assert!(login_page_body.contains(r#"<meta name="referrer" content="same-origin">"#));
    assert!(!login_page_body.contains("Authentication failed"));

    let rejected_token = "super-secret-rejected-token";
    let rejected_login = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/login")
                .header(header::HOST, TEST_ADMIN_HOST)
                .header(header::ORIGIN, format!("http://{TEST_ADMIN_HOST}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("token={rejected_token}")))
                .expect("rejected login request should build"),
        )
        .await
        .expect("rejected login request should be served");
    assert_eq!(rejected_login.status(), StatusCode::UNAUTHORIZED);
    assert!(rejected_login.headers().get(header::SET_COOKIE).is_none());
    assert_eq!(
        rejected_login.headers().get("referrer-policy"),
        Some(&HeaderValue::from_static("same-origin"))
    );
    assert_eq!(
        rejected_login.headers().get("cache-control"),
        Some(&HeaderValue::from_static("no-store, max-age=0"))
    );
    assert!(
        rejected_login
            .headers()
            .get("content-security-policy")
            .is_some()
    );
    let rejected_body = text_body(rejected_login).await;
    assert!(
        rejected_body.contains("Authentication failed. Check the capability token and try again.")
    );
    assert!(rejected_body.contains("role=\"alert\""));
    assert!(!rejected_body.contains(rejected_token));

    let accepted_login = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/login")
                .header(header::HOST, TEST_ADMIN_HOST)
                .header(header::ORIGIN, format!("http://{TEST_ADMIN_HOST}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("token={TEST_ADMIN_TOKEN}")))
                .expect("accepted login request should build"),
        )
        .await
        .expect("accepted login request should be served");
    assert_eq!(accepted_login.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        accepted_login.headers().get(header::LOCATION),
        Some(&HeaderValue::from_static("/admin"))
    );
    let set_cookie = accepted_login
        .headers()
        .get(header::SET_COOKIE)
        .expect("accepted login should set a session cookie")
        .to_str()
        .expect("session cookie should be valid text");
    let normalized_cookie = set_cookie.to_ascii_lowercase();
    assert!(normalized_cookie.starts_with("akra_admin_session="));
    assert!(normalized_cookie.contains("httponly"));
    assert!(normalized_cookie.contains("samesite=strict"));
    assert!(normalized_cookie.contains("path=/"));
    assert!(normalized_cookie.contains("max-age=1800"));
    assert!(!normalized_cookie.contains("domain="));
    let session_cookie = set_cookie
        .split(';')
        .next()
        .expect("session cookie should contain a name/value pair")
        .to_string();

    let authenticated_with_cookie = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/planning/summary")
                .header(header::HOST, TEST_ADMIN_HOST)
                .header(header::COOKIE, session_cookie.as_str())
                .body(Body::empty())
                .expect("session-authenticated request should build"),
        )
        .await
        .expect("session-authenticated request should be served");
    assert_eq!(authenticated_with_cookie.status(), StatusCode::OK);
    assert_eq!(
        authenticated_with_cookie
            .headers()
            .get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store, max-age=0")),
        "authenticated JSON responses must remain non-cacheable"
    );
    let refreshed_cookie = authenticated_with_cookie
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| {
            value
                .to_ascii_lowercase()
                .starts_with("akra_admin_session=")
        })
        .expect("an active browser session should receive a sliding cookie")
        .to_ascii_lowercase();
    assert!(refreshed_cookie.starts_with(&session_cookie.to_ascii_lowercase()));
    assert!(refreshed_cookie.contains("max-age=1800"));
    assert!(!refreshed_cookie.contains("domain="));

    let logout = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/logout")
                .header(header::HOST, TEST_ADMIN_HOST)
                .header(header::COOKIE, session_cookie.as_str())
                .body(Body::empty())
                .expect("logout request should build"),
        )
        .await
        .expect("logout request should be served");
    assert_eq!(logout.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        logout.headers().get(header::LOCATION),
        Some(&HeaderValue::from_static("/admin/login"))
    );
    assert!(
        logout
            .headers()
            .get(header::SET_COOKIE)
            .is_some_and(|value| value
                .to_str()
                .is_ok_and(|value| value.to_ascii_lowercase().contains("max-age=0")))
    );
    assert_eq!(
        logout.headers().get_all(header::SET_COOKIE).iter().count(),
        1,
        "logout must not append a sliding refresh after revocation"
    );

    let expired_session = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/planning/summary")
                .header(header::HOST, TEST_ADMIN_HOST)
                .header(header::COOKIE, session_cookie.as_str())
                .body(Body::empty())
                .expect("expired session request should build"),
        )
        .await
        .expect("expired session request should be served");
    assert_eq!(expired_session.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_json_mutations_require_header_csrf_and_share_reset_guard() {
    let workspace = TempAdminWorkspace::new("reset-guard");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;

    let forbidden = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/reset",
            json!({ "target": "queue" }),
            Some(&cookie),
            None,
        ))
        .await
        .expect("reset request should be served");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let bad_target = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/reset",
            json!({ "target": "tasks" }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("reset request should be served");
    assert_eq!(bad_target.status(), StatusCode::BAD_REQUEST);

    let accepted = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/reset",
            json!({ "target": "queue" }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("reset request should be served");
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted_body = json_body(accepted).await;
    assert_eq!(accepted_body["target"].as_str(), Some("queue"));
    assert!(
        accepted_body["rewritten_paths"].is_array(),
        "reset response should expose facade outcome JSON"
    );
}

#[tokio::test]
async fn admin_json_draft_routes_round_trip_through_router() {
    let workspace = TempAdminWorkspace::new_git("draft-routes");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;

    let created = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/drafts",
            json!({ "kind": "full_planning" }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("draft create request should be served");
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = json_body(created).await;
    let draft_name = created_body["draft_name"]
        .as_str()
        .expect("create draft API should return draft name");
    assert_eq!(created_body["kind"].as_str(), Some("full_planning"));

    let load_uri = format!("/api/planning/drafts/{draft_name}?kind=full_planning");
    let loaded = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri(load_uri)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("draft load request should build"),
        )
        .await
        .expect("draft load request should be served");
    assert_eq!(loaded.status(), StatusCode::OK);
    let loaded_body = json_body(loaded).await;
    assert_eq!(loaded_body["draft_name"].as_str(), Some(draft_name));
    assert!(
        loaded_body["files"]
            .as_array()
            .is_some_and(|files| !files.is_empty()),
        "loaded draft should expose editable files"
    );

    let save_body = json!({
        "kind": "full_planning",
        "files": []
    });
    let save_uri = format!("/api/planning/drafts/{draft_name}");
    let saved = router
        .clone()
        .oneshot(json_request(
            Method::PUT,
            &save_uri,
            save_body.clone(),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("draft save request should be served");
    assert_eq!(saved.status(), StatusCode::OK);
    assert_eq!(
        json_body(saved).await["draft_name"].as_str(),
        Some(draft_name)
    );

    let validate_uri = format!("/api/planning/drafts/{draft_name}/validate");
    let validated = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            &validate_uri,
            save_body,
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("draft validate request should be served");
    assert_eq!(validated.status(), StatusCode::OK);
    assert_eq!(
        json_body(validated).await["draft_name"].as_str(),
        Some(draft_name)
    );

    let promote_uri = format!("/api/planning/drafts/{draft_name}/promote");
    let promoted = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            &promote_uri,
            json!({
                "kind": "full_planning",
                "files": []
            }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("draft promote request should be served");
    assert_eq!(promoted.status(), StatusCode::OK);
    let promoted_body = json_body(promoted).await;
    assert!(promoted_body["promoted_file_count"].is_number());
    assert!(promoted_body["is_valid"].is_boolean());
    assert_eq!(
        promoted_body["session"]["draft_name"].as_str(),
        Some(draft_name)
    );
}

#[tokio::test]
async fn admin_json_draft_routes_reject_malformed_draft_names() {
    let workspace = TempAdminWorkspace::new("draft-name-json");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;

    for uri in [
        "/api/planning/drafts/%2E%2E?kind=full_planning",
        "/api/planning/drafts/%2E%2E%2Foutside?kind=full_planning",
        "/api/planning/drafts/bad%5Cname?kind=full_planning",
        "/api/planning/drafts/bad%3Aname?kind=full_planning",
        "/api/planning/drafts/bad%20name?kind=full_planning",
        "/api/planning/drafts/bad%0Aname?kind=full_planning",
    ] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(uri)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .expect("draft load request should build"),
            )
            .await
            .expect("draft load request should be served");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }

    for (method, uri) in [
        (Method::PUT, "/api/planning/drafts/%2E%2E%2Foutside"),
        (
            Method::POST,
            "/api/planning/drafts/%2E%2E%2Foutside/validate",
        ),
        (
            Method::POST,
            "/api/planning/drafts/%2E%2E%2Foutside/promote",
        ),
    ] {
        let response = router
            .clone()
            .oneshot(json_request(
                method,
                uri,
                json!({ "kind": "full_planning", "files": [] }),
                Some(&cookie),
                Some(&csrf_token),
            ))
            .await
            .expect("draft mutation request should be served");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[tokio::test]
async fn admin_akra_events_api_rejects_unbounded_limits() {
    let workspace = TempAdminWorkspace::new("events-limit");
    let router = admin_test_router(&workspace);

    let response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/events?limit=201")
                .body(Body::empty())
                .expect("events request should build"),
        )
        .await
        .expect("events request should be served");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["error"].as_str(), Some("event_limit_too_large"));
    assert!(
        body["operatorMessage"]
            .as_str()
            .is_some_and(|message| message.contains("200 or less"))
    );
}

#[tokio::test]
async fn admin_json_crud_and_file_sync_routes_round_trip_through_router() {
    let workspace = TempAdminWorkspace::new("crud-file-sync");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;

    let direction = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/directions",
            json!({
                "title": "Coverage Direction",
                "summary": "Exercise JSON admin route wiring.",
                "success_criteria_text": "Route returns facade outcome",
                "scope_hints_text": "admin api",
                "state": "active"
            }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("direction upsert request should be served");
    assert_eq!(direction.status(), StatusCode::OK);
    let direction_body = json_body(direction).await;
    let direction_id = direction_body["management"]["directions"]
        .as_array()
        .and_then(|directions| {
            directions
                .iter()
                .find(|direction| direction["title"].as_str() == Some("Coverage Direction"))
        })
        .and_then(|direction| direction["id"].as_str())
        .expect("direction create should return the created row")
        .to_string();

    let task = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/tasks",
            json!({
                "direction_id": direction_id,
                "title": "Coverage Task",
                "description": "Exercise task JSON route wiring.",
                "status": "ready",
                "base_priority": "30",
                "dynamic_priority_delta": "0",
                "priority_reason": "coverage",
                "depends_on_text": "",
                "blocked_by_text": ""
            }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("task upsert request should be served");
    assert_eq!(task.status(), StatusCode::OK);
    let task_body = json_body(task).await;
    let task_id = task_body["management"]["tasks"]
        .as_array()
        .and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task["title"].as_str() == Some("Coverage Task"))
        })
        .and_then(|task| task["id"].as_str())
        .expect("task create should return the created row")
        .to_string();

    let deleted_task = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/tasks/delete",
            json!({ "id": task_id }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("task delete request should be served");
    assert_eq!(deleted_task.status(), StatusCode::OK);

    let deleted_direction = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/directions/delete",
            json!({ "id": direction_id }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("direction delete request should be served");
    assert_eq!(deleted_direction.status(), StatusCode::OK);

    let exported = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/files/export",
            json!({}),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("file export request should be served");
    assert_eq!(exported.status(), StatusCode::OK);
    assert!(json_body(exported).await["paths"].is_array());

    let applied = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/planning/files/apply",
            json!({}),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("file apply request should be served");
    assert_eq!(applied.status(), StatusCode::OK);
    assert!(json_body(applied).await["paths"].is_array());
}

#[tokio::test]
async fn admin_akra_json_snapshot_routes_render_read_only_views() {
    let workspace = TempAdminWorkspace::new("akra-snapshots");
    let router = admin_test_router(&workspace);

    for uri in [
        "/api/admin/akra/dashboard",
        "/api/admin/akra/pool",
        "/api/admin/akra/agents",
        "/api/admin/akra/distributor",
        "/api/admin/akra/events?limit=1",
        "/api/admin/akra/control",
    ] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("Akra snapshot request should build"),
            )
            .await
            .expect("Akra snapshot request should be served");
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let body = json_body(response).await;
        assert!(
            body.is_object() || body.is_array(),
            "Akra snapshot route should return structured JSON for {uri}"
        );
        if uri == "/api/admin/akra/dashboard" {
            assert!(body["planningRevision"].is_number());
            assert!(body["eventFeed"]["eventCursor"].is_null());
            assert!(body["scene"]["stations"].is_array());
            assert!(body["scene"]["actors"].is_array());
            assert!(body["scene"]["standbyCharacters"].is_array());
            assert!(body["scene"]["diagnostics"].is_array());
            assert_eq!(body["scene"]["actors"].as_array().map(Vec::len), Some(0));
            assert_eq!(body["scene"]["standbyProfileCount"], 3);
            assert_eq!(
                body["scene"]["standbyCharacters"].as_array().map(Vec::len),
                Some(3)
            );
            assert_eq!(
                body["scene"]["standbyCharacters"][0]["presenceKind"],
                "configured_standby"
            );
            assert_eq!(
                body["scene"]["standbyCharacters"][0]["staticPose"],
                "laptop"
            );
            for runtime_identity in [
                "actorId",
                "taskId",
                "slotId",
                "sessionKey",
                "ownerAgentId",
                "ownerSessionKey",
                "leaseGeneration",
                "branchName",
                "queueItemId",
            ] {
                assert!(
                    body["scene"]["standbyCharacters"][0]
                        .get(runtime_identity)
                        .is_none(),
                    "standby profile must not fabricate {runtime_identity}"
                );
            }
            assert!(
                body["scene"]["stations"]
                    .as_array()
                    .is_some_and(|stations| !stations.is_empty())
            );
        }
    }
}

#[tokio::test]
async fn admin_akra_control_route_requires_csrf_and_returns_typed_projection() {
    let workspace = TempAdminWorkspace::new("akra-control");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token, _) = bootstrap_admin_html_session(&router).await;

    let rejected = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/admin/akra/control",
            json!({ "action": "disable" }),
            Some(&cookie),
            None,
        ))
        .await
        .expect("control request without CSRF should be served");
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

    let accepted = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/admin/akra/control",
            json!({ "action": "disable" }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("typed control request should be served");
    assert_eq!(accepted.status(), StatusCode::OK);
    let body = json_body(accepted).await;
    assert_eq!(body["modeEnabled"], false);
    assert_eq!(body["controlEffectInFlight"], false);
    assert_eq!(body["latestCommand"]["action"], "disable");
    assert_eq!(body["latestCommand"]["state"], "completed");
    let command_id = body["latestCommand"]["commandId"]
        .as_str()
        .expect("control response should expose a command id");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|value| value.contains("정지"))
    );
    let command = router
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri(format!("/api/admin/akra/commands/{command_id}"))
                .body(Body::empty())
                .expect("command status request should build"),
        )
        .await
        .expect("command status request should be served");
    assert_eq!(command.status(), StatusCode::OK);
    let command = json_body(command).await;
    assert_eq!(command["commandId"], command_id);
    assert_eq!(command["state"], "completed");
}

#[tokio::test]
async fn admin_debug_harness_drives_fake_application_projection_without_real_control_mutation() {
    let workspace = TempAdminWorkspace::new("akra-debug-harness");
    let router = admin_debug_harness_test_router(&workspace);
    let (cookie, csrf_token, _) = bootstrap_admin_html_session(&router).await;

    let initial = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/dashboard")
                .body(Body::empty())
                .expect("debug dashboard request should build"),
        )
        .await
        .expect("debug dashboard request should be served");
    assert_eq!(initial.status(), StatusCode::OK);
    let initial = json_body(initial).await;
    assert_eq!(initial["debugHarness"]["enabled"], true);
    assert_eq!(initial["debugHarness"]["stageKey"], "ready");
    assert_eq!(initial["scene"]["actors"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        initial["scene"]["standbyCharacters"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );

    let forbidden = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/admin/akra/debug-harness",
            json!({ "action": "step" }),
            Some(&cookie),
            None,
        ))
        .await
        .expect("debug command without CSRF should be served");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let selected = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/admin/akra/debug-harness",
            json!({ "action": "scenario", "scenario": "check_failure_recovery" }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("debug scenario command should be served");
    assert_eq!(selected.status(), StatusCode::OK);
    assert_eq!(
        json_body(selected).await["scenarioKey"],
        "check_failure_recovery"
    );

    for _ in 0..2 {
        let stepped = router
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/admin/akra/debug-harness",
                json!({ "action": "step" }),
                Some(&cookie),
                Some(&csrf_token),
            ))
            .await
            .expect("debug step command should be served");
        assert_eq!(stepped.status(), StatusCode::OK);
    }

    let blocked = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/dashboard")
                .body(Body::empty())
                .expect("blocked dashboard request should build"),
        )
        .await
        .expect("blocked dashboard request should be served");
    let blocked = json_body(blocked).await;
    assert_eq!(blocked["debugHarness"]["stageKey"], "blocked");
    assert_eq!(blocked["workspace"]["readiness"], "blocked");
    assert_eq!(blocked["pool"]["summary"]["blocked"], 0);
    assert_eq!(
        blocked["scene"]["actors"]
            .as_array()
            .expect("blocked scene actor projection should exist")
            .len(),
        0,
        "passive CI failure must not fabricate a worker"
    );
    assert_eq!(blocked["validation"]["records"][0]["phase"], "verifying");
    assert_eq!(blocked["scene"]["validation"]["phase"], "verifying");
    assert_eq!(blocked["scene"]["validation"]["workerLeaseActive"], false);

    for _ in 0..3 {
        let stepped = router
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/admin/akra/debug-harness",
                json!({ "action": "step" }),
                Some(&cookie),
                Some(&csrf_token),
            ))
            .await
            .expect("debug recovery step should be served");
        assert_eq!(stepped.status(), StatusCode::OK);
    }
    let working = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let working = json_body(working).await;
    assert_eq!(working["debugHarness"]["stageKey"], "working");
    assert_eq!(working["scene"]["actors"].as_array().map(Vec::len), Some(1));
    assert_eq!(working["scene"]["validation"]["workerLeaseActive"], true);

    let real_control = router
        .oneshot(json_request(
            Method::POST,
            "/api/admin/akra/control",
            json!({ "action": "enable" }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .expect("real control request should be served");
    assert_eq!(real_control.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn admin_debug_harness_all_validation_scenarios_keep_board_and_scene_semantics_aligned() {
    let workspace = TempAdminWorkspace::new("akra-debug-validation-scenarios");
    let router = admin_debug_harness_test_router(&workspace);
    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;
    let scenarios = [
        "post_merge_success",
        "check_failure_recovery",
        "optional_skipped",
        "required_missing",
        "rate_limit_recovery",
        "provider_outage_retry",
        "closed_unmerged_attested",
        "restart_verifying",
        "poll_claim_race",
        "duplicate_late_review",
    ];

    for scenario in scenarios {
        let selected = router
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/admin/akra/debug-harness",
                json!({ "action": "scenario", "scenario": scenario }),
                Some(&cookie),
                Some(&csrf_token),
            ))
            .await
            .expect("validation scenario selection should be served");
        assert_eq!(selected.status(), StatusCode::OK, "scenario {scenario}");
        let selected = json_body(selected).await;
        assert_eq!(selected["scenarioKey"], scenario);
        assert_eq!(selected["scenarios"].as_array().map(Vec::len), Some(10));
        let stage_count = selected["stageCount"]
            .as_u64()
            .expect("scenario stage count should be numeric") as usize;

        for stage_index in 0..stage_count {
            let dashboard = router
                .clone()
                .oneshot(
                    admin_request_builder()
                        .method(Method::GET)
                        .uri("/api/admin/akra/dashboard")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(dashboard.status(), StatusCode::OK);
            let dashboard = json_body(dashboard).await;
            let evidence = router
                .clone()
                .oneshot(
                    admin_request_builder()
                        .method(Method::GET)
                        .uri("/api/admin/akra/pr-validation/evidence?limit=1")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(evidence.status(), StatusCode::OK);
            let evidence = json_body(evidence).await;
            assert_eq!(
                evidence["latest"]["summary"]["status"],
                dashboard["validation"]["rolloutEvidence"]["status"],
                "scenario {scenario} stage {stage_index} evidence status"
            );
            assert_eq!(
                evidence["latest"]["summary"]["actualFastGate"]["label"],
                dashboard["validation"]["rolloutEvidence"]["actualFastGate"]["label"],
                "scenario {scenario} stage {stage_index} actual metric label"
            );
            if evidence["latest"]["summary"]["actualFastGate"]["label"] == "unavailable" {
                for field in ["sampleCount", "p50Seconds", "p95Seconds"] {
                    assert!(
                        evidence["latest"]["summary"]["actualFastGate"][field].is_null(),
                        "scenario {scenario} stage {stage_index} unavailable actual metric must leave {field} uncollected"
                    );
                }
            }
            assert_eq!(evidence["history"].as_array().map(Vec::len), Some(1));
            assert!(evidence["nextCursor"].as_str().is_some());
            if matches!(
                evidence["latest"]["summary"]["status"].as_str(),
                Some("invalid" | "unavailable")
            ) {
                assert_eq!(evidence["lastValid"]["summary"]["status"], "ready");
            }
            let record = &dashboard["validation"]["records"][0];
            let scene = &dashboard["scene"]["validation"];
            assert_eq!(
                scene["recordKey"], record["recordKey"],
                "scenario {scenario} stage {stage_index} record identity"
            );
            assert_eq!(
                scene["phase"], record["phase"],
                "scenario {scenario} stage {stage_index} semantic phase"
            );
            let actor_count = dashboard["scene"]["actors"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0);
            let lease_active = scene["workerLeaseActive"].as_bool().unwrap_or(false);
            assert_eq!(
                actor_count > 0,
                lease_active,
                "scenario {scenario} stage {stage_index} must only show an actor for a real fake lease"
            );
            if actor_count > 0 {
                assert_eq!(scenario, "check_failure_recovery");
                assert_eq!(dashboard["debugHarness"]["stageKey"], "working");
                assert_eq!(record["phase"], "remediation_running");
            }

            if stage_index + 1 < stage_count {
                let stepped = router
                    .clone()
                    .oneshot(json_request(
                        Method::POST,
                        "/api/admin/akra/debug-harness",
                        json!({ "action": "step" }),
                        Some(&cookie),
                        Some(&csrf_token),
                    ))
                    .await
                    .unwrap();
                assert_eq!(stepped.status(), StatusCode::OK);
            }
        }
    }
}

#[tokio::test]
async fn admin_akra_realtime_stream_is_authenticated_sse_with_resume_cursor() {
    let workspace = TempAdminWorkspace::new("akra-realtime-stream");
    let router = admin_test_router(&workspace);

    let response = router
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/stream?afterSequence=0")
                .header("last-event-id", "0")
                .body(Body::empty())
                .expect("realtime stream request should build"),
        )
        .await
        .expect("realtime stream request should be served");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store, max-age=0"))
    );
}

#[tokio::test]
async fn admin_akra_realtime_stream_resumes_oldest_unseen_events_without_loss() {
    let workspace = TempAdminWorkspace::new("akra-realtime-lossless-resume");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    for index in 0..30 {
        let record = admin_validation_record("validation-stream", 400 + index);
        assert!(
            adapter
                .compare_and_swap_runtime_pr_validation_record(
                    &workspace.path,
                    record.key(),
                    None,
                    Some(&record),
                )
                .unwrap()
        );
        assert!(
            adapter
                .compare_and_swap_runtime_pr_validation_record(
                    &workspace.path,
                    record.key(),
                    Some(&record),
                    None,
                )
                .unwrap()
        );
    }
    let location = adapter.resolve_authority_location(&workspace.path).unwrap();
    let connection = Connection::open(&location.authority_store_path).unwrap();
    connection
        .execute(
            "UPDATE authority_metadata SET value = '61' WHERE key = 'runtime_event_sequence'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO runtime_events (
                 sequence, event_kind, projection_kind, projection_key,
                 observed_planning_revision, summary, payload_json, recorded_at
             ) VALUES (61, 'upsert', 'session_detail', 'unrelated-session', 0,
                       'unrelated worker activity', '{}', '2026-08-10T00:00:00Z')",
            [],
        )
        .unwrap();
    drop(connection);
    let router = admin_test_router(&workspace);

    let first_response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/stream?afterSequence=0")
                .header("last-event-id", "0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_response.status(), StatusCode::OK);
    let (first_id, first) = first_sse_json_frame(first_response).await;
    assert_eq!(first_id, 50);
    assert_eq!(first["feed"]["eventCursor"], 61);
    assert_eq!(first["feed"]["visibleEventCount"], 50);
    assert_eq!(first["cursorResetRequired"], false);
    assert_eq!(first["validation"]["changed"], true);
    assert_eq!(
        first["validation"]["revision"], 60,
        "unrelated runtime backlog must not advance the validation-board revision"
    );

    let second_response = router
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/stream")
                .header("last-event-id", first_id.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (second_id, second) = first_sse_json_frame(second_response).await;
    assert_eq!(second_id, 61);
    assert_eq!(second["feed"]["visibleEventCount"], 11);
    assert_eq!(second["cursorResetRequired"], false);

    let mut sequences = first["events"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["events"].as_array().unwrap())
        .map(|event| event["sequence"].as_i64().unwrap())
        .collect::<Vec<_>>();
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=61).collect::<Vec<_>>());
}

fn admin_validation_record(key: &str, pull_request_number: u64) -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new(key).unwrap(),
        PrValidationTarget::new("acme/widgets", pull_request_number).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new(format!("{pull_request_number:040x}")).unwrap(),
            PrValidationCommitSha::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
        ),
    )
}

fn admin_integrated_validation_record(key: &str, pull_request_number: u64) -> PrValidationRecord {
    let record = admin_validation_record(key, pull_request_number);
    let evidence_sha = PrValidationCommitSha::new(format!("{pull_request_number:039x}b")).unwrap();
    let integrated_at = chrono::DateTime::parse_from_rfc3339("2026-08-10T07:58:00+00:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let attestation = IntegrationAttestation::new(
        IntegrationMethod::DistributorCherryPick,
        record.target_shas().source_sha().clone(),
        Some(record.target_shas().base_sha().clone()),
        evidence_sha,
        Some(pull_request_number),
        None,
        integrated_at,
        integrated_at + chrono::Duration::seconds(5),
    )
    .unwrap();
    record
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::IntegrationAttested(attestation))
        .unwrap()
}

fn persist_admin_validation_record(
    adapter: &SqlitePlanningAuthorityAdapter,
    workspace: &TempAdminWorkspace,
    record: &PrValidationRecord,
) {
    assert!(
        adapter
            .compare_and_swap_runtime_pr_validation_record(
                &workspace.path,
                record.key(),
                None,
                Some(record),
            )
            .expect("admin validation fixture should persist")
    );
}

fn admin_validation_observation(
    required_status: PrValidationObservedCheckStatus,
) -> PrValidationObservationProjection {
    PrValidationObservationProjection::new(
        vec![PrValidationObservedCheck::new(
            PrValidationCheckContext::new(Some("github-actions".to_string()), "Post-Merge Gate")
                .unwrap(),
            required_status,
            Some(3),
            Some("2026-08-10T08:00:00+00:00".to_string()),
            (required_status == PrValidationObservedCheckStatus::Succeeded)
                .then(|| "2026-08-10T08:02:00+00:00".to_string()),
        )],
        vec![PrValidationObservedCheck::new(
            PrValidationCheckContext::new(Some("github-actions".to_string()), "CI Scope").unwrap(),
            PrValidationObservedCheckStatus::Succeeded,
            Some(2),
            Some("2026-08-10T07:59:00+00:00".to_string()),
            Some("2026-08-10T08:01:00+00:00".to_string()),
        )],
        vec![
            PrValidationObservedWorkflow::new(
                "Post-Merge Validation",
                if required_status == PrValidationObservedCheckStatus::Succeeded {
                    PrValidationObservedRunStatus::Succeeded
                } else {
                    PrValidationObservedRunStatus::InProgress
                },
                3,
                Some("2026-08-10T08:00:00+00:00".to_string()),
                Some("2026-08-10T08:02:00+00:00".to_string()),
            )
            .unwrap(),
        ],
        vec![
            PrValidationObservedProvider::new(
                "github:CheckRuns",
                PrValidationObservedProviderLifecycle::Finite,
                if required_status == PrValidationObservedCheckStatus::Succeeded {
                    PrValidationObservedProviderStatus::Complete
                } else {
                    PrValidationObservedProviderStatus::Pending
                },
                true,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[tokio::test]
async fn admin_pr_validation_board_endpoint_is_available_when_empty() {
    let workspace = TempAdminWorkspace::new("pr-validation-board-empty");
    let router = admin_test_router(&workspace);

    let response = router
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/validations?limit=20")
                .body(Body::empty())
                .expect("validation board request should build"),
        )
        .await
        .expect("validation board request should be served");

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["records"], json!([]));
    assert_eq!(body["summary"]["active"], 0);
    assert_eq!(body["cursorResetRequired"], false);
}

#[tokio::test]
async fn admin_pr_validation_rollout_evidence_is_typed_bounded_and_redacted() {
    let workspace = TempAdminWorkspace::new_git("pr-validation-rollout-evidence");
    write_rollout_evidence_fixture(&workspace);
    let router = admin_test_router(&workspace);

    let response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/pr-validation/evidence?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["latest"]["summary"]["status"], "ready");
    assert_eq!(
        body["latest"]["summary"]["historicalFastGate"]["label"],
        "projected"
    );
    assert_eq!(
        body["latest"]["summary"]["actualFastGate"]["label"],
        "actual"
    );
    assert_eq!(body["latest"]["summary"]["evidenceShortSha"], "01234567");
    assert_eq!(body["history"].as_array().unwrap().len(), 1);
    let serialized = body.to_string();
    assert!(!serialized.contains("evidence.json"));
    assert!(!serialized.contains("0123456789abcdef0123456789abcdef01234567"));

    for uri in [
        "/api/admin/akra/pr-validation/evidence?limit=21",
        "/api/admin/akra/pr-validation/evidence?limit=1&cursor=malformed",
    ] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let dashboard_response = router
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dashboard_response.status(), StatusCode::OK);
    let dashboard = json_body(dashboard_response).await;
    assert_eq!(
        dashboard["validation"]["rolloutEvidence"]["status"],
        "ready"
    );
    assert!(dashboard["validation"].get("history").is_none());
}

#[tokio::test]
async fn admin_pr_validation_board_pages_phases_evidence_correlations_and_redaction() {
    let workspace = TempAdminWorkspace::new("pr-validation-board-records");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let integrated = admin_integrated_validation_record("validation-integrated", 301);
    let verifying = admin_integrated_validation_record("validation-verifying", 302)
        .transition(PrValidationEvent::ObservationProjected(
            admin_validation_observation(PrValidationObservedCheckStatus::Pending),
        ))
        .unwrap()
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: 7,
            cursor: Some("provider-cursor-secret".to_string()),
            evidence_fingerprint: "safe-fingerprint".to_string(),
        })
        .unwrap();
    let queued_base = admin_integrated_validation_record("validation-queued", 303);
    let queued_finding = PrValidationFinding::new(
        PrValidationFindingKey::new(
            PrValidationFindingSource::new("review").unwrap(),
            "provider-event-secret",
        )
        .unwrap(),
        queued_base.target_shas().source_sha().clone(),
        "raw-body-secret",
    )
    .unwrap();
    let queued = queued_base
        .transition(PrValidationEvent::FindingObserved(queued_finding.clone()))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                queued_finding.key().clone(),
                PrValidationRecordKey::new("remediation-task-queued").unwrap(),
            ),
        ))
        .unwrap();
    let running_base = admin_integrated_validation_record("validation-running", 304);
    let running_finding = PrValidationFinding::new(
        PrValidationFindingKey::new(
            PrValidationFindingSource::new("check_run").unwrap(),
            "provider-run-secret",
        )
        .unwrap(),
        running_base.target_shas().source_sha().clone(),
        "raw-running-body-secret",
    )
    .unwrap();
    let running_key = running_finding.key().clone();
    let running = running_base
        .transition(PrValidationEvent::FindingObserved(running_finding))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                running_key.clone(),
                PrValidationRecordKey::new("remediation-task-running").unwrap(),
            ),
        ))
        .unwrap()
        .transition(PrValidationEvent::RemediationStarted {
            finding_key: running_key,
        })
        .unwrap();
    let verified_observing = admin_integrated_validation_record("validation-verified", 305)
        .transition(PrValidationEvent::ObservationProjected(
            admin_validation_observation(PrValidationObservedCheckStatus::Succeeded),
        ))
        .unwrap()
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: 9,
            cursor: None,
            evidence_fingerprint: "verified-fingerprint".to_string(),
        })
        .unwrap();
    let verified = verified_observing
        .transition(PrValidationEvent::Settle(PrValidationCompletion::new(
            verified_observing.evidence_sha().unwrap().clone(),
            vec![PrValidationProviderCompletion::terminal(
                PrValidationProviderKey::new("github:CheckRuns").unwrap(),
            )],
            Vec::new(),
            PrValidationCatchUpState::NoUnseenRelevantEvents,
        )))
        .unwrap();
    let provider_blocked = admin_integrated_validation_record("validation-provider-blocked", 306);

    for record in [
        &integrated,
        &verifying,
        &queued,
        &running,
        &verified,
        &provider_blocked,
    ] {
        persist_admin_validation_record(&adapter, &workspace, record);
    }
    adapter
        .upsert_runtime_slot_lease(
            &workspace.path,
            &ParallelModeSlotLeaseSnapshot::new(
                "slot-validation",
                "remediation-task-running",
                "Repair post-merge check",
                "agent-validation",
                "akra-agent/slot-validation/remediation",
                "C:/tmp/slot-validation",
                ParallelModeSlotLeaseState::Leased,
                "2026-08-10T08:10:00+00:00",
                None,
            ),
        )
        .unwrap();
    let location = adapter.resolve_authority_location(&workspace.path).unwrap();
    let connection = Connection::open(&location.authority_store_path).unwrap();
    for (key, updated_at) in [
        ("validation-integrated", "2026-08-10T10:00:00+00:00"),
        ("validation-verifying", "2026-08-10T11:00:00+00:00"),
        ("validation-queued", "2026-08-10T12:00:00+00:00"),
        ("validation-running", "2026-08-10T13:00:00+00:00"),
        ("validation-provider-blocked", "2026-08-10T14:00:00+00:00"),
        ("validation-verified", "2026-08-10T15:00:00+00:00"),
    ] {
        connection
            .execute(
                "UPDATE runtime_pr_validation_records SET updated_at = ?2 WHERE record_key = ?1",
                (key, updated_at),
            )
            .unwrap();
    }
    connection
        .execute(
            "UPDATE runtime_pr_validation_records
             SET last_polled_at = '2026-08-10T14:00:00+00:00',
                 next_poll_at = '2026-08-10T14:05:00+00:00',
                 poll_attempt = 4,
                 consecutive_error_count = 2,
                 last_error_class = 'authentication_blocked',
                 rate_limit_remaining = 17,
                 rate_limit_reset_at = '2026-08-10T14:10:00+00:00',
                 poll_lease_owner = 'poll-owner-secret',
                 poll_lease_token = 'poll-lease-token-secret',
                 poll_lease_expires_at = '2026-08-10T14:01:00+00:00'
             WHERE record_key = 'validation-provider-blocked'",
            [],
        )
        .unwrap();
    drop(connection);

    let router = admin_test_router(&workspace);
    let dashboard_response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dashboard_response.status(), StatusCode::OK);
    let dashboard = json_body(dashboard_response).await;
    assert_eq!(dashboard["validation"]["summary"]["active"], 5);
    assert_eq!(dashboard["validation"]["summary"]["verified"], 1);

    let first_response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/validations?limit=3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_response.status(), StatusCode::OK);
    let first = json_body(first_response).await;
    assert_eq!(first["records"].as_array().unwrap().len(), 3);
    let cursor = first["nextCursor"]
        .as_str()
        .expect("first bounded page should have a cursor");
    let second_response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/admin/akra/validations?limit=3&cursor={cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_response.status(), StatusCode::OK);
    let second = json_body(second_response).await;
    assert_eq!(second["records"].as_array().unwrap().len(), 3);

    let records = first["records"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["records"].as_array().unwrap())
        .collect::<Vec<_>>();
    let phases = records
        .iter()
        .filter_map(|record| record["phase"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for expected in [
        "integrated",
        "verifying",
        "remediation_queued",
        "remediation_running",
        "verified",
    ] {
        assert!(
            phases.contains(expected),
            "missing phase {expected}: {phases:?}"
        );
    }
    let verifying_json = records
        .iter()
        .find(|record| record["recordKey"] == "validation-verifying")
        .unwrap();
    assert_eq!(verifying_json["latestRequiredAttempt"], 3);
    assert_eq!(verifying_json["requiredChecksTotal"], 1);
    assert!(verifying_json["evidenceShortSha"].as_str().is_some());
    for detail_only in [
        "checks",
        "evidenceSha",
        "baseBeforeSha",
        "schedule",
        "correlations",
    ] {
        assert!(
            verifying_json.get(detail_only).is_none(),
            "board record leaked detail-only field {detail_only}"
        );
    }
    let provider_json = records
        .iter()
        .find(|record| record["recordKey"] == "validation-provider-blocked")
        .unwrap();
    assert_eq!(provider_json["providerBlocked"], true);
    assert_eq!(provider_json["severity"], "danger");

    let verifying_detail_response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/validations/validation-verifying")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(verifying_detail_response.status(), StatusCode::OK);
    let verifying_detail = json_body(verifying_detail_response).await;
    assert_eq!(verifying_detail["checks"][0]["latestAttempt"], 3);
    assert_eq!(
        verifying_detail["workflows"][0]["selectionBasis"],
        "legacy_unknown"
    );
    assert_eq!(verifying_detail["workflows"][0]["selected"], true);
    assert!(verifying_detail["workflows"][0]["createdAt"].is_null());
    assert_eq!(verifying_detail["evidenceSha"].as_str().unwrap().len(), 40);
    assert_eq!(verifying_detail["integrationPullRequestNumber"], 302);
    assert_eq!(
        verifying_detail["baseBeforeSha"].as_str().unwrap().len(),
        40
    );
    assert_eq!(
        verifying_detail["integratedAt"],
        "2026-08-10T07:58:00+00:00"
    );
    assert_eq!(
        verifying_detail["remoteVerifiedAt"],
        "2026-08-10T07:58:05+00:00"
    );

    let detail_response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/validations/validation-running")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail_response.status(), StatusCode::OK);
    let detail = json_body(detail_response).await;
    assert_eq!(detail["correlations"][0]["slotId"], "slot-validation");
    assert_eq!(detail["correlations"][0]["taskState"], "leased");

    let visible_json = format!("{first}\n{second}\n{verifying_detail}\n{detail}");
    for secret in [
        "raw-body-secret",
        "raw-running-body-secret",
        "provider-event-secret",
        "provider-run-secret",
        "provider-cursor-secret",
        "poll-owner-secret",
        "poll-lease-token-secret",
    ] {
        assert!(
            !visible_json.contains(secret),
            "admin validation JSON leaked {secret}"
        );
    }

    let new_record = admin_validation_record("validation-new", 307)
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap();
    persist_admin_validation_record(&adapter, &workspace, &new_record);
    let reset_response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/admin/akra/validations?limit=3&cursor={cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset_response.status(), StatusCode::OK);
    assert_eq!(json_body(reset_response).await["cursorResetRequired"], true);
}

#[tokio::test]
async fn admin_pr_validation_board_rejects_malformed_opaque_cursor() {
    let workspace = TempAdminWorkspace::new("pr-validation-invalid-cursor");
    let response = admin_test_router(&workspace)
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/api/admin/akra/validations?cursor=not-a-valid-cursor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_pr_validation_commands_require_csrf_and_expose_typed_conflicts() {
    let workspace = TempAdminWorkspace::new("pr-validation-command-api");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let base = admin_integrated_validation_record("validation-command-api", 308);
    let finding = PrValidationFinding::new(
        PrValidationFindingKey::new(
            PrValidationFindingSource::new("required_check").unwrap(),
            "Fast Gate",
        )
        .unwrap(),
        base.target_shas().source_sha().clone(),
        "actionable check failure",
    )
    .unwrap();
    let record = base
        .transition(PrValidationEvent::FindingObserved(finding))
        .unwrap();
    let expected_revision = record.observation_revision();
    persist_admin_validation_record(&adapter, &workspace, &record);

    let router = admin_test_router(&workspace);
    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;
    let endpoint = "/api/admin/akra/validations/validation-command-api/commands";
    let pause_body = json!({
        "commandId": "api-validation-pause",
        "action": "pause",
        "expectedRevision": expected_revision,
    });

    let forbidden = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            endpoint,
            pause_body.clone(),
            Some(&cookie),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let accepted = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            endpoint,
            pause_body.clone(),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted = json_body(accepted).await;
    assert_eq!(accepted["state"], "applied");
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["observedRevision"], expected_revision);

    let replay = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            endpoint,
            pause_body,
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(json_body(replay).await["duplicate"], true);

    let idempotency_conflict = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            endpoint,
            json!({
                "commandId": "api-validation-pause",
                "action": "resume",
                "expectedRevision": expected_revision,
            }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .unwrap();
    assert_eq!(idempotency_conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(idempotency_conflict).await["rejection"],
        "idempotency_conflict"
    );

    let stale = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            endpoint,
            json!({
                "commandId": "api-validation-stale",
                "action": "acknowledge",
                "expectedRevision": expected_revision.saturating_add(1),
            }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(stale).await["rejection"], "stale_revision");

    let observe_admission = router
        .oneshot(json_request(
            Method::POST,
            endpoint,
            json!({
                "commandId": "api-validation-observe",
                "action": "queue_remediation",
                "expectedRevision": expected_revision,
            }),
            Some(&cookie),
            Some(&csrf_token),
        ))
        .await
        .unwrap();
    assert_eq!(observe_admission.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(observe_admission).await["rejection"],
        "observe_mode_admission"
    );
}

#[tokio::test]
async fn admin_html_page_routes_render_live_templates() {
    let workspace = TempAdminWorkspace::new("html-pages");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token, dashboard_body) = bootstrap_admin_html_session(&router).await;

    assert!(dashboard_body.contains("Planning Admin"));
    assert!(dashboard_body.contains("Open Full Planning Draft"));
    assert!(dashboard_body.contains(r#"<a href="/admin/akra">Graphic dashboard</a>"#));
    assert!(dashboard_body.contains("name=\"csrf_token\""));
    assert_eq!(csrf_token.len(), 32);

    for (uri, expected) in [
        ("/admin?notice=hello", "hello"),
        ("/admin/directions", "Directions"),
        ("/admin/tasks", "Task catalog view"),
        ("/admin/reviews", "Shared review-center inbox"),
        ("/admin/controls", "Controls"),
        ("/admin/app-server-prompts", "App-server prompt I/O"),
        ("/admin/akra", "data-admin-graphic"),
        ("/admin/akra/metrics", "AKRA detached metrics"),
        ("/admin/akra/directions", "게임발전국 작전 방향"),
        ("/admin/akra/tasks", "게임발전국 작업 관리"),
    ] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(uri)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .expect("HTML page request should build"),
            )
            .await
            .expect("HTML page request should be served");
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = text_body(response).await;
        assert!(
            content_type.starts_with("text/html"),
            "HTML page should use text/html for {uri}: {content_type}"
        );
        assert!(
            body.contains(expected),
            "HTML page {uri} should render {expected}"
        );
        if uri == "/admin/tasks" {
            assert!(body.contains(r#"<a href="/admin/tasks" class="active">Tasks</a>"#));
            assert!(body.contains(r#"<a href="/admin/akra">Graphic dashboard</a>"#));
            assert!(!body.contains(r#"<body class="akra-graphic">"#));
            assert!(!body.contains(r#"<aside class="sidebar" lang="ko">"#));
        }
        if uri == "/admin/akra" {
            assert!(
                body.contains(r#"<nav class="nav graphic-nav" aria-label="Admin navigation">"#)
            );
            assert!(body.contains(r#"<body class="akra-graphic akra-dashboard-page">"#));
            assert!(body.contains(r#"<aside class="sidebar" lang="ko">"#));
            assert!(body.contains(r#"class="akra-game""#));
            assert!(body.contains(r#"lang="ko""#));
            assert!(body.contains(r#"<a href="/admin/akra" class="active" aria-current="page">"#));
            assert!(body.contains("href=\"/admin/akra/directions\""));
            assert!(body.contains("href=\"/admin/akra/tasks\""));
            assert!(body.contains("href=\"/admin/akra/metrics#metrics\""));
            assert!(body.contains(r#"<a href="/admin"><span class="nav-icon" aria-hidden="true">P</span><span>Planning</span></a>"#));
            assert!(body.contains(r#"<a href="/admin/controls"><span class="nav-icon" aria-hidden="true">O</span><span>Controls</span></a>"#));
        }
        if uri == "/admin/app-server-prompts" {
            assert!(body.contains(
                r#"<a href="/admin/app-server-prompts" class="active">App-server I/O</a>"#
            ));
        }
        if uri == "/admin/akra/directions" {
            assert!(body.contains(r#"<body class="akra-graphic">"#));
            assert!(body.contains(r#"<a href="/admin/akra/directions" class="active" aria-current="page"><span class="nav-icon" aria-hidden="true">G</span><span>작전 방향</span></a>"#));
            assert!(!body.contains(r#"<a href="/admin/directions" class="active">Directions</a>"#));
        }
        if uri == "/admin/akra/tasks" {
            assert!(body.contains(r#"<body class="akra-graphic">"#));
            assert!(body.contains(r#"<a href="/admin/akra/tasks" class="active" aria-current="page"><span class="nav-icon" aria-hidden="true">T</span><span>작업 관리</span></a>"#));
            assert!(!body.contains(r#"<a href="/admin/tasks" class="active">Tasks</a>"#));
        }
        if uri == "/admin/reviews" {
            assert!(body.contains(r#"<a href="/admin/reviews" class="active">Reviews</a>"#));
            assert!(body.contains("Top pending inbox thread"));
            assert!(!body.contains(r#"<body class="akra-graphic">"#));
        }
    }
}

#[tokio::test]
async fn reviews_page_renders_top_pending_inbox_thread_content_from_repository_projection() {
    let workspace = TempAdminWorkspace::new("reviews-page");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let mut thread_review = ReviewCenterThreadProjection::new(
        "thread-1",
        "review-1",
        "Manual review",
        "pending",
        "Need operator follow-up",
        "2026-07-06T10:00:00Z",
        "2026-07-06T10:01:00Z",
    );
    thread_review.handoff_target = Some("operator".to_string());
    thread_review.handoff_note = Some("open review center inbox".to_string());
    let mut inbox_item = ReviewCenterInboxItem::new(
        "review-1",
        "thread-1",
        "pending",
        "Need operator follow-up",
        "2026-07-06T10:00:00Z",
        "2026-07-06T10:01:00Z",
    );
    inbox_item.handoff_target = Some("operator".to_string());

    adapter
        .upsert_thread_review(&workspace.path, &thread_review)
        .expect("thread review should persist");
    adapter
        .replace_pending_inbox(&workspace.path, &[inbox_item])
        .expect("inbox should persist");

    let router = admin_test_router(&workspace);
    let (cookie, _, _) = bootstrap_admin_html_session(&router).await;
    let response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/admin/reviews")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("reviews page request should build"),
        )
        .await
        .expect("reviews page request should be served");
    let body = text_body(response).await;

    assert!(body.contains("Top pending inbox thread"));
    assert!(
        body.contains(
            "first inbox item's current review context, not the local active shell thread"
        )
    );
    assert!(body.contains("Manual review"));
    assert!(body.contains("Need operator follow-up"));
    assert!(body.contains("handoff → operator: open review center inbox"));
}

#[tokio::test]
async fn admin_graphic_asset_routes_serve_known_assets_and_reject_unknown_names() {
    let workspace = TempAdminWorkspace::new("asset-routes");
    let router = admin_test_router(&workspace);

    for asset_name in [
        "akra-operations-studio-v3.png",
        "gamebaljeonguk_atlas_64x96.png",
        "gamebaljeonguk_atlas_128x192.png",
    ] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(format!("/admin/assets/graphics/{asset_name}"))
                    .body(Body::empty())
                    .expect("asset request should build"),
            )
            .await
            .expect("asset request should be served");

        assert_eq!(response.status(), StatusCode::OK, "{asset_name}");
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&header::HeaderValue::from_static("image/png")),
            "{asset_name}"
        );
        assert_bundled_asset_cache_headers(&response);
        let body = bytes_body(response).await;
        assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"), "{asset_name}");
    }

    for removed_or_unknown in [
        "akra-object-sprites.png",
        "akra-office-background.png",
        "sprite_desk_workstation.png",
        "sprite_floor_tile.png",
        "sprite_potted_plant.png",
        "sprite_server_rack.png",
        "sprite_sofa.png",
        "sprite_whiteboard.png",
        "not-found.png",
    ] {
        let missing = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(format!("/admin/assets/graphics/{removed_or_unknown}"))
                    .body(Body::empty())
                    .expect("missing asset request should build"),
            )
            .await
            .expect("missing asset request should be served");
        assert_eq!(
            missing.status(),
            StatusCode::NOT_FOUND,
            "{removed_or_unknown}"
        );
    }
}

#[tokio::test]
async fn admin_game_asset_route_serves_diorama_bundle_and_rejects_unknown_names() {
    let workspace = TempAdminWorkspace::new("game-asset-routes");
    let router = admin_test_router(&workspace);

    let response = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/admin/assets/game/akra-diorama.js")
                .body(Body::empty())
                .expect("game asset request should build"),
        )
        .await
        .expect("game asset request should be served");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&header::HeaderValue::from_static(
            "text/javascript; charset=utf-8"
        ))
    );
    assert_bundled_asset_cache_headers(&response);
    let body = text_body(response).await;
    assert!(body.contains("AkraAdminGame"));
    assert!(body.contains("akra-operations-studio-v3.png"));
    assert!(body.contains("gamebaljeonguk_atlas_128x192.png"));
    assert!(body.contains("PixiJS - The MIT License"));
    assert!(body.len() > 100_000, "PixiJS should be bundled locally");

    let missing = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/admin/assets/game/missing.js")
                .body(Body::empty())
                .expect("missing game asset request should build"),
        )
        .await
        .expect("missing game asset request should be served");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_script_asset_routes_serve_externalized_code_and_reject_unknown_names() {
    let workspace = TempAdminWorkspace::new("script-asset-routes");
    let router = admin_test_router(&workspace);

    for (asset_name, expected_token) in [
        ("admin-shell.js", "akraHashTabRoutes"),
        ("akra-dashboard.js", "renderDashboardPanels"),
    ] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(format!("/admin/assets/scripts/{asset_name}"))
                    .body(Body::empty())
                    .expect("script asset request should build"),
            )
            .await
            .expect("script asset request should be served");
        assert_eq!(response.status(), StatusCode::OK, "{asset_name}");
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&header::HeaderValue::from_static(
                "text/javascript; charset=utf-8"
            )),
            "{asset_name}"
        );
        assert_bundled_asset_cache_headers(&response);
        assert!(text_body(response).await.contains(expected_token));
    }

    let missing = router
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/admin/assets/scripts/missing.js")
                .body(Body::empty())
                .expect("missing script asset request should build"),
        )
        .await
        .expect("missing script asset request should be served");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_font_asset_routes_serve_bundled_korean_fonts_and_reject_unknown_names() {
    let workspace = TempAdminWorkspace::new("font-asset-routes");
    let router = admin_test_router(&workspace);
    assert_eq!(
        BASE_TEMPLATE.matches("font-display: swap;").count(),
        2,
        "both bundled font weights must keep first paint non-blocking"
    );

    for asset_name in ["Galmuri11.woff2", "Galmuri11-Bold.woff2"] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(format!("/admin/assets/fonts/{asset_name}"))
                    .body(Body::empty())
                    .expect("font asset request should build"),
            )
            .await
            .expect("font asset request should be served");
        assert_eq!(response.status(), StatusCode::OK, "{asset_name}");
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&header::HeaderValue::from_static("font/woff2")),
            "{asset_name}"
        );
        assert_bundled_asset_cache_headers(&response);
        assert!(bytes_body(response).await.len() > 100_000, "{asset_name}");
    }

    let missing = router
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri("/admin/assets/fonts/missing.woff2")
                .body(Body::empty())
                .expect("missing font asset request should build"),
        )
        .await
        .expect("missing font asset request should be served");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bundled_admin_assets_revalidate_with_content_etag_and_empty_304() {
    let workspace = TempAdminWorkspace::new("asset-etag");
    let router = admin_test_router(&workspace);

    for asset_path in [
        "/admin/assets/graphics/akra-operations-studio-v3.png",
        "/admin/assets/game/akra-diorama.js",
        "/admin/assets/scripts/admin-shell.js",
        "/admin/assets/fonts/Galmuri11.woff2",
    ] {
        let initial = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(asset_path)
                    .body(Body::empty())
                    .expect("initial asset request should build"),
            )
            .await
            .expect("initial asset request should be served");
        assert_eq!(initial.status(), StatusCode::OK, "{asset_path}");
        let etag = assert_bundled_asset_cache_headers(&initial);

        let not_modified = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(asset_path)
                    .header(header::IF_NONE_MATCH, etag.clone())
                    .body(Body::empty())
                    .expect("conditional asset request should build"),
            )
            .await
            .expect("conditional asset request should be served");
        assert_eq!(
            not_modified.status(),
            StatusCode::NOT_MODIFIED,
            "{asset_path}"
        );
        assert_eq!(
            assert_bundled_asset_cache_headers(&not_modified),
            etag,
            "{asset_path}"
        );
        assert!(
            bytes_body(not_modified).await.is_empty(),
            "304 asset response must not transfer the bundled body: {asset_path}"
        );

        let changed_validator = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(asset_path)
                    .header(header::IF_NONE_MATCH, "\"sha256-stale\"")
                    .body(Body::empty())
                    .expect("stale validator request should build"),
            )
            .await
            .expect("stale validator request should be served");
        assert_eq!(changed_validator.status(), StatusCode::OK, "{asset_path}");
        assert_eq!(
            assert_bundled_asset_cache_headers(&changed_validator),
            etag,
            "{asset_path}"
        );
    }
}

#[tokio::test]
async fn admin_html_form_routes_redirect_through_shared_facade() {
    let workspace = TempAdminWorkspace::new("html-forms");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token, _) = bootstrap_admin_html_session(&router).await;

    let forbidden = router
        .clone()
        .oneshot(html_form_request(
            "/admin/controls/reset",
            encoded_form(&[("csrf_token", csrf_token.as_str()), ("target", "queue")]),
            None,
            false,
        ))
        .await
        .expect("CSRF failure should be served");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let direction_upsert = router
        .clone()
        .oneshot(html_form_request(
            "/admin/directions/upsert",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", "html-direction"),
                ("title", "HTML Direction"),
                ("summary", "Rendered through HTML form"),
                ("success_criteria_text", "direction is editable"),
                ("scope_hints_text", "admin"),
                ("detail_doc_path", "docs/html-direction.md"),
                ("state", "active"),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("direction upsert should be served");
    assert_eq!(direction_upsert.status(), StatusCode::SEE_OTHER);
    assert!(
        direction_upsert
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|location| location.starts_with("/admin/directions?notice="))
    );

    let akra_direction_upsert = router
        .clone()
        .oneshot(html_form_request(
            "/admin/akra/directions/upsert",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", "akra-html-direction"),
                ("title", "AKRA HTML Direction"),
                ("summary", "Rendered through the graphic admin form"),
                ("success_criteria_text", "graphic direction is editable"),
                ("scope_hints_text", "admin,akra"),
                ("detail_doc_path", "docs/akra-html-direction.md"),
                ("state", "active"),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("AKRA direction upsert should be served");
    assert_eq!(akra_direction_upsert.status(), StatusCode::SEE_OTHER);
    assert!(
        akra_direction_upsert
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|location| location.starts_with("/admin/akra/directions?notice="))
    );

    let task_upsert = router
        .clone()
        .oneshot(html_form_request(
            "/admin/tasks/upsert",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", ""),
                ("direction_id", "html-direction"),
                ("title", "HTML Task"),
                ("description", "Created through the browser adapter"),
                ("status", "ready"),
                ("base_priority", "60"),
                ("dynamic_priority_delta", "0"),
                ("priority_reason", ""),
                ("depends_on_text", ""),
                ("blocked_by_text", ""),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("task upsert should be served");
    assert_eq!(task_upsert.status(), StatusCode::SEE_OTHER);
    let task_location = task_upsert
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("task create should redirect with generated task id");
    let task_notice = percent_encoding::percent_decode_str(task_location)
        .decode_utf8_lossy()
        .to_string();
    let task_id = task_notice
        .split('`')
        .nth(1)
        .expect("task create notice should include generated task id")
        .to_string();

    let akra_task_upsert = router
        .clone()
        .oneshot(html_form_request(
            "/admin/akra/tasks/upsert",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", ""),
                ("direction_id", "akra-html-direction"),
                ("title", "AKRA HTML Task"),
                ("description", "Created through the graphic browser adapter"),
                ("status", "ready"),
                ("base_priority", "60"),
                ("dynamic_priority_delta", "0"),
                ("priority_reason", ""),
                ("depends_on_text", ""),
                ("blocked_by_text", ""),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("AKRA task upsert should be served");
    assert_eq!(akra_task_upsert.status(), StatusCode::SEE_OTHER);
    assert!(
        akra_task_upsert
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|location| location.starts_with("/admin/akra/tasks?notice="))
    );
    let akra_task_location = akra_task_upsert
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("AKRA task create should redirect with generated task id");
    let akra_task_notice = percent_encoding::percent_decode_str(akra_task_location)
        .decode_utf8_lossy()
        .to_string();
    let akra_task_id = akra_task_notice
        .split('`')
        .nth(1)
        .expect("AKRA task create notice should include generated task id")
        .to_string();

    for (uri, body, location_prefix) in [
        (
            "/admin/files/export",
            encoded_form(&[("csrf_token", csrf_token.as_str())]),
            "/admin/controls?notice=",
        ),
        (
            "/admin/files/apply",
            encoded_form(&[("csrf_token", csrf_token.as_str())]),
            "/admin/controls?notice=",
        ),
        (
            "/admin/tasks/delete",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", task_id.as_str()),
            ]),
            "/admin/tasks?notice=",
        ),
        (
            "/admin/akra/tasks/delete",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", akra_task_id.as_str()),
            ]),
            "/admin/akra/tasks?notice=",
        ),
        (
            "/admin/directions/delete",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", "html-direction"),
            ]),
            "/admin/directions?notice=",
        ),
        (
            "/admin/akra/directions/delete",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("id", "akra-html-direction"),
            ]),
            "/admin/akra/directions?notice=",
        ),
        (
            "/admin/controls/reset",
            encoded_form(&[("csrf_token", csrf_token.as_str()), ("target", "queue")]),
            "/admin/controls?notice=planning%20workspace%20reset",
        ),
    ] {
        let response = router
            .clone()
            .oneshot(html_form_request(uri, body, Some(&cookie), false))
            .await
            .expect("HTML mutation should be served");
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{uri}");
        assert!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|location| location.starts_with(location_prefix)),
            "{uri} should redirect to {location_prefix}"
        );
    }

    let invalid_profiles = router
        .clone()
        .oneshot(html_form_request(
            "/admin/controls/agent-profiles",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("profiles_json", "{not json"),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("invalid agent profile form should be served");
    assert_eq!(invalid_profiles.status(), StatusCode::BAD_REQUEST);

    let valid_profiles = router
        .clone()
        .oneshot(html_form_request(
            "/admin/controls/agent-profiles",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                (
                    "profiles_json",
                    r#"{"profiles":[{"agent_id":"agent-html","display_name":"HTML Agent","role":"reviewer","persona_prompt":"Check admin pages","avatar_class":"Scribe","capabilities":["admin"],"enabled":true}]}"#,
                ),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("valid agent profile form should be served");
    assert_eq!(valid_profiles.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        valid_profiles
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/admin/controls?notice=parallel%20agent%20profiles%20saved")
    );
}

#[tokio::test]
async fn admin_html_draft_routes_render_editor_and_validation_responses() {
    let workspace = TempAdminWorkspace::new_git("html-drafts");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token, _) = bootstrap_admin_html_session(&router).await;

    let created = router
        .clone()
        .oneshot(html_form_request(
            "/admin/drafts",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("kind", "full_planning"),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("draft create should be served");
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    let editor_location = created
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("draft create should redirect to editor")
        .to_string();
    let editor_path = editor_location
        .split('?')
        .next()
        .expect("editor location should include path");
    assert!(editor_path.starts_with("/admin/drafts/"));

    let loaded = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri(editor_location.as_str())
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("editor page request should build"),
        )
        .await
        .expect("editor page request should be served");
    assert_eq!(loaded.status(), StatusCode::OK);
    let loaded_body = text_body(loaded).await;
    let save_uri = format!("{editor_path}/save");
    let validate_uri = format!("{editor_path}/validate");
    let promote_uri = format!("{editor_path}/promote");
    assert!(loaded_body.contains("file_result_output"));
    assert!(loaded_body.contains(&format!("action=\"{save_uri}\"")));
    assert!(loaded_body.contains(&format!("formaction=\"{validate_uri}\"")));
    assert!(loaded_body.contains(&format!("formaction=\"{promote_uri}\"")));

    let draft_body = "# Planning\n\n## Result\n\nHTML draft round trip\n";
    let saved = router
        .clone()
        .oneshot(html_form_request(
            &save_uri,
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("kind", "full_planning"),
                ("file_result_output", draft_body),
            ]),
            Some(&cookie),
            true,
        ))
        .await
        .expect("HTMX draft save should be served");
    assert_eq!(saved.status(), StatusCode::OK);
    assert!(text_body(saved).await.contains("draft saved"));

    let validated = router
        .clone()
        .oneshot(html_form_request(
            &validate_uri,
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("kind", "full_planning"),
                ("file_result_output", draft_body),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("draft validate should be served");
    assert_eq!(validated.status(), StatusCode::OK);
    assert!(text_body(validated).await.contains("draft validated"));

    let promoted = router
        .clone()
        .oneshot(html_form_request(
            &promote_uri,
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("kind", "full_planning"),
                ("file_result_output", draft_body),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("draft promote should be served");
    assert_eq!(promoted.status(), StatusCode::OK);
    assert!(text_body(promoted).await.contains("file_result_output"));
}

#[test]
fn editor_template_uses_shared_encoded_mutation_paths() {
    let draft_name = "draft name/with?encoding#needed";
    let action_paths = EditorActionPaths {
        save: draft_mutation_path(draft_name, "save"),
        validate: draft_mutation_path(draft_name, "validate"),
        promote: draft_mutation_path(draft_name, "promote"),
    };
    let rendered = EditorTemplate {
        page_title: "Editor".to_string(),
        current_nav: nav_for_kind(PlanningAdminDraftKind::FullPlanning),
        workspace_dir: "/workspace".to_string(),
        csrf_token: "csrf".to_string(),
        notice: None,
        editor_surface_token: Some("akra"),
        action_paths: action_paths.clone(),
        session: PlanningAdminSessionView {
            kind: PlanningAdminDraftKind::FullPlanning,
            direction_id: None,
            draft_name: draft_name.to_string(),
            draft_directory: "/workspace/drafts/synthetic".to_string(),
            editor_heading: "Full Planning Draft".to_string(),
            return_path: "/admin/akra/directions".to_string(),
            files: vec![PlanningAdminDraftFileView {
                key: PlanningAdminFileKey::ResultOutput,
                label: "Result Output".to_string(),
                active_path: "planning/result_output.md".to_string(),
                editor_language: "markdown".to_string(),
                body: "# Synthetic".to_string(),
            }],
            validation: PlanningAdminValidationView {
                is_valid: true,
                error_count: 0,
                warning_count: 0,
                issues: Vec::new(),
            },
            queue_preview: None,
        },
    }
    .render()
    .expect("editor template should render");
    assert!(rendered.contains(&format!("action=\"{}\"", action_paths.save)));
    assert!(rendered.contains(&format!("formaction=\"{}\"", action_paths.validate)));
    assert!(rendered.contains(&format!("formaction=\"{}\"", action_paths.promote)));
    assert!(rendered.contains(r#"name="surface" value="akra""#));
    assert!(rendered.contains(r#"href="/admin/akra/directions""#));
    assert!(!rendered.contains(&format!("action=\"/admin/drafts/{draft_name}/save\"")));
}

#[tokio::test]
async fn akra_html_draft_routes_preserve_surface_continuity() {
    let workspace = TempAdminWorkspace::new_git("akra-html-drafts");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token, _) = bootstrap_admin_html_session(&router).await;

    let created = router
        .clone()
        .oneshot(html_form_request(
            "/admin/drafts",
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("kind", "queue_idle_prompt"),
                ("surface", "akra"),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("akra draft create should be served");
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    let editor_location = created
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("akra draft create should redirect to editor")
        .to_string();
    assert!(editor_location.contains("surface=akra"));
    let editor_path = editor_location
        .split('?')
        .next()
        .expect("editor location should include path");

    let loaded = router
        .clone()
        .oneshot(
            admin_request_builder()
                .method(Method::GET)
                .uri(editor_location.as_str())
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("AKRA editor page request should build"),
        )
        .await
        .expect("AKRA editor page request should be served");
    assert_eq!(loaded.status(), StatusCode::OK);
    let loaded_body = text_body(loaded).await;
    let validate_uri = format!("{editor_path}/validate");
    assert!(loaded_body.contains(r#"name="surface" value="akra""#));
    assert!(loaded_body.contains(r#"href="/admin/akra/directions""#));
    assert!(loaded_body.contains(&format!("formaction=\"{validate_uri}\"")));

    let validated = router
        .clone()
        .oneshot(html_form_request(
            &validate_uri,
            encoded_form(&[
                ("csrf_token", csrf_token.as_str()),
                ("kind", "queue_idle_prompt"),
                ("surface", "akra"),
                ("file_queue_idle_prompt", "Queue prompt draft"),
            ]),
            Some(&cookie),
            false,
        ))
        .await
        .expect("AKRA draft validate should be served");
    assert_eq!(validated.status(), StatusCode::OK);
    let validated_body = text_body(validated).await;
    assert!(validated_body.contains(r#"href="/admin/akra/directions""#));
    assert!(validated_body.contains(r#"name="surface" value="akra""#));
}

#[tokio::test]
async fn admin_html_draft_routes_reject_malformed_draft_names() {
    let workspace = TempAdminWorkspace::new("draft-name-html");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token, _) = bootstrap_admin_html_session(&router).await;

    for uri in [
        "/admin/drafts/%2E%2E?kind=full_planning",
        "/admin/drafts/%2E%2E%2Foutside?kind=full_planning",
        "/admin/drafts/bad%0Aname?kind=full_planning",
    ] {
        let response = router
            .clone()
            .oneshot(
                admin_request_builder()
                    .method(Method::GET)
                    .uri(uri)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .expect("editor page request should build"),
            )
            .await
            .expect("editor page request should be served");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }

    for uri in [
        "/admin/drafts/%2E%2E%2Foutside/save",
        "/admin/drafts/%2E%2E%2Foutside/validate",
        "/admin/drafts/%2E%2E%2Foutside/promote",
    ] {
        let response = router
            .clone()
            .oneshot(html_form_request(
                uri,
                encoded_form(&[
                    ("csrf_token", csrf_token.as_str()),
                    ("kind", "full_planning"),
                    ("file_result_output", "# Invalid draft name\n"),
                ]),
                Some(&cookie),
                false,
            ))
            .await
            .expect("draft mutation request should be served");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[test]
fn admin_html_and_json_reset_routes_share_parser_and_facade() {
    /*
     * Reset is exposed as both a browser POST and a JSON POST. They may render
     * different responses, but they must share the same text-to-target parser
     * and facade mutation so queue/directions/all cannot drift by transport.
     */
    for route in [
        ".route(\"/admin/controls/reset\", post(pages::reset_page))",
        ".route(\"/api/planning/reset\", post(api::reset_api))",
    ] {
        assert!(
            source_contains(ADMIN_MOD, route),
            "route table should keep paired reset route {route}"
        );
    }

    assert!(ADMIN_PAGES.contains("let target = parse_reset_target(&form.target)?;"));
    assert!(ADMIN_PAGES.contains(".reset_workspace(target)"));
    assert!(ADMIN_API.contains(".reset_workspace(parse_reset_target(&request.target)?)"));
}

#[test]
fn admin_html_and_json_draft_routes_share_mutation_facade_methods() {
    /*
     * Draft save/validate/promote has HTML and JSON variants. This source-level
     * guard keeps both transports on PlanningAdminDraftMutationRequest and the
     * same facade methods while still allowing different response rendering.
     */
    for route in [
        ".route(\n            \"/admin/drafts/{draft_name}/save\",\n            post(pages::save_draft_page),\n        )",
        ".route(\n            \"/admin/drafts/{draft_name}/validate\",\n            post(pages::validate_draft_page),\n        )",
        ".route(\n            \"/admin/drafts/{draft_name}/promote\",\n            post(pages::promote_draft_page),\n        )",
        ".route(\n            \"/api/planning/drafts/{draft_name}\",\n            get(api::load_draft_api).put(api::save_draft_api),\n        )",
        ".route(\n            \"/api/planning/drafts/{draft_name}/validate\",\n            post(api::validate_draft_api),\n        )",
        ".route(\n            \"/api/planning/drafts/{draft_name}/promote\",\n            post(api::promote_draft_api),\n        )",
    ] {
        assert!(
            source_contains(ADMIN_MOD, route),
            "route table should keep paired draft route {route}"
        );
    }

    for (label, source) in [("HTML", ADMIN_PAGES), ("JSON", ADMIN_API)] {
        assert!(
            source.contains("PlanningAdminDraftMutationRequest"),
            "{label} draft path should use the shared draft mutation request"
        );
        assert!(
            source.contains(".save_draft_session("),
            "{label} draft path should call the shared save inbound-port method"
        );
        assert!(
            source.contains(".promote_draft_session("),
            "{label} draft path should call the shared promote inbound-port method"
        );
    }
    assert!(ADMIN_PAGES.contains("page_mutation_request(draft_name, form)"));
    assert!(ADMIN_API.contains("PlanningAdminDraftMutationRequest {"));
}

#[test]
fn admin_html_and_json_direction_task_routes_share_facade_methods() {
    /*
     * Direction and task CRUD are the easiest places to accidentally add a
     * browser-only or API-only rule. Pair the route table and facade calls so
     * both transports keep the same application mutation owner.
     */
    for route in [
        ".route(\n            \"/admin/directions/upsert\",\n            post(pages::upsert_direction_page),\n        )",
        ".route(\n            \"/admin/directions/delete\",\n            post(pages::delete_direction_page),\n        )",
        ".route(\n            \"/admin/akra/directions/upsert\",\n            post(pages::upsert_akra_direction_page),\n        )",
        ".route(\n            \"/admin/akra/directions/delete\",\n            post(pages::delete_akra_direction_page),\n        )",
        ".route(\"/admin/tasks/upsert\", post(pages::upsert_task_page))",
        ".route(\"/admin/tasks/delete\", post(pages::delete_task_page))",
        ".route(\n            \"/admin/akra/tasks/upsert\",\n            post(pages::upsert_akra_task_page),\n        )",
        ".route(\n            \"/admin/akra/tasks/delete\",\n            post(pages::delete_akra_task_page),\n        )",
        ".route(\"/api/planning/directions\", post(api::upsert_direction_api))",
        ".route(\n            \"/api/planning/directions/delete\",\n            post(api::delete_direction_api),\n        )",
        ".route(\"/api/planning/tasks\", post(api::upsert_task_api))",
        ".route(\"/api/planning/tasks/delete\", post(api::delete_task_api))",
    ] {
        assert!(
            source_contains(ADMIN_MOD, route),
            "route table should keep paired admin CRUD route {route}"
        );
    }

    for method in [
        ".upsert_direction(",
        ".delete_direction(",
        ".upsert_task(",
        ".delete_task(",
    ] {
        assert!(
            ADMIN_PAGES.contains(method),
            "HTML admin path should call shared facade method {method}"
        );
        assert!(
            ADMIN_API.contains(method),
            "JSON admin path should call shared facade method {method}"
        );
    }
}

/*
 * admin 개편의 첫 화면 계약은 route handler가 아니라 template shell에 있다.
 * sidebar landmark와 dashboard quick routes가 사라지면 로컬 운영자가 편집/제어 surface로 바로 이동하지 못하므로
 * fixture test로 최소 구조를 고정한다.
 */
#[test]
fn admin_shell_exposes_sidebar_navigation_and_dashboard_routes() {
    assert!(BASE_TEMPLATE.contains("class=\"admin-layout\""));
    assert!(BASE_TEMPLATE.contains("aria-label=\"Admin navigation\""));
    assert!(BASE_TEMPLATE.contains("class=\"workspace-chip\""));
    assert!(!BASE_TEMPLATE.contains("legacy"));
    assert!(BASE_TEMPLATE.contains("href=\"/admin/akra#pool\""));
    assert!(BASE_TEMPLATE.contains("href=\"/admin/akra#pipeline\""));
    assert!(BASE_TEMPLATE.contains("href=\"/admin/akra/metrics#system\""));
    assert!(BASE_TEMPLATE.contains(
        r#"{% set is_akra = current_nav == "akra_dashboard" || current_nav == "akra_metrics" || current_nav == "akra_directions" || current_nav == "akra_tasks" %}"#
    ));
    assert!(BASE_TEMPLATE.contains(r#"<body class="{% if is_akra %}akra-graphic{% endif %}{% if current_nav == "akra_dashboard" %} akra-dashboard-page{% endif %}">"#));
    assert!(BASE_TEMPLATE.contains(r#"{% if is_akra %}"#));
    assert!(!BASE_TEMPLATE.contains(
        r#"<body class="{% if current_nav == "akra_dashboard" || current_nav == "akra_metrics" || current_nav == "tasks" %}akra-graphic{% endif %}">"#
    ));
    assert!(BASE_TEMPLATE.contains(
        r#"href="/admin/tasks" class="{% if current_nav == "tasks" %}active{% endif %}""#
    ));
    assert!(BASE_TEMPLATE.contains(
        r#"href="/admin/app-server-prompts" class="{% if current_nav == "app_server_prompts" %}active{% endif %}""#
    ));
    assert!(APP_SERVER_PROMPTS_TEMPLATE.contains("App-server prompt I/O"));
    assert!(APP_SERVER_PROMPTS_TEMPLATE.contains("Developer Instructions"));
    assert!(ADMIN_MOD.contains("\"/admin/app-server-prompts\""));
    assert!(ADMIN_MOD.contains("get(pages::app_server_prompts_page)"));
    assert!(BASE_TEMPLATE.contains("/admin/assets/scripts/admin-shell.js"));
    assert!(!BASE_TEMPLATE.contains("<script>"));
    assert!(ADMIN_SHELL_JS.contains("akraHashTabRoutes"));
    assert!(ADMIN_SHELL_JS.contains("window.location.pathname !== \"/admin/akra\""));
    assert!(ADMIN_SHELL_JS.contains("directions: \"/admin/akra/directions\""));
    assert!(ADMIN_SHELL_JS.contains("tasks: \"/admin/akra/tasks\""));
    assert!(
        ADMIN_SHELL_JS.contains("window.addEventListener(\"hashchange\", redirectAkraHashTab)")
    );
    assert!(BASE_TEMPLATE.contains(r#"href="/admin/akra/directions" class="{% if current_nav == "akra_directions" %}active{% endif %}"{% if current_nav == "akra_directions" %} aria-current="page"{% endif %}><span class="nav-icon" aria-hidden="true">G</span><span>작전 방향</span></a>"#));
    assert!(BASE_TEMPLATE.contains(r#"href="/admin/akra/tasks" class="{% if current_nav == "akra_tasks" %}active{% endif %}"{% if current_nav == "akra_tasks" %} aria-current="page"{% endif %}><span class="nav-icon" aria-hidden="true">T</span><span>작업 관리</span></a>"#));
    assert!(BASE_TEMPLATE.contains("AKRA Admin"));
    assert!(BASE_TEMPLATE.contains("실시간 상태와 빌드 정보는 각 화면 본문에서 확인"));
    assert!(!BASE_TEMPLATE.contains("AKRA v0.9.0-beta"));
    assert!(!BASE_TEMPLATE.contains("모든 시스템 정상"));
    assert!(ADMIN_MOD.contains("AKRA_ADMIN_GRAPHIC_ENABLED"));
    assert!(!ADMIN_MOD.contains("AKRA_ADMIN_API_BASE_URL"));
    assert!(ADMIN_MOD.contains("AKRA_ADMIN_GRAPHIC_POLL_MS"));

    for route in [
        "href=\"/admin/tasks\"",
        "href=\"/admin/directions\"",
        "href=\"/admin/controls\"",
    ] {
        assert!(
            DASHBOARD_TEMPLATE.contains(route),
            "dashboard should expose quick route {route}"
        );
    }

    assert!(DASHBOARD_TEMPLATE.contains("Open Full Planning Draft"));
    assert!(ADMIN_MOD.contains(".route(\"/admin\", get(pages::dashboard_page))"));
    assert!(ADMIN_MOD.contains(".route(\"/\", get(pages::dashboard_page))"));
    assert!(BASE_TEMPLATE.contains("current_nav == \"dashboard\""));
}

#[test]
fn tasks_page_uses_default_admin_catalog_without_losing_forms() {
    for token in [
        "class=\"toolbar\"",
        "class=\"create-panel\"",
        "<summary>Add task</summary>",
        "class=\"actions\"",
        "class=\"metric-row\"",
        "class=\"list-panel\"",
        "Task catalog view. Open a row to edit details; queue order is derived from priority.",
        "class=\"entity-list\" id=\"task-list\"",
        "class=\"entity-row\"",
        "Skipped tasks",
        "No skipped tasks are currently visible.",
        "data-list-filter=\"task-list\"",
        "data-filter-empty=\"task-list\"",
        "overview.runtime.proposed_tasks",
        "overview.runtime.skipped_count",
        "overview.runtime.skipped_tasks",
        "{{ task.reason }}",
        "management.tasks.len()",
        "management.directions.len()",
    ] {
        assert!(
            TASKS_TEMPLATE.contains(token),
            "default tasks tab should expose {token}"
        );
    }

    for token in [
        "action=\"{{ task_upsert_path }}\"",
        "action=\"{{ task_delete_path }}\"",
        "action=\"/admin/files/export\"",
        "action=\"/admin/files/apply\"",
        "name=\"csrf_token\"",
        "name=\"id\"",
        "name=\"title\"",
        "name=\"direction_id\"",
        "name=\"status\"",
        "name=\"base_priority\"",
        "name=\"dynamic_priority_delta\"",
        "name=\"priority_reason\"",
        "name=\"description\"",
        "name=\"depends_on_text\"",
        "name=\"blocked_by_text\"",
    ] {
        assert!(
            TASKS_TEMPLATE.contains(token),
            "default tasks tab should keep admin form contract {token}"
        );
    }
    for token in [
        r#"<input type="number" name="base_priority" placeholder="default: 80">"#,
        r#"<input type="number" name="base_priority" value="{{ task.base_priority }}">"#,
        r#"<input type="number" name="dynamic_priority_delta" value="{{ task.dynamic_priority_delta }}">"#,
        r#">default: {{ management.default_direction_id }}</option>"#,
        "{{ direction.title }} / {{ direction.id }}",
    ] {
        assert!(
            TASKS_TEMPLATE.contains(token),
            "default tasks tab should keep ergonomic form token {token}"
        );
    }

    assert_eq!(
        TASKS_TEMPLATE
            .matches("action=\"{{ task_upsert_path }}\"")
            .count(),
        2
    );
    assert_eq!(
        TASKS_TEMPLATE
            .matches("action=\"{{ task_delete_path }}\"")
            .count(),
        1
    );
    assert!(!TASKS_TEMPLATE.contains("akra-task-console"));
    assert!(!TASKS_TEMPLATE.contains("게임발전국 작업 관리"));
    assert!(ADMIN_PAGES.contains("Self::Default => \"Tasks\""));
    assert!(ADMIN_PAGES.contains("Self::Default => \"/admin/tasks/upsert\""));
    assert!(ADMIN_PAGES.contains("Self::Akra => \"/admin/akra/tasks/upsert\""));
}

#[test]
fn directions_page_lists_tasks_for_each_direction_row() {
    for token in [
        "class=\"direction-task-list\" aria-label=\"Tasks for {{ direction.title }}\"",
        "class=\"direction-task-rows\"",
        "{% for task in direction.tasks %}",
        "class=\"direction-task-row status-{{ task.status }}\"",
        "linked task count: {{ direction.task_count }}",
        "No tasks reference this direction.",
        "{{ task.title }}",
        "{{ task.id }}",
        "{{ task.base_priority }}",
        "{{ task.dynamic_priority_delta }}",
        "{{ task.updated_at }}",
    ] {
        assert!(
            DIRECTIONS_TEMPLATE.contains(token),
            "directions page should expose direction-scoped task list token {token}"
        );
    }
}

#[test]
fn akra_graphic_dashboard_keeps_admin_and_snapshot_surfaces() {
    for copy in [
        "게임발전국",
        "AKRA ADMIN CONTROL CENTER",
        "AKRA COMMAND",
        "ATTENTION",
        "LOOP CONTROL",
        "data-summary-active-agents",
        "data-summary-idle-slots",
        "data-summary-queue-depth",
        "data-summary-generated-time",
        "data-command-readiness",
        "data-command-branch",
        "data-operational-notice",
        "data-operational-action",
        "미집계",
        "워크트리 풀",
        "배포 파이프라인",
        "실시간 이벤트",
        "활성 레인",
        "data-admin-graphic",
        "class=\"akra-page-title\"",
        "data-poll-interval-ms",
        "gamebaljeonguk_atlas_64x96.png",
        "background-image: var(--agent-sprite-sheet)",
        "background-size: 384px 504px",
        "avatar-Artificer",
        "agentAvatarClass",
        "akra-operations-studio-v3.png",
        "office-map-image",
        "background: var(--office-bg-image) 0 0 / 100% 100% no-repeat",
        "class=\"scene-object boss-seat\"",
        "class=\"scene-object rest-area\"",
        "role-distributor",
        "role-events",
        "data-focus-target=\"pipeline\"",
        "data-event-list",
        "data-detail-drawer",
        "id=\"akra-detail-drawer\"",
        "role=\"dialog\"",
        "detailType: \"campaignLane\"",
        "data-projection-kind",
        "data-agent-id=\"{{ item.source_agent }}\"",
        "data-refresh-dashboard",
        "openDetailDrawer",
        "navigateDetailSelection",
        "selectionTokens",
        "projectionSlotToken",
        "aria-controls",
        "aria-expanded",
        "relatedSelectionCount",
        "openRefreshDetail",
        "setManualRefreshState",
        "aria-busy",
        "data-event-feed-status",
        "data-realtime-status",
        "command-refresh",
        "detailSourceKey(node) === nextKey",
        "data-actor-id",
        "data-standby-character",
        "data-presence-kind",
        "대기 프로필",
        "data-visual-state",
        "data-scene-diagnostics",
        "prependEventRows",
        "stale snapshot",
        "pollState",
        "pollEvents",
        "new EventSource",
        "cursorResetRequired",
        "polling fallback",
        "latestCommand",
        "/admin/assets/game/akra-diorama.js",
        "/admin/assets/scripts/admin-shell.js",
        "/admin/assets/scripts/akra-dashboard.js",
        "data-planning-revision",
        "akra:dashboard-rendered",
        "renderDashboardPanels",
        "dashboardSignature",
        "renderCampaign",
        "renderBoard",
        "renderPipeline",
        "agents: dashboard.agents || null",
        "scene: dashboard.scene || null",
        "pool: dashboard.pool || null",
        "distributor: dashboard.distributor || null",
        "campaign: dashboard.campaign || null",
        "selectedTask: dashboard.selectedTask || null",
        "kpis: dashboard.kpis || null",
        "workspace: dashboard.workspace || null",
        "eventFeed: dashboard.eventFeed || null",
        "events: asArray(dashboard.events)",
        "score-chip",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(copy)
                || AKRA_DASHBOARD_JS.contains(copy)
                || BASE_TEMPLATE.contains(copy)
                || ADMIN_SHELL_JS.contains(copy),
            "graphic dashboard should expose {copy}"
        );
    }

    for anchor in [
        "id=\"pool\"",
        "id=\"agents\"",
        "id=\"pipeline\"",
        "id=\"events\"",
        "id=\"campaign\"",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(anchor),
            "graphic dashboard should expose sidebar target {anchor}"
        );
    }

    for route in [
        ".route(\"/admin/akra\", get(pages::akra_dashboard_page))",
        ".route(\"/admin/akra/metrics\", get(pages::akra_metrics_page))",
        ".route(\"/admin/akra/directions\", get(pages::akra_directions_page))",
        ".route(\"/admin/akra/tasks\", get(pages::akra_tasks_page))",
        "\"/api/admin/akra/dashboard\"",
        "\"/api/admin/akra/pool\"",
        "\"/api/admin/akra/agents\"",
        "\"/api/admin/akra/distributor\"",
        "\"/api/admin/akra/events\"",
        "\"/api/admin/akra/stream\"",
        "\"/api/admin/akra/commands/{command_id}\"",
        "\"/admin/assets/graphics/{asset_name}\"",
        "\"/admin/assets/game/{asset_name}\"",
        "\"/admin/assets/scripts/{asset_name}\"",
        "\"/admin/assets/fonts/{asset_name}\"",
    ] {
        assert!(
            source_contains(ADMIN_MOD, route),
            "admin route table should keep {route}"
        );
    }

    for token in [
        "mountDiorama",
        "rebuildAgentUnits",
        "new Application()",
        "gamebaljeonguk_atlas_128x192.png",
        "akra-operations-studio-v3.png",
        "STATIC_POSE_MANIFEST",
        "Promise.allSettled",
        "inspectScene",
        "buildAgentFrameSets",
        "makeAtlasFrameByIndex",
        "STANDBY_LOUNGE_POINTS",
        "SceneCameraController",
        "DashboardSceneStore",
    ] {
        assert!(
            admin_game_source_contains(token),
            "admin game source modules should expose {token}"
        );
    }
}

#[test]
fn akra_admin_never_reports_uncollected_health_as_success() {
    for fabricated_copy in [
        "<strong>68%</strong>",
        "<strong>23 / 30</strong>",
        "<strong>128</strong>",
        "<strong>14:32:21</strong>",
        "<strong>62%</strong>",
        "<strong>231 / 30</strong>",
        "<strong>42%</strong>",
        "<strong>58%</strong>",
        "<strong>120 MB/s</strong>",
        "<small>정상</small>",
        "<span class=\"system-ready-dot\"></span>정상",
        "봄맞이 프로젝트",
        "content: \"66%\"",
        "content: \"42%\"",
        "setKpiState(\"success\")",
        "stage 75/100",
    ] {
        assert!(
            !AKRA_DASHBOARD_TEMPLATE.contains(fabricated_copy)
                && !AKRA_DASHBOARD_JS.contains(fabricated_copy),
            "dashboard must not preserve fabricated operator signal {fabricated_copy}"
        );
    }

    for snapshot_copy in [
        "{{ dashboard.kpis.active_agents }} / {{ dashboard.kpis.total_agents }}",
        "{{ dashboard.kpis.pool_idle }}",
        "{{ dashboard.kpis.queue_depth }}",
        "{{ dashboard.generated_time_label }}",
        "{{ dashboard.workspace.readiness }}",
        "{{ dashboard.workspace.readiness_notice }}",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(snapshot_copy),
            "dashboard should render collected snapshot signal {snapshot_copy}"
        );
    }
    assert!(!AKRA_DASHBOARD_TEMPLATE.contains(">미집계<"));
    assert!(!AKRA_DASHBOARD_TEMPLATE.contains("id=\"system-mini\""));
    assert!(!AKRA_DASHBOARD_TEMPLATE.contains("시스템 상태 요약"));
    assert!(AKRA_DASHBOARD_JS.contains("const pollState"));
    assert!(AKRA_DASHBOARD_TEMPLATE.contains("data-detail-progress"));
    assert!(AKRA_DASHBOARD_RS.contains("\"stage 미집계\""));
    assert!(AKRA_METRICS_TEMPLATE.contains("Git 상태 미집계"));
    assert!(AKRA_METRICS_TEMPLATE.contains("GitHub 연동 미집계"));
    assert!(!AKRA_METRICS_TEMPLATE.contains("Git 상태 정상"));
    assert!(!AKRA_METRICS_TEMPLATE.contains("GitHub 연동 정상"));
}

#[test]
fn akra_graphic_dashboard_event_rows_reset_native_button_chrome() {
    assert!(
        source_contains(
            AKRA_DASHBOARD_TEMPLATE,
            ".event-row {\n    appearance: none;\n    width: 100%;\n    color: inherit;\n    font: inherit;\n    text-align: left;\n    background: transparent;\n    border-top: 0;\n    border-right: 0;\n    border-left: 0;\n    border-radius: 0;",
        ),
        "runtime event rows are buttons, so they must reset native button background and borders"
    );
}

#[test]
fn akra_graphic_dashboard_event_status_uses_readable_counts() {
    assert!(
        AKRA_DASHBOARD_TEMPLATE.contains("{{ dashboard.event_feed.status_label }}"),
        "event feed status should use the server-formatted readable label on initial render"
    );
    assert!(
        AKRA_DASHBOARD_JS.contains("formatEventFeedStatus"),
        "event feed polling should preserve the readable count label"
    );
    assert!(
        !AKRA_DASHBOARD_TEMPLATE.contains(
            "LIVE · {{ dashboard.event_feed.visible_event_count }}/{{ dashboard.event_feed.total_event_count }}"
        ),
        "event feed status should not render the capped feed as an ambiguous fraction"
    );
}

#[test]
fn akra_graphic_dashboard_keeps_one_primary_surface_per_operational_fact() {
    for primary_surface in [
        "class=\"command-summary",
        "data-realtime-status",
        "class=\"attention-strip",
        "class=\"command-controls\"",
        "<h3>활성 레인</h3>",
        "class=\"game-panel\" id=\"pipeline\"",
        "data-detail-drawer",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(primary_surface),
            "operations cockpit should keep primary surface {primary_surface}"
        );
    }

    for removed_duplicate in [
        "MISSION FLOW",
        "OPERATOR BRIEF",
        "class=\"stage-hud\"",
        "stage-refresh-btn",
        "class=\"campaign-summary\"",
        "id=\"tasks\"",
        "renderSelectedTask",
    ] {
        assert!(
            !AKRA_DASHBOARD_TEMPLATE.contains(removed_duplicate)
                && !AKRA_DASHBOARD_JS.contains(removed_duplicate),
            "operations cockpit should remove duplicate surface {removed_duplicate}"
        );
    }
}

#[test]
fn akra_graphic_dashboard_validation_rail_keeps_accessible_typed_operations_contract() {
    for token in [
        "data-validation-kpi=\"verifying\"",
        "data-validation-kpi=\"failed\"",
        "data-validation-kpi=\"queued\"",
        "data-validation-kpi=\"stale\"",
        "id=\"validation-rail\"",
        "data-validation-rollout",
        "data-rollout-stage=\"{{ dashboard.validation.rollout.stage }}\"",
        "QUEUE ADMISSION BLOCKED",
        "RULESET · APPROVAL REQUIRED",
        "data-validation-list",
        "data-validation-record-key",
        "PR / Akra ID",
        "Integrated",
        "Actions",
        "Findings",
        "Remediation",
        "Verified",
        "latest attempt {{ record.latest_required_attempt }}",
        "{{ record.correlation_count }} worker link",
        "@media (max-width: 860px)",
        "content: attr(data-label)",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(token),
            "validation rail template should keep {token}"
        );
    }
    for token in [
        "renderValidationRail",
        "data-validation-rollout-status",
        "rollout.queueAdmissionEnabled",
        "rollout.rulesetChangeRequiresApproval",
        "openValidationDetailDrawer",
        "renderValidationDetail",
        "runValidationCommand",
        "X-CSRF-Token",
        "expectedRevision",
        "commandId",
        "validationCommandFeedback",
        "validationCommandOutcome",
        "priorDisabledStates",
        "command.disabled = priorDisabledStates.get(command) ?? true;",
        "validation: dashboard.validation || null",
        "latestRequiredAttempt",
        "data-validation-command-status",
        "validation: \"#validation-rail [data-validation-record-key]\"",
    ] {
        assert!(
            AKRA_DASHBOARD_JS.contains(token),
            "validation rail client should keep {token}"
        );
    }
    for token in [
        "GameValidationProjection",
        "QA_CI_STATION_POINT",
        "QA_CI_SIGNAL_POINTS",
        "buildValidationStation",
        "syncValidationStation",
        "sceneValidationPacketVisible",
        "workerLeaseActive",
        "packetVisible",
    ] {
        assert!(
            admin_game_source_contains(token),
            "validation game projection should keep {token}"
        );
    }
    assert!(source_contains(
        ADMIN_MOD,
        "\"/api/admin/akra/validations/{record_key}/commands\""
    ));
    assert!(source_contains(
        ADMIN_API,
        "verify_header_csrf(&jar, &headers)?"
    ));
}

#[test]
fn akra_graphic_dashboard_validation_evidence_is_explainable_responsive_and_accessible() {
    for token in [
        "data-validation-evidence",
        "data-validation-evidence-state",
        "data-evidence-status",
        "data-evidence-status-mark",
        "data-evidence-attention",
        "data-evidence-attention-action",
        "data-evidence-detail-trigger",
        "Projected Fast Gate",
        "Actual Fast Gate",
        "CI Gate",
        "Post-Merge Gate",
        "data-metric-p50",
        "data-metric-p95",
        "data-metric-samples",
        "data-metric-source",
        "미수집",
        ".validation-evidence-metrics { grid-template-columns: 1fr; }",
        ".validation-workflow-times { grid-template-columns: 1fr; }",
        "aria-controls=\"akra-detail-drawer\"",
        "aria-expanded=\"false\"",
        "role=\"status\"",
        "aria-live=\"polite\"",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(token),
            "validation evidence template should keep {token}"
        );
    }
    for token in [
        "renderValidationEvidence",
        "shell.dataset.validationEvidenceState = status",
        "const evidenceNumber = (value)",
        "value === null || value === undefined || value === \"\"",
        "renderEvidenceAttention",
        "formatEvidenceSeconds",
        "formatEvidenceTimestamp",
        "currentValidationEvidence",
        "workflowSelectionCopy",
        "workflowNotSelectedCopy",
        "workflow.selected ? \" is-selected\"",
        "newest_run",
        "latest_attempt",
        "attempt 번호는 서로 다른 run 사이에서 비교하지 않습니다.",
        "openEvidenceDetailDrawer",
        "renderEvidenceHistoryPage",
        "canonicalGithubUrl",
        "/api/admin/akra/pr-validation/evidence",
        "대시보드의 마지막 summary는 유지됩니다.",
        "evidenceDetailTrigger?.setAttribute(\"aria-expanded\", \"false\")",
    ] {
        assert!(
            AKRA_DASHBOARD_JS.contains(token),
            "validation evidence client should keep {token}"
        );
    }
    for unsafe_copy in ["0s", "href = record.canonicalPrUrl"] {
        assert!(
            !AKRA_DASHBOARD_TEMPLATE.contains(unsafe_copy)
                && !AKRA_DASHBOARD_JS.contains(unsafe_copy),
            "validation evidence must not expose unsafe fallback {unsafe_copy}"
        );
    }
    assert!(
        !AKRA_DASHBOARD_JS.contains("shell.dataset.evidenceStatus"),
        "evidence shell state must not collide with the nested status badge selector"
    );
}

#[test]
fn akra_graphic_dashboard_game_bundle_is_vite_typescript_input() {
    for token in [
        "\"build\": \"vite build --config vite.config.ts && node scripts/promote-build.mjs\"",
        "\"check\": \"tsc --noEmit --project tsconfig.json\"",
        "\"pixi.js\": \"^8.19.0\"",
        "\"typescript\":",
        "\"vite\":",
    ] {
        assert!(
            ADMIN_GAME_PACKAGE_JSON.contains(token),
            "admin game package should keep {token}"
        );
    }

    for token in [
        "entry: \"src/akra-diorama.ts\"",
        "formats: [\"iife\"]",
        "fileName: () => \"akra-diorama.js\"",
        "name: \"AkraAdminDioramaBundle\"",
        "outDir: \"dist\"",
    ] {
        assert!(
            ADMIN_GAME_VITE_CONFIG.contains(token),
            "admin game Vite config should keep {token}"
        );
    }

    for token in [
        "import \"pixi.js/unsafe-eval\";",
        "import { Application, Assets, Texture, type Ticker } from \"pixi.js\";",
        "export type StatusSeverity",
        "export interface DioramaHandle",
        "const mountDiorama = (): DioramaHandle | null",
        "window.AkraAdminGame",
        "Assets.load<Texture>",
        "preference: \"webgl\"",
        "Math.min(window.devicePixelRatio || 1, 2)",
        "Promise.allSettled",
        "requestSceneRender",
        "inspectScene",
        "export const ACTIVE_FRAME_INTERVAL_MS = 1000 / 60",
        "export const RAF_HEALTHY_GAP_MS = 80",
        "frameDriver = \"interval-fallback\"",
        "window.setInterval",
        "document.hidden",
        "visibilitychange",
        "app.ticker.stop()",
        "export type Facing = \"down\" | \"side\" | \"up\"",
        "export type AgentAnimationKind",
        "export interface AgentFrameSet",
        "export const WALK_FRAME_ALIGNMENT",
        "export const alignmentForFacing",
        "export const WALK_FRAME_VISUAL_SCALE",
        "export const scaleForFacing",
        "export type VisualState",
        "export type PresenceKind",
        "const STATIC_POSE_MANIFEST",
        "STATIC_POSE_MANIFEST[archetype][pose]",
        "export const ARCHETYPE_BY_PROFILE",
        "const drawStaticMarker",
        "export const AGENT_MOVEMENT_SPEED_RATIO = 0.3",
        "export const AGENT_TRAVEL_SPEED_WORLD_PX_PER_SECOND = 168",
        "export const STANDBY_LOUNGE_TRAVEL_SPEED_WORLD_PX_PER_SECOND = 52",
        "STANDBY_LOUNGE_PATROL_ROUTES",
        "advanceStandbyPatrol",
        "ambientActivityCount",
        "export const WALK_IN_PLACE_CYCLE_MS = 760",
        "export const IDLE_IN_PLACE_CYCLE_MS = 960",
        "export const IDLE_IN_PLACE_AMPLITUDE_RATIO = 0.65",
        "export const WALK_SWAY_WORLD_PX = 0.7",
        "export const WALK_LIFT_WORLD_PX = 1.6",
        "const visibleStepFrameIndex",
        "const applyVisibleStepAppearance",
        "Pixel-art poses vary in silhouette width",
        "unit.sprite.alpha = 1",
        "unit.blendSprite.alpha = 0",
        "blendSprite",
        "animationKind: unit.animationKind",
        "animationBlend: Number(unit.animationBlend.toFixed(3))",
        "gaitOffsetX: Number(unit.gaitOffsetX.toFixed(2))",
        "gaitOffsetY: Number(unit.gaitOffsetY.toFixed(2))",
        "movementSpeedRatio: AGENT_MOVEMENT_SPEED_RATIO",
        "export const AGENT_FRAME_WIDTH = 128",
        "export const AGENT_FRAME_HEIGHT = 192",
        "export const AGENT_SPRITE_SCALE = 0.72",
        "const spriteBounds = unit.sprite.getBounds()",
        "displayWidth: Math.round(spriteBounds.width)",
        "displayHeight: Math.round(spriteBounds.height)",
        "boardX: Math.round(boardPoint.x)",
        "boardY: Math.round(boardPoint.y)",
        "gamebaljeonguk_atlas_128x192.png",
        "akra-operations-studio-v3.png",
        "class DashboardSceneStore",
        "class SceneCameraController",
        "OCCLUSION_POLYGONS",
        "akra:scene-selection-requested",
    ] {
        assert!(
            admin_game_source_contains(token),
            "admin game TypeScript modules should keep {token}"
        );
    }

    let pixi_import = AKRA_DIORAMA_TS
        .find("import { Application, Assets, Texture, type Ticker } from \"pixi.js\";")
        .expect("admin game should import Pixi");
    let strict_csp_adapter_import = AKRA_DIORAMA_TS
        .find("import \"pixi.js/unsafe-eval\";")
        .expect("admin game should install the Pixi 8 strict-CSP adapter");
    let renderer_initialization = AKRA_DIORAMA_TS
        .find("const app = new Application()")
        .expect("admin game should initialize the Pixi renderer");
    assert!(strict_csp_adapter_import < pixi_import);
    assert!(pixi_import < renderer_initialization);
    assert!(!ADMIN_GAME_PACKAGE_JSON.contains("@pixi/unsafe-eval"));
    assert!(!admin_game_source_contains("import \"@pixi/unsafe-eval\";"));

    for token in [
        "dist/akra-diorama.js",
        "akra-diorama.js",
        "readFileSync",
        "writeFileSync",
    ] {
        assert!(
            ADMIN_GAME_PROMOTE_BUILD.contains(token),
            "admin game promote script should keep {token}"
        );
    }
}

#[test]
fn akra_graphic_dashboard_visual_contract_has_regression_guardrails() {
    for token in [
        "class=\"office-board\" id=\"agents\"",
        "class=\"game-panel pool-overlay\" id=\"pool\"",
        "class=\"scene-object boss-seat\"",
        "class=\"scene-object rest-area\"",
        "background-image: var(--agent-sprite-sheet)",
        "background-size: 384px 504px",
        "background-position: -288px 0",
        "akra-operations-studio-v3.png",
        "office-map-image",
        "max-width: 1784px",
        "max-width: 1280px",
        "align-content: start",
        "grid-template-columns: 200px minmax(520px, 1280px) 270px",
        "background: var(--office-bg-image) 0 0 / 100% 100% no-repeat",
        "grid-template-columns: minmax(0, 1fr)",
        "overflow: auto",
        "text-overflow: ellipsis",
        "@media (max-width: 860px)",
        "generated_time_label",
        "planning_revision",
        "readiness_notice",
        "blocked_action",
        "queue_depth_basis",
        "mock_metric_note",
        "CampaignView",
        "map_campaign",
        "stage 미집계",
        "--office-bg-image",
        "--agent-sprite-sheet",
        "var(--office-bg-image)",
        "data-detail-type=\"slot\"",
        "data-detail-type=\"distributor\"",
        "data-detail-type=\"queueItem\"",
        "{% for actor in dashboard.scene.actors %}",
        "class=\"scene-object desk agent-{{ actor.seat_index }} severity-{{ actor.severity }}\"",
        "data-actor-id=\"{{ actor.actor_id }}\"",
        "{% for character in dashboard.scene.standby_characters %}",
        "data-standby-character=\"true\"",
        "data-presence-kind=\"{{ character.presence_kind }}\"",
        "data-scene-standby-index=\"{{ character.location_index }}\"",
        "대기 프로필 {{ dashboard.scene.standby_characters.len() }}/{{ dashboard.scene.standby_profile_count }}",
        "data-agent-id=\"{{ actor.agent_id }}\"",
        "data-visual-state=\"{{ actor.visual_state }}\"",
        "data-static-pose=\"{{ actor.static_pose }}\"",
        "avatar-{{ actor.archetype_key }}",
        "const createActorButton",
        "renderActors(dashboard.scene)",
        "actorDetailDataset",
        "data-scene-actor-list",
        "data-scene-diagnostics",
        "GameSceneView",
        "GameStandbyCharacterView",
        "map_game_scene",
        "분배관 호출",
        "optionalText(distributor.bubbleLabel, \"배포 파이프라인\")",
        "worker_lifecycle_bubble",
        "distributor_bubble",
        "data-detail-title=\"워크트리 풀 · {{ slot.display_slot_label }}\"",
        "data-detail-subtitle=\"{{ slot.label }}\"",
        "data-detail-slot=\"{{ slot.display_slot_label }}\"",
        "data-detail-task=\"{{ slot.task_id.as_deref().unwrap_or(\"-\") }}\"",
        "data-detail-branch=\"{{ slot.branch_name }}\"",
        "data-detail-worktree=\"{{ slot.worktree_label }}\"",
        "data-detail-owner=\"{{ slot.owner_label }}\"",
        "data-owner-agent-id=\"{{ slot.owner_agent_id.as_deref().unwrap_or(\"\") }}\"",
        "data-owner-session-key=\"{{ slot.owner_session_key.as_deref().unwrap_or(\"\") }}\"",
        "data-lease-generation=\"{{ slot.lease_generation.as_deref().unwrap_or(\"\") }}\"",
        "detailOwnerAgent: optionalText(slot.ownerAgentId, \"-\")",
        "detailOwnerSession: optionalText(slot.ownerSessionKey, \"-\")",
        "detailLeaseGeneration: optionalText(slot.leaseGeneration, \"-\")",
        "dashboard.planningRevision == null ? \"미집계\"",
        "title=\"{{ slot.display_slot_label }} · {{ slot.label }} · task",
        "const slotDisplayLabel = optionalText(slot.displaySlotLabel || slot.slotId, \"슬롯\")",
        "const slotTaskId = optionalText(slot.taskId, \"-\")",
        "detailTitle: `워크트리 풀 · ${slotDisplayLabel}`",
        "detailSlot: slotDisplayLabel",
        "detailTask: slotTaskId",
        "createText(\"strong\", \"\", slotDisplayLabel)",
        "createText(\"small\", \"slot-state\", slotStateLabel)",
        "class=\"admin-detail-drawer\"",
        "data-loop-command=\"enable\"",
        "data-loop-command=\"dispatch\"",
        "data-loop-command=\"disable\"",
        "MAP_WIDTH = 1672",
        "MAP_HEIGHT = 941",
        "OCCLUSION_POLYGONS",
        "SceneCameraController",
        "DashboardSceneStore",
        "data-scene-zoom-readout",
        "akra:scene-selection-requested",
        "STATIC_POSE_MANIFEST",
        "STANDBY_LOUNGE_POINTS",
        "makeAtlasFrameByIndex",
        "configured_standby",
        "sceneStandbyCount",
        "standby_pose_for_avatar_class",
        "requestSceneRender",
        "Promise.allSettled",
        "inspectScene",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(token)
                || BASE_TEMPLATE.contains(token)
                || AKRA_DASHBOARD_RS.contains(token)
                || AKRA_DIORAMA_JS.contains(token)
                || admin_game_source_contains(token)
                || ADMIN_SHELL_JS.contains(token)
                || AKRA_DASHBOARD_JS.contains(token),
            "graphic visual contract should keep {token}"
        );
    }

    for semantic_motion in [
        "actorTargetPoint",
        "travelStep",
        "rebuildSignalPackets",
        "world.update",
        "prefers-reduced-motion",
        "sceneSemanticMotionCount",
        "scenePacketCount",
        "gaitOffsetX",
        "gaitOffsetY",
        "animationBlend",
    ] {
        assert!(
            admin_game_source_contains(semantic_motion),
            "truthful dynamic scene should keep semantic motion token {semantic_motion}"
        );
    }

    for removed_scale_pulse in [
        "Math.sqrt(1 - step.blend)",
        "Math.sqrt(step.blend)",
        "unit.group.scale.set(1.055)",
    ] {
        assert!(
            !admin_game_source_contains(removed_scale_pulse),
            "agent movement must not reintroduce a perceived scale pulse: {removed_scale_pulse}"
        );
    }

    for removed in [
        "class=\"akra-topbar\"",
        "class=\"ops-status\"",
        "id=\"metrics\"",
        "akra_admin",
        "Last Updated",
        "길드 성과",
        "운영 지표",
        "운영 관제 · 하네스 제어",
        "게임화 정책",
        "도메인 매핑",
        "blocked-copy",
        "renderOpsStatus",
        "syncTopNotice",
        "renderMetrics",
        "renderSystem",
        "error-notice",
        "전체 진행률",
        "class=\"draft-nav\"",
        "id=\"system-mini\"",
        "시스템 상태 요약",
        "class=\"game-panel notice-card\"",
        "data-event-drawer",
        "aria-pressed",
        "akraStageScan",
        "akraServerBlink",
        "akraEventPulse",
        "akraStepSweep",
    ] {
        assert!(
            !AKRA_DASHBOARD_TEMPLATE.contains(removed),
            "graphic dashboard should not restore removed top header token {removed}"
        );
    }

    for removed in [
        "data-detail-title=\"풀 슬롯 · {{ slot.slot_id }}\"",
        "data-detail-subtitle=\"{{ slot.label }} / {{ slot.note }}\"",
        "title=\"{{ slot.branch_name }} / {{ slot.worktree_label }} / {{ slot.note }}\"",
        "<strong>{{ slot.slot_id }}</strong>",
        "<small>{{ slot.owner_agent_id.as_deref().unwrap_or(\"-\") }}</small>",
        "detailTitle: `풀 슬롯 · ${optionalText(slot.slotId)}`",
        "detailSubtitle: `${optionalText(slot.label)} / ${optionalText(slot.note)}`",
        "button.title = `${optionalText(slot.branchName)} / ${optionalText(slot.worktreeLabel)} / ${optionalText(slot.note)}`",
        "createText(\"strong\", \"\", slot.slotId)",
        "createText(\"small\", \"\", slot.ownerAgentId || \"-\")",
        "const slotStatusLabel = optionalText(slot.bubbleLabel || slot.label, \"풀\")",
    ] {
        assert!(
            !AKRA_DASHBOARD_TEMPLATE.contains(removed),
            "pool slot hover should not expose raw operator token {removed}"
        );
    }

    for token in [
        "aria-label=\"AKRA detached metrics\"",
        "id=\"metrics\"",
        "id=\"system\"",
        "길드 성과",
        "운영 지표",
        "풀 활용률",
        "지표 출처",
        "dashboard.metrics.badges",
        "dashboard.metrics.pool_utilization_percent",
    ] {
        assert!(
            AKRA_METRICS_TEMPLATE.contains(token),
            "detached metrics page should expose {token}"
        );
    }

    for token in [
        "templates/admin/resources/main-sprite.png",
        "gamebaljeonguk_atlas_64x96.png",
        "ADMIN_GRAPHIC_CAPTURE",
        "ADMIN_GAME_BUILD",
        "npm --prefix assets/admin/game run check",
        "npm --prefix assets/admin/game run build",
        "akra-admin",
        "/admin/akra",
        "/admin/akra/metrics",
        "/admin/akra/tasks",
        "/admin/akra/directions",
        "/admin/tasks",
        "admin-tasks.html",
        "/admin/assets/graphics/akra-operations-studio-v3.png",
        "/admin/assets/graphics/gamebaljeonguk_atlas_64x96.png",
        "/admin/assets/graphics/gamebaljeonguk_atlas_128x192.png",
        "/admin/assets/game/akra-diorama.js",
        "/api/admin/akra/dashboard",
        "/api/admin/akra/events?limit=50",
        "/api/admin/akra/events?afterSequence=0&limit=50",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "${HOME}/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "id=\"campaign\"",
        "OPERATOR BRIEF",
        "id=\"system\"",
        "Task catalog view",
        "Skipped tasks",
        "class=\"entity-list\" id=\"task-list\"",
        "data-list-filter=\"task-list\"",
        "\"campaign\"",
        "\"laneCards\"",
        "\"intelCards\"",
        "served operations studio asset does not match workspace asset",
        "served gamebaljeonguk agent atlas does not match workspace asset",
        "served large gamebaljeonguk agent atlas does not match workspace asset",
        "served admin shell script does not match workspace asset",
        "served admin dashboard script does not match workspace asset",
        "served regular admin font does not match workspace asset",
        "served bold admin font does not match workspace asset",
        "AKRA_ADMIN_TOKEN",
        "command curl -q \"$@\"",
        "--curl-config-probe",
        "randomBytes(32).toString(\"hex\")",
        "ADMIN_GRAPHIC_TOKEN must be exactly 64 hexadecimal characters",
        "/admin/login",
        "authenticated_curl",
        "cookie_jar",
        "--screenshot=",
        "--mobile-screenshot=",
        "--compact-screenshot=",
        "--full-hd-screenshot=",
        "--qhd-screenshot=",
        "admin-graphic-mobile.png",
        "admin-graphic-compact.png",
        "admin-graphic-full-hd.png",
        "admin-graphic-qhd.png",
        "mainBottomGap",
        "admin graphic visual contract ok",
    ] {
        assert!(
            ADMIN_GRAPHIC_VISUAL_SCRIPT.contains(token)
                || ADMIN_GRAPHIC_CAPTURE_SCRIPT.contains(token),
            "visual regression script should keep {token}"
        );
    }
    assert!(
        !ADMIN_GRAPHIC_VISUAL_SCRIPT
            .to_ascii_lowercase()
            .contains("firefox")
    );

    for token in [
        "../../../../assets/admin/graphics/akra-operations-studio-v3.png",
        "../../../../assets/admin/graphics/gamebaljeonguk_atlas_64x96.png",
        "../../../../assets/admin/graphics/gamebaljeonguk_atlas_128x192.png",
        "../../../../assets/admin/game/akra-diorama.js",
        "../../../../assets/admin/scripts/admin-shell.js",
        "../../../../assets/admin/scripts/akra-dashboard.js",
        "../../../../assets/admin/fonts/Galmuri11.woff2",
        "../../../../assets/admin/fonts/Galmuri11-Bold.woff2",
        "image/png",
        "text/javascript; charset=utf-8",
        "font/woff2",
        "private, no-cache",
        "Sha256::digest",
        "header::IF_NONE_MATCH",
        "StatusCode::NOT_MODIFIED",
    ] {
        assert!(
            ADMIN_STATIC_ASSETS.contains(token),
            "admin graphic asset route should keep {token}"
        );
    }
}

#[test]
fn akra_dashboard_reads_planning_queue_through_admin_inbound_ports() {
    assert!(
        AKRA_DASHBOARD_RS.contains("load_dashboard_snapshot"),
        "dashboard should ask the parallel admin port for one application-owned snapshot"
    );
    assert!(
        AKRA_DASHBOARD_RS.contains("ParallelModeAdminPort"),
        "dashboard should depend on the parallel admin inbound contract"
    );
    assert!(
        !AKRA_DASHBOARD_RS.contains("PlanningApplicationProjection::from_runtime_projection"),
        "dashboard adapter should not rebuild planning projection from runtime internals"
    );
    assert!(
        !AKRA_DASHBOARD_RS.contains("PlanningServices"),
        "dashboard adapter should not depend on the broad planning service bundle"
    );
    assert!(
        !AKRA_DASHBOARD_RS.contains(".queue_projection()"),
        "dashboard adapter should not read queue projection internals directly"
    );
}

#[test]
fn akra_parallel_admin_surface_reuses_typed_control_plane_for_browser_commands() {
    /*
     * Admin Akra reads passive snapshots and sends browser commands through the
     * same typed application control plane used by the native surface.
     */
    assert!(
        AKRA_DASHBOARD_RS.contains("load_dashboard_snapshot"),
        "admin dashboard should render through the parallel admin inbound port"
    );
    assert!(
        AKRA_DASHBOARD_RS.contains("load_runtime_events"),
        "admin event feed should render through the parallel admin inbound port"
    );
    for command in [
        "ParallelModeAdminCommand::Enable",
        "ParallelModeAdminCommand::Dispatch",
        "ParallelModeAdminCommand::Refresh",
        "ParallelModeAdminCommand::Disable",
    ] {
        assert!(
            ADMIN_API.contains(command),
            "admin API should map transport actions to typed inbound command {command}"
        );
    }
    assert!(ADMIN_MOD.contains("\"/api/admin/akra/control\""));
    assert!(!AKRA_DASHBOARD_RS.contains("ParallelModeService"));
    assert!(!ADMIN_API.contains("ParallelModeControlPlaneCommand"));
    assert!(!ADMIN_API.contains("process_distributor_queue"));
}

#[test]
fn akra_admin_debug_harness_is_explicit_safe_and_browser_controllable() {
    for token in [
        "data-debug-harness",
        "FAKE · SAFE",
        "실제 authority와 분리된 UI 시연",
        "data-debug-command=\"play\"",
        "data-debug-command=\"pause\"",
        "data-debug-command=\"step\"",
        "data-debug-command=\"reset\"",
        "data-debug-scenario",
        "data-debug-stage-count",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(token),
            "debug harness template should expose {token}"
        );
    }
    for token in [
        "\"/api/admin/akra/debug-harness\"",
        "runDebugHarnessCommand",
        "renderDebugHarness",
        "Application Fake 활성",
    ] {
        assert!(
            ADMIN_MOD.contains(token) || AKRA_DASHBOARD_JS.contains(token),
            "debug harness browser contract should expose {token}"
        );
    }
    assert!(ADMIN_API.contains("AdminDebugHarnessCommand::SelectScenario"));
    assert!(ADMIN_API.contains("verify_header_csrf"));
    assert!(ADMIN_API.contains("StatusCode::CONFLICT"));
}

#[test]
fn akra_graphic_dashboard_gamebaljeonguk_sprite_pack_is_reviewable() {
    for token in [
        "gamebaljeonguk_original_transparent.png",
        "gamebaljeonguk_atlas_128x192.png",
        "gamebaljeonguk_atlas_64x96.png",
        "$gamebaljeonguk_planner.png",
        "$gamebaljeonguk_coffee_addict.png",
        "Cell size: 64x96",
    ] {
        assert!(
            GAMEBALJEONGUK_SPRITE_PACK_README.contains(token),
            "gamebaljeonguk sprite pack readme should keep {token}"
        );
    }

    for token in [
        "\"file\": \"gamebaljeonguk_atlas_64x96.png\"",
        "\"cell_width\": 64",
        "\"cell_height\": 96",
        "\"$gamebaljeonguk_planner.png\"",
        "\"$gamebaljeonguk_coffee_addict.png\"",
        "\"planner_down_01\"",
        "\"coffee_addict_down_01\"",
    ] {
        assert!(
            GAMEBALJEONGUK_SPRITE_METADATA.contains(token),
            "gamebaljeonguk sprite metadata should keep {token}"
        );
    }
}

/*
 * browser confirmation은 destructive admin POST가 page를 떠나기 전 마지막 inbound guard다.
 * 서버의 CSRF 검증은 caller intent를 확인하지만, operator가 클릭 실수를 했는지는 template만 막을 수 있다.
 * 그래서 이 테스트는 global submit hook과 per-button data-confirm marker를 함께 확인한다.
 */
#[test]
fn risky_admin_mutations_require_browser_confirmation() {
    // capture-phase registration은 nested form/button 구조가 confirmation hook을 우회하지 못하게 한다.
    assert!(BASE_TEMPLATE.contains("/admin/assets/scripts/admin-shell.js"));
    assert!(ADMIN_SHELL_JS.contains("document.addEventListener(\"submit\""));
    assert!(ADMIN_SHELL_JS.contains("}, true);"));

    // 첫 pass는 특정 template이 risky-action marker를 모두 잃었을 때 page 이름이 보이는 실패 메시지를 제공한다.
    for (template_name, template) in [
        ("controls", CONTROLS_TEMPLATE),
        ("directions", DIRECTIONS_TEMPLATE),
        ("editor", EDITOR_TEMPLATE),
        ("tasks", TASKS_TEMPLATE),
    ] {
        assert!(
            template.contains("data-confirm="),
            "{template_name} should mark risky submit buttons"
        );
    }

    // exact count는 mutating button 추가/삭제가 confirmation contract 변경으로 review되도록 강제한다.
    assert_eq!(CONTROLS_TEMPLATE.matches("data-confirm=").count(), 4);
    assert_eq!(DIRECTIONS_TEMPLATE.matches("data-confirm=").count(), 2);
    assert_eq!(EDITOR_TEMPLATE.matches("data-confirm=").count(), 1);
    assert_eq!(TASKS_TEMPLATE.matches("data-confirm=").count(), 2);
}

#[test]
fn controls_page_uses_agent_profiles_for_parallel_agent_prompting() {
    assert!(CONTROLS_TEMPLATE.contains("Agent Profiles"));
    assert!(CONTROLS_TEMPLATE.contains("name=\"profiles_json\""));
    assert!(CONTROLS_TEMPLATE.contains("persona_prompt"));
    assert!(ADMIN_MOD.contains("\"/admin/controls/agent-profiles\""));
    assert!(!CONTROLS_TEMPLATE.contains("Parallel Agent Persona"));
    assert!(!CONTROLS_TEMPLATE.contains("name=\"persona\""));
    assert!(!ADMIN_MOD.contains("\"/admin/controls/parallel-persona\""));
}
