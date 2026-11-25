//! The real Linear source adapter against a fake GraphQL server keeps GitHub's collection
//! contract: discovery tracks open team issues, resync reads stable IDs regardless of filters.
use super::*;
use crate::adapter::outbound::db::sqlite_issue_source_claims::SqliteIssueSourceClaimsAdapter;
use crate::adapter::outbound::linear::LinearIssueSourceAdapter;
use crate::adapter::outbound::linear::test_support::{
    Api, Call, Credentials, DONE, ISSUE, TEAM, VIEWER, issue,
};
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_source_claim_port::IssueSourceClaimPort;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::domain::issue_automation::source::LINEAR_SERVER_ID;
use serde_json::json;

struct LinearPolicy {
    project: &'static str,
}
impl IssueAutomationPolicySourcePort for LinearPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(Some(format!(
            "[binding.linear]\nteam_id = \"{TEAM}\"\nexpected_viewer_id = \"{VIEWER}\"\nmode = \"observe\"\n[select]\nmode = \"all_open\"\n"
        )))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(Some(self.project.into()))
    }
}

struct Harness {
    api: Arc<Api>,
    service: IssueCollectionService,
}

fn harness(
    fixture: &Fixture,
    project: &'static str,
    claims: Arc<dyn IssueSourceClaimPort>,
) -> Harness {
    let credentials = Arc::new(Credentials::new());
    let api = Arc::new(Api::new(credentials.clone()));
    let policy = Arc::new(LinearPolicy { project });
    let binding = IssueAutomationPolicyService::new(policy.clone())
        .resolve(&fixture.workspace)
        .unwrap()
        .policy
        .binding
        .linear;
    let adapter = binding.map(|binding| {
        Arc::new(LinearIssueSourceAdapter::new(binding, credentials.clone(), api.clone()).unwrap())
    });
    let (discover, resync) = (adapter.clone(), adapter);
    let source = FakeSource::new(
        move |request| discover.as_ref().expect("Linear binding").discover(request),
        move |request| resync.as_ref().expect("Linear binding").resync(request),
    );
    let origin = Arc::new(FakeOrigin::default());
    origin.unavailable.store(true, Ordering::SeqCst);
    let service = fixture
        .service(source, origin, Arc::default())
        .with_connected_source_collection(
            IssueAutomationPolicyService::new(policy),
            Some(credentials),
            claims,
        );
    Harness { api, service }
}

fn claims(fixture: &Fixture) -> Arc<SqliteIssueSourceClaimsAdapter> {
    let directory = fixture.root.join("profile-claims");
    crate::private_directory::ensure_private_directory(&directory).unwrap();
    Arc::new(SqliteIssueSourceClaimsAdapter::new(move || {
        Ok(directory.clone())
    }))
}

fn expected(fixture: &Fixture) -> IssueReference {
    IssueReference {
        provider: IssueProvider::Linear,
        server_id: LINEAR_SERVER_ID.into(),
        repository_id: TEAM.into(),
        issue_id: ISSUE.into(),
        project_scope: IssueProjectScope {
            workspace_dir: fixture.workspace.clone(),
        },
    }
}

fn current(service: &IssueCollectionService) -> IssueSourceObservation {
    match query(service).items[0].current.clone() {
        Some(IssueSourceRead::Observed(observation)) => *observation,
        other => panic!("expected a current observation, got {other:?}"),
    }
}

#[test]
fn a_bound_team_issue_is_discovered_then_resynchronized_by_its_stable_id() {
    let fixture = Fixture::new();
    let harness = harness(&fixture, "[binding.linear]\n", claims(&fixture));
    harness.api.page(vec![issue(ISSUE, "ENG-7")], None);
    harness.api.resync(issue(ISSUE, "ENG-7"));
    let first = harness
        .service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert_eq!(first.newly_tracked, [expected(&fixture)]);
    let linear = current(&harness.service).linear.unwrap();
    assert_eq!(linear.branch_name, "operator/eng-7-keep-the-description");

    // The issue leaves the discovery filter (completed). Resync still reads and closes it.
    let mut completed = issue(ISSUE, "ENG-7");
    completed["state"] = json!({"id": DONE, "type": "completed"});
    harness.api.answers.lock().unwrap().clear();
    harness.api.page(vec![], None);
    harness.api.resync(completed);
    let second = harness
        .service
        .collect_once(&IssueCollectionCancellation::new(), 200)
        .unwrap();
    assert!(second.newly_tracked.is_empty());
    assert_eq!(
        current(&harness.service).remote_state,
        IssueRemoteState::Closed
    );
    assert!(
        harness
            .api
            .calls
            .lock()
            .unwrap()
            .contains(&Call::Issue(ISSUE.into()))
    );
}

#[test]
fn a_repository_without_a_linear_binding_never_reads_or_tracks_the_team() {
    let fixture = Fixture::new();
    // The user file names the team, but this repository did not opt in.
    let harness = harness(&fixture, "", claims(&fixture));
    let outcome = harness
        .service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert!(outcome.newly_tracked.is_empty());
    assert_eq!(
        outcome.projection.state.status.last_failure,
        Some(IssueCollectionFailure::OriginUnavailable)
    );
    assert!(harness.api.calls.lock().unwrap().is_empty());
    assert!(query(&harness.service).items.is_empty());
}

#[test]
fn an_issue_of_another_team_is_rejected_without_tracking() {
    let fixture = Fixture::new();
    let harness = harness(&fixture, "[binding.linear]\n", claims(&fixture));
    let mut foreign = issue(ISSUE, "OPS-1");
    foreign["team"]["id"] = json!("6f1c1a52-0000-4000-8000-00000000a999");
    harness.api.page(vec![foreign], None);
    let outcome = harness
        .service
        .collect_once(&IssueCollectionCancellation::new(), 100)
        .unwrap();
    assert!(outcome.newly_tracked.is_empty());
    assert_eq!(
        outcome.projection.state.status.last_failure,
        Some(IssueCollectionFailure::Source(
            IssueSourceFailure::IdentityMismatch
        ))
    );
    assert!(query(&harness.service).items.is_empty());
}

#[test]
fn the_same_team_issue_is_claimed_by_only_one_checkout() {
    let first = Fixture::new();
    let second = Fixture::new();
    let shared = claims(&first);
    let one = harness(&first, "[binding.linear]\n", shared.clone());
    let two = harness(&second, "[binding.linear]\n", shared);
    one.api.page(vec![issue(ISSUE, "ENG-7")], None);
    one.api.resync(issue(ISSUE, "ENG-7"));
    two.api.page(vec![issue(ISSUE, "ENG-7")], None);
    assert_eq!(
        one.service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .unwrap()
            .newly_tracked
            .len(),
        1
    );
    let skipped = two
        .service
        .collect_once(&IssueCollectionCancellation::new(), 200)
        .unwrap();
    assert!(skipped.newly_tracked.is_empty());
    assert!(skipped.projection.state.discovery_cursor.is_some());
    assert!(query(&two.service).items.is_empty());
}
