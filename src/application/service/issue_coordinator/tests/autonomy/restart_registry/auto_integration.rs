use super::*;

fn auto_policy() -> String {
    AUTO_POLICY
        .replace("mode = \"propose\"", "mode = \"auto\"")
        .replace("autonomy_cap = \"propose\"", "autonomy_cap = \"auto\"")
}

#[test]
fn auto_requires_its_current_confirmation_and_remains_discoverable_under_lower_caps() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = auto_policy();
    let registry = Arc::new(RecordingRegistry::default());
    let service = service(&h, &registry);
    let IssueAutonomyProjection::Ready(current) = service.state() else {
        panic!("expected current autonomy policy");
    };
    assert!(
        current
            .choices
            .iter()
            .find(|choice| choice.level == IssueAutonomyLevel::Auto)
            .unwrap()
            .requires_confirmation
    );
    assert!(
        service
            .set("auto-consent".into(), IssueAutonomyLevel::Auto, None)
            .is_err()
    );
    let wrong_target = confirm(&service, "auto-consent", IssueAutonomyLevel::Gated);
    assert!(
        service
            .set(
                "auto-consent".into(),
                IssueAutonomyLevel::Auto,
                Some(&wrong_target),
            )
            .is_err()
    );
    assert_no_registry_calls(&registry);
    assert_eq!(selection(&h), IssueAutonomySelection::default());
    let store = h.fixture.store();
    assert!(
        store
            .get_issue_autonomy_command(&h.fixture.workspace, "auto-consent")
            .unwrap()
            .is_none()
    );

    let proof = confirm(&service, "auto-consent", IssueAutonomyLevel::Auto);
    assert!(
        service
            .set(
                "auto-consent".into(),
                IssueAutonomyLevel::Auto,
                Some(&proof),
            )
            .unwrap()
            .changed
    );
    let receipt = store
        .get_issue_autonomy_command(&h.fixture.workspace, "auto-consent")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.completed_revision, Some(1));
    assert_present(&h, &registry);

    for (cap, effective) in [
        ("gated", IssueAutomationAutonomy::Gated),
        ("propose", IssueAutomationAutonomy::Propose),
    ] {
        *h.policy.text.lock().unwrap() = auto_policy().replace(
            "autonomy_cap = \"auto\"",
            &format!("autonomy_cap = \"{cap}\""),
        );
        registry.state.lock().unwrap().entries.clear();
        let restarted = super::service(&h, &registry);
        restarted.recover_before_runtime().unwrap();
        let IssueAutonomyProjection::Ready(current) = restarted.state() else {
            panic!("expected current capped policy");
        };
        assert_eq!(current.requested_level, Some(IssueAutonomyLevel::Auto));
        assert_eq!(current.effective_level, effective);
        assert_eq!(current.revision, 1);
        assert!(!current.pending);
        assert_present(&h, &registry);
        assert_eq!(
            store
                .get_issue_autonomy_command(&h.fixture.workspace, "auto-consent")
                .unwrap()
                .unwrap(),
            receipt
        );
    }
    assert_eq!(registry.state.lock().unwrap().unregister_calls, 0);
    let execution = h.execution.state.lock().unwrap();
    assert_eq!(execution.registrations, 0);
    assert!(execution.grants.is_empty());
}

#[test]
fn old_auto_commands_reconcile_current_selection_without_rewriting_receipts() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = auto_policy();
    let registry = Arc::new(RecordingRegistry::default());
    let service = service(&h, &registry);
    service
        .set("old-off".into(), IssueAutonomyLevel::Off, None)
        .unwrap();
    let proof = confirm(&service, "old-auto", IssueAutonomyLevel::Auto);
    service
        .set("old-auto".into(), IssueAutonomyLevel::Auto, Some(&proof))
        .unwrap();
    let store = h.fixture.store();
    let old_receipts = ["old-off", "old-auto"].map(|id| {
        store
            .get_issue_autonomy_command(&h.fixture.workspace, id)
            .unwrap()
            .unwrap()
    });
    let automatic = selection(&h);
    assert_eq!(automatic.requested_level, Some(IssueAutonomyLevel::Auto));
    assert_eq!(automatic.revision, 2);
    registry.state.lock().unwrap().entries.clear();

    assert!(
        !service
            .set("old-off".into(), IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
    assert_present(&h, &registry);
    assert_eq!(selection(&h), automatic);

    service
        .set("new-propose".into(), IssueAutonomyLevel::Propose, None)
        .unwrap();
    let lowered = selection(&h);
    assert_eq!(lowered.requested_level, Some(IssueAutonomyLevel::Propose));
    assert_eq!(lowered.revision, 3);
    let registrations = registry.state.lock().unwrap().register_calls;
    assert!(
        !service
            .set("old-auto".into(), IssueAutonomyLevel::Auto, None)
            .unwrap()
            .changed
    );
    assert_eq!(selection(&h), lowered);
    assert!(registry.state.lock().unwrap().entries.is_empty());
    assert_eq!(registry.state.lock().unwrap().register_calls, registrations);
    for receipt in old_receipts {
        assert_eq!(
            store
                .get_issue_autonomy_command(&h.fixture.workspace, &receipt.intent.command_id)
                .unwrap()
                .unwrap(),
            receipt
        );
    }
    let execution = h.execution.state.lock().unwrap();
    assert_eq!(execution.registrations, 0);
    assert!(execution.grants.is_empty());
}
