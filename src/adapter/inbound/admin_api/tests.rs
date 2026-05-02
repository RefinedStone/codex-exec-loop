// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::pages::{extract_file_updates, nav_for_kind};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::planning::{PlanningAdminDraftKind, PlanningAdminFileKey};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::collections::HashMap;

// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const BASE_TEMPLATE: &str = include_str!("../../../../templates/admin/base.html");
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const CONTROLS_TEMPLATE: &str = include_str!("../../../../templates/admin/controls.html");
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const DIRECTIONS_TEMPLATE: &str = include_str!("../../../../templates/admin/directions.html");
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const EDITOR_TEMPLATE: &str = include_str!("../../../../templates/admin/editor.html");
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const TASKS_TEMPLATE: &str = include_str!("../../../../templates/admin/tasks.html");

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn page_mutation_ignores_removed_raw_authority_file_updates() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn risky_admin_mutations_require_browser_confirmation() {
    assert!(BASE_TEMPLATE.contains("document.addEventListener(\"submit\""));
    assert!(BASE_TEMPLATE.contains("}, true);"));

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
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

    assert_eq!(CONTROLS_TEMPLATE.matches("data-confirm=").count(), 4);
    assert_eq!(DIRECTIONS_TEMPLATE.matches("data-confirm=").count(), 2);
    assert_eq!(EDITOR_TEMPLATE.matches("data-confirm=").count(), 1);
    assert_eq!(TASKS_TEMPLATE.matches("data-confirm=").count(), 2);
}
