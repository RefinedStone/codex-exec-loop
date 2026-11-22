//! #2555: real application flow with local Git/SQLite and isolated remote doubles.
//! No provider request or model turn is made by these tests.
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderRepositoryPort;
use crate::domain::issue_automation::collection::IssueExternalState;
use crate::domain::issue_automation::execution::{IssueExecutionDetail, IssueExecutionState};
use crate::domain::issue_automation::feedback::IssueFeedbackOperation;
use std::sync::atomic::Ordering;

mod control_boundaries;
mod feedback_recovery;
mod registration_recovery;
mod support;

use support::{FlowFixture, ISSUE_URL, PULL_REQUEST_URL};

#[test]
fn propose_flow_collects_approves_creates_pr_and_returns_one_status_comment() {
    let fixture = FlowFixture::new("issue-flow-approved");
    let services = fixture.services();
    let original_integration = fixture.remote_integration_head();

    services.collect_and_triage().unwrap();
    services.tick(10).unwrap();
    assert_eq!(fixture.source.discovery_calls.load(Ordering::SeqCst), 1);
    assert!(services.task_ids().unwrap().is_empty());
    assert!(fixture.operations().is_empty());

    services.approve_start().unwrap();
    services.tick(20).unwrap();
    let work = services
        .work()
        .unwrap()
        .expect("approval should create durable issue work");
    let task_ids = services.task_ids().unwrap();
    assert_eq!(task_ids.len(), 1);
    assert_eq!(work.task_ids, task_ids);
    assert_eq!(work.order.link_reference.as_deref(), Some(ISSUE_URL));
    assert!(!work.order.provider_link);
    let tasks = services
        .planning
        .load_task_authority_snapshot(&services.workspace)
        .unwrap()
        .unwrap()
        .task_authority
        .tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, task_ids[0]);
    assert_eq!(fixture.provider.create_count(), 1);

    let worker = fixture.finish_worker(&services).unwrap();
    assert_eq!(worker.task_id, task_ids[0]);
    assert_eq!(worker.lease.task_id, task_ids[0]);
    services.tick(30).unwrap();
    assert!(services.work().unwrap().unwrap().delivery_granted);
    let queued = SqlitePlanningAuthorityAdapter::load_runtime_projections(&services.workspace)
        .unwrap()
        .distributor_queue_records;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].task_id, task_ids[0]);
    let planning_work = services
        .planning
        .load_work_order_for_task(&services.workspace, &queued[0].task_id)
        .unwrap()
        .unwrap();
    assert_eq!(planning_work.work_order_id, work.order.work_order_id);
    assert_eq!(
        queued[0]
            .link_reference
            .as_ref()
            .map(|link| link.url.as_str()),
        Some(ISSUE_URL)
    );

    // Publish the real queued result with no review. Creation of the PR must not
    // move the integration ref while the current source has no human approval.
    let waiting_notices = services.process_delivery().unwrap();
    assert_eq!(fixture.integration_push_count(), 0, "{waiting_notices:?}");
    assert_eq!(fixture.remote_integration_head(), original_integration);
    let waiting = SqlitePlanningAuthorityAdapter::load_runtime_projections(&services.workspace)
        .unwrap()
        .distributor_queue_records;
    assert_eq!(waiting.len(), 1);
    let waiting = &waiting[0];
    assert_eq!(waiting.queue_item_id, queued[0].queue_item_id);
    assert_eq!(waiting.task_id, task_ids[0]);
    assert_eq!(waiting.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        waiting.integration_note.contains("APPROVED"),
        "{waiting_notices:?}"
    );
    assert_eq!(waiting.pull_request_number, Some(77));
    assert_eq!(waiting.pull_request_url.as_deref(), Some(PULL_REQUEST_URL));
    assert_eq!(waiting.source_commit_sha, worker.source_sha);
    assert_eq!(waiting.retry_attempts, 0);
    assert_eq!(fixture.github.pull_request_bodies.lock().unwrap().len(), 1);

    let reviewed_head = fixture
        .github
        .remote_branch_head(
            &services.workspace,
            DEFAULT_PUSH_REMOTE_NAME,
            &worker.lease.branch_name,
        )
        .unwrap()
        .expect("the existing PR source must be present on the local bare remote");
    assert_eq!(reviewed_head, worker.source_sha);
    fixture.approve_pull_request(&reviewed_head);
    // The public queue API performs the normal retryable-review-block recovery.
    // No queue state, retry timer, permit or task record is rewritten by the test.
    let notices = services.process_delivery().unwrap();
    assert_eq!(fixture.integration_push_count(), 1, "{notices:?}");
    assert_ne!(fixture.remote_integration_head(), original_integration);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture.workspace().as_str(),
                "show",
                &format!("refs/remotes/origin/{POOL_BASELINE_BRANCH}:flow-result.txt"),
            ],
            None
        )
        .as_deref(),
        Some("approved result"),
    );
    let bodies = fixture.github.pull_request_bodies.lock().unwrap().clone();
    assert_eq!(
        bodies.len(),
        2,
        "the resumed delivery ensures the same existing PR again"
    );
    assert_eq!(
        bodies[0], bodies[1],
        "retry must keep the frozen issue reference"
    );
    assert!(bodies[0].contains(ISSUE_URL));
    assert!(!bodies[0].contains("Closes "));
    let delivered = SqlitePlanningAuthorityAdapter::load_runtime_projections(&services.workspace)
        .unwrap()
        .distributor_queue_records;
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].queue_item_id, waiting.queue_item_id);
    assert_eq!(
        delivered[0].pull_request_number,
        waiting.pull_request_number
    );
    assert_eq!(delivered[0].pull_request_url, waiting.pull_request_url);
    assert_eq!(delivered[0].retry_attempts, 1);
    assert_eq!(delivered[0].queue_state, ParallelModeQueueItemState::Done);

    services.tick(40).unwrap();
    let execution = services
        .store
        .get_issue_execution(&services.workspace, &fixture.issue)
        .unwrap()
        .expect("real execution observations should be persisted");
    assert_eq!(execution.progress.state, IssueExecutionState::Done);
    assert_eq!(execution.progress.attempts.execution, 1);
    let integration = execution
        .progress
        .integration
        .as_ref()
        .expect("the pushed integration commit should be independently recorded");
    assert_eq!(integration.source_sha, worker.source_sha);
    assert_eq!(
        integration.integration_sha,
        fixture.remote_integration_head()
    );
    assert_eq!(
        integration.pull_request_url.as_deref(),
        Some(PULL_REQUEST_URL)
    );

    let writes_before_query = fixture.provider.writes().len();
    let readbacks_before_query = fixture.provider.reconcile_count();
    let source_reads_before_query = fixture.source.resync_calls.load(Ordering::SeqCst);
    let detail = services.collection.issue(&fixture.issue).unwrap();
    let Some(IssueExecutionDetail::Ready(projected)) = detail.execution else {
        panic!("the actual stored execution should reach the detail query");
    };
    assert_eq!(projected.work_order_id, work.order.work_order_id);
    assert_eq!(projected.state, IssueExecutionState::Done);
    assert_eq!(
        projected.execution_attempts,
        execution.progress.attempts.execution
    );
    assert_eq!(
        projected.verification_attempts,
        execution.progress.attempts.verification
    );
    assert!(projected.execution_attempt_limit.is_some());
    let feedback = detail.feedback.unwrap();
    assert_eq!(fixture.provider.writes().len(), writes_before_query);
    assert_eq!(fixture.provider.reconcile_count(), readbacks_before_query);
    assert_eq!(
        fixture.source.resync_calls.load(Ordering::SeqCst),
        source_reads_before_query
    );
    assert_eq!(feedback.external_state, IssueExternalState::Open);
    assert_eq!(feedback.integration.as_ref(), Some(integration));
    assert!(feedback.reason.is_none());
    assert_eq!(fixture.provider.comment_count(), 1);
    assert_eq!(fixture.provider.create_count(), 1);
    assert!(fixture.provider.writes().iter().all(|intent| matches!(
        intent.operation,
        IssueFeedbackOperation::CreateComment { .. } | IssueFeedbackOperation::UpdateComment { .. }
    )));
    let comment = fixture.provider.comments().pop().unwrap();
    assert_eq!(
        feedback.comment_id.as_deref(),
        Some(comment.comment_id.as_str())
    );
    assert!(comment.body.contains("상태: 통합 확인"));
    assert!(comment.body.contains("외부 이슈의 닫힘은 별도 상태입니다."));
    assert!(
        services
            .feedback
            .unresolved(&fixture.issue)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        feedback.publication_attempts,
        fixture.provider.writes().len() as u64
    );

    // Reconstruct every service against the same durable stores. Re-observation
    // must retain the task, integration and owned comment without another send.
    let writes = fixture.provider.writes().len();
    drop(services);
    let restarted = fixture.services();
    restarted.tick(50).unwrap();
    assert_eq!(restarted.task_ids().unwrap(), task_ids);
    assert_eq!(fixture.integration_push_count(), 1);
    assert_eq!(fixture.provider.writes().len(), writes);
    assert_eq!(fixture.provider.comment_count(), 1);
}

#[test]
fn propose_flow_process_opt_in_never_replaces_start_or_pull_request_human_approval() {
    let fixture = FlowFixture::new("issue-flow-human-gates");
    let services = fixture.services();
    let original_integration = fixture.remote_integration_head();
    services.collect_and_triage().unwrap();

    for now in [10, 20, 30] {
        services.tick(now).unwrap();
        services.process_delivery().unwrap();
    }
    assert!(services.task_ids().unwrap().is_empty());
    assert!(
        services
            .planning
            .load_task_authority_snapshot(&services.workspace)
            .unwrap()
            .unwrap()
            .task_authority
            .tasks
            .is_empty()
    );
    assert!(fixture.operations().is_empty());
    assert_eq!(fixture.integration_push_count(), 0);
    assert_eq!(fixture.remote_integration_head(), original_integration);

    // A start approval permits implementation and PR delivery. It still does not
    // supply the independent review required for integration.
    services.approve_start().unwrap();
    services.tick(40).unwrap();
    let worker = fixture.finish_worker(&services).unwrap();
    services.tick(50).unwrap();
    let notices = services.process_delivery().unwrap();
    assert_eq!(services.task_ids().unwrap(), vec![worker.task_id.clone()]);
    assert!(
        fixture
            .operations()
            .iter()
            .any(|operation| operation.starts_with("ensure-pr:"))
    );
    assert_eq!(fixture.github.pull_request_bodies.lock().unwrap().len(), 1);
    assert_eq!(fixture.integration_push_count(), 0, "{notices:?}");
    assert_eq!(fixture.remote_integration_head(), original_integration);
    let queue = SqlitePlanningAuthorityAdapter::load_runtime_projections(&services.workspace)
        .unwrap()
        .distributor_queue_records;
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].task_id, worker.task_id);
    assert_eq!(queue[0].queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        queue[0].integration_note.contains("APPROVED"),
        "{notices:?}"
    );

    services.tick(60).unwrap();
    let execution = services
        .store
        .get_issue_execution(&services.workspace, &fixture.issue)
        .unwrap()
        .unwrap();
    assert!(execution.progress.integration.is_none());
    assert_ne!(execution.progress.state, IssueExecutionState::Done);
    assert_eq!(fixture.provider.create_count(), 1);
    assert_eq!(fixture.provider.comment_count(), 1);
}
