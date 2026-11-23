use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::adapter::outbound::filesystem::RepositoryKernelLockAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkExecutionPort, IssueWorkExecutionSnapshot, IssueWorkPermitRequest,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_binding_admission::IssueBindingAdmission;
use crate::application::service::issue_binding_pause::IssueBindingPauseService;
use crate::domain::issue_automation::binding_pause::{
    IssueBindingPauseCompletion, IssueBindingPauseIntent,
};
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::{IssueAutomationBinding, IssueAutomationBindingId};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};
use crate::domain::planning::PlanningAuthorityLocation;

struct NoPolicy;
impl IssueAutomationPolicySourcePort for NoPolicy {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        anyhow::bail!("policy unavailable")
    }
    fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
        anyhow::bail!("policy unavailable")
    }
}
struct NoExecution;
impl IssueWorkExecutionPort for NoExecution {
    fn snapshot(&self, _: &str, _: &str) -> anyhow::Result<IssueWorkExecutionSnapshot> {
        panic!("no work in the TUI fixture")
    }
    fn submit(&self, _: &str, _: &IssueWorkOrder) -> anyhow::Result<Vec<String>> {
        panic!("TUI attachment cannot start work")
    }
    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> anyhow::Result<()> {
        panic!("TUI attachment cannot grant work")
    }
    fn revoke(&self, _: &str, _: &str) -> anyhow::Result<()> {
        panic!("no work in the TUI fixture")
    }
}
#[derive(Default)]
struct Log(std::sync::Mutex<Vec<RecordedKernelEvent>>);
impl KernelEventLogPort for Log {
    fn append_kernel_events(&self, _: &str, records: &[RecordedKernelEvent]) -> anyhow::Result<()> {
        self.0.lock().unwrap().extend_from_slice(records);
        Ok(())
    }
    fn load_kernel_events(&self, _: &str, kinds: &[&str]) -> anyhow::Result<Vec<KernelEvent>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|record| kinds.contains(&record.event.kind()))
            .map(|record| record.event.clone())
            .collect())
    }
}
struct NoClipboard;
impl ClipboardImageProbeTrigger for NoClipboard {
    fn request_probe(
        &self,
        _: Box<dyn FnOnce(crate::domain::clipboard_image::ClipboardImageProbeOutcome) + Send>,
    ) -> bool {
        false
    }
}

#[test]
fn tui_attachment_keeps_known_open_binding_and_ordinary_runtime_without_an_issue_observer() {
    assert_tui_binding_owner(TuiOwnerCase::Matching);
}

#[test]
fn tui_without_kernel_owner_keeps_issue_admission_closed_and_pause_facts_unchanged() {
    assert_tui_binding_owner(TuiOwnerCase::Missing);
}

#[test]
fn tui_with_another_repository_owner_keeps_issue_admission_closed_and_pause_facts_unchanged() {
    assert_tui_binding_owner(TuiOwnerCase::Mismatched);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TuiOwnerCase {
    Matching,
    Missing,
    Mismatched,
}

fn assert_tui_binding_owner(owner_case: TuiOwnerCase) {
    let root = crate::test_utils::platform_safe_temp_dir()
        .join(format!("pause-tui-{:016x}", rand::random::<u64>()));
    crate::private_directory::ensure_private_directory(&root).unwrap();
    let workspace = root.to_string_lossy().into_owned();
    let runtime_dir = root.join("runtime");
    crate::private_directory::ensure_private_directory(&runtime_dir).unwrap();
    let store_runtime_dir = runtime_dir.clone();
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
        Ok(store_runtime_dir.clone())
    }));
    let ids: Vec<_> = ["open", "paused", "pending"]
        .into_iter()
        .map(|id| IssueAutomationBindingId::new(id).unwrap())
        .collect();
    for id in &ids {
        store
            .insert_binding(
                &workspace,
                &IssueAutomationBinding::new(
                    id.clone(),
                    IssueProvider::GitHub,
                    "github.com",
                    format!("R_{id}"),
                    IssueProjectScope {
                        workspace_dir: workspace.clone(),
                    },
                )
                .unwrap(),
            )
            .unwrap();
    }
    for (index, id) in ids.iter().enumerate().skip(1) {
        let intent = IssueBindingPauseIntent {
            command_id: format!("pause-{id}"),
            binding_id: id.clone(),
            paused: true,
            expected_revision: 0,
            resume_basis: None,
        };
        store
            .begin_issue_binding_pause(&workspace, &intent)
            .unwrap();
        if index == 1 {
            store
                .finish_issue_binding_pause(&workspace, &intent, IssueBindingPauseCompletion::Apply)
                .unwrap();
        }
    }
    let admission = Arc::new(IssueBindingAdmission::new());
    let service = Arc::new(IssueBindingPauseService::new(
        workspace.clone(),
        store.clone(),
        IssueAutomationPolicyService::new(Arc::new(NoPolicy)),
        Arc::new(NoExecution),
        admission.clone(),
    ));
    let target = crate::composition::production::RepositoryKernelTarget {
        repository_key: "a".repeat(64),
        workspace_root: workspace.clone(),
    };
    let kernel_lock = match owner_case {
        TuiOwnerCase::Missing => None,
        TuiOwnerCase::Matching | TuiOwnerCase::Mismatched => {
            let (lock_workspace, lock_runtime, repository_identity) =
                if owner_case == TuiOwnerCase::Matching {
                    (
                        root.clone(),
                        runtime_dir.clone(),
                        target.repository_key.clone(),
                    )
                } else {
                    (
                        root.join("other-repository"),
                        root.join("other-runtime"),
                        "b".repeat(64),
                    )
                };
            crate::private_directory::ensure_private_directory(&lock_workspace).unwrap();
            crate::private_directory::ensure_private_directory(&lock_runtime).unwrap();
            let lock_workspace = lock_workspace.to_string_lossy().into_owned();
            Some(
                RepositoryKernelLockAdapter::acquire(
                    PlanningAuthorityLocation {
                        workspace_root: lock_workspace.clone(),
                        canonical_repo_root: lock_workspace.clone(),
                        repository_identity,
                        runtime_dir: lock_runtime.to_string_lossy().into_owned(),
                        authority_store_path: lock_runtime
                            .join("authority.sqlite")
                            .to_string_lossy()
                            .into_owned(),
                    },
                    &lock_workspace,
                )
                .unwrap(),
            )
        }
    };
    let before_states: Vec<_> = ids.iter().map(|id| service.state(id).unwrap()).collect();
    let before_events = store
        .pending_issue_binding_pause_events(&workspace, 64)
        .unwrap();
    assert_eq!(before_events.len(), 1);
    let restored = crate::composition::production::restore_native_tui_issue_bindings(
        kernel_lock.as_ref(),
        &target,
        service.as_ref(),
        admission.as_ref(),
    );
    assert_eq!(restored, owner_case == TuiOwnerCase::Matching);
    let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let parallel = ParallelModeControlPlaneComposition::new(
        crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service().with_issue_binding_admission(admission.clone(), store.clone()),
        planning, Arc::new(crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort),
    );
    let port = Arc::new(RecordingTurnRuntimePort::default());
    let application = NativeTuiApplicationComposition::from_services(
        StartupService::new(port.clone()),
        SessionService::new(port.clone()),
        ConversationService::new(port.clone()),
        parallel,
        Arc::new(NoopConversationThreadTurnOptionsPort),
        Arc::new(NoClipboard),
    );
    let application = if restored {
        application.with_issue_binding_pause(
            service.clone(),
            admission.clone(),
            workspace.clone(),
            Arc::new(Log::default()),
        )
    } else {
        application
    };
    let mut bound = application.bind_client_runtime();
    assert!(bound.client_runtime.issue_observer.is_none());
    let active = if restored {
        assert!(Arc::ptr_eq(
            bound.client_runtime.binding_admission.as_ref().unwrap(),
            &admission
        ));
        let open = admission.capture(&ids[0]).unwrap();
        Some(admission.acquire(&ids[0], open).unwrap())
    } else {
        assert!(bound.client_runtime.binding_pause.is_none());
        assert!(bound.client_runtime.binding_admission.is_none());
        assert!(admission.capture(&ids[0]).is_err());
        assert!(
            admission
                .acquire(&ids[0], before_states[0].revision)
                .is_err()
        );
        assert!(
            service.recover_before_runtime().is_err(),
            "a later state read cannot manufacture ownership"
        );
        None
    };
    assert!(admission.capture(&ids[1]).is_err());
    assert!(admission.capture(&ids[2]).is_err());
    assert!(
        admission
            .capture(&IssueAutomationBindingId::new("unknown").unwrap())
            .is_err()
    );
    if restored {
        assert!(bound.client_runtime.has_remaining_work());
    }
    let _ = bound
        .client_runtime
        .dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
            AppCommand::SetAutoFollowMaxTurns { value: 2 },
        )));
    assert!(port.turns.lock().unwrap().is_empty());
    assert_eq!(
        bound
            .client_runtime
            .snapshot()
            .conversation_runtime
            .auto_follow
            .max_auto_turns,
        2,
        "ordinary TUI input remains available without an issue owner"
    );
    if let Some(active) = &active {
        assert!(
            active.validate().is_ok(),
            "ordinary runtime input cannot close another open binding"
        );
    }
    drop(bound);
    if let Some(active) = &active {
        assert!(
            active.validate().is_err(),
            "TUI Drop closes the shared owner even while a guard survives"
        );
    }
    drop(active);
    let after_states: Vec<_> = ids.iter().map(|id| service.state(id).unwrap()).collect();
    assert!(after_states[1].paused);
    if !restored {
        assert_eq!(
            after_states, before_states,
            "an unowned TUI preserves durable pause facts"
        );
        assert_eq!(
            store
                .pending_issue_binding_pause_events(&workspace, 64)
                .unwrap(),
            before_events,
            "an unowned fallback must leave completion ACKs to the actual owner"
        );
    }
    drop(kernel_lock);
    drop(service);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
