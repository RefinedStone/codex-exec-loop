use super::autonomy::{autonomy, confirm};
use super::*;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::autonomy::{IssueAutonomyLevel, IssueAutonomyProjection};
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::triage::{IssueTriageEligibility, IssueTriageReasonCode};

pub(super) fn gated_policy() -> String {
    AUTO_POLICY
        .replace("mode = \"propose\"", "mode = \"gated\"")
        .replace("autonomy_cap = \"propose\"", "autonomy_cap = \"gated\"")
        .replace("[gates.auto.normal]", "[gates.gated.normal]")
        .replace("delivery = \"auto\"", "delivery = \"human\"")
        .replace("merge = \"auto\"", "merge = \"human\"")
}

pub(super) fn enable_gated(h: &Harness) {
    let service = autonomy(h);
    let proof = confirm(&service, "gated-consent", IssueAutonomyLevel::Gated);
    service
        .set(
            "gated-consent".into(),
            IssueAutonomyLevel::Gated,
            Some(&proof),
        )
        .unwrap();
}

fn tick_at(h: &Harness, now_unix_ms: u64) -> Result<()> {
    run(&h.triage);
    h.coordinator().tick(
        &IssueCollectionCancellation::new(),
        now_unix_ms,
        false,
        &|_| {},
    )
}

#[test]
fn gated_auto_records_policy_authority_without_a_human_decision_or_parallel_enable() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = gated_policy();
    enable_gated(&h);
    let execution = h.execution.clone();
    h.execution.state.lock().unwrap().after_grant = Some(Box::new(move || {
        assert_eq!(
            execution
                .state
                .lock()
                .unwrap()
                .execution_limits
                .last()
                .map(|(_, limit)| *limit),
            Some(3),
            "the attempt limit must be installed before the first permit",
        );
    }));
    h.tick(false).unwrap();

    let work = h.work().unwrap();
    assert_eq!(work.start_authority, IssueWorkStartAuthority::Policy);
    assert_eq!(work.decision_id, None);
    assert_eq!(work.current_authorization_decision_id(), None);
    assert!(work.authorization.is_none());
    assert_eq!(work.task_ids, ["task-1"]);
    assert_eq!(
        work.blocked_reason,
        Some(IssueWorkBlockReason::ExecutionDisabled)
    );
    assert!(!work.delivery_granted);
    let page = page(&h.triage);
    assert!(page.items[0].open_decisions.is_empty());
    let assessment = &page.items[0].triage.as_ref().unwrap().assessment;
    assert!(policy_start(assessment));
    for gate in [IssueAutomationGate::Delivery, IssueAutomationGate::Merge] {
        assert_eq!(
            assessment
                .gates()
                .iter()
                .find(|entry| entry.gate == gate)
                .unwrap()
                .action,
            IssueAutomationGateAction::Human,
        );
    }
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    h.tick(false).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
}

#[test]
fn a_gated_file_requires_explicit_native_runtime_consent_before_automatic_start() {
    let h = Harness::new();
    let text = gated_policy();
    *h.policy.text.lock().unwrap() = text.clone();
    let service = autonomy(&h);
    let IssueAutonomyProjection::Ready(current) = service.state() else {
        panic!("current policy is available");
    };
    assert_eq!(current.requested_level, None);
    assert_eq!(current.effective_level, IssueAutomationAutonomy::Propose);
    assert!(
        current
            .choices
            .iter()
            .find(|choice| choice.level == IssueAutonomyLevel::Gated)
            .unwrap()
            .requires_confirmation
    );
    assert!(
        service
            .set("unconfirmed".into(), IssueAutonomyLevel::Gated, None)
            .is_err()
    );
    h.tick(false).unwrap();
    assert!(h.work().is_none());
    assert_eq!(h.decision().request.state(), IssueDecisionState::Open);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    assert_eq!(*h.policy.text.lock().unwrap(), text);

    let preview = service
        .prepare_confirmation("gated-consent", IssueAutonomyLevel::Gated)
        .unwrap();
    assert!(preview.display.detail.contains("매번 묻지 않고 등록"));
    assert!(preview.display.detail.contains("사람 리뷰"));
    enable_gated(&h);
    // The old human request is superseded before the next tick admits Policy work.
    h.tick(false).unwrap();
    h.tick(false).unwrap();
    let work = h.work().unwrap();
    assert_eq!(work.start_authority, IssueWorkStartAuthority::Policy);
    assert_eq!(work.decision_id, None);
}

#[test]
fn a_selected_gated_level_cannot_raise_the_current_user_policy_cap() {
    let h = Harness::new();
    assert!(
        autonomy(&h)
            .prepare_confirmation("above-cap", IssueAutonomyLevel::Gated)
            .is_err()
    );
    *h.policy.text.lock().unwrap() = gated_policy();
    enable_gated(&h);
    *h.policy.text.lock().unwrap() =
        gated_policy().replace("autonomy_cap = \"gated\"", "autonomy_cap = \"propose\"");
    let IssueAutonomyProjection::Ready(current) = autonomy(&h).state() else {
        panic!("the constrained policy is readable");
    };
    assert_eq!(current.requested_level, Some(IssueAutonomyLevel::Gated));
    assert_eq!(current.effective_level, IssueAutomationAutonomy::Propose);
    h.tick(false).unwrap();
    assert!(h.work().is_none());
    assert_eq!(h.decision().request.state(), IssueDecisionState::Open);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn gated_human_start_still_requires_a_human_decision() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() =
        gated_policy().replace("start = \"auto\"", "start = \"human\"");
    enable_gated(&h);
    h.tick(false).unwrap();
    assert!(h.work().is_none());
    let decision_id = h.decision().request.id().clone();
    h.answer("start");
    h.tick(false).unwrap();
    let work = h.work().unwrap();
    assert_eq!(work.start_authority, IssueWorkStartAuthority::Human);
    assert_eq!(work.decision_id, Some(decision_id));
    assert_eq!(
        work.blocked_reason,
        Some(IssueWorkBlockReason::ExecutionDisabled)
    );
}

#[test]
fn gated_does_not_bypass_actor_trust_or_required_human_rules() {
    for case in ["untrusted", "high", "protected", "required"] {
        let h = Harness::with_observation(|observation| match case {
            "untrusted" => observation.author.as_mut().unwrap().id = "U_untrusted".into(),
            "high" => {
                observation.labels = complete(vec![IssueSourceLabel {
                    id: "L_high".into(),
                    name: "high-risk".into(),
                }]);
            }
            "protected" => {
                if let IssueSourceContent::Shared(snapshot) = &mut observation.content {
                    snapshot
                        .shared_copy
                        .body
                        .push_str("\n변경 경로: `.github/workflows/release.yml`\n");
                    snapshot.basis.field_hash = IssueFieldHash::of_fields(
                        &snapshot.shared_copy.title,
                        &snapshot.shared_copy.body,
                    );
                }
            }
            _ => {
                observation.labels = complete(vec![IssueSourceLabel {
                    id: "L_review".into(),
                    name: "review-first".into(),
                }]);
            }
        });
        let mut policy = gated_policy();
        let reason = match case {
            "untrusted" => IssueTriageReasonCode::UntrustedActor,
            "high" => {
                policy.push_str(
                    "\n[[classify.rule]]\nclass = \"high\"\nlabels_any = [\"high-risk\"]\n",
                );
                policy.push_str("\n[gates.gated.high]\nstart = \"auto\"\n");
                IssueTriageReasonCode::HighRisk
            }
            "protected" => IssueTriageReasonCode::ProtectedPath,
            _ => {
                policy.push_str("\n[[human_required.additional]]\nlabels_any = [\"review-first\"]\ngates = [\"start\"]\n");
                IssueTriageReasonCode::PolicyHumanRequired
            }
        };
        *h.policy.text.lock().unwrap() = policy;
        enable_gated(&h);
        h.tick(false).unwrap();
        assert!(h.work().is_none(), "{case}");
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0, "{case}");
        let current = page(&h.triage);
        let assessment = &current.items[0].triage.as_ref().unwrap().assessment;
        assert!(assessment.reasons().contains(&reason), "{case}");
        assert!(!policy_start(assessment), "{case}");
        if case == "untrusted" {
            assert_eq!(assessment.eligibility(), IssueTriageEligibility::Ineligible);
        } else {
            assert_eq!(
                h.decision().request.state(),
                IssueDecisionState::Open,
                "{case}"
            );
        }
    }
}

#[test]
fn admission_wait_is_ready_and_does_not_change_another_approved_work_basis() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = format!("{}\n[budget]\nmax_running = 1\n", gated_policy());
    enable_gated(&h);
    h.tick(false).unwrap();
    let before = h.work().unwrap();
    let next = h.fixture.observation("I_002");
    h.fixture.seed(vec![next.clone()], &disclosure_version());
    *h.source.read.lock().unwrap() = IssueSourceRead::Observed(Box::new(next));
    h.tick(false).unwrap();

    let page = page(&h.triage);
    let waiting = page
        .items
        .iter()
        .find(|item| item.issue.issue_id == "I_002")
        .unwrap();
    assert_eq!(
        waiting.tracked.as_ref().unwrap().state,
        IssueAutomationState::Ready
    );
    assert_eq!(
        waiting.start_budget_wait.as_ref().unwrap().exhausted,
        [IssueStartBudgetLimit::Running],
    );
    assert!(waiting.work.is_none());
    assert!(waiting.open_decisions.is_empty());
    let triage = waiting.triage.as_ref().unwrap();
    assert!(!triage.stale);
    assert!(
        !triage
            .assessment
            .reasons()
            .contains(&IssueTriageReasonCode::BudgetExhausted)
    );
    assert_eq!(page.report.as_ref().unwrap().deferred, 2);
    let after = h.work().unwrap();
    assert_eq!(
        after.current_authorization_basis(),
        before.current_authorization_basis()
    );
    assert_eq!(after.permit_revision, before.permit_revision);
    assert_ne!(
        after.blocked_reason,
        Some(IssueWorkBlockReason::ApprovalChanged)
    );
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert!(!h.execution.state.lock().unwrap().revoked);
}

#[test]
fn gated_registration_and_grant_reassembly_reuses_the_original_admission() {
    for failure in ["before_register", "after_register", "after_grant"] {
        let h = Harness::new();
        *h.policy.text.lock().unwrap() = format!(
            "{}\n[budget]\nmax_running = 1\nmax_started_per_day = 1\n",
            gated_policy()
        );
        enable_gated(&h);
        {
            let mut state = h.execution.state.lock().unwrap();
            state.fail_before_register = failure == "before_register";
            state.fail_after_register = failure == "after_register";
            state.fail_after_grant = failure == "after_grant";
        }
        assert!(tick_at(&h, 123).is_err(), "{failure}");
        let original = h.work().unwrap();
        tick_at(&h, 86_400_123).unwrap();
        tick_at(&h, 86_400_124).unwrap();
        let recovered = h.work().unwrap();
        assert_eq!(recovered.order, original.order);
        assert_eq!(recovered.approved_basis, original.approved_basis);
        assert_eq!(recovered.start_authority, IssueWorkStartAuthority::Policy);
        assert_eq!(recovered.decision_id, None);
        assert_eq!(recovered.task_ids, ["task-1"]);
        assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
        assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
        assert!(page(&h.triage).items[0].start_budget_wait.is_none());
    }
}

#[test]
fn explicit_gated_consent_renews_authority_without_rewriting_the_original_human_answer() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() =
        gated_policy().replace("mode = \"gated\"", "mode = \"propose\"");
    h.tick(false).unwrap();
    h.answer("start");
    h.tick(false).unwrap();
    let original = h.work().unwrap();
    assert_eq!(original.start_authority, IssueWorkStartAuthority::Human);
    enable_gated(&h);
    h.tick(false).unwrap();
    let renewed = h.work().unwrap();
    assert_eq!(renewed.start_authority, IssueWorkStartAuthority::Human);
    assert_eq!(renewed.decision_id, original.decision_id);
    assert_eq!(renewed.approved_basis, original.approved_basis);
    assert_eq!(renewed.order, original.order);
    let authority = renewed.authorization.unwrap();
    assert_eq!(authority.start_authority, IssueWorkStartAuthority::Policy);
    assert_eq!(authority.decision_id, None);
    assert!(authority.basis.authorization_epoch > original.approved_basis.authorization_epoch);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn zero_start_budget_stays_ready_without_creating_a_question_or_work() {
    for limit in ["max_running", "max_started_per_day", "max_attempts"] {
        let h = Harness::new();
        *h.policy.text.lock().unwrap() = format!("{}\n[budget]\n{limit} = 0\n", gated_policy());
        enable_gated(&h);
        h.tick(false).unwrap();
        let page = page(&h.triage);
        let item = &page.items[0];
        assert_eq!(
            item.tracked.as_ref().unwrap().state,
            IssueAutomationState::Ready
        );
        assert!(item.open_decisions.is_empty());
        assert!(item.work.is_none());
        assert!(
            item.triage
                .as_ref()
                .unwrap()
                .assessment
                .reasons()
                .contains(&IssueTriageReasonCode::BudgetExhausted)
        );
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    }
}
