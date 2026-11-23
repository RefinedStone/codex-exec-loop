use super::*;
use crate::domain::issue_automation::IssueAutomationFlag;
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::decision::{
    IssueDecisionId, IssueDecisionKind, IssueDecisionTimeout,
};
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::{IssueSourceContent, IssueSourceRead};
use crate::domain::issue_work::IssueFieldHash;

#[test]
fn arbiter_first_human_answer_wins_and_the_late_reply_has_one_discarded_event() {
    let fixture = Fixture::new("arbiter-human-first");
    let prepared = prepare_unanswered_arbiter(&fixture, "I_human", &budget(1, 5));
    let claim = claim_request(&fixture, &prepared, DUE);
    let attempt = start(&fixture, &claim);
    let human = save_human(&fixture, &prepared);
    let result = finished(&fixture, &finish_request(&attempt, &claim));
    assert_eq!(result.record, human);
    assert_eq!(
        result.record.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    assert_eq!(
        result.event.outcome,
        IssueArbiterFinishedOutcome::Discarded {}
    );
    assert_eq!(result.event.fact, human.request.fact());
    assert_eq!(decision(&fixture, &prepared), human);
    assert_eq!(count(&fixture, "issue_arbiter_finished_events"), 1);
}

#[test]
fn arbiter_evidence_and_route_drift_keep_the_same_decision_open_without_old_input_authority() {
    for drift in ["source", "evidence", "input", "route"] {
        let fixture = Fixture::new("arbiter-input-drift");
        let prepared = prepare_unanswered_arbiter(&fixture, "I_drift", &budget(1, 5));
        let original = decision(&fixture, &prepared);
        let claim = claim_request(&fixture, &prepared, DUE);
        let attempt = start(&fixture, &claim);
        let mut finish = finish_request(&attempt, &claim);
        let input = &mut finish.current.as_mut().unwrap().input_basis;
        match drift {
            "source" => input.repository_commit_sha = "e".repeat(40),
            "evidence" => input.evidence_sha256 = "e".repeat(64),
            "input" => input.input_sha256 = "e".repeat(64),
            "route" => input.model_route_revision = "route:2".into(),
            _ => unreachable!(),
        }
        let wait = finished(&fixture, &finish);
        assert_eq!(wait.record, original);
        assert_eq!(
            wait.event.outcome,
            IssueArbiterFinishedOutcome::Waiting {
                reason: if drift == "route" {
                    IssueArbiterWaitReason::ContextUnavailable
                } else {
                    IssueArbiterWaitReason::EvidenceChanged
                },
            }
        );
        assert_eq!(count(&fixture, "issue_work_admissions"), 0);
    }
}

#[test]
fn arbiter_hold_off_stale_or_pending_intervention_blocks_both_claim_and_late_apply() {
    for fence in ["held", "stale", "off", "pending", "taken_over", "cancelled"] {
        for already_claimed in [false, true] {
            let fixture = Fixture::new("arbiter-control-fence");
            let prepared = prepare_unanswered_arbiter(&fixture, "I_control", &budget(1, 5));
            let claim = claim_request(&fixture, &prepared, DUE);
            let attempt = already_claimed.then(|| start(&fixture, &claim));
            let adapter = fixture.adapter();
            match fence {
                "held" | "stale" => {
                    let mut tracked = adapter
                        .get_issue(fixture.cwd(), &prepared.work.order.issue)
                        .unwrap()
                        .unwrap();
                    tracked.set_flag(
                        if fence == "held" {
                            IssueAutomationFlag::Held
                        } else {
                            IssueAutomationFlag::Stale
                        },
                        true,
                    );
                    super::super::super::super::tests::save_changed(
                        &adapter,
                        fixture.cwd(),
                        &tracked,
                    );
                }
                "off" => {
                    adapter
                        .set_collection_mode(
                            fixture.cwd(),
                            Some(prepared.fence.collection_epoch),
                            IssueCollectionMode::Off,
                        )
                        .unwrap();
                }
                _ => {
                    let request = IssueInterventionRequest {
                        command_id: "test-control".into(),
                        issue: prepared.work.order.issue.clone(),
                        action: match fence {
                            "taken_over" => IssueInterventionAction::TakeOver,
                            "cancelled" => IssueInterventionAction::Cancel { confirm: true },
                            _ => IssueInterventionAction::Hold,
                        },
                    };
                    adapter
                        .begin_issue_intervention(fixture.cwd(), &request)
                        .unwrap();
                    if fence != "pending" {
                        adapter
                            .finish_issue_intervention(
                                fixture.cwd(),
                                &IssueInterventionReceipt {
                                    issue: request.issue,
                                    command_id: request.command_id,
                                    action: request.action.label().into(),
                                    work_order_id: None,
                                    authorization_epoch: 1,
                                    changed: true,
                                    delivery: None,
                                    handoff: None,
                                },
                            )
                            .unwrap();
                    }
                }
            }
            if let Some(attempt) = attempt {
                let result = finished(&fixture, &finish_request(&attempt, &claim));
                assert_ne!(
                    result.event.outcome,
                    IssueArbiterFinishedOutcome::Applied {},
                    "{fence}"
                );
                assert!(result.record.request.answer().is_none(), "{fence}");
                if matches!(fence, "taken_over" | "cancelled") {
                    assert_eq!(
                        result.event.outcome,
                        IssueArbiterFinishedOutcome::Superseded {}
                    );
                }
            } else {
                assert_eq!(
                    adapter.claim_issue_arbiter(fixture.cwd(), &claim).unwrap(),
                    IssueArbiterClaimResult::Ineligible,
                    "{fence}"
                );
                assert_eq!(count(&fixture, "issue_arbiter_attempts"), 0);
            }
            assert_eq!(count(&fixture, "issue_work_admissions"), 0);
        }
    }
}

#[test]
fn arbiter_issue_edit_supersedes_the_old_question_even_when_the_current_reader_is_unavailable() {
    let fixture = Fixture::new("arbiter-stale-issue");
    let prepared = prepare_unanswered_arbiter(&fixture, "I_edit", &budget(1, 5));
    let original = decision(&fixture, &prepared);
    let claim = claim_request(&fixture, &prepared, DUE);
    let attempt = start(&fixture, &claim);
    let mut observation = prepared.observation.clone();
    let IssueSourceContent::Shared(snapshot) = &mut observation.content else {
        unreachable!()
    };
    snapshot
        .shared_copy
        .body
        .push_str("\n- 새 완료 조건을 만족한다.\n");
    snapshot.basis.field_hash =
        IssueFieldHash::of_fields(&snapshot.shared_copy.title, &snapshot.shared_copy.body);
    let adapter = fixture.adapter();
    let control = adapter.get_collection(fixture.cwd()).unwrap().unwrap();
    adapter
        .apply_issue_resync(
            fixture.cwd(),
            control.fence(),
            &prepared.work.order.issue,
            &IssueSourceRead::Observed(Box::new(observation)),
        )
        .unwrap();
    let mut finish = finish_request(&attempt, &claim);
    finish.current = None;
    let result = finished(&fixture, &finish);
    assert_eq!(
        result.event.outcome,
        IssueArbiterFinishedOutcome::Superseded {}
    );
    assert_eq!(
        result.record.request.state(),
        IssueDecisionState::Superseded
    );
    assert_eq!(
        result.record.request.definition(),
        original.request.definition()
    );
    assert_eq!(result.record.revision, original.revision + 1);
    assert!(result.record.request.answer().is_none());
}

#[test]
fn arbiter_eligibility_does_not_retrofit_legacy_wait_or_human_only_decision_kinds() {
    let fixture = Fixture::new("arbiter-eligibility");
    let prepared = prepare_unanswered_arbiter(&fixture, "I_eligibility", &budget(1, 5));
    let original = decision(&fixture, &prepared).request;
    assert!(arbiter_eligible(&original, &prepared.assessment, DUE));
    for timeout in [IssueDecisionTimeout::Wait, IssueDecisionTimeout::Default] {
        let mut definition = original.definition().clone();
        definition.on_timeout = timeout;
        let legacy =
            IssueDecisionRequest::new(IssueDecisionId::new("legacy-wait").unwrap(), definition)
                .unwrap();
        assert!(!arbiter_eligible(
            &legacy,
            &prepared.assessment,
            DUE + 10_000
        ));
    }
    for (kind, gate) in [
        (
            IssueDecisionKind::SpecConfirmation,
            IssueAutomationGate::Spec,
        ),
        (
            IssueDecisionKind::RiskException,
            IssueAutomationGate::Delivery,
        ),
        (IssueDecisionKind::RiskException, IssueAutomationGate::Merge),
        (IssueDecisionKind::AgentQuestion, IssueAutomationGate::Plan),
        (
            IssueDecisionKind::FailureRecovery,
            IssueAutomationGate::Start,
        ),
        (IssueDecisionKind::IssueChanged, IssueAutomationGate::Start),
    ] {
        let mut definition = original.definition().clone();
        definition.kind = kind;
        definition.gate = gate;
        let request =
            IssueDecisionRequest::new(IssueDecisionId::new("human-only").unwrap(), definition)
                .unwrap();
        assert!(
            !arbiter_eligible(&request, &prepared.assessment, DUE),
            "{kind:?}/{gate:?}"
        );
    }
    let mut bound = original.definition().clone();
    bound.basis.work_order_id = Some("work-bound".into());
    let bound =
        IssueDecisionRequest::new(IssueDecisionId::new("work-bound").unwrap(), bound).unwrap();
    assert!(!arbiter_eligible(&bound, &prepared.assessment, DUE));
    let mut close = original.definition().clone();
    close.kind = IssueDecisionKind::CloseApproval;
    close.gate = IssueAutomationGate::Close;
    close.basis.work_order_id = Some("work-close".into());
    assert!(IssueDecisionRequest::new(IssueDecisionId::new("close").unwrap(), close).is_err());
}
