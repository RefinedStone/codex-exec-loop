//! Cross-store failure boundaries: discovery is durable, but never execution authority.
use super::*;
use crate::application::port::outbound::{
    IssueAutomationRestartEntry, IssueAutomationRestartRegistryPort,
};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct RegistryState {
    entries: BTreeMap<String, IssueAutomationRestartEntry>,
    register_calls: usize,
    unregister_calls: usize,
    list_calls: usize,
    fail_register_at: Option<usize>,
    fail_register: bool,
    fail_register_after_write: bool,
    fail_unregister: bool,
}

#[derive(Default)]
struct RecordingRegistry {
    state: Mutex<RegistryState>,
}

impl IssueAutomationRestartRegistryPort for RecordingRegistry {
    fn register(&self, entry: &IssueAutomationRestartEntry) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.register_calls += 1;
        if state.fail_register || state.fail_register_at == Some(state.register_calls) {
            bail!("simulated restart registration failure");
        }
        state
            .entries
            .insert(entry.repository_key.clone(), entry.clone());
        if state.fail_register_after_write {
            bail!("simulated restart registration acknowledgement loss");
        }
        Ok(())
    }

    fn unregister(&self, repository_key: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.unregister_calls += 1;
        if state.fail_unregister {
            bail!("simulated restart removal failure");
        }
        state.entries.remove(repository_key);
        Ok(())
    }

    fn list(&self) -> Result<Vec<IssueAutomationRestartEntry>> {
        let mut state = self.state.lock().unwrap();
        state.list_calls += 1;
        Ok(state.entries.values().cloned().collect())
    }
}

fn registry_entry(h: &Harness) -> IssueAutomationRestartEntry {
    IssueAutomationRestartEntry {
        repository_key: "test-repository-incarnation".into(),
        workspace_root: h.fixture.workspace.clone(),
    }
}

fn service(h: &Harness, registry: &Arc<RecordingRegistry>) -> IssueAutonomyService {
    let entry = registry_entry(h);
    autonomy(h).with_restart_registry(registry.clone(), move |_| Ok(Some(entry.clone())))
}

fn use_gated_policy(h: &Harness) {
    *h.policy.text.lock().unwrap() = super::super::gated::gated_policy();
}

fn enable(service: &IssueAutonomyService, command_id: &str) {
    let proof = confirm(service, command_id, IssueAutonomyLevel::Gated);
    service
        .set(command_id.into(), IssueAutonomyLevel::Gated, Some(&proof))
        .unwrap();
}

fn selection(h: &Harness) -> IssueAutonomySelection {
    h.fixture
        .store()
        .issue_autonomy_selection(&h.fixture.workspace)
        .unwrap()
}

fn assert_present(h: &Harness, registry: &RecordingRegistry) {
    let expected = registry_entry(h);
    assert_eq!(
        registry
            .state
            .lock()
            .unwrap()
            .entries
            .get(&expected.repository_key),
        Some(&expected)
    );
}

fn assert_no_registry_calls(registry: &RecordingRegistry) {
    let state = registry.state.lock().unwrap();
    assert_eq!(state.register_calls, 0);
    assert_eq!(state.unregister_calls, 0);
    assert_eq!(state.list_calls, 0);
}

#[test]
fn queries_and_confirmation_do_not_resolve_or_write_restart_discovery() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let resolutions = Arc::new(AtomicUsize::new(0));
    let recorded_resolutions = resolutions.clone();
    let service = autonomy(&h).with_restart_registry(registry.clone(), move |_| {
        recorded_resolutions.fetch_add(1, Ordering::SeqCst);
        bail!("queries must not resolve restart identity");
    });

    assert!(matches!(service.state(), IssueAutonomyProjection::Ready(_)));
    service
        .prepare_confirmation("preview", IssueAutonomyLevel::Gated)
        .unwrap();
    resolve_runtime_policy(
        &h.fixture.workspace,
        h.fixture.store().as_ref(),
        &IssueAutomationPolicyService::new(h.policy.clone()),
    )
    .unwrap();
    // A repository with no runtime choice does not depend on the discovery cache.
    service.recover_before_runtime().unwrap();
    service.recover_before_manual_runtime().unwrap();

    assert_eq!(resolutions.load(Ordering::SeqCst), 0);
    assert_no_registry_calls(&registry);
    assert_eq!(selection(&h), IssueAutonomySelection::default());
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn unsupported_non_git_keeps_manual_selection_without_registry_calls() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let service = autonomy(&h).with_restart_registry(registry.clone(), |_| Ok(None));

    enable(&service, "manual-gated");
    service.recover_before_runtime().unwrap();
    service
        .set("manual-propose".into(), IssueAutonomyLevel::Propose, None)
        .unwrap();

    assert_eq!(
        selection(&h).requested_level,
        Some(IssueAutonomyLevel::Propose)
    );
    assert_eq!(selection(&h).revision, 2);
    assert_no_registry_calls(&registry);
    let execution = h.execution.state.lock().unwrap();
    assert_eq!(execution.registrations, 0);
    assert!(execution.grants.is_empty());
}

#[test]
fn invalid_restart_identity_cannot_publish_a_new_automatic_selection() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let service = autonomy(&h).with_restart_registry(registry.clone(), |_| {
        bail!("existing Git incarnation is unavailable");
    });
    let proof = confirm(&service, "invalid-identity", IssueAutonomyLevel::Gated);

    assert!(
        service
            .set(
                "invalid-identity".into(),
                IssueAutonomyLevel::Gated,
                Some(&proof),
            )
            .is_err()
    );
    assert_eq!(selection(&h), IssueAutonomySelection::default());
    assert_no_registry_calls(&registry);
}

#[test]
fn pre_registration_failure_preserves_selection_and_existing_permits() {
    let h = Harness::new();
    approved_work(&h);
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    registry.state.lock().unwrap().fail_register_at = Some(1);
    let service = service(&h, &registry);
    let proof = confirm(&service, "pre-register-fails", IssueAutonomyLevel::Gated);

    assert!(
        service
            .set(
                "pre-register-fails".into(),
                IssueAutonomyLevel::Gated,
                Some(&proof),
            )
            .is_err()
    );
    assert_eq!(selection(&h), IssueAutonomySelection::default());
    assert!(
        h.fixture
            .store()
            .get_issue_autonomy_command(&h.fixture.workspace, "pre-register-fails")
            .unwrap()
            .is_none()
    );
    assert!(registry.state.lock().unwrap().entries.is_empty());
    let execution = h.execution.state.lock().unwrap();
    assert_eq!(execution.control.authorization_epoch, 0);
    assert!(execution.control_commands.is_empty());
    assert!(!execution.revoked);
    assert_eq!(execution.grants.len(), 1);
}

#[test]
fn pre_registration_acknowledgement_loss_never_publishes_automatic_authority() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    registry.state.lock().unwrap().fail_register_after_write = true;
    let service = service(&h, &registry);
    let proof = confirm(&service, "pre-register-ack-loss", IssueAutonomyLevel::Gated);

    assert!(
        service
            .set(
                "pre-register-ack-loss".into(),
                IssueAutonomyLevel::Gated,
                Some(&proof),
            )
            .is_err()
    );
    assert_present(&h, &registry);
    assert_eq!(selection(&h), IssueAutonomySelection::default());
    // Gateway removes this known orphan under its repository lock. Ordinary state
    // reads and manual recovery do not mistake the candidate for a runtime choice.
    service.recover_before_manual_runtime().unwrap();
    let IssueAutonomyProjection::Ready(current) = service.state() else {
        panic!("expected current policy");
    };
    assert_eq!(current.requested_level, None);
    assert_eq!(current.effective_level, IssueAutomationAutonomy::Propose);
    assert_eq!(registry.state.lock().unwrap().register_calls, 1);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn pending_gated_recovery_registers_before_publish_and_reuses_fences() {
    let h = Harness::new();
    approved_work(&h);
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    registry.state.lock().unwrap().fail_register_at = Some(2);
    let service = service(&h, &registry);
    let proof = confirm(&service, "pending-gated", IssueAutonomyLevel::Gated);

    assert!(
        service
            .set(
                "pending-gated".into(),
                IssueAutonomyLevel::Gated,
                Some(&proof),
            )
            .is_err()
    );
    assert_present(&h, &registry);
    let pending = selection(&h);
    assert_eq!(pending.requested_level, None);
    assert_eq!(pending.revision, 0);
    assert_eq!(pending.pending.unwrap().command_id, "pending-gated");
    {
        let execution = h.execution.state.lock().unwrap();
        assert!(execution.revoked);
        assert_eq!(execution.control.authorization_epoch, 1);
        assert_eq!(execution.control_commands.len(), 1);
    }

    registry.state.lock().unwrap().fail_register_at = None;
    let restarted = self::service(&h, &registry);
    restarted.recover_before_runtime().unwrap();
    restarted.recover_before_runtime().unwrap();
    assert!(
        !restarted
            .set("pending-gated".into(), IssueAutonomyLevel::Gated, None)
            .unwrap()
            .changed
    );
    assert_present(&h, &registry);
    assert_eq!(
        selection(&h),
        IssueAutonomySelection {
            requested_level: Some(IssueAutonomyLevel::Gated),
            revision: 1,
            pending: None,
        }
    );
    assert_eq!(
        h.fixture
            .store()
            .get_issue_autonomy_command(&h.fixture.workspace, "pending-gated")
            .unwrap()
            .unwrap()
            .completed_revision,
        Some(1)
    );
    let execution = h.execution.state.lock().unwrap();
    assert_eq!(execution.control.authorization_epoch, 1);
    assert_eq!(execution.control_commands.len(), 1);
    assert_eq!(execution.registrations, 1);
    assert_eq!(execution.grants.len(), 1);
}

#[test]
fn completed_registration_failure_keeps_manual_recovery_but_not_strict_success() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    registry.state.lock().unwrap().fail_register_at = Some(3);
    let service = service(&h, &registry);
    let proof = confirm(&service, "committed-gated", IssueAutonomyLevel::Gated);

    assert!(
        service
            .set(
                "committed-gated".into(),
                IssueAutonomyLevel::Gated,
                Some(&proof),
            )
            .is_err()
    );
    let committed = selection(&h);
    assert_eq!(committed.requested_level, Some(IssueAutonomyLevel::Gated));
    assert_eq!(committed.revision, 1);
    assert!(committed.pending.is_none());
    assert_present(&h, &registry);

    registry.state.lock().unwrap().fail_register = true;
    let restarted = self::service(&h, &registry);
    assert!(restarted.recover_before_runtime().is_err());
    restarted.recover_before_manual_runtime().unwrap();
    assert!(
        restarted
            .set("committed-gated".into(), IssueAutonomyLevel::Gated, None)
            .is_err()
    );
    assert_eq!(selection(&h), committed);

    registry.state.lock().unwrap().fail_register = false;
    restarted.recover_before_runtime().unwrap();
    assert!(
        !restarted
            .set("committed-gated".into(), IssueAutonomyLevel::Gated, None)
            .unwrap()
            .changed
    );
    assert_eq!(selection(&h), committed);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn registry_outage_does_not_block_lowering_fences_or_the_completed_receipt() {
    for level in [
        IssueAutonomyLevel::Off,
        IssueAutonomyLevel::Observe,
        IssueAutonomyLevel::Propose,
    ] {
        let h = Harness::new();
        approved_work(&h);
        use_gated_policy(&h);
        let registry = Arc::new(RecordingRegistry::default());
        let service = service(&h, &registry);
        enable(&service, "gated-before-outage");
        let registrations = registry.state.lock().unwrap().register_calls;
        let collection = h
            .fixture
            .store()
            .ensure_collection(&h.fixture.workspace)
            .unwrap();
        {
            let mut state = registry.state.lock().unwrap();
            state.fail_register = true;
            state.fail_unregister = true;
        }

        assert!(service.set("lower".into(), level, None).is_err());
        let lowered = selection(&h);
        assert_eq!(lowered.requested_level, Some(level));
        assert_eq!(lowered.revision, 2);
        assert!(lowered.pending.is_none());
        assert_eq!(
            h.fixture
                .store()
                .get_issue_autonomy_command(&h.fixture.workspace, "lower")
                .unwrap()
                .unwrap()
                .completed_revision,
            Some(2)
        );
        assert_present(&h, &registry);
        assert_eq!(registry.state.lock().unwrap().register_calls, registrations);
        {
            let execution = h.execution.state.lock().unwrap();
            assert!(execution.revoked);
            assert_eq!(execution.control.authorization_epoch, 2);
            assert_eq!(execution.control_commands.len(), 2);
            assert_eq!(execution.grants.len(), 1);
        }

        let restarted = self::service(&h, &registry);
        assert!(restarted.recover_before_runtime().is_err());
        restarted.recover_before_manual_runtime().unwrap();
        assert!(restarted.set("lower".into(), level, None).is_err());
        assert_eq!(selection(&h), lowered);
        registry.state.lock().unwrap().fail_unregister = false;
        restarted.recover_before_runtime().unwrap();
        assert!(registry.state.lock().unwrap().entries.is_empty());
        assert!(!restarted.set("lower".into(), level, None).unwrap().changed);
        assert_eq!(selection(&h), lowered);
        assert_eq!(
            h.execution
                .state
                .lock()
                .unwrap()
                .control
                .authorization_epoch,
            2
        );
        assert_eq!(
            h.fixture
                .store()
                .ensure_collection(&h.fixture.workspace)
                .unwrap(),
            collection
        );
    }
}

#[test]
fn old_command_replay_reconciles_only_the_latest_durable_selection() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let service = service(&h, &registry);
    service
        .set("old-off".into(), IssueAutonomyLevel::Off, None)
        .unwrap();
    enable(&service, "old-gated");
    registry.state.lock().unwrap().entries.clear();

    assert!(
        !service
            .set("old-off".into(), IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
    assert_present(&h, &registry);
    assert_eq!(
        selection(&h).requested_level,
        Some(IssueAutonomyLevel::Gated)
    );
    assert_eq!(selection(&h).revision, 2);

    service
        .set("new-propose".into(), IssueAutonomyLevel::Propose, None)
        .unwrap();
    let registrations = registry.state.lock().unwrap().register_calls;
    assert!(
        !service
            .set("old-gated".into(), IssueAutonomyLevel::Gated, None)
            .unwrap()
            .changed
    );
    assert!(registry.state.lock().unwrap().entries.is_empty());
    assert_eq!(registry.state.lock().unwrap().register_calls, registrations);
    assert_eq!(
        selection(&h).requested_level,
        Some(IssueAutonomyLevel::Propose)
    );
    assert_eq!(selection(&h).revision, 3);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn same_level_retry_repairs_discovery_without_new_selection_or_work_epoch() {
    let h = Harness::new();
    approved_work(&h);
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let service = service(&h, &registry);
    enable(&service, "gated-first");
    registry.state.lock().unwrap().entries.clear();

    assert!(
        !service
            .set("gated-same-level".into(), IssueAutonomyLevel::Gated, None)
            .unwrap()
            .changed
    );
    assert_present(&h, &registry);
    assert_eq!(selection(&h).revision, 1);
    let execution = h.execution.state.lock().unwrap();
    assert_eq!(execution.control.authorization_epoch, 1);
    assert_eq!(execution.control_commands.len(), 1);
    assert_eq!(execution.registrations, 1);
    assert_eq!(execution.grants.len(), 1);
}

#[test]
fn a_lower_current_policy_cap_retains_the_requested_automatic_candidate() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let service = service(&h, &registry);
    enable(&service, "gated-before-cap");
    *h.policy.text.lock().unwrap() = super::super::gated::gated_policy()
        .replace("autonomy_cap = \"gated\"", "autonomy_cap = \"propose\"");
    registry.state.lock().unwrap().entries.clear();

    service.recover_before_runtime().unwrap();
    assert_present(&h, &registry);
    assert_eq!(registry.state.lock().unwrap().unregister_calls, 0);
    let IssueAutonomyProjection::Ready(current) = service.state() else {
        panic!("expected current policy");
    };
    assert_eq!(current.requested_level, Some(IssueAutonomyLevel::Gated));
    assert_eq!(current.effective_level, IssueAutomationAutonomy::Propose);
    assert_eq!(current.revision, 1);
}

#[test]
fn pending_fence_failure_blocks_manual_and_unattended_recovery() {
    let h = Harness::new();
    approved_work(&h);
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let service = service(&h, &registry);
    let proof = confirm(&service, "pending-fence", IssueAutonomyLevel::Gated);
    h.execution.state.lock().unwrap().fail_revoke_work_order =
        Some(h.work().unwrap().order.work_order_id);

    assert!(
        service
            .set(
                "pending-fence".into(),
                IssueAutonomyLevel::Gated,
                Some(&proof),
            )
            .is_err()
    );
    assert_present(&h, &registry);
    assert!(service.recover_before_runtime().is_err());
    assert!(service.recover_before_manual_runtime().is_err());
    assert!(selection(&h).pending.is_some());
    assert_eq!(selection(&h).revision, 0);

    h.execution.state.lock().unwrap().fail_revoke_work_order = None;
    self::service(&h, &registry)
        .recover_before_manual_runtime()
        .unwrap();
    assert!(selection(&h).pending.is_none());
    assert_eq!(selection(&h).revision, 1);
    assert_eq!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control
            .authorization_epoch,
        1
    );
}

#[test]
fn pending_automatic_registration_failure_also_blocks_manual_recovery() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    registry.state.lock().unwrap().fail_register_at = Some(2);
    let service = service(&h, &registry);
    let proof = confirm(&service, "pending-discovery", IssueAutonomyLevel::Gated);

    assert!(
        service
            .set(
                "pending-discovery".into(),
                IssueAutonomyLevel::Gated,
                Some(&proof),
            )
            .is_err()
    );
    registry.state.lock().unwrap().fail_register = true;
    assert!(service.recover_before_runtime().is_err());
    assert!(service.recover_before_manual_runtime().is_err());
    assert!(selection(&h).pending.is_some());
    assert_eq!(selection(&h).requested_level, None);
    assert_eq!(selection(&h).revision, 0);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn missing_wrong_target_and_stale_confirmation_never_create_restart_discovery_or_intents() {
    let h = Harness::new();
    use_gated_policy(&h);
    let registry = Arc::new(RecordingRegistry::default());
    let resolutions = Arc::new(AtomicUsize::new(0));
    let recorded_resolutions = resolutions.clone();
    let entry = registry_entry(&h);
    let service = autonomy(&h).with_restart_registry(registry.clone(), move |_| {
        recorded_resolutions.fetch_add(1, Ordering::SeqCst);
        Ok(Some(entry.clone()))
    });

    assert!(
        service
            .set("missing-proof".into(), IssueAutonomyLevel::Gated, None)
            .is_err()
    );
    let wrong_target = confirm(&service, "another-command", IssueAutonomyLevel::Gated);
    assert!(
        service
            .set(
                "wrong-proof".into(),
                IssueAutonomyLevel::Gated,
                Some(&wrong_target),
            )
            .is_err()
    );
    let stale = confirm(&service, "stale-proof", IssueAutonomyLevel::Gated);
    *h.policy.text.lock().unwrap() = super::super::gated::gated_policy()
        .replace("autonomy_cap = \"gated\"", "autonomy_cap = \"auto\"");
    let IssueAutonomyProjection::Ready(current) = service.state() else {
        panic!("changed policy must stay valid");
    };
    assert_eq!(current.cap, IssueAutomationAutonomy::Auto);
    assert!(
        service
            .set(
                "stale-proof".into(),
                IssueAutonomyLevel::Gated,
                Some(&stale),
            )
            .is_err()
    );

    assert_eq!(resolutions.load(Ordering::SeqCst), 0);
    assert_no_registry_calls(&registry);
    assert_eq!(selection(&h), IssueAutonomySelection::default());
    for command_id in ["missing-proof", "wrong-proof", "stale-proof"] {
        assert!(
            h.fixture
                .store()
                .get_issue_autonomy_command(&h.fixture.workspace, command_id)
                .unwrap()
                .is_none()
        );
    }
}
