//! 실제 수집·SQLite·판정·커널 조회를 가짜 외부 이슈에 연결한다.
use super::*;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::adapter::inbound::kernel_protocol::{IssueState, IssuesState};
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_collection_origin_port::{
    IssueCollectionOrigin, IssueCollectionOriginPort,
};
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget, IssueResyncRequest,
    IssueSourceCapabilities, IssueSourcePort,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::{
    IssueCollectionCancellation, IssueCollectionService,
};
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_triage::IssueTriageService;
use crate::composition::native_client_runtime::KernelClientCommand;
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::collection::{
    IssueCollectionDetail, IssueCollectionPage, IssueCollectionQuery,
};
use crate::domain::issue_automation::source::{
    IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence, IssueRemoteState,
    IssueSourceActor, IssueSourceContent, IssueSourceDisclosure, IssueSourceFailure,
    IssueSourceObservation, IssueSourceRead, IssueSourceRepository,
};
use crate::domain::issue_automation::triage::{IssueTriageEligibility, IssueTriageReadiness};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};

const WAIT: Duration = Duration::from_secs(10);
const TITLE: &str = "제어 가능한 판정 회귀";
const READY_BODY: &str = "## 완료 기준\n- [ ] 결과를 다시 확인할 수 있다\n";
const CHANGED_BODY: &str = "## 설명\n완료 기준을 보완해야 한다\n";
const AUTOMATION_POLICY: &str = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "auto"
[select]
mode = "all_open"
[[select.trusted_actors]]
provider = "github"
server_id = "github.com"
actor_id = "U_triage_trusted"
[gates.auto.normal]
start = "auto"
"#;

struct TriageFixture {
    root: PathBuf,
    workspace: String,
    runtime: PathBuf,
}

impl TriageFixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-triage-kernel-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).expect("private fixture");
        let root = fs::canonicalize(root).expect("canonical fixture");
        let workspace = root.join("workspace");
        let runtime = root.join("runtime");
        crate::private_directory::ensure_private_directory(&workspace).expect("private workspace");
        crate::private_directory::ensure_private_directory(&runtime).expect("private runtime");
        Self {
            root,
            workspace: workspace.to_str().expect("UTF-8 fixture").to_string(),
            runtime,
        }
    }

    fn service(&self, source: Arc<SimIssueSource>) -> Arc<IssueCollectionService> {
        let runtime = self.runtime.clone();
        let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(runtime.clone())
        }));
        let triage = Arc::new(IssueTriageService::new(
            self.workspace.clone(),
            store.clone(),
            IssueAutomationPolicyService::new(Arc::new(SimAutomationPolicy)),
            IssuePolicyService::new(Arc::new(SimDisclosurePolicy)),
        ));
        Arc::new(
            IssueCollectionService::new(
                self.workspace.clone(),
                store,
                source,
                Arc::new(SimIssueOrigin),
                IssuePolicyService::new(Arc::new(SimDisclosurePolicy)),
            )
            .with_triage(triage),
        )
    }
}

impl Drop for TriageFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct SimIssueOrigin;
impl IssueCollectionOriginPort for SimIssueOrigin {
    fn github_origin(&self, _: &str) -> anyhow::Result<IssueCollectionOrigin> {
        Ok(IssueCollectionOrigin {
            owner: "owner".to_string(),
            name: "repository".to_string(),
        })
    }
}

struct SimDisclosurePolicy;
impl IssuePolicySourcePort for SimDisclosurePolicy {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

struct SimAutomationPolicy;
impl IssueAutomationPolicySourcePort for SimAutomationPolicy {
    fn load_user_policy_toml(&self) -> anyhow::Result<Option<String>> {
        Ok(Some(AUTOMATION_POLICY.to_string()))
    }
    fn load_project_policy_toml(&self, _: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

struct SimIssueSource {
    body: Mutex<String>,
    calls: AtomicUsize,
}

impl SimIssueSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            body: Mutex::new(READY_BODY.to_string()),
            calls: AtomicUsize::new(0),
        })
    }

    fn observation(
        &self,
        issue: IssueReference,
        disclosure: &IssueSourceDisclosure,
    ) -> IssueSourceObservation {
        let body = self.body.lock().expect("body").clone();
        IssueSourceObservation {
            content_problems: vec![],
            jira: None,
            linear: None,
            repository_visibility:
                crate::domain::issue_automation::source::IssueRepositoryVisibility::Private,
            repository_name_with_owner: Some("owner/repository".into()),
            creation_provenance:
                crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
            issue,
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            assignees: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            author: Some(IssueSourceActor {
                kind: crate::domain::issue_automation::source::IssueSourceActorKind::UserAccount,
                id: "U_triage_trusted".to_string(),
                login: "display-only".to_string(),
            }),
            label_actors: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            linked_pull_request: IssuePrPresence::Absent,
            content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(TITLE, &body),
                    provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
                },
                shared_copy: IssueSharedCopy {
                    title: TITLE.to_string(),
                    body,
                },
                policy_version: disclosure.policy_version.clone(),
            }),
        }
    }
}

impl IssueSourcePort for SimIssueSource {
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
                server_id: "github.com".to_string(),
                repository_id: "R_triage_sim".to_string(),
                project_scope,
            },
        };
        let issue = IssueReference {
            provider: repository.provider,
            server_id: repository.server_id.clone(),
            repository_id: repository.repository_id.clone(),
            issue_id: "I_triage_sim".to_string(),
            project_scope: repository.project_scope.clone(),
        };
        Ok(IssueDiscoveryPage {
            jira_connection: None,
            jira_binding: None,
            linear: None,
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

fn page(sim: &mut SimKernel) -> IssueCollectionPage {
    let (reply, answer) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Issues {
        query: IssueCollectionQuery::default(),
        reply,
    });
    answer
        .recv_timeout(WAIT)
        .expect("page reply")
        .expect("page snapshot")
}

fn detail(sim: &mut SimKernel, issue: IssueReference) -> IssueCollectionDetail {
    let (reply, answer) = mpsc::channel();
    sim.host
        .handle_input(KernelHostInput::Issue { issue, reply });
    answer
        .recv_timeout(WAIT)
        .expect("detail reply")
        .expect("detail snapshot")
}

fn wait_for_assessment(
    sim: &mut SimKernel,
    now: Instant,
    readiness: IssueTriageReadiness,
    hash: &IssueFieldHash,
) -> IssueCollectionPage {
    let deadline = Instant::now() + WAIT;
    loop {
        sim.host.run_cycle(now, Duration::from_millis(10));
        let current = page(sim);
        let ready = current
            .items
            .first()
            .and_then(|item| item.triage.as_ref())
            .is_some_and(|triage| {
                !triage.stale
                    && triage.assessment.readiness() == readiness
                    && triage.assessment.basis().field_hash.as_ref() == Some(hash)
            });
        let finished = !sim
            .host
            .runtime
            .issue_collection_view(now)
            .expect("view")
            .collecting;
        if ready && finished {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "kernel did not publish the current assessment: {current:?}"
        );
    }
}

fn start_gate_revision(page: &IssueCollectionPage) -> u64 {
    page.items[0]
        .triage
        .as_ref()
        .expect("assessment")
        .assessment
        .gates()
        .iter()
        .find(|gate| {
            gate.gate == crate::domain::issue_automation::policy::IssueAutomationGate::Start
        })
        .expect("start gate")
        .gate_revision
}

fn sync(sim: &mut SimKernel) {
    assert!(matches!(
        sim.host
            .handle_command(KernelClientCommand::SyncIssues, None),
        KernelCommandOutcome::Handled { .. }
    ));
}

#[test]
fn kernel_simulation_collects_triages_queries_and_reassesses_changed_issue_without_turns() {
    let fixture = TriageFixture::new();
    let source = SimIssueSource::new();
    let service = fixture.service(source.clone());
    let mut sim = build_sim_kernel();
    let log = Arc::new(SimKernelLog::default());
    sim.host
        .runtime
        .attach_kernel_event_log(log.clone(), WORKSPACE);
    sim.host
        .runtime
        .attach_issue_collection(service.clone())
        .expect("attach collector and triage");
    let now = Instant::now() + FAKE_CLOCK_OFFSET;
    let original_hash = IssueFieldHash::of_fields(TITLE, READY_BODY);
    let first = wait_for_assessment(&mut sim, now, IssueTriageReadiness::Ready, &original_hash);
    assert_eq!(first.items.len(), 1);
    let first_triage = first.items[0].triage.as_ref().expect("first assessment");
    assert_eq!(
        first_triage.assessment.eligibility(),
        IssueTriageEligibility::Eligible
    );
    assert_eq!(
        first.items[0].tracked.as_ref().expect("tracked").state,
        crate::domain::issue_automation::IssueAutomationState::Discovered,
        "advisory triage does not advance the workflow"
    );
    let report = first.report.as_ref().expect("whole binding report");
    assert_eq!(
        (report.total, report.automatic_start, report.stale),
        (1, 1, 0)
    );
    assert_eq!(report.revision, first.revision);
    let projection = sim
        .host
        .runtime
        .issue_collection_view(now)
        .expect("public clock")
        .projection;
    assert_eq!(projection.catalog_revision, first.revision);
    assert!(
        projection.catalog_revision > projection.state.revision,
        "triage advances the public clock beyond control CAS"
    );
    let original_gate = start_gate_revision(&first);
    assert_eq!(original_gate, 1);
    let issue = first.items[0].issue.clone();
    let queried = detail(&mut sim, issue.clone());
    assert_eq!(queried.revision, first.revision);
    assert_eq!(queried.item.as_ref(), first.items.first());
    let calls = source.calls.load(Ordering::SeqCst);
    let first_wire =
        serde_json::to_string(&IssuesState::from_domain(&first)).expect("list protocol");
    let detail_wire =
        serde_json::to_string(&IssueState::from_domain(&queried)).expect("detail protocol");
    assert!(first_wire.contains("triage") && detail_wire.contains("triage"));
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        calls,
        "local queries do not call a provider"
    );
    assert!(
        sim.effects.take().is_empty(),
        "triage creates no model or execution effect"
    );

    // Stop after a real observation commit so the public query must show stale before the worker re-triages.
    *source.body.lock().expect("change provider body") = CHANGED_BODY.to_string();
    service
        .collect_once(&IssueCollectionCancellation::new(), 2_000)
        .expect("commit changed observation");
    let stale = page(&mut sim);
    assert!(
        stale.items[0]
            .triage
            .as_ref()
            .expect("old assessment")
            .stale
    );
    assert_eq!(stale.report.as_ref().expect("stale report").stale, 1);
    assert_eq!(
        stale.report.as_ref().expect("stale report").automatic_start,
        0
    );
    assert!(stale.revision > first.revision);

    sync(&mut sim);
    let changed_hash = IssueFieldHash::of_fields(TITLE, CHANGED_BODY);
    let changed = wait_for_assessment(
        &mut sim,
        now,
        IssueTriageReadiness::NeedsInfo,
        &changed_hash,
    );
    let changed_triage = changed.items[0].triage.as_ref().expect("new assessment");
    assert!(
        changed_triage
            .assessment
            .reasons()
            .iter()
            .any(|reason| reason.as_str() == "criteria_missing")
    );
    assert_eq!(start_gate_revision(&changed), original_gate + 1);
    assert_eq!(
        changed.report.as_ref().expect("changed report").needs_info,
        1
    );
    assert_eq!(
        changed
            .report
            .as_ref()
            .expect("changed report")
            .automatic_start,
        0
    );
    let changed_detail = detail(&mut sim, issue.clone());
    assert_eq!(changed_detail.item.as_ref(), changed.items.first());
    let wire =
        serde_json::to_string(&IssueState::from_domain(&changed_detail)).expect("reason protocol");
    assert!(wire.contains("criteria_missing") && wire.contains("summary") && wire.contains("next"));
    let assessment_revision = changed_triage.revision;

    // A repeated source poll has the same semantic basis and cannot mint another gate revision/event.
    sync(&mut sim);
    let repeated = wait_for_assessment(
        &mut sim,
        now,
        IssueTriageReadiness::NeedsInfo,
        &changed_hash,
    );
    assert_eq!(
        repeated.items[0]
            .triage
            .as_ref()
            .expect("same assessment")
            .revision,
        assessment_revision
    );
    assert_eq!(start_gate_revision(&repeated), original_gate + 1);
    let mut foreign = issue.clone();
    foreign.project_scope = IssueProjectScope {
        workspace_dir: fixture.root.join("foreign").to_string_lossy().into_owned(),
    };
    assert!(
        service.issue(&foreign).is_err(),
        "single-issue reads enforce the representative workspace"
    );
    assert!(
        !sim.host.runtime.has_remaining_work(),
        "observation advice is not executable work"
    );
    assert!(
        sim.effects.take().is_empty(),
        "sync and reassessment create no model effects"
    );

    drop(sim);
    let records = log.0.lock().expect("flushed audit log");
    let triaged: Vec<_> = records
        .iter()
        .filter(|record| record.event.kind() == "issue_triaged")
        .collect();
    assert_eq!(triaged.len(), 2, "only two changed assessments committed");
    assert!(matches!(
        &triaged[0].event,
        KernelEvent::IssueTriaged {
            gate_revision: 1,
            ..
        }
    ));
    assert!(matches!(
        &triaged[1].event,
        KernelEvent::IssueTriaged {
            gate_revision: 2,
            ..
        }
    ));
}
