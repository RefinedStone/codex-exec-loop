use super::*;
use crate::domain::issue_automation::{IssueAutomationBinding, IssueAutomationBindingId};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};
use crate::domain::provider_credentials::{ConnectionRef, Provider};

const POLICY: &str = r#"
[binding.jira]
site_url = "https://example.atlassian.net"
cloud_id = "cloud-1"
project_id = "10"
expected_account_id = "account-1"
jql = "assignee = currentUser()"
mode = "propose"
[feedback]
comment = true
"#;

fn setup(fixture: &Fixture) -> IssueReference {
    let adapter = fixture.adapter();
    let collection = adapter.ensure_collection(fixture.cwd()).unwrap();
    let binding = IssueAutomationBinding::new(
        IssueAutomationBindingId::new("jira-feedback").unwrap(),
        IssueProvider::Jira,
        "cloud-1",
        "10",
        IssueProjectScope {
            workspace_dir: fixture.cwd().into(),
        },
    )
    .unwrap();
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    let tracked = adapter
        .insert_issue(fixture.cwd(), &discovered(&binding, "100", "Shared task"))
        .unwrap()
        .issue;
    let mut shared = shared_snapshot("## 완료 기준\n- 결과를 검증한다.\n");
    shared.policy_version = issue_policy_version(&IssuePolicy::builtin());
    let observed = IssueSourceObservation {
        repository_visibility: Default::default(),
        repository_name_with_owner: None,
        issue: tracked.reference().clone(),
        web: None,
        linear: None,
        jira: Some(IssueSourceJiraMetadata {
            key: "APP-1".into(),
            url: "https://example.atlassian.net/browse/APP-1".into(),
            status_id: "1".into(),
            status_category: JiraIssueStatusCategory::Indeterminate,
            pull_request_lookup: JiraPullRequestLookup::Unsupported,
            connection: ConnectionRef {
                profile_id: "a".repeat(64),
                provider: Provider::Jira,
                connection_id: "b".repeat(32),
                revision: 1,
            },
        }),
        content_problems: vec![],
        remote_state: IssueRemoteState::Open,
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        assignees: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: None,
        label_actors: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Unknown,
        content: IssueSourceContent::Shared(shared),
    };
    adapter
        .apply_issue_resync(
            fixture.cwd(),
            collection.fence(),
            tracked.reference(),
            &IssueSourceRead::Observed(Box::new(observed)),
        )
        .unwrap();
    let batch = adapter.read_triage_batch(fixture.cwd(), None, 100).unwrap();
    let item = batch
        .items
        .into_iter()
        .find(|item| item.issue == *tracked.reference())
        .unwrap();
    let policy = resolve_issue_automation_policy(Some(POLICY), None).unwrap();
    let assessment = triage_issue(IssueTriageInput {
        issue: tracked.reference(),
        current: item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        runtime_selection_revision: 0,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: false,
        pinned_source_commit: None,
        previous: None,
    })
    .unwrap();
    assert!(matches!(
        adapter
            .save_triage(
                fixture.cwd(),
                IssueTriageFence {
                    binding_run_revision: 0,
                    collection_epoch: collection.epoch,
                    observation_revision: item.observation_revision,
                    tracked_revision: item.tracked_revision,
                    assessment_revision: None
                },
                &assessment
            )
            .unwrap(),
        IssueTriageSaveResult::Saved { .. }
    ));
    tracked.reference().clone()
}
fn intent(snapshot: &IssueFeedbackSnapshot) -> IssueFeedbackIntent {
    let desired = snapshot.state.desired.as_ref().unwrap();
    let id = "jira-intent";
    let body = format!("{}\n\n{}", desired.comment_body, feedback_marker(id));
    let basis = snapshot.item.triage.as_ref().unwrap().assessment.basis();
    IssueFeedbackIntent {
        intent_id: id.into(),
        issue: snapshot.item.issue.clone(),
        actor_id: "account-1".into(),
        connection: Some(IssueFeedbackConnection {
            profile_id: "a".repeat(64),
            connection_id: "b".repeat(32),
            revision: 1,
        }),
        automation_policy_version: basis.automation_policy_version.clone(),
        disclosure_policy_version: basis.disclosure_policy_version.clone().unwrap(),
        desired_revision: snapshot.state.desired_revision,
        operation: IssueFeedbackOperation::CreateComment {
            body_sha256: feedback_body_hash(&body),
            body,
            content_sha256: desired.comment_body_sha256.clone(),
        },
        recorded_at: "2026-11-23T12:00:00Z".into(),
    }
}

#[test]
fn jira_connection_fence_sent_claim_and_receipt_survive_adapter_restart() {
    let fixture = Fixture::new("jira-feedback-roundtrip");
    let issue = setup(&fixture);
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "Shared status");
    let before = current(&adapter, &issue);
    let valid = intent(&before);
    let mut stale = valid.clone();
    stale.connection.as_mut().unwrap().revision = 2;
    assert!(matches!(
        adapter.record(before.fence(), &stale).unwrap(),
        IssueFeedbackRecordResult::Conflict
    ));
    let recorded = match adapter.record(before.fence(), &valid).unwrap() {
        IssueFeedbackRecordResult::Created(record) => record,
        other => panic!("{other:?}"),
    };
    let sent = match adapter
        .claim_sent(
            &issue,
            &valid.intent_id,
            recorded.revision,
            current(&adapter, &issue).fence(),
        )
        .unwrap()
    {
        IssueFeedbackClaimResult::Claimed(record) => record,
        other => panic!("{other:?}"),
    };
    let restarted = fixture.adapter();
    assert_eq!(restarted.pending(fixture.cwd()).unwrap()[0], sent);
    assert!(matches!(
        restarted
            .claim_sent(
                &issue,
                &valid.intent_id,
                recorded.revision,
                current(&restarted, &issue).fence()
            )
            .unwrap(),
        IssueFeedbackClaimResult::Existing(_)
    ));
    let IssueFeedbackOperation::CreateComment { body_sha256, .. } = &valid.operation else {
        unreachable!()
    };
    let receipt = IssueFeedbackReceipt {
        issue: issue.clone(),
        intent_id: valid.intent_id.clone(),
        actor_id: valid.actor_id.clone(),
        evidence: IssueFeedbackEvidence::Comment {
            comment_id: "1000".into(),
            body_sha256: body_sha256.clone(),
        },
    };
    let completed = restarted
        .complete(
            &issue,
            &valid.intent_id,
            sent.revision,
            &IssueFeedbackCompletion::Confirmed(Box::new(receipt)),
        )
        .unwrap()
        .unwrap();
    assert_eq!(completed.state, IssueFeedbackIntentState::Confirmed);
    assert_eq!(completed.intent.connection, valid.connection);
    assert_eq!(restarted.pending_events(fixture.cwd()).unwrap().len(), 1);
    let state = current(&fixture.adapter(), &issue).state;
    assert_eq!(state.comment_owner.unwrap().comment_id, "1000");
}
