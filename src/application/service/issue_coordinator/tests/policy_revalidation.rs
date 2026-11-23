use super::*;

#[test]
fn current_policy_failures_revoke_dispatch_and_delivery_permits() {
    for (unavailable, delivery) in [(true, false), (false, false), (true, true), (false, true)] {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        h.tick(true).unwrap();
        if delivery {
            h.execution.state.lock().unwrap().frozen = Some("b".repeat(40));
            h.tick(true).unwrap();
        }
        let before = h.work().unwrap();
        let grants_before = h.execution.state.lock().unwrap().grants.len();
        let revocations_before = h.execution.state.lock().unwrap().revoked_work_orders.len();
        let reads_before = h.source.calls.load(Ordering::SeqCst);
        assert_eq!(before.delivery_granted, delivery);
        if unavailable {
            h.policy.unavailable.store(true, Ordering::Release);
        } else {
            *h.policy.text.lock().unwrap() = "[binding.github".into();
        }

        assert!(h.tick(true).is_err());

        {
            let state = h.execution.state.lock().unwrap();
            assert!(state.revoked);
            assert_eq!(
                &state.revoked_work_orders[revocations_before..],
                std::slice::from_ref(&before.order.work_order_id)
            );
            assert_eq!(state.grants.len(), grants_before);
            assert_eq!(state.registrations, 1);
        }
        assert_eq!(h.source.calls.load(Ordering::SeqCst), reads_before);
        assert_eq!(h.work().unwrap().order, before.order);

        h.policy.unavailable.store(false, Ordering::Release);
        *h.policy.text.lock().unwrap() = AUTO_POLICY.into();
        h.tick(true).unwrap();
        assert_eq!(
            h.execution.state.lock().unwrap().grants.len(),
            grants_before,
            "policy readability alone must not replay a revoked permit at the same revision"
        );
        if !delivery {
            // A new frozen result is revalidated and must exceed the revoked revision floor.
            h.execution.state.lock().unwrap().frozen = Some("b".repeat(40));
            h.tick(true).unwrap();
            let recovered = h.work().unwrap();
            assert!(recovered.delivery_granted);
            assert!(recovered.permit_revision > before.permit_revision);
            assert_eq!(
                recovered.permit_source.as_deref(),
                Some("b".repeat(40).as_str())
            );
            assert_eq!(
                h.execution.state.lock().unwrap().grants.len(),
                grants_before + 1
            );
            assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
            assert_eq!(h.source.calls.load(Ordering::SeqCst), reads_before + 1);
        }
    }
}

#[test]
fn unreadable_policy_without_existing_work_does_not_revoke_or_submit() {
    let h = Harness::new();
    h.policy.unavailable.store(true, Ordering::Release);

    assert!(h.tick(true).is_err());

    assert!(h.work().is_none());
    let state = h.execution.state.lock().unwrap();
    assert!(!state.revoked);
    assert!(state.revoked_work_orders.is_empty());
    assert!(state.grants.is_empty());
    assert_eq!(state.registrations, 0);
}

#[test]
fn current_policy_failure_checks_later_pages_even_when_an_earlier_revoke_fails() {
    for fail_first_revoke in [false, true] {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        h.tick(true).unwrap();
        let first = h.work().unwrap();
        h.fixture.seed(
            (2..=101)
                .map(|index| h.fixture.observation(&format!("I_{index:03}")))
                .collect(),
            &disclosure_version(),
        );
        run(&h.triage);
        let last_issue = h.fixture.reference("I_101");
        let last = seed_approved_work_checkpoint(&h, &last_issue);
        let store = h.fixture.store();
        let first_page = store
            .read_triage_batch(&h.fixture.workspace, None, 100)
            .unwrap();
        assert!(first_page.next_after.is_some());
        assert!(!first_page.items.iter().any(|item| item.issue == last_issue));
        let last_page = store
            .read_triage_batch(&h.fixture.workspace, first_page.next_after.as_ref(), 100)
            .unwrap();
        assert!(last_page.items.iter().any(|item| item.issue == last_issue));
        if fail_first_revoke {
            h.execution.state.lock().unwrap().fail_revoke_work_order =
                Some(first.order.work_order_id.clone());
        }
        h.policy.unavailable.store(true, Ordering::Release);

        let error = h
            .tick(true)
            .expect_err("the policy failure must remain visible");

        let state = h.execution.state.lock().unwrap();
        assert_eq!(
            state.revoked_work_orders,
            [first.order.work_order_id, last.order.work_order_id],
            "an early error must not leave the later page's permits unchecked"
        );
        assert_eq!(state.grants.len(), 2);
        assert_eq!(state.registrations, 1);
        assert!(state.revoked, "the later work's revocation must succeed");
        if fail_first_revoke {
            assert!(
                error
                    .to_string()
                    .contains("simulated permit revocation failure")
            );
        }
    }
}

// Model a restored second checkpoint through the normal approval and SQLite admission gates.
// No second task is submitted: this test isolates revocation of already persisted work.
pub(super) fn seed_approved_work_checkpoint(
    h: &Harness,
    issue: &IssueReference,
) -> IssueWorkCheckpoint {
    let coordinator = h.coordinator();
    let (_, _, assessment) = coordinator.current(issue, &|_| {}).unwrap();
    let basis = coordinator.approval_basis(&assessment).unwrap();
    let decision_id = start_decision_id(issue, &basis);
    h.decisions
        .ensure_request(decision_id.clone(), start_definition(issue, basis).unwrap())
        .unwrap();
    let preview = h
        .decisions
        .prepare_confirmation(decision_id.as_str(), "start")
        .unwrap();
    use crate::application::port::inbound::stored_command_port::{
        StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
    };
    let context = StoredCommandContext {
        human_confirmation: Some(
            VerifiedHumanConfirmation::from_verified(
                preview.target,
                format!("desktop-confirmation:{}", "f".repeat(32)),
            )
            .unwrap(),
        ),
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: "2026-11-21T00:00:00Z".into(),
    };
    h.decisions
        .answer(decision_id.as_str(), "start", None, &context)
        .unwrap();
    let decision = h
        .fixture
        .store()
        .get_issue_decision(&h.fixture.workspace, &decision_id)
        .unwrap()
        .unwrap();
    let (fence, item, assessment) = coordinator.current(issue, &|_| {}).unwrap();
    let candidate = coordinator
        .build_work(&item, StartAuthorization::Human(&decision.request))
        .unwrap();
    let budget = h
        .triage
        .start_budget_for_policy(&candidate.approved_basis.policy_version)
        .unwrap();
    let IssueWorkAdmissionResult::Saved { record, .. } = h
        .fixture
        .store()
        .admit_issue_work(
            &h.fixture.workspace,
            fence,
            &assessment,
            &candidate,
            &budget,
            123,
        )
        .unwrap()
    else {
        panic!("the restored approved work intent should be admitted");
    };
    let mut work = *record;
    let (fence, _, assessment) = coordinator.current(issue, &|_| {}).unwrap();
    work.task_ids = vec!["task-restored-last-page".into()];
    work.permit_source = Some("a".repeat(40));
    work.permit_revision = work.current_authorization_basis().gate_revision.max(1);
    work.permit_authorization_epoch = work.current_authorization_basis().authorization_epoch;
    work.blocked_reason = Some(IssueWorkBlockReason::AwaitingSource);
    let IssueWorkSaveResult::Saved { record, .. } = h
        .fixture
        .store()
        .save_issue_work(&h.fixture.workspace, fence, &assessment, &work)
        .unwrap()
    else {
        panic!("the restored approved work should be admitted");
    };
    h.execution
        .grant(
            &h.fixture.workspace,
            &IssueWorkPermitRequest {
                delivery_review: None,
                authorization_epoch: record.current_authorization_basis().authorization_epoch,
                work_order_id: record.order.work_order_id.clone(),
                approval: record.current_authorization_basis().clone(),
                source_sha: record.permit_source.clone().unwrap(),
                gate_revision: record.permit_revision,
                delivery: false,
            },
        )
        .unwrap();
    *record
}

#[test]
fn lost_delivery_grant_response_remains_recoverable_with_valid_current_approval() {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    {
        let mut state = h.execution.state.lock().unwrap();
        state.frozen = Some("b".repeat(40));
        state.fail_after_grant = true;
    }

    assert!(h.tick(true).is_err());
    // 새 diff 판정을 저장할 때 이전 Dispatch 허가만 한 번 회수했다.
    // 정확한 범위의 grant 응답 유실은 같은 판의 멱등 복구를 막지 않는다.
    assert_eq!(
        h.execution.state.lock().unwrap().revoked_work_orders.len(),
        1
    );
    h.tick(true).unwrap();

    let work = h.work().unwrap();
    assert!(work.delivery_granted);
    assert_eq!(work.permit_source.as_deref(), Some("b".repeat(40).as_str()));
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.revoked_work_orders.len(), 1);
    assert_eq!(state.grants.len(), 2);
    assert_eq!(state.registrations, 1);
}

#[test]
fn changing_provider_link_policy_does_not_reauthorize_an_immutable_closing_reference() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = format!("{AUTO_POLICY}\n[close]\nwhen = \"provider_link\"\n");
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    let original = h.work().unwrap();
    assert!(original.order.provider_link);
    let grants_before = h.execution.state.lock().unwrap().grants.len();

    *h.policy.text.lock().unwrap() = AUTO_POLICY.into();
    h.tick(true).unwrap();
    // A policy change revokes the old grant. Explicit retriage creates the new authorization
    // epoch before the operator can approve this same work again.
    let store = h.fixture.store();
    store
        .begin_issue_intervention(
            &h.fixture.workspace,
            &crate::domain::issue_automation::intervention::IssueInterventionRequest {
                command_id: "provider-link-policy-reapproval".into(),
                issue: original.order.issue.clone(),
                action: crate::domain::issue_automation::intervention::IssueInterventionAction::Retriage,
            },
        )
        .unwrap();
    crate::application::service::issue_intervention::recover_pending_controls(
        &h.fixture.workspace,
        store.as_ref(),
        h.execution.as_ref(),
    )
    .unwrap();
    h.tick(true).unwrap();
    assert!(
        h.decision().request.definition().basis.authorization_epoch
            > original.current_authorization_basis().authorization_epoch
    );
    h.answer("start");
    h.tick(true).unwrap();

    let current = h.work().unwrap();
    assert_eq!(current.order, original.order);
    assert_eq!(
        current.blocked_reason,
        Some(IssueWorkBlockReason::ApprovalChanged)
    );
    let execution = h.execution.state.lock().unwrap();
    assert!(execution.revoked);
    assert_eq!(execution.registrations, 1);
    assert_eq!(execution.grants.len(), grants_before);
}
