//! 기존 커널/SQLite fixture를 사용하고 실제 실행기나 provider 쓰기는 연결하지 않는다.
use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueTriageFence, IssueWorkAdmissionResult, IssueWorkSaveResult,
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

/// 결정 생성 이전의 host diff 관측만 준비한다. 답변은 아래 시험의 실제 커널 명령으로 보낸다.
fn open_delivery(
    services: &Services,
    fixture: &Fixture,
    id: &str,
    source: &str,
) -> (IssueWorkCheckpoint, IssueDecisionCommit) {
    services
        .issues
        .set_mode(IssueCollectionMode::Observe)
        .unwrap();
    let batch = services
        .store
        .read_triage_batch(&fixture.workspace, None, 100)
        .unwrap();
    let item = &batch.items[0];
    let assessment = &item.triage.as_ref().unwrap().assessment;
    let fence = IssueTriageFence {
        collection_epoch: batch.collection.as_ref().unwrap().epoch,
        observation_revision: item.observation_revision,
        tracked_revision: item.tracked_revision,
        assessment_revision: Some(item.triage.as_ref().unwrap().revision),
    };
    let mut work = match services
        .store
        .get_issue_work(&fixture.workspace, &item.issue)
        .unwrap()
    {
        Some(work) => work,
        None => {
            let Some(IssueSourceRead::Observed(observation)) = &item.current else {
                panic!("fixture observation is present")
            };
            let IssueSourceContent::Shared(snapshot) = &observation.content else {
                panic!("fixture content is shared")
            };
            let candidate = IssueWorkCheckpoint {
                authorization: None,
                permit_authorization_epoch: 0,
                order: IssueWorkOrder {
                    work_order_id: issue_work_order_id(&item.issue),
                    issue: item.issue.clone(),
                    field_hash: snapshot.basis.field_hash.clone(),
                    title: snapshot.shared_copy.title.clone(),
                    handoff: snapshot.shared_copy.clone(),
                    acceptance: issue_acceptance_criteria(&snapshot.shared_copy.body).unwrap(),
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
                approved_basis: IssueDecisionBasis::from_assessment(
                    assessment,
                    IssueAutomationGate::Start,
                    None,
                )
                .unwrap(),
                task_ids: vec![],
                permit_source: None,
                permit_revision: 0,
                delivery_granted: false,
                delivery_assessment: None,
                delivery_revision: 0,
                blocked_reason: None,
                revision: 0,
            };
            let IssueWorkAdmissionResult::Saved { record, .. } = services
                .store
                .admit_issue_work(
                    &fixture.workspace,
                    fence,
                    assessment,
                    &candidate,
                    &IssueAutomationBudget {
                        max_running: 1,
                        max_started_per_day: 1,
                        max_attempts: 1,
                    },
                    0,
                )
                .unwrap()
            else {
                panic!("fixture work must be admitted")
            };
            *record
        }
    };
    let mut basis = IssueDecisionBasis::from_assessment(
        assessment,
        IssueAutomationGate::Delivery,
        Some(work.order.work_order_id.clone()),
    )
    .unwrap();
    let triage_gate_revision = basis.gate_revision;
    basis.authorization_epoch = work.current_authorization_basis().authorization_epoch;
    basis.gate_revision = work.delivery_revision.max(triage_gate_revision) + 1;
    basis.delivery = Some(IssueDecisionDeliveryContext {
        base_commit_sha: "a".repeat(40),
        source_commit_sha: source.repeat(40),
    });
    let verdict = IssueDeliveryDiffVerdict {
        changed_files: 2,
        changed_lines: Some(17),
        reasons: vec![IssueDeliveryRiskReason::ProtectedPath],
    };
    work.delivery_revision = basis.gate_revision;
    work.delivery_assessment = Some(IssueDeliveryAssessment {
        basis: basis.clone(),
        triage_gate_revision,
        verdict: verdict.clone(),
    });
    work.delivery_granted = false;
    work.blocked_reason = Some(IssueWorkBlockReason::AwaitingDeliveryApproval);
    let IssueWorkSaveResult::Saved { record, .. } = services
        .store
        .save_issue_work(&fixture.workspace, fence, assessment, &work)
        .unwrap()
    else {
        panic!("fixture delivery assessment must be saved")
    };
    services.issues.set_mode(IssueCollectionMode::Off).unwrap();
    let mut definition = services.definition();
    definition.kind = IssueDecisionKind::RiskException;
    definition.gate = IssueAutomationGate::Delivery;
    definition.basis = basis;
    definition.delivery_review = Some(verdict);
    let commit = services
        .decisions
        .ensure_request(IssueDecisionId::new(id).unwrap(), definition)
        .unwrap();
    (*record, commit)
}

#[test]
fn kernel_simulation_delivery_review_survives_query_human_command_and_replay() {
    let fixture = Fixture::new();
    let source = Source::new();
    let services = fixture.services(source.clone());
    services.seed();
    let (work, opened) = open_delivery(&services, &fixture, "delivery-current", "b");
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log.clone());
    let events = subscribe(&mut sim);
    sim.host
        .runtime
        .record_issue_decision_for_test(&opened.record.request.fact(), opened.catalog_revision);
    sim.host
        .run_cycle(Instant::now() + FAKE_CLOCK_OFFSET, Duration::ZERO);
    let reads = source.calls.load(Ordering::SeqCst);
    let before = query(&mut sim, &work.order.issue);
    let wire = serde_json::to_value(IssueState::from_domain(&before)).unwrap();
    let request = &wire["open_decisions"][0];
    assert_eq!(request["gate"], "delivery");
    assert_eq!(request["kind"], "risk_exception");
    assert_eq!(
        request["basis"]["delivery"]["base_commit_sha"],
        "a".repeat(40)
    );
    assert_eq!(
        request["basis"]["delivery"]["source_commit_sha"],
        "b".repeat(40)
    );
    assert_eq!(
        request["delivery_review"],
        serde_json::json!({"changed_files":2,"changed_lines":17,"reasons":["protected_path"]})
    );
    let requested = live_decisions(&events);
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0]["decision"]["basis"], request["basis"]);
    assert_eq!(
        requested[0]["decision"]["delivery_review"],
        request["delivery_review"]
    );

    let blocked = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("delivery-current"),
        "delivery-unconfirmed",
        None,
    );
    assert_confirmation_rejected(&blocked);
    assert_eq!(query(&mut sim, &work.order.issue).revision, before.revision);
    assert!(live_decisions(&events).is_empty());
    let preview = services
        .decisions
        .prepare_confirmation("delivery-current", "proceed")
        .unwrap();
    assert!(preview.display.detail.contains(&"b".repeat(40)));
    let accepted = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("delivery-current"),
        "delivery-confirmed",
        Some(verified(preview.target)),
    );
    let (fact, outcome, changed, _, receipt) = done(&accepted);
    assert_eq!(outcome, IssueDecisionOutcome::Answered);
    assert!(changed);
    assert_eq!(fact.answer_by, Some(IssueDecisionAnswerBy::Human));
    assert_eq!(fact.basis, opened.record.request.definition().basis);
    assert_eq!(
        fact.delivery_review,
        opened.record.request.definition().delivery_review
    );
    let resolved = live_decisions(&events);
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0]["decision"]["delivery_review"],
        request["delivery_review"]
    );
    assert_eq!(
        services
            .store
            .get_issue_work(&fixture.workspace, &work.order.issue)
            .unwrap()
            .unwrap(),
        work,
        "recording an answer cannot issue an execution permit"
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), reads);
    assert_no_execution_effects(&mut sim, &fixture);
    drop(sim);

    let mut restarted = fixture.boot(&services, log.clone());
    let replay = submit(
        &mut restarted,
        &fixture.workspace,
        answer_command("delivery-current"),
        "delivery-confirmed",
        None,
    );
    assert!(
        matches!(replay, KernelCommandOutcome::Replayed(other) if other.op_id == receipt.op_id)
    );
    assert!(
        query(&mut restarted, &work.order.issue)
            .item
            .unwrap()
            .open_decisions
            .is_empty()
    );
    assert_no_execution_effects(&mut restarted, &fixture);
    drop(restarted);
    let records = log.0.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.event.kind() == "issue_decision_resolved")
            .count(),
        1
    );
    assert!(
        !serde_json::to_string(&*records)
            .unwrap()
            .contains(REFERENCE)
    );
}

#[test]
fn kernel_simulation_delivery_replacement_rejects_old_proof_and_supersedes_old_basis() {
    let fixture = Fixture::new();
    let source = Source::new();
    let services = fixture.services(source.clone());
    services.seed();
    let (_, old) = open_delivery(&services, &fixture, "delivery-old", "b");
    let old_target = services
        .decisions
        .prepare_confirmation("delivery-old", "proceed")
        .unwrap()
        .target;
    let (work, current) = open_delivery(&services, &fixture, "delivery-new", "c");
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log);
    let events = subscribe(&mut sim);
    let reads = source.calls.load(Ordering::SeqCst);
    let wrong = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("delivery-new"),
        "delivery-old-proof-new-request",
        Some(verified(old_target.clone())),
    );
    assert_confirmation_rejected(&wrong);
    assert!(live_decisions(&events).is_empty());
    let obsolete = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("delivery-old"),
        "delivery-obsolete",
        Some(verified(old_target)),
    );
    let (fact, outcome, changed, _, _) = done(&obsolete);
    assert_eq!(outcome, IssueDecisionOutcome::Superseded);
    assert!(changed);
    assert_eq!(fact.answer_by, None);
    assert_eq!(fact.basis, old.record.request.definition().basis);
    assert_eq!(
        fact.delivery_review,
        old.record.request.definition().delivery_review
    );
    let resolved = live_decisions(&events);
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0]["decision"]["basis"]["delivery"]["source_commit_sha"],
        "b".repeat(40)
    );
    assert_eq!(resolved[0]["reason"], "decision_superseded");
    let detail = query(&mut sim, &work.order.issue);
    assert_eq!(detail.item.unwrap().open_decisions, vec![current.record]);
    assert_eq!(
        services
            .store
            .get_issue_work(&fixture.workspace, &work.order.issue)
            .unwrap()
            .unwrap(),
        work
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), reads);
    assert_no_execution_effects(&mut sim, &fixture);
}
