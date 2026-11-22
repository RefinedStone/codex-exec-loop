use super::super::tests::{Fixture, transition_and_save};
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::domain::issue_automation::coordinator::{IssueWorkOrder, issue_work_order_id};
use crate::domain::issue_automation::decision::{IssueDecisionBasis, IssueDecisionId};
use crate::domain::issue_automation::{IssueAutomationFlag, IssueTriageResult};
use std::sync::{Arc, Barrier};

fn seed(fixture: &Fixture) -> IssueWorkCheckpoint {
    let adapter = fixture.adapter();
    adapter.ensure_collection(fixture.cwd()).unwrap();
    let (_, tracked) = fixture.insert();
    let tracked = transition_and_save(
        &adapter,
        fixture.cwd(),
        &tracked,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let tracked = transition_and_save(&adapter, fixture.cwd(), &tracked, IssueTransition::Queue);
    let issue = tracked.reference().clone();
    let field_hash = tracked.snapshot().basis.field_hash.clone();
    let handoff = tracked.snapshot().shared_copy.clone();
    let work = IssueWorkCheckpoint {
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            work_order_id: issue_work_order_id(&issue),
            issue: issue.clone(),
            field_hash: field_hash.clone(),
            title: handoff.title.clone(),
            handoff,
            acceptance: vec!["기존 승인한 작업을 검증한다.".into()],
            scope: fixture.cwd().into(),
            branch_hint: None,
            link_reference: None,
            provider_link: false,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        decision_id: IssueDecisionId::new("fixture-start").unwrap(),
        approved_basis: IssueDecisionBasis {
            authorization_epoch: 0,
            policy_version: "a".repeat(64),
            field_hash: Some(field_hash),
            work_order_id: None,
            gate_revision: 1,
        },
        task_ids: vec!["task-1".into()],
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        blocked_reason: None,
        revision: 1,
    };
    work.validate().unwrap();
    adapter
        .with_connection(fixture.cwd(), |connection, guard| {
            transaction(connection, guard, |tx| {
                tx.execute(
                    "INSERT INTO issue_work_checkpoints
                     (workspace_dir,provider,server_id,repository_id,issue_id,
                      work_order_id,checkpoint,revision) VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
                    params![
                        fixture.cwd(),
                        super::super::enum_name(&issue.provider)?,
                        issue.server_id,
                        issue.repository_id,
                        issue.issue_id,
                        work.order.work_order_id,
                        serde_json::to_string(&work)?
                    ],
                )?;
                Ok(())
            })
        })
        .unwrap();
    work
}

fn running(work: &IssueWorkCheckpoint) -> IssueWorkExecutionObservation {
    IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Running,
        task_ids: work.task_ids.clone(),
        active_task_id: Some("task-1".into()),
        execution_ids: vec!["task-1".into()],
        ..Default::default()
    }
}

fn saved(result: IssueExecutionSaveResult) -> IssueExecutionRecord {
    match result {
        IssueExecutionSaveResult::Saved { record, .. } => *record,
        other => panic!("expected a saved execution observation, got {other:?}"),
    }
}

#[test]
fn receipt_replay_after_reopen_preserves_separate_counters_and_catalog() {
    let fixture = Fixture::new("execution-replay");
    let work = seed(&fixture);
    let mut observed = running(&work);
    observed.phase = IssueExecutionPhase::Terminal;
    observed.terminals = vec![IssueExecutionTerminalRecord {
        record_id: "verification-1".into(),
        classification: IssueExecutionTerminalClass::ObserveAgain,
    }];
    observed.current_terminal_id = Some("verification-1".into());
    let first = saved(
        fixture
            .adapter()
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(1))
            .unwrap(),
    );
    assert_eq!(
        first.progress.attempts,
        IssueExecutionAttempts {
            execution: 1,
            verification: 1,
            publication: 0
        }
    );
    assert!(!first.progress.attempts_exhausted);
    assert_eq!(first.progress.state, IssueExecutionState::InReview);
    let events = fixture
        .adapter()
        .pending_issue_work_events(fixture.cwd())
        .unwrap();
    let reopened = fixture.adapter();
    let repeated = reopened
        .apply_issue_execution(
            fixture.cwd(),
            &work,
            first.progress.revision,
            &observed,
            Some(1),
        )
        .unwrap();
    assert!(matches!(
        repeated,
        IssueExecutionSaveResult::Saved { changed: false, .. }
    ));
    assert_eq!(saved(repeated), first);
    assert_eq!(
        reopened.pending_issue_work_events(fixture.cwd()).unwrap(),
        events
    );
}

#[test]
fn conflicting_terminal_identity_rolls_back_new_attempt_and_lineage() {
    let fixture = Fixture::new("execution-immutable-terminal");
    let work = seed(&fixture);
    let mut observed = running(&work);
    observed.terminals = vec![IssueExecutionTerminalRecord {
        record_id: "terminal-1".into(),
        classification: IssueExecutionTerminalClass::ObserveAgain,
    }];
    let adapter = fixture.adapter();
    let first = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(3))
            .unwrap(),
    );
    observed.task_ids.push("task-2".into());
    observed.execution_ids.push("task-2".into());
    observed.terminals[0].classification = IssueExecutionTerminalClass::ReworkNeeded;
    assert!(
        adapter
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                first.progress.revision,
                &observed,
                Some(3),
            )
            .is_err()
    );
    assert_eq!(
        adapter
            .get_issue_execution(fixture.cwd(), &work.order.issue)
            .unwrap(),
        Some(first)
    );
    assert_eq!(
        adapter
            .get_issue_work(fixture.cwd(), &work.order.issue)
            .unwrap(),
        Some(work.clone())
    );
    let count: i64 = adapter
        .with_connection(fixture.cwd(), |connection, _| {
            Ok(connection.query_row(
                "SELECT count(*) FROM issue_execution_receipts WHERE kind='execution'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn competing_sqlite_writers_count_a_started_task_once() {
    let fixture = Fixture::new("execution-concurrent-receipt");
    let work = seed(&fixture);
    let observed = running(&work);
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let writers = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let adapter = fixture.adapter();
                let work = work.clone();
                let observed = observed.clone();
                let cwd = fixture.cwd().to_owned();
                scope.spawn(move || {
                    barrier.wait();
                    adapter
                        .apply_issue_execution(&cwd, &work, 0, &observed, Some(3))
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueExecutionSaveResult::Saved { .. }))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueExecutionSaveResult::Conflict))
            .count(),
        1
    );
    let record = fixture
        .adapter()
        .get_issue_execution(fixture.cwd(), &work.order.issue)
        .unwrap()
        .unwrap();
    assert_eq!(record.progress.attempts.execution, 1);
    assert_eq!(record.progress.revision, 1);
    assert_eq!(
        fixture
            .adapter()
            .pending_issue_work_events(fixture.cwd())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn trusted_integration_is_recorded_once_and_later_contract_recovery_stays_done() {
    let fixture = Fixture::new("execution-integration");
    let work = seed(&fixture);
    let adapter = fixture.adapter();
    let mut tracked = adapter
        .get_issue(fixture.cwd(), &work.order.issue)
        .unwrap()
        .unwrap();
    tracked.set_flag(IssueAutomationFlag::Held, true);
    super::super::tests::save_changed(&adapter, fixture.cwd(), &tracked);
    let mut observed = running(&work);
    observed.phase = IssueExecutionPhase::Integrated;
    observed.integration = Some(IssueExecutionIntegration {
        record_id: "integration-evidence-1".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: Some("https://github.com/acme/project/pull/1".into()),
    });
    let first = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(1))
            .unwrap(),
    );
    assert_eq!(first.progress.state, IssueExecutionState::Done);
    let tracked = adapter
        .get_issue(fixture.cwd(), &work.order.issue)
        .unwrap()
        .unwrap();
    assert_eq!(tracked.state(), IssueAutomationState::Done);
    assert!(tracked.flags().held);
    observed.phase = IssueExecutionPhase::Terminal;
    observed.terminals = vec![IssueExecutionTerminalRecord {
        record_id: "post-merge-contract".into(),
        classification: IssueExecutionTerminalClass::ContractRecovery,
    }];
    observed.current_terminal_id = Some("post-merge-contract".into());
    let later = saved(
        fixture
            .adapter()
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                first.progress.revision,
                &observed,
                Some(1),
            )
            .unwrap(),
    );
    assert_eq!(later.progress.state, IssueExecutionState::Done);
    assert_eq!(later.progress.attempts.execution, 1);
    assert!(!later.progress.allows_additional_execution(100));
    let events = adapter.pending_issue_work_events(fixture.cwd()).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, IssueWorkEventKind::Integrated { .. }))
            .count(),
        1
    );
}

#[test]
fn confirmed_lineage_append_requires_a_fresh_work_cas_and_never_drops_tasks() {
    let fixture = Fixture::new("execution-lineage-cas");
    let work = seed(&fixture);
    let adapter = fixture.adapter();
    let mut observed = running(&work);
    observed.task_ids.push("supplement-1".into());
    let first = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(3))
            .unwrap(),
    );
    let linked = adapter
        .get_issue_work(fixture.cwd(), &work.order.issue)
        .unwrap()
        .unwrap();
    assert_eq!(linked.task_ids, observed.task_ids);
    assert_eq!(linked.order, work.order);
    assert_eq!(linked.approved_basis, work.approved_basis);
    assert_eq!(linked.revision, work.revision + 1);
    assert!(matches!(
        adapter
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                first.progress.revision,
                &observed,
                Some(3)
            )
            .unwrap(),
        IssueExecutionSaveResult::Conflict
    ));
    observed.task_ids.pop();
    assert!(
        adapter
            .apply_issue_execution(
                fixture.cwd(),
                &linked,
                first.progress.revision,
                &observed,
                Some(3)
            )
            .is_err()
    );
}

#[test]
fn awaiting_question_and_same_task_resume_preserve_the_original_attempt() {
    let fixture = Fixture::new("execution-question");
    let work = seed(&fixture);
    let adapter = fixture.adapter();
    let mut observed = running(&work);
    observed.question = Some(IssueExecutionQuestion {
        question_id: "question-1".into(),
        task_id: "task-1".into(),
        session_key: "session-1".into(),
        thread_id: "thread-1".into(),
        slot_id: "slot-1".into(),
        worktree_path: fixture
            .workspace
            .join("slot-1")
            .to_string_lossy()
            .into_owned(),
        branch_name: "work/task-1".into(),
        lease_generation: "generation-1".into(),
        awaiting_task_updated_at: "2026-11-21T00:00:00Z".into(),
    });
    let waiting = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(1))
            .unwrap(),
    );
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap()
            .state(),
        IssueAutomationState::AwaitingDecision
    );
    observed.question = None;
    let resumed = saved(
        adapter
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                waiting.progress.revision,
                &observed,
                Some(1),
            )
            .unwrap(),
    );
    assert_eq!(resumed.progress.attempts.execution, 1);
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap()
            .state(),
        IssueAutomationState::Running
    );
}

#[test]
fn only_rework_exhausts_execution_and_cancelled_work_is_not_reactivated() {
    let fixture = Fixture::new("execution-budget");
    let work = seed(&fixture);
    let adapter = fixture.adapter();
    let mut observed = running(&work);
    observed.phase = IssueExecutionPhase::Terminal;
    observed.terminals = vec![IssueExecutionTerminalRecord {
        record_id: "rework-1".into(),
        classification: IssueExecutionTerminalClass::ReworkNeeded,
    }];
    observed.current_terminal_id = Some("rework-1".into());
    let failed = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(1))
            .unwrap(),
    );
    assert!(failed.progress.attempts_exhausted);
    let tracked = adapter
        .get_issue(fixture.cwd(), &work.order.issue)
        .unwrap()
        .unwrap();
    assert_eq!(tracked.state(), IssueAutomationState::Failed);
    transition_and_save(&adapter, fixture.cwd(), &tracked, IssueTransition::Cancel);
    let repeated = saved(
        adapter
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                failed.progress.revision,
                &observed,
                Some(1),
            )
            .unwrap(),
    );
    assert_eq!(repeated.progress.attempts.execution, 1);
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap()
            .state(),
        IssueAutomationState::Cancelled
    );
}

#[test]
fn integration_after_exhaustion_completes_failed_work_once_and_preserves_cancelled_work() {
    for cancelled in [false, true] {
        let fixture = Fixture::new(if cancelled {
            "execution-cancelled-integrated"
        } else {
            "execution-failed-integrated"
        });
        let work = seed(&fixture);
        let adapter = fixture.adapter();
        let mut tracked = adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap();
        tracked.set_flag(IssueAutomationFlag::Held, true);
        super::super::tests::save_changed(&adapter, fixture.cwd(), &tracked);
        let mut observed = running(&work);
        observed.phase = IssueExecutionPhase::Terminal;
        observed.terminals = vec![IssueExecutionTerminalRecord {
            record_id: "rework-at-cap".into(),
            classification: IssueExecutionTerminalClass::ReworkNeeded,
        }];
        observed.current_terminal_id = Some("rework-at-cap".into());
        let failed = saved(
            adapter
                .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(1))
                .unwrap(),
        );
        assert!(failed.progress.attempts_exhausted);
        let tracked = adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap();
        assert_eq!(tracked.state(), IssueAutomationState::Failed);
        if cancelled {
            transition_and_save(&adapter, fixture.cwd(), &tracked, IssueTransition::Cancel);
        }

        observed.phase = IssueExecutionPhase::Integrated;
        observed.current_terminal_id = None;
        observed.integration = Some(IssueExecutionIntegration {
            record_id: "integration-after-cap".into(),
            source_sha: "a".repeat(40),
            integration_sha: "b".repeat(40),
            pull_request_url: Some("https://github.com/acme/project/pull/1".into()),
        });
        let integrated = saved(
            adapter
                .apply_issue_execution(
                    fixture.cwd(),
                    &work,
                    failed.progress.revision,
                    &observed,
                    Some(1),
                )
                .unwrap(),
        );
        assert_eq!(integrated.progress.state, IssueExecutionState::Done);
        assert_eq!(integrated.progress.attempts, failed.progress.attempts);
        assert!(!integrated.progress.attempts_exhausted);
        assert!(!integrated.progress.allows_additional_execution(100));
        let tracked = adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap();
        assert_eq!(
            tracked.state(),
            if cancelled {
                IssueAutomationState::Cancelled
            } else {
                IssueAutomationState::Done
            },
        );
        assert!(tracked.flags().held);
        let replay = adapter
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                integrated.progress.revision,
                &observed,
                Some(1),
            )
            .unwrap();
        assert!(matches!(
            replay,
            IssueExecutionSaveResult::Saved { changed: false, .. }
        ));
        assert_eq!(saved(replay), integrated);
        assert_eq!(
            adapter
                .pending_issue_work_events(fixture.cwd())
                .unwrap()
                .iter()
                .filter(|event| matches!(event.kind, IssueWorkEventKind::Integrated { .. }))
                .count(),
            1,
        );
    }
}

mod questions;

#[test]
fn observe_again_successors_keep_done_and_emit_no_duplicate_integration_event() {
    let fixture = Fixture::new("execution-postintegration-recovery");
    let work = seed(&fixture);
    let mut observed = running(&work);
    observed.phase = IssueExecutionPhase::Integrated;
    observed.integration = Some(IssueExecutionIntegration {
        record_id: "stable-integration-evidence".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: Some("https://github.com/acme/project/pull/1".into()),
    });
    let mut record = saved(
        fixture
            .adapter()
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(1))
            .unwrap(),
    );
    for round in 1..=2 {
        let terminal = IssueExecutionTerminalRecord {
            record_id: format!("post-merge-observation-{round}"),
            classification: IssueExecutionTerminalClass::ObserveAgain,
        };
        observed.current_terminal_id = Some(terminal.record_id.clone());
        observed.terminals.push(terminal);
        record = saved(
            fixture
                .adapter()
                .apply_issue_execution(
                    fixture.cwd(),
                    &work,
                    record.progress.revision,
                    &observed,
                    Some(0),
                )
                .unwrap(),
        );
        assert_eq!(record.progress.state, IssueExecutionState::Done);
        assert_eq!(record.progress.attempts.execution, 1);
        assert_eq!(record.progress.attempts.verification, round);
        assert_eq!(record.progress.integration, observed.integration);
        assert!(!record.progress.allows_additional_execution(100));

        // A fresh successor clears only the current terminal; historical receipt IDs remain.
        observed.current_terminal_id = None;
        record = saved(
            fixture
                .adapter()
                .apply_issue_execution(
                    fixture.cwd(),
                    &work,
                    record.progress.revision,
                    &observed,
                    Some(0),
                )
                .unwrap(),
        );
        assert_eq!(record.progress.state, IssueExecutionState::Done);
        assert_eq!(record.progress.attempts.verification, round);
        assert!(record.progress.terminal.is_none());
        let repeated = fixture
            .adapter()
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                record.progress.revision,
                &observed,
                Some(0),
            )
            .unwrap();
        assert!(matches!(
            repeated,
            IssueExecutionSaveResult::Saved { changed: false, .. }
        ));
        assert_eq!(saved(repeated), record);
    }
    let adapter = fixture.adapter();
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap()
            .state(),
        IssueAutomationState::Done,
    );
    assert_eq!(
        adapter
            .pending_issue_work_events(fixture.cwd())
            .unwrap()
            .iter()
            .filter(|event| matches!(event.kind, IssueWorkEventKind::Integrated { .. }))
            .count(),
        1,
    );
}
