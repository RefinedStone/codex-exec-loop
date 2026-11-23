use super::*;
use crate::domain::issue_automation::delivery::*;

fn started() -> Harness {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    h
}

pub(super) fn freeze(h: &Harness, source: char, path: &str) {
    let mut execution = h.execution.state.lock().unwrap();
    let source = source.to_string().repeat(40);
    execution.frozen = Some(source.clone());
    execution.delivery_diff = Some(IssueDeliveryDiff {
        base_commit_sha: "a".repeat(40),
        source_commit_sha: source,
        files: vec![IssueDeliveryDiffFile {
            path: path.into(),
            lines: IssueDeliveryDiffLines::Text {
                added: 3,
                deleted: 1,
            },
        }],
    });
}

fn decision(h: &Harness) -> StoredIssueDecision {
    let work = h.work().unwrap();
    let id = super::super::delivery::delivery_decision_id(
        &work.delivery_assessment.as_ref().unwrap().basis,
    )
    .unwrap();
    h.fixture
        .store()
        .get_issue_decision(&h.fixture.workspace, &id)
        .unwrap()
        .unwrap()
}

pub(super) fn answer(h: &Harness, option: &str) {
    use crate::application::port::inbound::stored_command_port::{
        StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
    };
    let decision = decision(h);
    let id = decision.request.id().as_str();
    let preview = h.decisions.prepare_confirmation(id, option).unwrap();
    h.decisions
        .answer(
            id,
            option,
            None,
            &StoredCommandContext {
                human_confirmation: Some(
                    VerifiedHumanConfirmation::from_verified(
                        preview.target,
                        format!("desktop-confirmation:{}", "e".repeat(32)),
                    )
                    .unwrap(),
                ),
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-23T00:00:00Z".into(),
            },
        )
        .unwrap();
}

fn delivery_grants(h: &Harness) -> usize {
    h.execution
        .state
        .lock()
        .unwrap()
        .grants
        .iter()
        .filter(|grant| grant.delivery)
        .count()
}

#[test]
fn protected_delivery_waits_for_exact_human_answer_without_changing_start() {
    let h = started();
    let start = h.work().unwrap().approved_basis;
    freeze(&h, 'b', ".github/workflows/ci.yml");
    h.tick(true).unwrap();
    let pending = h.work().unwrap();
    let review = pending.delivery_assessment.as_ref().unwrap();
    assert_eq!(
        review.verdict.reasons,
        [IssueDeliveryRiskReason::ProtectedPath]
    );
    assert_eq!(pending.approved_basis, start);
    assert_eq!(
        pending.blocked_reason,
        Some(IssueWorkBlockReason::AwaitingDeliveryApproval)
    );
    assert!(!pending.delivery_granted);
    assert_eq!(delivery_grants(&h), 0);
    assert_eq!(
        decision(&h).request.definition().on_timeout,
        IssueDecisionTimeout::Wait
    );
    h.tick(true).unwrap();
    assert_eq!(
        h.work().unwrap().delivery_revision,
        pending.delivery_revision
    );
    assert_eq!(delivery_grants(&h), 0);
    answer(&h, "deliver");
    h.tick(true).unwrap();
    let submitted = h.work().unwrap();
    assert!(submitted.delivery_granted);
    assert_eq!(submitted.approved_basis, start);
    assert_eq!(submitted.permit_revision, review.basis.gate_revision);
    assert_eq!(delivery_grants(&h), 1);
    h.tick(true).unwrap();
    assert_eq!(delivery_grants(&h), 1);
}

#[test]
fn changed_source_and_returning_to_old_source_never_reuse_old_approval() {
    let h = started();
    freeze(&h, 'b', ".github/workflows/ci.yml");
    h.tick(true).unwrap();
    let original = decision(&h);
    answer(&h, "deliver");
    for source in ['c', 'b'] {
        freeze(&h, source, ".github/workflows/ci.yml");
        h.tick(true).unwrap();
        let current = decision(&h);
        assert_ne!(current.request.id(), original.request.id());
        assert!(
            current.request.definition().basis.gate_revision
                > original.request.definition().basis.gate_revision
        );
        assert_eq!(current.request.state(), IssueDecisionState::Open);
        assert_eq!(delivery_grants(&h), 0);
    }
}

#[test]
fn unreadable_diff_revokes_and_recovery_requires_a_new_human_answer() {
    let h = started();
    freeze(&h, 'b', ".github/workflows/ci.yml");
    h.tick(true).unwrap();
    let old = decision(&h);
    answer(&h, "deliver");
    h.execution.state.lock().unwrap().delivery_diff_unavailable = true;
    assert!(h.tick(true).is_err());
    let cleared = h.work().unwrap();
    assert!(cleared.delivery_assessment.is_none());
    assert!(!cleared.delivery_granted);
    assert_eq!(
        cleared.delivery_revision,
        old.request.definition().basis.gate_revision
    );
    assert!(h.execution.state.lock().unwrap().revoked);
    h.execution.state.lock().unwrap().delivery_diff_unavailable = false;
    h.tick(true).unwrap();
    assert_ne!(decision(&h).request.id(), old.request.id());
    assert_eq!(delivery_grants(&h), 0);
}

#[test]
fn holding_delivery_never_changes_the_start_or_creates_a_permit() {
    let h = started();
    freeze(&h, 'b', ".github/workflows/ci.yml");
    h.tick(true).unwrap();
    let original = h.work().unwrap();
    answer(&h, "hold");
    for _ in 0..2 {
        h.tick(true).unwrap();
    }
    let work = h.work().unwrap();
    assert_eq!(work.approved_basis, original.approved_basis);
    assert_eq!(work.delivery_revision, original.delivery_revision);
    assert!(!work.delivery_granted);
    assert_eq!(delivery_grants(&h), 0);
}

#[test]
fn large_and_binary_diffs_escalate_only_delivery() {
    for lines in [
        IssueDeliveryDiffLines::Text {
            added: 1001,
            deleted: 0,
        },
        IssueDeliveryDiffLines::Binary,
    ] {
        let h = started();
        freeze(&h, 'b', "src/feature.rs");
        h.execution
            .state
            .lock()
            .unwrap()
            .delivery_diff
            .as_mut()
            .unwrap()
            .files[0]
            .lines = lines;
        let start = h.work().unwrap().approved_basis;
        h.tick(true).unwrap();
        assert_eq!(
            decision(&h).request.definition().gate,
            IssueAutomationGate::Delivery
        );
        assert_eq!(h.work().unwrap().approved_basis, start);
        assert_eq!(delivery_grants(&h), 0);
    }
}

#[test]
fn local_tracking_consumes_a_delivery_answer_without_waiting_for_collection() {
    use crate::domain::issue_automation::execution::{
        IssueExecutionPhase, IssueWorkExecutionObservation,
    };
    let h = started();
    freeze(&h, 'b', ".github/workflows/ci.yml");
    h.execution.state.lock().unwrap().observation = Some(IssueWorkExecutionObservation {
        task_ids: vec!["task-1".into()],
        active_task_id: Some("task-1".into()),
        // 실제 PR 생성 전 frozen queue는 Queued다. 최초 판정을 PR 상태에 묶지 않는다.
        phase: IssueExecutionPhase::Queued,
        ..Default::default()
    });
    let track = || {
        h.coordinator()
            .track_execution(&IssueCollectionCancellation::new(), 200, true, &|_| {})
            .unwrap()
    };
    track();
    assert_eq!(delivery_grants(&h), 0);
    answer(&h, "deliver");
    track();
    assert_eq!(delivery_grants(&h), 1);
}

#[test]
fn clearing_a_stale_checkpoint_uses_the_latest_persisted_revision() {
    let h = started();
    freeze(&h, 'b', ".github/workflows/ci.yml");
    h.tick(true).unwrap();
    let mut stale = h.work().unwrap();
    freeze(&h, 'c', ".github/workflows/ci.yml");
    h.tick(true).unwrap();
    let newest = h.work().unwrap();
    assert!(newest.delivery_revision > stale.delivery_revision);
    h.coordinator()
        .invalidate_delivery(&mut stale, IssueWorkBlockReason::SourceUnavailable)
        .unwrap();
    let cleared = h.work().unwrap();
    assert_eq!(cleared.delivery_revision, newest.delivery_revision);
    assert!(cleared.delivery_assessment.is_none());
    assert_eq!(cleared, stale);
}
