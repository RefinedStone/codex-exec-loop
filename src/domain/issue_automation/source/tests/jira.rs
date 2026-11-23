use super::*;
use crate::domain::provider_credentials::{ConnectionRef, Provider};

fn jira() -> IssueSourceObservation {
    let mut value = observation();
    value.issue.provider = IssueProvider::Jira;
    value.issue.server_id = "cloud-1".into();
    value.issue.repository_id = "10001".into();
    value.issue.issue_id = "10041".into();
    value.linked_pull_request = IssuePrPresence::Unknown;
    value.jira = Some(IssueSourceJiraMetadata {
        key: "APP-41".into(),
        url: "https://jira.example.invalid/browse/APP-41".into(),
        status_id: "3".into(),
        status_category: JiraIssueStatusCategory::Indeterminate,
        pull_request_lookup: JiraPullRequestLookup::Unsupported,
        connection: ConnectionRef {
            profile_id: "a".repeat(64),
            provider: Provider::Jira,
            connection_id: "b".repeat(32),
            revision: 1,
        },
    });
    value
}

#[test]
fn legacy_source_serialization_has_no_new_empty_fields() {
    let original = observation();
    let encoded = serde_json::to_value(&original).unwrap();
    assert!(encoded.get("contentProblems").is_none());
    assert!(encoded.get("jira").is_none());
    let restored: IssueSourceObservation = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored, original);
}

#[test]
fn jira_metadata_and_problem_codes_round_trip_without_remote_body_or_tokens() {
    let mut original = jira();
    original.content_problems = vec![
        IssueSourceContentProblem::UnsupportedAdf,
        IssueSourceContentProblem::MissingRequiredField,
    ];
    original.validate_repository_metadata().unwrap();
    let encoded = serde_json::to_value(&original).unwrap();
    let restored: IssueSourceObservation = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(restored, original);
    assert_eq!(
        encoded["contentProblems"],
        serde_json::json!(["unsupported_adf", "missing_required_field"])
    );
    assert_eq!(encoded["jira"]["pullRequestLookup"], "unsupported");
    assert!(encoded["jira"].get("token").is_none());
}

#[test]
fn invalid_connection_coordinates_problem_order_and_inconsistent_remote_facts_are_rejected() {
    let changes: &[fn(&mut IssueSourceObservation)] = &[
        |value| value.issue.provider = IssueProvider::GitHub,
        |value| value.jira.as_mut().unwrap().url = "https://other.invalid/browse/APP-99".into(),
        |value| value.jira.as_mut().unwrap().connection.provider = Provider::Linear,
        |value| value.jira.as_mut().unwrap().connection.revision = 0,
        |value| value.jira.as_mut().unwrap().status_id = "Done".into(),
        |value| value.linked_pull_request = IssuePrPresence::Absent,
        |value| value.remote_state = IssueRemoteState::Closed,
        |value| {
            value.content_problems = vec![
                IssueSourceContentProblem::MissingRequiredField,
                IssueSourceContentProblem::UnsupportedAdf,
            ]
        },
        |value| value.content_problems = vec![IssueSourceContentProblem::UnsupportedAdf; 2],
    ];
    for change in changes {
        let mut value = jira();
        change(&mut value);
        assert_eq!(
            value.validate_repository_metadata(),
            Err(IssueSourceFailure::MalformedResponse)
        );
    }
}
