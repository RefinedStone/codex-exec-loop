use super::pages::{extract_file_updates, nav_for_kind};
use crate::application::service::planning::{PlanningAdminDraftKind, PlanningAdminFileKey};
use std::collections::HashMap;

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
