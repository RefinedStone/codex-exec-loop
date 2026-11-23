//! 실제 장부·결정 서비스·저장 명령을 커널과 연결하고 외부 조회/효과만 가짜로 둔다.
use super::*;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::adapter::inbound::kernel_protocol::{
    CommandRequest, IssueDecisionChangedState, IssueState, KernelCommand as WireCommand,
    KernelCommandInput, KernelEvent as WireEvent,
};
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandOutcome, StoredCommandResult,
};
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueDecisionCommit,
};
use crate::application::port::outbound::issue_collection_origin_port::{
    IssueCollectionOrigin, IssueCollectionOriginPort,
};
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget, IssueResyncRequest,
    IssueSourceCapabilities, IssueSourcePort,
};
use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::{
    IssueCollectionCancellation, IssueCollectionService,
};
use crate::application::service::issue_decision::IssueDecisionService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_triage::IssueTriageService;
use crate::application::service::planning::PlanningAdminFacadeService;
use crate::application::service::stored_command::StoredCommandService;
use crate::composition::native_client_runtime::{KernelClientCommand, StoredCommandEnd};
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::collection::{
    IssueCollectionDetail, IssueCollectionMode, IssueCollectionQuery,
};
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProvider, IssueReference, IssueSharedCopy,
};
use crate::domain::kernel_event::{OperationReceipt, ReasonCode};

const WAIT: Duration = Duration::from_secs(10);
const READY_BODY: &str = "## 완료 기준\n- [ ] 답은 한 번만 적용된다\n";
const SECRET: &str = "ghp_SYNTHETIC_DECISION_SECRET_00000000";
const POLICY: &str = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[select]
mode = "all_open"
[[select.trusted_actors]]
provider = "github"
server_id = "github.com"
actor_id = "U_decision"
[gates.gated.normal]
start = "auto"
"#;

struct Fixture {
    root: PathBuf,
    workspace: String,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-decision-kernel-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let runtime = root.join("runtime");
        crate::private_directory::ensure_private_directory(&workspace).unwrap();
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        Self {
            workspace: workspace.to_str().unwrap().to_owned(),
            root,
            runtime,
        }
    }

    fn services(&self, source: Arc<Source>) -> Services {
        self.services_with_policy(source, POLICY)
    }

    fn services_with_policy(&self, source: Arc<Source>, policy: &'static str) -> Services {
        let runtime = self.runtime.clone();
        let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(runtime.clone())
        }));
        let resolved =
            crate::application::service::issue_automation_policy::resolve_issue_automation_policy(
                Some(policy),
                None,
            )
            .unwrap();
        if resolved.policy.binding.github.mode
            == crate::domain::issue_automation::IssueAutomationAutonomy::Gated
            && store
                .issue_autonomy_selection(&self.workspace)
                .unwrap()
                .requested_level
                .is_none()
        {
            // Durable fixture receipt for an earlier verified native selection.
            let intent = crate::domain::issue_automation::autonomy::IssueAutonomyIntent {
                command_id: "fixture-confirmed-gated".into(),
                level: crate::domain::issue_automation::autonomy::IssueAutonomyLevel::Gated,
                expected_revision: 0,
                policy_version: resolved.version,
            };
            store
                .begin_issue_autonomy(&self.workspace, &intent)
                .unwrap();
            store
                .finish_issue_autonomy(&self.workspace, &intent)
                .unwrap();
        }
        let triage = Arc::new(IssueTriageService::new(
            self.workspace.clone(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(AutomationPolicy(policy))),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        ));
        let decisions = Arc::new(IssueDecisionService::new(
            self.workspace.clone(),
            store.clone(),
            triage.clone(),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        ));
        let issues = Arc::new(
            IssueCollectionService::new(
                self.workspace.clone(),
                store.clone(),
                source,
                Arc::new(Origin),
                IssuePolicyService::new(Arc::new(DisclosurePolicy)),
            )
            .with_triage(triage.clone()),
        );
        Services {
            store,
            triage,
            issues,
            decisions,
        }
    }

    fn boot(&self, services: &Services, log: Arc<SimKernelLog>) -> SimKernel {
        let mut sim = build_sim_kernel();
        sim.host
            .runtime
            .attach_kernel_event_log(log, &self.workspace);
        let workspace = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let planning = test_planning_services(workspace.clone());
        let admin = PlanningAdminFacadeService::from_planning_with_authority(
            &self.workspace,
            planning,
            workspace,
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
        );
        let stored = StoredCommandService::new(admin)
            .with_issue_collection(services.issues.clone())
            .with_issue_decisions(services.decisions.clone());
        sim.host
            .runtime
            .attach_stored_commands(Arc::new(stored), &self.workspace);
        sim.host
            .runtime
            .attach_issue_collection(services.issues.clone())
            .unwrap();
        sim
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Services {
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    triage: Arc<IssueTriageService>,
    issues: Arc<IssueCollectionService>,
    decisions: Arc<IssueDecisionService>,
}

impl Services {
    fn seed(&self) {
        self.issues.projection().unwrap();
        self.issues
            .collect_once(&IssueCollectionCancellation::new(), 1_000)
            .unwrap();
        let triaged = self
            .triage
            .triage_all(&IssueCollectionCancellation::new(), &|_| {})
            .unwrap();
        assert_eq!(triaged.changed, 1);
        // 가짜 시계의 커널 주기가 별도 수집 worker를 시작하지 않게 한다.
        self.issues.set_mode(IssueCollectionMode::Off).unwrap();
    }

    fn definition(&self) -> IssueDecisionDefinition {
        let item = self
            .issues
            .issues(&IssueCollectionQuery::default())
            .unwrap()
            .items
            .remove(0);
        let basis = IssueDecisionBasis::from_assessment(
            &item.triage.unwrap().assessment,
            IssueAutomationGate::Start,
            None,
        )
        .unwrap();
        IssueDecisionDefinition {
            delivery_review: None,
            subject: IssueDecisionSubject::Issue(item.issue),
            gate: IssueAutomationGate::Start,
            kind: IssueDecisionKind::StartApproval,
            question: shared(&format!("공유 질문 token={SECRET}의 검토를 마쳤나요?")),
            options: vec![
                IssueDecisionOption {
                    id: "proceed".into(),
                    label: shared("진행"),
                    effect: shared(&format!("판단만 기록한다 token={SECRET}")),
                },
                IssueDecisionOption {
                    id: "defer".into(),
                    label: shared("보류"),
                    effect: shared("보류 의사를 기록한다"),
                },
            ],
            recommended: Some("defer".into()),
            basis,
            on_timeout: IssueDecisionTimeout::Wait,
            due_at: None,
            asked_by: "agent:simulation".into(),
            evidence: vec!["triage:current".into()],
        }
    }

    fn ensure(&self, sim: &mut SimKernel, id: &str) -> IssueDecisionCommit {
        let commit = self
            .decisions
            .ensure_request(IssueDecisionId::new(id).unwrap(), self.definition())
            .unwrap();
        if commit.changed {
            sim.host.runtime.record_issue_decision_for_test(
                &commit.record.request.fact(),
                commit.catalog_revision,
            );
        }
        commit
    }
}

fn shared(value: &str) -> IssueDecisionSharedText {
    IssueDecisionSharedText::new_shared(value).unwrap()
}

struct Origin;
impl IssueCollectionOriginPort for Origin {
    fn github_origin(&self, _: &str) -> anyhow::Result<IssueCollectionOrigin> {
        Ok(IssueCollectionOrigin {
            owner: "owner".into(),
            name: "decisions".into(),
        })
    }
}

struct DisclosurePolicy;
impl IssuePolicySourcePort for DisclosurePolicy {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

struct AutomationPolicy(&'static str);
impl IssueAutomationPolicySourcePort for AutomationPolicy {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        Ok(Some(self.0.into()))
    }
    fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

struct Source {
    body: Mutex<String>,
    calls: AtomicUsize,
}

impl Source {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            body: Mutex::new(READY_BODY.into()),
            calls: AtomicUsize::new(0),
        })
    }

    fn observation(
        &self,
        issue: IssueReference,
        disclosure: &IssueSourceDisclosure,
    ) -> IssueSourceObservation {
        let body = self.body.lock().unwrap().clone();
        IssueSourceObservation {
            content_problems: vec![],
            jira: None,
            repository_visibility: Default::default(),
            repository_name_with_owner: None,
            creation_provenance: IssueCreationProvenance::Unattributed,
            issue,
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: complete(Vec::new()),
            assignees: complete(Vec::new()),
            author: Some(IssueSourceActor {
                kind: IssueSourceActorKind::UserAccount,
                id: "U_decision".into(),
                login: "display-only".into(),
            }),
            label_actors: complete(Vec::new()),
            linked_pull_request: IssuePrPresence::Absent,
            content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields("결정 요청 회귀", &body),
                    provider_updated_at: "2026-11-20T00:00:00Z".into(),
                },
                shared_copy: IssueSharedCopy {
                    title: "결정 요청 회귀".into(),
                    body,
                },
                policy_version: disclosure.policy_version.clone(),
            }),
        }
    }
}

fn complete<T>(items: Vec<T>) -> IssueMetadataList<T> {
    IssueMetadataList {
        items,
        completeness: IssueMetadataCompleteness::Complete,
    }
}

impl IssueSourcePort for Source {
    fn capabilities(&self) -> IssueSourceCapabilities {
        IssueSourceCapabilities::read_only()
    }

    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let repository = match request.target {
            IssueDiscoveryTarget::Repository(repository) => repository,
            IssueDiscoveryTarget::GitHubLocator { project_scope, .. } => IssueSourceRepository {
                provider: IssueProvider::GitHub,
                server_id: "github.com".into(),
                repository_id: "R_decision_sim".into(),
                project_scope,
            },
        };
        let issue = IssueReference {
            provider: repository.provider,
            server_id: repository.server_id.clone(),
            repository_id: repository.repository_id.clone(),
            issue_id: "I_decision_sim".into(),
            project_scope: repository.project_scope.clone(),
        };
        Ok(IssueDiscoveryPage {
            jira_connection: None,
            jira_binding: None,
            repository,
            observations: vec![self.observation(issue, &request.disclosure)],
            next_cursor: None,
            scanned_count: 1,
        })
    }

    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        self.calls.fetch_add(1, Ordering::SeqCst);
        IssueSourceRead::Observed(Box::new(
            self.observation(request.issue, &request.disclosure),
        ))
    }
}

fn query(sim: &mut SimKernel, issue: &IssueReference) -> IssueCollectionDetail {
    let (reply, answer) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Issue {
        issue: issue.clone(),
        reply,
    });
    answer.recv_timeout(WAIT).unwrap().unwrap()
}

fn subscribe(sim: &mut SimKernel) -> Receiver<KernelHostEvent> {
    let (reply, answer) = mpsc::channel();
    sim.host
        .handle_input(KernelHostInput::AttachTransient { reply });
    answer.try_recv().unwrap().1
}

fn live_decisions(events: &Receiver<KernelHostEvent>) -> Vec<serde_json::Value> {
    events
        .try_iter()
        .filter_map(|event| match event {
            KernelHostEvent::IssueDecision {
                decision,
                revision,
                reason,
            } => {
                let wire = WireEvent::IssueDecision(IssueDecisionChangedState {
                    decision: *decision,
                    revision,
                    reason,
                });
                Some(serde_json::to_value(wire).unwrap())
            }
            _ => None,
        })
        .collect()
}

fn answer(
    sim: &mut SimKernel,
    workspace: &str,
    decision_id: &str,
    option: &str,
    note: Option<&str>,
    key: &str,
) -> KernelCommandOutcome {
    let request = CommandRequest {
        command: WireCommand::AnswerIssueDecision {
            decision_id: decision_id.into(),
            option: option.into(),
            note: note.map(str::to_owned),
        },
        idempotency_key: Some(key.into()),
    };
    let mut descriptor = request.descriptor().unwrap().unwrap();
    descriptor.caller = Some("Desktop".into());
    let KernelCommandInput::Stored(command) = request.command.into_input(workspace).unwrap() else {
        panic!("answer is a stored command");
    };
    let (reply, response) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Command {
        command: KernelClientCommand::Stored(Box::new(command)),
        descriptor: Some(descriptor),
        reply,
    });
    response
        .try_recv()
        .expect("decision replies without waiting for planning")
}

fn done(
    outcome: &KernelCommandOutcome,
) -> (
    &IssueDecisionFact,
    IssueDecisionOutcome,
    bool,
    u64,
    OperationReceipt,
) {
    let KernelCommandOutcome::Handled {
        operation: Some(receipt),
        stored:
            Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueDecisionResolved {
                    fact,
                    outcome,
                    changed,
                    catalog_revision,
                },
            ))),
        events,
    } = outcome
    else {
        panic!("expected a completed decision command: {outcome:?}");
    };
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AppEvent::PlanningRuntimeRefreshStarted { .. }))
    );
    (fact, *outcome, *changed, *catalog_revision, *receipt)
}

fn assert_no_execution_effects(sim: &mut SimKernel, fixture: &Fixture) {
    assert!(
        sim.effects.take().is_empty(),
        "decisions produce no model, planning or execution effect"
    );
    assert!(sim.host.held_stored_answers.is_empty());
    assert!(
        NoopPlanningTaskRepositoryPort
            .load_task_authority_snapshot(&fixture.workspace)
            .unwrap()
            .is_none()
    );
}

#[test]
fn kernel_simulation_issue_decision_answers_once_replays_and_publishes_matching_live_revisions() {
    let fixture = Fixture::new();
    let source = Source::new();
    let services = fixture.services(source.clone());
    services.seed();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log.clone());
    let events = subscribe(&mut sim);
    let commit = services.ensure(&mut sim, "decision-once");
    let repeated = services.ensure(&mut sim, "decision-same-definition");
    assert!(!repeated.changed);
    assert_eq!(repeated.record, commit.record);
    let issue = commit.record.request.issue();
    sim.host
        .run_cycle(Instant::now() + FAKE_CLOCK_OFFSET, Duration::ZERO);
    let opened = query(&mut sim, issue);
    let wire = serde_json::to_string(&IssueState::from_domain(&opened)).unwrap();
    assert!(!wire.contains(SECRET));
    assert!(wire.contains("open_decisions"));
    assert_eq!(opened.item.as_ref().unwrap().open_decisions.len(), 1);
    let requested = live_decisions(&events);
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0]["type"], "issue_decision");
    assert_eq!(requested[0]["revision"], opened.revision);

    let calls = source.calls.load(Ordering::SeqCst);
    let note = format!("검토를 마쳤다 token={SECRET}");
    let first = answer(
        &mut sim,
        &fixture.workspace,
        commit.record.request.id().as_str(),
        "proceed",
        Some(&note),
        "answer-once",
    );
    let (fact, outcome, changed, revision, receipt) = done(&first);
    assert_eq!(outcome, IssueDecisionOutcome::Answered);
    assert!(changed);
    assert_eq!(fact.state, IssueDecisionState::Answered);
    assert_eq!(fact.answer_by, Some(IssueDecisionAnswerBy::Agent));
    let closed = query(&mut sim, issue);
    assert_eq!(revision, closed.revision);
    assert!(closed.item.as_ref().unwrap().open_decisions.is_empty());
    let resolved = live_decisions(&events);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0]["type"], "issue_decision");
    assert_eq!(resolved[0]["revision"], closed.revision);
    assert!(!serde_json::to_string(&resolved).unwrap().contains(SECRET));

    let replay = answer(
        &mut sim,
        &fixture.workspace,
        commit.record.request.id().as_str(),
        "proceed",
        Some(&note),
        "answer-once",
    );
    assert!(
        matches!(replay, KernelCommandOutcome::Replayed(replayed) if replayed.op_id == receipt.op_id && replayed.replayed)
    );
    let reused = answer(
        &mut sim,
        &fixture.workspace,
        commit.record.request.id().as_str(),
        "defer",
        Some(&note),
        "answer-once",
    );
    assert_eq!(reused, KernelCommandOutcome::KeyReused);
    let second = answer(
        &mut sim,
        &fixture.workspace,
        commit.record.request.id().as_str(),
        "defer",
        None,
        "answer-second",
    );
    let (_, outcome, changed, repeated_revision, _) = done(&second);
    assert_eq!(outcome, IssueDecisionOutcome::AlreadyResolved);
    assert!(!changed);
    assert_eq!(repeated_revision, revision);
    assert!(live_decisions(&events).is_empty());
    assert_eq!(source.calls.load(Ordering::SeqCst), calls);
    let stored = services
        .store
        .get_issue_decision(&fixture.workspace, commit.record.request.id())
        .unwrap()
        .unwrap();
    let first_answer = stored.request.answer().unwrap();
    assert_eq!(first_answer.by, IssueDecisionAnswerBy::Agent);
    assert_eq!(first_answer.confirmation, None);
    assert_eq!(first_answer.caller.as_deref(), Some("Desktop"));
    assert_eq!(first_answer.option, "proceed");
    assert!(
        !first_answer
            .note
            .as_ref()
            .unwrap()
            .as_str()
            .contains(SECRET)
    );
    assert_no_execution_effects(&mut sim, &fixture);
    assert!(
        sim.host.runtime.has_remaining_work(),
        "the explicit Gated selection keeps the kernel alive without execution effects"
    );
    drop(sim);
    let records = log.0.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|entry| entry.event.kind() == "issue_decision_requested")
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|entry| entry.event.kind() == "issue_decision_resolved")
            .count(),
        1
    );
    assert!(!serde_json::to_string(&*records).unwrap().contains(SECRET));
}

#[test]
fn kernel_simulation_issue_decision_restart_preserves_open_request_first_answer_and_receipt() {
    let fixture = Fixture::new();
    let source = Source::new();
    let log = Arc::new(SimKernelLog::default());
    let services = fixture.services(source.clone());
    services.seed();
    let mut sim = fixture.boot(&services, log.clone());
    let initial = services.ensure(&mut sim, "decision-restart");
    let issue = initial.record.request.issue().clone();
    drop(sim);
    drop(services);

    let reopened = fixture.services(source.clone());
    let mut sim = fixture.boot(&reopened, log.clone());
    let current = query(&mut sim, &issue);
    assert_eq!(
        current.item.as_ref().unwrap().open_decisions,
        vec![initial.record.clone()]
    );
    let first = answer(
        &mut sim,
        &fixture.workspace,
        initial.record.request.id().as_str(),
        "proceed",
        None,
        "restart-answer",
    );
    let (_, outcome, changed, _, receipt) = done(&first);
    assert_eq!(outcome, IssueDecisionOutcome::Answered);
    assert!(changed);
    let saved = reopened
        .store
        .get_issue_decision(&fixture.workspace, initial.record.request.id())
        .unwrap()
        .unwrap();
    assert_no_execution_effects(&mut sim, &fixture);
    assert!(
        sim.host.runtime.has_remaining_work(),
        "the explicit Gated selection keeps the kernel alive without execution effects"
    );
    drop(sim);
    drop(reopened);

    let reopened = fixture.services(source);
    let mut sim = fixture.boot(&reopened, log.clone());
    let replay = answer(
        &mut sim,
        &fixture.workspace,
        initial.record.request.id().as_str(),
        "proceed",
        None,
        "restart-answer",
    );
    assert!(
        matches!(replay, KernelCommandOutcome::Replayed(replayed) if replayed.op_id == receipt.op_id && replayed.replayed)
    );
    assert_eq!(
        reopened
            .store
            .get_issue_decision(&fixture.workspace, initial.record.request.id())
            .unwrap()
            .unwrap(),
        saved
    );
    assert!(
        query(&mut sim, &issue)
            .item
            .unwrap()
            .open_decisions
            .is_empty()
    );
    assert_no_execution_effects(&mut sim, &fixture);
    assert!(
        sim.host.runtime.has_remaining_work(),
        "the explicit Gated selection keeps the kernel alive without execution effects"
    );
    drop(sim);
    let records = log.0.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|entry| entry.event.kind() == "issue_decision_resolved")
            .count(),
        1
    );
}

#[test]
fn kernel_simulation_issue_decision_supersedes_body_changes_before_retriage() {
    let fixture = Fixture::new();
    let source = Source::new();
    let services = fixture.services(source.clone());
    services.seed();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log.clone());
    let events = subscribe(&mut sim);
    let initial = services.ensure(&mut sim, "decision-stale");
    let issue = initial.record.request.issue();
    sim.host
        .run_cycle(Instant::now() + FAKE_CLOCK_OFFSET, Duration::ZERO);
    live_decisions(&events);
    let original = query(&mut sim, issue)
        .item
        .unwrap()
        .triage
        .unwrap()
        .assessment;

    *source.body.lock().unwrap() = "## 완료 기준\n- [ ] 새 완료 기준이다\n".into();
    services
        .issues
        .set_mode_for_test(IssueCollectionMode::Observe)
        .unwrap();
    services
        .issues
        .collect_once(&IssueCollectionCancellation::new(), 2_000)
        .unwrap();
    services.issues.set_mode(IssueCollectionMode::Off).unwrap();
    let stale = query(&mut sim, issue);
    let before_answer = stale.item.as_ref().unwrap().triage.as_ref().unwrap();
    assert!(before_answer.stale);
    assert_eq!(before_answer.assessment, original);

    let answer = answer(
        &mut sim,
        &fixture.workspace,
        initial.record.request.id().as_str(),
        "proceed",
        None,
        "stale-answer",
    );
    let (fact, outcome, changed, revision, _) = done(&answer);
    assert_eq!(outcome, IssueDecisionOutcome::Superseded);
    assert!(changed);
    assert_eq!(fact.state, IssueDecisionState::Superseded);
    assert_eq!(fact.answer_by, None);
    assert_eq!(fact.option, None);
    let current = query(&mut sim, issue);
    assert_eq!(current.revision, revision);
    assert!(current.item.as_ref().unwrap().open_decisions.is_empty());
    assert_eq!(current.item.unwrap().triage.unwrap().assessment, original);
    let changed = live_decisions(&events);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["type"], "issue_decision");
    assert_eq!(changed[0]["revision"], revision);
    assert_eq!(changed[0]["reason"], "decision_superseded");
    let stored = services
        .store
        .get_issue_decision(&fixture.workspace, initial.record.request.id())
        .unwrap()
        .unwrap();
    assert_eq!(stored.request.state(), IssueDecisionState::Superseded);
    assert!(stored.request.answer().is_none());
    assert_no_execution_effects(&mut sim, &fixture);
    assert!(
        sim.host.runtime.has_remaining_work(),
        "the explicit Gated selection keeps the kernel alive without execution effects"
    );
    drop(sim);
    let records = log.0.lock().unwrap();
    assert!(records.iter().any(|record| matches!(
        &record.event,
        KernelEvent::IssueDecisionResolved {
            reason: Some(ReasonCode::DecisionSuperseded),
            ..
        }
    )));
}

#[test]
fn kernel_simulation_issue_decision_answers_after_planning_failure_without_refresh_or_hold() {
    let fixture = Fixture::new();
    let services = fixture.services(Source::new());
    services.seed();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log);
    let events = subscribe(&mut sim);
    let initial = services.ensure(&mut sim, "decision-planning-failed");
    sim.host
        .run_cycle(Instant::now() + FAKE_CLOCK_OFFSET, Duration::ZERO);
    live_decisions(&events);
    sim.host
        .handle_input(KernelHostInput::Inputs(vec![CoreInput::Command(
            AppCommand::RefreshPlanningRuntime {
                workspace_directory: fixture.workspace.clone(),
            },
        )]));
    let effects = sim.effects.take();
    let correlation = effects
        .into_iter()
        .find_map(|effect| match effect {
            CoreEffect::LoadPlanningRuntime { correlation } => Some(correlation),
            _ => None,
        })
        .expect("fake executor receives the planning refresh");
    sim.host
        .handle_input(KernelHostInput::Inputs(vec![CoreInput::EffectCompleted(
            CoreEffectCompletion::PlanningRuntimeLoaded {
                correlation,
                result: Err("synthetic planning read failure".into()),
            },
        )]));
    let failed = events.try_iter().any(|event| {
        matches!(event,
        KernelHostEvent::Core(event)
            if matches!(*event, AppEvent::PlanningRuntimeRefreshed { result: Err(_), .. }))
    });
    assert!(failed, "the real kernel observed a planning failure");
    let result = answer(
        &mut sim,
        &fixture.workspace,
        initial.record.request.id().as_str(),
        "defer",
        None,
        "answer-after-planning-error",
    );
    let (_, outcome, changed, _, _) = done(&result);
    assert_eq!(outcome, IssueDecisionOutcome::Answered);
    assert!(changed);
    assert_eq!(live_decisions(&events).len(), 1);
    assert_no_execution_effects(&mut sim, &fixture);
    assert!(
        sim.host.runtime.has_remaining_work(),
        "the explicit Gated selection keeps the kernel alive without execution effects"
    );
}

mod deadlines;
mod human_confirmation;

#[test]
fn kernel_simulation_propose_coordinator_survives_reconnect_without_live_turns() {
    use crate::application::port::inbound::stored_command_port::{
        StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
    };
    use crate::application::port::outbound::issue_work_execution_port::{
        IssueWorkExecutionPort, IssueWorkExecutionSnapshot, IssueWorkPermitRequest,
    };
    use crate::application::service::issue_coordinator::IssueCoordinatorService;
    use crate::domain::issue_automation::coordinator::{IssueWorkBlockReason, IssueWorkOrder};

    #[derive(Default)]
    struct Execution {
        order: Mutex<Option<IssueWorkOrder>>,
        applied_limit: Mutex<Option<(String, u32)>>,
    }
    impl IssueWorkExecutionPort for Execution {
        fn execution_limit(
            &self,
            _: &str,
            work_order_id: &str,
            max_attempts: u32,
        ) -> anyhow::Result<()> {
            let order = self.order.lock().unwrap();
            assert_eq!(
                order
                    .as_ref()
                    .expect("work is registered before its cap")
                    .work_order_id,
                work_order_id
            );
            *self.applied_limit.lock().unwrap() = Some((work_order_id.into(), max_attempts));
            Ok(())
        }
        fn snapshot(&self, _: &str, _: &str) -> anyhow::Result<IssueWorkExecutionSnapshot> {
            Ok(IssueWorkExecutionSnapshot {
                dispatch_source: Some("a".repeat(40)),
                ..Default::default()
            })
        }
        fn submit(&self, _: &str, order: &IssueWorkOrder) -> anyhow::Result<Vec<String>> {
            let mut stored = self.order.lock().unwrap();
            if let Some(existing) = stored.as_ref() {
                assert_eq!(existing, order);
            } else {
                *stored = Some(order.clone());
            }
            Ok(vec!["fake-task-1".into()])
        }
        fn grant(&self, _: &str, request: &IssueWorkPermitRequest) -> anyhow::Result<()> {
            assert_eq!(
                *self.applied_limit.lock().unwrap(),
                Some((request.work_order_id.clone(), 2)),
                "the policy cap must reach the executor before a permit"
            );
            assert!(!request.delivery);
            Ok(())
        }
        fn revoke(&self, _: &str, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }
    const PROPOSE: &str = r#"
[binding.github]
mode = "propose"
[budget]
max_attempts = 2
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_decision"}]
"#;
    let fixture = Fixture::new();
    let source = Source::new();
    let mut services = fixture.services_with_policy(source.clone(), PROPOSE);
    services.seed();
    let execution = Arc::new(Execution::default());
    let coordinator = Arc::new(IssueCoordinatorService::new(
        fixture.workspace.clone(),
        services.store.clone(),
        source.clone(),
        services.triage.clone(),
        services.decisions.clone(),
        IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        execution.clone(),
    ));
    services.issues = Arc::new(
        IssueCollectionService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            source.clone(),
            Arc::new(Origin),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        )
        .with_triage(services.triage.clone())
        .with_coordinator(coordinator),
    );
    let state = services
        .store
        .get_collection(&fixture.workspace)
        .unwrap()
        .unwrap();
    services
        .store
        .set_collection_mode(
            &fixture.workspace,
            Some(state.epoch),
            IssueCollectionMode::Observe,
        )
        .unwrap();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log.clone());
    let deadline = Instant::now() + WAIT;
    let decision = loop {
        sim.host
            .run_cycle(Instant::now(), Duration::from_millis(20));
        let page = services
            .issues
            .issues(&IssueCollectionQuery::default())
            .unwrap();
        if let Some(decision) = page.items[0].open_decisions.first()
            && !sim
                .host
                .runtime
                .issue_collection_view(Instant::now())
                .unwrap()
                .collecting
        {
            break decision.clone();
        }
        assert!(
            Instant::now() < deadline,
            "host did not open the propose decision"
        );
    };
    assert!(execution.order.lock().unwrap().is_none());
    assert!(
        !sim.host.runtime.parallel_mode_enabled(),
        "Propose must not enable the repository's unrelated queued work"
    );
    let preview = services
        .decisions
        .prepare_confirmation(decision.request.id().as_str(), "start")
        .unwrap();
    let answer = services
        .decisions
        .answer(
            decision.request.id().as_str(),
            "start",
            None,
            &StoredCommandContext {
                human_confirmation: Some(
                    VerifiedHumanConfirmation::from_verified(
                        preview.target,
                        format!("desktop-confirmation:{}", "d".repeat(32)),
                    )
                    .unwrap(),
                ),
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-21T00:00:00Z".into(),
            },
        )
        .unwrap();
    sim.host.runtime.record_issue_decision_for_test(
        &answer.commit.record.request.fact(),
        answer.commit.catalog_revision,
    );
    let issue = decision.request.issue().clone();
    let deadline = Instant::now() + WAIT;
    loop {
        sim.host
            .run_cycle(Instant::now(), Duration::from_millis(20));
        let work = services
            .store
            .get_issue_work(&fixture.workspace, &issue)
            .unwrap();
        if work
            .as_ref()
            .is_some_and(|work| work.permit_source.is_some())
            && !sim
                .host
                .runtime
                .issue_collection_view(Instant::now())
                .unwrap()
                .collecting
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "host did not submit approved work"
        );
    }
    assert!(
        sim.effects.take().is_empty(),
        "the fake host must not start a model turn"
    );
    let work = services
        .store
        .get_issue_work(&fixture.workspace, &issue)
        .unwrap()
        .unwrap();
    assert_eq!(
        work.blocked_reason,
        Some(IssueWorkBlockReason::ExecutionDisabled)
    );
    assert!(
        !sim.host.runtime.parallel_mode_enabled(),
        "Start approval grants work authority without enabling the whole pool"
    );
    let first_order = execution.order.lock().unwrap().clone();
    drop(sim);
    let mut restarted = fixture.boot(&services, log.clone());
    let deadline = Instant::now() + WAIT;
    loop {
        restarted
            .host
            .run_cycle(Instant::now(), Duration::from_millis(20));
        if !restarted
            .host
            .runtime
            .issue_collection_view(Instant::now())
            .unwrap()
            .collecting
        {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    assert_eq!(*execution.order.lock().unwrap(), first_order);
    assert_eq!(
        *execution.applied_limit.lock().unwrap(),
        Some((work.order.work_order_id.clone(), 2)),
        "reconnect preserves the configured work-order cap"
    );
    assert!(restarted.effects.take().is_empty());
    assert!(
        !restarted.host.runtime.parallel_mode_enabled(),
        "reconnecting must not implicitly enable approved issue work"
    );
    drop(restarted);
    let records = log.0.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.event.kind() == "issue_work_submitted")
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.event.kind() == "issue_work_permit_granted")
            .count(),
        1
    );
}
