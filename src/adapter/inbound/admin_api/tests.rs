// 학습 주석: admin page helpers는 HTML form 입력을 planning file update로 변환하고 draft kind별 nav 위치를
// 결정합니다. 이 테스트는 inbound admin API의 작은 parsing/mapping boundary를 직접 검증합니다.
use super::pages::{extract_file_updates, nav_for_kind};
// 학습 주석: draft kind와 file key는 application planning admin 계약입니다. adapter test가 이 enum을 확인해
// HTML field 이름이 application boundary로 잘못 매핑되지 않게 합니다.
use crate::application::service::planning::{PlanningAdminDraftKind, PlanningAdminFileKey};
// 학습 주석: form post body를 흉내 내기 위해 field name/value map을 직접 구성합니다.
use std::collections::HashMap;

// 학습 주석: template include는 admin HTML을 compile-time test fixture로 고정합니다. 실제 template 파일이
// confirmation hook이나 data-confirm attribute를 잃으면 이 테스트가 바로 실패합니다.
const BASE_TEMPLATE: &str = include_str!("../../../../templates/admin/base.html");
// 학습 주석: controls page는 start/stop/reset 류의 위험한 admin action을 가진 template입니다.
const CONTROLS_TEMPLATE: &str = include_str!("../../../../templates/admin/controls.html");
// 학습 주석: directions page는 planning direction mutation action을 가진 template입니다.
const DIRECTIONS_TEMPLATE: &str = include_str!("../../../../templates/admin/directions.html");
// 학습 주석: editor page는 staged draft 저장/적용 action을 가진 template입니다.
const EDITOR_TEMPLATE: &str = include_str!("../../../../templates/admin/editor.html");
// 학습 주석: tasks page는 task ledger mutation action을 가진 template입니다.
const TASKS_TEMPLATE: &str = include_str!("../../../../templates/admin/tasks.html");

// 학습 주석: 이 회귀 테스트는 admin form이 raw task authority와 directions file을 직접 수정하지 못하게 한
// 정책을 고정합니다. queue idle prompt만 허용 update로 추출되어야 합니다.
#[test]
fn page_mutation_ignores_removed_raw_authority_file_updates() {
    // 학습 주석: form body에 제거된 legacy field와 아직 허용되는 prompt field를 함께 넣어 parser가
    // unsupported field를 조용히 버리는지 확인합니다.
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

// 학습 주석: raw task authority draft kind가 nav surface에서 사라진 뒤, 남은 draft kinds가 어느 admin tab에
// 속하는지 고정합니다. UI navigation과 backend enum mapping이 어긋나는 회귀를 잡습니다.
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

// 학습 주석: admin API는 브라우저에서 위험한 mutation submit을 실행하므로 template-level confirmation을
// 안전장치로 둡니다. 이 테스트는 JS hook과 각 submit button의 data-confirm marker를 함께 검증합니다.
#[test]
fn risky_admin_mutations_require_browser_confirmation() {
    // 학습 주석: base template의 capture-phase submit listener가 있어야 개별 form submit이 confirmation
    // 없이 서버로 넘어가지 않습니다.
    assert!(BASE_TEMPLATE.contains("document.addEventListener(\"submit\""));
    assert!(BASE_TEMPLATE.contains("}, true);"));

    // 학습 주석: 각 admin page template은 위험 action button에 `data-confirm`을 달아야 합니다. loop로 page별
    // 최소 존재 여부를 먼저 확인해 어느 template이 빠졌는지 메시지에 표시합니다.
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

    // 학습 주석: count assertion은 단순 존재 여부보다 강한 gate입니다. 위험 button이 새로 추가되거나 제거될
    // 때 confirmation coverage를 의식적으로 업데이트하게 만듭니다.
    assert_eq!(CONTROLS_TEMPLATE.matches("data-confirm=").count(), 4);
    assert_eq!(DIRECTIONS_TEMPLATE.matches("data-confirm=").count(), 2);
    assert_eq!(EDITOR_TEMPLATE.matches("data-confirm=").count(), 1);
    assert_eq!(TASKS_TEMPLATE.matches("data-confirm=").count(), 2);
}
