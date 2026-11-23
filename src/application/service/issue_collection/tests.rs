use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueSourceCapabilities,
};
use crate::domain::issue_automation::source::{
    IssueDiscoveryCursor, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueRemoteState, IssueSourceContent, IssueSourceObservation,
};
use crate::domain::issue_automation::{IssueAutomationSnapshot, IssueAutomationState};
use crate::domain::issue_work::{IssueComparisonBasis, IssueFieldHash, IssueSharedCopy};

mod jira;

struct Fixture {
    root: PathBuf,
    workspace: String,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-collector-{}-{:016x}",
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

    fn service(
        &self,
        source: Arc<FakeSource>,
        origin: Arc<FakeOrigin>,
        policy: Arc<FakePolicy>,
    ) -> IssueCollectionService {
        IssueCollectionService::new(
            self.workspace.clone(),
            self.store(),
            source,
            origin,
            IssuePolicyService::new(policy),
        )
    }

    fn repository(&self) -> IssueSourceRepository {
        repository(&self.workspace)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct FakeOrigin {
    calls: AtomicUsize,
    unavailable: AtomicBool,
}

impl IssueCollectionOriginPort for FakeOrigin {
    fn github_origin(&self, _: &str) -> Result<IssueCollectionOrigin> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.unavailable.load(Ordering::SeqCst) {
            bail!("synthetic-private-origin-error");
        }
        Ok(IssueCollectionOrigin {
            owner: "owner".to_string(),
            name: "repository".to_string(),
        })
    }
}

#[derive(Default)]
struct FakePolicy {
    calls: AtomicUsize,
    text: Mutex<Option<String>>,
}

impl IssuePolicySourcePort for FakePolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }

    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.text.lock().unwrap().clone())
    }
}

type Discover =
    dyn Fn(IssueDiscoveryRequest) -> Result<IssueDiscoveryPage, IssueSourceFailure> + Send + Sync;
type Resync = dyn Fn(IssueResyncRequest) -> IssueSourceRead + Send + Sync;

struct FakeSource {
    discoveries: Mutex<Vec<IssueDiscoveryRequest>>,
    resyncs: Mutex<Vec<IssueResyncRequest>>,
    discover: Box<Discover>,
    resync: Box<Resync>,
}

impl FakeSource {
    fn new(
        discover: impl Fn(IssueDiscoveryRequest) -> Result<IssueDiscoveryPage, IssueSourceFailure>
        + Send
        + Sync
        + 'static,
        resync: impl Fn(IssueResyncRequest) -> IssueSourceRead + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            discoveries: Mutex::new(Vec::new()),
            resyncs: Mutex::new(Vec::new()),
            discover: Box::new(discover),
            resync: Box::new(resync),
        })
    }

    fn three() -> Arc<Self> {
        Self::new(
            |request| Ok(page(&request, &["I_1", "I_2", "I_3"], None)),
            open_read,
        )
    }
}

impl IssueSourcePort for FakeSource {
    fn capabilities(&self) -> IssueSourceCapabilities {
        IssueSourceCapabilities::read_only()
    }

    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        self.discoveries.lock().unwrap().push(request.clone());
        (self.discover)(request)
    }

    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        self.resyncs.lock().unwrap().push(request.clone());
        (self.resync)(request)
    }
}

fn repository(workspace: &str) -> IssueSourceRepository {
    IssueSourceRepository {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_1".to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: workspace.to_string(),
        },
    }
}

fn reference(repository: &IssueSourceRepository, id: &str) -> IssueReference {
    IssueReference {
        provider: repository.provider,
        server_id: repository.server_id.clone(),
        repository_id: repository.repository_id.clone(),
        issue_id: id.to_string(),
        project_scope: repository.project_scope.clone(),
    }
}

fn observation(
    issue: IssueReference,
    disclosure: &IssueSourceDisclosure,
) -> IssueSourceObservation {
    IssueSourceObservation {
        content_problems: vec![],
        jira: None,
        repository_visibility: Default::default(),
        repository_name_with_owner: None,
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
        author: None,
        label_actors: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields("synthetic title", "synthetic body"),
                provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
            },
            shared_copy: IssueSharedCopy {
                title: "Shared title".to_string(),
                body: "Shared body".to_string(),
            },
            policy_version: disclosure.policy_version.clone(),
        }),
    }
}

fn page(request: &IssueDiscoveryRequest, ids: &[&str], after: Option<&str>) -> IssueDiscoveryPage {
    let repository = match &request.target {
        IssueDiscoveryTarget::Repository(repository) => repository.clone(),
        IssueDiscoveryTarget::GitHubLocator { project_scope, .. } => {
            repository(&project_scope.workspace_dir)
        }
    };
    let observations = ids
        .iter()
        .map(|id| observation(reference(&repository, id), &request.disclosure))
        .collect();
    let next_cursor = after.map(|after| {
        IssueDiscoveryCursor::new(
            repository.clone(),
            request.selection.clone(),
            request.page_size,
            &request.disclosure,
            after,
        )
        .unwrap()
    });
    IssueDiscoveryPage {
        jira_connection: None,
        jira_binding: None,
        repository,
        observations,
        next_cursor,
        scanned_count: ids.len().try_into().unwrap(),
    }
}

fn open_read(request: IssueResyncRequest) -> IssueSourceRead {
    IssueSourceRead::Observed(Box::new(observation(request.issue, &request.disclosure)))
}

fn no_source() -> Arc<FakeSource> {
    FakeSource::new(
        |_| panic!("discovery must not be called"),
        |_| panic!("resync must not be called"),
    )
}

fn query(service: &IssueCollectionService) -> IssueCollectionPage {
    service.issues(&IssueCollectionQuery::default()).unwrap()
}

#[test]
fn three_issues_are_persisted_once_and_survive_a_service_restart() {
    let fixture = Fixture::new();
    let source = FakeSource::three();
    let service = fixture.service(source.clone(), Arc::default(), Arc::default());
    let first = service
        .collect_once(&IssueCollectionCancellation::new(), 1_000)
        .unwrap();
    assert_eq!(first.newly_tracked.len(), 3);
    assert!(!first.discarded);
    assert_eq!(
        first.projection.state.status.last_success_at_ms,
        Some(1_000)
    );
    assert_eq!(first.projection.binding.unwrap().repository_id(), "R_1");
    drop(service);

    let restarted = fixture.service(source.clone(), Arc::default(), Arc::default());
    let second = restarted
        .collect_once(&IssueCollectionCancellation::new(), 2_000)
        .unwrap();
    assert!(second.newly_tracked.is_empty());
    let result = query(&restarted);
    assert_eq!(result.items.len(), 3);
    assert!(result.next_after.is_none());
    for item in result.items {
        assert_eq!(
            item.tracked.unwrap().state,
            IssueAutomationState::Discovered
        );
        assert!(matches!(item.current, Some(IssueSourceRead::Observed(_))));
    }
    assert_eq!(source.discoveries.lock().unwrap().len(), 2);
    assert_eq!(source.resyncs.lock().unwrap().len(), 6);
}

#[test]
fn restart_resumes_the_committed_page_cursor_in_the_same_stable_repository() {
    let fixture = Fixture::new();
    let source = FakeSource::new(
        |request| {
            if let Some(cursor) = &request.cursor {
                let IssueDiscoveryTarget::Repository(repository) = &request.target else {
                    panic!("stable ID required")
                };
                assert_eq!(repository.repository_id, "R_1");
                assert_eq!(
                    cursor
                        .validate_for(
                            repository,
                            &request.selection,
                            request.page_size,
                            &request.disclosure,
                        )
                        .unwrap(),
                    "second-page"
                );
                Ok(page(&request, &["I_3"], None))
            } else {
                Ok(page(&request, &["I_1", "I_2"], Some("second-page")))
            }
        },
        open_read,
    );
    let service = fixture.service(source.clone(), Arc::default(), Arc::default());
    let first = service
        .collect_once(&IssueCollectionCancellation::new(), 1_000)
        .unwrap();
    assert_eq!(first.newly_tracked.len(), 2);
    assert!(first.projection.state.discovery_cursor.is_some());
    drop(service);
    let restarted = fixture.service(source.clone(), Arc::default(), Arc::default());
    let second = restarted
        .collect_once(&IssueCollectionCancellation::new(), 2_000)
        .unwrap();
    assert_eq!(
        second.newly_tracked,
        vec![reference(&fixture.repository(), "I_3")]
    );
    assert!(second.projection.state.discovery_cursor.is_none());
    assert_eq!(query(&restarted).items.len(), 3);
}

#[test]
fn a_cycle_reads_one_page_and_at_most_ten_resyncs_with_fair_persisted_progress() {
    let fixture = Fixture::new();
    let source = FakeSource::new(
        |request| {
            assert_eq!(request.page_size, 25);
            let ids = (1..=25).map(|i| format!("I_{i:02}")).collect::<Vec<_>>();
            Ok(page(
                &request,
                &ids.iter().map(String::as_str).collect::<Vec<_>>(),
                None,
            ))
        },
        open_read,
    );
    let service = fixture.service(source.clone(), Arc::default(), Arc::default());
    for time in 1..=3 {
        service
            .collect_once(&IssueCollectionCancellation::new(), time)
            .unwrap();
    }
    assert_eq!(source.discoveries.lock().unwrap().len(), 3);
    let reads = source.resyncs.lock().unwrap();
    assert_eq!(reads.len(), 25);
    let mut ids = reads
        .iter()
        .map(|read| read.issue.issue_id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 25);
    assert_eq!(query(&service).items.len(), 25);
}

#[test]
fn remote_closed_and_unknown_remain_distinct_without_workflow_transitions() {
    let fixture = Fixture::new();
    let source = FakeSource::new(
        |request| Ok(page(&request, &["I_1", "I_2", "I_3"], None)),
        |request| {
            if request.issue.issue_id == "I_2" {
                return IssueSourceRead::Unknown {
                    issue: request.issue,
                    reason: IssueSourceFailure::ProviderUnavailable,
                };
            }
            let mut observed = observation(request.issue, &request.disclosure);
            if observed.issue.issue_id == "I_1" {
                observed.remote_state = IssueRemoteState::Closed;
            }
            IssueSourceRead::Observed(Box::new(observed))
        },
    );
    let service = fixture.service(source, Arc::default(), Arc::default());
    let result = service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert_eq!(
        result.projection.state.status.last_failure,
        Some(IssueCollectionFailure::Source(
            IssueSourceFailure::ProviderUnavailable
        ))
    );
    let items = query(&service).items;
    for item in &items {
        assert_eq!(
            item.tracked.as_ref().unwrap().state,
            IssueAutomationState::Discovered
        );
        assert_eq!(
            item.tracked.as_ref().unwrap().flags.unknown,
            item.issue.issue_id == "I_2"
        );
    }
    assert!(
        matches!(&items[0].current, Some(IssueSourceRead::Observed(read)) if read.remote_state == IssueRemoteState::Closed)
    );
    assert!(matches!(
        &items[1].current,
        Some(IssueSourceRead::Unknown { .. })
    ));
    assert!(
        matches!(&items[2].current, Some(IssueSourceRead::Observed(read)) if read.remote_state == IssueRemoteState::Open)
    );
}

#[test]
fn off_is_persisted_without_an_intermediate_observe_or_any_source_policy_origin_read() {
    let fixture = Fixture::new();
    let source = no_source();
    let origin = Arc::new(FakeOrigin::default());
    let policy = Arc::new(FakePolicy::default());
    let service = fixture.service(source.clone(), origin.clone(), policy.clone());
    let changed = service.set_mode(IssueCollectionMode::Off).unwrap();
    assert_eq!(changed.previous, None);
    assert!(changed.changed);
    assert_eq!(changed.projection.state.epoch, 1);
    assert_eq!(changed.projection.state.revision, 1);
    let same = service.set_mode(IssueCollectionMode::Off).unwrap();
    assert!(!same.changed);
    drop(service);
    let restarted = fixture.service(source.clone(), origin.clone(), policy.clone());
    assert_eq!(
        restarted.projection().unwrap().state.config.mode,
        IssueCollectionMode::Off
    );
    assert!(
        restarted
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .unwrap()
            .discarded
    );
    assert_eq!(origin.calls.load(Ordering::SeqCst), 0);
    assert_eq!(policy.calls.load(Ordering::SeqCst), 0);
    assert!(source.discoveries.lock().unwrap().is_empty());
    assert!(source.resyncs.lock().unwrap().is_empty());
}

#[test]
fn off_during_discovery_rejects_the_entire_late_page_and_its_cursor() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let workspace = fixture.workspace.clone();
    let source = FakeSource::new(
        move |request| {
            let state = store.get_collection(&workspace).unwrap().unwrap();
            store
                .set_collection_mode(&workspace, Some(state.epoch), IssueCollectionMode::Off)
                .unwrap();
            Ok(page(
                &request,
                &["I_1", "I_2", "I_3"],
                Some("must-not-commit"),
            ))
        },
        |_| panic!("off must prevent resync"),
    );
    let service = fixture.service(source.clone(), Arc::default(), Arc::default());
    let result = service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert!(result.discarded);
    assert_eq!(
        result.projection.state.config.mode,
        IssueCollectionMode::Off
    );
    assert!(result.projection.state.discovery_cursor.is_none());
    assert!(result.newly_tracked.is_empty());
    assert!(query(&service).items.is_empty());
    assert_eq!(source.discoveries.lock().unwrap().len(), 1);
    assert!(source.resyncs.lock().unwrap().is_empty());
}

#[test]
fn off_between_resyncs_stops_further_reads_and_discards_the_partial_batch() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let workspace = fixture.workspace.clone();
    let source = FakeSource::new(
        |request| Ok(page(&request, &["I_1", "I_2", "I_3"], None)),
        move |request| {
            let state = store.get_collection(&workspace).unwrap().unwrap();
            store
                .set_collection_mode(&workspace, Some(state.epoch), IssueCollectionMode::Off)
                .unwrap();
            let mut observed = observation(request.issue, &request.disclosure);
            observed.remote_state = IssueRemoteState::Closed;
            IssueSourceRead::Observed(Box::new(observed))
        },
    );
    let service = fixture.service(source.clone(), Arc::default(), Arc::default());
    let facts = Mutex::new(Vec::new());
    let result = service
        .collect_once_with_committed(&IssueCollectionCancellation::new(), 100, &|committed| {
            facts.lock().unwrap().extend(committed);
        })
        .unwrap();
    assert!(result.discarded);
    assert_eq!(source.resyncs.lock().unwrap().len(), 1);
    assert_eq!(result.newly_tracked.len(), 3);
    assert_eq!(facts.lock().unwrap().len(), 3);
    assert!(facts.lock().unwrap().iter().all(|fact| fact.revision == 2));
    for item in query(&service).items {
        assert!(
            matches!(item.current, Some(IssueSourceRead::Observed(read)) if read.remote_state == IssueRemoteState::Open)
        );
    }
}

#[test]
fn cancellation_blocks_initial_and_late_commits_without_waiting_for_another_source_read() {
    let fixture = Fixture::new();
    let token = Arc::new(IssueCollectionCancellation::new());
    token.cancel();
    let service = fixture.service(no_source(), Arc::default(), Arc::default());
    assert!(service.collect_once(&token, 100).is_err());
    assert!(
        fixture
            .store()
            .get_collection(&fixture.workspace)
            .unwrap()
            .is_none()
    );

    let token = Arc::new(IssueCollectionCancellation::new());
    let trigger = token.clone();
    let source = FakeSource::new(
        move |request| {
            trigger.cancel();
            Ok(page(&request, &["I_1"], Some("cancelled-page")))
        },
        |_| panic!("cancelled worker must not resync"),
    );
    let service = fixture.service(source, Arc::default(), Arc::default());
    let result = service.collect_once(&token, 100).unwrap();
    assert!(result.discarded);
    assert!(result.newly_tracked.is_empty());
    assert!(result.projection.state.discovery_cursor.is_none());
    assert!(result.projection.state.status.last_attempt_at_ms.is_none());
    assert!(query(&service).items.is_empty());
}

#[test]
fn origin_and_policy_failures_have_closed_statuses_and_do_not_query_the_source() {
    let fixture = Fixture::new();
    let origin = Arc::new(FakeOrigin::default());
    let policy = Arc::new(FakePolicy::default());
    origin.unavailable.store(true, Ordering::SeqCst);
    let service = fixture.service(no_source(), origin.clone(), policy.clone());
    let origin_failure = service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert_eq!(
        origin_failure.projection.state.status.last_failure,
        Some(IssueCollectionFailure::OriginUnavailable)
    );
    assert_eq!(policy.calls.load(Ordering::SeqCst), 0);
    origin.unavailable.store(false, Ordering::SeqCst);
    *policy.text.lock().unwrap() = Some("synthetic-private-policy = [".to_string());
    let policy_failure = service
        .collect_once(&IssueCollectionCancellation::new(), 200)
        .unwrap();
    assert_eq!(
        policy_failure.projection.state.status.last_failure,
        Some(IssueCollectionFailure::PolicyUnavailable)
    );
    assert_eq!(
        policy_failure.projection.state.status.consecutive_failures,
        2
    );
    assert!(!format!("{policy_failure:?}").contains("synthetic-private"));
}

#[test]
fn a_changed_policy_restarts_discovery_before_reusing_a_cursor() {
    let fixture = Fixture::new();
    let source = FakeSource::new(
        |request| {
            assert!(request.cursor.is_none());
            Ok(page(&request, &["I_1"], Some("next-page")))
        },
        open_read,
    );
    let policy = Arc::new(FakePolicy::default());
    let service = fixture.service(source.clone(), Arc::default(), policy.clone());
    let first = service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    *policy.text.lock().unwrap() =
        Some("[convention]\nrequired_sections = [\"Changed section\"]\n".to_string());
    let second = service
        .collect_once(&IssueCollectionCancellation::new(), 200)
        .unwrap();
    assert!(second.projection.state.epoch > first.projection.state.epoch);
    let requests = source.discoveries.lock().unwrap();
    assert_ne!(
        requests[0].disclosure.policy_version,
        requests[1].disclosure.policy_version
    );
    assert!(matches!(&requests[1].target,
        IssueDiscoveryTarget::GitHubLocator { expected_repository_id: Some(id), .. } if id == "R_1"));
    assert!(second.newly_tracked.is_empty());
}

#[test]
fn discovery_failure_keeps_its_checkpoint_but_still_refreshes_known_ids() {
    let fixture = Fixture::new();
    let calls = AtomicUsize::new(0);
    let source = FakeSource::new(
        move |request| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(page(&request, &["I_1", "I_2", "I_3"], Some("next-page")))
            } else {
                Err(IssueSourceFailure::ProviderUnavailable)
            }
        },
        open_read,
    );
    let service = fixture.service(source.clone(), Arc::default(), Arc::default());
    let first = service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    let second = service
        .collect_once(&IssueCollectionCancellation::new(), 200)
        .unwrap();
    assert_eq!(
        first.projection.state.discovery_cursor,
        second.projection.state.discovery_cursor
    );
    assert_eq!(second.projection.state.status.last_success_at_ms, Some(100));
    assert_eq!(
        second.projection.state.status.last_failure,
        Some(IssueCollectionFailure::Source(
            IssueSourceFailure::ProviderUnavailable
        ))
    );
    assert_eq!(source.resyncs.lock().unwrap().len(), 6);
    assert_eq!(query(&service).items.len(), 3);
}

#[test]
fn a_mismatched_resync_identity_is_unknown_for_the_requested_issue() {
    let fixture = Fixture::new();
    let source = FakeSource::new(
        |request| Ok(page(&request, &["I_1"], None)),
        |request| {
            let mut issue = request.issue;
            issue.issue_id = "I_wrong".to_string();
            IssueSourceRead::Observed(Box::new(observation(issue, &request.disclosure)))
        },
    );
    let service = fixture.service(source, Arc::default(), Arc::default());
    service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    let result = query(&service);
    assert_eq!(result.items.len(), 1);
    assert!(matches!(&result.items[0].current,
        Some(IssueSourceRead::Unknown { issue, reason: IssueSourceFailure::IdentityMismatch }) if issue.issue_id == "I_1"));
}

#[test]
fn inaccessible_ledger_never_becomes_default_observe_or_a_provider_call() {
    let fixture = Fixture::new();
    let broken =
        SqliteIssueAutomationStoreAdapter::new(|_| bail!("synthetic-private-ledger-error"));
    let origin = Arc::new(FakeOrigin::default());
    let service = IssueCollectionService::new(
        fixture.workspace.clone(),
        Arc::new(broken),
        no_source(),
        origin.clone(),
        IssuePolicyService::new(Arc::new(FakePolicy::default())),
    );
    for error in [
        service.projection().unwrap_err(),
        service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .unwrap_err(),
        service.set_mode(IssueCollectionMode::Off).unwrap_err(),
    ] {
        assert_eq!(
            error.to_string(),
            "the issue collection ledger is unavailable"
        );
        assert!(!format!("{error:#}").contains("synthetic-private"));
    }
    assert_eq!(origin.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn resync_with_a_different_disclosure_policy_is_unknown_without_a_stale_shared_copy() {
    for withheld in [false, true] {
        let fixture = Fixture::new();
        let source = FakeSource::new(
            |request| Ok(page(&request, &["I_1"], None)),
            move |request| {
                let mut observed = observation(request.issue, &request.disclosure);
                let IssueSourceContent::Shared(mut snapshot) = observed.content else {
                    unreachable!()
                };
                observed.content = if withheld {
                    IssueSourceContent::Withheld {
                    basis: snapshot.basis, policy_version: "wrong-policy".to_string(),
                    reason: crate::domain::issue_automation::source::IssueContentWithheldReason::UserConfirmationRequired,
                }
                } else {
                    snapshot.policy_version = "wrong-policy".to_string();
                    IssueSourceContent::Shared(snapshot)
                };
                IssueSourceRead::Observed(Box::new(observed))
            },
        );
        let service = fixture.service(source, Arc::default(), Arc::default());
        let result = service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .unwrap();
        assert_eq!(
            result.projection.state.status.last_failure,
            Some(IssueCollectionFailure::Source(
                IssueSourceFailure::MalformedResponse
            ))
        );
        let item = query(&service).items.remove(0);
        assert!(item.tracked.unwrap().flags.unknown);
        assert!(matches!(
            item.current,
            Some(IssueSourceRead::Unknown {
                reason: IssueSourceFailure::MalformedResponse,
                ..
            })
        ));
    }
}

#[test]
fn committed_facts_are_delivered_before_a_later_local_store_failure() {
    let fixture = Fixture::new();
    let fail = Arc::new(AtomicBool::new(false));
    let resolver_fail = fail.clone();
    let runtime = fixture.runtime.clone();
    let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
        if resolver_fail.load(Ordering::SeqCst) {
            bail!("synthetic-later-store-failure");
        }
        Ok(runtime.clone())
    }));
    let source_fail = fail.clone();
    let source = FakeSource::new(
        |request| Ok(page(&request, &["I_1", "I_2", "I_3"], None)),
        move |request| {
            source_fail.store(true, Ordering::SeqCst);
            open_read(request)
        },
    );
    let service = IssueCollectionService::new(
        fixture.workspace.clone(),
        store,
        source,
        Arc::new(FakeOrigin::default()),
        IssuePolicyService::new(Arc::new(FakePolicy::default())),
    );
    let facts = Mutex::new(Vec::new());
    let error = service
        .collect_once_with_committed(&IssueCollectionCancellation::new(), 100, &|committed| {
            facts.lock().unwrap().extend(committed);
        })
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the issue collection ledger is unavailable"
    );
    let facts = facts.into_inner().unwrap();
    assert_eq!(facts.len(), 3);
    assert!(facts.iter().all(|fact| fact.revision == 2));
    assert!(
        facts
            .iter()
            .all(|fact| fact.binding_id == facts[0].binding_id)
    );
    fail.store(false, Ordering::SeqCst);
    assert_eq!(query(&service).items.len(), 3);
}

mod commit_projection;

fn mode_confirmation(
    service: &IssueCollectionService,
    mode: IssueCollectionMode,
) -> VerifiedHumanConfirmation {
    let preview = service.prepare_mode_confirmation(mode).unwrap();
    VerifiedHumanConfirmation::from_verified(
        preview.target,
        format!("desktop-confirmation:{}", "d".repeat(32)),
    )
    .unwrap()
}

#[test]
fn mode_confirmation_prepare_does_not_initialize_or_read_a_provider_policy_or_origin() {
    let fixture = Fixture::new();
    let source = no_source();
    let origin = Arc::new(FakeOrigin::default());
    let policy = Arc::new(FakePolicy::default());
    let service = fixture.service(source.clone(), origin.clone(), policy.clone());
    let preview = service
        .prepare_mode_confirmation(IssueCollectionMode::Observe)
        .unwrap();
    assert!(matches!(
        preview.target,
        IssueConfirmationTarget::Mode {
            from: None,
            epoch: None,
            revision: None,
            ..
        }
    ));
    assert!(
        fixture
            .store()
            .get_collection(&fixture.workspace)
            .unwrap()
            .is_none()
    );
    assert_eq!(origin.calls.load(Ordering::SeqCst), 0);
    assert_eq!(policy.calls.load(Ordering::SeqCst), 0);
    assert!(source.discoveries.lock().unwrap().is_empty());
    assert!(source.resyncs.lock().unwrap().is_empty());
}

#[test]
fn turning_observation_on_requires_confirmation_but_lowering_and_noop_do_not() {
    let fixture = Fixture::new();
    let service = fixture.service(
        no_source(),
        Arc::new(FakeOrigin::default()),
        Arc::new(FakePolicy::default()),
    );
    let off = service.set_mode(IssueCollectionMode::Off).unwrap();
    let error = service.set_mode(IssueCollectionMode::Observe).unwrap_err();
    assert_eq!(
        error.downcast_ref::<IssueConfirmationError>(),
        Some(&IssueConfirmationError::Required)
    );
    assert_eq!(service.projection().unwrap(), off.projection);
    let confirmation = mode_confirmation(&service, IssueCollectionMode::Observe);
    let on = service
        .set_mode_with_confirmation(IssueCollectionMode::Observe, Some(&confirmation))
        .unwrap();
    assert!(on.changed);
    assert_eq!(on.projection.state.epoch, off.projection.state.epoch + 1);
    assert_eq!(
        on.projection.state.revision,
        off.projection.state.revision + 1
    );
    let noop = service.set_mode(IssueCollectionMode::Observe).unwrap();
    assert!(!noop.changed);
    assert_eq!(noop.projection, on.projection);
    assert!(service.set_mode(IssueCollectionMode::Off).unwrap().changed);
}

#[test]
fn an_old_mode_confirmation_does_not_authorize_a_later_off_observe_off_cycle() {
    let fixture = Fixture::new();
    let service = fixture.service(
        no_source(),
        Arc::new(FakeOrigin::default()),
        Arc::new(FakePolicy::default()),
    );
    service.set_mode(IssueCollectionMode::Off).unwrap();
    let old = mode_confirmation(&service, IssueCollectionMode::Observe);
    service
        .set_mode_for_test(IssueCollectionMode::Observe)
        .unwrap();
    let off = service.set_mode(IssueCollectionMode::Off).unwrap();
    let error = service
        .set_mode_with_confirmation(IssueCollectionMode::Observe, Some(&old))
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<IssueConfirmationError>(),
        Some(&IssueConfirmationError::ScopeMismatch)
    );
    assert_eq!(service.projection().unwrap(), off.projection);
}

#[test]
fn a_mismatched_verified_mode_confirmation_is_not_silently_treated_as_an_agent_noop() {
    let fixture = Fixture::new();
    let service = fixture.service(
        no_source(),
        Arc::new(FakeOrigin::default()),
        Arc::new(FakePolicy::default()),
    );
    service.set_mode(IssueCollectionMode::Off).unwrap();
    let wrong_target = mode_confirmation(&service, IssueCollectionMode::Observe);
    let error = service
        .set_mode_with_confirmation(IssueCollectionMode::Off, Some(&wrong_target))
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<IssueConfirmationError>(),
        Some(&IssueConfirmationError::ScopeMismatch)
    );
    assert_eq!(
        service.projection().unwrap().state.config.mode,
        IssueCollectionMode::Off
    );
}

mod feedback_lease;
