use super::*;
use crate::adapter::outbound::db::sqlite_issue_automation_store::{
    SqliteIssueAutomationStoreAdapter, test_support::seed_issue_work,
};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::service::issue_binding_admission::IssueBindingAdmission;
use crate::domain::issue_automation::binding_pause::*;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

struct BindingFixture {
    root: std::path::PathBuf,
    workspace: String,
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    admission: Arc<IssueBindingAdmission>,
    initial: IssueBindingPauseState,
    work_id: String,
}

impl BindingFixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "host-pause-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let workspace_path = root.join("workspace");
        let runtime = root.join("runtime");
        crate::private_directory::ensure_private_directory(&workspace_path).unwrap();
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        let workspace = workspace_path.to_str().unwrap().to_owned();
        let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(runtime.clone())
        }));
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_host_pause".into(),
            issue_id: "I_host_pause".into(),
            project_scope: IssueProjectScope {
                workspace_dir: workspace.clone(),
            },
        };
        let work = seed_issue_work(store.as_ref(), &issue, &["task-1".into()]).unwrap();
        let binding = store
            .resolve_issue_work_binding(&workspace, &work.order.work_order_id)
            .unwrap()
            .unwrap();
        let initial = store
            .issue_binding_pause_state(&workspace, &binding)
            .unwrap();
        Self {
            root,
            workspace,
            store,
            admission: Arc::new(IssueBindingAdmission::new()),
            initial,
            work_id: work.order.work_order_id,
        }
    }

    fn host(&self) -> Fixture {
        Fixture::with_issue_binding(
            &self.workspace,
            self.admission.clone(),
            self.store.clone(),
            &self.work_id,
        )
    }
}

impl Drop for BindingFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn pausing(initial: &IssueBindingPauseState) -> IssueBindingPauseState {
    IssueBindingPauseState {
        binding_id: initial.binding_id.clone(),
        revision: initial.revision + 1,
        paused: initial.paused,
        pending: Some(IssueBindingPauseIntent {
            command_id: "pause-host".into(),
            binding_id: initial.binding_id.clone(),
            paused: true,
            expected_revision: initial.revision,
            resume_basis: None,
        }),
    }
}

fn completed(pending: &IssueBindingPauseState, paused: bool) -> IssueBindingPauseState {
    IssueBindingPauseState {
        binding_id: pending.binding_id.clone(),
        revision: pending.revision,
        paused,
        pending: None,
    }
}

#[test]
fn delayed_host_job_cannot_start_after_pause_and_guard_lasts_until_durable_receipt() {
    let binding = BindingFixture::new();
    let host = binding.host();
    let claim = host.start();
    assert!(binding.admission.has_inflight());
    let pending = pausing(&binding.initial);
    binding
        .admission
        .begin_transition(&pending.binding_id, || Ok(((), pending.clone())))
        .unwrap();
    let paused = completed(&pending, true);
    assert!(
        binding
            .admission
            .finish_if_drained(&paused.binding_id, paused.revision, || Ok((
                (),
                paused.clone()
            )))
            .unwrap()
            .is_none()
    );
    let (job, _) = host.take_job();
    job();
    assert_eq!(host.runner.prepares.load(Ordering::SeqCst), 0);
    host.repository
        .fail_finish_once
        .store(true, Ordering::SeqCst);
    assert!(!host.service.poll(Instant::now()));
    assert!(binding.admission.has_inflight());
    assert!(host.service.poll(Instant::now() + Duration::from_secs(2)));
    assert!(!binding.admission.has_inflight());
    binding
        .admission
        .finish_if_drained(&paused.binding_id, paused.revision, || {
            Ok(((), paused.clone()))
        })
        .unwrap()
        .unwrap();
    let terminal = host
        .repository
        .load_host_verification(&binding.workspace, &claim.stored.attempt.attempt_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        terminal.receipt.unwrap().result,
        TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::Cancelled
        }
    ));
    let replay = host
        .service
        .request_verification(request(), &context())
        .unwrap();
    assert!(!replay.changed && replay.execution.is_none());
    assert_eq!(host.spawn_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn pause_and_resume_during_claim_cannot_lend_the_new_generation_to_old_host_work() {
    let binding = BindingFixture::new();
    let host = binding.host();
    let admission = binding.admission.clone();
    let pausing = pausing(&binding.initial);
    *host.repository.claim_hook.lock().unwrap() = Some(Arc::new(move || {
        admission
            .begin_transition(&pausing.binding_id, || Ok(((), pausing.clone())))
            .unwrap();
        let paused = completed(&pausing, true);
        admission
            .finish_if_drained(&paused.binding_id, paused.revision, || {
                Ok(((), paused.clone()))
            })
            .unwrap()
            .unwrap();
        let resuming = IssueBindingPauseState {
            binding_id: paused.binding_id.clone(),
            revision: paused.revision + 1,
            paused: true,
            pending: Some(IssueBindingPauseIntent {
                command_id: "resume-host".into(),
                binding_id: paused.binding_id.clone(),
                paused: false,
                expected_revision: paused.revision,
                resume_basis: Some(IssueBindingResumeBasis {
                    binding_revision: 0,
                    autonomy_revision: 1,
                    policy_version: "a".repeat(64),
                    effective_level:
                        crate::domain::issue_automation::IssueAutomationAutonomy::Gated,
                }),
            }),
        };
        admission
            .begin_transition(&paused.binding_id, || Ok(((), resuming.clone())))
            .unwrap();
        let resumed = completed(&resuming, false);
        admission
            .finish_if_drained(&resumed.binding_id, resumed.revision, || {
                Ok(((), resumed.clone()))
            })
            .unwrap()
            .unwrap();
    }));
    let claim = host.start();
    assert!(claim.changed);
    assert_eq!(host.spawn_calls.load(Ordering::SeqCst), 0);
    assert_eq!(host.runner.prepares.load(Ordering::SeqCst), 0);
    assert_eq!(
        binding
            .admission
            .capture(&binding.initial.binding_id)
            .unwrap(),
        2
    );
    assert!(host.service.poll(Instant::now()));
    assert!(!binding.admission.has_inflight());
    *host.repository.claim_hook.lock().unwrap() = None;
    assert!(
        !host
            .service
            .request_verification(request(), &context())
            .unwrap()
            .changed
    );
    assert_eq!(host.spawn_calls.load(Ordering::SeqCst), 0);
}
