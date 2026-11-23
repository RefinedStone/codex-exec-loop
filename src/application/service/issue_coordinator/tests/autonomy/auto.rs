use super::*;

fn auto_policy() -> String {
    AUTO_POLICY
        .replace("autonomy_cap = \"propose\"", "autonomy_cap = \"auto\"")
        .replace("mode = \"propose\"", "mode = \"auto\"")
}

#[test]
fn file_auto_stays_propose_until_exact_native_consent_and_replay_does_not_regrant() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = auto_policy();
    let service = autonomy(&h);
    let IssueAutonomyProjection::Ready(initial) = service.state() else {
        panic!("expected the current runtime choice");
    };
    assert_eq!(initial.requested_level, None);
    assert_eq!(initial.effective_level, IssueAutomationAutonomy::Propose);
    let choice = initial
        .choices
        .iter()
        .find(|choice| choice.level == IssueAutonomyLevel::Auto)
        .unwrap();
    assert!(choice.enabled && choice.requires_confirmation);
    assert!(
        service
            .set("auto-consent".into(), IssueAutonomyLevel::Auto, None)
            .is_err()
    );

    let preview = service
        .prepare_confirmation("auto-consent", IssueAutonomyLevel::Auto)
        .unwrap();
    for condition in [
        "공개 저장소",
        "검증을 강제",
        "정책이 정한 사람 승인",
        "이번 실행에서 자동 통합을 따로 허용하지 않았다면",
        "PR에 사람 승인",
        "Auto 선택만으로 자동 통합이 켜지지는 않습니다",
    ] {
        assert!(preview.display.detail.contains(condition), "{condition}");
    }
    let confirmation = VerifiedHumanConfirmation::from_verified(
        preview.target,
        format!("desktop-confirmation:{}", "a".repeat(32)),
    )
    .unwrap();
    assert!(
        service
            .set(
                "different-command".into(),
                IssueAutonomyLevel::Auto,
                Some(&confirmation)
            )
            .is_err()
    );
    let applied = service
        .set(
            "auto-consent".into(),
            IssueAutonomyLevel::Auto,
            Some(&confirmation),
        )
        .unwrap();
    assert!(applied.changed);
    let IssueAutonomyProjection::Ready(selected) = applied.autonomy else {
        panic!("expected the confirmed Auto choice");
    };
    assert_eq!(selected.requested_level, Some(IssueAutonomyLevel::Auto));
    assert_eq!(selected.effective_level, IssueAutomationAutonomy::Auto);
    assert_eq!(selected.revision, initial.revision + 1);
    assert!(
        !service
            .set("auto-consent".into(), IssueAutonomyLevel::Auto, None)
            .unwrap()
            .changed
    );
    assert!(
        h.execution.state.lock().unwrap().grants.is_empty(),
        "choosing Auto does not itself grant delivery"
    );
    assert!(
        h.fixture
            .store()
            .list_issue_work(&h.fixture.workspace)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn auto_confirmation_is_invalidated_by_policy_or_revision_changes() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = auto_policy();
    let service = autonomy(&h);
    let stale_policy = confirm(&service, "auto-policy", IssueAutonomyLevel::Auto);
    *h.policy.text.lock().unwrap() =
        auto_policy().replace("delivery = \"auto\"", "delivery = \"human\"");
    assert!(
        service
            .set(
                "auto-policy".into(),
                IssueAutonomyLevel::Auto,
                Some(&stale_policy)
            )
            .is_err()
    );
    let stale_revision = confirm(&service, "auto-revision", IssueAutonomyLevel::Auto);
    service
        .set("lower-before-auto".into(), IssueAutonomyLevel::Off, None)
        .unwrap();
    assert!(
        service
            .set(
                "auto-revision".into(),
                IssueAutonomyLevel::Auto,
                Some(&stale_revision)
            )
            .is_err()
    );
    let current = confirm(&service, "auto-current", IssueAutonomyLevel::Auto);
    assert!(
        service
            .set(
                "auto-current".into(),
                IssueAutonomyLevel::Auto,
                Some(&current)
            )
            .unwrap()
            .changed
    );
    let file = IssueAutomationPolicyService::new(h.policy.clone())
        .resolve(&h.fixture.workspace)
        .unwrap();
    let (effective, _) = resolve_runtime_policy(
        &h.fixture.workspace,
        h.fixture.store().as_ref(),
        &IssueAutomationPolicyService::new(h.policy.clone()),
    )
    .unwrap();
    assert_eq!(
        effective.policy.gates, file.policy.gates,
        "Auto consent preserves policy-required Human gates"
    );
}

#[test]
fn auto_choice_cannot_exceed_the_current_user_cap_even_with_a_prior_confirmation() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = auto_policy();
    let service = autonomy(&h);
    let stale = confirm(&service, "auto-cap", IssueAutonomyLevel::Auto);
    *h.policy.text.lock().unwrap() =
        auto_policy().replace("autonomy_cap = \"auto\"", "autonomy_cap = \"gated\"");
    let IssueAutonomyProjection::Ready(current) = service.state() else {
        panic!("expected the capped runtime choice");
    };
    let choice = current
        .choices
        .iter()
        .find(|choice| choice.level == IssueAutonomyLevel::Auto)
        .unwrap();
    assert!(!choice.enabled);
    assert!(choice.reason.is_some());
    assert!(
        service
            .prepare_confirmation("auto-cap", IssueAutonomyLevel::Auto)
            .is_err()
    );
    assert!(
        service
            .set("auto-cap".into(), IssueAutonomyLevel::Auto, Some(&stale))
            .is_err()
    );
    assert_eq!(
        h.fixture
            .store()
            .issue_autonomy_selection(&h.fixture.workspace)
            .unwrap()
            .revision,
        0
    );
}
