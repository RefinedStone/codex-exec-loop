use super::*;
use crate::domain::issue_automation::arbiter::arbiter_eligible;
use crate::domain::issue_automation::decision::{
    IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionId, IssueDecisionKind,
    IssueDecisionOption, IssueDecisionRequest, IssueDecisionSharedText, IssueDecisionSubject,
    IssueDecisionTimeout,
};

fn request(assessment: &IssueTriageAssessment, gate: IssueAutomationGate) -> IssueDecisionRequest {
    let shared = |text: &str| IssueDecisionSharedText::new_shared(text).unwrap();
    IssueDecisionRequest::new(
        IssueDecisionId::new("arbiter-eligibility-matrix").unwrap(),
        IssueDecisionDefinition {
            subject: IssueDecisionSubject::Issue(issue()),
            gate,
            kind: if gate == IssueAutomationGate::Start {
                IssueDecisionKind::StartApproval
            } else {
                IssueDecisionKind::PlanApproval
            },
            question: shared("현재 단계를 진행할까요?"),
            options: vec![IssueDecisionOption {
                id: "proceed".into(),
                label: shared("진행"),
                effect: shared("현재 단계를 진행한다."),
            }],
            recommended: None,
            basis: IssueDecisionBasis::from_assessment(assessment, gate, None).unwrap(),
            on_timeout: IssueDecisionTimeout::Arbiter,
            due_at: Some("1970-01-01T00:00:01Z".into()),
            asked_by: "triage".into(),
            evidence: vec![],
            delivery_review: None,
        },
    )
    .unwrap()
}

#[test]
fn arbiter_only_accepts_safe_start_and_plan_gates_under_the_current_autonomy_mode() {
    use IssueAutomationAutonomy::{Auto, Gated, Observe, Off, Propose};
    for (mode, label, high, start, plan) in [
        (Auto, None, false, true, true),
        (Gated, None, false, true, true),
        (Propose, None, false, false, true),
        (Gated, Some("akra:manual"), false, false, false),
        (Gated, Some("danger"), true, false, false),
        (Auto, Some("akra:manual"), false, false, false),
        (Auto, Some("danger"), true, false, false),
        (Observe, None, false, false, false),
        (Off, None, false, false, false),
    ] {
        let mut policy = policy();
        policy.binding.github.mode = mode;
        policy.authority.autonomy_cap = Auto;
        policy.gates.auto.normal.start = IssueAutomationGateAction::TimeoutArbiter;
        policy.gates.auto.normal.plan = IssueAutomationGateAction::TimeoutArbiter;
        policy.gates.gated.normal.start = IssueAutomationGateAction::TimeoutArbiter;
        policy.gates.gated.normal.plan = IssueAutomationGateAction::TimeoutArbiter;
        policy.gates.propose.normal.plan = IssueAutomationGateAction::TimeoutArbiter;
        if high {
            policy.classify.rule.push(IssueAutomationRiskRule {
                class: IssueAutomationRiskClass::High,
                labels_any: vec!["danger".into()],
                paths_any: vec![],
            });
        }
        let mut source = source();
        if let Some(label) = label {
            observed(&mut source).labels.items.push(IssueSourceLabel {
                id: "L_control".into(),
                name: label.into(),
            });
        }
        let assessment = run(Some(&source), &policy, None).unwrap();
        for (gate, expected) in [
            (IssueAutomationGate::Start, start),
            (IssueAutomationGate::Plan, plan),
        ] {
            let request = request(&assessment, gate);
            assert!(!arbiter_eligible(&request, &assessment, 999));
            assert_eq!(
                arbiter_eligible(&request, &assessment, 1000),
                expected,
                "{mode:?}/{label:?}/{gate:?}"
            );
        }
    }
}
