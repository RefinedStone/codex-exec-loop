use super::*;

fn started(enabled: bool) -> Harness {
    let h = Harness::new();
    h.tick(enabled).unwrap();
    h.answer("start");
    h.tick(enabled).unwrap();
    h
}

#[test]
fn pre_dispatch_base_change_revalidates_without_registering_another_task() {
    for enabled in [false, true] {
        let h = started(enabled);
        let before = h.work().unwrap();
        let reads = h.source.calls.load(Ordering::SeqCst);
        h.execution.state.lock().unwrap().dispatch_source = Some("c".repeat(40));

        h.tick(enabled).unwrap();

        let after = h.work().unwrap();
        assert_eq!(after.order, before.order);
        assert_eq!(after.task_ids, before.task_ids);
        assert_eq!(after.decision_id, before.decision_id);
        assert_eq!(after.approved_basis, before.approved_basis);
        assert_eq!(after.permit_source, Some("c".repeat(40)));
        assert_eq!(after.permit_revision, before.permit_revision + 1);
        assert!(!after.delivery_granted);
        assert_eq!(h.source.calls.load(Ordering::SeqCst), reads + 1);
        {
            let state = h.execution.state.lock().unwrap();
            assert_eq!(state.registrations, 1);
            assert_eq!(state.grants.len(), 2);
            let grant = state.grants.last().unwrap();
            assert_eq!(grant.source_sha, "c".repeat(40));
            assert_eq!(grant.gate_revision, after.permit_revision);
            assert_eq!(grant.approval, before.approved_basis);
            assert!(!grant.delivery);
        }

        // 각 tick은 새 coordinator/store adapter로 같은 durable work를 다시 읽는다.
        h.tick(enabled).unwrap();
        h.tick(enabled).unwrap();
        assert_eq!(h.source.calls.load(Ordering::SeqCst), reads + 1);
        let state = h.execution.state.lock().unwrap();
        assert_eq!(state.registrations, 1);
        assert_eq!(state.grants.len(), 2);
    }
}

#[test]
fn pre_dispatch_refresh_advances_beyond_the_saved_gate_revision() {
    let h = started(true);
    let before = h.work().unwrap();
    let saved_revision = before.permit_revision + 3;
    {
        let mut state = h.execution.state.lock().unwrap();
        state.saved_revision = saved_revision;
        state.dispatch_source = Some("c".repeat(40));
    }

    h.tick(true).unwrap();

    let after = h.work().unwrap();
    assert_eq!(after.permit_source, Some("c".repeat(40)));
    assert_eq!(after.permit_revision, saved_revision + 1);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn pre_dispatch_base_change_rechecks_the_remote_issue_before_granting() {
    for scenario in 0..3 {
        let h = started(true);
        let before = h.work().unwrap();
        let grants = h.execution.state.lock().unwrap().grants.clone();
        let reads = h.source.calls.load(Ordering::SeqCst);
        h.execution.state.lock().unwrap().dispatch_source = Some("c".repeat(40));
        let mut observation = h.fixture.observation("I_001");
        let read = match scenario {
            0 => {
                observation.remote_state = IssueRemoteState::Closed;
                IssueSourceRead::Observed(Box::new(observation))
            }
            1 => IssueSourceRead::Unknown {
                issue: observation.issue,
                reason: IssueSourceFailure::ProviderUnavailable,
            },
            _ => {
                let IssueSourceContent::Shared(snapshot) = &mut observation.content else {
                    panic!("fixture must contain shared issue fields");
                };
                snapshot.shared_copy.title = "변경된 작업 범위".into();
                snapshot.basis.field_hash = IssueFieldHash::of_fields(
                    &snapshot.shared_copy.title,
                    &snapshot.shared_copy.body,
                );
                IssueSourceRead::Observed(Box::new(observation))
            }
        };
        *h.source.read.lock().unwrap() = read;

        h.tick(true).unwrap();

        let after = h.work().unwrap();
        assert_eq!(after.order, before.order);
        assert_eq!(after.permit_source, before.permit_source);
        assert_eq!(after.permit_revision, before.permit_revision);
        assert_eq!(
            after.blocked_reason,
            Some(IssueWorkBlockReason::ApprovalChanged)
        );
        assert_eq!(h.source.calls.load(Ordering::SeqCst), reads + 1);
        let state = h.execution.state.lock().unwrap();
        assert!(state.revoked);
        assert_eq!(state.grants, grants);
        assert_eq!(state.registrations, 1);
    }
}

#[test]
fn frozen_source_stays_pinned_when_the_dispatch_base_advances() {
    let h = started(true);
    let dispatch = h.work().unwrap();
    {
        let mut state = h.execution.state.lock().unwrap();
        state.frozen = Some("b".repeat(40));
        state.dispatch_source = Some("c".repeat(40));
    }
    h.tick(true).unwrap();
    let frozen = h.work().unwrap();
    assert_eq!(frozen.permit_source, Some("b".repeat(40)));
    assert_eq!(frozen.permit_revision, dispatch.permit_revision + 1);
    assert!(frozen.delivery_granted);

    let reads = h.source.calls.load(Ordering::SeqCst);
    let grants = h.execution.state.lock().unwrap().grants.clone();
    h.execution.state.lock().unwrap().dispatch_source = Some("d".repeat(40));
    h.tick(true).unwrap();

    let after = h.work().unwrap();
    assert_eq!(after.permit_source, frozen.permit_source);
    assert_eq!(after.permit_revision, frozen.permit_revision);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), reads);
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.grants, grants);
    assert_eq!(state.registrations, 1);
}

#[test]
fn pre_dispatch_base_change_uses_the_renewed_human_authorization_epoch() {
    use crate::application::port::inbound::issue_intervention_port::IssueInterventionPort;
    use crate::application::port::inbound::stored_command_port::StoredCommandContext;
    use crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering;
    use crate::domain::issue_automation::intervention::{
        IssueInterventionAction, IssueInterventionRequest,
    };

    let h = started(true);
    let original = h.work().unwrap();
    let service = crate::application::service::issue_intervention::IssueInterventionService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.execution.clone(),
        Arc::new(UnavailableIssueWorkSteering),
    );
    let context = StoredCommandContext {
        human_confirmation: None,
        caller: Some("test".into()),
        channel: "test".into(),
        at: "2026-11-21T00:00:00Z".into(),
    };
    let request = |id: &str, action| IssueInterventionRequest {
        command_id: id.into(),
        issue: h.fixture.reference("I_001"),
        action,
    };
    service
        .intervene(
            request("hold-before-dispatch", IssueInterventionAction::Hold),
            &context,
        )
        .unwrap();
    h.tick(true).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    service
        .intervene(
            request("release-before-dispatch", IssueInterventionAction::Release),
            &context,
        )
        .unwrap();
    h.tick(true).unwrap();
    assert_eq!(
        h.decision().request.definition().basis.authorization_epoch,
        2
    );
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    h.answer("start");
    h.tick(true).unwrap();
    let renewed = h.work().unwrap();
    assert_eq!(renewed.order, original.order);
    assert_eq!(renewed.approved_basis, original.approved_basis);
    assert_eq!(renewed.current_authorization_basis().authorization_epoch, 2);
    assert_eq!(renewed.permit_authorization_epoch, 2);
    assert_eq!(renewed.permit_source, original.permit_source);
    assert!(renewed.permit_revision > original.permit_revision);

    let reads = h.source.calls.load(Ordering::SeqCst);
    h.execution.state.lock().unwrap().dispatch_source = Some("c".repeat(40));
    h.tick(true).unwrap();

    let after = h.work().unwrap();
    assert_eq!(after.order, original.order);
    assert_eq!(after.task_ids, original.task_ids);
    assert_eq!(after.approved_basis, original.approved_basis);
    assert_eq!(
        after.current_authorization_basis(),
        renewed.current_authorization_basis()
    );
    assert_eq!(after.permit_authorization_epoch, 2);
    assert_eq!(after.permit_source, Some("c".repeat(40)));
    assert_eq!(after.permit_revision, renewed.permit_revision + 1);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), reads + 1);
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.registrations, 1);
    assert_eq!(state.grants.len(), 3);
    let grant = state.grants.last().unwrap();
    assert_eq!(grant.authorization_epoch, 2);
    assert_eq!(&grant.approval, renewed.current_authorization_basis());
    assert!(!grant.delivery);
}
