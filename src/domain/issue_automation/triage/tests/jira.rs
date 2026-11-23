use super::*;
use crate::domain::issue_automation::coordinator::{
    IssueCoordinatorAction, IssueCoordinatorInput, next_action, policy_start,
    ready_for_human_start, ready_for_start,
};
use crate::domain::issue_automation::source::{
    IssueSourceContentProblem, IssueSourceJiraMetadata, JiraIssueStatusCategory,
    JiraPullRequestLookup,
};
use crate::domain::provider_credentials::{ConnectionRef, Provider};

fn jira_source() -> IssueSourceRead {
    let mut current = source();
    let value = observed(&mut current);
    value.issue.provider = IssueProvider::Jira;
    value.issue.server_id = "cloud-123".into();
    value.issue.repository_id = "10001".into();
    value.issue.issue_id = "10002".into();
    value.repository_visibility = IssueRepositoryVisibility::Unknown;
    value.repository_name_with_owner = None;
    value.linked_pull_request = IssuePrPresence::Unknown;
    value.jira = Some(IssueSourceJiraMetadata {
        key: "AKRA-42".into(),
        url: "https://example.atlassian.net/browse/AKRA-42".into(),
        status_id: "10000".into(),
        status_category: JiraIssueStatusCategory::New,
        pull_request_lookup: JiraPullRequestLookup::Unsupported,
        connection: ConnectionRef {
            provider: Provider::Jira,
            profile_id: "a".repeat(64),
            connection_id: "c".repeat(32),
            revision: 1,
        },
    });
    current
}

fn jira_policy(mode: IssueAutomationAutonomy) -> IssueAutomationPolicy {
    let mut policy = policy();
    policy.binding.jira = Some(
        crate::domain::issue_automation::policy::IssueAutomationJiraBinding {
            site_url: "https://example.atlassian.net".into(),
            cloud_id: "cloud-123".into(),
            project_id: "10001".into(),
            expected_account_id: "U_trusted".into(),
            jql: "statusCategory != Done".into(),
            mode,
            close_transition: None,
        },
    );
    policy.feedback.labels = false;
    policy.select.trusted_actors[0].provider = IssueProvider::Jira;
    policy.select.trusted_actors[0].server_id = "cloud-123".into();
    policy
}

fn assess(
    current: &IssueSourceRead,
    policy: &IssueAutomationPolicy,
    previous: Option<&IssueTriageAssessment>,
) -> IssueTriageAssessment {
    let reference = match current {
        IssueSourceRead::Observed(observation) => &observation.issue,
        IssueSourceRead::Unknown { issue, .. } => issue,
    };
    triage_issue(input(
        reference,
        Some(current),
        policy,
        &IssueRequirements::default(),
        previous,
    ))
    .unwrap()
}

#[test]
fn unsupported_jira_pr_lookup_keeps_unknown_and_opens_only_explicit_human_start() {
    let source = jira_source();
    for mode in [
        IssueAutomationAutonomy::Off,
        IssueAutomationAutonomy::Observe,
        IssueAutomationAutonomy::Propose,
        IssueAutomationAutonomy::Gated,
        IssueAutomationAutonomy::Auto,
    ] {
        let assessment = assess(&source, &jira_policy(mode), None);
        assert_eq!(assessment.eligibility(), IssueTriageEligibility::Unknown);
        assert_eq!(assessment.readiness(), IssueTriageReadiness::Ready);
        assert!(assessment.requires_jira_human_start_review());
        assert!(!ready_for_start(&assessment));
        assert!(!policy_start(&assessment));
        let can_review = mode >= IssueAutomationAutonomy::Propose;
        assert_eq!(ready_for_human_start(&assessment), can_review);
        for gate in assessment.gates() {
            assert_eq!(gate.action, IssueAutomationGateAction::Human);
            assert_eq!(
                gate.enabled,
                can_review && gate.gate == IssueAutomationGate::Start
            );
        }
        assert_eq!(
            next_action(IssueCoordinatorInput {
                authorization_epoch: 0,
                state: crate::domain::issue_automation::IssueAutomationState::Discovered,
                flags: Default::default(),
                assessment: Some(&assessment),
                decision: None,
                work: None,
                frozen_source: None,
                now_unix_ms: u64::MAX,
            }),
            if can_review {
                IssueCoordinatorAction::OpenDecision
            } else {
                IssueCoordinatorAction::None
            }
        );
    }
}

#[test]
fn jira_pr_review_never_covers_other_unknowns_or_unsupported_content() {
    let policy = jira_policy(IssueAutomationAutonomy::Gated);
    for case in 0..12 {
        let mut current = jira_source();
        let value = observed(&mut current);
        match case {
            0 => value.assignees.completeness = IssueMetadataCompleteness::Unavailable,
            1 => value.labels.completeness = IssueMetadataCompleteness::Truncated,
            2 => value.author = None,
            3 => value.content_problems = vec![IssueSourceContentProblem::UnsupportedAdf],
            4 => value.content_problems = vec![IssueSourceContentProblem::UnsupportedRequiredField],
            5 => value.content_problems = vec![IssueSourceContentProblem::MissingRequiredField],
            6 => value.assignees.items.push(actor("someone")),
            7 => value.remote_state = IssueRemoteState::Closed,
            8 => {
                value.jira.as_mut().unwrap().pull_request_lookup = JiraPullRequestLookup::Available
            }
            9 => value.jira = None,
            10 => {
                value.jira = None;
                value.linked_pull_request = IssuePrPresence::Absent;
            }
            11 => value.linked_pull_request = IssuePrPresence::Absent,
            _ => unreachable!(),
        }
        let assessment = assess(&current, &policy, None);
        assert!(
            !assessment.requires_jira_human_start_review(),
            "case {case}"
        );
        assert!(!ready_for_human_start(&assessment), "case {case}");
        assert!(assessment.gates().iter().all(|gate| !gate.enabled));
        if (3..=5).contains(&case) {
            assert_eq!(assessment.readiness(), IssueTriageReadiness::NeedsInfo);
            assert_eq!(
                assessment.observation().disposition,
                IssueObservationDisposition::NeedsInfo
            );
        }
    }
    let reference = observed(&mut jira_source()).issue.clone();
    for reason in [
        IssueSourceFailure::ProviderUnavailable,
        IssueSourceFailure::MetadataIncomplete,
        IssueSourceFailure::IdentityMismatch,
        IssueSourceFailure::MalformedResponse,
    ] {
        let current = IssueSourceRead::Unknown {
            issue: reference.clone(),
            reason,
        };
        let assessment = assess(&current, &policy, None);
        assert!(!ready_for_human_start(&assessment));
        assert_eq!(assessment.readiness(), IssueTriageReadiness::Unknown);
        assert!(assessment.gates().iter().all(|gate| !gate.enabled));
    }
}

#[test]
fn jira_status_key_and_provider_limits_expire_the_basis_but_display_and_connection_do_not() {
    let current = jira_source();
    let policy = jira_policy(IssueAutomationAutonomy::Propose);
    let original = assess(&current, &policy, None);
    for case in 0..5 {
        let mut changed = current.clone();
        let value = observed(&mut changed);
        let jira = value.jira.as_mut().unwrap();
        match case {
            0 => jira.status_id = "10001".into(),
            1 => jira.status_category = JiraIssueStatusCategory::Indeterminate,
            2 => {
                jira.key = "AKRA-43".into();
                jira.url = "https://example.atlassian.net/browse/AKRA-43".into();
            }
            3 => jira.pull_request_lookup = JiraPullRequestLookup::Available,
            4 => value.content_problems = vec![IssueSourceContentProblem::UnsupportedAdf],
            _ => unreachable!(),
        }
        let next = assess(&changed, &policy, Some(&original));
        assert_ne!(
            next.basis().evidence_sha256,
            original.basis().evidence_sha256,
            "case {case}"
        );
        assert_eq!(
            next.gates()[0].gate_revision,
            original.gates()[0].gate_revision + 1
        );
        assert!(
            original
                .validate_current(&original.basis().issue, Some(&changed), false)
                .is_err()
        );
    }
    let mut display = current;
    let jira = observed(&mut display).jira.as_mut().unwrap();
    jira.url = "https://renamed.atlassian.net/browse/AKRA-42".into();
    jira.connection.connection_id = "e".repeat(32);
    jira.connection.revision += 1;
    assert_eq!(assess(&display, &policy, Some(&original)), original);
}

#[test]
fn persisted_jira_review_cannot_claim_automatic_eligibility_or_enable_another_gate() {
    let original = assess(
        &jira_source(),
        &jira_policy(IssueAutomationAutonomy::Propose),
        None,
    );
    let mut record = original.to_record();
    record.eligibility = IssueTriageEligibility::Eligible;
    assert!(IssueTriageAssessment::hydrate(record).is_err());
    let mut record = original.to_record();
    record.gates[0].action = IssueAutomationGateAction::Auto;
    assert!(IssueTriageAssessment::hydrate(record).is_err());
    let mut record = original.to_record();
    record.gates[1].enabled = true;
    assert!(IssueTriageAssessment::hydrate(record).is_err());
    let mut record = original.to_record();
    record
        .reasons
        .push(IssueTriageReasonCode::MetadataIncomplete);
    record.reasons.sort();
    assert!(IssueTriageAssessment::hydrate(record).is_err());
}
