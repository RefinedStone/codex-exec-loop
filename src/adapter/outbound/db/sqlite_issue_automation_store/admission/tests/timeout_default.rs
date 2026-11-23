use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueDecisionCommit, IssueDecisionFence, IssueDecisionSaveResult,
};
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};
use crate::domain::issue_automation::coordinator::{
    IssueCoordinatorAction, IssueCoordinatorInput, next_action, policy_start, timeout_default_start,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionAgentAnswer, IssueDecisionAnswerBy, IssueDecisionId, IssueDecisionOption,
    IssueDecisionRequest, IssueDecisionSharedText, IssueDecisionState, StoredIssueDecision,
};
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::source::{
    IssueSourceActor, IssueSourceActorKind, IssueSourceRead,
};

fn decision(fixture: &Fixture, prepared: &Prepared) -> StoredIssueDecision {
    fixture
        .adapter()
        .get_issue_decision(fixture.cwd(), prepared.work.decision_id.as_ref().unwrap())
        .unwrap()
        .unwrap()
}

fn timeout(fixture: &Fixture, prepared: &Prepared, at: u64) -> IssueDecisionCommit {
    let stored = decision(fixture, prepared);
    match fixture
        .adapter()
        .timeout_issue_decision(
            fixture.cwd(),
            IssueDecisionFence {
                triage: prepared.fence,
                decision_revision: stored.revision,
            },
            &prepared.assessment,
            stored.request.id(),
            at,
        )
        .unwrap()
    {
        IssueDecisionSaveResult::Saved(commit) => *commit,
        IssueDecisionSaveResult::Conflict => panic!("expected current timeout decision"),
    }
}

fn action(
    prepared: &Prepared,
    decision: Option<&IssueDecisionRequest>,
    work: Option<&IssueWorkCheckpoint>,
) -> IssueCoordinatorAction {
    next_action(IssueCoordinatorInput {
        authorization_epoch: 0,
        state: IssueAutomationState::Ready,
        flags: Default::default(),
        assessment: Some(&prepared.assessment),
        decision,
        work,
        frozen_source: None,
        now_unix_ms: u64::MAX,
    })
}

#[test]
fn timeout_default_requires_a_committed_answer_and_replays_one_admission_after_restart() {
    let fixture = Fixture::new("admission-timeout-default");
    let budget = budget(1, 1);
    let prepared = prepare_unanswered_default(&fixture, "I_default", &budget);
    let open = decision(&fixture, &prepared);
    assert!(!policy_start(&prepared.assessment));
    assert_eq!(
        action(&prepared, None, None),
        IssueCoordinatorAction::OpenDecision
    );
    assert_eq!(
        action(&prepared, Some(&open.request), None),
        IssueCoordinatorAction::None
    );
    assert!(
        fixture
            .adapter()
            .admit_issue_work(
                fixture.cwd(),
                prepared.fence,
                &prepared.assessment,
                &prepared.work,
                &budget,
                1_000,
            )
            .is_err()
    );
    assert_eq!(count(&fixture, "issue_work_admissions"), 0);

    let before = timeout(&fixture, &prepared, 999);
    assert!(!before.changed);
    assert_eq!(before.record.request.state(), IssueDecisionState::Open);
    let due = timeout(&fixture, &prepared, 1_000);
    assert!(due.changed);
    let answer = due.record.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Default);
    assert!(answer.confirmation.is_none());
    assert!(timeout_default_start(
        &prepared.assessment,
        &due.record.request
    ));
    assert_eq!(
        action(&prepared, Some(&due.record.request), None),
        IssueCoordinatorAction::SubmitWork
    );

    let registered = saved(run(&fixture, &prepared, &budget, 1_000));
    assert_eq!(
        registered.start_authority,
        IssueWorkStartAuthority::TimeoutDefault
    );
    assert_eq!(registered.decision_id, prepared.work.decision_id);
    assert_eq!(
        action(&prepared, Some(&due.record.request), Some(&registered)),
        IssueCoordinatorAction::SubmitWork
    );
    assert_eq!(
        action(&prepared, None, Some(&registered)),
        IssueCoordinatorAction::CancelWork
    );
    let after_restart = timeout(&fixture, &prepared, 86_400_000);
    assert!(!after_restart.changed);
    assert_eq!(after_restart.record, due.record);
    let replay = run(&fixture, &prepared, &budget, 86_400_000);
    assert!(matches!(
        &replay,
        IssueWorkAdmissionResult::Saved { changed: false, .. }
    ));
    assert_eq!(saved(replay), registered);
    assert_eq!(count(&fixture, "issue_work_admissions"), 1);
    assert_eq!(count(&fixture, "issue_work_checkpoints"), 1);
}

#[test]
fn default_authority_cannot_borrow_an_agent_or_human_answer() {
    for human in [false, true] {
        let fixture = Fixture::new("admission-default-origin");
        let budget = budget(1, 1);
        let prepared = prepare_unanswered_default(&fixture, "I_origin", &budget);
        let open = decision(&fixture, &prepared);
        let input = IssueDecisionAgentAnswer {
            option: "start".into(),
            note: None,
            caller: None,
            channel: "native".into(),
            at: "1970-01-01T00:00:00.500Z".into(),
        };
        let answered = if human {
            open.request.answer_for_gate(
                &prepared.work.approved_basis,
                IssueAutomationGateAction::TimeoutDefault,
                input,
                Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
            )
        } else {
            // Even a caller using the ungated pure transition must pass the current DB gate.
            open.request
                .answer_agent(&prepared.work.approved_basis, input)
        }
        .unwrap()
        .request;
        let saved_answer = fixture.adapter().save_issue_decision(
            fixture.cwd(),
            IssueDecisionFence {
                triage: prepared.fence,
                decision_revision: open.revision,
            },
            &prepared.assessment,
            &answered,
        );
        if human {
            saved_answer.unwrap();
        } else {
            assert!(saved_answer.is_err());
            assert_eq!(decision(&fixture, &prepared), open);
        }
        assert!(!timeout_default_start(&prepared.assessment, &answered));
        assert!(
            fixture
                .adapter()
                .admit_issue_work(
                    fixture.cwd(),
                    prepared.fence,
                    &prepared.assessment,
                    &prepared.work,
                    &budget,
                    1_000,
                )
                .is_err()
        );
        assert_eq!(count(&fixture, "issue_work_admissions"), 0);
        if human {
            let mut authorized = prepared.clone();
            authorized.work.start_authority = IssueWorkStartAuthority::Human;
            saved(run(&fixture, &authorized, &budget, 1_000));
        }
    }
}

#[test]
fn a_default_recommendation_to_decline_does_not_authorize_start() {
    let fixture = Fixture::new("admission-default-decline");
    let budget = budget(1, 1);
    let mut prepared = prepare_unanswered_default(&fixture, "I_decline", &budget);
    let mut definition = decision(&fixture, &prepared).request.definition().clone();
    definition.options.push(IssueDecisionOption {
        id: "reject".into(),
        label: IssueDecisionSharedText::new_shared("거절").unwrap(),
        effect: IssueDecisionSharedText::new_shared("작업을 시작하지 않는다.").unwrap(),
    });
    definition.recommended = Some("reject".into());
    let request =
        IssueDecisionRequest::new(IssueDecisionId::new("default-decline").unwrap(), definition)
            .unwrap();
    fixture
        .adapter()
        .ensure_issue_decision(
            fixture.cwd(),
            prepared.fence,
            &prepared.assessment,
            &request,
        )
        .unwrap();
    prepared.work.decision_id = Some(request.id().clone());
    let resolved = timeout(&fixture, &prepared, 1_000);
    assert_eq!(resolved.record.request.answer().unwrap().option, "reject");
    assert!(!timeout_default_start(
        &prepared.assessment,
        &resolved.record.request
    ));
    assert_eq!(
        action(&prepared, Some(&resolved.record.request), None),
        IssueCoordinatorAction::None
    );
    assert!(
        fixture
            .adapter()
            .admit_issue_work(
                fixture.cwd(),
                prepared.fence,
                &prepared.assessment,
                &prepared.work,
                &budget,
                1_000,
            )
            .is_err()
    );
    assert_eq!(count(&fixture, "issue_work_checkpoints"), 0);
}

#[test]
fn timeout_default_authority_requires_an_exact_decision_and_basis() {
    for mismatch in [
        "missing_id",
        "unknown_id",
        "policy",
        "gate",
        "epoch",
        "legacy",
    ] {
        let fixture = Fixture::new("admission-default-identity");
        let budget = budget(1, 1);
        let mut prepared = prepare(
            &fixture,
            "I_identity",
            IssueWorkStartAuthority::TimeoutDefault,
            &budget,
        );
        match mismatch {
            "missing_id" => prepared.work.decision_id = None,
            "unknown_id" => {
                prepared.work.decision_id = Some(IssueDecisionId::new("unknown-default").unwrap())
            }
            "policy" => prepared.work.approved_basis.policy_version = "0".repeat(64),
            "gate" => prepared.work.approved_basis.gate_revision += 1,
            "epoch" => prepared.work.approved_basis.authorization_epoch += 1,
            "legacy" => {
                let mut legacy = serde_json::to_value(&prepared.work).unwrap();
                legacy.as_object_mut().unwrap().remove("startAuthority");
                prepared.work = serde_json::from_value(legacy).unwrap();
                assert_eq!(
                    prepared.work.start_authority,
                    IssueWorkStartAuthority::Human
                );
            }
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
                    1_000,
                )
                .is_err(),
            "{mismatch}"
        );
        assert_eq!(count(&fixture, "issue_work_admissions"), 0, "{mismatch}");
    }
}

#[test]
fn human_policy_and_timeout_default_starts_share_the_same_last_budget_slot() {
    for (budget, limit) in [
        (budget(1, 10), IssueStartBudgetLimit::Running),
        (budget(10, 1), IssueStartBudgetLimit::StartedPerDay),
    ] {
        let fixture = Fixture::new("admission-default-budget-race");
        let default = prepare(
            &fixture,
            "I_default",
            IssueWorkStartAuthority::TimeoutDefault,
            &budget,
        );
        let policy = prepare(
            &fixture,
            "I_policy",
            IssueWorkStartAuthority::Policy,
            &budget,
        );
        let human = prepare(&fixture, "I_human", IssueWorkStartAuthority::Human, &budget);
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = [default, policy, human]
            .into_iter()
            .map(|prepared| {
                let adapter = fixture.adapter();
                let workspace = fixture.cwd().to_owned();
                let barrier = barrier.clone();
                let budget = budget.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    adapter
                        .admit_issue_work(
                            &workspace,
                            prepared.fence,
                            &prepared.assessment,
                            &prepared.work,
                            &budget,
                            1_000,
                        )
                        .unwrap()
                })
            })
            .collect();
        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, IssueWorkAdmissionResult::Saved { .. }))
                .count(),
            1
        );
        assert_eq!(outcomes.iter().filter(|outcome| matches!(outcome,
            IssueWorkAdmissionResult::BudgetExhausted { wait, .. } if wait.exhausted == vec![limit]
        )).count(), 2);
        assert_eq!(count(&fixture, "issue_work_admissions"), 1);
        assert_eq!(count(&fixture, "issue_work_checkpoints"), 1);
        assert_eq!(count(&fixture, "issue_start_budget_waits"), 2);
    }
}

#[test]
fn completed_default_cannot_bypass_changed_actor_or_pending_autonomy() {
    for change in ["actor", "pending_selection"] {
        let fixture = Fixture::new("admission-default-current-authority");
        let budget = budget(1, 1);
        let mut prepared = prepare(
            &fixture,
            "I_changed",
            IssueWorkStartAuthority::TimeoutDefault,
            &budget,
        );
        let original = decision(&fixture, &prepared);
        if change == "actor" {
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
        } else {
            let selection = fixture
                .adapter()
                .issue_autonomy_selection(fixture.cwd())
                .unwrap();
            fixture
                .adapter()
                .begin_issue_autonomy(
                    fixture.cwd(),
                    &IssueAutonomyIntent {
                        command_id: "default-disable-pending".into(),
                        level: IssueAutonomyLevel::Off,
                        expected_revision: selection.revision,
                        policy_version: "a".repeat(64),
                    },
                )
                .unwrap();
        }
        assert!(
            matches!(
                run(&fixture, &prepared, &budget, 1_000),
                IssueWorkAdmissionResult::Conflict
            ),
            "{change}"
        );
        assert_eq!(decision(&fixture, &prepared), original);
        assert_eq!(count(&fixture, "issue_work_admissions"), 0);
        assert_eq!(count(&fixture, "issue_work_checkpoints"), 0);
    }
}
