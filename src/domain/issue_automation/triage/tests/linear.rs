use super::*;
use crate::domain::issue_automation::coordinator::{
    policy_start, ready_for_human_start, ready_for_start,
};
use crate::domain::issue_automation::source::{
    IssueSourceLinearMetadata, LINEAR_SERVER_ID, LinearIssueStateType,
};
use crate::domain::provider_credentials::{ConnectionRef, Provider};

const TEAM: &str = "6f1c1a52-0000-4000-8000-00000000a001";

fn linear_source(branch_name: &str) -> IssueSourceRead {
    let mut current = source();
    let value = observed(&mut current);
    value.issue.provider = IssueProvider::Linear;
    value.issue.server_id = LINEAR_SERVER_ID.into();
    value.issue.repository_id = TEAM.into();
    value.issue.issue_id = "6f1c1a52-0000-4000-8000-00000000c001".into();
    value.repository_visibility = IssueRepositoryVisibility::Unknown;
    value.repository_name_with_owner = None;
    value.linked_pull_request = IssuePrPresence::Unknown;
    value.linear = Some(IssueSourceLinearMetadata {
        identifier: "ENG-7".into(),
        url: "https://linear.app/example/issue/ENG-7".into(),
        state_id: "6f1c1a52-0000-4000-8000-00000000d001".into(),
        state_type: LinearIssueStateType::Started,
        branch_name: Some(branch_name.into()),
        connection: ConnectionRef {
            provider: Provider::Linear,
            profile_id: "a".repeat(64),
            connection_id: "c".repeat(32),
            revision: 1,
        },
    });
    current
}

fn linear_policy(mode: IssueAutomationAutonomy) -> IssueAutomationPolicy {
    let mut policy = policy();
    policy.binding.linear = Some(
        crate::domain::issue_automation::policy::IssueAutomationLinearBinding {
            team_id: TEAM.into(),
            expected_viewer_id: "U_trusted".into(),
            mode,
            close_state: None,
        },
    );
    policy.feedback.labels = false;
    policy.select.trusted_actors[0].provider = IssueProvider::Linear;
    policy.select.trusted_actors[0].server_id = LINEAR_SERVER_ID.into();
    policy
}

fn assess(current: &IssueSourceRead, policy: &IssueAutomationPolicy) -> IssueTriageAssessment {
    let IssueSourceRead::Observed(observation) = current else {
        unreachable!()
    };
    triage_issue(input(
        &observation.issue,
        Some(current),
        policy,
        &IssueRequirements::default(),
        None,
    ))
    .unwrap()
}

#[test]
fn unknown_linear_pr_presence_opens_only_the_explicit_human_start() {
    let source = linear_source("operator/eng-7-fix");
    for mode in [
        IssueAutomationAutonomy::Observe,
        IssueAutomationAutonomy::Propose,
        IssueAutomationAutonomy::Auto,
    ] {
        let assessment = assess(&source, &linear_policy(mode));
        assert_eq!(assessment.eligibility(), IssueTriageEligibility::Unknown);
        assert!(
            assessment
                .reasons()
                .contains(&IssueTriageReasonCode::LinearPullRequestLookupUnsupported)
        );
        assert!(
            !assessment
                .reasons()
                .contains(&IssueTriageReasonCode::JiraPullRequestLookupUnsupported)
        );
        assert!(assessment.requires_provider_human_start_review());
        assert!(!ready_for_start(&assessment) && !policy_start(&assessment));
        assert_eq!(
            ready_for_human_start(&assessment),
            mode >= IssueAutomationAutonomy::Propose
        );
    }
}

#[test]
fn a_changed_branch_name_or_missing_metadata_changes_the_approval_basis() {
    let policy = linear_policy(IssueAutomationAutonomy::Propose);
    let first = assess(&linear_source("operator/eng-7-fix"), &policy);
    let renamed = assess(&linear_source("operator/eng-7-other"), &policy);
    assert_ne!(first.input_fingerprint(), renamed.input_fingerprint());
    let mut missing = linear_source("operator/eng-7-fix");
    observed(&mut missing).linear = None;
    let missing = assess(&missing, &policy);
    assert!(!missing.requires_provider_human_start_review());
    assert!(!ready_for_human_start(&missing));
}
