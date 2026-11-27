use super::*;
use crate::domain::issue_automation::decision::IssueDecisionKind;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};
use crate::domain::kernel_event::{
    IssueDecisionBasisRecord, IssueDecisionFactRecord, IssueDecisionSubjectRecord,
};

fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_project".into(),
        issue_id: "I_42".into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".into(),
        },
    }
}

fn tracked(issue: &IssueReference, sequence: i64) -> RecordedKernelEvent {
    RecordedKernelEvent {
        sequence,
        // 표시 시각이 역전되어도 실제 기록 순서가 우선한다.
        recorded_at: format!("2026-11-22T00:00:{:02}Z", 59 - sequence),
        event: KernelEvent::issue_tracked(issue, "binding-1", sequence as u64),
    }
}

fn decision(issue: &IssueReference) -> KernelEvent {
    KernelEvent::IssueDecisionRequested {
        decision: IssueDecisionFactRecord {
            delivery_review: None,
            decision_id: "decision-1".into(),
            subject: IssueDecisionSubjectRecord::Issue {
                issue: issue.into(),
            },
            gate: IssueDecisionGateRecord::Close,
            kind: IssueDecisionKind::CloseApproval,
            basis: IssueDecisionBasisRecord {
                delivery: None,
                policy_version: "policy-1".into(),
                field_hash: None,
                work_order_id: Some("work-1".into()),
                gate_revision: 1,
                authorization_epoch: 1,
            },
            state: IssueDecisionState::Open,
            answer_by: None,
            option: None,
        },
        revision: 1,
    }
}

#[test]
fn issue_timeline_matches_all_coordinates_in_regular_and_decision_events() {
    let expected = issue();
    let mut other_provider = expected.clone();
    other_provider.provider = IssueProvider::Jira;
    let mut other_server = expected.clone();
    other_server.server_id = "github.example".into();
    let mut other_repository = expected.clone();
    other_repository.repository_id = "R_other".into();
    let mut other_issue = expected.clone();
    other_issue.issue_id = "I_other".into();
    let mut other_workspace = expected.clone();
    other_workspace.project_scope.workspace_dir = "/other-workspace".into();

    for reference in [
        other_provider,
        other_server,
        other_repository,
        other_issue,
        other_workspace,
    ] {
        assert!(!issue_event_matches(
            &tracked(&reference, 1).event,
            &expected
        ));
        assert!(!issue_event_matches(&decision(&reference), &expected));
    }
    assert!(issue_event_matches(&tracked(&expected, 1).event, &expected));
    assert!(issue_event_matches(&decision(&expected), &expected));
    assert!(!issue_event_matches(
        &KernelEvent::TurnStarted {
            thread_id: expected.issue_id.clone(),
            turn_id: "turn-1".into(),
            op_id: None,
        },
        &expected,
    ));
}

#[test]
fn issue_timeline_merges_persisted_and_recent_records_before_taking_twenty() {
    let issue = issue();
    let stored = (1..=21)
        .map(|sequence| tracked(&issue, sequence))
        .collect::<Vec<_>>();
    let recent = (19..=25)
        .map(|sequence| tracked(&issue, sequence))
        .collect::<Vec<_>>();
    let IssueTimelineProjection::Ready { entries, has_older } =
        project_issue_timeline(&issue, &stored, &recent)
    else {
        panic!("consistent records should be available");
    };
    assert!(has_older);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>(),
        (6..=25).collect::<Vec<_>>(),
    );
    assert!(entries[0].recorded_at > entries[19].recorded_at);
    assert_eq!(entries.last().unwrap().kind, "issue_tracked");
}

#[test]
fn issue_timeline_twenty_records_and_duplicates_do_not_claim_older_history() {
    let issue = issue();
    let stored = (1..=20)
        .map(|sequence| tracked(&issue, sequence))
        .collect::<Vec<_>>();
    let IssueTimelineProjection::Ready { entries, has_older } =
        project_issue_timeline(&issue, &stored, &stored)
    else {
        panic!("duplicate copies should be deduplicated");
    };
    assert_eq!(entries.len(), 20);
    assert!(!has_older);
    assert_eq!(
        project_issue_timeline(&issue, &[], std::iter::empty()),
        IssueTimelineProjection::Ready {
            entries: Vec::new(),
            has_older: false
        },
    );
}

#[test]
fn issue_timeline_conflicting_sequence_is_unavailable_even_across_issue_coordinates() {
    let issue = issue();
    let stored = vec![tracked(&issue, 1)];
    let mut conflicting = stored[0].clone();
    conflicting.recorded_at = "2026-11-22T12:00:00Z".into();
    assert_eq!(
        project_issue_timeline(&issue, &stored, [&conflicting]),
        IssueTimelineProjection::Unavailable,
    );
    let mut other = issue.clone();
    other.repository_id = "R_other".into();
    let conflicting = tracked(&other, 1);
    assert_eq!(
        project_issue_timeline(&issue, &stored, [&conflicting]),
        IssueTimelineProjection::Unavailable,
    );
}

#[test]
fn issue_timeline_summary_distinguishes_external_close_without_exposing_provider_metadata() {
    let issue = issue();
    let record = |sequence, origin| RecordedKernelEvent {
        sequence,
        recorded_at: "2026-11-22T00:00:00Z".into(),
        event: KernelEvent::IssueClosed {
            event_id: "private-outbox-id".into(),
            issue: (&issue).into(),
            origin,
            intent_id: Some("private-intent-id".into()),
            actor_id: Some("private-actor-id".into()),
            closure_event_id: Some("private-provider-event-id".into()),
            observation_revision: Some(1),
            revision: 1,
        },
    };
    let stored = [
        record(1, IssueClosureRecordOrigin::ProviderObservation),
        record(2, IssueClosureRecordOrigin::ConfirmedFeedback),
    ];
    let IssueTimelineProjection::Ready { entries, .. } =
        project_issue_timeline(&issue, &stored, std::iter::empty())
    else {
        panic!("confirmed records should be available");
    };
    assert_eq!(
        entries[0].summary,
        "외부에서 이슈가 닫힌 상태를 관찰했습니다."
    );
    assert_eq!(entries[1].summary, "승인된 이슈 닫기 전송을 확인했습니다.");
    assert!(
        entries
            .iter()
            .all(|entry| !entry.summary.contains("private-"))
    );
}

#[test]
fn issue_timeline_execution_summary_keeps_execution_and_verification_counts_separate() {
    let progress = IssueExecutionProgress {
        state: IssueExecutionState::InReview,
        attempts: crate::domain::issue_automation::execution::IssueExecutionAttempts {
            execution: 2,
            verification: 7,
            publication: 99,
        },
        ..IssueExecutionProgress::default()
    };
    assert_eq!(
        execution_summary(&progress),
        "PR 검증을 진행 중입니다. 실행 2회 · 검증 7회",
    );
}

#[test]
fn issue_timeline_replays_frozen_journal_with_the_same_event_tags_revisions_and_states() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/issue_automation_timeline_journal_v1.json"
    ))
    .unwrap();
    let records = fixture
        .as_array()
        .unwrap()
        .iter()
        .map(|record| {
            let event: KernelEvent = serde_json::from_value(record["event"].clone()).unwrap();
            assert_eq!(serde_json::to_value(&event).unwrap(), record["event"]);
            RecordedKernelEvent {
                sequence: record["sequence"].as_i64().unwrap(),
                recorded_at: record["recorded_at"].as_str().unwrap().into(),
                event,
            }
        })
        .collect::<Vec<_>>();

    let restored = project_issue_timeline(&issue(), &records, std::iter::empty());
    assert_eq!(
        project_issue_timeline(&issue(), &records[..2], records.iter().skip(1)),
        restored,
        "replayed durable records and recent duplicates must produce the same timeline"
    );
    let IssueTimelineProjection::Ready { entries, has_older } = restored else {
        panic!("the frozen journal remains readable");
    };
    assert!(!has_older);
    assert_eq!(
        entries.iter().map(|entry| entry.sequence).collect::<Vec<_>>(),
        [7, 8, 9]
    );
    assert_eq!(entries[0].kind, "issue_tracked");
    assert_eq!(entries[1].kind, "issue_work_permit_revoked");
    assert_eq!(entries[2].kind, "issue_execution_progressed");
    assert_eq!(entries[1].summary, "기존 작업 허가를 철회했습니다.");
    assert_eq!(
        entries[2].summary,
        "검증 결과를 다시 확인해야 합니다. 실행 2회 · 검증 7회"
    );
}
