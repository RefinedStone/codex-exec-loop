use super::*;
use crate::domain::issue_automation::arbiter::{IssueArbiterProposal, IssueArbiterSettings};
use crate::domain::issue_automation::coordinator::{
    policy_start, ready_for_start, timeout_arbiter_start, timeout_default_start,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionId, IssueDecisionKind,
    IssueDecisionOption, IssueDecisionRequest, IssueDecisionSharedText, IssueDecisionSubject,
    IssueDecisionTimeout,
};

fn action(
    assessment: &IssueTriageAssessment,
    gate: IssueAutomationGate,
) -> IssueAutomationGateAction {
    assessment
        .gates()
        .iter()
        .find(|row| row.gate == gate)
        .unwrap()
        .action
}

#[test]
fn auto_requires_current_private_or_internal_visibility_and_verified_repository_coordinates() {
    for visibility in [
        IssueRepositoryVisibility::Private,
        IssueRepositoryVisibility::Internal,
    ] {
        let mut current = source();
        observed(&mut current).repository_visibility = visibility;
        let assessment = run(Some(&current), &policy(), None).unwrap();
        assert_eq!(assessment.effective_mode(), IssueAutomationAutonomy::Auto);
        assert!(
            !assessment
                .reasons()
                .contains(&IssueTriageReasonCode::AutoRepositoryRestricted)
        );
        assert!(policy_start(&assessment));
        assert_eq!(
            action(&assessment, IssueAutomationGate::Delivery),
            IssueAutomationGateAction::Auto
        );
        assert_eq!(
            action(&assessment, IssueAutomationGate::Merge),
            IssueAutomationGateAction::Auto
        );
        assert_eq!(
            action(&assessment, IssueAutomationGate::Spec),
            IssueAutomationGateAction::Human
        );
        assert_eq!(
            action(&assessment, IssueAutomationGate::Close),
            IssueAutomationGateAction::Human
        );
    }
    for (visibility, coordinate) in [
        (
            IssueRepositoryVisibility::Public,
            Some("refinedstone/codex-exec-loop"),
        ),
        (
            IssueRepositoryVisibility::Unknown,
            Some("refinedstone/codex-exec-loop"),
        ),
        (IssueRepositoryVisibility::Private, None),
        (IssueRepositoryVisibility::Internal, None),
        (
            IssueRepositoryVisibility::Private,
            Some("RefinedStone/codex-exec-loop"),
        ),
        (
            IssueRepositoryVisibility::Private,
            Some("invalid coordinate"),
        ),
    ] {
        let mut current = source();
        let observation = observed(&mut current);
        observation.repository_visibility = visibility;
        observation.repository_name_with_owner = coordinate.map(str::to_owned);
        let assessment = run(Some(&current), &policy(), None).unwrap();
        assert_eq!(
            assessment.effective_mode(),
            IssueAutomationAutonomy::Propose,
            "{visibility:?}/{coordinate:?}"
        );
        assert_eq!(
            assessment.observation().assumed_mode,
            IssueAutomationAutonomy::Propose
        );
        assert!(
            assessment
                .reasons()
                .contains(&IssueTriageReasonCode::AutoRepositoryRestricted)
        );
        assert!(!policy_start(&assessment));
        for gate in [
            IssueAutomationGate::Start,
            IssueAutomationGate::Delivery,
            IssueAutomationGate::Merge,
        ] {
            assert_eq!(action(&assessment, gate), IssueAutomationGateAction::Human);
        }
    }
}

#[test]
fn missing_remote_metadata_caps_auto_without_changing_existing_non_auto_modes() {
    let unknown = IssueSourceRead::Unknown {
        issue: issue(),
        reason: IssueSourceFailure::ProviderUnavailable,
    };
    for current in [None, Some(&unknown)] {
        let result = run(current, &policy(), None).unwrap();
        assert_eq!(result.effective_mode(), IssueAutomationAutonomy::Propose);
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::AutoRepositoryRestricted)
        );
        assert!(result.gates().iter().all(|gate| !gate.enabled));
    }
    let mut current = source();
    observed(&mut current).repository_visibility = IssueRepositoryVisibility::Public;
    for mode in [
        IssueAutomationAutonomy::Off,
        IssueAutomationAutonomy::Observe,
        IssueAutomationAutonomy::Propose,
        IssueAutomationAutonomy::Gated,
    ] {
        let mut policy = policy();
        policy.binding.github.mode = mode;
        let result = run(Some(&current), &policy, None).unwrap();
        assert_eq!(result.effective_mode(), mode);
        assert_eq!(
            result.observation().assumed_mode,
            IssueAutomationAutonomy::Propose.max(mode)
        );
        assert!(result.observation().assumed_mode >= result.effective_mode());
    }
}

#[test]
fn repository_visibility_and_rename_change_gate_basis_without_changing_the_body_hash() {
    let current = source();
    let policy = policy();
    let first = run(Some(&current), &policy, None).unwrap();
    let old_basis =
        IssueDecisionBasis::from_assessment(&first, IssueAutomationGate::Merge, None).unwrap();
    for mutation in 0..4 {
        let mut changed = current.clone();
        let observation = observed(&mut changed);
        match mutation {
            0 => observation.repository_visibility = IssueRepositoryVisibility::Public,
            1 => observation.repository_visibility = IssueRepositoryVisibility::Internal,
            2 => observation.repository_name_with_owner = Some("refinedstone/renamed".into()),
            3 => observation.repository_name_with_owner = None,
            _ => unreachable!(),
        }
        let next = run(Some(&changed), &policy, Some(&first)).unwrap();
        assert_eq!(next.basis().field_hash, first.basis().field_hash);
        assert_ne!(next.input_fingerprint(), first.input_fingerprint());
        assert!(next.gates().iter().all(|gate| gate.gate_revision == 2));
        assert!(
            first
                .validate_current(&issue(), Some(&changed), false)
                .is_err()
        );
        assert_ne!(
            IssueDecisionBasis::from_assessment(&next, IssueAutomationGate::Merge, None).unwrap(),
            old_basis
        );
    }
}

#[test]
fn legacy_unknown_repository_metadata_retains_the_original_canonical_and_v1_body_hashes() {
    let mut current = source();
    let observation = observed(&mut current);
    observation.repository_visibility = IssueRepositoryVisibility::Unknown;
    observation.repository_name_with_owner = None;
    let result = run(Some(&current), &policy(), None).unwrap();
    // The pre-Auto SourceEvidence v1 JSON, including its original field order and adjacent
    // enum tags, is pinned here. Unknown/None add no serialized fields to that input.
    assert_eq!(
        result.basis().evidence_sha256,
        "55758f2d0f8570c29af528ae41679e0627c5458f93087ce5597b25f3cd3275f2"
    );
    let field_hash = result.basis().field_hash.as_ref().unwrap();
    assert_eq!(field_hash.version, 1);
    assert_eq!(
        field_hash.sha256,
        "4bb8110881a8b4c03e4fd5d004aac1c2378844e0ed71e23383dbbd70f0c65b4a"
    );

    let mut encoded = serde_json::to_value(&current).unwrap();
    encoded["value"]
        .as_object_mut()
        .unwrap()
        .remove("repositoryVisibility");
    encoded["value"]
        .as_object_mut()
        .unwrap()
        .remove("repositoryNameWithOwner");
    let decoded: IssueSourceRead = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        run(Some(&decoded), &policy(), None).unwrap().basis(),
        result.basis()
    );
}

#[test]
fn display_web_and_logins_cannot_supply_missing_auto_repository_authority() {
    let mut current = source();
    observed(&mut current).repository_name_with_owner = None;
    let before = run(Some(&current), &policy(), None).unwrap();
    let observation = observed(&mut current);
    observation.web = Some(crate::domain::issue_automation::source::IssueSourceWeb {
        number: 99,
        url: "https://github.com/refinedstone/codex-exec-loop/issues/99".into(),
    });
    observation.author.as_mut().unwrap().login = "renamed-display-user".into();
    let after = run(Some(&current), &policy(), Some(&before)).unwrap();
    assert_eq!(after, before);
    assert_eq!(after.effective_mode(), IssueAutomationAutonomy::Propose);
}

#[test]
fn auto_delivery_and_merge_require_explicit_auto_policy_and_preserve_custom_human_rules() {
    let current = source();
    for delivery in [
        IssueAutomationGateAction::Human,
        IssueAutomationGateAction::TimeoutDefault,
        IssueAutomationGateAction::TimeoutArbiter,
    ] {
        let mut policy = policy();
        policy.gates.auto.normal.delivery = delivery;
        let result = run(Some(&current), &policy, None).unwrap();
        assert_eq!(
            action(&result, IssueAutomationGate::Delivery),
            IssueAutomationGateAction::Human
        );
    }
    for gate in [
        IssueAutomationGate::Start,
        IssueAutomationGate::Plan,
        IssueAutomationGate::Delivery,
        IssueAutomationGate::Merge,
    ] {
        let mut policy = policy();
        policy.gates.auto.normal.plan = IssueAutomationGateAction::TimeoutArbiter;
        policy
            .human_required
            .additional
            .push(IssueAutomationAdditionalHumanRule {
                labels_any: vec!["akra:auto".into()],
                paths_any: vec![],
                gates: vec![gate],
            });
        let result = run(Some(&current), &policy, None).unwrap();
        assert_eq!(action(&result, gate), IssueAutomationGateAction::Human);
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::PolicyHumanRequired)
        );
    }
    for mode in [
        IssueAutomationAutonomy::Propose,
        IssueAutomationAutonomy::Gated,
    ] {
        let mut policy = policy();
        policy.binding.github.mode = mode;
        let result = run(Some(&current), &policy, None).unwrap();
        for gate in [IssueAutomationGate::Delivery, IssueAutomationGate::Merge] {
            assert_eq!(action(&result, gate), IssueAutomationGateAction::Human);
            let mut forged = result.to_record();
            forged
                .gates
                .iter_mut()
                .find(|row| row.gate == gate)
                .unwrap()
                .action = IssueAutomationGateAction::Auto;
            assert!(IssueTriageAssessment::hydrate(forged).is_err());
        }
    }
}

#[test]
fn auto_delivery_and_merge_select_the_current_risk_class_policy() {
    for risk in [
        IssueAutomationRiskClass::Low,
        IssueAutomationRiskClass::Normal,
    ] {
        let mut policy = policy();
        policy.gates.auto.low.delivery = IssueAutomationGateAction::Human;
        policy.gates.auto.low.merge = IssueAutomationGateAction::Auto;
        policy.gates.auto.normal.delivery = IssueAutomationGateAction::Auto;
        policy.gates.auto.normal.merge = IssueAutomationGateAction::Human;
        policy.classify.rule = vec![IssueAutomationRiskRule {
            class: risk,
            labels_any: vec!["akra:auto".into()],
            paths_any: vec![],
        }];
        let result = run(Some(&source()), &policy, None).unwrap();
        assert_eq!(result.risk_class(), risk);
        let (delivery, merge) = if risk == IssueAutomationRiskClass::Low {
            (
                IssueAutomationGateAction::Human,
                IssueAutomationGateAction::Auto,
            )
        } else {
            (
                IssueAutomationGateAction::Auto,
                IssueAutomationGateAction::Human,
            )
        };
        assert_eq!(action(&result, IssueAutomationGate::Delivery), delivery);
        assert_eq!(action(&result, IssueAutomationGate::Merge), merge);
    }
}

#[test]
fn auto_cannot_override_protected_high_risk_masked_held_or_exhausted_conditions() {
    for condition in 0..7 {
        let mut current = source();
        let mut policy = policy();
        let mut held = false;
        match condition {
            0 => set_body(
                &mut current,
                format!("{VALID_BODY}\nEdit `.github/workflows/ci.yml`."),
            ),
            1 => policy.classify.rule.push(IssueAutomationRiskRule {
                class: IssueAutomationRiskClass::High,
                labels_any: vec!["akra:auto".into()],
                paths_any: vec![],
            }),
            2 => set_body(&mut current, format!("{VALID_BODY}\n[removed]")),
            3 => held = true,
            4 => observed(&mut current).labels.items.push(IssueSourceLabel {
                id: "L_hold".into(),
                name: policy.labels.hold.clone(),
            }),
            5 => observed(&mut current).labels.items.push(IssueSourceLabel {
                id: "L_manual".into(),
                name: policy.labels.manual.clone(),
            }),
            6 => policy.budget.max_running = 0,
            _ => unreachable!(),
        }
        let issue = issue();
        let requirements = IssueRequirements::default();
        let mut request = input(&issue, Some(&current), &policy, &requirements, None);
        request.held = held;
        let result = triage_issue(request).unwrap();
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human),
            "condition {condition}"
        );
        assert!(!policy_start(&result));
    }
}

fn start_request(
    assessment: &IssueTriageAssessment,
    timeout: IssueDecisionTimeout,
) -> IssueDecisionRequest {
    let shared = |value| IssueDecisionSharedText::new_shared(value).unwrap();
    IssueDecisionRequest::new(
        IssueDecisionId::new("auto-start-deadline").unwrap(),
        IssueDecisionDefinition {
            subject: IssueDecisionSubject::Issue(issue()),
            gate: IssueAutomationGate::Start,
            kind: IssueDecisionKind::StartApproval,
            question: shared("Start this exact issue?"),
            options: vec![IssueDecisionOption {
                id: "start".into(),
                label: shared("Start"),
                effect: shared("Start this exact issue"),
            }],
            recommended: Some("start".into()),
            basis: IssueDecisionBasis::from_assessment(
                assessment,
                IssueAutomationGate::Start,
                None,
            )
            .unwrap(),
            on_timeout: timeout,
            due_at: Some("1970-01-01T00:00:01Z".into()),
            asked_by: "triage".into(),
            evidence: vec![],
            delivery_review: None,
        },
    )
    .unwrap()
}

#[test]
fn auto_start_reuses_exact_default_and_arbiter_answers_without_bypassing_human_gates() {
    for timeout in [IssueDecisionTimeout::Default, IssueDecisionTimeout::Arbiter] {
        let mut policy = policy();
        let action = if timeout == IssueDecisionTimeout::Default {
            IssueAutomationGateAction::TimeoutDefault
        } else {
            IssueAutomationGateAction::TimeoutArbiter
        };
        policy.gates.auto.normal.start = action;
        let current = source();
        let assessment = run(Some(&current), &policy, None).unwrap();
        assert!(ready_for_start(&assessment));
        assert!(!policy_start(&assessment));
        let request = start_request(&assessment, timeout);
        let basis = &request.definition().basis;
        let answer = if timeout == IssueDecisionTimeout::Default {
            assert!(
                request
                    .timeout_for_gate(basis, action, 999)
                    .unwrap()
                    .is_none()
            );
            request
                .timeout_for_gate(basis, action, 1000)
                .unwrap()
                .unwrap()
                .request
        } else {
            request
                .answer_arbiter(
                    basis,
                    &assessment,
                    &IssueArbiterSettings::default(),
                    &IssueArbiterProposal {
                        decision: "start".into(),
                        confidence: 10000,
                        has_blockers: false,
                        note: None,
                    },
                    1000,
                )
                .unwrap()
                .request
        };
        assert_eq!(
            timeout_default_start(&assessment, &answer),
            timeout == IssueDecisionTimeout::Default
        );
        assert_eq!(
            timeout_arbiter_start(&assessment, &answer),
            timeout == IssueDecisionTimeout::Arbiter
        );
        let mut public = current.clone();
        observed(&mut public).repository_visibility = IssueRepositoryVisibility::Public;
        let changed = run(Some(&public), &policy, Some(&assessment)).unwrap();
        assert!(!timeout_default_start(&changed, &answer));
        assert!(!timeout_arbiter_start(&changed, &answer));
        policy
            .human_required
            .additional
            .push(IssueAutomationAdditionalHumanRule {
                labels_any: vec!["akra:auto".into()],
                paths_any: vec![],
                gates: vec![IssueAutomationGate::Start],
            });
        let human = run(Some(&current), &policy, None).unwrap();
        assert!(!timeout_default_start(&human, &answer));
        assert!(!timeout_arbiter_start(&human, &answer));
    }
}
