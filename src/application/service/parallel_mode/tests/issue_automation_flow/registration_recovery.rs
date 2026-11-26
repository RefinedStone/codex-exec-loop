//! Reassemble services after injected call-boundary failures; no OS process termination is simulated.
use super::support::{FlowFixture, FlowServices};
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderRepositoryPort;
use crate::application::service::issue_coordinator::IssueCoordinatorFact;
use crate::domain::issue_automation::coordinator::{IssueWorkBlockReason, IssueWorkCheckpoint};
use crate::domain::kernel_event::{KernelEvent, RETAINED_KERNEL_EVENTS, RecordedKernelEvent};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering;

mod fault_execution;
use fault_execution::{InterruptedSubmission, SubmissionFault, SubmissionProbe};

fn authority_task_ids(services: &FlowServices) -> Vec<String> {
    services
        .planning
        .load_task_authority_snapshot(&services.workspace)
        .unwrap()
        .expect("the real planning authority is initialized")
        .task_authority
        .tasks
        .into_iter()
        .map(|task| task.id)
        .collect()
}

fn assert_one_durable_task(services: &FlowServices, expected: &[String]) -> IssueWorkCheckpoint {
    assert_eq!(expected.len(), 1);
    assert_eq!(authority_task_ids(services), expected);
    assert_eq!(services.task_ids().unwrap(), expected);
    let work = services.work().unwrap().expect("the issue work is durable");
    assert_eq!(work.task_ids, expected);
    let planning_work = services
        .planning
        .load_work_order(&services.workspace, &work.order.work_order_id)
        .unwrap()
        .expect("the work-order identity survives service reassembly");
    assert_eq!(planning_work.task_ids, expected);
    assert_eq!(planning_work.initial_task_id().unwrap(), expected[0]);
    assert_eq!(
        services
            .planning
            .load_work_order_for_task(&services.workspace, &expected[0])
            .unwrap(),
        Some(planning_work)
    );
    work
}

fn submitted_event(facts: &[IssueCoordinatorFact]) -> KernelEvent {
    let submitted: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            IssueCoordinatorFact::WorkSubmitted {
                event_id,
                issue,
                work_order_id,
                task_ids,
                revision,
            } => Some(KernelEvent::IssueWorkSubmitted {
                event_id: event_id.clone(),
                issue: issue.into(),
                work_order_id: work_order_id.clone(),
                task_ids: task_ids.clone(),
                revision: *revision,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(
        submitted.len(),
        1,
        "one durable registration fact is replayed"
    );
    submitted.into_iter().next().unwrap()
}

fn retain_only_newer_events(services: &FlowServices, submitted: &KernelEvent) {
    let log = services.planning.as_ref();
    let workspace = &services.workspace;
    let first = log.last_kernel_event_sequence(workspace).unwrap() + 1;
    log.append_kernel_events(
        workspace,
        &[RecordedKernelEvent {
            sequence: first,
            recorded_at: "2026-11-22T00:00:00Z".into(),
            event: submitted.clone(),
        }],
    )
    .unwrap();
    assert_eq!(
        log.load_kernel_events(workspace, &["issue_work_submitted"])
            .unwrap(),
        vec![submitted.clone()]
    );
    let last = log.last_kernel_event_sequence(workspace).unwrap();
    let noise: Vec<_> = (1..=RETAINED_KERNEL_EVENTS + 1)
        .map(|offset| RecordedKernelEvent {
            sequence: last + offset,
            recorded_at: "2026-11-22T00:01:00Z".into(),
            event: KernelEvent::TurnStarted {
                thread_id: "retention-fixture-thread".into(),
                turn_id: format!("retention-fixture-turn-{offset}"),
                op_id: None,
            },
        })
        .collect();
    assert_eq!(noise.len(), 10_001);
    // Exercise the production append transaction and its retention DELETE, not direct SQL pruning.
    log.append_kernel_events(workspace, &noise).unwrap();
    assert_eq!(
        log.load_kernel_events(workspace, &["turn_started"])
            .unwrap()
            .len(),
        usize::try_from(RETAINED_KERNEL_EVENTS).unwrap()
    );
    assert!(
        log.load_kernel_events(workspace, &["issue_work_submitted"])
            .unwrap()
            .is_empty()
    );
    assert!(
        log.has_issue_work_event(workspace, submitted.issue_work_event_id().unwrap())
            .unwrap()
    );
}

fn registration_recovers_after_reassembly_and_retention(fault: SubmissionFault) {
    let prefix = match fault {
        SubmissionFault::BeforeCommit => "issue-flow-before-registration",
        SubmissionFault::AfterCommit => "issue-flow-registration-ack-loss",
        SubmissionFault::ProcessDeath => unreachable!("process death has its own test"),
    };
    let fixture = FlowFixture::new(prefix);
    let probe = Arc::new(SubmissionProbe::default());
    let services = fixture.services_with_execution(|inner| {
        Arc::new(InterruptedSubmission::new(inner, fault, probe.clone()))
    });
    services.collect_and_triage().unwrap();
    services.tick(1).unwrap();
    services.approve_start().unwrap();
    assert!(authority_task_ids(&services).is_empty());
    assert!(
        services.tick(2).is_err(),
        "the registration boundary is interrupted"
    );
    assert_eq!(probe.attempts.load(Ordering::SeqCst), 1);
    let interrupted = services
        .work()
        .unwrap()
        .expect("intent is saved before submit");
    assert!(
        interrupted.task_ids.is_empty(),
        "the caller never received registration ACK"
    );
    assert_eq!(
        interrupted.blocked_reason,
        Some(IssueWorkBlockReason::SubmissionUnavailable)
    );
    let committed_before_restart = authority_task_ids(&services);
    match fault {
        SubmissionFault::BeforeCommit => {
            assert_eq!(probe.committed.load(Ordering::SeqCst), 0);
            assert!(committed_before_restart.is_empty());
            assert!(
                services
                    .planning
                    .load_work_order(&services.workspace, &interrupted.order.work_order_id,)
                    .unwrap()
                    .is_none()
            );
        }
        SubmissionFault::AfterCommit | SubmissionFault::ProcessDeath => {
            assert_eq!(probe.committed.load(Ordering::SeqCst), 1);
            assert_eq!(committed_before_restart.len(), 1);
            assert_eq!(
                services
                    .execution
                    .task_ids(&services.workspace, &interrupted.order.work_order_id,)
                    .unwrap(),
                committed_before_restart
            );
        }
    }
    let workspace = services.workspace.clone();
    drop(services);

    // New coordinator, issue adapter, planning adapter and real execution adapter; same durable paths.
    let recovered = fixture.services();
    assert_eq!(recovered.workspace, workspace);
    let facts = recovered.tick(3).unwrap();
    let task_ids = authority_task_ids(&recovered);
    assert_one_durable_task(&recovered, &task_ids);
    if matches!(fault, SubmissionFault::AfterCommit) {
        assert_eq!(
            task_ids, committed_before_restart,
            "ACK loss reuses the committed task"
        );
    }
    let submitted = submitted_event(&facts);
    let KernelEvent::IssueWorkSubmitted {
        work_order_id,
        task_ids: submitted_tasks,
        ..
    } = &submitted
    else {
        unreachable!()
    };
    assert_eq!(work_order_id, &interrupted.order.work_order_id);
    assert_eq!(submitted_tasks, &task_ids);
    let event_id = submitted.issue_work_event_id().unwrap().to_string();
    retain_only_newer_events(&recovered, &submitted);
    assert_one_durable_task(&recovered, &task_ids);
    // Leave the issue outbox unacknowledged, exactly as an interrupted recipient would.
    drop(recovered);

    let replay = fixture.services();
    assert_eq!(replay.workspace, workspace);
    assert!(
        replay
            .planning
            .has_issue_work_event(&workspace, &event_id)
            .unwrap()
    );
    assert!(
        replay
            .planning
            .load_kernel_events(&workspace, &["issue_work_submitted"])
            .unwrap()
            .is_empty()
    );
    let replayed = replay.tick(4).unwrap();
    assert_eq!(submitted_event(&replayed), submitted);
    assert_one_durable_task(&replay, &task_ids);
    // A repeated delivery after event retention remains a no-op through the durable receipt table.
    let last = replay
        .planning
        .last_kernel_event_sequence(&workspace)
        .unwrap();
    replay
        .planning
        .append_kernel_events(
            &workspace,
            &[RecordedKernelEvent {
                sequence: last + 1,
                recorded_at: "2026-11-22T00:02:00Z".into(),
                event: submitted,
            }],
        )
        .unwrap();
    assert_eq!(
        replay
            .planning
            .last_kernel_event_sequence(&workspace)
            .unwrap(),
        last
    );
    replay.coordinator.acknowledge_event(&event_id).unwrap();
    drop(replay);

    let acknowledged = fixture.services();
    let final_facts = acknowledged.tick(5).unwrap();
    assert!(
        !final_facts
            .iter()
            .any(|fact| matches!(fact, IssueCoordinatorFact::WorkSubmitted { .. }))
    );
    assert_one_durable_task(&acknowledged, &task_ids);
}

#[test]
fn issue_automation_flow_registration_before_commit_recovers_one_task_after_event_retention() {
    registration_recovers_after_reassembly_and_retention(SubmissionFault::BeforeCommit);
}

#[test]
fn issue_automation_flow_registration_ack_loss_keeps_one_task_after_event_retention() {
    registration_recovers_after_reassembly_and_retention(SubmissionFault::AfterCommit);
}

#[test]
fn issue_automation_flow_process_death_after_registration_commit_keeps_one_task_and_one_comment() {
    let fixture = FlowFixture::new("issue-flow-registration-death");
    let probe = Arc::new(SubmissionProbe::default());
    let services = fixture.services_with_execution(|inner| {
        Arc::new(InterruptedSubmission::new(
            inner,
            SubmissionFault::ProcessDeath,
            probe.clone(),
        ))
    });
    services.collect_and_triage().unwrap();
    services.tick(1).unwrap();
    services.approve_start().unwrap();
    let died = catch_unwind(AssertUnwindSafe(|| services.tick(2)));
    assert!(died.is_err(), "the process dies inside submit");
    assert_eq!(probe.committed.load(Ordering::SeqCst), 1);
    let committed = authority_task_ids(&services);
    assert_eq!(committed.len(), 1, "the task registration committed");
    let orphan = services
        .work()
        .unwrap()
        .expect("intent is saved before submit");
    assert!(
        orphan.task_ids.is_empty(),
        "the checkpoint never saw the task"
    );
    assert_ne!(
        orphan.blocked_reason,
        Some(IssueWorkBlockReason::SubmissionUnavailable),
        "no caller error path ran"
    );
    assert_eq!(fixture.provider.create_count(), 0);
    drop(services);

    // Same workspace, SQLite files and remote doubles; every service is new.
    let recovered = fixture.services();
    recovered.tick(3).unwrap();
    recovered.tick(4).unwrap();
    assert_one_durable_task(&recovered, &committed);
    assert_eq!(fixture.provider.create_count(), 1);
    assert_eq!(fixture.provider.comment_count(), 1);
}
