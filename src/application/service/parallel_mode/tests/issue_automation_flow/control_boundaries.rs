//! Issue observations and operator controls must reach the durable planning gate.
use super::support::FlowServices;
use super::*;
use crate::application::port::inbound::issue_intervention_port::IssueInterventionPort;
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitDecision, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest,
};
use crate::domain::issue_automation::source::{
    IssueRemoteState, IssueSourceContent, IssueSourceFailure, IssueSourceRead,
};
use crate::domain::issue_work::IssueFieldHash;
use std::sync::atomic::AtomicUsize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitingChange {
    Hold,
    StaleFields,
    Closed,
    SelectionLabelRemoved,
    Unreadable,
}

impl WaitingChange {
    fn label(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::StaleFields => "stale-fields",
            Self::Closed => "closed",
            Self::SelectionLabelRemoved => "selection-label-removed",
            Self::Unreadable => "unreadable",
        }
    }
}

fn context() -> StoredCommandContext {
    StoredCommandContext {
        human_confirmation: None,
        caller: Some("flow-operator".into()),
        channel: "test".into(),
        at: "2026-11-22T00:00:00Z".into(),
    }
}

fn change_waiting_issue(fixture: &FlowFixture, services: &FlowServices, change: WaitingChange) {
    if change == WaitingChange::Hold {
        let receipt = services
            .interventions
            .intervene(
                IssueInterventionRequest {
                    command_id: "hold-before-slot-claim".into(),
                    issue: fixture.issue.clone(),
                    action: IssueInterventionAction::Hold,
                },
                &context(),
            )
            .unwrap();
        assert!(receipt.changed);
        assert_eq!(receipt.issue, fixture.issue);
        return;
    }
    let mut read = fixture.source.read.lock().unwrap();
    if change == WaitingChange::Unreadable {
        *read = IssueSourceRead::Unknown {
            issue: fixture.issue.clone(),
            reason: IssueSourceFailure::ProviderUnavailable,
        };
        return;
    }
    let IssueSourceRead::Observed(observation) = &mut *read else {
        panic!("the source starts with one complete observation");
    };
    match change {
        WaitingChange::StaleFields => {
            let IssueSourceContent::Shared(snapshot) = &mut observation.content else {
                panic!("the approved issue has shared fields");
            };
            snapshot.shared_copy.title = "승인 후 변경된 요구사항".into();
            snapshot.basis.field_hash =
                IssueFieldHash::of_fields(&snapshot.shared_copy.title, &snapshot.shared_copy.body);
            snapshot.basis.provider_updated_at = "2026-11-22T00:00:00Z".into();
        }
        WaitingChange::Closed => observation.remote_state = IssueRemoteState::Closed,
        WaitingChange::SelectionLabelRemoved => {
            observation
                .labels
                .items
                .retain(|label| label.name != "akra:ready");
            observation
                .label_actors
                .items
                .retain(|event| event.label.name != "akra:ready");
        }
        WaitingChange::Hold | WaitingChange::Unreadable => unreachable!(),
    }
}

fn authority_task_ids(services: &FlowServices) -> Vec<String> {
    services
        .planning
        .load_task_authority_snapshot(&services.workspace)
        .unwrap()
        .unwrap()
        .task_authority
        .tasks
        .into_iter()
        .map(|task| task.id)
        .collect()
}

fn assert_all_stages_denied(services: &FlowServices, task_id: &str, source_sha: &str) {
    for stage in PlanningWorkPermitStage::ALL {
        let decision = services
            .planning
            .check_work_order_permit_for_task(&services.workspace, task_id, stage, Some(source_sha))
            .unwrap();
        assert!(
            matches!(decision, PlanningWorkPermitDecision::Denied(_)),
            "the affected task retained {stage:?}: {decision:?}"
        );
    }
    // This unregistered task exercises only the legacy fallback, not another task's execution.
    // Actual unrelated delivery is covered by the existing bare-Git work_permits regressions.
    assert_eq!(
        services
            .planning
            .check_work_order_permit_for_task(
                &services.workspace,
                "unrelated-flow-task",
                PlanningWorkPermitStage::Dispatch,
                None,
            )
            .unwrap(),
        PlanningWorkPermitDecision::Legacy
    );
}

#[test]
fn issue_automation_flow_waiting_hold_stale_and_source_changes_fence_real_slot_admission() {
    for change in [
        WaitingChange::Hold,
        WaitingChange::StaleFields,
        WaitingChange::Closed,
        WaitingChange::SelectionLabelRemoved,
        WaitingChange::Unreadable,
    ] {
        let fixture = FlowFixture::new(&format!("issue-flow-waiting-{}", change.label()));
        if change == WaitingChange::SelectionLabelRemoved {
            fixture.require_selection_label("akra:ready").unwrap();
        }
        let services = fixture.services();
        services.collect_and_triage().unwrap();
        services.tick(10).unwrap();
        services.approve_start().unwrap();
        services.tick(20).unwrap();
        let work = services.work().unwrap().unwrap();
        let tasks = authority_task_ids(&services);
        assert_eq!(tasks.len(), 1);
        assert_eq!(work.task_ids, tasks);
        let task_id = &tasks[0];
        let source_sha = work.permit_source.as_deref().unwrap();
        assert!(matches!(
            services
                .planning
                .check_work_order_permit_for_task(
                    &services.workspace,
                    task_id,
                    PlanningWorkPermitStage::Dispatch,
                    Some(source_sha),
                )
                .unwrap(),
            PlanningWorkPermitDecision::Granted { .. }
        ));
        assert!(
            SqlitePlanningAuthorityAdapter::load_runtime_projections(&services.workspace)
                .unwrap()
                .slot_leases
                .is_empty()
        );

        change_waiting_issue(&fixture, &services, change);
        // Discovery failure still resyncs tracked IDs; the real collection transaction stores Unknown.
        let collected = services
            .collection
            .collect_once(&IssueCollectionCancellation::new(), 30)
            .unwrap();
        assert!(!collected.discarded);
        assert_eq!(
            collected.projection.state.status.last_failure.is_some(),
            change == WaitingChange::Unreadable,
            "{change:?}"
        );
        services.tick(40).unwrap();
        assert_all_stages_denied(&services, task_id, source_sha);
        drop(services);

        let restarted = fixture.services();
        restarted.tick(50).unwrap();
        assert_eq!(authority_task_ids(&restarted), tasks, "{change:?}");
        assert_eq!(
            restarted.work().unwrap().unwrap().order,
            work.order,
            "{change:?}"
        );
        assert_all_stages_denied(&restarted, task_id, source_sha);
        let admission = restarted.parallel.acquire_slot_lease(
            &restarted.workspace,
            sample_lease_request(task_id, "fenced issue", "blocked-worker", "blocked-issue"),
        );
        let error = admission.expect_err("the affected issue must not obtain a slot lease");
        assert!(
            error.contains(&format!("dispatch permission denied for task {task_id}")),
            "{change:?} must reach the durable dispatch gate, not another failure: {error}"
        );
        let runtime =
            SqlitePlanningAuthorityAdapter::load_runtime_projections(&restarted.workspace).unwrap();
        assert!(runtime.slot_leases.is_empty(), "{change:?}");
        assert!(runtime.distributor_queue_records.is_empty(), "{change:?}");
        assert!(fixture.operations().is_empty(), "{change:?}");
    }
}

fn remote_heads(fixture: &FlowFixture) -> String {
    run_command(
        "git",
        [
            "-C",
            fixture.workspace().as_str(),
            "ls-remote",
            "origin",
            "refs/heads/*",
        ],
        None,
    )
    .expect("the local bare remote branches should be readable")
}

#[test]
fn issue_automation_flow_takeover_at_source_push_preflight_fences_the_real_delivery() {
    let fixture = FlowFixture::new("issue-flow-takeover-prepush");
    let services = fixture.services();
    services.collect_and_triage().unwrap();
    services.tick(10).unwrap();
    services.approve_start().unwrap();
    services.tick(20).unwrap();
    let work = services.work().unwrap().unwrap();
    let worker = fixture.finish_worker(&services).unwrap();
    services.tick(30).unwrap();
    fixture.approve_pull_request(&worker.source_sha);
    assert!(matches!(
        services
            .planning
            .check_work_order_permit_for_task(
                &services.workspace,
                &worker.task_id,
                PlanningWorkPermitStage::Push,
                Some(&worker.source_sha),
            )
            .unwrap(),
        PlanningWorkPermitDecision::Granted { .. }
    ));
    let before = remote_heads(&fixture);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let interventions = services.interventions.clone();
    let issue = fixture.issue.clone();
    let expected_work = work.order.work_order_id.clone();
    fixture.github.set_boundary_hook(
        FakeGithubAutomationBoundary::SourcePushPreflight,
        move || {
            let receipt = interventions
                .intervene(
                    IssueInterventionRequest {
                        command_id: "takeover-during-source-push-preflight".into(),
                        issue: issue.clone(),
                        action: IssueInterventionAction::TakeOver,
                    },
                    &context(),
                )
                .unwrap();
            assert!(receipt.changed);
            assert_eq!(receipt.issue, issue);
            assert_eq!(
                receipt.work_order_id.as_deref(),
                Some(expected_work.as_str())
            );
            seen.fetch_add(1, Ordering::SeqCst);
        },
    );
    let notices = services.process_delivery().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "{notices:?}");
    assert!(
        fixture.operations().is_empty(),
        "takeover must prevent source push and PR creation: {notices:?}"
    );
    assert_eq!(remote_heads(&fixture), before);
    assert_all_stages_denied(&services, &worker.task_id, &worker.source_sha);
    let control = services
        .execution
        .control(&services.workspace, &work.order.work_order_id)
        .unwrap();
    assert!(control.taken_over);
    assert!(!control.allows_automation());
    let queue = SqlitePlanningAuthorityAdapter::load_runtime_projections(&services.workspace)
        .unwrap()
        .distributor_queue_records;
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].task_id, worker.task_id);
    assert_eq!(queue[0].queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        queue[0]
            .integration_note
            .contains(&format!("permit_stale: push / task {}", worker.task_id)),
        "takeover must block at the current Push permit gate: {}",
        queue[0].integration_note
    );
    drop(services);

    let restarted = fixture.services();
    restarted.tick(40).unwrap();
    restarted.process_delivery().unwrap();
    assert_eq!(authority_task_ids(&restarted), vec![worker.task_id.clone()]);
    assert_all_stages_denied(&restarted, &worker.task_id, &worker.source_sha);
    assert!(
        restarted
            .execution
            .control(&restarted.workspace, &work.order.work_order_id)
            .unwrap()
            .taken_over
    );
    assert_eq!(remote_heads(&fixture), before);
    assert!(fixture.operations().is_empty());
    assert_eq!(fixture.integration_push_count(), 0);
    assert!(Path::new(&worker.lease.worktree_path).is_dir());
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                worker.lease.worktree_path.as_str(),
                "rev-parse",
                "HEAD"
            ],
            None
        )
        .as_deref(),
        Some(worker.source_sha.as_str()),
        "operator-owned worker output must remain intact"
    );
}
