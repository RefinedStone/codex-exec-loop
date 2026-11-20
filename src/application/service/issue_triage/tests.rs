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
use crate::domain::issue_automation::source::{
    IssueContentWithheldReason, IssueDiscoverySelection, IssueMetadataCompleteness,
    IssueMetadataList, IssuePrPresence, IssueRemoteState, IssueSourceActor, IssueSourceContent,
    IssueSourceDisclosure, IssueSourceFailure, IssueSourceObservation, IssueSourceRepository,
};
use crate::domain::issue_automation::triage::IssueTriageSourceState;
use crate::domain::issue_automation::{
    IssueAutomationAutonomy, IssueAutomationBinding, IssueAutomationSnapshot, TrackedIssue,
};
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssuePolicy};
use crate::domain::issue_work::{IssueComparisonBasis, IssueProjectScope, IssueSharedCopy};

const AUTO_POLICY: &str = r#"
[binding.github]
mode = "observe"
[authority]
autonomy_cap = "auto"
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
            issue: self.reference(id),
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: complete(Vec::new()),
            assignees: complete(Vec::new()),
            author: Some(IssueSourceActor {
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

#[test]
fn automation_policy_change_and_blocked_read_are_immediately_stale_without_disclosure_reuse() {
    let fixture = Fixture::new();
    fixture.seed(vec![fixture.observation("I_001")], &disclosure_version());
    let source = MutableAutomationPolicy::new();
    let service = fixture.service(source.clone());
    let (first, facts) = run(&service);
    assert_eq!((first.inspected, first.changed, facts.len()), (1, 1, 1));
    let first_page = page(&service);
    let first_projection = fixture
        .store()
        .get_collection_projection(&fixture.workspace)
        .unwrap()
        .unwrap();
    assert_eq!(first_projection.catalog_revision, first_page.revision);
    let first_triage = first_page.items[0].triage.as_ref().unwrap();
    let first_disclosure = first_triage
        .assessment
        .basis()
        .disclosure_policy_version
        .clone();
    assert_eq!(
        first_page.report.as_ref().unwrap().assumed_mode,
        Some(IssueAutomationAutonomy::Auto)
    );

    *source.text.lock().unwrap() = format!(
        "{AUTO_POLICY}\n[[classify.rule]]\nclass = \"high\"\npaths_any = [\"src/**\"]\nlabels_any = [\"sensitive\"]\n"
    );
    let stale = page(&service);
    assert!(stale.items[0].triage.as_ref().unwrap().stale);
    assert_eq!(stale.report.as_ref().unwrap().stale, 1);
    assert_eq!(
        stale.revision, first_page.revision,
        "query must not persist its fresh computation"
    );
    let (changed, second_facts) = run(&service);
    assert_eq!((changed.changed, second_facts.len()), (1, 1));
    let second = page(&service);
    let second_projection = fixture
        .store()
        .get_collection_projection(&fixture.workspace)
        .unwrap()
        .unwrap();
    assert_eq!(
        second_projection.state, first_projection.state,
        "triage must not repurpose the collection CAS clock"
    );
    assert_eq!(second_projection.catalog_revision, second.revision);
    assert!(second_projection.catalog_revision > first_projection.catalog_revision);
    let second_triage = second.items[0].triage.as_ref().unwrap();
    assert!(!second_triage.stale);
    assert_eq!(
        second_triage.assessment.basis().disclosure_policy_version,
        first_disclosure
    );
    assert_ne!(
        second_triage.assessment.basis().automation_policy_version,
        first_triage.assessment.basis().automation_policy_version
    );
    assert!(
        second_triage
            .assessment
            .gates()
            .iter()
            .all(|gate| gate.gate_revision == 2)
    );

    source.unavailable.store(true, Ordering::Release);
    let blocked = page(&service);
    let report = blocked.report.as_ref().unwrap();
    assert!(report.policy_blocked);
    assert_eq!(
        (
            report.blocked,
            report.automatic_start,
            report.human_approval
        ),
        (1, 0, 0)
    );
    assert_eq!(report.assumed_mode, None);
    assert!(blocked.items[0].triage.as_ref().unwrap().stale);
    assert!(!format!("{blocked:?}").contains("UNSHARED_POLICY_SOURCE_ERROR"));
    let (blocked_run, events) = run(&service);
    assert!(blocked_run.policy_blocked);
    assert!(events.is_empty());
    assert_eq!(page(&service).revision, second.revision);
}

#[test]
fn current_withheld_unknown_and_legacy_rows_never_reuse_the_workflow_snapshot() {
    const HISTORICAL: &str = "HISTORICAL_SHARED_MARKER";
    let fixture = Fixture::new();
    let mut observed = fixture.observation("I_001");
    let IssueSourceContent::Shared(snapshot) = &mut observed.content else {
        unreachable!()
    };
    snapshot.shared_copy.body = HISTORICAL.into();
    fixture.seed(vec![observed.clone()], &disclosure_version());
    let service = fixture.service(MutableAutomationPolicy::new());
    run(&service);

    let state = fixture
        .store()
        .get_collection(&fixture.workspace)
        .unwrap()
        .unwrap();
    let binding = fixture
        .store()
        .get_binding(&fixture.workspace, state.binding_id.as_ref().unwrap())
        .unwrap()
        .unwrap()
        .binding;
    let IssueSourceContent::Shared(snapshot) = &observed.content else {
        unreachable!()
    };
    let legacy =
        TrackedIssue::discovered(fixture.reference("I_legacy"), &binding, snapshot.clone())
            .unwrap();
    fixture
        .store()
        .insert_issue(&fixture.workspace, &legacy)
        .unwrap();
    observed.content = IssueSourceContent::Withheld {
        basis: snapshot.basis.clone(),
        policy_version: disclosure_version(),
        reason: IssueContentWithheldReason::UserConfirmationRequired,
    };
    fixture.seed(vec![observed], &disclosure_version());
    run(&service);
    let current = page(&service);
    assert!(!format!("{current:?}").contains(HISTORICAL));
    let withheld = current
        .items
        .iter()
        .find(|item| item.issue.issue_id == "I_001")
        .unwrap();
    assert_eq!(
        withheld.triage.as_ref().unwrap().assessment.basis().source,
        IssueTriageSourceState::Withheld
    );
    let legacy = current
        .items
        .iter()
        .find(|item| item.issue.issue_id == "I_legacy")
        .unwrap();
    assert!(legacy.current.is_none());
    assert_eq!(
        legacy.triage.as_ref().unwrap().assessment.basis().source,
        IssueTriageSourceState::NotYetObserved
    );
    assert!(
        legacy
            .triage
            .as_ref()
            .unwrap()
            .assessment
            .basis()
            .field_hash
            .is_none()
    );

    let store = fixture.store();
    let state = store.get_collection(&fixture.workspace).unwrap().unwrap();
    let batch = store
        .read_resync_batch(&fixture.workspace, state.fence(), 1)
        .unwrap()
        .unwrap();
    assert_eq!(batch.targets, vec![fixture.reference("I_001")]);
    store
        .apply_resync_batch(
            &fixture.workspace,
            &batch,
            &[IssueSourceRead::Unknown {
                issue: fixture.reference("I_001"),
                reason: IssueSourceFailure::ProviderUnavailable,
            }],
        )
        .unwrap();
    run(&service);
    let unknown = service
        .issue(&fixture.reference("I_001"))
        .unwrap()
        .item
        .unwrap();
    let basis = unknown.triage.as_ref().unwrap().assessment.basis();
    assert_eq!(basis.source, IssueTriageSourceState::Unknown);
    assert!(basis.field_hash.is_none());
    assert!(basis.disclosure_policy_version.is_none());
    assert!(!format!("{unknown:?}").contains(HISTORICAL));
    assert!(
        store
            .get_issue(&fixture.workspace, &unknown.issue)
            .unwrap()
            .unwrap()
            .snapshot()
            .shared_copy
            .body
            .contains(HISTORICAL)
    );
}

#[test]
fn global_report_spans_more_than_one_database_page_and_is_independent_of_ui_pagination() {
    let fixture = Fixture::new();
    fixture.seed(
        (0..105)
            .map(|index| fixture.observation(&format!("I_{index:03}")))
            .collect(),
        &disclosure_version(),
    );
    let store = fixture.store();
    let mut other = fixture.reference("I_other");
    other.repository_id = "R_other".into();
    let binding = IssueAutomationBinding::new(
        IssueAutomationBindingId::new("other-binding").unwrap(),
        other.provider,
        other.server_id.clone(),
        other.repository_id.clone(),
        other.project_scope.clone(),
    )
    .unwrap();
    store.insert_binding(&fixture.workspace, &binding).unwrap();
    let IssueSourceContent::Shared(snapshot) = fixture.observation("I_template").content else {
        unreachable!()
    };
    store
        .insert_issue(
            &fixture.workspace,
            &TrackedIssue::discovered(other, &binding, snapshot).unwrap(),
        )
        .unwrap();
    let service = fixture.service(MutableAutomationPolicy::new());
    let (outcome, facts) = run(&service);
    assert_eq!(
        (outcome.inspected, outcome.changed, facts.len()),
        (106, 106, 106)
    );

    let first = service
        .issues(&IssueCollectionQuery {
            limit: 2,
            ..IssueCollectionQuery::default()
        })
        .unwrap();
    let second = service
        .issues(&IssueCollectionQuery {
            after: first.next_after.clone(),
            limit: 2,
            ..IssueCollectionQuery::default()
        })
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert_eq!(second.items.len(), 2);
    assert_eq!(first.revision, second.revision);
    let projection = store
        .get_collection_projection(&fixture.workspace)
        .unwrap()
        .unwrap();
    assert_eq!(projection.catalog_revision, first.revision);
    assert!(
        projection.catalog_revision > projection.state.revision + 100,
        "many triage commits advance the public clock without advancing collection CAS"
    );
    assert!(projection.binding.is_some());
    assert_eq!(
        projection.binding.as_ref().map(|binding| binding.id()),
        projection.state.binding_id.as_ref()
    );
    assert_eq!(first.report, second.report);
    let report = first.report.as_ref().unwrap();
    assert_eq!(report.revision, first.revision);
    assert_eq!(
        (report.total, report.automatic_start, report.unknown),
        (106, 105, 1)
    );
    assert_eq!(service.report(Some(binding.id())).unwrap().total, 1);

    let detail = service.issue(&fixture.reference("I_104")).unwrap();
    assert!(detail.item.is_some());
    assert_eq!(detail.revision, first.revision);
    assert_eq!(detail.report, first.report);
    let missing = service.issue(&fixture.reference("I_missing")).unwrap();
    assert!(missing.item.is_none());
    assert_eq!(missing.revision, first.revision);
    let (repeated, facts) = run(&service);
    assert_eq!(repeated.changed, 0);
    assert!(facts.is_empty());
    assert_eq!(page(&service).revision, first.revision);
}

#[test]
fn cancelled_triage_does_not_change_catalog_or_emit_committed_facts() {
    let fixture = Fixture::new();
    fixture.seed(vec![fixture.observation("I_001")], &disclosure_version());
    let service = fixture.service(MutableAutomationPolicy::new());
    let before = page(&service);
    let cancel = IssueCollectionCancellation::new();
    cancel.cancel();
    let facts = Mutex::new(Vec::new());
    let outcome = service
        .triage_all(&cancel, &|committed| {
            facts.lock().unwrap().extend(committed)
        })
        .unwrap();
    assert!(outcome.discarded);
    assert_eq!(outcome.changed, 0);
    assert!(facts.into_inner().unwrap().is_empty());
    assert_eq!(page(&service), before);
}

#[test]
fn a_later_invalid_source_cannot_erase_an_earlier_committed_triage_fact() {
    let fixture = Fixture::new();
    fixture.seed(vec![fixture.observation("I_001")], &disclosure_version());
    let mut invalid = fixture.observation("I_002");
    let IssueSourceContent::Shared(snapshot) = &mut invalid.content else {
        unreachable!()
    };
    snapshot.policy_version = "legacy-policy-not-a-current-sha".into();
    fixture.seed(vec![invalid], "legacy-policy-not-a-current-sha");
    let service = fixture.service(MutableAutomationPolicy::new());
    let facts = Mutex::new(Vec::new());
    let result = service.triage_all(&IssueCollectionCancellation::new(), &|committed| {
        facts.lock().unwrap().extend(committed);
    });
    assert!(result.is_err());
    let facts = facts.into_inner().unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].issue, fixture.reference("I_001"));
    let current = page(&service);
    assert!(current.items[0].triage.is_some());
    assert!(current.items[1].triage.is_none());
    assert_eq!(facts[0].revision, current.revision);
    assert_eq!(current.report.as_ref().unwrap().untriaged, 1);
}
