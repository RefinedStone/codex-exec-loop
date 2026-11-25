//! The existing restart path preserves Auto, binding Pause, and the one live Host owner.
use super::*;
use crate::application::service::task_verification::TaskVerificationHandle;
use crate::domain::issue_automation::binding_pause::IssueBindingPauseOutcome;
use crate::domain::issue_automation::{IssueAutomationBinding, IssueAutomationBindingId};

const AUTO_POLICY: &str = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "auto"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_restart_author"}]
[gates.gated.normal]
start = "auto"
delivery = "human"
merge = "human"
[gates.auto.normal]
start = "auto"
delivery = "human"
merge = "human"
[budget]
max_running = 1
max_started_per_day = 1
max_attempts = 1
"#;

#[test]
fn an_explicit_auto_restart_obeys_the_current_auto_gated_or_propose_cap() {
    for (cap, effective) in [
        ("auto", IssueAutomationAutonomy::Auto),
        ("gated", IssueAutomationAutonomy::Gated),
        ("propose", IssueAutomationAutonomy::Propose),
    ] {
        let f = Fixture::new();
        *f.policy.0.lock().unwrap() = AUTO_POLICY.into();
        f.select(IssueAutonomyLevel::Auto);
        *f.policy.0.lock().unwrap() = AUTO_POLICY.replace(
            "autonomy_cap = \"auto\"",
            &format!("autonomy_cap = \"{cap}\""),
        );
        let kernels = f.kernels();
        let expected = if effective >= IssueAutomationAutonomy::Gated {
            RepositoryRestartOutcome::Started
        } else {
            RepositoryRestartOutcome::Ineligible
        };
        assert_eq!(f.restore(&kernels).unwrap(), expected);
        let IssueAutonomyProjection::Ready(current) = f.autonomy().state() else {
            panic!("the explicit selection remains readable");
        };
        assert_eq!(current.requested_level, Some(IssueAutonomyLevel::Auto));
        assert_eq!(current.effective_level, effective);
        assert_eq!(f.registry.list().unwrap(), vec![f.candidate.clone()]);
        if expected == RepositoryRestartOutcome::Started {
            wait_until(|| f.work().is_some_and(|work| work.permit_source.is_some()));
            let (client, key, _) = kernels
                .attach_with_workspace_root(&f.workspace, false)
                .unwrap();
            assert_eq!(client.client_id(), 1);
            assert_eq!(key, f.candidate.repository_key);
            assert!(client.query().is_some());
            assert_eq!(f.builds.load(Ordering::SeqCst), 1);
            assert_eq!(f.execution.0.lock().unwrap().submissions, 1);
            drop(client);
        } else {
            assert_eq!(f.builds.load(Ordering::SeqCst), 0);
            assert_eq!(f.source.discoveries.load(Ordering::SeqCst), 0);
            assert_eq!(f.execution.0.lock().unwrap().submissions, 0);
        }
        kernels.shutdown();
        f.no_model_turns();
    }
}

#[test]
fn completed_and_pending_binding_pauses_close_both_graphs_before_clientless_start() {
    for completed in [false, true] {
        let f = Fixture::new();
        f.select(IssueAutonomyLevel::Gated);
        let issue = &f.source.observation.issue;
        let binding = IssueAutomationBinding::new(
            IssueAutomationBindingId::new("restart-paused-binding").unwrap(),
            issue.provider,
            &issue.server_id,
            &issue.repository_id,
            issue.project_scope.clone(),
        )
        .unwrap();
        f.store.insert_binding(&f.workspace, &binding).unwrap();
        let preparing = f.service(Arc::new(IssueBindingAdmission::new()));
        let pause = preparing.binding_pause_service().unwrap();
        let before = pause.state(binding.id()).unwrap();
        let accepted = pause
            .set(
                "restart-pause".into(),
                binding.id().clone(),
                true,
                before.revision,
                None,
            )
            .unwrap();
        assert!(accepted.receipt.is_none());
        if completed {
            assert!(pause.poll().unwrap());
        }
        drop(pause);
        drop(preparing);

        let recovered: Arc<Mutex<Option<Arc<IssueBindingAdmission>>>> = Arc::new(Mutex::new(None));
        let expected = recovered.clone();
        let binding_id = binding.id().clone();
        let kernels = f.kernels_with_runtime_hook(move |_, admission| {
            let saved = expected.lock().unwrap();
            assert!(Arc::ptr_eq(saved.as_ref().unwrap(), admission));
            assert!(admission.capture(&binding_id).is_err());
            assert!(!admission.has_inflight());
            Ok(())
        });
        assert_eq!(
            kernels
                .restore_registered_with(&f.candidate, f.registry.as_ref(), |_, admission| {
                    *recovered.lock().unwrap() = Some(admission.clone());
                    f.recover(admission)
                })
                .unwrap(),
            RepositoryRestartOutcome::Started
        );
        wait_until(|| {
            f.store
                .get_issue_binding_pause_command(&f.workspace, "restart-pause")
                .unwrap()
                .is_some_and(|command| command.receipt.is_some())
        });
        wait_until(|| {
            f.store.get_issue(&f.workspace, issue).unwrap().is_some()
                && f.source.resyncs.load(Ordering::SeqCst) > 0
        });
        let (client, _, _) = kernels
            .attach_with_workspace_root(&f.workspace, false)
            .unwrap();
        assert_eq!(client.client_id(), 1);
        assert_eq!(f.builds.load(Ordering::SeqCst), 1);
        drop(client);
        kernels.shutdown();

        let stored = f
            .store
            .get_issue_binding_pause_command(&f.workspace, "restart-pause")
            .unwrap()
            .unwrap();
        let receipt = stored.receipt.unwrap();
        assert_eq!(receipt.revision, accepted.accepted_revision);
        assert_eq!(receipt.outcome, IssueBindingPauseOutcome::Paused);
        assert!(receipt.paused);
        assert!(f.work().is_none());
        let execution = f.execution.0.lock().unwrap();
        assert_eq!(execution.submissions, 0);
        assert!(execution.grants.is_empty());
        f.no_model_turns();
    }
}

#[test]
fn clientless_restart_and_manual_attach_keep_one_host_verification_owner() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let host = crate::application::service::task_verification::tests::Fixture::new();
    let service = host.service.clone();
    let handle = TaskVerificationHandle::default();
    let attached_handle = handle.clone();
    let kernels = f.kernels_with_runtime_hook(move |runtime, _| {
        runtime.attach_task_verification(service.clone(), &attached_handle)
    });
    assert_eq!(
        f.restore(&kernels).unwrap(),
        RepositoryRestartOutcome::Started
    );
    assert_eq!(host.repository.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.restore(&kernels).unwrap(),
        RepositoryRestartOutcome::AlreadyRunning
    );
    let (first, key, _) = kernels
        .attach_with_workspace_root(&f.workspace, false)
        .unwrap();
    let (second, again, _) = kernels
        .attach_with_workspace_root(&f.workspace, false)
        .unwrap();
    assert_eq!(first.client_id(), 1);
    assert_eq!(second.client_id(), 2);
    assert_eq!(key, again);
    assert_eq!(f.builds.load(Ordering::SeqCst), 1);
    assert_eq!(host.repository.recoveries.load(Ordering::SeqCst), 1);
    handle.attach(&host.service).unwrap();
    assert_eq!(host.repository.claims.load(Ordering::SeqCst), 0);
    assert_eq!(host.spawn_calls.load(Ordering::SeqCst), 0);
    assert_eq!(host.runner.prepares.load(Ordering::SeqCst), 0);
    drop(first);
    drop(second);
    kernels.shutdown();
    assert!(
        host.service
            .initialize_live_owner("2026-11-23T12:00:00Z")
            .is_err()
    );
    assert_eq!(host.repository.recoveries.load(Ordering::SeqCst), 1);
    f.no_model_turns();
}
