//! Delays are released by the test, never by sleeps or larger production timeouts.
use super::*;

fn complete_work(work: &crate::domain::issue_automation::coordinator::IssueWorkCheckpoint) -> bool {
    work.task_ids == ["restart-task"]
        && work.blocked_reason == Some(IssueWorkBlockReason::ExecutionDisabled)
        && work.permit_source == Some("a".repeat(40))
}

#[test]
fn restart_phase_delays_preserve_one_durable_work_and_permit() {
    for phase in [
        Phase::RuntimeStarting,
        Phase::DiscoveryStarted,
        Phase::StoreOpen,
    ] {
        let f = Arc::new(Fixture::new());
        f.select(IssueAutonomyLevel::Gated);
        let pause = if phase == Phase::StoreOpen {
            f.progress.pause_store_after_discovery()
        } else {
            f.progress.pause(phase)
        };
        let kernels = Arc::new(f.kernels());
        let restarting = kernels.clone();
        let fixture = f.clone();
        let (finished, restored) = std::sync::mpsc::channel();
        let restore = std::thread::spawn(move || {
            finished.send(fixture.restore(&restarting)).unwrap();
        });
        f.wait_pause(&pause);
        assert_eq!(f.execution.0.lock().unwrap().submissions, 0, "{phase:?}");
        assert!(f.work().is_none(), "{phase:?}");
        pause.release();
        assert_eq!(
            restored
                .recv_timeout(RESTART_WAIT)
                .unwrap_or_else(|error| panic!(
                    "{}",
                    f.diagnostic("restore_return", Instant::now(), &format!("{error:?}"))
                ))
                .unwrap(),
            RepositoryRestartOutcome::Started
        );
        restore.join().unwrap();
        let before = f.wait_restart_work("delayed durable work", complete_work);
        assert!(!f.observe_host(&kernels).parallel.enabled);
        assert_eq!(f.store.list_issue_work(&f.workspace).unwrap().len(), 1);
        assert_eq!(f.execution.0.lock().unwrap().submissions, 1);
        let permits = f.execution.0.lock().unwrap().grants.clone();
        assert_eq!(permits.len(), 1);
        kernels.shutdown();

        let mut reopened = f.reopen();
        reopened.execution = f.execution.clone();
        let second = reopened.kernels();
        assert_eq!(
            reopened.restore(&second).unwrap(),
            RepositoryRestartOutcome::Started
        );
        let after = reopened.wait_restart_work("reopened delayed work", complete_work);
        assert!(!reopened.observe_host(&second).parallel.enabled);
        let (client, _, _) = second
            .attach_with_workspace_root(&reopened.workspace, false)
            .unwrap();
        assert_eq!(
            client.client_id(),
            1,
            "phase waits do not attach a hidden client"
        );
        drop(client);
        second.shutdown();
        assert_eq!(before.order, after.order, "{phase:?}");
        assert_eq!(before.task_ids, after.task_ids, "{phase:?}");
        assert_eq!(
            reopened
                .store
                .list_issue_work(&reopened.workspace)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(f.execution.0.lock().unwrap().submissions, 1);
        assert_eq!(f.execution.0.lock().unwrap().grants, permits);
        reopened.no_model_turns();
        f.no_model_turns();
    }
}

#[test]
fn missing_restart_phase_reports_selection_collection_and_work_state() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let error = f
        .try_wait_restart_work("missing work", Duration::ZERO, complete_work)
        .unwrap_err();
    assert!(error.contains("stage=DiscoveryReturned"), "{error}");
    assert!(error.contains("phase signal missing"), "{error}");
    assert!(
        error.contains("selection[revision=1 pending=false"),
        "{error}"
    );
    assert!(error.contains("collection["), "{error}");
    assert!(error.contains("work[absent]"), "{error}");
    assert!(error.contains("submissions=0 grants=0"), "{error}");
    assert!(error.contains("elapsed_ms="), "{error}");
    assert!(
        !error.contains(&f.workspace),
        "diagnostics omit filesystem paths"
    );
    assert!(
        !error.contains("Restart fixture issue"),
        "diagnostics omit issue content"
    );
}

#[test]
fn delayed_host_mailbox_reports_its_phase_and_recovers_without_duplicate_work() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let kernels = f.kernels();
    f.restore(&kernels).unwrap();
    let before = f.wait_restart_work("before host pause", complete_work);
    let pause = f.pause_host(&kernels);
    f.wait_pause(&pause);
    // The host has acknowledged the barrier, so this is an intentionally withheld reply,
    // not a timing assumption about whether the host thread has started.
    let error = f.try_observe_host(&kernels, Duration::ZERO).unwrap_err();
    assert!(error.contains("stage=host_mailbox"), "{error}");
    assert!(error.contains("reply=TimedOut"), "{error}");
    assert!(error.contains("work[revision="), "{error}");
    assert!(error.contains("submissions=1 grants=1"), "{error}");
    pause.release();
    assert!(!f.observe_host(&kernels).parallel.enabled);
    let after = f.work().unwrap();
    assert_eq!(before.order, after.order);
    assert_eq!(before.task_ids, after.task_ids);
    assert_eq!(f.store.list_issue_work(&f.workspace).unwrap().len(), 1);
    assert_eq!(f.execution.0.lock().unwrap().submissions, 1);
    assert_eq!(f.execution.0.lock().unwrap().grants.len(), 1);
    let (client, _, _) = kernels
        .attach_with_workspace_root(&f.workspace, false)
        .unwrap();
    assert_eq!(client.client_id(), 1);
    drop(client);
    kernels.shutdown();
    f.no_model_turns();
}
