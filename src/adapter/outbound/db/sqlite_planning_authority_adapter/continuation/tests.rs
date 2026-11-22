use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningWorkerQuestionIdentity;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControl, PlanningWorkPermit, PlanningWorkPermitBasis, PlanningWorkPermitStage,
};
use crate::domain::parallel_mode::{
    ParallelModeDeliveryTargetSnapshot, ParallelModeLiveSessionDetailDefaults,
    ParallelModeRepositoryVisibility, ParallelModeSlotLeaseState,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    QueueIdleConfig, QueueIdlePolicy, TaskAuthorityDocument, TaskDefinition,
};

struct Fixture {
    connection: Connection,
    location: PlanningAuthorityLocation,
    request: PlanningWorkerContinuationRequest,
    lease: ParallelModeSlotLeaseSnapshot,
}

impl Fixture {
    fn new() -> Self {
        let mut connection = Connection::open_in_memory().unwrap();
        let location = PlanningAuthorityLocation {
            workspace_root: "/repo".into(),
            canonical_repo_root: "/repo".into(),
            repository_identity: "continuation-test".into(),
            runtime_dir: "/runtime".into(),
            authority_store_path: ":memory:".into(),
        };
        super::super::store::ensure_schema(&mut connection, &location).unwrap();
        work_orders::ensure_work_order_tables(&connection).unwrap();
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::Stop,
                prompt_path: String::new(),
            },
            directions: vec![DirectionDefinition {
                id: "direction-a".into(),
                title: "Fixture".into(),
                summary: "Fixture".into(),
                success_criteria: vec!["preserve the waiting session".into()],
                scope_hints: vec![],
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        };
        let task = TaskDefinition {
            id: "task-a".into(),
            direction_id: "direction-a".into(),
            direction_relation_note: "fixture".into(),
            title: "Waiting task".into(),
            description: "Synthetic fixture".into(),
            status: TaskStatus::AwaitingUser,
            base_priority: 50,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: vec![],
            blocked_by: vec![],
            created_by: TaskActor::System,
            last_updated_by: TaskActor::System,
            source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::System),
            updated_at: "2026-11-21T00:00:00Z".into(),
        };
        let tasks = TaskAuthorityDocument {
            version: 1,
            tasks: vec![task.clone()],
        };
        let queue = PriorityQueueService::new()
            .build_projection(&directions, &tasks)
            .unwrap();
        let tx = connection.transaction().unwrap();
        super::super::store::replace_direction_authority_tables(&tx, &directions).unwrap();
        SqlitePlanningAuthorityAdapter::write_task_authority_change_in_transaction(
            &tx,
            &location,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &tasks,
                queue_projection: &queue,
            },
            None,
            None,
        )
        .unwrap();
        tx.commit().unwrap();
        let basis = PlanningWorkPermitBasis {
            authorization_epoch: 0,
            policy_version: "policy-v1".into(),
            field_hash: "fixture-field-hash".into(),
            gate_revision: 1,
            source_sha: "a".repeat(40),
        };
        let permit = PlanningWorkPermit {
            work_order_id: "order-a".into(),
            stage: PlanningWorkPermitStage::Dispatch,
            basis: basis.clone(),
            requires_human_review: false,
        };
        connection.execute("INSERT INTO planning_work_orders(work_order_id,origin,permits_json) VALUES('order-a','fixture',?1)",
            params![serde_json::to_string(&vec![permit]).unwrap()]).unwrap();
        connection.execute("INSERT INTO planning_work_order_tasks(task_id,work_order_id,ordinal) VALUES('task-a','order-a',0)",[]).unwrap();
        connection.execute("INSERT INTO planning_work_order_permit_basis(work_order_id,basis_json) VALUES('order-a',?1)",
            params![serde_json::to_string(&basis).unwrap()]).unwrap();
        let lease = ParallelModeSlotLeaseSnapshot::new(
            "slot-1",
            "task-a",
            "Waiting task",
            "agent-a",
            "akra-agent/slot-1/task-a",
            "/repo/.pool/slot-1",
            ParallelModeSlotLeaseState::Running,
            "2026-11-20T23:00:00Z",
            Some("2026-11-20T23:01:00Z".into()),
        )
        .with_lease_generation("b".repeat(64))
        .with_delivery_target(ParallelModeDeliveryTargetSnapshot::new(
            "origin",
            "owner/repo",
            ParallelModeRepositoryVisibility::Private,
            "prerelease",
            "a".repeat(40),
        ));
        connection
            .execute(
                "INSERT INTO runtime_slot_leases(slot_id,updated_at,content) VALUES(?1,?2,?3)",
                params![
                    lease.slot_id,
                    lease.leased_at,
                    serde_json::to_string(&lease).unwrap()
                ],
            )
            .unwrap();
        let mut detail = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
            &lease,
            ParallelModeLiveSessionDetailDefaults {
                validation_summary: "pending",
                authority_refresh_outcome: "pending",
            },
        );
        detail.thread_id = Some("thread-a".into());
        detail.state_label = "awaiting_user".into();
        detail.completion_state_label = "awaiting_user".into();
        write_detail(&connection, &detail).unwrap();
        // The first launch already consumed its attempt. Resuming it must work at the cap.
        connection.execute("INSERT INTO planning_work_order_execution_limits(work_order_id,max_attempts) VALUES('order-a',1)",[]).unwrap();
        connection.execute("INSERT INTO planning_work_order_started_tasks(work_order_id,task_id,first_session_key) VALUES('order-a','task-a',?1)",
            params![lease.session_key()]).unwrap();
        let request = PlanningWorkerContinuationRequest {
            authorization_epoch: 0,
            command_id: "answer-a".into(),
            work_order_id: "order-a".into(),
            message: "Approved answer text".into(),
            question: PlanningWorkerQuestionIdentity {
                question_id: "question-a".into(),
                task_id: "task-a".into(),
                session_key: lease.session_key(),
                thread_id: "thread-a".into(),
                slot_id: lease.slot_id.clone(),
                worktree_path: lease.worktree_path.clone(),
                branch_name: lease.branch_name.clone(),
                lease_generation: "b".repeat(64),
                awaiting_task_updated_at: task.updated_at,
            },
        };
        Self {
            connection,
            location,
            request,
            lease,
        }
    }

    fn enqueue(&mut self) -> PlanningWorkerContinuationReceipt {
        let request = self.request.clone();
        self.enqueue_request(&request).unwrap()
    }
    fn enqueue_request(
        &mut self,
        request: &PlanningWorkerContinuationRequest,
    ) -> Result<PlanningWorkerContinuationReceipt> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = enqueue_in_transaction(&tx, request)?;
        tx.commit()?;
        Ok(receipt)
    }
    fn claim(&mut self) -> PlanningWorkerContinuationClaimOutcome {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let result =
            claim_in_transaction(&tx, &self.request.command_id, &"c".repeat(64), None).unwrap();
        tx.commit().unwrap();
        result
    }
    fn claimed(&mut self) -> PlanningWorkerContinuationClaim {
        let PlanningWorkerContinuationClaimOutcome::Claimed(claim) = self.claim() else {
            panic!("claim expected")
        };
        *claim
    }
    fn receipt(&self) -> PlanningWorkerContinuationReceipt {
        load(&self.connection, &self.request.command_id)
            .unwrap()
            .unwrap()
            .receipt
    }
    fn task(&self) -> TaskDefinition {
        load_task_authority_snapshot_from_connection(&self.connection)
            .unwrap()
            .unwrap()
            .task_authority
            .tasks
            .into_iter()
            .find(|task| task.id == "task-a")
            .unwrap()
    }
    fn change_task(&mut self, change: impl FnOnce(&mut TaskDefinition)) {
        let before = load_task_authority_snapshot_from_connection(&self.connection)
            .unwrap()
            .unwrap();
        let mut candidate = before.task_authority.clone();
        change(&mut candidate.tasks[0]);
        let directions = load_direction_catalog_from_connection(&self.connection)
            .unwrap()
            .unwrap();
        let queue = PriorityQueueService::new()
            .build_projection(&directions, &candidate)
            .unwrap();
        let tx = self.connection.transaction().unwrap();
        SqlitePlanningAuthorityAdapter::write_task_authority_change_in_transaction(
            &tx,
            &self.location,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(before.planning_revision),
                task_authority: &candidate,
                queue_projection: &queue,
            },
            Some(&before.task_authority),
            None,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    fn control(&self, control: PlanningWorkControl) {
        self.connection.execute("INSERT INTO planning_work_order_controls(work_order_id,control_json)
            VALUES('order-a',?1) ON CONFLICT(work_order_id) DO UPDATE SET control_json=excluded.control_json",
            params![serde_json::to_string(&control).unwrap()]).unwrap();
    }
    fn authorize_epoch(&self, epoch: u64) {
        self.control(PlanningWorkControl {
            authorization_epoch: epoch,
            ..PlanningWorkControl::default()
        });
        let json: String = self.connection.query_row(
            "SELECT basis_json FROM planning_work_order_permit_basis WHERE work_order_id='order-a'",
            [], |row| row.get(0),
        ).unwrap();
        let mut basis: PlanningWorkPermitBasis = serde_json::from_str(&json).unwrap();
        basis.authorization_epoch = epoch;
        self.connection.execute(
            "UPDATE planning_work_order_permit_basis SET basis_json=?1 WHERE work_order_id='order-a'",
            params![serde_json::to_string(&basis).unwrap()],
        ).unwrap();
        let permit = PlanningWorkPermit {
            work_order_id: "order-a".into(),
            stage: PlanningWorkPermitStage::Dispatch,
            basis,
            requires_human_review: false,
        };
        self.connection
            .execute(
                "UPDATE planning_work_orders SET permits_json=?1 WHERE work_order_id='order-a'",
                params![serde_json::to_string(&vec![permit]).unwrap()],
            )
            .unwrap();
    }

    fn ack(
        &mut self,
        claim: &PlanningWorkerContinuationClaim,
        thread: &str,
        turn: &str,
    ) -> Result<PlanningWorkerContinuationReceipt> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = acknowledge_in_transaction(&tx, &self.location, claim, thread, turn)?;
        tx.commit()?;
        Ok(receipt)
    }
}

#[test]
fn continuation_seals_unknown_with_new_generation_and_never_reclaims_it() {
    let mut fixture = Fixture::new();
    assert_eq!(
        fixture.enqueue().delivery,
        PlanningWorkerContinuationDelivery::Queued
    );
    let claim = fixture.claimed();
    assert_eq!(claim.previous_lease, fixture.lease);
    assert_eq!(claim.lease.worktree_path, fixture.lease.worktree_path);
    assert_eq!(claim.lease.branch_name, fixture.lease.branch_name);
    assert_eq!(claim.lease.task_id, fixture.lease.task_id);
    assert_eq!(
        claim.lease.lease_generation.as_deref(),
        Some("c".repeat(64).as_str())
    );
    assert!(!claim.previous_lease.same_generation_as(&claim.lease));
    assert_eq!(
        fixture.receipt().delivery,
        PlanningWorkerContinuationDelivery::Unknown
    );
    assert_eq!(fixture.task().status, TaskStatus::AwaitingUser);
    assert!(
        matches!(fixture.claim(),PlanningWorkerContinuationClaimOutcome::Settled(receipt)
        if receipt.delivery==PlanningWorkerContinuationDelivery::Unknown)
    );
    assert_eq!(
        load_lease(&fixture.connection, "slot-1").unwrap(),
        Some(claim.lease)
    );
    assert!(task_preserved_in_connection(&fixture.connection, "task-a").unwrap());
}

#[test]
fn continuation_ack_is_exact_immutable_and_keeps_the_same_execution_attempt() {
    let mut fixture = Fixture::new();
    fixture.enqueue();
    let claim = fixture.claimed();
    assert!(fixture.ack(&claim, "other-thread", "turn-a").is_err());
    assert!(fixture.ack(&claim, "thread-a", "").is_err());
    assert_eq!(
        fixture.receipt().delivery,
        PlanningWorkerContinuationDelivery::Unknown
    );
    let receipt = fixture.ack(&claim, "thread-a", "turn-a").unwrap();
    assert_eq!(
        receipt.delivery,
        PlanningWorkerContinuationDelivery::Accepted {
            thread_id: "thread-a".into(),
            turn_id: "turn-a".into(),
            lease_generation: "c".repeat(64)
        }
    );
    assert_eq!(fixture.task().status, TaskStatus::InProgress);
    assert!(!task_preserved_in_connection(&fixture.connection, "task-a").unwrap());
    assert_eq!(fixture.ack(&claim, "thread-a", "turn-a").unwrap(), receipt);
    assert!(fixture.ack(&claim, "thread-a", "other-turn").is_err());
    assert_eq!(fixture.receipt(), receipt);
    let count: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM planning_work_order_started_tasks WHERE work_order_id='order-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "an answer continues the first execution at its cap"
    );
}

#[test]
fn continuation_queue_and_claim_reject_a_changed_authorization_epoch() {
    let mut fixture = Fixture::new();
    fixture.control(PlanningWorkControl {
        authorization_epoch: 1,
        ..PlanningWorkControl::default()
    });
    assert!(matches!(
        fixture.enqueue().delivery,
        PlanningWorkerContinuationDelivery::RejectedBeforeStart {
            reason: PlanningWorkerContinuationRejection::WorkControlled
        }
    ));
    let mut fixture = Fixture::new();
    fixture.enqueue();
    // Hold followed by Release, or Retriage, can be allowed again but belongs to another epoch.
    fixture.control(PlanningWorkControl {
        authorization_epoch: 2,
        ..PlanningWorkControl::default()
    });
    assert!(
        matches!(fixture.claim(),PlanningWorkerContinuationClaimOutcome::Settled(receipt)
        if matches!(receipt.delivery,PlanningWorkerContinuationDelivery::RejectedBeforeStart{
            reason:PlanningWorkerContinuationRejection::WorkControlled}))
    );
    assert_eq!(
        load_lease(&fixture.connection, "slot-1").unwrap(),
        Some(fixture.lease)
    );
}

#[test]
fn continuation_control_changes_after_claim_block_the_worker_without_a_retry() {
    for control in [
        PlanningWorkControl {
            cancelled: true,
            authorization_epoch: 1,
            ..PlanningWorkControl::default()
        },
        PlanningWorkControl {
            taken_over: true,
            authorization_epoch: 1,
            ..PlanningWorkControl::default()
        },
        PlanningWorkControl {
            authorization_epoch: 1,
            ..PlanningWorkControl::default()
        },
    ] {
        let mut fixture = Fixture::new();
        fixture.enqueue();
        let claim = fixture.claimed();
        fixture.control(control);
        let tx = fixture.connection.transaction().unwrap();
        let row = load(&tx, &fixture.request.command_id).unwrap().unwrap();
        assert!(validate_start(&tx, &row, &claim).is_err());
        tx.commit().unwrap();
        assert!(
            matches!(fixture.claim(),PlanningWorkerContinuationClaimOutcome::Settled(receipt)
            if receipt.delivery==PlanningWorkerContinuationDelivery::Unknown)
        );
    }
}

#[test]
fn continuation_stale_question_and_lease_do_not_overwrite_newer_owners() {
    let mut fixture = Fixture::new();
    fixture.enqueue();
    fixture.change_task(|task| task.updated_at = "2026-11-21T00:01:00Z".into());
    assert!(
        matches!(fixture.claim(),PlanningWorkerContinuationClaimOutcome::Settled(receipt)
        if matches!(receipt.delivery,PlanningWorkerContinuationDelivery::RejectedBeforeStart{
            reason:PlanningWorkerContinuationRejection::StaleQuestion}))
    );
    assert_eq!(
        load_lease(&fixture.connection, "slot-1").unwrap(),
        Some(fixture.lease.clone())
    );
    let mut fixture = Fixture::new();
    fixture.enqueue();
    let newer = fixture.lease.clone().with_lease_generation("d".repeat(64));
    fixture
        .connection
        .execute(
            "UPDATE runtime_slot_leases SET content=?1 WHERE slot_id='slot-1'",
            params![serde_json::to_string(&newer).unwrap()],
        )
        .unwrap();
    assert!(
        matches!(fixture.claim(),PlanningWorkerContinuationClaimOutcome::Settled(receipt)
        if matches!(receipt.delivery,PlanningWorkerContinuationDelivery::RejectedBeforeStart{
            reason:PlanningWorkerContinuationRejection::InvalidLease}))
    );
    assert_eq!(
        load_lease(&fixture.connection, "slot-1").unwrap(),
        Some(newer)
    );
}

#[test]
fn continuation_waits_for_the_old_terminal_and_binds_immutable_payload() {
    let mut fixture = Fixture::new();
    let mut detail: ParallelModeAgentSessionDetailSnapshot = serde_json::from_str(
        &fixture
            .connection
            .query_row(
                "SELECT content FROM runtime_session_details WHERE session_key=?1",
                params![fixture.lease.session_key()],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    detail.state_label = "running".into();
    detail.completion_state_label = "in_progress".into();
    write_detail(&fixture.connection, &detail).unwrap();
    fixture.enqueue();
    assert_eq!(
        fixture.claim(),
        PlanningWorkerContinuationClaimOutcome::Waiting
    );
    let mut changed = fixture.request.clone();
    changed.message = "another answer".into();
    assert!(fixture.enqueue_request(&changed).is_err());
    changed = fixture.request.clone();
    changed.authorization_epoch = 1;
    assert!(fixture.enqueue_request(&changed).is_err());
    detail.state_label = "awaiting_user".into();
    detail.completion_state_label = "awaiting_user".into();
    write_detail(&fixture.connection, &detail).unwrap();
    let claim = fixture.claimed();
    assert_eq!(claim.request.message, fixture.request.message);
}

#[test]
fn continuation_ack_transaction_failure_preserves_unknown_and_waiting_task() {
    let mut fixture = Fixture::new();
    fixture.enqueue();
    let claim = fixture.claimed();
    fixture.connection.execute_batch("CREATE TRIGGER reject_continuation_task_mutation
        BEFORE INSERT ON planning_task_mutation_events BEGIN SELECT RAISE(ABORT,'synthetic ACK commit failure'); END;").unwrap();
    assert!(fixture.ack(&claim, "thread-a", "turn-a").is_err());
    assert_eq!(
        fixture.receipt().delivery,
        PlanningWorkerContinuationDelivery::Unknown
    );
    assert_eq!(fixture.task().status, TaskStatus::AwaitingUser);
    assert!(
        matches!(fixture.claim(),PlanningWorkerContinuationClaimOutcome::Settled(receipt)
        if receipt.delivery==PlanningWorkerContinuationDelivery::Unknown)
    );
}

#[test]
fn continuation_ack_records_delivery_without_reverting_a_newer_task_edit() {
    let mut fixture = Fixture::new();
    fixture.enqueue();
    let claim = fixture.claimed();
    fixture.change_task(|task| {
        task.status = TaskStatus::Cancelled;
        task.updated_at = "2026-11-21T00:02:00Z".into();
    });
    assert!(task_preserved_in_connection(&fixture.connection, "task-a").unwrap());
    assert!(matches!(
        fixture.ack(&claim, "thread-a", "turn-a").unwrap().delivery,
        PlanningWorkerContinuationDelivery::Accepted { .. }
    ));
    assert_eq!(fixture.task().status, TaskStatus::Cancelled);
}

#[test]
fn continuation_lineage_loss_cannot_inherit_legacy_dispatch_permission() {
    let mut fixture = Fixture::new();
    fixture.enqueue();
    let claim = fixture.claimed();
    fixture
        .connection
        .execute(
            "DELETE FROM planning_work_order_tasks WHERE task_id='task-a'",
            [],
        )
        .unwrap();
    let tx = fixture.connection.transaction().unwrap();
    let row = load(&tx, &fixture.request.command_id).unwrap().unwrap();
    assert!(validate_start(&tx, &row, &claim).is_err());
    tx.commit().unwrap();
    assert_eq!(
        fixture.receipt().delivery,
        PlanningWorkerContinuationDelivery::Unknown
    );
    assert!(
        matches!(fixture.claim(),PlanningWorkerContinuationClaimOutcome::Settled(receipt)
        if receipt.delivery==PlanningWorkerContinuationDelivery::Unknown)
    );
}

#[test]
fn stale_queued_answer_is_rejected_without_resolving_a_removed_worktree() {
    let mut fixture = Fixture::new();
    fixture.enqueue();
    fixture
        .connection
        .execute("DELETE FROM runtime_slot_leases WHERE slot_id='slot-1'", [])
        .unwrap();
    let tx = fixture
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let receipt = recheck_queued_in_transaction(&tx, &fixture.request.command_id).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        receipt.delivery,
        PlanningWorkerContinuationDelivery::RejectedBeforeStart {
            reason: PlanningWorkerContinuationRejection::InvalidLease
        }
    );
    assert_eq!(fixture.receipt(), receipt);
    assert!(!matches!(
        fixture.claim(),
        PlanningWorkerContinuationClaimOutcome::Claimed(_)
    ));
}

#[test]
fn rejected_continuation_allows_a_new_epoch_approval_and_preserves_the_first_receipt() {
    let mut fixture = Fixture::new();
    let original = fixture.request.clone();
    fixture.enqueue();
    fixture.authorize_epoch(1);
    let PlanningWorkerContinuationClaimOutcome::Settled(rejected) = fixture.claim() else {
        panic!("the old epoch must settle without a worker")
    };
    assert!(matches!(
        rejected.delivery,
        PlanningWorkerContinuationDelivery::RejectedBeforeStart { .. }
    ));
    let mut same_epoch = original.clone();
    same_epoch.command_id = "same-epoch-answer".into();
    assert!(fixture.enqueue_request(&same_epoch).is_err());
    fixture.request.command_id = "new-epoch-answer".into();
    fixture.request.authorization_epoch = 1;
    assert_eq!(
        fixture.enqueue().delivery,
        PlanningWorkerContinuationDelivery::Queued
    );
    assert_eq!(fixture.enqueue_request(&original).unwrap(), rejected);
    assert_eq!(
        load(&fixture.connection, &original.command_id)
            .unwrap()
            .unwrap()
            .receipt,
        rejected
    );
    assert_eq!(fixture.claimed().request.command_id, "new-epoch-answer");
    let count: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM runtime_worker_continuations WHERE work_order_id=?1 AND question_id=?2",
            params![original.work_order_id, original.question.question_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 2,
        "the rejection history survives the fresh approval"
    );
}

#[test]
fn a_fresh_answer_never_replaces_a_still_queued_question() {
    let mut fixture = Fixture::new();
    let original = fixture.enqueue();
    fixture.authorize_epoch(1);
    let mut candidate = fixture.request.clone();
    candidate.command_id = "competing-answer".into();
    candidate.authorization_epoch = 1;
    assert!(fixture.enqueue_request(&candidate).is_err());
    assert_eq!(fixture.receipt(), original);
    assert!(
        load(&fixture.connection, &candidate.command_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        load_lease(&fixture.connection, "slot-1").unwrap(),
        Some(fixture.lease)
    );
}

#[test]
fn unknown_and_accepted_questions_reject_new_answer_commands_even_in_a_fresh_epoch() {
    for accepted in [false, true] {
        let mut fixture = Fixture::new();
        fixture.enqueue();
        let claim = fixture.claimed();
        if accepted {
            fixture.ack(&claim, "thread-a", "turn-a").unwrap();
        }
        let original = fixture.receipt();
        fixture.authorize_epoch(1);
        let mut candidate = fixture.request.clone();
        candidate.command_id = "replacement-answer".into();
        candidate.authorization_epoch = 1;
        assert!(fixture.enqueue_request(&candidate).is_err());
        assert_eq!(fixture.receipt(), original);
        assert!(
            load(&fixture.connection, &candidate.command_id)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn the_same_question_id_is_independent_between_work_orders() {
    let deliveries = [
        PlanningWorkerContinuationDelivery::Queued,
        PlanningWorkerContinuationDelivery::Unknown,
        PlanningWorkerContinuationDelivery::Accepted {
            thread_id: "thread-other".into(),
            turn_id: "turn-other".into(),
            lease_generation: "e".repeat(64),
        },
        PlanningWorkerContinuationDelivery::RejectedBeforeStart {
            reason: PlanningWorkerContinuationRejection::InvalidLease,
        },
    ];
    for delivery in deliveries {
        let mut fixture = Fixture::new();
        let mut other_request = fixture.request.clone();
        other_request.command_id = "answer-other".into();
        other_request.work_order_id = "order-other".into();
        other_request.authorization_epoch = 9;
        other_request.question.task_id = "task-other".into();
        other_request.question.session_key = "session-other".into();
        other_request.question.thread_id = "thread-other".into();
        other_request.question.slot_id = "slot-other".into();
        other_request.question.worktree_path = "/repo/.pool/slot-other".into();
        other_request.question.branch_name = "akra-agent/slot-other/task-other".into();
        let other_receipt = PlanningWorkerContinuationReceipt {
            command_id: other_request.command_id.clone(),
            question_id: other_request.question.question_id.clone(),
            delivery,
        };
        // Another work's durable history cannot consume this question or require its newer epoch.
        save(
            &fixture.connection,
            &StoredContinuation {
                request: other_request.clone(),
                receipt: other_receipt.clone(),
                authorization_epoch: other_request.authorization_epoch,
                previous_lease: None,
                lease: None,
            },
        )
        .unwrap();

        assert_eq!(
            fixture.enqueue().delivery,
            PlanningWorkerContinuationDelivery::Queued
        );
        let stored_other = load(&fixture.connection, &other_request.command_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored_other.request, other_request);
        assert_eq!(stored_other.receipt, other_receipt);

        let mut duplicate = fixture.request.clone();
        duplicate.command_id = "duplicate-for-this-work".into();
        duplicate.authorization_epoch = 10;
        assert!(fixture.enqueue_request(&duplicate).is_err());
        assert!(
            load(&fixture.connection, &duplicate.command_id)
                .unwrap()
                .is_none()
        );
    }
}
