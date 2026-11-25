use super::super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

use crate::adapter::outbound::db::{
    SqliteIssueAutomationRestartRegistryAdapter, SqliteIssueAutomationStoreAdapter,
};
use crate::application::port::inbound::stored_command_port::issue_confirmation::VerifiedHumanConfirmation;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_collection_origin_port::{
    IssueCollectionOrigin, IssueCollectionOriginPort,
};
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_source_port::*;
use crate::application::port::outbound::issue_work_execution_port::*;
use crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_autonomy::IssueAutonomyService;
use crate::application::service::issue_binding_pause::IssueBindingPauseService;
use crate::application::service::issue_coordinator::IssueCoordinatorService;
use crate::application::service::issue_decision::IssueDecisionService;
use crate::application::service::issue_intervention::IssueInterventionService;
use crate::application::service::issue_policy::{IssuePolicyService, issue_policy_version};
use crate::application::service::issue_triage::IssueTriageService;
use crate::composition::native_client_runtime::tests::{RecordedTurns, recording_runtime_for_test};
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::autonomy::IssueAutonomyLevel;
use crate::domain::issue_automation::coordinator::{IssueWorkCheckpoint, IssueWorkOrder};
use crate::domain::issue_automation::source::*;
use crate::domain::issue_automation::work_control::{IssueWorkControl, IssueWorkControlAction};
use crate::domain::issue_policy::IssuePolicy;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};

const GATED_POLICY: &str = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_restart_author"}]
[gates.gated.normal]
start = "auto"
delivery = "human"
merge = "human"
[budget]
max_running = 1
max_started_per_day = 1
max_attempts = 1
"#;

#[derive(Default)]
pub(super) struct MemoryRegistry(pub(super) Mutex<BTreeMap<String, IssueAutomationRestartEntry>>);
impl IssueAutomationRestartRegistryPort for MemoryRegistry {
    fn register(&self, entry: &IssueAutomationRestartEntry) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(entry.repository_key.clone(), entry.clone());
        Ok(())
    }
    fn unregister(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
    fn list(&self) -> Result<Vec<IssueAutomationRestartEntry>> {
        Ok(self.0.lock().unwrap().values().cloned().collect())
    }
}

pub(super) struct Policy(pub(super) Mutex<String>);
impl IssueAutomationPolicySourcePort for Policy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(Some(self.0.lock().unwrap().clone()))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}
struct Disclosure;
impl IssuePolicySourcePort for Disclosure {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(None)
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}
struct Origin;
impl IssueCollectionOriginPort for Origin {
    fn github_origin(&self, _: &str) -> Result<IssueCollectionOrigin> {
        Ok(IssueCollectionOrigin {
            owner: "restart-fixture".into(),
            name: "repository".into(),
        })
    }
}
pub(super) struct Source {
    pub(super) observation: IssueSourceObservation,
    pub(super) discoveries: AtomicUsize,
    pub(super) resyncs: AtomicUsize,
}
impl IssueSourcePort for Source {
    fn capabilities(&self) -> IssueSourceCapabilities {
        IssueSourceCapabilities::read_only()
    }
    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        request.validate()?;
        self.discoveries.fetch_add(1, Ordering::SeqCst);
        let mut observation = self.observation.clone();
        if let IssueSourceContent::Shared(snapshot) = &mut observation.content {
            snapshot.policy_version = request.disclosure.policy_version;
        }
        Ok(IssueDiscoveryPage {
            jira_connection: None,
            jira_binding: None,
            linear: None,
            repository: IssueSourceRepository::from_reference(&observation.issue),
            observations: vec![observation],
            next_cursor: None,
            scanned_count: 1,
        })
    }
    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        assert_eq!(request.issue, self.observation.issue);
        self.resyncs.fetch_add(1, Ordering::SeqCst);
        let mut observation = self.observation.clone();
        if let IssueSourceContent::Shared(snapshot) = &mut observation.content {
            snapshot.policy_version = request.disclosure.policy_version;
        }
        IssueSourceRead::Observed(Box::new(observation))
    }
}

#[derive(Default)]
pub(super) struct ExecutionState {
    order: Option<IssueWorkOrder>,
    pub(super) submissions: usize,
    pub(super) grants: Vec<IssueWorkPermitRequest>,
    pub(super) control: IssueWorkControl,
    commands: BTreeMap<String, (IssueWorkControlAction, IssueWorkControl)>,
}
#[derive(Default)]
pub(super) struct Execution(pub(super) Mutex<ExecutionState>);
impl IssueWorkExecutionPort for Execution {
    fn execution_limit(&self, _: &str, _: &str, _: u32) -> Result<()> {
        Ok(())
    }
    fn control(&self, _: &str, _: &str) -> Result<IssueWorkControl> {
        Ok(self.0.lock().unwrap().control.clone())
    }
    fn apply_control(
        &self,
        _: &str,
        _: &str,
        command: &str,
        action: IssueWorkControlAction,
    ) -> Result<IssueWorkControl> {
        let mut state = self.0.lock().unwrap();
        if let Some((old, control)) = state.commands.get(command) {
            assert_eq!(*old, action);
            return Ok(control.clone());
        }
        state.control = state.control.apply(action)?;
        let control = state.control.clone();
        state
            .commands
            .insert(command.into(), (action, control.clone()));
        Ok(control)
    }
    fn snapshot(&self, _: &str, _: &str) -> Result<IssueWorkExecutionSnapshot> {
        let state = self.0.lock().unwrap();
        let last = state.grants.last();
        Ok(IssueWorkExecutionSnapshot {
            task_ids: state
                .order
                .as_ref()
                .map(|_| vec!["restart-task".into()])
                .unwrap_or_default(),
            dispatch_source: Some("a".repeat(40)),
            saved_source: last.map(|grant| grant.source_sha.clone()),
            saved_gate_revision: last.map_or(0, |grant| grant.gate_revision),
            saved_authorization_epoch: last.map_or(0, |grant| grant.authorization_epoch),
            ..IssueWorkExecutionSnapshot::default()
        })
    }
    fn submit(&self, _: &str, order: &IssueWorkOrder) -> Result<Vec<String>> {
        let mut state = self.0.lock().unwrap();
        if let Some(old) = &state.order {
            assert_eq!(old, order);
        } else {
            state.order = Some(order.clone());
            state.submissions += 1;
        }
        Ok(vec!["restart-task".into()])
    }
    fn grant(&self, _: &str, request: &IssueWorkPermitRequest) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        state.control.require_epoch(request.authorization_epoch)?;
        if !state.grants.contains(request) {
            state.grants.push(request.clone());
        }
        Ok(())
    }
    fn revoke(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
}

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) workspace: String,
    runtime: PathBuf,
    pub(super) candidate: IssueAutomationRestartEntry,
    pub(super) store: Arc<SqliteIssueAutomationStoreAdapter>,
    pub(super) registry: Arc<SqliteIssueAutomationRestartRegistryAdapter>,
    cleanup: bool,
    pub(super) policy: Arc<Policy>,
    pub(super) source: Arc<Source>,
    pub(super) execution: Arc<Execution>,
    pub(super) builds: Arc<AtomicUsize>,
    turns: Arc<Mutex<Vec<RecordedTurns>>>,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-restart-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        crate::private_directory::ensure_private_directory(&root.join("repository")).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let workspace = root.join("repository").display().to_string();
        initialize_git(Path::new(&workspace));
        // Simulate an already selected manual repository, not a boot discovery side effect.
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&workspace).unwrap();
        Self::from_existing(root, true)
    }

    pub(super) fn reopen(&self) -> Self {
        Self::reopen_root(self.root.clone())
    }

    pub(super) fn reopen_root(root: PathBuf) -> Self {
        Self::from_existing(root, false)
    }

    fn from_existing(root: PathBuf, cleanup: bool) -> Self {
        let workspace = root.join("repository").display().to_string();
        let location =
            SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(&workspace)
                .unwrap()
                .expect("the fixture already has an identity");
        let candidate = production::issue_automation_restart_target(&workspace)
            .unwrap()
            .unwrap();
        let workspace = candidate.workspace_root.clone();
        let runtime = PathBuf::from(location.runtime_dir);
        let stored_runtime = runtime.clone();
        let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(stored_runtime.clone())
        }));
        let registry = Arc::new(SqliteIssueAutomationRestartRegistryAdapter::with_runtime(
            root.join("restart-profile")
                .join("issue-automation")
                .join("runtime"),
        ));
        let shared_copy = IssueSharedCopy {
            title: "Restart fixture issue".into(),
            body: "## 완료 기준\n- 재시작 뒤에도 작업 하나와 동일한 결과를 유지한다.\n".into(),
        };
        let complete = |items| IssueMetadataList {
            items,
            completeness: IssueMetadataCompleteness::Complete,
        };
        let observation = IssueSourceObservation {
            content_problems: vec![],
            jira: None,
            linear: None,
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".into(),
                repository_id: "R_restart".into(),
                issue_id: "I_restart".into(),
                project_scope: IssueProjectScope {
                    workspace_dir: workspace.clone(),
                },
            },
            creation_provenance: IssueCreationProvenance::Unattributed,
            repository_visibility: IssueRepositoryVisibility::Private,
            repository_name_with_owner: Some("restart-fixture/repository".into()),
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: complete(Vec::new()),
            assignees: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            author: Some(IssueSourceActor {
                kind: IssueSourceActorKind::UserAccount,
                id: "U_restart_author".into(),
                login: "fixture".into(),
            }),
            label_actors: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            linked_pull_request: IssuePrPresence::Absent,
            content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                    provider_updated_at: "2026-11-23T00:00:00Z".into(),
                },
                shared_copy,
                policy_version: issue_policy_version(&IssuePolicy::builtin()),
            }),
        };
        Self {
            root,
            workspace,
            runtime,
            candidate,
            store,
            registry,
            cleanup,
            policy: Arc::new(Policy(Mutex::new(GATED_POLICY.into()))),
            source: Arc::new(Source {
                observation,
                discoveries: AtomicUsize::new(0),
                resyncs: AtomicUsize::new(0),
            }),
            execution: Arc::new(Execution::default()),
            builds: Arc::new(AtomicUsize::new(0)),
            turns: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn autonomy(&self) -> Arc<IssueAutonomyService> {
        self.autonomy_with_registry(self.registry.clone())
    }

    pub(super) fn autonomy_with_registry(
        &self,
        registry: Arc<dyn IssueAutomationRestartRegistryPort>,
    ) -> Arc<IssueAutonomyService> {
        Arc::new(
            IssueAutonomyService::new(
                self.workspace.clone(),
                self.store.clone(),
                IssueAutomationPolicyService::new(self.policy.clone()),
                self.execution.clone(),
            )
            .with_restart_registry(registry, production::issue_automation_restart_target),
        )
    }

    pub(super) fn select(&self, level: IssueAutonomyLevel) {
        let autonomy = self.autonomy();
        let command = format!("restart-selection-{level:?}");
        let proof = if level == IssueAutonomyLevel::Off {
            None
        } else {
            Some(
                VerifiedHumanConfirmation::from_verified(
                    autonomy
                        .prepare_confirmation(&command, level)
                        .unwrap()
                        .target,
                    format!("desktop-confirmation:{}", "a".repeat(32)),
                )
                .unwrap(),
            )
        };
        autonomy.set(command, level, proof.as_ref()).unwrap();
    }

    pub(super) fn service(
        &self,
        admission: Arc<IssueBindingAdmission>,
    ) -> Arc<IssueCollectionService> {
        let triage = Arc::new(IssueTriageService::new(
            self.workspace.clone(),
            self.store.clone(),
            IssueAutomationPolicyService::new(self.policy.clone()),
            IssuePolicyService::new(Arc::new(Disclosure)),
        ));
        let decisions = Arc::new(IssueDecisionService::new(
            self.workspace.clone(),
            self.store.clone(),
            triage.clone(),
            IssuePolicyService::new(Arc::new(Disclosure)),
        ));
        let coordinator = Arc::new(
            IssueCoordinatorService::new(
                self.workspace.clone(),
                self.store.clone(),
                self.source.clone(),
                triage.clone(),
                decisions,
                IssuePolicyService::new(Arc::new(Disclosure)),
                self.execution.clone(),
            )
            .with_issue_binding_admission(admission.clone()),
        );
        let binding_pause = Arc::new(IssueBindingPauseService::new(
            self.workspace.clone(),
            self.store.clone(),
            IssueAutomationPolicyService::new(self.policy.clone()),
            self.execution.clone(),
            admission,
        ));
        Arc::new(
            IssueCollectionService::new(
                self.workspace.clone(),
                self.store.clone(),
                self.source.clone(),
                Arc::new(Origin),
                IssuePolicyService::new(Arc::new(Disclosure)),
            )
            .with_triage(triage)
            .with_autonomy(self.autonomy())
            .with_binding_pause(binding_pause)
            .with_coordinator(coordinator),
        )
    }

    pub(super) fn recover(
        &self,
        admission: Arc<IssueBindingAdmission>,
    ) -> Result<Arc<IssueCollectionService>> {
        IssueInterventionService::new(
            self.workspace.clone(),
            self.store.clone(),
            self.execution.clone(),
            Arc::new(UnavailableIssueWorkSteering),
        )
        .recover_before_runtime()?;
        self.autonomy().recover_before_runtime()?;
        Ok(self.service(admission))
    }

    pub(super) fn kernels(&self) -> RepositoryKernelRegistry {
        self.kernels_with_runtime_hook(|_, _| Ok(()))
    }

    pub(super) fn kernels_with_runtime_hook(
        &self,
        hook: impl Fn(&mut NativeClientRuntime, &Arc<IssueBindingAdmission>) -> Result<()>
        + Send
        + Sync
        + 'static,
    ) -> RepositoryKernelRegistry {
        let turns = self.turns.clone();
        let builds = self.builds.clone();
        RepositoryKernelRegistry {
            restart_runtime: Some(Arc::new(move |_workspace, issues, admission| {
                builds.fetch_add(1, Ordering::SeqCst);
                let (mut runtime, recorded) = recording_runtime_for_test();
                turns.lock().unwrap().push(recorded);
                hook(&mut runtime, &admission)?;
                runtime.attach_issue_binding_pause(
                    issues
                        .binding_pause_service()
                        .expect("fixture binding pause"),
                    admission,
                )?;

                Ok(runtime)
            })),
            ..RepositoryKernelRegistry::default()
        }
    }

    pub(super) fn restore(
        &self,
        kernels: &RepositoryKernelRegistry,
    ) -> Result<RepositoryRestartOutcome> {
        kernels.restore_registered_with(
            &self.candidate,
            self.registry.as_ref(),
            |workspace, admission| {
                assert_eq!(workspace, self.workspace);
                self.recover(admission)
            },
        )
    }

    pub(super) fn work(&self) -> Option<IssueWorkCheckpoint> {
        self.store
            .get_issue_work(&self.workspace, &self.source.observation.issue)
            .unwrap()
    }

    pub(super) fn no_model_turns(&self) {
        for turns in self.turns.lock().unwrap().iter() {
            assert!(turns.lock().unwrap().is_empty());
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.no_model_turns();
        if self.cleanup {
            let _ = std::fs::remove_dir_all(&self.root);
            let _ = std::fs::remove_dir_all(&self.runtime);
        }
    }
}

pub(super) fn git(directory: &Path, arguments: &[&str]) {
    let output = crate::test_utils::git_command()
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
pub(super) fn initialize_git(directory: &Path) {
    git(directory, &["init", "-q"]);
    git(
        directory,
        &["config", "user.email", "restart-fixture@example.invalid"],
    );
    git(directory, &["config", "user.name", "Restart Fixture"]);
    std::fs::write(directory.join("README"), "synthetic restart fixture\n").unwrap();
    git(directory, &["add", "README"]);
    git(directory, &["commit", "-qm", "fixture"]);
}
pub(super) fn git_path(path: &Path) -> String {
    path.display()
        .to_string()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/")
}
pub(super) fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "the expected durable restart result did not arrive"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
