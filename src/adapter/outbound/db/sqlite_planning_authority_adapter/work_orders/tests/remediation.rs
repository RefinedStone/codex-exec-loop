use std::sync::Arc;

use super::*;
use crate::adapter::outbound::filesystem::planning_workspace::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort, PlanningWorkPermitDecision,
    PlanningWorkPermitStage,
};
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::port::outbound::pr_validation_remediation_port::{
    PrValidationRemediationKey, PrValidationRemediationPort, PrValidationRemediationRequest,
};
use crate::application::service::parallel_mode::PlanningQueuePrValidationRemediationPort;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationService,
};
use crate::domain::parallel_mode::{
    PrValidationCommitSha, PrValidationFindingKey, PrValidationFindingSource,
    PrValidationRecordKey, PrValidationTarget,
};
use crate::domain::planning::PriorityQueueService;

fn planning(adapter: Arc<SqlitePlanningAuthorityAdapter>) -> PlanningServices {
    PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        adapter.clone(),
        adapter.clone(),
        Arc::new(NoopPlanningWorkerPort),
    )
    .with_work_order_repository(adapter)
}

fn request(key: &str) -> PrValidationRemediationRequest {
    PrValidationRemediationRequest {
        idempotency_key: PrValidationRemediationKey::new(format!("remediate-{key}")).unwrap(),
        validation_record_key: PrValidationRecordKey::new(key).unwrap(),
        target: PrValidationTarget::new("synthetic/repository", 1).unwrap(),
        target_sha: PrValidationCommitSha::new("a".repeat(40)).unwrap(),
        finding_key: PrValidationFindingKey::new(
            PrValidationFindingSource::new("ci").unwrap(),
            "finding",
        )
        .unwrap(),
        summary: "Repair the failing test".into(),
    }
}

fn setup(
    label: &str,
) -> (
    String,
    Arc<SqlitePlanningAuthorityAdapter>,
    PlanningServices,
) {
    let workspace = workspace(label);
    seed(&workspace, vec![task("legacy-source")]);
    register_next(&workspace, "order-a", "original");
    let adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    adapter
        .link_work_order_validation_record(&workspace, "owned-record", "original")
        .unwrap();
    adapter
        .link_work_order_validation_record(&workspace, "legacy-record", "legacy-source")
        .unwrap();
    let planning = planning(adapter.clone());
    (workspace, adapter, planning)
}

#[test]
fn remediation_creation_already_has_lineage_and_no_execution_permission() {
    let (workspace, adapter, planning) = setup("remediation-production-boundary");
    let port = PlanningQueuePrValidationRemediationPort::new(&planning.queue, &workspace);
    let first = port.request_remediation(&request("owned-record")).unwrap();
    assert_eq!(
        port.request_remediation(&request("owned-record")).unwrap(),
        first
    );
    assert_eq!(
        stored(&workspace, "order-a").task_ids,
        ["original", first.as_str()]
    );
    assert_eq!(snapshot(&workspace).task_authority.tasks.len(), 3);
    assert_ne!(
        PlanningAuthorityPort::check_work_order_permit_for_task(
            adapter.as_ref(),
            &workspace,
            first.as_str(),
            PlanningWorkPermitStage::Dispatch,
            None,
        )
        .unwrap(),
        PlanningWorkPermitDecision::Legacy
    );
    adapter
        .apply_work_order_control(
            &workspace,
            "order-a",
            "cancel",
            PlanningWorkControlAction::Cancel,
        )
        .unwrap();
    assert_eq!(
        snapshot(&workspace)
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == first.as_str())
            .unwrap()
            .status,
        TaskStatus::Cancelled
    );
}

#[test]
fn remediation_preserves_legacy_but_missing_correlation_is_rejected() {
    let (workspace, adapter, planning) = setup("remediation-legacy");
    let port = PlanningQueuePrValidationRemediationPort::new(&planning.queue, &workspace);
    let first = port.request_remediation(&request("legacy-record")).unwrap();
    assert_eq!(
        port.request_remediation(&request("legacy-record")).unwrap(),
        first
    );
    assert!(
        adapter
            .load_work_order_for_task(&workspace, first.as_str())
            .unwrap()
            .is_none()
    );
    let before = snapshot(&workspace);
    assert!(
        port.request_remediation(&request("unknown-record"))
            .is_err()
    );
    assert_eq!(snapshot(&workspace), before);
}

#[test]
fn historical_idle_remediation_is_promoted_once_without_a_duplicate_task() {
    let (workspace, adapter, planning) = setup("remediation-historical");
    let request = request("owned-record");
    let previous = PlanningTaskMutationService::new(adapter.clone(), PriorityQueueService::new())
        .admit_system_task_once(
            &workspace,
            request.idempotency_key.as_str(),
            PlanningTaskCreateInput {
                acceptance: None,
                direction_id: None,
                direction_relation_note: Some("legacy remediation".into()),
                title: "Existing remediation".into(),
                description: Some("already admitted".into()),
                status: Some(TaskStatus::Ready),
                base_priority: None,
                dynamic_priority_delta: None,
                priority_reason: None,
                depends_on: vec![],
                blocked_by: vec![],
            },
        )
        .unwrap();
    let before = snapshot(&workspace).task_authority.tasks.len();
    let port = PlanningQueuePrValidationRemediationPort::new(&planning.queue, &workspace);
    let promoted = port.request_remediation(&request).unwrap();
    assert_eq!(promoted.as_str(), previous.task_id);
    assert_eq!(snapshot(&workspace).task_authority.tasks.len(), before);
    assert_eq!(
        stored(&workspace, "order-a").task_ids,
        ["original", previous.task_id.as_str()]
    );
    assert_eq!(port.request_remediation(&request).unwrap(), promoted);
}

#[test]
fn cancelled_taken_over_or_exhausted_work_never_creates_remediation() {
    for scenario in 0..3 {
        let (workspace, adapter, planning) = setup(&format!("remediation-blocked-{scenario}"));
        match scenario {
            0 => {
                adapter
                    .apply_work_order_control(
                        &workspace,
                        "order-a",
                        "cancel",
                        PlanningWorkControlAction::Cancel,
                    )
                    .unwrap();
            }
            1 => {
                adapter
                    .apply_work_order_control(
                        &workspace,
                        "order-a",
                        "manual",
                        PlanningWorkControlAction::TakeOver,
                    )
                    .unwrap();
            }
            _ => adapter
                .set_work_order_execution_limit(&workspace, "order-a", 0)
                .unwrap(),
        }
        let before = snapshot(&workspace);
        let port = PlanningQueuePrValidationRemediationPort::new(&planning.queue, &workspace);
        assert!(port.request_remediation(&request("owned-record")).is_err());
        assert_eq!(snapshot(&workspace), before);
        assert_eq!(stored(&workspace, "order-a").task_ids, ["original"]);
    }
}
