use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::application::service::issue_policy::issue_policy_version;
use crate::application::service::issue_triage::{IssueTriageRunOutcome, IssueTriagedFact};
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::collection::{
    IssueCollectionMode, IssueCollectionPage, IssueCollectionQuery,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssuePolicy};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueSharedCopy,
};
const AUTO_POLICY: &str = r#"
[binding.github]
mode = "propose"
[authority]
autonomy_cap = "propose"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_author"}]
[gates.auto.normal]
start = "auto"
delivery = "auto"
merge = "auto"
"#;

struct Fixture {
    root: PathBuf,
    workspace: String,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-triage-service-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let runtime = root.join("runtime");
        crate::private_directory::ensure_private_directory(&workspace).unwrap();
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        Self {
            root,
            workspace: workspace.to_str().unwrap().to_string(),
            runtime,
        }
    }

    fn store(&self) -> Arc<SqliteIssueAutomationStoreAdapter> {
        let runtime = self.runtime.clone();
        Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(runtime.clone())
        }))
    }

    fn service(&self, policy: Arc<MutableAutomationPolicy>) -> IssueTriageService {
        IssueTriageService::new(
            self.workspace.clone(),
            self.store(),
            IssueAutomationPolicyService::new(policy),
            IssuePolicyService::new(Arc::new(ValidationPolicy)),
        )
    }

    fn reference(&self, id: &str) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_triage".into(),
            issue_id: id.into(),
            project_scope: IssueProjectScope {
                workspace_dir: self.workspace.clone(),
            },
        }
    }

    fn observation(&self, id: &str) -> IssueSourceObservation {
        let shared_copy = IssueSharedCopy {
            title: "공유 제목".into(),
            body: "## 완료 기준\n- 기능이 요구한 결과를 반환한다.\n".into(),
        };
        IssueSourceObservation {
            content_problems: vec![],
            jira: None,
            repository_visibility: Default::default(),
            repository_name_with_owner: None,
            creation_provenance:
                crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
            issue: self.reference(id),
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: complete(Vec::new()),
            assignees: complete(Vec::new()),
            author: Some(IssueSourceActor {
                kind: crate::domain::issue_automation::source::IssueSourceActorKind::UserAccount,
                id: "U_author".into(),
                login: "author".into(),
            }),
            label_actors: complete(Vec::new()),
            linked_pull_request: IssuePrPresence::Absent,
            content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                    provider_updated_at: "2026-11-20T00:00:00Z".into(),
                },
                shared_copy,
                policy_version: disclosure_version(),
            }),
        }
    }

    fn seed(&self, observations: Vec<IssueSourceObservation>, policy_version: &str) {
        for observations in observations.chunks(100) {
            let store = self.store();
            let state = store.ensure_collection(&self.workspace).unwrap();
            let repository = IssueSourceRepository::from_reference(&observations[0].issue);
            let request = IssueDiscoveryRequest {
                target: IssueDiscoveryTarget::Repository(repository.clone()),
                selection: IssueDiscoverySelection::AllOpen,
                cursor: state.discovery_cursor.clone(),
                page_size: 100,
                disclosure: IssueSourceDisclosure {
                    policy: IssueDisclosurePolicy::builtin(),
                    policy_version: policy_version.to_string(),
                },
            };
            let page = IssueDiscoveryPage {
                jira_connection: None,
                jira_binding: None,
                repository,
                observations: observations.to_vec(),
                next_cursor: None,
                scanned_count: u16::try_from(observations.len()).unwrap(),
            };
            store
                .apply_discovery_page(&self.workspace, state.fence(), &request, &page)
                .unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn complete<T>(items: Vec<T>) -> IssueMetadataList<T> {
    IssueMetadataList {
        items,
        completeness: IssueMetadataCompleteness::Complete,
    }
}

fn disclosure_version() -> String {
    issue_policy_version(&IssuePolicy::builtin())
}

struct ValidationPolicy;

impl IssuePolicySourcePort for ValidationPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(None)
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

struct MutableAutomationPolicy {
    text: Mutex<String>,
    unavailable: AtomicBool,
}

impl MutableAutomationPolicy {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            text: Mutex::new(AUTO_POLICY.to_string()),
            unavailable: AtomicBool::new(false),
        })
    }
}

impl IssueAutomationPolicySourcePort for MutableAutomationPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        if self.unavailable.load(Ordering::Acquire) {
            bail!("UNSHARED_POLICY_SOURCE_ERROR")
        }
        Ok(Some(self.text.lock().unwrap().clone()))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

fn run(service: &IssueTriageService) -> (IssueTriageRunOutcome, Vec<IssueTriagedFact>) {
    let facts = Mutex::new(Vec::new());
    let outcome = service
        .triage_all(&IssueCollectionCancellation::new(), &|committed| {
            facts.lock().unwrap().extend(committed);
        })
        .unwrap();
    (outcome, facts.into_inner().unwrap())
}

fn page(service: &IssueTriageService) -> IssueCollectionPage {
    service.issues(&IssueCollectionQuery::default()).unwrap()
}

struct FakeSource {
    read: Mutex<IssueSourceRead>,
    calls: std::sync::atomic::AtomicUsize,
}
impl IssueSourcePort for FakeSource {
    fn capabilities(
        &self,
    ) -> crate::application::port::outbound::issue_source_port::IssueSourceCapabilities {
        crate::application::port::outbound::issue_source_port::IssueSourceCapabilities::read_only()
    }
    fn discover(
        &self,
        _: IssueDiscoveryRequest,
    ) -> std::result::Result<IssueDiscoveryPage, IssueSourceFailure> {
        panic!("coordinator does not discover")
    }
    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.disclosure.policy_version, disclosure_version());
        self.read.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct FakeExecutionState {
    observation: Option<crate::domain::issue_automation::execution::IssueWorkExecutionObservation>,
    observations_by_work: std::collections::BTreeMap<
        String,
        crate::domain::issue_automation::execution::IssueWorkExecutionObservation,
    >,
    execution_limits: Vec<(String, u32)>,
    observed_work_orders: Vec<String>,
    fail_observation_work_order: Option<String>,
    after_observation: Option<Box<dyn Fn() + Send + Sync>>,
    recoveries: Vec<
        crate::application::port::outbound::issue_work_execution_port::IssueWorkRecoveryRequest,
    >,
    permit_floor: u64,
    dispatch_source: Option<String>,
    control: crate::domain::issue_automation::work_control::IssueWorkControl,
    control_commands: std::collections::BTreeMap<
        String,
        (
            crate::domain::issue_automation::work_control::IssueWorkControlAction,
            crate::domain::issue_automation::work_control::IssueWorkControl,
        ),
    >,
    saved_authorization_epoch: u64,
    order: Option<IssueWorkOrder>,
    grants: Vec<IssueWorkPermitRequest>,
    frozen: Option<String>,
    saved_source: Option<String>,
    saved_revision: u64,
    registrations: usize,
    revoked: bool,
    revoked_work_orders: Vec<String>,
    fail_revoke_work_order: Option<String>,
    after_grant: Option<Box<dyn Fn() + Send + Sync>>,
    after_revoke: Option<Box<dyn Fn() + Send + Sync>>,
    source_unavailable: bool,
    delivery_diff: Option<crate::domain::issue_automation::delivery::IssueDeliveryDiff>,
    delivery_diff_unavailable: bool,
    fail_before_register: bool,
    fail_after_register: bool,
    fail_after_grant: bool,
}
#[derive(Default)]
struct FakeExecution {
    state: Mutex<FakeExecutionState>,
}
impl IssueWorkExecutionPort for FakeExecution {
    fn frozen_delivery_source(&self, _: &str, _: &str) -> Result<Option<String>> {
        let state = self.state.lock().unwrap();
        // source_unavailable은 remote dispatch base 조회 장애다. 이 경계는 로컬 queue만 읽는다.
        Ok(state.frozen.clone())
    }
    fn delivery_diff(
        &self,
        _: &str,
        _: &str,
        expected_source: &str,
    ) -> Result<crate::domain::issue_automation::delivery::IssueDeliveryDiff> {
        use crate::domain::issue_automation::delivery::*;
        let state = self.state.lock().unwrap();
        if state.delivery_diff_unavailable || state.frozen.as_deref() != Some(expected_source) {
            bail!("the fixture delivery diff is unavailable");
        }
        Ok(state
            .delivery_diff
            .clone()
            .unwrap_or_else(|| IssueDeliveryDiff {
                base_commit_sha: "a".repeat(40),
                source_commit_sha: expected_source.into(),
                files: vec![IssueDeliveryDiffFile {
                    path: "src/feature.rs".into(),
                    lines: IssueDeliveryDiffLines::Text {
                        added: 1,
                        deleted: 0,
                    },
                }],
            }))
    }
    fn execution_limit(&self, _: &str, work_order_id: &str, max_attempts: u32) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        let entry = (work_order_id.to_string(), max_attempts);
        if state.execution_limits.last() != Some(&entry) {
            state.execution_limits.push(entry);
        }
        Ok(())
    }

    fn permit_gate_revision(&self, _: &str, _: &str) -> Result<u64> {
        Ok(self.state.lock().unwrap().permit_floor)
    }

    fn observe_execution(
        &self,
        _: &str,
        work_order_id: &str,
    ) -> Result<Option<crate::domain::issue_automation::execution::IssueWorkExecutionObservation>>
    {
        let mut state = self.state.lock().unwrap();
        state.observed_work_orders.push(work_order_id.to_string());
        if state.fail_observation_work_order.as_deref() == Some(work_order_id) {
            bail!("simulated execution observation failure");
        }
        let value = state
            .observations_by_work
            .get(work_order_id)
            .cloned()
            .or_else(|| state.observation.clone());
        let hook = state.after_observation.take();
        drop(state);
        if let Some(hook) = hook {
            hook();
        }
        Ok(value)
    }

    fn recover_execution(
        &self,
        _: &str,
        request: &crate::application::port::outbound::issue_work_execution_port::IssueWorkRecoveryRequest,
    ) -> Result<
        crate::application::port::outbound::issue_work_execution_port::IssueWorkRecoveryReceipt,
    > {
        use crate::application::port::outbound::issue_work_execution_port::{
            IssueWorkRecoveryOutcome, IssueWorkRecoveryReceipt,
        };
        let mut state = self.state.lock().unwrap();
        state.control.require_epoch(request.authorization_epoch)?;
        if !state.recoveries.contains(request) {
            state.recoveries.push(request.clone());
        }
        Ok(IssueWorkRecoveryReceipt {
            command_id: request.command_id.clone(),
            terminal_record_id: request.terminal.record_id.clone(),
            outcome: IssueWorkRecoveryOutcome::Registered {
                validation_record_key: format!("successor-{}", request.terminal.record_id),
            },
        })
    }

    fn control(
        &self,
        _: &str,
        _: &str,
    ) -> Result<crate::domain::issue_automation::work_control::IssueWorkControl> {
        Ok(self.state.lock().unwrap().control.clone())
    }
    fn apply_control(
        &self,
        _: &str,
        _: &str,
        command_id: &str,
        action: crate::domain::issue_automation::work_control::IssueWorkControlAction,
    ) -> Result<crate::domain::issue_automation::work_control::IssueWorkControl> {
        let mut state = self.state.lock().unwrap();
        if let Some((previous, result)) = state.control_commands.get(command_id) {
            if *previous != action {
                bail!("control command reused");
            }
            return Ok(result.clone());
        }
        state.control = state.control.apply(action)?;
        state.revoked = true;
        let result = state.control.clone();
        state
            .control_commands
            .insert(command_id.to_owned(), (action, result.clone()));
        Ok(result)
    }
    fn snapshot(
        &self,
        _: &str,
        _: &str,
    ) -> Result<
        crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionSnapshot,
    > {
        let state = self.state.lock().unwrap();
        if state.source_unavailable {
            bail!("configured source is unavailable");
        }
        Ok(crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionSnapshot {
            task_ids: state.order.as_ref().map(|_| vec!["task-1".into()]).unwrap_or_default(),
            dispatch_source: Some(state.dispatch_source.clone().unwrap_or_else(|| "a".repeat(40))), frozen_source: state.frozen.clone(),
            saved_source: state.saved_source.clone(), saved_gate_revision: state.saved_revision, saved_authorization_epoch: state.saved_authorization_epoch,
            blocked_reason: None,
        })
    }
    fn task_ids(&self, _: &str, _: &str) -> Result<Vec<String>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .order
            .as_ref()
            .map(|_| vec!["task-1".into()])
            .unwrap_or_default())
    }
    fn submit(&self, _: &str, order: &IssueWorkOrder) -> Result<Vec<String>> {
        let mut state = self.state.lock().unwrap();
        if std::mem::take(&mut state.fail_before_register) {
            bail!("simulated crash before durable registration");
        }
        if state.order.is_none() {
            state.order = Some(order.clone());
            state.registrations += 1;
        }
        assert_eq!(state.order.as_ref(), Some(order));
        if std::mem::take(&mut state.fail_after_register) {
            bail!("simulated crash after durable registration");
        }
        Ok(state
            .observation
            .as_ref()
            .map(|value| value.task_ids.clone())
            .unwrap_or_else(|| vec!["task-1".into()]))
    }
    fn grant(&self, _: &str, request: &IssueWorkPermitRequest) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.control.require_epoch(request.authorization_epoch)?;
        if state.revoked && request.gate_revision <= state.saved_revision {
            bail!("revoked permit");
        }
        if state
            .saved_source
            .as_deref()
            .is_some_and(|source| source != request.source_sha)
            && request.gate_revision <= state.saved_revision
        {
            bail!("source change requires a new revision");
        }
        state.saved_source = Some(request.source_sha.clone());
        state.saved_revision = request.gate_revision;
        state.saved_authorization_epoch = request.authorization_epoch;
        state.revoked = false;
        if !state.grants.contains(request) {
            state.grants.push(request.clone());
        }
        if std::mem::take(&mut state.fail_after_grant) {
            bail!("simulated crash after durable grant");
        }
        let hook = state.after_grant.take();
        drop(state);
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }
    fn revoke(&self, _: &str, work_order_id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.revoked_work_orders.push(work_order_id.to_string());
        if state.fail_revoke_work_order.as_deref() == Some(work_order_id) {
            bail!("simulated permit revocation failure");
        }
        state.revoked = true;
        let hook = state.after_revoke.take();
        drop(state);
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }
}

struct Harness {
    fixture: Fixture,
    policy: Arc<MutableAutomationPolicy>,
    source: Arc<FakeSource>,
    execution: Arc<FakeExecution>,
    triage: Arc<IssueTriageService>,
    decisions: Arc<IssueDecisionService>,
}
impl Harness {
    fn new() -> Self {
        Self::with_observation(|_| {})
    }
    fn with_observation(edit: impl FnOnce(&mut IssueSourceObservation)) -> Self {
        let fixture = Fixture::new();
        let policy = MutableAutomationPolicy::new();
        let mut observation = fixture.observation("I_001");
        edit(&mut observation);
        fixture.seed(vec![observation.clone()], &disclosure_version());
        let triage = Arc::new(fixture.service(policy.clone()));
        run(&triage);
        let decisions = Arc::new(IssueDecisionService::new(
            fixture.workspace.clone(),
            fixture.store(),
            triage.clone(),
            IssuePolicyService::new(Arc::new(ValidationPolicy)),
        ));
        Self {
            fixture,
            policy,
            triage,
            decisions,
            source: Arc::new(FakeSource {
                read: Mutex::new(IssueSourceRead::Observed(Box::new(observation))),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            execution: Arc::new(FakeExecution::default()),
        }
    }
    fn coordinator(&self) -> IssueCoordinatorService {
        IssueCoordinatorService::new(
            self.fixture.workspace.clone(),
            self.fixture.store(),
            self.source.clone(),
            self.triage.clone(),
            self.decisions.clone(),
            IssuePolicyService::new(Arc::new(ValidationPolicy)),
            self.execution.clone(),
        )
    }
    fn tick(&self, enabled: bool) -> Result<Vec<IssueCoordinatorFact>> {
        run(&self.triage);
        let facts = Mutex::new(Vec::new());
        self.coordinator().tick(
            &IssueCollectionCancellation::new(),
            123,
            enabled,
            &|values| facts.lock().unwrap().extend(values),
        )?;
        Ok(facts.into_inner().unwrap())
    }
    fn decision(&self) -> StoredIssueDecision {
        page(&self.triage).items[0].open_decisions[0].clone()
    }
    fn answer(&self, option: &str) {
        let id = self.decision().request.id().clone();
        let preview = self
            .decisions
            .prepare_confirmation(id.as_str(), option)
            .unwrap();
        use crate::application::port::inbound::stored_command_port::{
            StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
        };
        let context = StoredCommandContext {
            human_confirmation: Some(
                VerifiedHumanConfirmation::from_verified(
                    preview.target,
                    format!("desktop-confirmation:{}", "e".repeat(32)),
                )
                .unwrap(),
            ),
            caller: Some("Desktop".into()),
            channel: "gateway".into(),
            at: "2026-11-21T00:00:00Z".into(),
        };
        self.decisions
            .answer(id.as_str(), option, None, &context)
            .unwrap();
    }
    fn work(&self) -> Option<IssueWorkCheckpoint> {
        self.fixture
            .store()
            .get_issue_work(&self.fixture.workspace, &self.fixture.reference("I_001"))
            .unwrap()
    }
}

#[test]
fn propose_opens_one_human_wait_decision_with_a_fixed_deadline_and_no_task() {
    let h = Harness::new();
    let facts = h.tick(false).unwrap();
    assert!(matches!(
        &facts[..],
        [IssueCoordinatorFact::Decision { .. }]
    ));
    let first = h.decision();
    assert_eq!(
        first.request.definition().kind,
        IssueDecisionKind::StartApproval
    );
    assert_eq!(
        first.request.definition().due_at.as_deref(),
        Some("1970-01-02T00:00:00.123Z")
    );
    assert_eq!(
        first.request.definition().on_timeout,
        IssueDecisionTimeout::Wait
    );
    h.tick(false).unwrap();
    assert_eq!(h.decision(), first);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn human_rejection_and_unverified_answer_do_not_create_work() {
    let h = Harness::new();
    h.tick(false).unwrap();
    use crate::application::port::inbound::stored_command_port::StoredCommandContext;
    let id = h.decision().request.id().clone();
    assert!(
        h.decisions
            .answer(
                id.as_str(),
                "start",
                None,
                &StoredCommandContext {
                    human_confirmation: None,
                    caller: Some("Desktop".into()),
                    channel: "gateway".into(),
                    at: "2026-11-21T00:00:00Z".into(),
                }
            )
            .is_err()
    );
    h.answer("reject");
    h.tick(false).unwrap();
    h.tick(false).unwrap();
    assert!(h.work().is_none());
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn approval_queues_once_across_restarts_and_uses_the_shared_approved_fields() {
    let h = Harness::new();
    h.tick(false).unwrap();
    h.answer("start");
    let facts = h.tick(false).unwrap();
    assert!(
        facts
            .iter()
            .any(|fact| matches!(fact, IssueCoordinatorFact::WorkSubmitted { .. }))
    );
    let work = h.work().unwrap();
    assert_eq!(work.task_ids, ["task-1"]);
    assert_eq!(
        work.blocked_reason,
        Some(IssueWorkBlockReason::ExecutionDisabled)
    );
    assert_eq!(work.order.acceptance, ["기능이 요구한 결과를 반환한다."]);
    assert_eq!(
        work.approved_basis.field_hash.as_ref(),
        Some(&work.order.field_hash)
    );
    assert!(!work.delivery_granted);
    assert_eq!(
        h.fixture
            .store()
            .get_issue(&h.fixture.workspace, &work.order.issue)
            .unwrap()
            .unwrap()
            .state(),
        IssueAutomationState::Queued
    );
    for _ in 0..3 {
        h.tick(false).unwrap();
    }
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        page(&h.triage).items[0]
            .work
            .as_ref()
            .unwrap()
            .blocked_reason,
        Some(IssueWorkBlockReason::ExecutionDisabled)
    );
}

#[test]
fn fresh_closed_unreadable_or_changed_fields_never_submit() {
    for scenario in 0..3 {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        let mut observation = h.fixture.observation("I_001");
        match scenario {
            0 => observation.remote_state = IssueRemoteState::Closed,
            1 => {
                *h.source.read.lock().unwrap() = IssueSourceRead::Unknown {
                    issue: observation.issue,
                    reason: IssueSourceFailure::ProviderUnavailable,
                };
                let facts = h.tick(true).unwrap();
                assert!(
                    facts
                        .iter()
                        .any(|fact| matches!(fact, IssueCoordinatorFact::Triaged(_)))
                );
                assert!(h.work().is_none());
                continue;
            }
            _ => {
                if let IssueSourceContent::Shared(snapshot) = &mut observation.content {
                    snapshot.shared_copy.title = "변경된 제목".into();
                    snapshot.basis.field_hash = IssueFieldHash::of_fields(
                        &snapshot.shared_copy.title,
                        &snapshot.shared_copy.body,
                    );
                }
            }
        }
        *h.source.read.lock().unwrap() = IssueSourceRead::Observed(Box::new(observation));
        let facts = h.tick(true).unwrap();
        assert!(
            facts
                .iter()
                .any(|fact| matches!(fact, IssueCoordinatorFact::Triaged(_)))
        );
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    }
}

#[test]
fn crash_after_registration_or_grant_recovers_without_a_second_task() {
    for after_grant in [false, true] {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        {
            let mut execution = h.execution.state.lock().unwrap();
            execution.fail_after_register = !after_grant;
            execution.fail_after_grant = after_grant;
        }
        assert!(h.tick(true).is_err());
        assert!(h.work().is_some(), "intent must precede the executor");
        h.tick(true).unwrap();
        let work = h.work().unwrap();
        assert_eq!(work.task_ids, ["task-1"]);
        assert_eq!(work.permit_source.as_deref(), Some("a".repeat(40).as_str()));
        assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
        assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    }
}

#[test]
fn delivery_waits_for_host_frozen_source_then_revalidates_and_raises_revision() {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    let dispatch = h.work().unwrap();
    h.execution.state.lock().unwrap().frozen = Some("b".repeat(40));
    let facts = h.tick(true).unwrap();
    let work = h.work().unwrap();
    assert!(work.delivery_granted);
    assert_eq!(work.permit_source.as_deref(), Some("b".repeat(40).as_str()));
    assert_eq!(work.permit_revision, dispatch.permit_revision + 1);
    assert!(facts.iter().any(|fact| matches!(
        fact,
        IssueCoordinatorFact::WorkPermitGranted { delivery: true, .. }
    )));
    // 이후 branch head가 바뀌어도 이미 동결한 전달 결과를 다시 승인하지 않는다.
    h.execution.state.lock().unwrap().dispatch_source = Some("c".repeat(40));
    h.tick(true).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 2);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn stale_approval_revokes_without_reissuing_same_source_or_spawning_again() {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    *h.policy.text.lock().unwrap() =
        AUTO_POLICY.replace("mode = \"propose\"", "mode = \"observe\"");
    h.tick(true).unwrap();
    assert!(h.execution.state.lock().unwrap().revoked);
    assert_eq!(
        h.work().unwrap().blocked_reason,
        Some(IssueWorkBlockReason::ApprovalChanged)
    );
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn reducer_table_covers_observe_propose_auto_flags_and_decisions() {
    let h = Harness::new();
    let item = page(&h.triage).items.pop().unwrap();
    let assessment = item.triage.unwrap().assessment;
    let basis =
        IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None).unwrap();
    let open = IssueDecisionRequest::new(
        start_decision_id(&item.issue, &basis),
        start_definition(&item.issue, basis).unwrap(),
    )
    .unwrap();
    for (state, flags, decision, expected) in [
        (
            IssueAutomationState::Discovered,
            Default::default(),
            None,
            IssueCoordinatorAction::OpenDecision,
        ),
        (
            IssueAutomationState::AwaitingDecision,
            Default::default(),
            Some(&open),
            IssueCoordinatorAction::None,
        ),
        (
            IssueAutomationState::Discovered,
            crate::domain::issue_automation::IssueAutomationFlags {
                held: true,
                ..Default::default()
            },
            None,
            IssueCoordinatorAction::None,
        ),
        (
            IssueAutomationState::Discovered,
            crate::domain::issue_automation::IssueAutomationFlags {
                unknown: true,
                ..Default::default()
            },
            None,
            IssueCoordinatorAction::None,
        ),
        (
            IssueAutomationState::Done,
            Default::default(),
            None,
            IssueCoordinatorAction::None,
        ),
    ] {
        for now_unix_ms in [0, u64::MAX] {
            assert_eq!(
                next_action(IssueCoordinatorInput {
                    authorization_epoch: 0,
                    state,
                    flags,
                    assessment: Some(&assessment),
                    decision,
                    work: None,
                    frozen_source: None,
                    now_unix_ms
                }),
                expected
            );
        }
    }
    // An unconfirmed Auto file is capped at Propose and still asks a human to start.
    for (mode, expected) in [
        ("observe", IssueCoordinatorAction::None),
        ("auto", IssueCoordinatorAction::OpenDecision),
    ] {
        *h.policy.text.lock().unwrap() = AUTO_POLICY.replace("propose", mode);
        run(&h.triage);
        let item = page(&h.triage).items.pop().unwrap();
        assert_eq!(
            next_action(IssueCoordinatorInput {
                authorization_epoch: 0,
                state: IssueAutomationState::Discovered,
                flags: Default::default(),
                assessment: item.triage.as_ref().map(|triage| &triage.assessment),
                decision: None,
                work: None,
                frozen_source: None,
                now_unix_ms: 123
            }),
            expected
        );
    }
}

#[test]
fn fresh_resync_does_not_consume_periodic_cursor_and_off_fences_reject() {
    let h = Harness::new();
    let store = h.fixture.store();
    let before = store.get_collection(&h.fixture.workspace).unwrap().unwrap();
    let issue = h.fixture.reference("I_001");
    let read = h.source.read.lock().unwrap().clone();
    store
        .apply_issue_resync(&h.fixture.workspace, before.fence(), &issue, &read)
        .unwrap();
    let after = store.get_collection(&h.fixture.workspace).unwrap().unwrap();
    assert_eq!(after.resync_cursor, before.resync_cursor);
    store
        .set_collection_mode(
            &h.fixture.workspace,
            Some(after.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    assert!(matches!(
        store
            .apply_issue_resync(&h.fixture.workspace, after.fence(), &issue, &read)
            .unwrap(),
        IssueCollectionApplyResult::Discarded { .. }
    ));
    h.coordinator()
        .tick(&IssueCollectionCancellation::new(), 1, true, &|_| {})
        .unwrap();
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn approved_work_queues_with_a_reason_when_dispatch_source_is_unavailable() {
    let h = Harness::new();
    h.tick(false).unwrap();
    h.answer("start");
    h.execution.state.lock().unwrap().source_unavailable = true;
    h.tick(false).unwrap();
    let work = h.work().unwrap();
    assert_eq!(work.task_ids, ["task-1"]);
    assert_eq!(
        work.blocked_reason,
        Some(IssueWorkBlockReason::SourceUnavailable)
    );
    assert!(work.permit_source.is_none());
    assert!(h.execution.state.lock().unwrap().grants.is_empty());
    h.execution.state.lock().unwrap().source_unavailable = false;
    h.tick(false).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert!(h.work().unwrap().permit_source.is_some());
}

#[test]
fn durable_outbox_replays_lost_callbacks_until_the_host_acknowledges() {
    let h = Harness::new();
    h.tick(false).unwrap();
    h.answer("start");
    h.tick(false).unwrap();
    let store = h.fixture.store();
    let pending = store
        .pending_issue_work_events(&h.fixture.workspace)
        .unwrap();
    assert_eq!(pending.len(), 2);
    assert!(matches!(
        pending[0].kind,
        IssueWorkEventKind::Submitted { .. }
    ));
    assert!(matches!(
        pending[1].kind,
        IssueWorkEventKind::PermitGranted { .. }
    ));
    // callback이 유실되고 process가 재시작되어도 같은 event identity를 반환한다.
    let replay = h.tick(false).unwrap();
    for event in &pending {
        assert!(replay.iter().any(|fact| match fact {
            IssueCoordinatorFact::WorkSubmitted { event_id, .. }
            | IssueCoordinatorFact::WorkPermitGranted { event_id, .. } =>
                event_id == &event.event_id,
            _ => false,
        }));
        h.coordinator().acknowledge_event(&event.event_id).unwrap();
    }
    assert!(
        store
            .pending_issue_work_events(&h.fixture.workspace)
            .unwrap()
            .is_empty()
    );
    assert!(h.tick(false).unwrap().is_empty());
}

#[test]
fn removing_selection_label_after_approval_prevents_submission() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = AUTO_POLICY.replace(
        "mode = \"all_open\"",
        "mode = \"labels_any\"
labels_any = [\"akra:ready\"]",
    );
    let mut observation = h.fixture.observation("I_001");
    let label = IssueSourceLabel {
        id: "L_ready".into(),
        name: "akra:ready".into(),
    };
    observation.labels = complete(vec![label.clone()]);
    observation.label_actors = complete(vec![IssueLabelAttribution {
        label,
        event_id: "E_ready".into(),
        actor: observation.author.clone(),
        created_at: "2026-11-21T00:00:00Z".into(),
    }]);
    h.fixture.seed(vec![observation], &disclosure_version());
    h.tick(false).unwrap();
    h.answer("start");
    // source는 최초 무라벨 관찰을 반환한다. 본문 해시는 같아도 선택 근거가 사라졌다.
    h.tick(false).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn grant_postcheck_revokes_on_changed_policy_or_unreadable_current_state() {
    for unavailable in [false, true] {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        let policy = h.policy.clone();
        h.execution.state.lock().unwrap().after_grant = Some(Box::new(move || {
            if unavailable {
                policy.unavailable.store(true, Ordering::Release);
            } else {
                *policy.text.lock().unwrap() =
                    AUTO_POLICY.replace("mode = \"propose\"", "mode = \"observe\"");
            }
        }));
        let result = h.tick(true);
        if unavailable {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        assert!(h.execution.state.lock().unwrap().revoked);
        let work = h.work().unwrap();
        assert!(work.permit_source.is_none());
        assert!(!work.delivery_granted);
        let pending = h
            .fixture
            .store()
            .pending_issue_work_events(&h.fixture.workspace)
            .unwrap();
        assert!(
            pending
                .iter()
                .all(|event| !matches!(event.kind, IssueWorkEventKind::PermitGranted { .. }))
        );
    }
}

#[test]
fn interrupted_registration_recovers_existing_task_links_even_after_approval_changes() {
    for registered in [false, true] {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        {
            let mut execution = h.execution.state.lock().unwrap();
            execution.fail_before_register = !registered;
            execution.fail_after_register = registered;
        }
        assert!(h.tick(true).is_err());
        assert!(h.work().unwrap().task_ids.is_empty());
        *h.policy.text.lock().unwrap() =
            AUTO_POLICY.replace("mode = \"propose\"", "mode = \"observe\"");
        // 복구는 원격 source 조회에 기대지 않는다.
        h.execution.state.lock().unwrap().source_unavailable = true;
        h.tick(false).unwrap();
        h.tick(false).unwrap();
        let work = h.work().unwrap();
        assert_eq!(work.task_ids.len(), usize::from(registered));
        assert!(work.permit_source.is_none());
        let state = h.execution.state.lock().unwrap();
        assert_eq!(state.registrations, usize::from(registered));
        assert!(state.grants.is_empty());
        assert!(state.revoked);
        let submitted = h
            .fixture
            .store()
            .pending_issue_work_events(&h.fixture.workspace)
            .unwrap()
            .into_iter()
            .filter(|event| matches!(event.kind, IssueWorkEventKind::Submitted { .. }))
            .count();
        assert_eq!(submitted, usize::from(registered));
    }
}

#[test]
fn intervention_release_requires_new_human_epoch_before_same_work_resumes() {
    use crate::application::port::inbound::issue_intervention_port::IssueInterventionPort;
    use crate::application::port::inbound::stored_command_port::StoredCommandContext;
    use crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering;
    use crate::domain::issue_automation::intervention::{
        IssueInterventionAction, IssueInterventionRequest,
    };
    let h = Harness::new();
    h.tick(true).unwrap();
    let initial_decision = h.decision().request.id().clone();
    h.answer("start");
    h.tick(true).unwrap();
    let original = h.work().unwrap();
    let service = crate::application::service::issue_intervention::IssueInterventionService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.execution.clone(),
        Arc::new(UnavailableIssueWorkSteering),
    );
    let context = StoredCommandContext {
        human_confirmation: None,
        caller: Some("test".into()),
        channel: "test".into(),
        at: "2026-11-21T00:00:00Z".into(),
    };
    let command = |id: &str, action| IssueInterventionRequest {
        command_id: id.into(),
        issue: h.fixture.reference("I_001"),
        action,
    };
    service
        .intervene(command("hold", IssueInterventionAction::Hold), &context)
        .unwrap();
    h.execution.state.lock().unwrap().frozen = Some("b".repeat(40));
    h.tick(true).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    service
        .intervene(
            command("release", IssueInterventionAction::Release),
            &context,
        )
        .unwrap();
    h.tick(true).unwrap();
    let renewed = h.decision();
    assert_ne!(renewed.request.id(), &initial_decision);
    assert_eq!(renewed.request.definition().basis.authorization_epoch, 2);
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    h.answer("start");
    h.tick(true).unwrap();
    let resumed = h.work().unwrap();
    assert_eq!(resumed.order, original.order);
    assert_eq!(resumed.decision_id, original.decision_id);
    assert_eq!(resumed.approved_basis, original.approved_basis);
    assert_eq!(resumed.current_authorization_basis().authorization_epoch, 2);
    assert_eq!(resumed.permit_authorization_epoch, 2);
    assert_eq!(resumed.task_ids, original.task_ids);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 2);
}

#[test]
fn intervention_remote_hold_label_revokes_work_without_restarting_it() {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    let mut observation = h.fixture.observation("I_001");
    observation.labels.items.push(IssueSourceLabel {
        id: "L_hold".into(),
        name: "akra:hold".into(),
    });
    h.fixture.seed(vec![observation], &disclosure_version());
    h.tick(true).unwrap();
    assert!(h.execution.state.lock().unwrap().control.held);
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    // Release must recheck the still-present remote hold instead of replaying its old receipt.
    let service = crate::application::service::issue_intervention::IssueInterventionService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.execution.clone(),
        Arc::new(crate::application::port::outbound::issue_work_steering_port::UnavailableIssueWorkSteering),
    );
    let request = crate::domain::issue_automation::intervention::IssueInterventionRequest {
        command_id: "release-while-remotely-held".into(),
        issue: h.fixture.reference("I_001"),
        action: crate::domain::issue_automation::intervention::IssueInterventionAction::Release,
    };
    crate::application::port::inbound::issue_intervention_port::IssueInterventionPort::intervene(
        &service,
        request,
        &crate::application::port::inbound::stored_command_port::StoredCommandContext {
            human_confirmation: None,
            caller: Some("operator".into()),
            channel: "test".into(),
            at: "2026-11-21T00:00:00Z".into(),
        },
    )
    .unwrap();
    assert!(!h.execution.state.lock().unwrap().control.held);
    h.tick(true).unwrap();
    assert!(h.execution.state.lock().unwrap().control.held);
    assert_eq!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control
            .authorization_epoch,
        3
    );
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
}

mod dispatch_source;
mod gate;
mod policy_revalidation;

mod autonomy;

mod execution_lifecycle;
mod execution_questions;
mod interactions;

mod feedback_deferral;

mod closure;

mod execution_detail;

mod arbiter;
mod auto_delivery;
mod deadlines;
mod delivery;
mod gated;

mod binding_pause;
mod jira;
