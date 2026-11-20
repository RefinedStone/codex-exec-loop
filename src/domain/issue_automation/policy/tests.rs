use super::*;

#[test]
fn builtins_are_observation_only_with_all_human_gates() {
    let policy = IssueAutomationPolicy::builtin();
    policy.validate().expect("valid builtins");
    assert_eq!(policy.binding.github.mode, IssueAutomationAutonomy::Observe);
    assert_eq!(
        policy.authority.autonomy_cap,
        IssueAutomationAutonomy::Propose
    );
    assert_eq!(
        policy.budget,
        IssueAutomationBudget {
            max_running: 2,
            max_started_per_day: 5,
            max_attempts: 3,
        }
    );
    assert_eq!(policy.feedback, IssueAutomationFeedback::default());
    assert_eq!(policy.close.when, IssueAutomationCloseWhen::Never);
    assert_eq!(policy.gates, IssueAutomationPolicyGates::default());
    assert_eq!(policy.human_required.rules, IssueAutomationHumanRule::ALL);
    assert!(policy.select.trusted_actors.is_empty());
    assert!(policy.classify.rule.is_empty());
}

#[test]
fn repository_globs_use_only_relative_slash_segments_and_closed_wildcards() {
    for valid in [
        "README.md",
        "**/AGENTS.md",
        ".github/workflows/**",
        "**",
        "*.rs",
        "src/?/test*.rs",
        "한글/파일 *.md",
        "src/**/test.rs",
    ] {
        validate_glob(valid, "test").unwrap_or_else(|error| panic!("{valid}: {error}"));
    }
    for invalid in [
        "",
        " file",
        "file ",
        "/absolute",
        "C:/absolute",
        "C:\\absolute",
        "\\\\server\\share",
        "../parent",
        "src/../parent",
        "./local",
        "src/.",
        "src//file",
        "src/",
        "src/**file",
        "src/***",
        "src/[ab]",
        "src/{a,b}",
        "!src/file",
        "src/file\n",
        "src/\u{85}file",
        "src/\0file",
    ] {
        assert!(
            validate_glob(invalid, "test").is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn github_repository_requires_one_ascii_owner_and_one_name() {
    for valid in ["a/b", "Some-Owner/a_repo.test", "a/---", "a/.github"] {
        validate_repository(valid).expect("valid repository");
    }
    validate_repository(&format!("{}/{}", "a".repeat(39), "b".repeat(100))).expect("max lengths");
    for invalid in [
        "",
        "owner",
        "/name",
        "owner/",
        "owner/repo/extra",
        "-owner/repo",
        "owner-/repo",
        "owner/.",
        "owner/..",
        "owner/re po",
        "owner/한글",
        "https://github.com/owner/repo",
        " owner/repo",
        "owner/repo ",
    ] {
        assert!(
            validate_repository(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
    assert!(validate_repository(&format!("{}/repo", "a".repeat(40))).is_err());
    assert!(validate_repository(&format!("owner/{}", "a".repeat(101))).is_err());
}

#[test]
fn gate_actions_cannot_bypass_the_mode_or_human_only_gates() {
    use IssueAutomationAutonomy::{Auto, Gated, Propose};
    use IssueAutomationGate::{Delivery, Merge, Plan, Spec, Start};
    use IssueAutomationGateAction::{Auto as Automatic, Human, TimeoutArbiter, TimeoutDefault};
    let all = [Human, TimeoutDefault, TimeoutArbiter, Automatic];
    let cases: &[(
        IssueAutomationAutonomy,
        IssueAutomationGate,
        &[IssueAutomationGateAction],
    )] = &[
        (Propose, Start, &[Human]),
        (Propose, Spec, &[Human]),
        (Propose, Plan, &[Human, TimeoutArbiter]),
        (Propose, Delivery, &[Human]),
        (Propose, Merge, &[Human]),
        (Gated, Start, &all),
        (Gated, Spec, &[Human]),
        (Gated, Plan, &[Human, TimeoutArbiter]),
        (Gated, Delivery, &[Human]),
        (Gated, Merge, &[Human]),
        (Auto, Start, &all),
        (Auto, Spec, &[Human]),
        (Auto, Plan, &[Human, TimeoutArbiter]),
        (Auto, Delivery, &all),
        (Auto, Merge, &[Human, Automatic]),
    ];
    for (mode, gate, allowed) in cases {
        for action in all {
            assert_eq!(
                validate_gate_action(*mode, *gate, action).is_ok(),
                allowed.contains(&action),
                "{mode:?} {gate:?} {action:?}",
            );
        }
    }
}

#[test]
fn every_rule_needs_a_condition_and_added_human_rules_need_a_gate() {
    let mut risk = IssueAutomationRiskRule {
        class: IssueAutomationRiskClass::High,
        labels_any: Vec::new(),
        paths_any: Vec::new(),
    };
    assert_eq!(
        risk.validate().expect_err("condition").reason,
        "condition_required"
    );
    risk.labels_any.push("security".to_string());
    risk.validate().expect("label condition");
    risk.labels_any.clear();
    risk.paths_any.push("src/**".to_string());
    risk.validate().expect("path condition");
    let mut human = IssueAutomationAdditionalHumanRule {
        labels_any: vec!["manual".to_string()],
        paths_any: Vec::new(),
        gates: Vec::new(),
    };
    assert_eq!(human.validate().expect_err("gate").reason, "gate_required");
    human.gates.push(IssueAutomationGate::Close);
    human.validate().expect("close is a human gate");
    human.labels_any.clear();
    assert!(human.validate().is_err());
}

#[test]
fn typed_validation_retains_builtin_protection_and_human_conditions() {
    let mut policy = IssueAutomationPolicy::builtin();
    policy.protection.paths.clear();
    assert_eq!(
        policy.validate().expect_err("protection").reason,
        "builtin_protection_required"
    );
    let mut policy = IssueAutomationPolicy::builtin();
    policy.human_required.rules.clear();
    assert_eq!(
        policy.validate().expect_err("human rules").reason,
        "builtin_human_rules_required"
    );
    let mut policy = IssueAutomationPolicy::builtin();
    policy.binding.github.mode = IssueAutomationAutonomy::Auto;
    assert_eq!(
        policy.validate().expect_err("unclamped mode").reason,
        "exceeds_autonomy_cap"
    );
}
