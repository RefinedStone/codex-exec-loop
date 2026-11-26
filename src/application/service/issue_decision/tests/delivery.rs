//! 실제 SQLite transaction과 service 경계를 검증한다. 실행기와 provider는 호출하지 않는다.
use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueWorkAdmissionResult, IssueWorkSaveResult,
};
use crate::domain::issue_automation::coordinator::{
    IssueWorkBlockReason, IssueWorkCheckpoint, IssueWorkOrder, IssueWorkStartAuthority,
    issue_work_order_id,
};
use crate::domain::issue_automation::delivery::{
    IssueDeliveryAssessment, IssueDeliveryDiffVerdict, IssueDeliveryRiskReason,
};
use crate::domain::issue_automation::policy::IssueAutomationBudget;
use crate::domain::issue_automation::triage::issue_acceptance_criteria;

fn seed_delivery(
    fixture: &Fixture,
) -> (
    IssueDecisionService,
    IssueWorkCheckpoint,
    IssueDecisionCommit,
) {
    let (service, initial) = setup(fixture, MutableAutomationPolicy::new());
    let issue = initial.record.request.issue();
    let (fence, item) = service.current_item(issue).unwrap();
    let assessment = service.triage.assess_current(&item).unwrap();
    let observation = fixture.observation("I_001");
    let IssueSourceContent::Shared(snapshot) = observation.content else {
        unreachable!()
    };
    let work = IssueWorkCheckpoint {
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            criteria_adoption: None,
            work_order_id: issue_work_order_id(issue),
            issue: issue.clone(),
            field_hash: snapshot.basis.field_hash,
            title: snapshot.shared_copy.title.clone(),
            acceptance: issue_acceptance_criteria(&snapshot.shared_copy.body).unwrap(),
            handoff: snapshot.shared_copy,
            scope: fixture.workspace.clone(),
            branch_hint: None,
            link_reference: None,
            provider_link: false,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        start_authority: IssueWorkStartAuthority::Policy,
        decision_id: None,
        approved_basis: initial.record.request.definition().basis.clone(),
        task_ids: vec![],
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        delivery_revision: 0,
        delivery_assessment: None,
        blocked_reason: None,
        revision: 0,
    };
    let IssueWorkAdmissionResult::Saved { record, .. } = fixture
        .store()
        .admit_issue_work(
            &fixture.workspace,
            fence,
            &assessment,
            &work,
            &IssueAutomationBudget {
                max_running: 1,
                max_started_per_day: 1,
                max_attempts: 1,
            },
            0,
        )
        .unwrap()
    else {
        panic!("the fixture work must be admitted")
    };
    let work = save_assessment(&service, *record, "b", 1);
    let requested = service
        .ensure_request(
            IssueDecisionId::new("delivery-001").unwrap(),
            delivery_definition(&work),
        )
        .unwrap();
    assert!(
        fixture
            .store()
            .get_issue_execution(&fixture.workspace, issue)
            .unwrap()
            .is_none()
    );
    (service, work, requested)
}

fn save_work(
    service: &IssueDecisionService,
    work: &IssueWorkCheckpoint,
) -> Result<IssueWorkSaveResult> {
    let (fence, item) = service.current_item(&work.order.issue)?;
    let triage = service.triage.assess_current(&item)?;
    service
        .store
        .save_issue_work(&service.workspace_dir, fence, &triage, work)
}

fn save_assessment(
    service: &IssueDecisionService,
    mut work: IssueWorkCheckpoint,
    source: &str,
    files: u64,
) -> IssueWorkCheckpoint {
    let (_, item) = service.current_item(&work.order.issue).unwrap();
    let triage = service.triage.assess_current(&item).unwrap();
    let mut basis = IssueDecisionBasis::from_assessment(
        &triage,
        IssueAutomationGate::Delivery,
        Some(work.order.work_order_id.clone()),
    )
    .unwrap();
    let triage_gate_revision = basis.gate_revision;
    basis.authorization_epoch = work.current_authorization_basis().authorization_epoch;
    basis.gate_revision = triage_gate_revision
        .max(work.delivery_revision)
        .max(work.permit_revision)
        + 1;
    basis.delivery = Some(IssueDecisionDeliveryContext {
        base_commit_sha: "a".repeat(40),
        source_commit_sha: source.repeat(40),
    });
    work.delivery_revision = basis.gate_revision;
    work.delivery_granted = false;
    work.delivery_assessment = Some(IssueDeliveryAssessment {
        basis,
        triage_gate_revision,
        verdict: IssueDeliveryDiffVerdict {
            changed_files: files,
            changed_lines: Some(2),
            reasons: vec![IssueDeliveryRiskReason::ProtectedPath],
        },
    });
    let IssueWorkSaveResult::Saved { record, .. } = save_work(service, &work).unwrap() else {
        panic!("the fixture delivery assessment must be current")
    };
    *record
}

fn delivery_definition(work: &IssueWorkCheckpoint) -> IssueDecisionDefinition {
    let delivery = work.delivery_assessment.as_ref().unwrap();
    IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(work.order.issue.clone()),
        gate: IssueAutomationGate::Delivery,
        kind: IssueDecisionKind::RiskException,
        question: shared("고정된 변경 범위를 전달할까요?"),
        options: vec![
            IssueDecisionOption {
                id: "allow".into(),
                label: shared("승인"),
                effect: shared("표시된 범위만 승인한다."),
            },
            IssueDecisionOption {
                id: "reject".into(),
                label: shared("거절"),
                effect: shared("전달을 보류한다."),
            },
        ],
        recommended: Some("reject".into()),
        basis: delivery.basis.clone(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "delivery-coordinator".into(),
        evidence: vec![],
        delivery_review: Some(delivery.verdict.clone()),
    }
}

#[test]
fn delivery_binding_requires_human_and_preserves_start_and_the_first_answer() {
    let fixture = Fixture::new();
    let (service, work, requested) = seed_delivery(&fixture);
    let id = requested.record.request.id().as_str();
    assert!(service.answer(id, "allow", None, &caller()).is_err());
    let verified = human_caller(&service, id, "allow");
    let confirmed = service.answer(id, "allow", None, &verified).unwrap();
    assert_eq!(confirmed.outcome, IssueDecisionOutcome::Answered);
    assert_eq!(
        confirmed.commit.record.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    let replay = service.answer(id, "reject", None, &caller()).unwrap();
    assert_eq!(replay.outcome, IssueDecisionOutcome::AlreadyResolved);
    assert!(!replay.commit.changed);
    assert_eq!(replay.commit.record, confirmed.commit.record);
    let stored = fixture
        .store()
        .get_issue_work(&fixture.workspace, &work.order.issue)
        .unwrap()
        .unwrap();
    assert_eq!(stored.approved_basis, work.approved_basis);
    assert!(stored.approved_basis.delivery.is_none());
    assert!(!stored.delivery_granted);
}

#[test]
fn source_change_supersedes_old_proof_and_keeps_the_old_review_immutable() {
    let fixture = Fixture::new();
    let (service, work, requested) = seed_delivery(&fixture);
    let old_proof = human_caller(&service, requested.record.request.id().as_str(), "allow");
    let changed = save_assessment(&service, work, "c", 2);
    let resolved = service
        .answer(
            requested.record.request.id().as_str(),
            "allow",
            None,
            &old_proof,
        )
        .unwrap();
    assert_eq!(resolved.outcome, IssueDecisionOutcome::Superseded);
    assert!(resolved.commit.record.request.answer().is_none());
    assert_eq!(
        resolved
            .commit
            .record
            .request
            .fact()
            .delivery_review
            .unwrap()
            .changed_files,
        1
    );
    let next = service
        .ensure_request(
            IssueDecisionId::new("delivery-002").unwrap(),
            delivery_definition(&changed),
        )
        .unwrap();
    assert!(
        service
            .answer(next.record.request.id().as_str(), "allow", None, &old_proof)
            .is_err()
    );
    let new_proof = human_caller(&service, next.record.request.id().as_str(), "allow");
    let confirmed = service
        .answer(next.record.request.id().as_str(), "allow", None, &new_proof)
        .unwrap();
    assert_eq!(
        confirmed.commit.record.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
}

#[test]
fn clearing_an_assessment_preserves_the_floor_and_rejects_approval_reuse() {
    let fixture = Fixture::new();
    let (service, mut work, requested) = seed_delivery(&fixture);
    let proof = human_caller(&service, requested.record.request.id().as_str(), "allow");
    service
        .answer(
            requested.record.request.id().as_str(),
            "allow",
            None,
            &proof,
        )
        .unwrap();
    let old = work.delivery_assessment.take().unwrap();
    let IssueWorkSaveResult::Saved { record, .. } = save_work(&service, &work).unwrap() else {
        panic!("clear must be saved")
    };
    let mut cleared = *record;
    assert_eq!(cleared.delivery_revision, old.basis.gate_revision);
    cleared.delivery_assessment = Some(old);
    assert!(save_work(&service, &cleared).is_err());
    cleared.delivery_assessment = None;
    let new_work = save_assessment(&service, cleared, "b", 1);
    let replacement = delivery_definition(&new_work);
    assert!(new_work.delivery_revision > work.delivery_revision);
    assert_ne!(
        replacement.basis,
        requested.record.request.definition().basis
    );
    assert!(
        !new_work
            .delivery_assessment
            .as_ref()
            .unwrap()
            .matches_decision(requested.record.request.definition())
    );
    let next = service
        .ensure_request(IssueDecisionId::new("delivery-003").unwrap(), replacement)
        .unwrap();
    assert!(
        service
            .answer(next.record.request.id().as_str(), "allow", None, &proof)
            .is_err()
    );
}

#[test]
fn metadata_change_invalidates_delivery_without_a_content_hash_change() {
    let fixture = Fixture::new();
    let (service, work, requested) = seed_delivery(&fixture);
    let mut observation = fixture.observation("I_001");
    observation.labels.items.push(IssueSourceLabel {
        id: "new-label".into(),
        name: "metadata-change".into(),
    });
    fixture.seed(vec![observation], &disclosure_version());
    let resolved = service
        .answer(
            requested.record.request.id().as_str(),
            "allow",
            None,
            &caller(),
        )
        .unwrap();
    assert_eq!(resolved.outcome, IssueDecisionOutcome::Superseded);
    assert!(resolved.commit.record.request.answer().is_none());
    assert_eq!(
        fixture
            .store()
            .get_issue_work(&fixture.workspace, &work.order.issue)
            .unwrap()
            .unwrap()
            .approved_basis,
        work.approved_basis
    );
}

#[test]
fn an_unpersisted_review_cannot_reuse_the_current_range_and_basis() {
    let fixture = Fixture::new();
    let (service, work, _) = seed_delivery(&fixture);
    let mut changed = delivery_definition(&work);
    changed.delivery_review.as_mut().unwrap().changed_files = 2;
    assert!(
        service
            .ensure_request(IssueDecisionId::new("altered-review").unwrap(), changed)
            .is_err()
    );
    assert!(
        fixture
            .store()
            .get_issue_decision(
                &fixture.workspace,
                &IssueDecisionId::new("altered-review").unwrap()
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn native_delivery_preview_displays_the_exact_immutable_range_and_review() {
    let fixture = Fixture::new();
    let (service, work, requested) = seed_delivery(&fixture);
    let preview = service
        .prepare_confirmation(requested.record.request.id().as_str(), "allow")
        .unwrap();
    assert!(
        preview
            .display
            .detail
            .contains(&format!("기준 커밋: {}", "a".repeat(40)))
    );
    assert!(
        preview
            .display
            .detail
            .contains(&format!("전달 커밋: {}", "b".repeat(40)))
    );
    assert!(preview.display.detail.contains("변경 파일: 1"));
    assert!(preview.display.detail.contains("변경 줄: 2"));
    assert!(preview.display.detail.contains("검토 사유: 보호 경로 변경"));
    let _changed = save_assessment(&service, work, "c", 2);
    let old_preview = service
        .prepare_confirmation(requested.record.request.id().as_str(), "allow")
        .unwrap();
    assert_eq!(
        old_preview, preview,
        "the original target and display remain one immutable snapshot"
    );
}

#[test]
fn clearing_delivery_never_emits_a_new_permit_fact_or_permits_an_unassessed_regrant() {
    let fixture = Fixture::new();
    let (service, mut work, requested) = seed_delivery(&fixture);
    let proof = human_caller(&service, requested.record.request.id().as_str(), "allow");
    service
        .answer(
            requested.record.request.id().as_str(),
            "allow",
            None,
            &proof,
        )
        .unwrap();
    work.permit_source = Some("b".repeat(40));
    work.permit_revision = work.delivery_revision;
    work.delivery_granted = true;
    let IssueWorkSaveResult::Saved { record, .. } = save_work(&service, &work).unwrap() else {
        panic!("the assessed permit metadata must be saved")
    };
    let granted = fixture
        .store()
        .pending_issue_work_events(&fixture.workspace)
        .unwrap();
    assert_eq!(granted.len(), 1);
    let mut cleared = *record;
    cleared.delivery_granted = false;
    cleared.delivery_assessment = None;
    let IssueWorkSaveResult::Saved { record, .. } = save_work(&service, &cleared).unwrap() else {
        panic!("the cleared assessment must be saved")
    };
    assert_eq!(
        fixture
            .store()
            .pending_issue_work_events(&fixture.workspace)
            .unwrap(),
        granted
    );
    let mut unassessed = *record;
    unassessed.delivery_granted = true;
    assert!(save_work(&service, &unassessed).is_err());
}

#[test]
fn delivery_invalidation_works_when_collection_is_off_and_preserves_authority_history() {
    let fixture = Fixture::new();
    let (service, mut work, _) = seed_delivery(&fixture);
    work.permit_source = Some("b".repeat(40));
    work.permit_revision = work.delivery_revision;
    work.delivery_granted = true;
    let IssueWorkSaveResult::Saved { record, .. } = save_work(&service, &work).unwrap() else {
        panic!("permit metadata must be saved")
    };
    let work = *record;
    let store = fixture.store();
    let previous_events = store.pending_issue_work_events(&fixture.workspace).unwrap();
    let control = store.get_collection(&fixture.workspace).unwrap().unwrap();
    store
        .set_collection_mode(
            &fixture.workspace,
            Some(control.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    let reason = IssueWorkBlockReason::ApprovalChanged;
    let IssueWorkSaveResult::Saved {
        record, changed, ..
    } = store
        .clear_issue_delivery(&fixture.workspace, &work.order.issue, work.revision, reason)
        .unwrap()
    else {
        panic!("Off must permit delivery authority reduction")
    };
    assert!(changed);
    let mut expected = work.clone();
    expected.delivery_assessment = None;
    expected.delivery_granted = false;
    expected.blocked_reason = Some(reason);
    expected.revision += 1;
    assert_eq!(
        *record, expected,
        "only delivery authority and the CAS revision may change"
    );
    assert_eq!(
        store.pending_issue_work_events(&fixture.workspace).unwrap(),
        previous_events
    );
    let repeated = store
        .clear_issue_delivery(
            &fixture.workspace,
            &work.order.issue,
            record.revision,
            reason,
        )
        .unwrap();
    assert!(matches!(
        repeated,
        IssueWorkSaveResult::Saved { changed: false, .. }
    ));
}

#[test]
fn delivery_invalidation_rejects_a_stale_work_revision_without_touching_newer_review() {
    let fixture = Fixture::new();
    let (service, old, _) = seed_delivery(&fixture);
    let current = save_assessment(&service, old.clone(), "c", 2);
    let store = fixture.store();
    let result = store
        .clear_issue_delivery(
            &fixture.workspace,
            &old.order.issue,
            old.revision,
            IssueWorkBlockReason::SourceUnavailable,
        )
        .unwrap();
    assert_eq!(result, IssueWorkSaveResult::Conflict);
    assert_eq!(
        store
            .get_issue_work(&fixture.workspace, &old.order.issue)
            .unwrap()
            .unwrap(),
        current
    );
    let IssueWorkSaveResult::Saved { record, .. } = store
        .clear_issue_delivery(
            &fixture.workspace,
            &current.order.issue,
            current.revision,
            IssueWorkBlockReason::SourceUnavailable,
        )
        .unwrap()
    else {
        panic!("retry with the current revision must clear delivery")
    };
    assert!(record.delivery_assessment.is_none());
    assert_eq!(record.delivery_revision, current.delivery_revision);
    assert_eq!(record.approved_basis, current.approved_basis);
}
