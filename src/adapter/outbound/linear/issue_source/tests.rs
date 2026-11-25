use super::*;
use crate::adapter::outbound::linear::test_support::*;
use crate::domain::issue_policy::IssueDisclosurePolicy;
use crate::domain::issue_work::{IssueProjectScope, IssueReference};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

const NOW: u64 = 1_795_000_000_000;

struct Fixture {
    adapter: LinearIssueSourceAdapter,
    credentials: Arc<Credentials>,
    api: Arc<Api>,
    now: Arc<AtomicU64>,
}
impl Fixture {
    fn new() -> Self {
        let credentials = Arc::new(Credentials::new());
        let api = Arc::new(Api::new(credentials.clone()));
        let now = Arc::new(AtomicU64::new(NOW));
        let clock = now.clone();
        let mut adapter =
            LinearIssueSourceAdapter::new(binding(), credentials.clone(), api.clone()).unwrap();
        adapter.clock = Arc::new(move || clock.load(Ordering::SeqCst));
        Self {
            adapter,
            credentials,
            api,
            now,
        }
    }
    fn searches(&self) -> Vec<Call> {
        self.api
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, Call::Issues { .. }))
            .cloned()
            .collect()
    }
}

fn repository() -> IssueSourceRepository {
    IssueSourceRepository {
        provider: IssueProvider::Linear,
        server_id: LINEAR_SERVER_ID.into(),
        repository_id: TEAM.into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/work/linear-project".into(),
        },
    }
}

fn request(cursor: Option<IssueDiscoveryCursor>) -> IssueDiscoveryRequest {
    IssueDiscoveryRequest {
        target: IssueDiscoveryTarget::Repository(repository()),
        selection: IssueDiscoverySelection::AllOpen,
        cursor,
        page_size: 2,
        disclosure: IssueSourceDisclosure {
            policy: IssueDisclosurePolicy::builtin(),
            policy_version: "policy-v1".into(),
        },
    }
}

fn resync() -> IssueResyncRequest {
    let repository = repository();
    IssueResyncRequest {
        issue: IssueReference {
            provider: repository.provider,
            server_id: repository.server_id,
            repository_id: repository.repository_id,
            issue_id: ISSUE.into(),
            project_scope: repository.project_scope,
        },
        disclosure: request(None).disclosure,
    }
}

fn opaque(cursor: &IssueDiscoveryCursor) -> Value {
    let request = request(None);
    serde_json::from_str(
        cursor
            .validate_for(&repository(), &request.selection, 2, &request.disclosure)
            .unwrap(),
    )
    .unwrap()
}

fn observed(read: IssueSourceRead) -> IssueSourceObservation {
    match read {
        IssueSourceRead::Observed(value) => *value,
        other => panic!("expected an observation, got {other:?}"),
    }
}

#[test]
fn construction_is_inert_and_another_account_never_issues_a_request() {
    let fixture = Fixture::new();
    assert_eq!(fixture.credentials.reads.load(Ordering::SeqCst), 0);
    fixture
        .credentials
        .summary
        .lock()
        .unwrap()
        .metadata
        .account_id = "another-viewer".into();
    assert_eq!(
        fixture.adapter.discover(request(None)),
        Err(IssueSourceFailure::IdentityMismatch)
    );
    assert!(fixture.api.calls.lock().unwrap().is_empty());
}

#[test]
fn discovery_maps_a_team_issue_with_its_branch_name_and_exact_connection() {
    let fixture = Fixture::new();
    fixture.api.page(vec![issue(ISSUE, "ENG-7")], None);
    let page = fixture.adapter.discover(request(None)).unwrap();
    assert_eq!(page.scanned_count, 1);
    let operation = page.linear.unwrap();
    assert_eq!(operation.connection, fixture.credentials.reference());
    assert_eq!(operation.binding, binding());
    assert!(page.jira_binding.is_none() && page.jira_connection.is_none());
    let observation = &page.observations[0];
    assert_eq!(observation.issue, resync().issue);
    assert_eq!(observation.remote_state, IssueRemoteState::Open);
    assert_eq!(observation.linked_pull_request, IssuePrPresence::Unknown);
    let linear = observation.linear.as_ref().unwrap();
    assert_eq!(linear.identifier, "ENG-7");
    assert_eq!(linear.url, "https://linear.app/example/issue/ENG-7");
    assert_eq!(
        linear.branch_name.as_deref(),
        Some("operator/eng-7-keep-the-description")
    );
    assert_eq!(linear.state_type, LinearIssueStateType::Started);
    assert_eq!(linear.connection, fixture.credentials.reference());
    let IssueSourceContent::Shared(snapshot) = &observation.content else {
        panic!("the description should be shared");
    };
    assert!(snapshot.shared_copy.body.contains("완료 기준"));
    assert_eq!(
        fixture.api.calls.lock().unwrap()[..2],
        [Call::Viewer, Call::Team]
    );
}

#[test]
fn the_watermark_advances_only_after_the_last_page_of_a_window() {
    let fixture = Fixture::new();
    // Two issues with the same updatedAt meet at a page boundary.
    fixture
        .api
        .page(vec![issue(ISSUE, "ENG-7")], Some("cursor-1"));
    let first = fixture
        .adapter
        .discover(request(None))
        .unwrap()
        .next_cursor
        .unwrap();
    assert_eq!(opaque(&first)["watermark"], 0);
    fixture.now.store(NOW + 60_000, Ordering::SeqCst);
    let mut second_issue = issue("6f1c1a52-0000-4000-8000-00000000c002", "ENG-8");
    second_issue["updatedAt"] = json!("2026-11-20T12:00:00.123Z");
    fixture.api.page(vec![second_issue], None);
    let second = fixture
        .adapter
        .discover(request(Some(first.clone())))
        .unwrap();
    assert_eq!(
        second.observations[0].issue.issue_id.as_str(),
        "6f1c1a52-0000-4000-8000-00000000c002"
    );
    let second = second.next_cursor.unwrap();
    assert_eq!(opaque(&second)["watermark"], NOW);
    // A failed middle page stores nothing: the same continuation is read again.
    fixture.api.identity();
    fixture.api.fail(ProviderCredentialError::Unavailable);
    fixture.api.fail(ProviderCredentialError::Unavailable);
    assert_eq!(
        fixture.adapter.discover(request(Some(first))),
        Err(IssueSourceFailure::ProviderUnavailable)
    );
    let searches = fixture.searches();
    let Call::Issues {
        from,
        before,
        after,
    } = &searches[1]
    else {
        unreachable!()
    };
    assert_eq!(after.as_deref(), Some("cursor-1"));
    assert_eq!(from, "1970-01-01T00:00:00.000Z");
    assert_eq!(before, "2026-11-18T11:06:40.000Z");
    // The failed continuation restarts the whole window instead of skipping a page.
    assert!(matches!(&searches[3], Call::Issues { after: None, .. }));
}

#[test]
fn the_next_window_overlaps_the_previous_watermark() {
    let fixture = Fixture::new();
    fixture.api.page(vec![], None);
    let first = fixture.adapter.discover(request(None)).unwrap().next_cursor;
    fixture.now.store(NOW + 60_000, Ordering::SeqCst);
    fixture.api.page(vec![], None);
    fixture.adapter.discover(request(first)).unwrap();
    let Call::Issues { from, before, .. } = &fixture.searches()[1] else {
        unreachable!()
    };
    assert_eq!(from, "2026-11-18T10:56:40.000Z");
    assert_eq!(before, "2026-11-18T11:07:40.000Z");
}

#[test]
fn resync_reads_the_stable_id_and_rejects_another_team_or_issue() {
    let fixture = Fixture::new();
    fixture.api.resync(issue(ISSUE, "OPS-3"));
    let current = observed(fixture.adapter.resync(resync()));
    assert_eq!(current.issue, resync().issue);
    assert_eq!(current.linear.unwrap().identifier, "OPS-3");
    assert_eq!(
        fixture.api.calls.lock().unwrap().as_slice(),
        &[Call::Viewer, Call::Team, Call::Issue(ISSUE.into())]
    );
    for change in 0..2 {
        let fixture = Fixture::new();
        let mut value = issue(ISSUE, "ENG-7");
        if change == 0 {
            value["team"]["id"] = json!("6f1c1a52-0000-4000-8000-00000000a999");
        } else {
            value["id"] = json!("6f1c1a52-0000-4000-8000-00000000c999");
        }
        fixture.api.resync(value);
        assert_eq!(
            fixture.adapter.resync(resync()),
            IssueSourceRead::Unknown {
                issue: resync().issue,
                reason: IssueSourceFailure::IdentityMismatch,
            }
        );
    }
}

#[test]
fn completed_states_are_closed_and_an_unknown_state_is_never_closed() {
    let fixture = Fixture::new();
    let mut value = issue(ISSUE, "ENG-7");
    value["state"] = json!({"id": DONE, "type": "completed"});
    fixture.api.resync(value.clone());
    assert_eq!(
        observed(fixture.adapter.resync(resync())).remote_state,
        IssueRemoteState::Closed
    );
    let fixture = Fixture::new();
    value["state"]["type"] = json!("archived");
    fixture.api.resync(value);
    assert!(matches!(
        fixture.adapter.resync(resync()),
        IssueSourceRead::Unknown {
            reason: IssueSourceFailure::MalformedResponse,
            ..
        }
    ));
}

#[test]
fn a_viewer_change_or_credential_rotation_discards_the_read() {
    let fixture = Fixture::new();
    fixture
        .api
        .answer(json!({"viewer": {"id": "another-viewer", "active": true}}));
    assert_eq!(
        fixture.adapter.discover(request(None)),
        Err(IssueSourceFailure::IdentityMismatch)
    );
    let fixture = Fixture::new();
    fixture.api.page(vec![issue(ISSUE, "ENG-7")], None);
    fixture.api.rotate_on_call.store(3, Ordering::SeqCst);
    assert_eq!(
        fixture.adapter.discover(request(None)),
        Err(IssueSourceFailure::ProviderUnavailable)
    );
}

#[test]
fn label_selection_uses_complete_labels_and_an_unsafe_branch_name_is_only_dropped() {
    let fixture = Fixture::new();
    let mut unlabeled = issue(ISSUE, "ENG-7");
    unlabeled["labels"]["nodes"] = json!([]);
    fixture.api.page(vec![unlabeled], None);
    let mut selected = request(None);
    selected.selection = IssueDiscoverySelection::LabelsAny(vec!["akra:gated".into()]);
    let page = fixture.adapter.discover(selected).unwrap();
    assert_eq!((page.scanned_count, page.observations.len()), (1, 0));

    for unsafe_name in ["../main", "사용자/eng-7"] {
        let fixture = Fixture::new();
        let mut unsafe_branch = issue(ISSUE, "ENG-7");
        unsafe_branch["branchName"] = json!(unsafe_name);
        fixture.api.resync(unsafe_branch);
        let current = observed(fixture.adapter.resync(resync()));
        assert_eq!(current.linear.unwrap().branch_name, None);
    }
}
