use super::*;
use crate::application::port::outbound::issue_feedback_port::{
    IssueFeedbackClosureEvent, IssueFeedbackClosureSnapshot,
};
use crate::domain::issue_automation::source::{
    IssueSourceContent, IssueSourceJiraMetadata, JiraIssueStatusCategory, JiraPullRequestLookup,
};
use crate::domain::provider_credentials::{ConnectionRef, Provider};
use std::sync::atomic::Ordering;

const JIRA_POLICY: &str = r#"
[binding.jira]
site_url = "https://example.atlassian.net"
cloud_id = "cloud-1"
project_id = "10"
expected_account_id = "U_feedback"
jql = "assignee = currentUser()"
mode = "propose"
[feedback]
comment = true
labels = false
"#;

struct JiraProvider {
    inner: Arc<FakeProvider>,
    connection: Mutex<IssueFeedbackConnection>,
}
impl IssueFeedbackPort for JiraProvider {
    fn actor_id(&self, issue: &IssueReference) -> Result<String> {
        self.inner.actor_id(issue)
    }
    fn actor_context(&self, issue: &IssueReference) -> Result<IssueFeedbackActorContext> {
        Ok(IssueFeedbackActorContext {
            actor_id: self.actor_id(issue)?,
            connection: Some(self.connection.lock().unwrap().clone()),
        })
    }
    fn read_label(
        &self,
        _issue: &IssueReference,
        _name: &str,
    ) -> Result<IssueFeedbackLabelSnapshot> {
        bail!("Jira labels unsupported")
    }
    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        self.inner.write(intent)
    }
    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        self.inner.reconcile(intent)
    }
}
struct JiraFixture {
    base: Fixture,
    provider: Arc<JiraProvider>,
    issue: IssueReference,
}
impl JiraFixture {
    fn new() -> Self {
        let base = Fixture::new(true, false);
        *base.policy.user.lock().unwrap() = JIRA_POLICY.into();
        let mut issue = issue();
        issue.provider = IssueProvider::Jira;
        issue.server_id = "cloud-1".into();
        issue.repository_id = "10".into();
        issue.issue_id = "100".into();
        let connection = IssueFeedbackConnection {
            profile_id: "a".repeat(64),
            connection_id: "b".repeat(32),
            revision: 1,
        };
        {
            let mut data = base.store.data.lock().unwrap();
            let snapshot = &mut data.snapshot;
            snapshot.item.issue = issue.clone();
            snapshot.state.issue = issue.clone();
            snapshot.work.as_mut().unwrap().order.issue = issue.clone();
            let Some(IssueSourceRead::Observed(observed)) = snapshot.item.current.as_mut() else {
                panic!("source");
            };
            observed.issue = issue.clone();
            observed.jira = Some(IssueSourceJiraMetadata {
                key: "APP-1".into(),
                url: "https://example.atlassian.net/browse/APP-1".into(),
                status_id: "1".into(),
                status_category: JiraIssueStatusCategory::Indeterminate,
                pull_request_lookup: JiraPullRequestLookup::Unsupported,
                connection: ConnectionRef {
                    profile_id: connection.profile_id.clone(),
                    provider: Provider::Jira,
                    connection_id: connection.connection_id.clone(),
                    revision: connection.revision,
                },
            });
            if let IssueSourceContent::Shared(shared) = &mut observed.content {
                shared.policy_version = issue_policy_version(&IssuePolicy::builtin());
            }
        }
        base.provider.label.lock().unwrap().issue = issue.clone();
        base.provider.closure.lock().unwrap().issue = issue.clone();
        let provider = Arc::new(JiraProvider {
            inner: base.provider.clone(),
            connection: Mutex::new(connection),
        });
        Self {
            base,
            provider,
            issue,
        }
    }
    fn service(&self) -> IssueFeedbackService {
        IssueFeedbackService::new(
            WORKSPACE.into(),
            self.base.store.clone(),
            self.provider.clone(),
            self.base.journal.clone(),
            IssueAutomationPolicyService::new(self.base.policy.clone()),
            IssuePolicyService::new(Arc::new(DefaultDisclosure)),
        )
        .with_issue_binding_admission(self.base.admission.clone())
    }
    fn sync(&self) {
        self.service()
            .sync_issue(&self.issue, 123, &IssueCollectionCancellation::new())
            .unwrap();
    }
}

#[test]
fn jira_status_uses_the_sent_journal_and_recovery_never_repeats_a_lost_write() {
    let fixture = JiraFixture::new();
    *fixture.base.provider.write_behavior.lock().unwrap() = WriteBehavior::Lost;
    *fixture.base.provider.read_behavior.lock().unwrap() = ReadBehavior::Unavailable;
    fixture.sync();
    let original = fixture.base.store.record();
    assert_eq!(original.state, IssueFeedbackIntentState::NeedsConfirmation);
    assert_eq!(original.intent.connection.as_ref().unwrap().revision, 1);
    assert_eq!(fixture.base.writes().len(), 1);
    fixture.base.policy.fail.store(true, Ordering::SeqCst);
    *fixture.base.provider.read_behavior.lock().unwrap() = ReadBehavior::Confirm;
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    let saved = fixture.base.store.record();
    assert_eq!(saved.state, IssueFeedbackIntentState::Confirmed);
    assert_eq!(saved.intent, original.intent);
    assert_eq!(fixture.base.writes().len(), 1);
    assert_eq!(fixture.service().pending_events().unwrap().len(), 1);
}

#[test]
fn a_ready_connection_different_from_the_observed_source_is_not_adopted() {
    let fixture = JiraFixture::new();
    fixture.provider.connection.lock().unwrap().revision = 2;
    fixture.sync();
    assert!(fixture.base.writes().is_empty());
    assert!(fixture.base.store.data.lock().unwrap().records.is_empty());
    assert_eq!(fixture.base.store.snapshot_value().publication_attempts, 0);
}

#[test]
fn jira_missing_transition_ack_stays_unknown_even_when_the_same_account_closed_it() {
    let fixture = JiraFixture::new();
    let mut record = pending_record(fixture.issue.clone());
    record.intent.connection = Some(fixture.provider.connection.lock().unwrap().clone());
    record.intent.operation = IssueFeedbackOperation::CloseIssue {
        integration_record_id: "integration-1".into(),
        integration_sha: "a".repeat(40),
        decision_id: "human-close".into(),
        previous_event_id: None,
        jira_transition: Some(IssueFeedbackJiraTransition {
            transition_id: "31".into(),
            from_status_id: "1".into(),
            to_status_id: "3".into(),
        }),
    };
    record.state = IssueFeedbackIntentState::Sent;
    record.validate().unwrap();
    fixture.base.store.data.lock().unwrap().records.push(record);
    *fixture.base.provider.closure.lock().unwrap() = IssueFeedbackClosureSnapshot {
        issue: fixture.issue.clone(),
        closed: true,
        complete: true,
        latest_event: Some(IssueFeedbackClosureEvent {
            event_id: "2000".into(),
            actor_id: Some(ACTOR.into()),
            closed: true,
        }),
    };
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    let result = fixture.base.store.record();
    assert_eq!(result.state, IssueFeedbackIntentState::NeedsConfirmation);
    assert!(!result.closure_acknowledged);
    assert!(
        fixture
            .base
            .store
            .snapshot_value()
            .state
            .close_proof
            .is_none()
    );
    assert!(fixture.service().pending_events().unwrap().is_empty());
    assert!(fixture.base.writes().is_empty());
}

#[test]
fn jira_label_feedback_cannot_silently_satisfy_the_first_grant_requirement() {
    let fixture = JiraFixture::new();
    *fixture.base.policy.user.lock().unwrap() =
        JIRA_POLICY.replace("labels = false", "labels = true");
    let error = fixture
        .service()
        .ensure_first_grant(
            &fixture.issue,
            WORK_ORDER,
            &IssueCollectionCancellation::new(),
        )
        .expect_err("unsupported Jira label writes must block policy before a grant");
    assert!(error.to_string().contains("jira_label_writes_unsupported"));
    assert!(fixture.base.writes().is_empty());
    assert!(!fixture.base.store.snapshot_value().first_grant_entered);
}
