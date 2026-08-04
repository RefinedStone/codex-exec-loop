use super::admin_debug_dashboard::{
    AdminDebugHarnessView, actor_visual_state, build_debug_events_view, build_pool, build_scene,
    stage_readiness,
};
use crate::application::port::inbound::admin_debug_port::{
    AdminDebugHarnessCommand, AdminDebugHarnessConfig, AdminDebugScenario, AdminDebugStage,
};
use crate::application::service::admin_debug_harness::AdminDebugHarnessService;

#[test]
fn fake_projection_generates_stable_incremental_event_sequences() {
    let service = AdminDebugHarnessService::new(AdminDebugHarnessConfig::enabled());
    service
        .execute(AdminDebugHarnessCommand::Step)
        .expect("debug harness should step");
    let projection = service.projection();
    let (_, all_events) = build_debug_events_view(&projection, 20, None);
    let newest = all_events
        .iter()
        .map(|event| event.sequence)
        .max()
        .expect("events should contain the ready and intake stages");
    let (feed, incremental) = build_debug_events_view(&projection, 20, Some(newest - 1));
    assert!(feed.incremental);
    assert_eq!(incremental.len(), 1);
    assert_eq!(incremental[0].sequence, newest);
}

#[test]
fn blocked_recovery_projection_exposes_a_blocked_actor_and_slot() {
    let service = AdminDebugHarnessService::new(AdminDebugHarnessConfig::enabled());
    service
        .execute(AdminDebugHarnessCommand::SelectScenario(
            AdminDebugScenario::BlockedRecovery,
        ))
        .expect("scenario should select");
    for _ in 0..4 {
        service
            .execute(AdminDebugHarnessCommand::Step)
            .expect("scenario should step");
    }
    let projection = service.projection();

    assert_eq!(projection.stage.key(), "blocked");
    let actor_states = (0..3)
        .map(|index| actor_visual_state(projection.stage, index))
        .collect::<Vec<_>>();
    let pool = build_pool(&projection, &actor_states);
    let scene = build_scene(&projection, &actor_states);
    assert_eq!(pool.summary.blocked, 1);
    assert_eq!(
        scene
            .actors
            .iter()
            .filter(|actor| actor.visual_state.to_string() == "blocked")
            .count(),
        1
    );
}

#[test]
fn harness_view_is_disabled_without_application_opt_in() {
    let projection =
        AdminDebugHarnessService::new(AdminDebugHarnessConfig::disabled()).projection();
    let mut view = AdminDebugHarnessView::disabled();
    view.enabled = projection.enabled;
    assert!(!view.enabled);
}

#[test]
fn warning_stages_do_not_present_as_ready() {
    assert_eq!(stage_readiness(AdminDebugStage::QueuePressure), "degraded");
    assert_eq!(stage_readiness(AdminDebugStage::Recovering), "degraded");
    assert_eq!(stage_readiness(AdminDebugStage::Blocked), "blocked");
    assert_eq!(stage_readiness(AdminDebugStage::Working), "ready");
}
