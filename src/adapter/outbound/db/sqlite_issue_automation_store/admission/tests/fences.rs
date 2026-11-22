use super::*;
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};
use crate::domain::issue_automation::coordinator::{
    IssueCoordinatorAction, IssueCoordinatorInput, next_action,
};
use crate::domain::issue_automation::source::{
    IssueSourceActor, IssueSourceActorKind, IssueSourceRead,
};

#[test]
fn policy_authority_is_explicit_and_generic_save_cannot_bypass_admission() {
    let fixture = Fixture::new("admission-authority");
    let budget = budget(1, 1);
    let prepared = prepare(
        &fixture,
        "I_policy",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    let mut missing = serde_json::to_value(&prepared.work).unwrap();
    missing.as_object_mut().unwrap().remove("startAuthority");
    let missing: IssueWorkCheckpoint = serde_json::from_value(missing).unwrap();
    assert_eq!(missing.start_authority, IssueWorkStartAuthority::Human);
    assert!(
        missing.validate().is_err(),
        "missing authority plus missing decision must fail closed"
    );
    let mut forged = prepared.work.clone();
    forged.decision_id = Some(
        crate::domain::issue_automation::decision::IssueDecisionId::new("invented-human").unwrap(),
    );
    assert!(forged.validate().is_err());
    assert!(
        fixture
            .adapter()
            .save_issue_work(
                fixture.cwd(),
                prepared.fence,
                &prepared.assessment,
                &prepared.work,
            )
            .is_err()
    );
    assert_eq!(count(&fixture, "issue_work_checkpoints"), 0);
    assert_eq!(count(&fixture, "issue_work_admissions"), 0);
    assert_eq!(count(&fixture, "issue_decision_requests"), 0);
    assert_eq!(
        next_action(IssueCoordinatorInput {
            authorization_epoch: 0,
            state: IssueAutomationState::Ready,
            flags: Default::default(),
            assessment: Some(&prepared.assessment),
            decision: None,
            work: None,
            frozen_source: None,
            now_unix_ms: 0,
        }),
        IssueCoordinatorAction::SubmitWork
    );
    saved(run(&fixture, &prepared, &budget, 0));
    assert_eq!(
        count(&fixture, "issue_decision_requests"),
        0,
        "policy starts never invent a human answer"
    );
    let human = prepare(&fixture, "I_human", IssueWorkStartAuthority::Human, &budget);
    let mut legacy = serde_json::to_value(&human.work).unwrap();
    legacy.as_object_mut().unwrap().remove("startAuthority");
    let legacy: IssueWorkCheckpoint = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.start_authority, IssueWorkStartAuthority::Human);
    legacy.validate().unwrap();
    assert!(
        fixture
            .adapter()
            .save_issue_work(fixture.cwd(), human.fence, &human.assessment, &human.work,)
            .is_err(),
        "human starts use the same budget transaction"
    );
}

#[test]
fn policy_gate_epoch_and_immutable_handoff_must_match_the_current_assessment() {
    for mismatch in ["policy", "gate", "epoch", "handoff"] {
        let fixture = Fixture::new("admission-stale-basis");
        let budget = budget(1, 1);
        let mut prepared = prepare(
            &fixture,
            "I_stale",
            IssueWorkStartAuthority::Policy,
            &budget,
        );
        match mismatch {
            "policy" => prepared.work.approved_basis.policy_version = "0".repeat(64),
            "gate" => prepared.work.approved_basis.gate_revision += 1,
            "epoch" => prepared.work.approved_basis.authorization_epoch += 1,
            "handoff" => prepared
                .work
                .order
                .handoff
                .body
                .push_str("\nunauthorized shared content"),
            _ => unreachable!(),
        }
        assert!(
            fixture
                .adapter()
                .admit_issue_work(
                    fixture.cwd(),
                    prepared.fence,
                    &prepared.assessment,
                    &prepared.work,
                    &budget,
                    0,
                )
                .is_err(),
            "{mismatch}"
        );
        assert_eq!(count(&fixture, "issue_work_admissions"), 0, "{mismatch}");
        assert_eq!(count(&fixture, "issue_work_checkpoints"), 0, "{mismatch}");
    }
}

#[test]
fn actor_observation_and_runtime_changes_fence_admission_and_hide_old_waits() {
    let fixture = Fixture::new("admission-actor-fence");
    let budget = budget(1, 1);
    let mut prepared = prepare(
        &fixture,
        "I_actor",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    let zero = IssueAutomationBudget {
        max_running: 0,
        max_started_per_day: 0,
        max_attempts: 0,
    };
    assert!(matches!(
        run(&fixture, &prepared, &zero, 0),
        IssueWorkAdmissionResult::BudgetExhausted { .. }
    ));
    assert!(
        subject(&fixture, &prepared.work.order.issue)
            .1
            .start_budget_wait
            .is_some()
    );
    prepared.observation.author = Some(IssueSourceActor {
        kind: IssueSourceActorKind::UserAccount,
        id: "U_untrusted".into(),
        login: "changed".into(),
    });
    let control = fixture
        .adapter()
        .get_collection(fixture.cwd())
        .unwrap()
        .unwrap();
    fixture
        .adapter()
        .apply_issue_resync(
            fixture.cwd(),
            control.fence(),
            &prepared.work.order.issue,
            &IssueSourceRead::Observed(Box::new(prepared.observation.clone())),
        )
        .unwrap();
    assert!(matches!(
        run(&fixture, &prepared, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    let (fresh_fence, item) = subject(&fixture, &prepared.work.order.issue);
    assert!(item.start_budget_wait.is_none());
    assert!(
        fixture
            .adapter()
            .admit_issue_work(
                fixture.cwd(),
                fresh_fence,
                &prepared.assessment,
                &prepared.work,
                &budget,
                0,
            )
            .is_err(),
        "a fresh row fence cannot authorize stale actor evidence"
    );

    let runtime = prepare(
        &fixture,
        "I_runtime",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    assert!(matches!(
        run(&fixture, &runtime, &zero, 0),
        IssueWorkAdmissionResult::BudgetExhausted { .. }
    ));
    let selection = fixture
        .adapter()
        .issue_autonomy_selection(fixture.cwd())
        .unwrap();
    fixture
        .adapter()
        .begin_issue_autonomy(
            fixture.cwd(),
            &IssueAutonomyIntent {
                command_id: "native-disable".into(),
                level: IssueAutonomyLevel::Off,
                expected_revision: selection.revision,
                policy_version: "a".repeat(64),
            },
        )
        .unwrap();
    assert!(matches!(
        run(&fixture, &runtime, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    assert!(
        subject(&fixture, &runtime.work.order.issue)
            .1
            .start_budget_wait
            .is_none()
    );
    assert_eq!(count(&fixture, "issue_work_admissions"), 0);
    assert_eq!(count(&fixture, "issue_work_checkpoints"), 0);
}

#[test]
fn receipt_failure_rolls_back_checkpoint_and_catalog_in_the_same_transaction() {
    let fixture = Fixture::new("admission-rollback");
    let budget = budget(1, 1);
    let prepared = prepare(
        &fixture,
        "I_rollback",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    fixture
        .adapter()
        .with_connection(fixture.cwd(), |connection, guard| {
            let catalog = super::super::super::triage::catalog_revision(connection, fixture.cwd())?;
            connection.execute_batch(
                "CREATE TEMP TRIGGER reject_admission BEFORE INSERT ON issue_work_admissions
             BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;",
            )?;
            let result = transaction(connection, guard, |tx| {
                admit_current(
                    tx,
                    fixture.cwd(),
                    prepared.fence,
                    &prepared.assessment,
                    &prepared.work,
                    &budget,
                    0,
                )
            });
            assert!(result.is_err());
            assert!(
                coordinator::read(connection, fixture.cwd(), &prepared.work.order.issue)?.is_none()
            );
            assert_eq!(
                super::super::super::triage::catalog_revision(connection, fixture.cwd())?,
                catalog
            );
            assert_eq!(
                connection.query_row("SELECT count(*) FROM issue_work_admissions", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                0
            );
            Ok(())
        })
        .unwrap();
    saved(run(&fixture, &prepared, &budget, 0));
    assert_eq!(count(&fixture, "issue_work_admissions"), 1);
}

#[test]
fn collection_epoch_and_workflow_flags_cannot_reuse_an_old_admission_fence() {
    use crate::domain::issue_automation::IssueAutomationFlag;
    use crate::domain::issue_automation::collection::IssueCollectionMode;
    for flag in [
        IssueAutomationFlag::Held,
        IssueAutomationFlag::Stale,
        IssueAutomationFlag::Unknown,
    ] {
        let fixture = Fixture::new("admission-workflow-fence");
        let budget = budget(1, 1);
        let prepared = prepare(
            &fixture,
            "I_blocked",
            IssueWorkStartAuthority::Policy,
            &budget,
        );
        let adapter = fixture.adapter();
        let mut tracked = adapter
            .get_issue(fixture.cwd(), &prepared.work.order.issue)
            .unwrap()
            .unwrap();
        tracked.set_flag(flag, true);
        super::super::super::tests::save_changed(&adapter, fixture.cwd(), &tracked);
        let (fence, _) = subject(&fixture, &prepared.work.order.issue);
        assert!(
            adapter
                .admit_issue_work(
                    fixture.cwd(),
                    fence,
                    &prepared.assessment,
                    &prepared.work,
                    &budget,
                    0,
                )
                .is_err()
        );
        assert_eq!(count(&fixture, "issue_work_admissions"), 0);
    }
    let fixture = Fixture::new("admission-collection-epoch");
    let budget = budget(1, 1);
    let prepared = prepare(
        &fixture,
        "I_paused",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    let zero = IssueAutomationBudget {
        max_running: 0,
        max_started_per_day: 0,
        max_attempts: 0,
    };
    assert!(matches!(
        run(&fixture, &prepared, &zero, 0),
        IssueWorkAdmissionResult::BudgetExhausted { .. }
    ));
    let adapter = fixture.adapter();
    adapter
        .set_collection_mode(
            fixture.cwd(),
            Some(prepared.fence.collection_epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    assert!(matches!(
        run(&fixture, &prepared, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    assert!(
        subject(&fixture, &prepared.work.order.issue)
            .1
            .start_budget_wait
            .is_none()
    );
    let epoch = adapter
        .get_collection(fixture.cwd())
        .unwrap()
        .unwrap()
        .epoch;
    adapter
        .set_collection_mode(fixture.cwd(), Some(epoch), IssueCollectionMode::Observe)
        .unwrap();
    assert!(matches!(
        run(&fixture, &prepared, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    assert!(
        subject(&fixture, &prepared.work.order.issue)
            .1
            .start_budget_wait
            .is_none()
    );
    assert_eq!(count(&fixture, "issue_work_admissions"), 0);
}

#[test]
fn a_zero_budget_expires_an_old_open_approval_before_waiting_without_a_new_question() {
    use crate::domain::issue_automation::decision::IssueDecisionRequest;
    use crate::domain::issue_automation::triage::IssueTriageReasonCode;
    let fixture = Fixture::new("admission-zero-stale-question");
    let prepared = prepare(
        &fixture,
        "I_question",
        IssueWorkStartAuthority::Human,
        &budget(1, 1),
    );
    let answered = fixture
        .adapter()
        .get_issue_decision(fixture.cwd(), prepared.work.decision_id.as_ref().unwrap())
        .unwrap()
        .unwrap()
        .request;
    let open =
        IssueDecisionRequest::new(answered.id().clone(), answered.definition().clone()).unwrap();
    let mut lowered = prepared.assessment.to_record();
    lowered.basis.automation_policy_version = "b".repeat(64);
    lowered.reasons.push(IssueTriageReasonCode::BudgetExhausted);
    lowered.observation.disposition =
        crate::domain::issue_automation::triage::IssueObservationDisposition::HumanApproval;
    lowered.reasons.sort();
    lowered.reasons.dedup();
    for gate in &mut lowered.gates {
        gate.gate_revision += 1;
    }
    let lowered = IssueTriageAssessment::hydrate(lowered).unwrap();
    let action = |decision| {
        next_action(IssueCoordinatorInput {
            authorization_epoch: 0,
            state: IssueAutomationState::AwaitingDecision,
            flags: Default::default(),
            assessment: Some(&lowered),
            decision,
            work: None,
            frozen_source: None,
            now_unix_ms: 0,
        })
    };
    assert_eq!(action(Some(&open)), IssueCoordinatorAction::ExpireDecision);
    assert_eq!(action(None), IssueCoordinatorAction::None);
}
