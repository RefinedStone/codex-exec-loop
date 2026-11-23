use super::*;
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::domain::issue_automation::triage::{IssueTriageEligibility, IssueTriageReadiness};
use crate::domain::provider_credentials::{ConnectionRef, Provider};

const JIRA_POLICY: &str = r#"
[binding.jira]
site_url = "https://example.atlassian.net"
cloud_id = "cloud-123"
project_id = "10001"
expected_account_id = "U_author"
jql = "statusCategory != Done"
mode = "propose"
[authority]
autonomy_cap = "propose"
[select]
mode = "all_open"
trusted_actors = [{provider="jira",server_id="cloud-123",actor_id="U_author"}]
"#;

fn harness(edit: impl FnOnce(&mut IssueSourceObservation)) -> Harness {
    let h = Harness::with_observation(|value| {
        value.issue.provider = IssueProvider::Jira;
        value.issue.server_id = "cloud-123".into();
        value.issue.repository_id = "10001".into();
        value.issue.issue_id = "10002".into();
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
        edit(value);
    });
    *h.policy.text.lock().unwrap() = JIRA_POLICY.into();
    run(&h.triage);
    h
}

fn reference(h: &Harness) -> IssueReference {
    page(&h.triage).items[0].issue.clone()
}

fn work(h: &Harness) -> Option<IssueWorkCheckpoint> {
    h.fixture
        .store()
        .get_issue_work(&h.fixture.workspace, &reference(h))
        .unwrap()
}

#[test]
fn jira_pr_lookup_requires_native_human_confirmation_and_replays_the_exact_start_receipt() {
    let h = harness(|_| {});
    h.tick(false).unwrap();
    let first = h.decision();
    assert_eq!(
        first.request.definition().on_timeout,
        IssueDecisionTimeout::Wait
    );
    assert!(first.request.definition().recommended.is_none());
    let preview = h
        .decisions
        .prepare_confirmation(first.request.id().as_str(), "start")
        .unwrap();
    assert!(
        preview
            .display
            .message
            .contains("PR 유무를 확인할 수 없습니다")
    );
    assert!(
        preview
            .display
            .detail
            .contains("중복 작업이 없는지 직접 확인")
    );
    assert!(
        preview
            .display
            .detail
            .contains("자동 시작이나 자동 통합을 허용하지")
    );
    assert!(
        h.decisions
            .answer(
                first.request.id().as_str(),
                "start",
                None,
                &StoredCommandContext {
                    human_confirmation: None,
                    caller: Some("Desktop".into()),
                    channel: "gateway".into(),
                    at: "2026-11-21T00:00:00Z".into(),
                }
            )
            .is_err()
    );
    assert!(work(&h).is_none());
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);

    h.answer("start");
    h.tick(false).unwrap();
    let started = work(&h).expect("native Human approval admits current Jira work");
    assert_eq!(started.start_authority, IssueWorkStartAuthority::Human);
    assert_eq!(started.decision_id.as_ref(), Some(first.request.id()));
    assert_eq!(started.task_ids, ["task-1"]);
    assert_eq!(
        started.order.link_reference.as_deref(),
        Some("https://example.atlassian.net/browse/AKRA-42")
    );
    let item = &page(&h.triage).items[0];
    assert_eq!(
        item.triage.as_ref().unwrap().assessment.eligibility(),
        IssueTriageEligibility::Unknown
    );
    assert!(
        matches!(&item.current, Some(IssueSourceRead::Observed(value)) if value.linked_pull_request == IssuePrPresence::Unknown)
    );
    h.tick(false).unwrap();
    assert_eq!(work(&h).unwrap().order, started.order);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn sqlite_jira_start_admission_rejects_policy_default_and_arbiter_authorities() {
    let h = harness(|_| {});
    h.tick(false).unwrap();
    let decision = h.decision();
    h.answer("start");
    let coordinator = h.coordinator();
    let (fence, item, assessment) = coordinator.current(&reference(&h), &|_| {}).unwrap();
    let answer = h
        .fixture
        .store()
        .get_issue_decision(&h.fixture.workspace, decision.request.id())
        .unwrap()
        .unwrap();
    let human = coordinator
        .build_work(&item, StartAuthorization::Human(&answer.request))
        .unwrap();
    for authority in [
        IssueWorkStartAuthority::Policy,
        IssueWorkStartAuthority::TimeoutDefault,
        IssueWorkStartAuthority::TimeoutArbiter,
    ] {
        let mut invalid = human.clone();
        invalid.start_authority = authority;
        if authority == IssueWorkStartAuthority::Policy {
            invalid.decision_id = None;
        }
        assert!(
            h.fixture
                .store()
                .admit_issue_work(
                    &h.fixture.workspace,
                    fence,
                    &assessment,
                    &invalid,
                    &crate::domain::issue_automation::policy::IssueAutomationPolicy::builtin()
                        .budget,
                    123,
                )
                .is_err(),
            "authority {authority:?}"
        );
        assert!(work(&h).is_none());
    }
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    h.tick(false).unwrap();
    assert_eq!(
        work(&h).unwrap().start_authority,
        IssueWorkStartAuthority::Human
    );
}

#[test]
fn jira_other_unknowns_and_unsupported_adf_create_no_start_decision_or_work() {
    for case in 0..6 {
        let h = harness(|value| match case {
            0 => value.assignees.completeness = IssueMetadataCompleteness::Unavailable,
            1 => value.content_problems = vec![IssueSourceContentProblem::UnsupportedAdf],
            2 => value.content_problems = vec![IssueSourceContentProblem::MissingRequiredField],
            3 => {
                value.jira.as_mut().unwrap().pull_request_lookup = JiraPullRequestLookup::Available
            }
            4 => value.author = None,
            5 => value.jira = None,
            _ => unreachable!(),
        });
        h.tick(false).unwrap();
        let item = &page(&h.triage).items[0];
        assert!(item.open_decisions.is_empty(), "case {case}");
        assert!(work(&h).is_none(), "case {case}");
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
        if matches!(case, 1 | 2) {
            assert_eq!(
                item.triage.as_ref().unwrap().assessment.readiness(),
                IssueTriageReadiness::NeedsInfo
            );
        }
    }
}

#[test]
fn fresh_jira_unavailability_key_change_or_new_unknown_supersedes_human_start_before_submission() {
    for case in 0..4 {
        let h = harness(|_| {});
        h.tick(false).unwrap();
        let decision = h.decision();
        h.answer("start");
        let mut source = h.source.read.lock().unwrap();
        if case == 0 {
            *source = IssueSourceRead::Unknown {
                issue: reference(&h),
                reason: IssueSourceFailure::ProviderUnavailable,
            };
        } else {
            let IssueSourceRead::Observed(value) = &mut *source else {
                unreachable!()
            };
            match case {
                1 => {
                    let jira = value.jira.as_mut().unwrap();
                    jira.key = "AKRA-43".into();
                    jira.url = "https://example.atlassian.net/browse/AKRA-43".into();
                }
                2 => value.assignees.completeness = IssueMetadataCompleteness::Unavailable,
                3 => value.content_problems = vec![IssueSourceContentProblem::UnsupportedAdf],
                _ => unreachable!(),
            }
        }
        drop(source);
        h.tick(false).unwrap();
        assert!(work(&h).is_none(), "case {case}");
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
        let original = h
            .fixture
            .store()
            .get_issue_decision(&h.fixture.workspace, decision.request.id())
            .unwrap()
            .unwrap();
        assert_eq!(
            original.request.answer().unwrap().by,
            IssueDecisionAnswerBy::Human
        );
        assert_eq!(
            original.request.definition().basis,
            decision.request.definition().basis,
            "the historical Human receipt is preserved"
        );
    }
}
