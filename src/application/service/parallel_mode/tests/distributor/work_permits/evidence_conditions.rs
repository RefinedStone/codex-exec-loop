use super::*;
use crate::application::port::inbound::task_delivery_evidence_port::{
    TaskDeliveryEvidenceDecision, TaskDeliveryEvidenceGatePort, TaskDeliveryEvidenceIntegration,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::task_delivery_evidence_repository_port::TaskDeliveryEvidenceOperation;
use crate::domain::planning::delivery_evidence::{
    TaskEvidenceDeliveryMode, TaskEvidenceDeliveryStage,
};
use std::sync::atomic::{AtomicBool, Ordering};

struct PreflightRaceGate {
    inner: Arc<dyn TaskDeliveryEvidenceGatePort>,
    workspace: String,
    next_basis: PlanningWorkPermitBasis,
    remove_preflight: bool,
    changed: AtomicBool,
}

impl TaskDeliveryEvidenceGatePort for PreflightRaceGate {
    fn evaluate(
        &self,
        workspace: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
        stage: TaskEvidenceDeliveryStage,
        integration: Option<&TaskDeliveryEvidenceIntegration>,
        operation: &TaskDeliveryEvidenceOperation,
    ) -> Result<TaskDeliveryEvidenceDecision, String> {
        let mut integration = integration.cloned();
        if stage == TaskEvidenceDeliveryStage::Integrate
            && matches!(operation, TaskDeliveryEvidenceOperation::Effect { .. })
            && !self.changed.swap(true, Ordering::SeqCst)
        {
            let proof = integration.as_mut().unwrap();
            assert_eq!(proof.preflight_requires_human_review, Some(false));
            if self.remove_preflight {
                proof.preflight_requires_human_review = None;
            } else {
                set_basis_and_grant(
                    &self.workspace,
                    &self.next_basis,
                    &[PlanningWorkPermitStage::Integrate],
                    true,
                );
            }
        }
        self.inner
            .evaluate(workspace, record, stage, integration.as_ref(), operation)
    }
}

#[test]
fn a_new_human_review_requirement_or_missing_preflight_cannot_reuse_autonomous_pr_readiness() {
    for remove_preflight in [false, true] {
        let fixture = WorkPermitDeliveryFixture::new(
            "evidence-review-condition-race",
            FakeGithubAutomationPort::with_review_decision("REVIEW_REQUIRED"),
        );
        fixture.grant(
            &[
                PlanningWorkPermitStage::Push,
                PlanningWorkPermitStage::PullRequest,
                PlanningWorkPermitStage::Integrate,
            ],
            false,
        );
        let workspace = fixture.repo.workspace_dir();
        let authority = SqlitePlanningAuthorityAdapter::new();
        let identity = PlanningAuthorityPort::resolve_authority_location(&authority, &workspace)
            .unwrap()
            .repository_identity;
        let gate = Arc::new(PreflightRaceGate {
            inner: crate::composition::task_delivery_evidence::build(
                &workspace,
                &identity,
                Ok(TaskEvidenceDeliveryMode::Observe),
            ),
            workspace: workspace.clone(),
            next_basis: PlanningWorkPermitBasis {
                gate_revision: fixture.basis.gate_revision + 1,
                ..fixture.basis.clone()
            },
            remove_preflight,
            changed: AtomicBool::new(false),
        });
        let service =
            test_parallel_mode_service_with_autonomous_github(Arc::new(fixture.github.clone()))
                .with_delivery_evidence(gate.clone());
        let before = remote_integration_head(&fixture.repo);

        let error = service.process_distributor_queue(&workspace).unwrap_err();

        assert!(gate.changed.load(Ordering::SeqCst));
        assert!(
            error.contains("review requirements changed after the final pull request check"),
            "{error}"
        );
        assert_eq!(remote_integration_head(&fixture.repo), before);
        fixture.assert_no_integration_push();
        assert!(
            fixture
                .operations()
                .iter()
                .any(|operation| operation.starts_with("ensure-pr:"))
        );
    }
}
