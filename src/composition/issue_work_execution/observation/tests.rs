use super::*;
use std::collections::BTreeMap;

use crate::domain::parallel_mode::{
    IntegrationAttestation, IntegrationMethod, ParallelModeAgentSessionHistoryEntry,
    ParallelModeLiveSessionDetailDefaults, PrValidationCommitSha, PrValidationEvent,
    PrValidationFindingKey, PrValidationFindingSource, PrValidationRemediationCorrelation,
    PrValidationTarget, PrValidationTargetShaSnapshot,
};
use crate::domain::planning::{PLANNING_FORMAT_VERSION, TaskActor, TaskMutationProvenance};

mod legacy_validation;

const STAMP: &str = "2026-11-21T09:00:00Z";

fn task(id: &str, status: TaskStatus) -> TaskDefinition {
    TaskDefinition {
        acceptance: Vec::new(),
        acceptance_revision: 0,
        id: id.into(),
        direction_id: "direction".into(),
        direction_relation_note: "issue work".into(),
        title: id.into(),
        description: "Shared task description".into(),
        status,
        base_priority: 80,
        dynamic_priority_delta: 0,
        priority_reason: String::new(),
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        created_by: TaskActor::User,
        last_updated_by: TaskActor::User,
        source_turn_id: None,
        provenance: TaskMutationProvenance::default(),
        updated_at: STAMP.into(),
    }
}

fn tasks(values: Vec<TaskDefinition>) -> TaskAuthorityDocument {
    TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: values,
    }
}

fn order(ids: &[&str]) -> PlanningWorkOrderRecord {
    PlanningWorkOrderRecord {
        link_reference: None,
        work_order_id: "issue-work-1".into(),
        origin: "issue_automation".into(),
        task_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        permits: Vec::new(),
    }
}

fn runtime() -> PlanningAuthorityRuntimeProjectionSnapshot {
    PlanningAuthorityRuntimeProjectionSnapshot {
        slot_leases: BTreeMap::new(),
        invalid_slot_leases: BTreeSet::new(),
        session_details: Vec::new(),
        task_dispatch_blocks: Vec::new(),
        distributor_queue_records: Vec::new(),
        dispatch_commands: Vec::new(),
        runtime_events: Vec::new(),
    }
}

fn lease(task_id: &str, generation: char) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        task_id,
        task_id,
        "agent-1",
        "akra-agent/slot-1/work",
        "/work/pool/slot-1",
        ParallelModeSlotLeaseState::Running,
        STAMP,
        Some(STAMP.into()),
    )
    .with_lease_generation(generation.to_string().repeat(64))
}

fn detail(
    lease: &ParallelModeSlotLeaseSnapshot,
    state: &str,
) -> ParallelModeAgentSessionDetailSnapshot {
    let mut value = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
        lease,
        ParallelModeLiveSessionDetailDefaults {
            validation_summary: "not yet validated",
            authority_refresh_outcome: "not yet refreshed",
        },
    );
    value.thread_id = Some("worker-thread-1".into());
    value.state_label = state.into();
    value.completion_state_label = if matches!(state, "starting" | "running" | "awaiting_user") {
        "in_progress".into()
    } else {
        state.into()
    };
    if state != "starting" {
        value
            .history
            .push(ParallelModeAgentSessionHistoryEntry::new(
                "running",
                STAMP,
                "turn started",
            ));
    }
    value
}

fn queue(
    lease: &ParallelModeSlotLeaseSnapshot,
    pull_request_number: Option<u64>,
) -> PlanningAuthorityDistributorQueueRecord {
    serde_json::from_value(serde_json::json!({
        "queue_item_id": "delivery-1", "session_key": lease.session_key(),
        "slot_id": lease.slot_id, "agent_id": lease.agent_id,
        "task_id": lease.task_id, "task_title": lease.task_title,
        "branch_name": lease.branch_name, "worktree_path": lease.worktree_path,
        "commit_sha": "a".repeat(40), "source_commit_sha": "a".repeat(40),
        "source_base_commit_sha": "b".repeat(40),
        "validation_summary": "passed", "authority_refresh_outcome": "accepted",
        "queue_state": ParallelModeQueueItemState::Queued,
        "integration_state": "pending", "integration_note": "",
        "enqueued_at": STAMP, "updated_at": STAMP, "pull_request_number": pull_request_number
    }))
    .unwrap()
}

fn validation(key: &str) -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new(key).unwrap(),
        PrValidationTarget::new("sample/widgets", 2551).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("a".repeat(40)).unwrap(),
            PrValidationCommitSha::new("b".repeat(40)).unwrap(),
        ),
    )
    .transition(PrValidationEvent::BeginPreMergeObservation)
    .unwrap()
}

fn checkpoint(record: &PrValidationRecord, revision: u64) -> PrValidationRecord {
    record
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: revision,
            cursor: Some(format!("cursor-{revision}")),
            evidence_fingerprint: "provider-evidence".into(),
        })
        .unwrap()
}

fn evidence(record: PrValidationRecord, source_task: &str) -> ValidationEvidence {
    ValidationEvidence {
        source_task_id: source_task.into(),
        record,
        superseded: false,
        remediation_origin_key: None,
    }
}

fn finding() -> PrValidationFinding {
    PrValidationFinding::new(
        PrValidationFindingKey::new(
            PrValidationFindingSource::new("required-check").unwrap(),
            "ci-attempt-1",
        )
        .unwrap(),
        PrValidationCommitSha::new("a".repeat(40)).unwrap(),
        "The configured check requires a correction",
    )
    .unwrap()
}

fn needs_rework() -> PrValidationRecord {
    let finding = finding();
    validation("validation-1")
        .transition(PrValidationEvent::FindingObserved(finding.clone()))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                finding.key().clone(),
                PrValidationRecordKey::new("remediation").unwrap(),
            ),
        ))
        .unwrap()
}

fn integrated(key: &str) -> PrValidationRecord {
    let now = chrono::DateTime::parse_from_rfc3339(STAMP)
        .unwrap()
        .with_timezone(&chrono::Utc);
    validation(key)
        .transition(PrValidationEvent::IntegrationAttested(
            IntegrationAttestation::new(
                IntegrationMethod::DistributorCherryPick,
                PrValidationCommitSha::new("a".repeat(40)).unwrap(),
                Some(PrValidationCommitSha::new("b".repeat(40)).unwrap()),
                PrValidationCommitSha::new("c".repeat(40)).unwrap(),
                Some(2551),
                None,
                now,
                now,
            )
            .unwrap(),
        ))
        .unwrap()
}

#[test]
fn terminal_reasons_keep_the_existing_recovery_classification() {
    use IssueExecutionTerminalClass as Class;
    use PrValidationTerminalReason as Reason;
    for (reason, class) in [
        (
            Reason::PullRequestClosedWithoutMerge,
            Class::PullRequestClosed,
        ),
        (Reason::IntegrationEvidenceMissing, Class::ContractRecovery),
        (
            Reason::IntegrationAuthorityConflict,
            Class::IdentityConflict,
        ),
        (Reason::PostMergeContractBlocked, Class::ContractRecovery),
        (Reason::IdentityViolation, Class::IdentityConflict),
        (Reason::ObservationFailed, Class::ObserveAgain),
        (Reason::RemediationAdmissionFailed, Class::QueueRecovery),
    ] {
        assert_eq!(terminal_class(&reason).map(|value| value.0), Some(class));
    }
    assert_eq!(terminal_class(&Reason::AllConfiguredSourcesComplete), None);
}

#[test]
fn terminal_identity_survives_restart_and_distinguishes_validation_rounds() {
    let first = checkpoint(&validation("validation-1"), 3)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let restored: PrValidationRecord =
        serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
    assert_eq!(validation_terminal(&first), validation_terminal(&restored));

    let later = checkpoint(&validation("validation-1"), 4)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    assert_ne!(validation_terminal(&first), validation_terminal(&later));
    let fresh_record = checkpoint(&validation("validation-2"), 3)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    assert_ne!(
        validation_terminal(&first),
        validation_terminal(&fresh_record)
    );
}

#[test]
fn rework_identity_does_not_change_while_polling_the_same_finding() {
    let first = needs_rework();
    let later = checkpoint(&first, 9);
    let tasks = tasks(vec![
        task("initial", TaskStatus::Done),
        task("remediation", TaskStatus::Ready),
    ]);
    let first = project(
        &order(&["initial", "remediation"]),
        &tasks,
        &runtime(),
        &[evidence(first, "initial")],
        &["initial".into()],
    )
    .unwrap();
    let later = project(
        &order(&["initial", "remediation"]),
        &tasks,
        &runtime(),
        &[evidence(later, "initial")],
        &["initial".into()],
    )
    .unwrap();
    assert_eq!(first.phase, IssueExecutionPhase::Queued);
    assert_eq!(first.execution_ids, vec!["initial"]);
    assert_eq!(first.current_terminal_id, later.current_terminal_id);
    assert_eq!(first.terminals, later.terminals);
    assert_eq!(
        first.terminals[0].classification,
        IssueExecutionTerminalClass::ReworkNeeded
    );
}

#[test]
fn an_unadmitted_actionable_finding_is_visible_without_inventing_an_execution() {
    let record = validation("validation-1")
        .transition(PrValidationEvent::FindingObserved(finding()))
        .unwrap();
    let observed = project(
        &order(&["initial"]),
        &tasks(vec![task("initial", TaskStatus::Done)]),
        &runtime(),
        &[evidence(record, "initial")],
        &["initial".into()],
    )
    .unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::Terminal);
    assert!(observed.current_terminal_id.is_some());
    assert_eq!(observed.execution_ids, vec!["initial"]);
    assert_eq!(
        observed.terminals[0].classification,
        IssueExecutionTerminalClass::ReworkNeeded
    );
}

#[test]
fn a_running_remediation_keeps_old_findings_as_history() {
    let mut runtime = runtime();
    let lease = lease("remediation", 'a');
    runtime.session_details.push(detail(&lease, "running"));
    runtime.slot_leases.insert(lease.slot_id.clone(), lease);
    let observed = project(
        &order(&["initial", "remediation"]),
        &tasks(vec![
            task("initial", TaskStatus::Done),
            task("remediation", TaskStatus::InProgress),
        ]),
        &runtime,
        &[evidence(needs_rework(), "initial")],
        &["initial".into()],
    )
    .unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::Running);
    assert_eq!(observed.active_task_id.as_deref(), Some("remediation"));
    assert_eq!(observed.execution_ids, vec!["initial", "remediation"]);
    assert_eq!(observed.current_terminal_id, None);
    assert_eq!(observed.terminals.len(), 1);
}

#[test]
fn a_prepared_thread_or_planning_status_does_not_prove_execution() {
    let mut runtime = runtime();
    let mut lease = lease("initial", 'a');
    lease.state = ParallelModeSlotLeaseState::Leased;
    lease.running_started_at = None;
    runtime.session_details.push(detail(&lease, "starting"));
    runtime.slot_leases.insert(lease.slot_id.clone(), lease);
    let observed = project(
        &order(&["initial"]),
        &tasks(vec![task("initial", TaskStatus::InProgress)]),
        &runtime,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::Queued);
    assert!(observed.execution_ids.is_empty());
}

#[test]
fn started_ids_and_retained_running_history_are_counted_once_and_scoped() {
    let mut runtime = runtime();
    let owned_lease = lease("initial", 'a');
    runtime
        .session_details
        .push(detail(&owned_lease, "cleaned"));
    runtime
        .session_details
        .push(detail(&lease("foreign", 'b'), "running"));
    let observed = project(
        &order(&["initial"]),
        &tasks(vec![task("initial", TaskStatus::Done)]),
        &runtime,
        &[],
        &["initial".into(), "initial".into(), "foreign".into()],
    )
    .unwrap();
    assert_eq!(observed.execution_ids, vec!["initial"]);
    let history_only = project(
        &order(&["initial"]),
        &tasks(vec![task("initial", TaskStatus::Done)]),
        &runtime,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(history_only.execution_ids, vec!["initial"]);
}

#[test]
fn a_retained_running_lease_does_not_hide_an_open_pull_request() {
    for status in [TaskStatus::Ready, TaskStatus::InProgress, TaskStatus::Done] {
        let mut runtime = runtime();
        let lease = lease("initial", 'a');
        runtime
            .distributor_queue_records
            .push(queue(&lease, Some(2551)));
        runtime.session_details.push(detail(&lease, "pr_pending"));
        runtime.slot_leases.insert(lease.slot_id.clone(), lease);
        let observed = project(
            &order(&["initial"]),
            &tasks(vec![task("initial", status)]),
            &runtime,
            &[evidence(validation("validation-1"), "initial")],
            &[],
        )
        .unwrap();
        assert_eq!(observed.phase, IssueExecutionPhase::InReview);
        assert_eq!(observed.current_terminal_id, None);
    }
}

#[test]
fn a_remediations_own_validation_supersedes_the_parent_finding() {
    let observed = project(
        &order(&["initial", "remediation"]),
        &tasks(vec![
            task("initial", TaskStatus::Done),
            task("remediation", TaskStatus::Done),
        ]),
        &runtime(),
        &[
            evidence(needs_rework(), "initial"),
            evidence(validation("validation-2"), "remediation"),
        ],
        &["initial".into(), "remediation".into()],
    )
    .unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::InReview);
    assert_eq!(observed.current_terminal_id, None);
    assert_eq!(observed.terminals.len(), 1);
}

#[test]
fn terminal_failure_is_not_hidden_by_stale_ready_status() {
    let failed = validation("validation-1")
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let observed = project(
        &order(&["initial"]),
        &tasks(vec![task("initial", TaskStatus::Ready)]),
        &runtime(),
        &[evidence(failed, "initial")],
        &[],
    )
    .unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::Terminal);
    assert_eq!(
        observed.terminals[0].classification,
        IssueExecutionTerminalClass::ObserveAgain
    );
}

#[test]
fn post_merge_contract_failure_keeps_integration_and_never_invents_rework() {
    let record = checkpoint(&integrated("validation-1"), 8)
        .transition(PrValidationEvent::Block(
            PrValidationTerminalReason::PostMergeContractBlocked,
        ))
        .unwrap();
    let observed = project(
        &order(&["initial"]),
        &tasks(vec![task("initial", TaskStatus::Done)]),
        &runtime(),
        &[evidence(record, "initial")],
        &["initial".into()],
    )
    .unwrap();
    observed.validate().unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::Integrated);
    assert_eq!(observed.execution_ids, vec!["initial"]);
    assert_eq!(
        observed.terminals[0].classification,
        IssueExecutionTerminalClass::ContractRecovery
    );
    assert_eq!(
        observed.integration.as_ref().unwrap().integration_sha,
        "c".repeat(40)
    );
    assert!(observed.question.is_none());
}

#[test]
fn the_same_integration_has_one_identity_across_fresh_validation_records() {
    let first = integration_evidence(
        &[evidence(integrated("validation-1"), "initial")],
        &[],
        &|_| true,
    )
    .unwrap();
    let later = integration_evidence(
        &[evidence(
            checkpoint(&integrated("validation-2"), 20),
            "initial",
        )],
        &[],
        &|_| true,
    )
    .unwrap();
    assert_eq!(first, later);
}

#[test]
fn direct_delivery_requires_a_committed_success_and_ignores_foreign_work() {
    let lease = lease("initial", 'a');
    let mut delivery = queue(&lease, None);
    delivery.integration_commit_sha = Some("c".repeat(40));
    assert!(
        integration_evidence(&[], &[delivery.clone()], &|id| id == "initial")
            .unwrap()
            .is_none()
    );
    delivery.queue_state = ParallelModeQueueItemState::Done;
    delivery.integration_state = "done".into();
    let observed = integration_evidence(&[], &[delivery.clone()], &|id| id == "initial")
        .unwrap()
        .unwrap();
    assert_eq!(observed.source_sha, "a".repeat(40));
    assert!(observed.pull_request_url.is_none());
    assert!(
        integration_evidence(&[], &[delivery], &|_| false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn waiting_question_uses_the_exact_current_generation_and_raw_task_timestamp() {
    let mut runtime = runtime();
    let current = lease("initial", 'b');
    let historical = lease("initial", 'a');
    runtime.session_details.push(detail(&historical, "cleaned"));
    runtime
        .session_details
        .push(detail(&current, "awaiting_user"));
    runtime
        .slot_leases
        .insert(current.slot_id.clone(), current.clone());
    let task = task("initial", TaskStatus::AwaitingUser);
    let first = question_target(&task, &runtime).unwrap();
    runtime.session_details.reverse();
    let repeated = question_target(&task, &runtime).unwrap();
    assert_eq!(first, repeated);
    assert_eq!(first.session_key, current.session_key());
    assert_eq!(first.lease_generation, "b".repeat(64));
    assert_eq!(first.worktree_path, current.worktree_path);
    assert_eq!(first.awaiting_task_updated_at, task.updated_at);
    let observed = project(&order(&["initial"]), &tasks(vec![task]), &runtime, &[], &[]).unwrap();
    observed.validate().unwrap();
    assert_eq!(observed.phase, IssueExecutionPhase::Running);
    assert_eq!(observed.question, Some(first));
}

#[test]
fn question_identity_changes_when_awaiting_authority_changes() {
    let mut runtime = runtime();
    let lease = lease("initial", 'a');
    runtime
        .session_details
        .push(detail(&lease, "awaiting_user"));
    runtime.slot_leases.insert(lease.slot_id.clone(), lease);
    let first = task("initial", TaskStatus::AwaitingUser);
    let mut changed = first.clone();
    changed.updated_at = "2026-11-21T09:01:00+00:00".into();
    assert_ne!(
        question_target(&first, &runtime).unwrap().question_id,
        question_target(&changed, &runtime).unwrap().question_id
    );
    assert_eq!(
        question_target(&changed, &runtime)
            .unwrap()
            .awaiting_task_updated_at,
        changed.updated_at
    );
}

#[test]
fn absent_legacy_ambiguous_or_drifted_leases_do_not_offer_resumable_questions() {
    let task = task("initial", TaskStatus::AwaitingUser);
    let mut runtime = runtime();
    let mut current = lease("initial", 'a');
    runtime
        .session_details
        .push(detail(&current, "awaiting_user"));
    assert!(question_target(&task, &runtime).is_none());

    current.lease_generation = None;
    runtime.session_details = vec![detail(&current, "awaiting_user")];
    runtime.slot_leases.insert(current.slot_id.clone(), current);
    assert!(question_target(&task, &runtime).is_none());

    let current = lease("initial", 'a');
    runtime.session_details = vec![detail(&current, "awaiting_user")];
    runtime
        .slot_leases
        .insert(current.slot_id.clone(), current.clone());
    runtime.session_details[0].worktree_path.push_str("-other");
    assert!(question_target(&task, &runtime).is_none());

    runtime.session_details = vec![detail(&current, "awaiting_user")];
    let mut duplicate = lease("initial", 'b');
    duplicate.slot_id = "slot-2".into();
    runtime
        .slot_leases
        .insert(duplicate.slot_id.clone(), duplicate);
    assert!(question_target(&task, &runtime).is_none());
}

#[test]
fn incomplete_or_foreign_lineage_fails_closed() {
    assert!(
        project(
            &order(&["initial", "missing"]),
            &tasks(vec![task("initial", TaskStatus::Ready)]),
            &runtime(),
            &[],
            &[]
        )
        .is_err()
    );
    assert!(
        project(
            &order(&["initial"]),
            &tasks(vec![task("initial", TaskStatus::Done)]),
            &runtime(),
            &[evidence(validation("other"), "foreign")],
            &[]
        )
        .is_err()
    );
}

#[test]
fn an_unknown_continuation_generation_never_opens_another_question() {
    let current = lease("initial", 'b');
    let mut runtime = runtime();
    runtime
        .slot_leases
        .insert(current.slot_id.clone(), current.clone());
    let mut resumed = detail(&current, "awaiting_user");
    resumed.completion_state_label = "continuation_unknown".into();
    runtime.session_details.push(resumed);
    let waiting = task("initial", TaskStatus::AwaitingUser);
    assert!(question_target(&waiting, &runtime).is_none());

    runtime.session_details[0].state_label = "running".into();
    assert!(question_target(&waiting, &runtime).is_none());
    runtime.session_details[0].completion_state_label = "awaiting_user".into();
    assert!(question_target(&waiting, &runtime).is_some());
}

#[test]
fn recovery_successor_owns_current_state_and_copied_findings_keep_one_receipt_identity() {
    let original = checkpoint(&needs_rework(), 99)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let successor = original
        .fresh_recovery_successor(PrValidationRecordKey::new("validation-successor").unwrap())
        .unwrap();
    let original_terminal = validation_terminal(&original).unwrap();
    let documents = tasks(vec![
        task("initial", TaskStatus::Done),
        task("remediation", TaskStatus::Done),
    ]);
    let observe = |current: PrValidationRecord| {
        let mut historical = evidence(original.clone(), "initial");
        historical.superseded = true;
        let mut latest = evidence(current, "initial");
        latest.remediation_origin_key = Some(original.key().as_str().to_owned());
        // Deliberately reverse the link order: a known recovery parent cannot win even with
        // a higher observation revision or a later position in a stale projection.
        project(
            &order(&["initial", "remediation"]),
            &documents,
            &runtime(),
            &[latest, historical],
            &["initial".into(), "remediation".into()],
        )
        .unwrap()
    };
    let fresh = observe(successor.clone());
    assert_eq!(fresh.phase, IssueExecutionPhase::InReview);
    assert_eq!(fresh.current_terminal_id, None);
    assert_eq!(
        fresh
            .terminals
            .iter()
            .filter(|value| value.classification == IssueExecutionTerminalClass::ReworkNeeded)
            .count(),
        1
    );
    assert!(fresh.terminals.contains(&original_terminal));
    let first_poll = checkpoint(
        &successor
            .transition(PrValidationEvent::BeginPreMergeObservation)
            .unwrap(),
        1,
    );
    let later_poll = checkpoint(&first_poll, 9);
    assert_eq!(observe(first_poll).terminals, fresh.terminals);
    assert_eq!(observe(later_poll.clone()).terminals, fresh.terminals);
    let failed_successor = later_poll
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let failed = observe(failed_successor.clone());
    assert_eq!(failed.phase, IssueExecutionPhase::Terminal);
    assert_ne!(
        failed.current_terminal_id.as_deref(),
        Some(original_terminal.record_id.as_str())
    );
    assert_eq!(
        failed
            .terminals
            .iter()
            .filter(|value| value.classification.needs_verification_attempt())
            .count(),
        2
    );
    assert_eq!(observe(failed_successor), failed);
    assert_eq!(failed.execution_ids, fresh.execution_ids);
}

#[test]
fn recovery_origin_is_resolved_once_per_observation_and_refreshed_on_the_next_tick() {
    let origin = PrValidationRecordKey::new("origin").unwrap();
    let other = PrValidationRecordKey::new("other-origin").unwrap();
    let first = PrValidationRecordKey::new("successor-130").unwrap();
    let latest = PrValidationRecordKey::new("successor-131").unwrap();
    let calls = std::cell::Cell::new(0);
    let mut cache = BTreeMap::new();
    for _ in 0..131 {
        assert_eq!(
            cached_recovery_leaf(&mut cache, &origin, |key| {
                assert_eq!(key, &origin);
                calls.set(calls.get() + 1);
                Ok(first.clone())
            })
            .unwrap(),
            first,
        );
    }
    assert_eq!(calls.get(), 1);
    assert_eq!(
        cached_recovery_leaf(&mut cache, &other, |_| {
            calls.set(calls.get() + 1);
            Ok(other.clone())
        })
        .unwrap(),
        other,
    );
    assert_eq!(calls.get(), 2);

    let mut next_tick = BTreeMap::new();
    assert_eq!(
        cached_recovery_leaf(&mut next_tick, &origin, |_| {
            calls.set(calls.get() + 1);
            Ok(latest.clone())
        })
        .unwrap(),
        latest,
    );
    assert_eq!(calls.get(), 3);
}

#[test]
fn postintegration_successor_keeps_integration_identity_and_counts_only_fresh_verification() {
    use crate::domain::issue_automation::execution::{IssueExecutionProgress, IssueExecutionState};
    let original = checkpoint(&integrated("validation-integrated"), 99)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let successor = original
        .fresh_recovery_successor(PrValidationRecordKey::new("validation-successor").unwrap())
        .unwrap();
    let documents = tasks(vec![task("initial", TaskStatus::Done)]);
    let before = project(
        &order(&["initial"]),
        &documents,
        &runtime(),
        &[evidence(original.clone(), "initial")],
        &["initial".into()],
    )
    .unwrap();
    let observe = |current: PrValidationRecord| {
        let mut historical = evidence(original.clone(), "initial");
        historical.superseded = true;
        let mut current = evidence(current, "initial");
        current.remediation_origin_key = Some(original.key().as_str().to_owned());
        project(
            &order(&["initial"]),
            &documents,
            &runtime(),
            &[current, historical],
            &["initial".into()],
        )
        .unwrap()
    };
    let fresh = observe(successor.clone());
    assert_eq!(fresh.phase, IssueExecutionPhase::Integrated);
    assert_eq!(fresh.integration, before.integration);
    assert_eq!(fresh.current_terminal_id, None);
    assert_eq!(fresh.terminals, before.terminals);
    let done = IssueExecutionProgress::default()
        .reconcile(&before, 1, 1, 0)
        .unwrap()
        .reconcile(&fresh, 0, 0, 0)
        .unwrap();
    assert_eq!(done.state, IssueExecutionState::Done);
    assert_eq!(done.attempts.execution, 1);
    assert_eq!(done.attempts.verification, 1);
    assert!(!done.allows_additional_execution(100));

    let failed = checkpoint(&successor, 1)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let after = observe(failed.clone());
    assert_eq!(after.integration, before.integration);
    assert_ne!(after.current_terminal_id, before.current_terminal_id);
    assert_eq!(after.terminals.len(), 2);
    assert_eq!(observe(failed), after);
    let still_done = done.reconcile(&after, 0, 1, 0).unwrap();
    assert_eq!(still_done.state, IssueExecutionState::Done);
    assert_eq!(still_done.attempts.execution, 1);
    assert_eq!(still_done.attempts.verification, 2);
    assert_eq!(still_done.integration, before.integration);
}

mod review_link;
