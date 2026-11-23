use super::autonomy::{autonomy, confirm};
use super::delivery::{answer as answer_delivery, freeze};
use super::*;
use crate::application::port::outbound::issue_work_execution_port::IssueWorkDeliveryAuthority;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::autonomy::IssueAutonomyLevel;
use crate::domain::issue_automation::delivery::IssueDeliveryRiskReason;
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::triage::IssueTriageReasonCode;

const SOURCE_REPOSITORY: &str = "refinedstone/codex-exec-loop";

fn auto_policy() -> String {
    AUTO_POLICY
        .replace("mode = \"propose\"", "mode = \"auto\"")
        .replace("autonomy_cap = \"propose\"", "autonomy_cap = \"auto\"")
}

fn source_harness(visibility: IssueRepositoryVisibility, coordinate: Option<&str>) -> Harness {
    Harness::with_observation(|observation| {
        observation.repository_visibility = visibility;
        observation.repository_name_with_owner = coordinate.map(str::to_owned);
    })
}

fn select_auto(h: &Harness, policy: String) {
    *h.policy.text.lock().unwrap() = policy;
    let service = autonomy(h);
    let proof = confirm(&service, "auto-delivery-consent", IssueAutonomyLevel::Auto);
    service
        .set(
            "auto-delivery-consent".into(),
            IssueAutonomyLevel::Auto,
            Some(&proof),
        )
        .unwrap();
}

fn policy_started(visibility: IssueRepositoryVisibility, policy: String) -> Harness {
    let h = source_harness(visibility, Some(SOURCE_REPOSITORY));
    select_auto(&h, policy);
    h.tick(true).unwrap();
    let work = h.work().unwrap();
    assert_eq!(work.start_authority, IssueWorkStartAuthority::Policy);
    assert_eq!(work.decision_id, None);
    assert!(page(&h.triage).items[0].open_decisions.is_empty());
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.registrations, 1);
    assert_eq!(state.grants.len(), 1);
    let dispatch = &state.grants[0];
    assert!(!dispatch.delivery);
    assert_eq!(dispatch.delivery_review, None);
    assert_eq!(dispatch.delivery_authority, None);
    assert_eq!(dispatch.source_sha, "a".repeat(40));
    drop(state);
    h
}

fn delivery_grants(h: &Harness) -> Vec<IssueWorkPermitRequest> {
    h.execution
        .state
        .lock()
        .unwrap()
        .grants
        .iter()
        .filter(|grant| grant.delivery)
        .cloned()
        .collect()
}

fn assert_human_delivery(h: &Harness) {
    let grants = delivery_grants(h);
    assert_eq!(grants.len(), 1);
    assert_eq!(
        grants[0].delivery_authority,
        Some(IssueWorkDeliveryAuthority::HumanReview)
    );
    assert!(grants[0].delivery_review.is_some());
    assert!(h.work().unwrap().delivery_granted);
}

#[test]
fn private_and_internal_auto_delivery_bind_current_merge_to_the_frozen_range() {
    for visibility in [
        IssueRepositoryVisibility::Private,
        IssueRepositoryVisibility::Internal,
    ] {
        let h = policy_started(visibility, auto_policy());
        let start = h.work().unwrap().approved_basis;
        freeze(&h, 'b', "src/feature.rs");
        h.tick(true).unwrap();

        let work = h.work().unwrap();
        let grants = delivery_grants(&h);
        assert_eq!(grants.len(), 1);
        let grant = &grants[0];
        let review = work.delivery_assessment.as_ref().unwrap();
        assert!(work.delivery_granted);
        assert_eq!(grant.work_order_id, work.order.work_order_id);
        assert_eq!(grant.approval, start);
        assert_eq!(work.approved_basis, start);
        assert_eq!(grant.source_sha, "b".repeat(40));
        assert_eq!(grant.gate_revision, review.basis.gate_revision);
        assert_eq!(grant.delivery_review.as_ref(), Some(&review.basis));
        assert_eq!(
            review.basis.delivery,
            Some(IssueDecisionDeliveryContext {
                base_commit_sha: "a".repeat(40),
                source_commit_sha: "b".repeat(40),
            })
        );
        assert!(!review.verdict.requires_human());

        let Some(IssueWorkDeliveryAuthority::Auto {
            merge_basis,
            source_repository,
        }) = &grant.delivery_authority
        else {
            panic!("the current Auto policy must issue an explicit Auto authority");
        };
        let current = page(&h.triage);
        let assessment = &current.items[0].triage.as_ref().unwrap().assessment;
        assert_eq!(assessment.effective_mode(), IssueAutomationAutonomy::Auto);
        let merge = assessment
            .gates()
            .iter()
            .find(|gate| gate.gate == IssueAutomationGate::Merge)
            .unwrap();
        assert!(merge.enabled);
        assert_eq!(merge.action, IssueAutomationGateAction::Auto);
        assert_eq!(merge_basis.gate_revision, merge.gate_revision);
        assert_eq!(
            merge_basis.policy_version,
            assessment.basis().automation_policy_version
        );
        assert_eq!(merge_basis.field_hash, assessment.basis().field_hash);
        assert_eq!(
            merge_basis.work_order_id.as_deref(),
            Some(work.order.work_order_id.as_str())
        );
        assert_eq!(merge_basis.authorization_epoch, grant.authorization_epoch);
        assert_eq!(merge_basis.delivery, review.basis.delivery);
        assert_eq!(source_repository, SOURCE_REPOSITORY);
        assert!(current.items[0].open_decisions.is_empty());

        h.tick(true).unwrap();
        assert_eq!(delivery_grants(&h), grants);
        assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    }
}

#[test]
fn a_human_protected_diff_exception_never_becomes_auto_delivery_authority() {
    let h = policy_started(IssueRepositoryVisibility::Private, auto_policy());
    let start = h.work().unwrap().approved_basis;
    freeze(&h, 'b', ".github/workflows/ci.yml");
    h.tick(true).unwrap();
    let pending = h.work().unwrap();
    assert!(!pending.delivery_granted);
    assert_eq!(
        pending.blocked_reason,
        Some(IssueWorkBlockReason::AwaitingDeliveryApproval)
    );
    assert_eq!(
        pending
            .delivery_assessment
            .as_ref()
            .unwrap()
            .verdict
            .reasons,
        [IssueDeliveryRiskReason::ProtectedPath]
    );
    assert!(delivery_grants(&h).is_empty());
    let decision = h.decision();
    assert!(decision.request.definition().is_delivery_risk_exception());
    assert_eq!(
        decision.request.definition().on_timeout,
        IssueDecisionTimeout::Wait
    );

    answer_delivery(&h, "deliver");
    h.tick(true).unwrap();
    assert_human_delivery(&h);
    assert_eq!(h.work().unwrap().approved_basis, start);
    assert_eq!(
        h.work().unwrap().start_authority,
        IssueWorkStartAuthority::Policy
    );
    let current = page(&h.triage);
    assert!(
        current.items[0]
            .triage
            .as_ref()
            .unwrap()
            .assessment
            .gates()
            .iter()
            .filter(|gate| matches!(
                gate.gate,
                IssueAutomationGate::Delivery | IssueAutomationGate::Merge
            ))
            .all(|gate| gate.action == IssueAutomationGateAction::Auto)
    );
    h.tick(true).unwrap();
    assert_eq!(delivery_grants(&h).len(), 1);
}

#[test]
fn auto_mode_keeps_each_explicit_human_delivery_or_merge_gate() {
    for gate in ["delivery", "merge"] {
        let policy = auto_policy().replace(
            &format!("{gate} = \"auto\""),
            &format!("{gate} = \"human\""),
        );
        let h = policy_started(IssueRepositoryVisibility::Private, policy);
        freeze(&h, 'b', "src/feature.rs");
        h.tick(true).unwrap();
        assert_human_delivery(&h);
        assert_eq!(
            page(&h.triage).items[0]
                .triage
                .as_ref()
                .unwrap()
                .assessment
                .effective_mode(),
            IssueAutomationAutonomy::Auto
        );
    }
}

#[test]
fn gated_human_start_keeps_the_explicit_human_delivery_path() {
    let h = source_harness(IssueRepositoryVisibility::Private, Some(SOURCE_REPOSITORY));
    *h.policy.text.lock().unwrap() =
        super::gated::gated_policy().replace("start = \"auto\"", "start = \"human\"");
    super::gated::enable_gated(&h);
    h.tick(true).unwrap();
    assert!(h.work().is_none());
    h.answer("start");
    h.tick(true).unwrap();
    assert_eq!(
        h.work().unwrap().start_authority,
        IssueWorkStartAuthority::Human
    );
    freeze(&h, 'b', "src/feature.rs");
    h.tick(true).unwrap();
    assert_human_delivery(&h);
}

#[test]
fn restricted_sources_keep_the_propose_human_path_after_native_auto_selection() {
    for (visibility, coordinate) in [
        (IssueRepositoryVisibility::Public, Some(SOURCE_REPOSITORY)),
        (IssueRepositoryVisibility::Unknown, Some(SOURCE_REPOSITORY)),
        (IssueRepositoryVisibility::Private, None),
    ] {
        let h = source_harness(visibility, coordinate);
        select_auto(&h, auto_policy());
        h.tick(true).unwrap();
        assert!(h.work().is_none());
        assert!(h.execution.state.lock().unwrap().grants.is_empty());
        let current = page(&h.triage);
        let assessment = &current.items[0].triage.as_ref().unwrap().assessment;
        assert_eq!(
            assessment.effective_mode(),
            IssueAutomationAutonomy::Propose
        );
        assert!(
            assessment
                .reasons()
                .contains(&IssueTriageReasonCode::AutoRepositoryRestricted)
        );
        assert_eq!(h.decision().request.state(), IssueDecisionState::Open);

        h.answer("start");
        h.tick(true).unwrap();
        assert_eq!(
            h.work().unwrap().start_authority,
            IssueWorkStartAuthority::Human
        );
        freeze(&h, 'b', "src/feature.rs");
        h.tick(true).unwrap();
        assert_human_delivery(&h);
        assert!(
            h.execution
                .state
                .lock()
                .unwrap()
                .grants
                .iter()
                .all(|grant| {
                    !matches!(
                        grant.delivery_authority,
                        Some(IssueWorkDeliveryAuthority::Auto { .. })
                    )
                })
        );
    }
}

#[test]
fn auto_delivery_postcheck_revokes_changed_repository_or_policy_basis() {
    for change in [
        "public",
        "internal",
        "renamed",
        "missing_coordinate",
        "merge_policy",
    ] {
        let h = policy_started(IssueRepositoryVisibility::Private, auto_policy());
        freeze(&h, 'b', "src/feature.rs");
        let source = h.source.clone();
        let policy = h.policy.clone();
        let store = h.fixture.store();
        let workspace = h.fixture.workspace.clone();
        let issue = h.fixture.reference("I_001");
        h.execution.state.lock().unwrap().after_grant = Some(Box::new(move || {
            if change == "merge_policy" {
                *policy.text.lock().unwrap() =
                    auto_policy().replace("merge = \"auto\"", "merge = \"human\"");
                return;
            }
            let mut replacement = source.read.lock().unwrap().clone();
            let IssueSourceRead::Observed(observation) = &mut replacement else {
                panic!("the current fixture source must remain an observation");
            };
            match change {
                "public" => observation.repository_visibility = IssueRepositoryVisibility::Public,
                "internal" => {
                    observation.repository_visibility = IssueRepositoryVisibility::Internal
                }
                "renamed" => {
                    observation.repository_name_with_owner = Some("refinedstone/renamed".into())
                }
                "missing_coordinate" => observation.repository_name_with_owner = None,
                _ => unreachable!(),
            }
            *source.read.lock().unwrap() = replacement.clone();
            let collection = store.ensure_collection(&workspace).unwrap();
            assert!(matches!(
                store
                    .apply_issue_resync(&workspace, collection.fence(), &issue, &replacement)
                    .unwrap(),
                IssueCollectionApplyResult::Applied(_)
            ));
        }));

        h.tick(true).unwrap();

        let grants = delivery_grants(&h);
        assert_eq!(grants.len(), 1, "{change}");
        assert!(matches!(
            grants[0].delivery_authority,
            Some(IssueWorkDeliveryAuthority::Auto { .. })
        ));
        let state = h.execution.state.lock().unwrap();
        assert!(state.revoked, "{change}");
        assert!(state.after_grant.is_none());
        assert_eq!(state.registrations, 1);
        drop(state);
        let work = h.work().unwrap();
        assert!(!work.delivery_granted, "{change}");
        assert!(work.delivery_assessment.is_none(), "{change}");
        assert_eq!(
            work.blocked_reason,
            Some(IssueWorkBlockReason::ApprovalChanged)
        );
        assert!(
            h.fixture
                .store()
                .pending_issue_work_events(&h.fixture.workspace)
                .unwrap()
                .iter()
                .all(|event| {
                    !matches!(
                        event.kind,
                        IssueWorkEventKind::PermitGranted { delivery: true, .. }
                    )
                }),
            "a rejected postcheck must not publish a delivery success: {change}"
        );
    }
}
