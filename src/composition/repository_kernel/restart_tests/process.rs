//! A killed process leaves real pending authority and a real discovery row behind.
use super::*;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::adapter::outbound::db::{
    SqliteIssueAutomationRestartRegistryAdapter, SqliteIssueAutomationStoreAdapter,
};
use crate::application::port::inbound::stored_command_port::issue_confirmation::VerifiedHumanConfirmation;
use crate::domain::issue_automation::autonomy::IssueAutonomyProjection;
use anyhow::ensure;

const CHILD_TEST: &str =
    "composition::repository_kernel::restart_tests::process::pending_gated_selection_child";
const CHILD_ROOT: &str = "AKRA_TEST_GATEWAY_RESTART_ROOT";
const COMMAND_ID: &str = "restart-crash-pending-gated";
const READY_FILE: &str = "pending-gated-ready";

/// The first registration precedes begin; the second is immediately before finish.
/// Block that real service boundary after both independent SQLite commits are durable.
struct PauseBeforeSelectionCommit {
    registry: Arc<SqliteIssueAutomationRestartRegistryAdapter>,
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    workspace: String,
    ready: PathBuf,
    registrations: AtomicUsize,
}
impl IssueAutomationRestartRegistryPort for PauseBeforeSelectionCommit {
    fn register(&self, entry: &IssueAutomationRestartEntry) -> Result<()> {
        self.registry.register(entry)?;
        if self.registrations.fetch_add(1, Ordering::SeqCst) == 1 {
            let selection = self.store.issue_autonomy_selection(&self.workspace)?;
            ensure!(selection.requested_level.is_none());
            let intent = selection
                .pending
                .context("the real set must have begun its pending selection")?;
            ensure!(intent.command_id == COMMAND_ID && intent.level == IssueAutonomyLevel::Gated);
            let command = self
                .store
                .get_issue_autonomy_command(&self.workspace, COMMAND_ID)?
                .context("the pending command must be durable")?;
            ensure!(command.completed_revision.is_none());
            ensure!(self.registry.list()? == std::slice::from_ref(entry));
            let temporary = self.ready.with_extension("tmp");
            let mut signal = std::fs::File::create_new(&temporary)?;
            signal.write_all(b"candidate and pending Gated selection committed\n")?;
            signal.sync_all()?;
            drop(signal);
            std::fs::rename(temporary, &self.ready)?;
            loop {
                std::thread::park();
            }
        }
        Ok(())
    }
    fn unregister(&self, key: &str) -> Result<()> {
        self.registry.unregister(key)
    }
    fn list(&self) -> Result<Vec<IssueAutomationRestartEntry>> {
        self.registry.list()
    }
}

#[test]
fn pending_gated_selection_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        return;
    };
    // Opening an existing fixture performs no Git initialization and owns no cleanup.
    let f = Fixture::reopen_root(PathBuf::from(root));
    let location =
        SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(&f.workspace)
            .unwrap()
            .unwrap();
    let _owner = RepositoryKernelLockAdapter::acquire_for_owner(
        location,
        &f.workspace,
        RepositoryKernelOwner::Gateway,
    )
    .unwrap();
    let pause = Arc::new(PauseBeforeSelectionCommit {
        registry: f.registry.clone(),
        store: f.store.clone(),
        workspace: f.workspace.clone(),
        ready: f.root.join(READY_FILE),
        registrations: AtomicUsize::new(0),
    });
    let autonomy = f.autonomy_with_registry(pause);
    let confirmation = VerifiedHumanConfirmation::from_verified(
        autonomy
            .prepare_confirmation(COMMAND_ID, IssueAutonomyLevel::Gated)
            .unwrap()
            .target,
        format!("desktop-confirmation:{}", "b".repeat(32)),
    )
    .unwrap();
    autonomy
        .set(
            COMMAND_ID.into(),
            IssueAutonomyLevel::Gated,
            Some(&confirmation),
        )
        .unwrap();
    panic!("the parent must kill this process before the selection commit returns");
}

#[test]
fn killed_pending_selection_restarts_without_clients_and_creates_one_durable_work() {
    let f = Fixture::new();
    let ready = f.root.join(READY_FILE);
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD_ROOT, &f.root)
        .current_dir(&f.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    // ManagedChild also kills and waits during unwinding or a readiness timeout.
    let mut child = crate::subprocess::spawn(&mut command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.is_file() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "the child exited before the durable pending boundary"
        );
        assert!(
            Instant::now() < deadline,
            "the child never reached the durable pending boundary"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        child.try_wait().unwrap().is_none(),
        "the child must still own the repository lock"
    );
    assert_eq!(
        f.registry.list().unwrap(),
        std::slice::from_ref(&f.candidate)
    );
    let pending = f.store.issue_autonomy_selection(&f.workspace).unwrap();
    assert_eq!(pending.requested_level, None);
    assert_eq!(pending.pending.as_ref().unwrap().command_id, COMMAND_ID);
    let locked = production::acquire_repository_kernel_lock(&f.workspace).unwrap_err();
    assert!(locked.downcast_ref::<RepositoryKernelLockHeld>().is_some());

    child.terminate_and_wait().unwrap();
    assert!(
        !child
            .try_wait()
            .unwrap()
            .expect("the killed child was reaped")
            .success()
    );
    drop(child);

    // No service, execution port, registry adapter, or runtime from the child is reused.
    let reopened = f.reopen();
    let candidates = reopened.registry.list().unwrap();
    assert_eq!(candidates, std::slice::from_ref(&f.candidate));
    assert!(
        reopened
            .store
            .issue_autonomy_selection(&reopened.workspace)
            .unwrap()
            .pending
            .is_some()
    );
    let pause = reopened.progress.pause(Phase::DiscoveryStarted);
    let kernels = reopened.kernels();
    assert_eq!(
        kernels
            .restore_registered_with(
                &candidates[0],
                reopened.registry.as_ref(),
                |_, admission| reopened.recover(admission)
            )
            .unwrap(),
        RepositoryRestartOutcome::Started,
    );
    reopened.wait_pause(&pause);
    assert!(reopened.work().is_none());
    assert_eq!(reopened.execution.0.lock().unwrap().submissions, 0);
    pause.release();
    reopened.wait_restart_work("recovered durable work", |work| {
        work.task_ids == ["restart-task"]
            && work.blocked_reason == Some(IssueWorkBlockReason::ExecutionDisabled)
            && work.permit_source == Some("a".repeat(40))
    });
    let selection = reopened
        .store
        .issue_autonomy_selection(&reopened.workspace)
        .unwrap();
    assert_eq!(selection.requested_level, Some(IssueAutonomyLevel::Gated));
    assert!(selection.pending.is_none());
    assert_eq!(selection.revision, 1);
    let receipt = reopened
        .store
        .get_issue_autonomy_command(&reopened.workspace, COMMAND_ID)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.completed_revision, Some(selection.revision));
    assert!(
        matches!(reopened.autonomy().state(), IssueAutonomyProjection::Ready(state)
        if !state.pending && state.effective_level == IssueAutomationAutonomy::Gated)
    );
    assert!(reopened.source.discoveries.load(Ordering::SeqCst) > 0);
    assert!(reopened.source.resyncs.load(Ordering::SeqCst) > 0);
    let work = reopened.work().unwrap();
    let works = reopened.store.list_issue_work(&reopened.workspace).unwrap();
    assert_eq!(works.len(), 1);
    assert_eq!(works[0].order, work.order);
    assert_eq!(reopened.execution.0.lock().unwrap().submissions, 1);
    assert_eq!(reopened.execution.0.lock().unwrap().grants.len(), 1);
    assert!(!reopened.observe_host(&kernels).parallel.enabled);
    // The first client arrives only after recovery, discovery, and coordination finished.
    let (client, _, _) = kernels
        .attach_with_workspace_root(&reopened.workspace, false)
        .unwrap();
    assert_eq!(client.client_id(), 1);
    drop(client);
    kernels.shutdown();
    reopened.no_model_turns();
    f.no_model_turns();
}
