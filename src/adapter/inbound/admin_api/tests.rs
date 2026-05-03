use super::pages::{extract_file_updates, nav_for_kind};
use crate::application::service::planning::{PlanningAdminDraftKind, PlanningAdminFileKey};
use std::collections::HashMap;

// Admin API tests protect the inbound HTML/form boundary rather than service internals.
// Templates are included as compile-time fixtures so risky-action confirmation markers cannot drift silently.
const BASE_TEMPLATE: &str = include_str!("../../../../templates/admin/base.html");
const CONTROLS_TEMPLATE: &str = include_str!("../../../../templates/admin/controls.html");
const DIRECTIONS_TEMPLATE: &str = include_str!("../../../../templates/admin/directions.html");
const EDITOR_TEMPLATE: &str = include_str!("../../../../templates/admin/editor.html");
const TASKS_TEMPLATE: &str = include_str!("../../../../templates/admin/tasks.html");

// Removed raw-authority fields may still appear in stale browsers or old bookmarks.
// The parser must ignore those names instead of turning them into application-level file mutations.
#[test]
fn page_mutation_ignores_removed_raw_authority_file_updates() {
    // Keep one currently supported field in the same payload so the test proves selective filtering, not wholesale failure.
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

// Draft-kind navigation is adapter policy: service enums must land on the tab that owns the corresponding HTML flow.
// This also locks the removal of the raw task authority draft kind from the visible navigation surface.
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

// Browser confirmation is the last inbound guard before destructive admin POSTs leave the page.
// The test checks both halves of that guard: the global submit hook and per-button data-confirm markers.
#[test]
fn risky_admin_mutations_require_browser_confirmation() {
    // Capture-phase registration keeps nested forms/buttons from bypassing the confirmation hook.
    assert!(BASE_TEMPLATE.contains("document.addEventListener(\"submit\""));
    assert!(BASE_TEMPLATE.contains("}, true);"));

    // The first pass gives a page-specific failure when a template loses all risky-action markers.
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

    // Exact counts force reviewers to account for each new or removed mutating button in the confirmation contract.
    assert_eq!(CONTROLS_TEMPLATE.matches("data-confirm=").count(), 4);
    assert_eq!(DIRECTIONS_TEMPLATE.matches("data-confirm=").count(), 2);
    assert_eq!(EDITOR_TEMPLATE.matches("data-confirm=").count(), 1);
    assert_eq!(TASKS_TEMPLATE.matches("data-confirm=").count(), 2);
}
