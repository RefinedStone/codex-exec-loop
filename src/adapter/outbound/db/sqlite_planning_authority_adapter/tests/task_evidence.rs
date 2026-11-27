use super::{
    SqlitePlanningAuthorityAdapter, authority_connection, authority_task,
    empty_test_queue_projection, temp_workspace, test_direction_catalog,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::task_evidence_repository_port::*;
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::evidence::*;
use crate::domain::planning::{
    TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel, TaskAcceptanceCriterion,
    TaskAuthorityDocument, TaskDefinition, TaskStatus,
};
use std::sync::{Arc, Barrier};

const NOW: &str = "2026-11-23T12:00:00Z";

mod delivery;
mod preview;
mod verification;

struct Fixture {
    workspace: String,
    adapter: SqlitePlanningAuthorityAdapter,
    lease: ParallelModeSlotLeaseSnapshot,
}

impl Fixture {
    fn new(label: &str) -> Self {
        Self::with_before_lease(label, |_| {})
    }

    fn with_before_lease(label: &str, before_lease: impl FnOnce(&mut Self)) -> Self {
        let workspace = temp_workspace(label);
        let adapter = SqlitePlanningAuthorityAdapter::new();
        adapter
            .commit_direction_authority_snapshot(
                &workspace,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions: &test_direction_catalog(&["direction-1"]),
                    authority_mutation_owner_token: None,
                },
            )
            .unwrap();
        let mut task = authority_task("evidence-task", "direction-1");
        task.acceptance = vec![criterion("criterion one"), criterion("criterion two")];
        task.acceptance_revision = 1;
        let lease = ParallelModeSlotLeaseSnapshot::new(
            "slot-1",
            &task.id,
            &task.title,
            "fixture-worker",
            "akra-agent/slot-1",
            format!("{workspace}/slot-1"),
            ParallelModeSlotLeaseState::Running,
            NOW,
            Some(NOW.into()),
        )
        .with_lease_generation("a".repeat(64));
        let mut fixture = Self {
            workspace,
            adapter,
            lease,
        };
        fixture.save(vec![task]);
        before_lease(&mut fixture);
        fixture
            .adapter
            .upsert_runtime_slot_lease(&fixture.workspace, &fixture.lease)
            .unwrap();
        fixture
    }

    fn save(&self, tasks: Vec<TaskDefinition>) {
        let revision = self
            .adapter
            .load_task_authority_snapshot(&self.workspace)
            .unwrap()
            .map(|snapshot| snapshot.planning_revision);
        let result = self
            .adapter
            .commit_task_authority_snapshot(
                &self.workspace,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: revision,
                    task_authority: &TaskAuthorityDocument {
                        version: TASK_AUTHORITY_FORMAT_VERSION,
                        tasks,
                    },
                    queue_projection: &empty_test_queue_projection(),
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            PlanningTaskAuthorityCommitResult::Committed { .. }
        ));
    }

    fn snapshot(&self) -> TaskEvidenceStoredSnapshot {
        self.adapter
            .load_current_snapshot(&self.workspace, &TaskEvidenceSnapshotQuery::default())
            .unwrap()
    }

    fn prepare_commit(&self, id: &str) -> TaskEvidencePrepareCommit {
        TaskEvidencePrepareCommit {
            request_id: id.into(),
            scope: self.snapshot().tasks[0].scope.clone(),
            criterion_id: "criterion one".into(),
            lease: self.lease.clone(),
            prepared_tree: TaskEvidenceTree {
                head_commit_sha: "b".repeat(40),
                tree_sha: "c".repeat(40),
                worktree_clean: false,
            },
            at: NOW.into(),
        }
    }

    fn prepare(&self, id: &str) -> TaskEvidenceAttempt {
        self.adapter
            .prepare_attempt(&self.workspace, &self.prepare_commit(id))
            .unwrap()
            .value
    }

    fn record_commit(&self, attempt: &TaskEvidenceAttempt) -> TaskEvidenceRecordCommit {
        TaskEvidenceRecordCommit {
            attempt: attempt.clone(),
            lease: self.lease.clone(),
            status: TaskEvidenceStatus::Pass,
            artifact: TaskEvidenceArtifact {
                sha256: "d".repeat(64),
                size_bytes: 16,
            },
            recorded_tree: attempt.prepared_tree.clone(),
            note: Some("fixture result".into()),
            by: "fixture-worker".into(),
            at: NOW.into(),
        }
    }

    fn counters(&self) -> (u64, u64, u64, u64) {
        authority_connection(&self.workspace).query_row(
            "SELECT revision,entry_count,artifact_bytes,reserved_bytes FROM task_evidence_state WHERE singleton=1", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap()
    }
}

fn criterion(id: &str) -> TaskAcceptanceCriterion {
    TaskAcceptanceCriterion {
        id: id.into(),
        scenario: "The result is observable".into(),
        channel: TaskAcceptanceChannel::Command,
        essential: true,
        verification_recipe: None,
    }
}

fn reason(error: anyhow::Error) -> TaskEvidenceError {
    *error
        .downcast_ref::<TaskEvidenceError>()
        .expect("typed evidence rejection")
}

#[test]
fn task_evidence_prepare_record_replay_and_outbox_ack_keep_the_original_receipt() {
    let fixture = Fixture::new("task-evidence-replay");
    let mut prepare = fixture.prepare_commit("prepare-1");
    let first = fixture
        .adapter
        .prepare_attempt(&fixture.workspace, &prepare)
        .unwrap();
    assert!(first.changed);
    let before = fixture.counters();
    prepare.at = "2026-11-23T11:00:00Z".into();
    let replay = fixture
        .adapter
        .prepare_attempt(&fixture.workspace, &prepare)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.value, first.value);
    assert_eq!(fixture.counters(), before);
    let mut record = fixture.record_commit(&first.value);
    let recorded = fixture
        .adapter
        .record_evidence(&fixture.workspace, &record)
        .unwrap();
    record.at = "2026-11-23T10:00:00Z".into();
    record.by = "retry-caller".into();
    let replay = fixture
        .adapter
        .record_evidence(&fixture.workspace, &record)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.value, recorded.value);
    assert_eq!(
        fixture.snapshot().tasks[0].evidence,
        vec![recorded.value.clone()]
    );
    let events = fixture
        .adapter
        .pending_events(&fixture.workspace, usize::MAX)
        .unwrap();
    assert_eq!(
        events,
        vec![TaskEvidenceEvent::Recorded {
            receipt: Box::new(recorded.value.clone())
        }]
    );
    fixture
        .adapter
        .acknowledge_event(&fixture.workspace, &recorded.value.event_id)
        .unwrap();
    fixture
        .adapter
        .acknowledge_event(&fixture.workspace, &recorded.value.event_id)
        .unwrap();
    assert!(
        fixture
            .adapter
            .pending_events(&fixture.workspace, 10)
            .unwrap()
            .is_empty()
    );
    let reopened = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        reopened
            .load_attempt(&fixture.workspace, &first.value.attempt_id)
            .unwrap()
            .unwrap()
            .receipt,
        Some(recorded.value)
    );
    assert_eq!(fixture.counters().2, 16);
    assert_eq!(fixture.counters().3, 0);
}

#[test]
fn task_evidence_same_request_id_cannot_reserve_another_criterion_or_rewrite_a_record() {
    let fixture = Fixture::new("task-evidence-request-conflict");
    let attempt = fixture.prepare("same-id");
    let mut other = fixture.prepare_commit("same-id");
    other.criterion_id = "criterion two".into();
    assert_eq!(
        reason(
            fixture
                .adapter
                .prepare_attempt(&fixture.workspace, &other)
                .unwrap_err()
        ),
        TaskEvidenceError::Conflict
    );
    let mut record = fixture.record_commit(&attempt);
    fixture
        .adapter
        .record_evidence(&fixture.workspace, &record)
        .unwrap();
    record.status = TaskEvidenceStatus::Fail;
    assert_eq!(
        reason(
            fixture
                .adapter
                .record_evidence(&fixture.workspace, &record)
                .unwrap_err()
        ),
        TaskEvidenceError::Conflict
    );
    assert_eq!(
        fixture
            .adapter
            .pending_events(&fixture.workspace, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn task_evidence_scope_aba_and_noop_order_keep_history_without_restoring_authority() {
    let fixture = Fixture::new("task-evidence-scope-aba");
    let attempt = fixture.prepare("prepare-before-edit");
    let record = fixture.record_commit(&attempt);
    let receipt = fixture
        .adapter
        .record_evidence(&fixture.workspace, &record)
        .unwrap()
        .value;
    let original_task = fixture.snapshot().tasks[0].task.clone();
    let original_scope = fixture.snapshot().tasks[0].scope.clone();
    let mut reordered = original_task.clone();
    reordered.acceptance.reverse();
    reordered.title = "Updated title".into();
    fixture.save(vec![reordered]);
    assert_eq!(fixture.snapshot().tasks[0].scope, original_scope);
    let mut changed = original_task.clone();
    changed.acceptance[0].scenario = "A different observation".into();
    changed.acceptance_revision = 2;
    fixture.save(vec![changed]);
    let middle = fixture.snapshot().tasks[0].scope.clone();
    assert_ne!(middle.scope_id, original_scope.scope_id);
    fixture.save(vec![original_task]);
    let latest = fixture.snapshot().tasks[0].scope.clone();
    assert_ne!(latest.scope_id, middle.scope_id);
    assert_ne!(latest.scope_id, original_scope.scope_id);
    assert_eq!(
        latest.acceptance_revision,
        original_scope.acceptance_revision
    );
    assert_eq!(latest.criteria_sha256, original_scope.criteria_sha256);
    let replay = fixture
        .adapter
        .record_evidence(&fixture.workspace, &record)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.value, receipt);
    assert_ne!(
        fixture.snapshot().tasks[0].evidence[0]
            .evidence
            .attempt
            .scope,
        latest
    );
    let mut stale_prepare = fixture.prepare_commit("fresh-request-with-old-scope");
    stale_prepare.scope = original_scope;
    assert_eq!(
        reason(
            fixture
                .adapter
                .prepare_attempt(&fixture.workspace, &stale_prepare)
                .unwrap_err()
        ),
        TaskEvidenceError::ScopeChanged
    );
}

#[test]
fn task_evidence_delete_recreate_and_reset_invalidate_scope_but_preserve_pending_facts() {
    let fixture = Fixture::new("task-evidence-task-recreated");
    let attempt = fixture.prepare("before-delete");
    let receipt = fixture
        .adapter
        .record_evidence(&fixture.workspace, &fixture.record_commit(&attempt))
        .unwrap()
        .value;
    let task = fixture.snapshot().tasks[0].task.clone();
    fixture
        .adapter
        .remove_runtime_slot_lease(&fixture.workspace, &fixture.lease.slot_id)
        .unwrap();
    fixture.save(Vec::new());
    assert!(fixture.snapshot().tasks.is_empty());
    fixture.save(vec![task.clone()]);
    let recreated = fixture.snapshot().tasks[0].scope.clone();
    assert_ne!(recreated, attempt.scope);
    fixture
        .adapter
        .clear_task_authority_snapshot(&fixture.workspace)
        .unwrap();
    fixture.save(vec![task]);
    assert_ne!(fixture.snapshot().tasks[0].scope, recreated);
    assert_eq!(
        fixture
            .adapter
            .pending_events(&fixture.workspace, 10)
            .unwrap(),
        vec![TaskEvidenceEvent::Recorded {
            receipt: Box::new(receipt)
        }]
    );
}

#[test]
fn task_evidence_changed_lease_legacy_lease_and_waiting_task_cannot_finish() {
    let fixture = Fixture::new("task-evidence-stale-lease");
    let attempt = fixture.prepare("before-lease-change");
    let mut next = fixture.lease.clone();
    next.lease_generation = Some("e".repeat(64));
    fixture
        .adapter
        .upsert_runtime_slot_lease(&fixture.workspace, &next)
        .unwrap();
    assert_eq!(
        reason(
            fixture
                .adapter
                .record_evidence(&fixture.workspace, &fixture.record_commit(&attempt))
                .unwrap_err()
        ),
        TaskEvidenceError::LeaseChanged
    );
    next.lease_generation = None;
    fixture
        .adapter
        .upsert_runtime_slot_lease(&fixture.workspace, &next)
        .unwrap();
    let mut prepare = fixture.prepare_commit("legacy-lease");
    prepare.lease = next;
    assert_eq!(
        reason(
            fixture
                .adapter
                .prepare_attempt(&fixture.workspace, &prepare)
                .unwrap_err()
        ),
        TaskEvidenceError::LeaseChanged
    );
    fixture
        .adapter
        .upsert_runtime_slot_lease(&fixture.workspace, &fixture.lease)
        .unwrap();
    let mut task = fixture.snapshot().tasks[0].task.clone();
    task.status = TaskStatus::AwaitingUser;
    fixture.save(vec![task]);
    assert_eq!(
        reason(
            fixture
                .adapter
                .record_evidence(&fixture.workspace, &fixture.record_commit(&attempt))
                .unwrap_err()
        ),
        TaskEvidenceError::LeaseChanged
    );
    assert!(
        fixture
            .adapter
            .pending_events(&fixture.workspace, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn task_evidence_tree_change_and_failed_commit_keep_reservation_and_no_fact() {
    let fixture = Fixture::new("task-evidence-rollback");
    let attempt = fixture.prepare("prepared-tree");
    let before = fixture.counters();
    let mut record = fixture.record_commit(&attempt);
    record.recorded_tree.tree_sha = "f".repeat(40);
    assert_eq!(
        reason(
            fixture
                .adapter
                .record_evidence(&fixture.workspace, &record)
                .unwrap_err()
        ),
        TaskEvidenceError::Stale
    );
    assert_eq!(fixture.counters(), before);
    record.recorded_tree = attempt.prepared_tree.clone();
    record.by.clear();
    assert_eq!(
        reason(
            fixture
                .adapter
                .record_evidence(&fixture.workspace, &record)
                .unwrap_err()
        ),
        TaskEvidenceError::Invalid
    );
    assert_eq!(fixture.counters(), before);
    assert!(
        fixture
            .adapter
            .load_attempt(&fixture.workspace, &attempt.attempt_id)
            .unwrap()
            .unwrap()
            .receipt
            .is_none()
    );
    assert!(
        fixture
            .adapter
            .pending_events(&fixture.workspace, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn task_evidence_newer_finished_ordinal_wins_even_when_an_older_attempt_finishes_later() {
    let fixture = Fixture::new("task-evidence-ordinal");
    let first = fixture.prepare("attempt-one");
    fixture
        .adapter
        .record_evidence(&fixture.workspace, &fixture.record_commit(&first))
        .unwrap();
    let second = fixture.prepare("attempt-two");
    // Merely preparing another attempt does not replace a completed report for the same tree.
    assert_eq!(
        fixture.snapshot().tasks[0].evidence[0].evidence.attempt,
        first
    );
    let third = fixture.prepare("attempt-three");
    let mut newer = fixture.record_commit(&third);
    newer.status = TaskEvidenceStatus::Fail;
    fixture
        .adapter
        .record_evidence(&fixture.workspace, &newer)
        .unwrap();
    fixture
        .adapter
        .record_evidence(&fixture.workspace, &fixture.record_commit(&second))
        .unwrap();
    let latest = &fixture.snapshot().tasks[0].evidence[0];
    assert_eq!(latest.evidence.attempt, third);
    assert_eq!(latest.evidence.status, TaskEvidenceStatus::Fail);
}

#[test]
fn task_evidence_waiver_is_immutable_and_only_matches_its_acceptance_scope() {
    let fixture = Fixture::new("task-evidence-waiver");
    let scope = fixture.snapshot().tasks[0].scope.clone();
    let mut commit = TaskCriterionWaiverCommit {
        request_id: "waiver-1".into(),
        scope: scope.clone(),
        criterion_id: "criterion one".into(),
        reason: "  A human accepted this criterion exception.  ".into(),
        confirmation_ref: "native-proof-ref".into(),
        at: NOW.into(),
    };
    let first = fixture
        .adapter
        .waive_criterion(&fixture.workspace, &commit)
        .unwrap();
    assert!(first.changed);
    commit.reason = first.value.waiver.reason.clone();
    commit.at = "2026-11-23T10:00:00Z".into();
    let replay = fixture
        .adapter
        .waive_criterion(&fixture.workspace, &commit)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.value, first.value);
    commit.reason = "A different exception".into();
    assert_eq!(
        reason(
            fixture
                .adapter
                .waive_criterion(&fixture.workspace, &commit)
                .unwrap_err()
        ),
        TaskEvidenceError::Conflict
    );
    let mut task = fixture.snapshot().tasks[0].task.clone();
    task.acceptance_revision += 1;
    fixture.save(vec![task]);
    commit.request_id = "old-proof-new-command".into();
    assert_eq!(
        reason(
            fixture
                .adapter
                .waive_criterion(&fixture.workspace, &commit)
                .unwrap_err()
        ),
        TaskEvidenceError::ScopeChanged
    );
    assert_ne!(
        fixture.snapshot().tasks[0].scope,
        fixture.snapshot().tasks[0].waivers[0].waiver.scope
    );
    assert_eq!(
        fixture
            .adapter
            .pending_events(&fixture.workspace, 10)
            .unwrap(),
        vec![TaskEvidenceEvent::Waived {
            receipt: Box::new(first.value)
        }]
    );
}

#[test]
fn task_evidence_prepare_reserves_maximum_artifact_bytes_before_any_file_is_sealed() {
    let fixture = Fixture::new("task-evidence-reserved-quota");
    for index in 0..MAX_TASK_EVIDENCE_ATTEMPTS_PER_CRITERION {
        fixture.prepare(&format!("reserve-{index}"));
    }
    assert_eq!(fixture.counters().3, MAX_TASK_EVIDENCE_TOTAL_BYTES);
    let before = fixture.counters();
    let mut another = fixture.prepare_commit("would-exceed-reserved-bytes");
    another.criterion_id = "criterion two".into();
    assert_eq!(
        reason(
            fixture
                .adapter
                .prepare_attempt(&fixture.workspace, &another)
                .unwrap_err()
        ),
        TaskEvidenceError::Limit
    );
    assert_eq!(fixture.counters(), before);
    assert!(
        fixture
            .adapter
            .pending_events(&fixture.workspace, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn task_evidence_concurrent_prepare_claims_one_request_and_one_byte_reservation() {
    let fixture = Fixture::new("task-evidence-concurrent-prepare");
    let commit = fixture.prepare_commit("concurrent-id");
    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let workspace = fixture.workspace.clone();
            let commit = commit.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                SqlitePlanningAuthorityAdapter::new()
                    .prepare_attempt(&workspace, &commit)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.changed).count(), 1);
    assert_eq!(outcomes[0].value, outcomes[1].value);
    assert_eq!(fixture.counters().1, 1);
    assert_eq!(fixture.counters().3, MAX_TASK_EVIDENCE_ARTIFACT_BYTES);
}

#[test]
fn task_evidence_all_task_query_pages_canonical_done_and_skipped_authority_and_rejects_bad_batches()
{
    let fixture = Fixture::new("task-evidence-pages");
    let first = fixture.snapshot().tasks[0].task.clone();
    let mut done = first.clone();
    done.id = "done-task".into();
    done.status = TaskStatus::Done;
    let mut blocked = first.clone();
    blocked.id = "blocked-task".into();
    blocked.status = TaskStatus::Blocked;
    fixture.save(vec![first, done, blocked]);
    let mut query = TaskEvidenceSnapshotQuery {
        limit: 1,
        ..Default::default()
    };
    let mut ids = Vec::new();
    loop {
        let page = fixture
            .adapter
            .load_current_snapshot(&fixture.workspace, &query)
            .unwrap();
        ids.extend(page.tasks.into_iter().map(|task| task.task.id));
        if !page.truncated {
            assert!(page.next_task_id.is_none());
            break;
        }
        query.after_task_id = page.next_task_id;
    }
    assert_eq!(ids, vec!["blocked-task", "done-task", "evidence-task"]);
    let duplicate = TaskEvidenceSnapshotQuery {
        task_ids: vec!["evidence-task".into(); 2],
        ..Default::default()
    };
    assert_eq!(
        reason(
            fixture
                .adapter
                .load_current_snapshot(&fixture.workspace, &duplicate)
                .unwrap_err()
        ),
        TaskEvidenceError::Invalid
    );
    let too_many = TaskEvidenceSnapshotQuery {
        task_ids: vec!["done-task".into(), "evidence-task".into()],
        limit: 1,
        after_task_id: None,
    };
    assert_eq!(
        reason(
            fixture
                .adapter
                .load_current_snapshot(&fixture.workspace, &too_many)
                .unwrap_err()
        ),
        TaskEvidenceError::Limit
    );
}

#[test]
fn task_evidence_current_snapshot_and_outbox_do_not_accept_corrupt_authority_or_payload() {
    let fixture = Fixture::new("task-evidence-corrupt-snapshot");
    let attempt = fixture.prepare("record-1");
    let receipt = fixture
        .adapter
        .record_evidence(&fixture.workspace, &fixture.record_commit(&attempt))
        .unwrap()
        .value;
    let connection = authority_connection(&fixture.workspace);
    let mut corrupt = receipt.clone();
    corrupt.evidence.by = "another-worker".into();
    connection
        .execute(
            "UPDATE task_evidence_events SET payload_json=?1",
            [serde_json::to_string(&TaskEvidenceEvent::Recorded {
                receipt: Box::new(corrupt),
            })
            .unwrap()],
        )
        .unwrap();
    assert_eq!(
        reason(
            fixture
                .adapter
                .pending_events(&fixture.workspace, 10)
                .unwrap_err()
        ),
        TaskEvidenceError::Unavailable
    );
    let mut task = fixture.snapshot().tasks[0].task.clone();
    task.acceptance[0].scenario = "Changed by an older writer".into();
    connection
        .execute(
            "UPDATE planning_tasks SET content_json=?1 WHERE task_id=?2",
            rusqlite::params![serde_json::to_string(&task).unwrap(), task.id],
        )
        .unwrap();
    assert_eq!(
        reason(
            fixture
                .adapter
                .load_current_snapshot(&fixture.workspace, &TaskEvidenceSnapshotQuery::default())
                .unwrap_err()
        ),
        TaskEvidenceError::ScopeChanged
    );
}
