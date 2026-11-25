use std::sync::Arc;

use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderError;
use crate::application::service::planning::task_mutation::{
    PlanningTaskMutationCommand, PlanningTaskMutationRequest, PlanningTaskUpdateInput,
};
use crate::domain::planning::TaskActor;

mod fixture;

use fixture::{FixtureRepository, input};

fn service(repository: &Arc<FixtureRepository>) -> PlanningWorkOrderService {
    PlanningWorkOrderService::new(repository.clone(), PriorityQueueService::new())
}

fn request(repository: &FixtureRepository, work_order_id: &str) -> PlanningWorkOrderRequest {
    PlanningWorkOrderRequest {
        link_reference: None,
        workspace_directory: repository.workspace.clone(),
        work_order_id: work_order_id.to_string(),
        origin: "example_source".to_string(),
        input: input(),
    }
}

fn fixed_time() -> DateTime<Utc> {
    "2026-11-20T12:00:00Z".parse().unwrap()
}

#[test]
fn work_order_replay_returns_first_task_without_validating_replacement_input() {
    let repository = FixtureRepository::new();
    let service = service(&repository);
    let first = service
        .register_work_order(request(&repository, "order:first"))
        .unwrap();
    let before = repository.snapshot();
    let mut replacement = request(&repository, "order:first");
    replacement.input.title.clear();
    replacement.input.direction_id = Some("missing-direction".to_string());
    let replay = service.register_work_order(replacement).unwrap();

    assert!(first.created);
    assert!(!replay.created);
    assert_eq!(replay.task_id, first.task_id);
    assert_eq!(replay.work_order, first.work_order);
    assert!(replay.work_order.permits.is_empty());
    assert_eq!(repository.snapshot(), before);
    assert_eq!(before.task_authority.tasks.len(), 1);
    assert_eq!(before.task_authority.tasks[0].created_by, TaskActor::System);
    assert_eq!(repository.script.lock().unwrap().attempts.len(), 1);
}

#[test]
fn work_order_replay_survives_ordinary_task_provenance_edit_and_service_restart() {
    let repository = FixtureRepository::new();
    let first = service(&repository)
        .register_work_order(request(&repository, "order:edited"))
        .unwrap();
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System).with_thread_turn(
        Some("edited-thread".to_string()),
        Some("edited-turn".to_string()),
    );
    let task_mutation =
        PlanningTaskMutationService::new(repository.clone(), PriorityQueueService::new());
    task_mutation
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: repository.workspace.clone(),
            source: PlanningTaskMutationSource::User,
            legacy_source_turn_id: None,
            provenance: provenance.clone(),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(
                PlanningTaskUpdateInput {
                    acceptance: None,
                    task_id: first.task_id.clone(),
                    direction_id: None,
                    direction_relation_note: None,
                    title: Some("Operator edited the ordinary task".to_string()),
                    description: None,
                    status: None,
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: None,
                    blocked_by: None,
                    expected_planning_revision: None,
                },
            )],
        })
        .unwrap();
    let edited = repository.snapshot();
    assert_eq!(edited.task_authority.tasks[0].provenance, provenance);

    let replay = service(&repository)
        .register_work_order(request(&repository, "order:edited"))
        .unwrap();
    assert!(!replay.created);
    assert_eq!(replay.task_id, first.task_id);
    assert_eq!(repository.snapshot(), edited);
    assert_eq!(repository.script.lock().unwrap().attempts.len(), 1);
}

#[test]
fn work_order_replay_survives_missing_task_and_direction_authority() {
    let repository = FixtureRepository::new();
    let service = service(&repository);
    let first = service
        .register_work_order(request(&repository, "order:reset"))
        .unwrap();
    repository
        .clear_task_authority_snapshot(&repository.workspace)
        .unwrap();
    repository
        .clear_direction_authority_snapshot(&repository.workspace)
        .unwrap();

    let replay = service
        .register_work_order(request(&repository, "order:reset"))
        .unwrap();
    assert!(!replay.created);
    assert_eq!(replay.task_id, first.task_id);
    assert!(
        repository
            .load_task_authority_snapshot(&repository.workspace)
            .unwrap()
            .is_none()
    );
    assert!(
        repository
            .load_direction_authority_snapshot(&repository.workspace)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        service
            .load_work_order_for_task(&repository.workspace, &first.task_id)
            .unwrap(),
        Some(first.work_order)
    );
}

#[test]
fn work_order_revision_retry_reloads_authority_and_keeps_generated_identity_and_time() {
    let repository = FixtureRepository::new();
    repository.script.lock().unwrap().revision_conflicts = 1;
    let admitted = service(&repository)
        .register_work_order_at(request(&repository, "order:conflict"), fixed_time())
        .unwrap();

    assert!(admitted.created);
    assert_eq!(repository.snapshot().task_authority.tasks.len(), 1);
    let script = repository.script.lock().unwrap();
    assert_eq!(script.attempts.len(), 2);
    assert_eq!(script.attempts[0].task_id, script.attempts[1].task_id);
    assert_eq!(script.attempts[0].updated_at, script.attempts[1].updated_at);
    assert_eq!(
        script.attempts[1].observed_revision,
        script.attempts[0].observed_revision + 1
    );
}

#[test]
fn work_order_revision_retry_is_bounded_without_leaving_a_task_or_record() {
    let repository = FixtureRepository::new();
    repository.script.lock().unwrap().revision_conflicts = usize::MAX;
    let error = service(&repository)
        .register_work_order_at(request(&repository, "order:busy"), fixed_time())
        .unwrap_err();

    assert!(error.to_string().contains("planning state kept changing"));
    assert!(repository.snapshot().task_authority.tasks.is_empty());
    let script = repository.script.lock().unwrap();
    assert_eq!(script.attempts.len(), MAX_REVISION_CONFLICT_RETRIES + 1);
    assert!(script.orders.is_empty());
    assert!(
        script
            .attempts
            .iter()
            .all(|attempt| attempt.task_id == script.attempts[0].task_id)
    );
}

#[test]
fn work_order_allocation_respects_active_and_removed_lineage_task_identities() {
    let repository = FixtureRepository::new();
    let service = service(&repository);
    let first = service
        .register_work_order_at(request(&repository, "order:first"), fixed_time())
        .unwrap();
    let second = service
        .register_work_order_at(request(&repository, "order:second"), fixed_time())
        .unwrap();
    assert_ne!(second.task_id, first.task_id);
    assert_eq!(repository.snapshot().task_authority.tasks.len(), 2);

    repository
        .clear_task_authority_snapshot(&repository.workspace)
        .unwrap();
    repository.seed_empty_authority();
    let third = service
        .register_work_order_at(request(&repository, "order:third"), fixed_time())
        .unwrap();
    assert_ne!(third.task_id, first.task_id);
    assert_ne!(third.task_id, second.task_id);
    assert_eq!(repository.snapshot().task_authority.tasks.len(), 1);
    assert_eq!(
        service
            .load_work_order_for_task(&repository.workspace, &first.task_id)
            .unwrap(),
        Some(first.work_order)
    );
    assert_eq!(
        service
            .load_work_order(&repository.workspace, "order:third")
            .unwrap(),
        Some(third.work_order)
    );
    let script = repository.script.lock().unwrap();
    assert_eq!(script.attempts.len(), 5);
    assert!(
        script
            .attempts
            .iter()
            .all(|attempt| attempt.updated_at == script.attempts[0].updated_at)
    );
}

#[test]
fn work_order_found_by_atomic_registration_wins_over_a_stale_preflight_read() {
    let repository = FixtureRepository::new();
    let service = service(&repository);
    let first = service
        .register_work_order_at(request(&repository, "order:raced"), fixed_time())
        .unwrap();
    let before = repository.snapshot();
    repository.script.lock().unwrap().hide_existing_reads = 1;

    // A different valid candidate avoids the active-task ID collision loop. The hidden preflight
    // must therefore reach atomic registration, whose durable identity wins over this new input.
    let mut raced_request = request(&repository, "order:raced");
    raced_request.input.title = "A different candidate after the stale preflight".to_string();
    let replay = service
        .register_work_order_at(raced_request, fixed_time())
        .unwrap();

    assert!(!replay.created);
    assert_eq!(replay.task_id, first.task_id);
    assert_eq!(replay.work_order, first.work_order);
    assert_eq!(repository.snapshot(), before);
    let script = repository.script.lock().unwrap();
    assert_eq!(script.attempts.len(), 2);
    assert_eq!(script.attempts[0].task_id, first.task_id);
    assert_ne!(script.attempts[1].task_id, first.task_id);
    assert_eq!(script.atomic_existing_returns, 1);
}

#[test]
fn work_order_origin_collision_preserves_the_first_record_and_task() {
    let repository = FixtureRepository::new();
    let service = service(&repository);
    let first = service
        .register_work_order(request(&repository, "order:owned"))
        .unwrap();
    let before = repository.snapshot();
    let mut collision = request(&repository, "order:owned");
    collision.origin = "another_source".to_string();
    let error = service.register_work_order(collision).unwrap_err();

    assert_eq!(
        error.downcast_ref::<PlanningWorkOrderError>(),
        Some(&PlanningWorkOrderError::OriginConflict {
            work_order_id: "order:owned".to_string(),
        })
    );
    assert_eq!(repository.snapshot(), before);
    assert_eq!(
        service
            .load_work_order(&repository.workspace, "order:owned")
            .unwrap(),
        Some(first.work_order)
    );
    assert_eq!(repository.script.lock().unwrap().attempts.len(), 1);
}

#[test]
fn invalid_new_work_order_task_uses_existing_planning_validation_without_a_commit() {
    let repository = FixtureRepository::new();
    let before = repository.snapshot();
    let mut invalid = request(&repository, "order:invalid");
    invalid.input.direction_id = Some("missing-direction".to_string());
    let error = service(&repository)
        .register_work_order(invalid)
        .unwrap_err();

    assert!(!error.to_string().is_empty());
    assert_eq!(repository.snapshot(), before);
    let script = repository.script.lock().unwrap();
    assert!(script.orders.is_empty());
    assert!(script.attempts.is_empty());
}

#[test]
fn work_order_replay_preserves_its_publication_reference_and_rejects_scope_changes() {
    let repository = FixtureRepository::new();
    let mut original = request(&repository, "order:link-reference");
    original.link_reference = Some(PlanningWorkLinkReference {
        url: "https://github.com/acme/project/issues/7".into(),
        close_on_merge: false,
        branch_hint: None,
    });
    let first = service(&repository)
        .register_work_order(original.clone())
        .unwrap();
    assert_eq!(first.work_order.link_reference, original.link_reference);
    let replay = service(&repository)
        .register_work_order(original.clone())
        .unwrap();
    assert!(!replay.created);
    assert_eq!(replay.work_order, first.work_order);

    for edit in 0..3 {
        let mut changed = original.clone();
        match edit {
            0 => changed.link_reference = None,
            1 => changed.link_reference.as_mut().unwrap().close_on_merge = true,
            _ => {
                changed.link_reference.as_mut().unwrap().url =
                    "https://github.com/acme/project/issues/8".into()
            }
        }
        assert!(
            service(&repository)
                .register_work_order(changed)
                .unwrap_err()
                .to_string()
                .contains("publication reference changed")
        );
    }
    assert_eq!(repository.snapshot().task_authority.tasks.len(), 1);
}
