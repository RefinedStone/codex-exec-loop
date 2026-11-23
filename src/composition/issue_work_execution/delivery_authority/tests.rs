use super::*;
use crate::domain::issue_automation::decision::IssueDecisionDeliveryContext;
use crate::domain::issue_work::IssueFieldHash;

fn request() -> IssueWorkPermitRequest {
    let approval = IssueDecisionBasis {
        authorization_epoch: 4,
        policy_version: "a".repeat(64),
        field_hash: Some(IssueFieldHash::of_fields("제목", "완료 기준")),
        work_order_id: Some("issue-work-auto".into()),
        gate_revision: 3,
        delivery: None,
    };
    let review = IssueDecisionBasis {
        gate_revision: 8,
        delivery: Some(IssueDecisionDeliveryContext {
            base_commit_sha: "b".repeat(40),
            source_commit_sha: "c".repeat(40),
        }),
        ..approval.clone()
    };
    let merge_basis = IssueDecisionBasis {
        gate_revision: 6,
        ..review.clone()
    };
    IssueWorkPermitRequest {
        authorization_epoch: approval.authorization_epoch,
        work_order_id: "issue-work-auto".into(),
        approval,
        source_sha: "c".repeat(40),
        gate_revision: review.gate_revision,
        delivery: true,
        delivery_review: Some(review),
        delivery_authority: Some(IssueWorkDeliveryAuthority::Auto {
            merge_basis,
            source_repository: "sample/widgets.rs".into(),
        }),
    }
}

fn result(request: &IssueWorkPermitRequest) -> Result<PlanningWorkDeliveryAuthority> {
    validate(request, request.delivery_review.as_ref().unwrap())
}

#[test]
fn explicit_human_and_auto_authorities_remain_distinct_and_merge_revision_is_preserved() {
    let mut request = request();
    assert_eq!(
        result(&request).unwrap(),
        PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: 6,
            source_repository: "sample/widgets.rs".into(),
        }
    );
    request.delivery_authority = Some(IssueWorkDeliveryAuthority::HumanReview);
    assert_eq!(
        result(&request).unwrap(),
        PlanningWorkDeliveryAuthority::HumanReview
    );
    request.delivery_authority = None;
    assert!(
        result(&request).is_err(),
        "a missing authority cannot infer Auto from the policy"
    );
}

#[test]
fn another_merge_basis_cannot_authorize_the_reviewed_delivery() {
    for condition in 0..8 {
        let mut request = request();
        let Some(IssueWorkDeliveryAuthority::Auto { merge_basis, .. }) =
            request.delivery_authority.as_mut()
        else {
            unreachable!();
        };
        match condition {
            0 => merge_basis.authorization_epoch += 1,
            1 => merge_basis.work_order_id = Some("another-work".into()),
            2 => merge_basis.policy_version = "d".repeat(64),
            3 => merge_basis.field_hash = Some(IssueFieldHash::of_fields("다른 제목", "기준")),
            4 => merge_basis.delivery.as_mut().unwrap().base_commit_sha = "d".repeat(40),
            5 => merge_basis.delivery.as_mut().unwrap().source_commit_sha = "d".repeat(40),
            6 => merge_basis.delivery = None,
            7 => merge_basis.gate_revision = 0,
            _ => unreachable!(),
        }
        assert!(
            result(&request).is_err(),
            "changed merge basis condition {condition}"
        );
    }
}

#[test]
fn auto_mapping_rejects_dispatch_or_a_different_frozen_source() {
    let mut request = request();
    request.delivery = false;
    assert!(result(&request).is_err());
    request.delivery = true;
    request.source_sha = "d".repeat(40);
    assert!(result(&request).is_err());
}

#[test]
fn provider_coordinate_must_be_canonical_before_it_becomes_permit_authority() {
    for coordinate in [
        "Sample/widgets.rs",
        "sample/widgets/extra",
        "https://github.com/sample/widgets",
        "sample/..",
        "-sample/widgets",
        "sample/widgets\n",
        "sample/%2fwidgets",
        "",
    ] {
        let mut request = request();
        let Some(IssueWorkDeliveryAuthority::Auto {
            source_repository, ..
        }) = request.delivery_authority.as_mut()
        else {
            unreachable!();
        };
        *source_repository = coordinate.into();
        assert!(
            result(&request).is_err(),
            "noncanonical coordinate {coordinate:?}"
        );
    }
}
