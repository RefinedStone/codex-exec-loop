use super::*;
use crate::adapter::outbound::db::sqlite_issue_source_claims::SqliteIssueSourceClaimsAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_source_claim_port::IssueSourceClaimPort;
use crate::application::port::outbound::provider_credentials_port::{
    ProviderCredentialLease, ProviderCredentialReadPort,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::domain::issue_automation::policy::IssueAutomationJiraBinding;
use crate::domain::issue_automation::source::{
    IssueSourceJiraMetadata, JiraIssueStatusCategory, JiraPullRequestLookup,
};
use crate::domain::issue_automation::{IssueAutomationBinding, IssueAutomationBindingId};
use crate::domain::provider_credentials::{
    ConnectionConfiguration, ConnectionMetadata, ConnectionRef, ConnectionSummary,
    CredentialResult, JiraDeployment, Provider, ProviderConnectionStatus,
    ProviderCredentialCapabilities, ProviderCredentialError, ProviderCredentialState,
};
use std::sync::atomic::AtomicU64;

const JIRA_POLICY: &str = r#"
[binding.jira]
site_url = "https://example.atlassian.net"
cloud_id = "cloud-123"
project_id = "10001"
expected_account_id = "account-1"
jql = "statusCategory != Done"
mode = "observe"
[select]
mode = "all_open"
"#;

struct JiraPolicy {
    text: Mutex<String>,
}
impl Default for JiraPolicy {
    fn default() -> Self {
        Self {
            text: Mutex::new(JIRA_POLICY.into()),
        }
    }
}
impl IssueAutomationPolicySourcePort for JiraPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(Some(self.text.lock().unwrap().clone()))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(Some("[binding.jira]\n".into()))
    }
}

struct Credentials {
    revision: AtomicU64,
    state_reads: AtomicUsize,
    validations: AtomicUsize,
    metadata: Mutex<ConnectionMetadata>,
}
impl Credentials {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            revision: AtomicU64::new(1),
            state_reads: AtomicUsize::new(0),
            validations: AtomicUsize::new(0),
            metadata: Mutex::new(ConnectionMetadata {
                configuration: ConnectionConfiguration {
                    site_url: Some("https://example.atlassian.net".into()),
                    deployment: Some(JiraDeployment::Cloud),
                    email: Some("fixture@example.test".into()),
                },
                account_id: "account-1".into(),
                account_name: "Fixture Account".into(),
            }),
        })
    }
}
impl ProviderCredentialReadPort for Credentials {
    fn current_state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState> {
        self.state_reads.fetch_add(1, Ordering::SeqCst);
        if provider != Provider::Jira {
            return Err(ProviderCredentialError::Unsupported);
        }
        let reference = connection(self.revision.load(Ordering::SeqCst));
        Ok(ProviderCredentialState {
            profile_id: reference.profile_id.clone(),
            provider,
            // A later ledger operation does not itself replace the active connection.
            revision: reference.revision + 1,
            status: ProviderConnectionStatus::Connected,
            active_connection: Some(ConnectionSummary {
                reference,
                metadata: self.metadata.lock().unwrap().clone(),
            }),
            operation: None,
            capabilities: ProviderCredentialCapabilities {
                native_input: false,
                legacy_import: false,
                keyring: false,
            },
        })
    }
    fn acquire(&self, _: &ConnectionRef) -> CredentialResult<ProviderCredentialLease> {
        panic!("the collection test must never acquire a token")
    }
    fn validate_current(&self, expected: &ConnectionRef) -> CredentialResult<()> {
        self.validations.fetch_add(1, Ordering::SeqCst);
        if expected == &connection(self.revision.load(Ordering::SeqCst)) {
            Ok(())
        } else {
            Err(ProviderCredentialError::Conflict)
        }
    }
}
fn connection(revision: u64) -> ConnectionRef {
    ConnectionRef {
        profile_id: "a".repeat(64),
        provider: Provider::Jira,
        connection_id: "c".repeat(32),
        revision,
    }
}

fn jira_observation(
    issue: IssueReference,
    disclosure: &IssueSourceDisclosure,
) -> IssueSourceObservation {
    let mut value = observation(issue, disclosure);
    value.linked_pull_request = IssuePrPresence::Unknown;
    value.jira = Some(IssueSourceJiraMetadata {
        key: "AKRA-42".into(),
        url: "https://example.atlassian.net/browse/AKRA-42".into(),
        status_id: "10000".into(),
        status_category: JiraIssueStatusCategory::New,
        pull_request_lookup: JiraPullRequestLookup::Unsupported,
        connection: connection(1),
    });
    value
}

fn jira_page(request: &IssueDiscoveryRequest, next: Option<&str>) -> IssueDiscoveryPage {
    let mut value = page(request, &["10002"], next);
    value.observations[0] =
        jira_observation(value.observations[0].issue.clone(), &request.disclosure);
    value.jira_connection = Some(connection(1));
    value.jira_binding = Some(source_binding(
        Arc::new(JiraPolicy::default()),
        &value.repository.project_scope.workspace_dir,
    ));
    value
}
fn source_binding(policy: Arc<JiraPolicy>, workspace: &str) -> IssueAutomationJiraBinding {
    IssueAutomationPolicyService::new(policy)
        .resolve(workspace)
        .unwrap()
        .policy
        .binding
        .jira
        .unwrap()
}
fn jira_read(request: IssueResyncRequest) -> IssueSourceRead {
    IssueSourceRead::Observed(Box::new(jira_observation(
        request.issue,
        &request.disclosure,
    )))
}
fn claims(fixture: &Fixture) -> (PathBuf, Arc<SqliteIssueSourceClaimsAdapter>) {
    let directory = fixture.root.join("profile-claims");
    crate::private_directory::ensure_private_directory(&directory).unwrap();
    let runtime = directory.clone();
    (
        directory,
        Arc::new(SqliteIssueSourceClaimsAdapter::new(move || {
            Ok(runtime.clone())
        })),
    )
}
fn service(
    fixture: &Fixture,
    source: Arc<FakeSource>,
    credentials: Arc<Credentials>,
    claims: Arc<dyn IssueSourceClaimPort>,
) -> IssueCollectionService {
    service_with_policy(
        fixture,
        source,
        credentials,
        claims,
        Arc::new(JiraPolicy::default()),
    )
}
fn service_with_policy(
    fixture: &Fixture,
    source: Arc<FakeSource>,
    credentials: Arc<Credentials>,
    claims: Arc<dyn IssueSourceClaimPort>,
    policy: Arc<JiraPolicy>,
) -> IssueCollectionService {
    let origin = Arc::new(FakeOrigin::default());
    origin.unavailable.store(true, Ordering::SeqCst);
    fixture
        .service(source, origin, Arc::default())
        .with_connected_source_collection(
            IssueAutomationPolicyService::new(policy),
            Some(credentials),
            claims,
        )
}
fn expected_issue(fixture: &Fixture) -> IssueReference {
    let mut issue = reference(&fixture.repository(), "10002");
    issue.provider = IssueProvider::Jira;
    issue.server_id = "cloud-123".into();
    issue.repository_id = "10001".into();
    issue
}
fn assert_no_claim(fixture: &Fixture, claims: &SqliteIssueSourceClaimsAdapter) {
    let issue = expected_issue(fixture);
    let binding =
        IssueAutomationBinding::for_source(&IssueSourceRepository::from_reference(&issue)).unwrap();
    assert!(!claims.owns(&issue, binding.id()).unwrap());
}

#[test]
fn the_same_jira_issue_is_claimed_by_only_one_checkout_while_the_other_cursor_advances() {
    let first = Fixture::new();
    let second = Fixture::new();
    let (_, claims) = claims(&first);
    let source = FakeSource::new(
        |request| Ok(jira_page(&request, Some("next-page"))),
        jira_read,
    );
    let first_service = service(&first, source.clone(), Credentials::new(), claims.clone());
    let second_service = service(&second, source.clone(), Credentials::new(), claims.clone());
    let accepted = first_service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert_eq!(accepted.newly_tracked, [expected_issue(&first)]);
    let skipped = second_service
        .collect_once(&IssueCollectionCancellation::new(), 200)
        .unwrap();
    assert!(skipped.newly_tracked.is_empty());
    assert!(!skipped.discarded);
    let cursor = skipped
        .projection
        .state
        .discovery_cursor
        .as_ref()
        .expect("the duplicate issue still advances the remote page");
    let request = source.discoveries.lock().unwrap().last().unwrap().clone();
    let IssueDiscoveryTarget::Repository(repository) = &request.target else {
        unreachable!()
    };
    assert_eq!(
        cursor
            .validate_for(
                repository,
                &request.selection,
                request.page_size,
                &request.disclosure
            )
            .unwrap(),
        "next-page"
    );
    assert_eq!(query(&first_service).items.len(), 1);
    assert!(query(&second_service).items.is_empty());
    let owner = &query(&first_service).items[0].binding_id;
    assert!(claims.owns(&expected_issue(&first), owner).unwrap());
    assert_no_claim(&second, claims.as_ref());
}

#[test]
fn the_same_owner_retries_after_restart_without_new_claims_or_duplicate_rows() {
    let fixture = Fixture::new();
    let (_, claims) = claims(&fixture);
    let credentials = Credentials::new();
    let source = FakeSource::new(|request| Ok(jira_page(&request, None)), jira_read);
    let first = service(
        &fixture,
        source.clone(),
        credentials.clone(),
        claims.clone(),
    );
    assert_eq!(
        first
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .unwrap()
            .newly_tracked
            .len(),
        1
    );
    let owner = query(&first).items[0].binding_id.clone();
    drop(first);
    let restarted = service(&fixture, source, credentials, claims.clone());
    for now in [200, 300] {
        let result = restarted
            .collect_once(&IssueCollectionCancellation::new(), now)
            .unwrap();
        assert!(result.newly_tracked.is_empty());
        let page = query(&restarted);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].binding_id, owner);
        assert!(claims.owns(&page.items[0].issue, &owner).unwrap());
    }
}

#[test]
fn a_credential_revision_change_after_the_provider_response_claims_and_stores_nothing() {
    let fixture = Fixture::new();
    let (directory, claims) = claims(&fixture);
    let credentials = Credentials::new();
    let changed = credentials.clone();
    let source = FakeSource::new(
        move |request| {
            let result = jira_page(&request, None);
            changed.revision.store(2, Ordering::SeqCst);
            Ok(result)
        },
        |_| panic!("a rejected discovery must not add a resync target"),
    );
    let service = service(&fixture, source, credentials.clone(), claims.clone());
    assert!(
        service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .is_err()
    );
    assert!(credentials.state_reads.load(Ordering::SeqCst) > 0);
    assert!(query(&service).items.is_empty());
    assert_no_claim(&fixture, claims.as_ref());
    assert!(!directory.join("source-claims.db").exists());
}

#[test]
fn invalid_or_duplicate_jira_pages_are_checked_completely_before_the_first_global_claim() {
    for case in 0..3 {
        let fixture = Fixture::new();
        let (directory, claims) = claims(&fixture);
        let source = FakeSource::new(
            move |request| {
                let mut result = jira_page(&request, None);
                let mut invalid = result.observations[0].clone();
                if case != 0 {
                    invalid.issue.issue_id = "10003".into();
                }
                match case {
                    0 => {}
                    1 => invalid.issue.repository_id = "99999".into(),
                    2 => {
                        invalid.jira.as_mut().unwrap().status_category =
                            JiraIssueStatusCategory::Done
                    }
                    _ => unreachable!(),
                }
                result.observations.push(invalid);
                result.scanned_count = 2;
                Ok(result)
            },
            |_| panic!("an invalid page must add no resync target"),
        );
        let service = service(&fixture, source, Credentials::new(), claims.clone());
        assert!(
            service
                .collect_once(&IssueCollectionCancellation::new(), 100)
                .is_err(),
            "case {case}"
        );
        assert!(query(&service).items.is_empty(), "case {case}");
        assert_no_claim(&fixture, claims.as_ref());
        assert!(
            !directory.join("source-claims.db").exists(),
            "case {case}: even the first valid row must not claim"
        );
    }
}

struct ClaimHook {
    inner: Arc<SqliteIssueSourceClaimsAdapter>,
    calls: AtomicUsize,
    after_claim: Box<dyn Fn(usize) + Send + Sync>,
}
impl ClaimHook {
    fn new(
        inner: Arc<SqliteIssueSourceClaimsAdapter>,
        after_claim: impl Fn(usize) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner,
            calls: AtomicUsize::new(0),
            after_claim: Box::new(after_claim),
        })
    }
}
impl IssueSourceClaimPort for ClaimHook {
    fn claim(&self, issue: &IssueReference, binding: &IssueAutomationBindingId) -> Result<bool> {
        let owned = self.inner.claim(issue, binding)?;
        let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        (self.after_claim)(count);
        Ok(owned)
    }
    fn owns(&self, issue: &IssueReference, binding: &IssueAutomationBindingId) -> Result<bool> {
        self.inner.owns(issue, binding)
    }
}

fn assert_no_local_page(fixture: &Fixture, service: &IssueCollectionService) {
    assert!(query(service).items.is_empty());
    let projection = fixture
        .store()
        .get_collection_projection(&fixture.workspace)
        .unwrap()
        .expect("collection registration is retained after a rejected page");
    assert!(projection.state.discovery_cursor.is_none());
    assert!(projection.state.status.last_success_at_ms.is_none());
}

fn assert_last_claim_change_blocks_page(change_policy: bool) {
    let fixture = Fixture::new();
    let (_, claims) = claims(&fixture);
    let credentials = Credentials::new();
    let policy = Arc::new(JiraPolicy::default());
    let changed_credentials = credentials.clone();
    let changed_policy = policy.clone();
    let hooked = ClaimHook::new(claims.clone(), move |count| {
        if count == 2 {
            if change_policy {
                *changed_policy.text.lock().unwrap() =
                    JIRA_POLICY.replace("statusCategory != Done", "statusCategory = New");
            } else {
                changed_credentials.revision.store(2, Ordering::SeqCst);
            }
        }
    });
    let source = FakeSource::new(
        |request| {
            let mut result = jira_page(&request, Some("uncommitted-next-page"));
            let mut second = result.observations[0].clone();
            second.issue.issue_id = "10003".into();
            let jira = second.jira.as_mut().unwrap();
            jira.key = "AKRA-43".into();
            jira.url = "https://example.atlassian.net/browse/AKRA-43".into();
            result.observations.push(second);
            result.scanned_count = 2;
            Ok(result)
        },
        |_| panic!("a rejected page must not add any resync target"),
    );
    let service = service_with_policy(&fixture, source, credentials, hooked.clone(), policy);
    assert!(
        service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .is_err()
    );
    assert_eq!(hooked.calls.load(Ordering::SeqCst), 2);
    assert_no_local_page(&fixture, &service);
    // The completed profile claims remain retryable by this exact owner; only local
    // source rows and the page cursor are rejected by the final credential/policy fence.
    let mut issue = expected_issue(&fixture);
    let binding =
        IssueAutomationBinding::for_source(&IssueSourceRepository::from_reference(&issue)).unwrap();
    for id in ["10002", "10003"] {
        issue.issue_id = id.into();
        assert!(claims.owns(&issue, binding.id()).unwrap());
    }
}

#[test]
fn a_connection_change_during_the_last_global_claim_rejects_the_local_page_and_cursor() {
    assert_last_claim_change_blocks_page(false);
}

#[test]
fn a_policy_change_during_the_last_global_claim_rejects_the_local_page_and_cursor() {
    assert_last_claim_change_blocks_page(true);
}

#[test]
fn a_page_from_a_different_jql_is_rejected_after_the_policy_returns_to_its_original_value() {
    let fixture = Fixture::new();
    let (directory, claims) = claims(&fixture);
    let policy = Arc::new(JiraPolicy::default());
    let changed = policy.clone();
    let hooked = ClaimHook::new(claims.clone(), |_| {});
    let source = FakeSource::new(
        move |request| {
            let mut result = jira_page(&request, Some("uncommitted-next-page"));
            *changed.text.lock().unwrap() =
                JIRA_POLICY.replace("statusCategory != Done", "statusCategory = New");
            result.jira_binding = Some(source_binding(
                changed.clone(),
                &result.repository.project_scope.workspace_dir,
            ));
            *changed.text.lock().unwrap() = JIRA_POLICY.into();
            Ok(result)
        },
        |_| panic!("a page for another JQL must add no resync target"),
    );
    let service = service_with_policy(
        &fixture,
        source,
        Credentials::new(),
        hooked.clone(),
        policy.clone(),
    );
    assert!(
        service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .is_err()
    );
    assert_eq!(
        source_binding(policy, &fixture.workspace).jql,
        "statusCategory != Done"
    );
    assert_eq!(hooked.calls.load(Ordering::SeqCst), 0);
    assert_no_local_page(&fixture, &service);
    assert_no_claim(&fixture, claims.as_ref());
    assert!(!directory.join("source-claims.db").exists());
}

#[test]
fn an_empty_page_with_a_stale_connection_cannot_advance_the_cursor() {
    let fixture = Fixture::new();
    let (directory, claims) = claims(&fixture);
    let credentials = Credentials::new();
    let changed = credentials.clone();
    let hooked = ClaimHook::new(claims.clone(), |_| {});
    let source = FakeSource::new(
        move |request| {
            let mut result = jira_page(&request, Some("uncommitted-empty-page"));
            result.observations.clear();
            // A provider page can be entirely filtered while still advancing its cursor.
            assert_eq!(result.scanned_count, 1);
            changed.revision.store(2, Ordering::SeqCst);
            Ok(result)
        },
        |_| panic!("a rejected empty page must add no resync target"),
    );
    let service = service(&fixture, source, credentials.clone(), hooked.clone());
    assert!(
        service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .is_err()
    );
    assert!(credentials.state_reads.load(Ordering::SeqCst) > 0);
    assert_eq!(hooked.calls.load(Ordering::SeqCst), 0);
    assert_no_local_page(&fixture, &service);
    assert_no_claim(&fixture, claims.as_ref());
    assert!(!directory.join("source-claims.db").exists());
}

#[test]
fn the_current_account_or_site_must_match_policy_before_the_first_global_claim() {
    for case in 0..2 {
        let fixture = Fixture::new();
        let (directory, claims) = claims(&fixture);
        let credentials = Credentials::new();
        let changed = credentials.clone();
        let hooked = ClaimHook::new(claims.clone(), |_| {});
        let source = FakeSource::new(
            move |request| {
                let result = jira_page(&request, Some("uncommitted-next-page"));
                let mut metadata = changed.metadata.lock().unwrap();
                if case == 0 {
                    metadata.account_id = "another-account".into();
                } else {
                    metadata.configuration.site_url = Some("https://another.atlassian.net".into());
                }
                Ok(result)
            },
            |_| panic!("a page from a different account or site must not add a resync target"),
        );
        let service = service(&fixture, source, credentials.clone(), hooked.clone());
        assert!(
            service
                .collect_once(&IssueCollectionCancellation::new(), 100)
                .is_err(),
            "case {case}"
        );
        assert!(credentials.state_reads.load(Ordering::SeqCst) > 0);
        assert_eq!(credentials.revision.load(Ordering::SeqCst), 1);
        assert_eq!(hooked.calls.load(Ordering::SeqCst), 0);
        assert_no_local_page(&fixture, &service);
        assert_no_claim(&fixture, claims.as_ref());
        assert!(!directory.join("source-claims.db").exists(), "case {case}");
    }
}

#[test]
fn a_connection_change_after_resync_records_failure_and_preserves_the_last_shared_snapshot() {
    let fixture = Fixture::new();
    let (_, claims) = claims(&fixture);
    let credentials = Credentials::new();
    let changed = credentials.clone();
    let rotate = Arc::new(AtomicBool::new(false));
    let after_read = rotate.clone();
    let source = FakeSource::new(
        |request| Ok(jira_page(&request, None)),
        move |request| {
            let mut read = jira_read(request);
            if after_read.load(Ordering::SeqCst) {
                let IssueSourceRead::Observed(observation) = &mut read else {
                    unreachable!()
                };
                let IssueSourceContent::Shared(snapshot) = &mut observation.content else {
                    unreachable!()
                };
                snapshot.shared_copy.body =
                    "A later observation from the replaced connection".into();
                snapshot.basis.field_hash = IssueFieldHash::of_fields(
                    &snapshot.shared_copy.title,
                    &snapshot.shared_copy.body,
                );
                snapshot.basis.provider_updated_at = "2026-11-21T00:00:00Z".into();
                changed.revision.store(2, Ordering::SeqCst);
            }
            read
        },
    );
    let service = service(
        &fixture,
        source.clone(),
        credentials.clone(),
        claims.clone(),
    );
    let first = service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert_eq!(first.projection.state.status.last_success_at_ms, Some(100));
    let issue = expected_issue(&fixture);
    let original = fixture
        .store()
        .get_issue(&fixture.workspace, &issue)
        .unwrap()
        .unwrap();

    rotate.store(true, Ordering::SeqCst);
    let failed = service
        .collect_once(&IssueCollectionCancellation::new(), 200)
        .unwrap();
    assert!(!failed.discarded);
    assert!(failed.newly_tracked.is_empty());
    let status = &failed.projection.state.status;
    assert_eq!(status.last_attempt_at_ms, Some(200));
    assert_eq!(status.last_success_at_ms, Some(100));
    assert_eq!(status.consecutive_failures, 1);
    assert_eq!(
        status.last_failure,
        Some(IssueCollectionFailure::Source(
            IssueSourceFailure::IdentityMismatch
        ))
    );
    assert_eq!(source.resyncs.lock().unwrap().len(), 2);
    assert_eq!(credentials.revision.load(Ordering::SeqCst), 2);
    let page = query(&service);
    assert_eq!(page.items.len(), 1);
    assert!(matches!(
        &page.items[0].current,
        Some(IssueSourceRead::Unknown {
            issue: current,
            reason: IssueSourceFailure::IdentityMismatch,
        }) if current == &issue
    ));
    assert!(page.items[0].tracked.as_ref().unwrap().flags.unknown);
    let stored = fixture
        .store()
        .get_issue(&fixture.workspace, &issue)
        .unwrap()
        .unwrap();
    assert_eq!(stored.snapshot(), original.snapshot());
    assert_eq!(stored.state(), original.state());
    assert!(stored.flags().unknown);
    assert!(claims.owns(&issue, original.binding_id()).unwrap());
}
