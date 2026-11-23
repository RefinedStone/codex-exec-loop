//! The real composition submit boundary with SQLite; no worker, provider call or model turn.
use super::*;

use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::automation::GithubAutomationAdapter;
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkExecutionPort, IssueWorkPermitRequest,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitBlockReason, PlanningWorkPermitDecision, evaluate_work_order_permit,
};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter;
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::decision::IssueDecisionBasis;
use crate::domain::issue_work::{
    IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference, IssueSharedCopy,
};

fn execution_adapter() -> AkraIssueWorkExecutionAdapter {
    let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    // Non-closing link validation returns before Git/GitHub I/O. These adapters are
    // otherwise unused by submit and permit registration.
    let parallel = ParallelModeService::new(
        authority.clone(),
        Arc::new(GithubAutomationAdapter::new()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    AkraIssueWorkExecutionAdapter::new(authority.clone(), authority, parallel)
}

#[test]
fn legacy_publication_reference_real_submit_does_not_upgrade_unreviewed_delivery() {
    let workspace = legacy_workspace("legacy-reference-real-submit", "issue_automation");
    let original = stored(&workspace, ORDER);
    let before = rows_except_links(&workspace);
    let authority_before = snapshot(&workspace);
    let shared = IssueSharedCopy {
        title: "Synthetic approved issue".into(),
        body: "Synthetic shared handoff for legacy resubmission.".into(),
    };
    let order = IssueWorkOrder {
        work_order_id: ORDER.into(),
        issue: IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_legacy".into(),
            issue_id: "I_legacy".into(),
            project_scope: IssueProjectScope {
                workspace_dir: workspace.clone(),
            },
        },
        field_hash: IssueFieldHash::of_fields(&shared.title, &shared.body),
        title: shared.title.clone(),
        handoff: shared,
        acceptance: vec!["Keep the original work and its authority.".into()],
        scope: workspace.clone(),
        branch_hint: None,
        link_reference: Some(URL.into()),
        provider_link: false,
        profile: None,
        priority: 50,
        attempt: 1,
    };
    let mut serialized = serde_json::to_value(&order).unwrap();
    serialized.as_object_mut().unwrap().remove("providerLink");
    let legacy_order: IssueWorkOrder = serde_json::from_value(serialized).unwrap();
    assert!(!legacy_order.provider_link);
    assert_eq!(legacy_order.link_reference, order.link_reference);

    let adapter = execution_adapter();
    assert_eq!(
        adapter.submit(&workspace, &legacy_order).unwrap(),
        original.task_ids
    );
    let recovered = stored(&workspace, ORDER);
    assert_eq!(
        recovered,
        PlanningWorkOrderRecord {
            link_reference: Some(reference()),
            ..original
        }
    );
    assert_eq!(rows_except_links(&workspace), before);
    assert_eq!(snapshot(&workspace), authority_before);

    // Recovery is not approval: the retained source-a grant stays stale against source-b.
    let basis = SqlitePlanningAuthorityAdapter::new()
        .load_work_order_permit_basis(&workspace, ORDER)
        .unwrap()
        .unwrap();
    assert_eq!(
        evaluate_work_order_permit(
            &recovered,
            Some(&basis),
            PlanningWorkPermitStage::Push,
            Some(&basis.source_sha),
        ),
        PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Stale)
    );

    // Link recovery cannot invent either diff review or an owned frozen queue range.
    let frozen_source = "c".repeat(40);
    let approval = IssueDecisionBasis {
        authorization_epoch: 0,
        policy_version: "d".repeat(64),
        field_hash: Some(order.field_hash),
        work_order_id: Some(ORDER.into()),
        gate_revision: 3,
        delivery: None,
    };
    let mut review = approval.clone();
    review.delivery = Some(
        crate::domain::issue_automation::decision::IssueDecisionDeliveryContext {
            base_commit_sha: "b".repeat(40),
            source_commit_sha: frozen_source.clone(),
        },
    );
    for delivery_review in [None, Some(review)] {
        assert!(
            adapter
                .grant(
                    &workspace,
                    &IssueWorkPermitRequest {
                        authorization_epoch: 0,
                        work_order_id: ORDER.into(),
                        approval: approval.clone(),
                        source_sha: frozen_source.clone(),
                        gate_revision: 3,
                        delivery: true,
                        delivery_review,
                    }
                )
                .is_err()
        );
        assert_eq!(rows_except_links(&workspace), before);
        assert_eq!(stored(&workspace, ORDER), recovered);
    }
    assert_eq!(snapshot(&workspace), authority_before);
    assert_eq!(
        execution_adapter()
            .submit(&workspace, &legacy_order)
            .unwrap(),
        recovered.task_ids
    );
    assert_eq!(rows_except_links(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_links"), 1);
}
