use std::fs;
use std::path::{Path, PathBuf};

use super::super::super as authority;
use super::*;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::domain::issue_automation::decision::{IssueDecisionKind, IssueDecisionState};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};
use crate::domain::kernel_event::{
    IssueDecisionBasisRecord, IssueDecisionFactRecord, IssueDecisionGateRecord,
    IssueDecisionSubjectRecord,
};
use crate::domain::planning::PlanningAuthorityLocation;

struct Fixture {
    workspace: String,
    location: PlanningAuthorityLocation,
}

impl Fixture {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-timeline-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&path).unwrap();
        let workspace = path.display().to_string();
        let location =
            SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace)
                .unwrap();
        Self {
            workspace,
            location,
        }
    }

    fn issue(&self) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_project".into(),
            issue_id: "I_42".into(),
            project_scope: IssueProjectScope {
                workspace_dir: self.workspace.clone(),
            },
        }
    }

    fn load(&self, limit: usize) -> Result<Option<Vec<RecordedKernelEvent>>> {
        SqlitePlanningAuthorityAdapter::new().load_recent_issue_kernel_events(
            &self.workspace,
            &self.issue(),
            limit,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(project) = Path::new(&self.location.runtime_dir).parent() {
            let _ = fs::remove_dir_all(project);
        }
        let _ = fs::remove_dir_all(&self.workspace);
    }
}

fn recorded(sequence: i64, event: KernelEvent) -> RecordedKernelEvent {
    RecordedKernelEvent {
        sequence,
        recorded_at: "2026-11-22T00:00:00Z".into(),
        event,
    }
}

fn tracked(issue: &IssueReference, sequence: i64) -> RecordedKernelEvent {
    recorded(
        sequence,
        KernelEvent::issue_tracked(issue, "binding-1", sequence as u64),
    )
}

fn decision(issue: &IssueReference, sequence: i64) -> RecordedKernelEvent {
    recorded(
        sequence,
        KernelEvent::IssueDecisionRequested {
            decision: IssueDecisionFactRecord {
                delivery_review: None,
                decision_id: format!("decision-{sequence}"),
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
            revision: sequence as u64,
        },
    )
}

#[test]
fn issue_timeline_db_filters_full_scope_before_limiting_and_includes_nested_decisions() {
    let fixture = Fixture::new();
    let issue = fixture.issue();
    let mut records = (1..=30)
        .map(|sequence| tracked(&issue, sequence))
        .collect::<Vec<_>>();
    let mut references = vec![issue.clone(); 5];
    references[0].provider = IssueProvider::Jira;
    references[1].server_id = "github.example".into();
    references[2].repository_id = "R_other".into();
    references[3].issue_id = "I_other".into();
    references[4].project_scope.workspace_dir = "/another-workspace".into();
    for (index, other) in references.iter().enumerate() {
        records.push(tracked(other, 31 + index as i64));
        records.push(decision(other, 36 + index as i64));
    }
    records.push(decision(&issue, 41));
    let mut resolved = decision(&issue, 50);
    let KernelEvent::IssueDecisionRequested {
        mut decision,
        revision,
    } = resolved.event
    else {
        unreachable!();
    };
    decision.state = IssueDecisionState::Answered;
    resolved.event = KernelEvent::IssueDecisionResolved {
        decision,
        revision,
        reason: None,
    };
    records.push(resolved);
    records.extend((60..=90).map(|sequence| {
        recorded(
            sequence,
            KernelEvent::TurnStarted {
                thread_id: issue.issue_id.clone(),
                turn_id: format!("turn-{sequence}"),
                op_id: None,
            },
        )
    }));
    SqlitePlanningAuthorityAdapter::new()
        .append_kernel_events(&fixture.workspace, &records)
        .unwrap();

    let result = fixture.load(usize::MAX).unwrap().unwrap();
    assert_eq!(
        result
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        (12..=30).chain([41, 50]).collect::<Vec<_>>(),
    );
    assert!(
        result
            .iter()
            .all(|record| issue_event_matches(&record.event, &issue))
    );
    assert_eq!(fixture.load(1).unwrap().unwrap()[0].sequence, 50);
}

#[test]
fn issue_timeline_db_does_not_create_a_store_or_event_table_on_read() {
    let fixture = Fixture::new();
    assert_eq!(fixture.load(21).unwrap(), Some(Vec::new()));
    assert!(!Path::new(&fixture.location.runtime_dir).exists());

    // 기존 authority schema만 준비한다. timeline은 없는 kernel_events 표를 만들지 않는다.
    let connection = authority::open_authority_connection(&fixture.location).unwrap();
    drop(connection);
    let before = fs::read(&fixture.location.authority_store_path).unwrap();
    assert_eq!(fixture.load(21).unwrap(), Some(Vec::new()));
    assert_eq!(
        fs::read(&fixture.location.authority_store_path).unwrap(),
        before
    );
    let store = read_only::open(&fixture.location).unwrap().unwrap();
    let exists: bool = store
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='kernel_events')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
}

#[test]
fn issue_timeline_db_does_not_create_repository_identity_for_an_uninitialized_git_store() {
    let fixture = Fixture::new();
    let mut command = crate::git_subprocess::command(["init", "--quiet"]);
    command.current_dir(&fixture.workspace);
    let output =
        crate::subprocess::command_output(&mut command, "initialize timeline test repository")
            .unwrap();
    assert!(output.status.success());
    let marker = Path::new(&fixture.workspace).join(".git/akra-repository-id");
    assert!(!marker.exists());
    assert!(fixture.load(21).unwrap().is_none());
    assert!(!marker.exists());
    assert!(!Path::new(&fixture.location.runtime_dir).exists());
}

#[test]
fn issue_timeline_db_reads_existing_git_store_using_its_existing_repository_identity() {
    let fixture = Fixture::new();
    let mut command = crate::git_subprocess::command(["init", "--quiet"]);
    command.current_dir(&fixture.workspace);
    assert!(
        crate::subprocess::command_output(&mut command, "initialize timeline test repository")
            .unwrap()
            .status
            .success()
    );
    let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
        &fixture.workspace,
    )
    .unwrap();
    let record = tracked(&fixture.issue(), 1);
    SqlitePlanningAuthorityAdapter::new()
        .append_kernel_events(&fixture.workspace, std::slice::from_ref(&record))
        .unwrap();
    let marker = Path::new(&fixture.workspace).join(".git/akra-repository-id");
    let before = fs::read(&marker).unwrap();
    assert_eq!(fixture.load(21).unwrap(), Some(vec![record]));
    assert_eq!(fs::read(&marker).unwrap(), before);
    // Git을 초기화하며 선택한 incarnation은 non-Git fixture의 namespace와 다를 수 있다.
    if location.runtime_dir != fixture.location.runtime_dir
        && let Some(project) = Path::new(&location.runtime_dir).parent()
    {
        let _ = fs::remove_dir_all(project);
    }
}

#[test]
fn issue_timeline_db_rejects_wal_without_creating_shared_memory() {
    let fixture = Fixture::new();
    SqlitePlanningAuthorityAdapter::new()
        .append_kernel_events(&fixture.workspace, &[tracked(&fixture.issue(), 1)])
        .unwrap();
    let connection = authority::open_authority_connection(&fixture.location).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    drop(connection);
    let shm = PathBuf::from(format!("{}-shm", fixture.location.authority_store_path));
    assert!(!shm.exists());
    assert!(fixture.load(21).is_err());
    assert!(!shm.exists());
}
