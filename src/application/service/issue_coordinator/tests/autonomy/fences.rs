use super::*;
use crate::application::service::issue_intervention::recover_pending_controls;
use crate::domain::issue_automation::autonomy::IssueAutonomyIntent;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest,
};
use sha2::{Digest, Sha256};
use std::sync::Barrier;

fn pending_intent(h: &Harness, id: &str) -> IssueAutonomyIntent {
    let IssueAutonomyProjection::Ready(current) = autonomy(h).state() else {
        panic!("expected current autonomy policy");
    };
    let intent = IssueAutonomyIntent {
        command_id: id.into(),
        level: IssueAutonomyLevel::Off,
        expected_revision: current.revision,
        policy_version: current.policy_version,
    };
    h.fixture
        .store()
        .begin_issue_autonomy(&h.fixture.workspace, &intent)
        .unwrap();
    intent
}

#[test]
fn a_preseeded_completed_control_cannot_skip_the_new_off_fence() {
    let h = Harness::new();
    approved_work(&h);
    let work = h.work().unwrap();
    let command = "off-after-historical-control";
    let historical_id = format!(
        "autonomy-{:x}",
        Sha256::digest(serde_json::to_vec(&(command, &work.order.work_order_id)).unwrap(),)
    );
    // Model a v6 receipt admitted before the internal namespace was reserved.
    let historical = IssueInterventionRequest {
        command_id: historical_id.clone(),
        issue: work.order.issue.clone(),
        action: IssueInterventionAction::Retriage,
    };
    let store = h.fixture.store();
    store
        .begin_issue_intervention(&h.fixture.workspace, &historical)
        .unwrap();
    recover_pending_controls(&h.fixture.workspace, store.as_ref(), h.execution.as_ref()).unwrap();
    let first_receipt = store
        .get_issue_intervention(&h.fixture.workspace, &historical_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        first_receipt.receipt.as_ref().unwrap().authorization_epoch,
        1
    );

    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    let fresh = h
        .execution
        .state
        .lock()
        .unwrap()
        .grants
        .last()
        .unwrap()
        .clone();
    {
        let state = h.execution.state.lock().unwrap();
        assert_eq!(state.grants.len(), 2);
        assert!(!state.revoked);
        assert_eq!(fresh.authorization_epoch, 1);
        state
            .control
            .require_epoch(fresh.authorization_epoch)
            .unwrap();
    }

    assert!(
        autonomy(&h)
            .set(command.into(), IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
    let state = h.execution.state.lock().unwrap();
    assert!(state.revoked);
    assert_eq!(state.control.authorization_epoch, 2);
    assert_eq!(state.control_commands.len(), 2);
    assert!(
        state
            .control_commands
            .contains_key(&format!("{historical_id}-1"))
    );
    drop(state);
    assert_eq!(
        store
            .get_issue_intervention(&h.fixture.workspace, &historical_id)
            .unwrap(),
        Some(first_receipt)
    );
    let mut delayed = fresh;
    delayed.gate_revision += 100;
    delayed.source_sha = "b".repeat(40);
    assert!(h.execution.grant(&h.fixture.workspace, &delayed).is_err());
    assert!(
        !autonomy(&h)
            .set(command.into(), IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
    assert_eq!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control
            .authorization_epoch,
        2
    );
}

#[test]
fn a_generically_completed_owned_fence_still_requires_revocation_before_finalize() {
    let h = Harness::new();
    approved_work(&h);
    let work = h.work().unwrap();
    let store = h.fixture.store();
    let intent = pending_intent(&h, "owned-recovery");
    let reserved = store
        .claim_issue_autonomy_fence(&h.fixture.workspace, &intent, &work)
        .unwrap();
    assert!(reserved.receipt.is_none());
    assert!(
        store
            .finish_issue_autonomy(&h.fixture.workspace, &intent)
            .is_err()
    );

    recover_pending_controls(&h.fixture.workspace, store.as_ref(), h.execution.as_ref()).unwrap();
    let completed = store
        .get_issue_intervention(&h.fixture.workspace, &reserved.request.command_id)
        .unwrap()
        .unwrap();
    assert!(completed.receipt.is_some());
    h.execution.state.lock().unwrap().fail_revoke_work_order =
        Some(work.order.work_order_id.clone());
    assert!(autonomy(&h).recover_before_runtime().is_err());
    assert!(
        store
            .issue_autonomy_selection(&h.fixture.workspace)
            .unwrap()
            .pending
            .is_some()
    );
    assert_eq!(
        store
            .get_issue_intervention(&h.fixture.workspace, &reserved.request.command_id)
            .unwrap(),
        Some(completed.clone())
    );

    h.execution.state.lock().unwrap().fail_revoke_work_order = None;
    autonomy(&h).recover_before_runtime().unwrap();
    autonomy(&h).recover_before_runtime().unwrap();
    let selection = store
        .issue_autonomy_selection(&h.fixture.workspace)
        .unwrap();
    assert_eq!(selection.requested_level, Some(IssueAutonomyLevel::Off));
    assert_eq!(selection.revision, 1);
    assert!(selection.pending.is_none());
    assert_eq!(
        store
            .get_issue_intervention(&h.fixture.workspace, &reserved.request.command_id)
            .unwrap(),
        Some(completed)
    );
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.control.authorization_epoch, 1);
    assert_eq!(state.control_commands.len(), 1);
    assert!(
        state
            .revoked_work_orders
            .contains(&work.order.work_order_id)
    );
}

#[test]
fn competing_owned_fence_claims_reserve_one_control_and_restart_reuses_it() {
    let h = Harness::new();
    approved_work(&h);
    let work = h.work().unwrap();
    let intent = pending_intent(&h, "concurrent-fence");
    let barrier = Arc::new(Barrier::new(2));
    let claims = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let store = h.fixture.store();
                let workspace = h.fixture.workspace.clone();
                let work = work.clone();
                let intent = intent.clone();
                scope.spawn(move || {
                    barrier.wait();
                    store
                        .claim_issue_autonomy_fence(&workspace, &intent, &work)
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(claims[0], claims[1]);
    assert!(claims[0].receipt.is_none());
    let store = h.fixture.store();
    assert_eq!(
        store
            .pending_issue_interventions(&h.fixture.workspace)
            .unwrap(),
        vec![claims[0].clone()]
    );
    let mut different = intent.clone();
    different.command_id = "different-autonomy-command".into();
    assert!(
        store
            .claim_issue_autonomy_fence(&h.fixture.workspace, &different, &work)
            .is_err()
    );
    assert!(
        store
            .finish_issue_autonomy(&h.fixture.workspace, &intent)
            .is_err()
    );

    autonomy(&h).recover_before_runtime().unwrap();
    assert_eq!(h.execution.state.lock().unwrap().control_commands.len(), 1);
    assert_eq!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control
            .authorization_epoch,
        1
    );
    assert!(
        !autonomy(&h)
            .set(intent.command_id, IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
    assert_eq!(h.execution.state.lock().unwrap().control_commands.len(), 1);
}
