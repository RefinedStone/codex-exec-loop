use super::pages::{extract_file_updates, nav_for_kind};
use super::{build_admin_state, build_router, parse_reset_target};
use crate::application::service::planning::{
    PlanningAdminDraftKind, PlanningAdminFileKey, PlanningResetTarget,
};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

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
const DASHBOARD_TEMPLATE: &str = include_str!("../../../../templates/admin/dashboard.html");
const AKRA_DASHBOARD_TEMPLATE: &str =
    include_str!("../../../../templates/admin/akra_dashboard.html");
const AKRA_METRICS_TEMPLATE: &str = include_str!("../../../../templates/admin/akra_metrics.html");
const ADMIN_GRAPHIC_VISUAL_SCRIPT: &str =
    include_str!("../../../../scripts/check_admin_graphic_visual.sh");
const GAMEBALJEONGUK_SPRITE_PACK_README: &str =
    include_str!("../../../../templates/admin/resources/gamebaljeonguk_sprite_pack/README.txt");
const GAMEBALJEONGUK_SPRITE_METADATA: &str = include_str!(
    "../../../../templates/admin/resources/gamebaljeonguk_sprite_pack/gamebaljeonguk_sprite_metadata.json"
);
const AKRA_DIORAMA_JS: &str = include_str!("../../../../assets/admin/game/akra-diorama.js");
const AKRA_DIORAMA_TS: &str = include_str!("../../../../assets/admin/game/src/akra-diorama.ts");
const ADMIN_GAME_PACKAGE_JSON: &str = include_str!("../../../../assets/admin/game/package.json");
const ADMIN_GAME_VITE_CONFIG: &str = include_str!("../../../../assets/admin/game/vite.config.ts");
const ADMIN_GAME_PROMOTE_BUILD: &str =
    include_str!("../../../../assets/admin/game/scripts/promote-build.mjs");
const ADMIN_API: &str = include_str!("api.rs");
const AKRA_DASHBOARD_RS: &str = include_str!("akra_dashboard.rs");
const ADMIN_MOD: &str = include_str!("mod.rs");
const ADMIN_PAGES: &str = include_str!("pages.rs");
const ADMIN_STATIC_ASSETS: &str = include_str!("static_assets.rs");

fn source_contains(source: &str, needle: &str) -> bool {
    source.contains(needle) || source.replace("\r\n", "\n").contains(needle)
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
}

impl Drop for TempAdminWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn admin_test_router(workspace: &TempAdminWorkspace) -> Router {
    build_router(build_admin_state(workspace.path.clone()))
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

async fn bootstrap_admin_json_session(router: &Router) -> (String, String) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
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
            Request::builder()
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
    let mut builder = Request::builder()
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
    let mut builder = Request::builder()
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

#[tokio::test]
async fn admin_json_summary_and_runtime_bootstrap_csrf_session() {
    let workspace = TempAdminWorkspace::new("summary-runtime");
    let router = admin_test_router(&workspace);

    let (cookie, csrf_token) = bootstrap_admin_json_session(&router).await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
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
    let workspace = TempAdminWorkspace::new("draft-routes");
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
            Request::builder()
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
async fn admin_akra_events_api_rejects_unbounded_limits() {
    let workspace = TempAdminWorkspace::new("events-limit");
    let router = admin_test_router(&workspace);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
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
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
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
    }
}

#[tokio::test]
async fn admin_html_page_routes_render_live_templates() {
    let workspace = TempAdminWorkspace::new("html-pages");
    let router = admin_test_router(&workspace);
    let (cookie, csrf_token, dashboard_body) = bootstrap_admin_html_session(&router).await;

    assert!(dashboard_body.contains("Planning Admin"));
    assert!(dashboard_body.contains("Open Full Planning Draft"));
    assert!(dashboard_body.contains("name=\"csrf_token\""));
    assert_eq!(csrf_token.len(), 32);

    for (uri, expected) in [
        ("/admin?notice=hello", "hello"),
        ("/admin/directions", "Directions"),
        ("/admin/tasks", "Task catalog view"),
        ("/admin/controls", "Controls"),
        ("/admin/akra", "data-admin-graphic"),
        ("/admin/akra/metrics", "AKRA detached metrics"),
        ("/admin/akra/directions", "게임발전국 작전 방향"),
        ("/admin/akra/tasks", "게임발전국 작업 관리"),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
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
            assert!(!body.contains(r#"<body class="akra-graphic">"#));
        }
        if uri == "/admin/akra/directions" {
            assert!(body.contains(r#"<body class="akra-graphic">"#));
            assert!(body.contains(r#"<a href="/admin/akra/directions" class="active"><span class="nav-icon">G</span><span>작전 방향</span></a>"#));
            assert!(!body.contains(r#"<a href="/admin/directions" class="active">Directions</a>"#));
        }
        if uri == "/admin/akra/tasks" {
            assert!(body.contains(r#"<body class="akra-graphic">"#));
            assert!(body.contains(r#"<a href="/admin/akra/tasks" class="active"><span class="nav-icon">T</span><span>작업 관리</span></a>"#));
            assert!(!body.contains(r#"<a href="/admin/tasks" class="active">Tasks</a>"#));
        }
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
async fn admin_html_draft_routes_render_editor_and_htmx_fragments() {
    let workspace = TempAdminWorkspace::new("html-drafts");
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
            Request::builder()
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
    assert!(loaded_body.contains("file_result_output"));
    assert!(loaded_body.contains("action=\"/admin/drafts/"));

    let draft_body = "# Planning\n\n## Result\n\nHTML draft round trip\n";
    let save_uri = format!("{editor_path}/save");
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

    let validate_uri = format!("{editor_path}/validate");
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

    let promote_uri = format!("{editor_path}/promote");
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
            source.contains(".save_draft("),
            "{label} draft path should call the shared save facade method"
        );
        assert!(
            source.contains(".promote_draft("),
            "{label} draft path should call the shared promote facade method"
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
        r#"<body class="{% if current_nav == "akra_dashboard" || current_nav == "akra_metrics" || current_nav == "akra_directions" || current_nav == "akra_tasks" %}akra-graphic{% endif %}">"#
    ));
    assert!(!BASE_TEMPLATE.contains(
        r#"<body class="{% if current_nav == "akra_dashboard" || current_nav == "akra_metrics" || current_nav == "tasks" %}akra-graphic{% endif %}">"#
    ));
    assert!(BASE_TEMPLATE.contains(
        r#"href="/admin/tasks" class="{% if current_nav == "tasks" %}active{% endif %}""#
    ));
    assert!(BASE_TEMPLATE.contains("akraHashTabRoutes"));
    assert!(BASE_TEMPLATE.contains("window.location.pathname !== \"/admin/akra\""));
    assert!(BASE_TEMPLATE.contains("directions: \"/admin/akra/directions\""));
    assert!(BASE_TEMPLATE.contains("tasks: \"/admin/akra/tasks\""));
    assert!(BASE_TEMPLATE.contains("window.addEventListener(\"hashchange\", redirectAkraHashTab)"));
    assert!(BASE_TEMPLATE.contains(r#"href="/admin/akra/directions" class="{% if current_nav == "akra_directions" %}active{% endif %}"><span class="nav-icon">G</span><span>작전 방향</span></a>"#));
    assert!(BASE_TEMPLATE.contains(r#"href="/admin/akra/tasks" class="{% if current_nav == "akra_tasks" %}active{% endif %}"><span class="nav-icon">T</span><span>작업 관리</span></a>"#));
    assert!(BASE_TEMPLATE.contains("AKRA v0.9.0-beta"));
    assert!(ADMIN_MOD.contains("AKRA_ADMIN_GRAPHIC_ENABLED"));
    assert!(ADMIN_MOD.contains("AKRA_ADMIN_API_BASE_URL"));
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
        "AKRA Admin Control Center",
        "워크트리 풀",
        "배포 파이프라인",
        "실시간 이벤트",
        "시도 보드",
        "최근 시도 로그",
        "정보 카드",
        "data-admin-graphic",
        "data-poll-interval-ms",
        "gamebaljeonguk_atlas_64x96.png",
        "background-image: var(--agent-sprite-sheet)",
        "background-size: 384px 504px",
        "avatar-Artificer",
        "agentAvatarClass",
        "background-image: var(--object-sprite-sheet)",
        "background-size: 627px 627px",
        "role-distributor",
        "role-events",
        "data-focus-target=\"pipeline\"",
        "data-event-drawer",
        "data-detail-drawer",
        "id=\"akra-detail-drawer\"",
        "data-detail-type=\"campaignLane\"",
        "data-detail-type=\"campaignAttempt\"",
        "data-detail-type=\"campaignIntel\"",
        "data-projection-kind",
        "data-agent-id=\"{{ item.source_agent }}\"",
        "data-refresh-dashboard",
        "openDetailDrawer",
        "navigateDetailSelection",
        "selectionTokens",
        "projectionSlotToken",
        "aria-controls",
        "aria-pressed",
        "relatedSelectionCount",
        "openRefreshDetail",
        "data-event-feed-status",
        "akra-office-background.png",
        "akra-object-sprites.png",
        "MISSION FLOW",
        "stage-refresh-btn",
        "detailSourceKey(node) === nextKey",
        "is-bursting",
        "akra:mission-pulse",
        "pulseStage",
        "has-changed",
        "prependEventRows",
        "stale snapshot",
        "pollEvents",
        "/admin/assets/game/akra-diorama.js",
        "data-automation-epoch",
        "akra:dashboard-rendered",
        "renderDashboardPanels",
        "dashboardSignature",
        "renderCampaign",
        "renderBoard",
        "renderPipeline",
        "renderSelectedTask",
        "agents: dashboard.agents || null",
        "pool: dashboard.pool || null",
        "distributor: dashboard.distributor || null",
        "campaign: dashboard.campaign || null",
        "selectedTask: dashboard.selectedTask || null",
        "kpis: dashboard.kpis || null",
        "workspace: dashboard.workspace || null",
        "eventFeed: dashboard.eventFeed || null",
        "events: asArray(dashboard.events)",
        "skeleton-line",
        "campaign-grid",
        "score-chip",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(copy),
            "graphic dashboard should expose {copy}"
        );
    }

    for anchor in [
        "id=\"pool\"",
        "id=\"agents\"",
        "id=\"pipeline\"",
        "id=\"campaign\"",
        "id=\"attempts\"",
        "id=\"intel\"",
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
        "\"/admin/assets/graphics/{asset_name}\"",
        "\"/admin/assets/game/{asset_name}\"",
    ] {
        assert!(
            source_contains(ADMIN_MOD, route),
            "admin route table should keep {route}"
        );
    }

    for token in [
        "mountDiorama",
        "rebuildAgentUnits",
        "PIXI.Application",
        "gamebaljeonguk_atlas_128x192.png",
        "src/akra-diorama.ts",
        "chooseRoamPoint",
        "updateRoamMotion",
        "applyWalkFrame",
        "buildAgentFrameSets",
    ] {
        assert!(
            AKRA_DIORAMA_JS.contains(token),
            "admin game diorama asset should expose {token}"
        );
    }
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
        AKRA_DASHBOARD_TEMPLATE.contains("formatEventFeedStatus"),
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
fn akra_graphic_dashboard_game_bundle_is_vite_typescript_input() {
    for token in [
        "\"build\": \"vite build --config vite.config.ts && node scripts/promote-build.mjs\"",
        "\"check\": \"tsc --noEmit --project tsconfig.json\"",
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
        "type StatusSeverity",
        "interface DioramaHandle",
        "declare const PIXI",
        "const mountDiorama = (): DioramaHandle | null",
        "window.AkraAdminGame",
        "PIXI.Assets.load",
        "app.ticker.add",
        "type Facing = \"down\" | \"side\" | \"up\"",
        "interface AgentFrameSet",
        "interface AgentSpeechBubble",
        "const chooseRoamPoint",
        "const updateRoamMotion",
        "const applyWalkFrame",
        "const speechTextStyleFor",
        "window.getComputedStyle(speechNode)",
        "fontFamily: speechStyle?.fontFamily",
        "const makeSpeechBubble",
        "const applySpeechBubbleFrame",
        "const AGENT_FRAME_WIDTH = 128",
        "const AGENT_FRAME_HEIGHT = 192",
        "const AGENT_SPRITE_SCALE = 0.4675",
        "const AGENT_SPEECH_BUBBLES_DEFAULT_ENABLED = true",
        "setSpeechBubblesEnabled",
        "gamebaljeonguk_atlas_128x192.png",
    ] {
        assert!(
            AKRA_DIORAMA_TS.contains(token),
            "admin game TypeScript source should keep {token}"
        );
    }

    for token in ["dist/akra-diorama.js", "akra-diorama.js", "copyFileSync"] {
        assert!(
            ADMIN_GAME_PROMOTE_BUILD.contains(token),
            "admin game promote script should keep {token}"
        );
    }
}

#[test]
fn akra_graphic_dashboard_visual_contract_has_regression_guardrails() {
    for token in [
        "grid-template-columns: repeat(8",
        "class=\"office-board\" id=\"agents\"",
        "class=\"pool-overlay\" id=\"pool\"",
        "class=\"scene-object object-sprite server-rack\"",
        "background-image: var(--object-sprite-sheet)",
        "background-size: 627px 627px",
        "background-image: var(--agent-sprite-sheet)",
        "background-size: 384px 504px",
        "background-position: -288px 0",
        "--office-board-height: 720px",
        "grid-template-columns: minmax(0, 1fr)",
        "overflow: auto",
        "text-overflow: ellipsis",
        "@media (max-width: 860px)",
        "generated_time_label",
        "automation_epoch",
        "readiness_notice",
        "blocked_action",
        "queue_depth_basis",
        "mock_metric_note",
        "CampaignView",
        "map_campaign",
        "stage {progress}/100",
        "--office-bg-image",
        "--object-sprite-sheet",
        "--agent-sprite-sheet",
        "var(--office-bg-image)",
        "data-detail-type=\"slot\"",
        "data-detail-type=\"distributor\"",
        "data-detail-type=\"queueItem\"",
        "class=\"scene-object desk agent-{{ loop.index }} severity-{{ slot.severity }}\"",
        "data-task-id=\"{{ slot.task_id.as_deref().unwrap_or(\"\") }}\"",
        "avatar-{{ slot.avatar_class_label }}",
        "const createSlotAgentButton",
        "renderAgents(dashboard.pool)",
        "button.append(createText(\"span\", \"speech\", slot.bubbleLabel), sprite, label);",
        "{{ dashboard.distributor.bubble_label }}",
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
        "title=\"{{ slot.display_slot_label }} · {{ slot.label }} · task",
        "const slotDisplayLabel = optionalText(slot.displaySlotLabel || slot.slotId, \"슬롯\")",
        "const slotTaskId = optionalText(slot.taskId, \"-\")",
        "detailTitle: `워크트리 풀 · ${slotDisplayLabel}`",
        "detailSlot: slotDisplayLabel",
        "detailTask: slotTaskId",
        "createText(\"strong\", \"\", slotDisplayLabel)",
        "createText(\"small\", \"\", slotStateLabel)",
        "class=\"admin-detail-drawer\"",
        "pool reconcile, distributor tick, queue mutation은 호출하지 않습니다.",
        "var(--office-bg-image) center / cover no-repeat",
        "akraStageScan",
        "makePacket",
        "statusPalette",
        "chooseRoamPoint",
        "updateRoamMotion",
        "applyWalkFrame",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(token)
                || BASE_TEMPLATE.contains(token)
                || AKRA_DASHBOARD_RS.contains(token)
                || AKRA_DIORAMA_JS.contains(token),
            "graphic visual contract should keep {token}"
        );
    }

    for removed in [
        "class=\"akra-topbar\"",
        "class=\"ops-status\"",
        "class=\"right-stack\"",
        "id=\"metrics\"",
        "id=\"system\"",
        "akra_admin",
        "Last Updated",
        "길드 성과",
        "운영 지표",
        "read-only 운영 관제",
        "게임화 정책",
        "도메인 매핑",
        "blocked-copy",
        "renderOpsStatus",
        "syncTopNotice",
        "renderMetrics",
        "renderSystem",
        "error-notice",
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
        "/admin/assets/graphics/akra-office-background.png",
        "/admin/assets/graphics/akra-object-sprites.png",
        "/admin/assets/graphics/gamebaljeonguk_atlas_64x96.png",
        "/admin/assets/graphics/gamebaljeonguk_atlas_128x192.png",
        "/admin/assets/game/akra-diorama.js",
        "/api/admin/akra/dashboard",
        "/api/admin/akra/events?limit=50",
        "/api/admin/akra/events?afterSequence=0&limit=50",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "${HOME}/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "id=\"campaign\"",
        "id=\"attempts\"",
        "id=\"intel\"",
        "Task catalog view",
        "Skipped tasks",
        "class=\"entity-list\" id=\"task-list\"",
        "data-list-filter=\"task-list\"",
        "\"campaign\"",
        "\"laneCards\"",
        "\"intelCards\"",
        "served office background asset does not match workspace asset",
        "served object sprite asset does not match workspace asset",
        "served gamebaljeonguk agent atlas does not match workspace asset",
        "served large gamebaljeonguk agent atlas does not match workspace asset",
        "--screenshot=",
        "admin graphic visual contract ok",
    ] {
        assert!(
            ADMIN_GRAPHIC_VISUAL_SCRIPT.contains(token),
            "visual regression script should keep {token}"
        );
    }

    for token in [
        "include_bytes!(\"../../../../assets/admin/graphics/akra-office-background.png\")",
        "include_bytes!(\"../../../../assets/admin/graphics/akra-object-sprites.png\")",
        "include_bytes!(\"../../../../assets/admin/graphics/gamebaljeonguk_atlas_64x96.png\")",
        "include_bytes!(\"../../../../assets/admin/graphics/gamebaljeonguk_atlas_128x192.png\")",
        "include_bytes!(\"../../../../assets/admin/game/akra-diorama.js\")",
        "image/png",
        "text/javascript; charset=utf-8",
        "public, max-age=86400",
    ] {
        assert!(
            ADMIN_STATIC_ASSETS.contains(token),
            "admin graphic asset route should keep {token}"
        );
    }
}

#[test]
fn akra_dashboard_reads_planning_queue_through_admin_facade_projection() {
    assert!(
        AKRA_DASHBOARD_RS.contains("load_runtime_application_projection"),
        "dashboard should ask the admin facade for the shared planning projection"
    );
    assert!(
        AKRA_DASHBOARD_RS.contains("inspect_dashboard_snapshot_from_projection"),
        "dashboard should pass planning projection facts into parallel control-plane readiness"
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
fn akra_parallel_admin_surface_is_read_only_snapshot_projection() {
    /*
     * Admin Akra routes inspect parallel mode through the application
     * control-plane composition; they do not provide a second manual
     * tick/mutation surface beside CLI/TUI.
     */
    assert!(
        AKRA_DASHBOARD_RS.contains("inspect_dashboard_snapshot_from_projection"),
        "admin dashboard should render through the parallel control-plane composition"
    );
    assert!(
        AKRA_DASHBOARD_RS.contains("build_runtime_events_snapshot"),
        "admin event feed should render through the control-plane read surface"
    );
    for forbidden in [
        "run_orchestrator_tick",
        "process_distributor_queue",
        "ParallelModeService",
        "ParallelModeControlPlaneCommand",
        "ParallelModeControlPlaneEvent",
    ] {
        assert!(
            !AKRA_DASHBOARD_RS.contains(forbidden),
            "admin dashboard should not issue parallel control-plane commands: {forbidden}"
        );
        assert!(
            !ADMIN_API.contains(forbidden),
            "admin API routes should not issue parallel control-plane commands: {forbidden}"
        );
    }
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
    assert!(BASE_TEMPLATE.contains("document.addEventListener(\"submit\""));
    assert!(BASE_TEMPLATE.contains("}, true);"));

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
