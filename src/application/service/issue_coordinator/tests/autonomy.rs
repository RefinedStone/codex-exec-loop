use super::*;
use crate::application::port::inbound::stored_command_port::issue_confirmation::VerifiedHumanConfirmation;
use crate::application::service::issue_autonomy::{
    IssueAutonomyService, effective_policy, resolve_runtime_policy,
};
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::autonomy::{
    IssueAutonomyLevel, IssueAutonomyProjection, IssueAutonomySelection,
};
use crate::domain::issue_automation::work_control::IssueWorkControl;

fn autonomy(h: &Harness) -> IssueAutonomyService {
    IssueAutonomyService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        IssueAutomationPolicyService::new(h.policy.clone()),
        h.execution.clone(),
    )
}

fn confirm(
    service: &IssueAutonomyService,
    id: &str,
    level: IssueAutonomyLevel,
) -> VerifiedHumanConfirmation {
    VerifiedHumanConfirmation::from_verified(
        service.prepare_confirmation(id, level).unwrap().target,
        format!("desktop-confirmation:{}", "a".repeat(32)),
    )
    .unwrap()
}

fn approved_work(h: &Harness) {
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
}

#[test]
fn runtime_override_has_a_distinct_hash_and_preserves_file_policy_caps_and_gates() {
    let h = Harness::new();
    let policy = IssueAutomationPolicyService::new(h.policy.clone());
    let file = policy.resolve(&h.fixture.workspace).unwrap();
    let original_hash = file.version.clone();
    let inherited = effective_policy(file.clone(), &IssueAutonomySelection::default());
    assert_eq!(inherited, file);
    for level in IssueAutonomyLevel::ALL {
        let current = effective_policy(
            file.clone(),
            &IssueAutonomySelection {
                requested_level: Some(level),
                revision: 1,
                pending: None,
            },
        );
        assert_ne!(current.version, original_hash);
        assert_eq!(
            current.policy.binding.github.mode,
            IssueAutomationAutonomy::from(level)
        );
        let mut restored = current.policy;
        restored.binding.github.mode = file.policy.binding.github.mode;
        assert_eq!(restored, file.policy);
        restored.validate().unwrap();
    }
    assert_eq!(
        policy.resolve(&h.fixture.workspace).unwrap().version,
        original_hash
    );
}

#[test]
fn raising_requires_current_human_confirmation_and_cannot_exceed_cap() {
    let h = Harness::new();
    let service = autonomy(&h);
    service
        .set("off".into(), IssueAutonomyLevel::Off, None)
        .unwrap();
    assert!(
        service
            .set("raise".into(), IssueAutonomyLevel::Propose, None)
            .is_err()
    );
    let stale = confirm(&service, "raise", IssueAutonomyLevel::Propose);
    let observe = confirm(&service, "observe", IssueAutonomyLevel::Observe);
    service
        .set(
            "observe".into(),
            IssueAutonomyLevel::Observe,
            Some(&observe),
        )
        .unwrap();
    assert!(
        service
            .set("raise".into(), IssueAutonomyLevel::Propose, Some(&stale))
            .is_err()
    );
    let current = confirm(&service, "raise", IssueAutonomyLevel::Propose);
    assert!(
        service
            .set("raise".into(), IssueAutonomyLevel::Propose, Some(&current))
            .unwrap()
            .changed
    );
    *h.policy.text.lock().unwrap() =
        AUTO_POLICY.replace("autonomy_cap = \"propose\"", "autonomy_cap = \"observe\"");
    let IssueAutonomyProjection::Ready(state) = service.state() else {
        panic!("expected policy state");
    };
    assert_eq!(state.effective_level, IssueAutomationAutonomy::Observe);
    assert_eq!(state.cap, IssueAutomationAutonomy::Observe);
    assert!(
        !state
            .choices
            .iter()
            .find(|choice| choice.level == IssueAutonomyLevel::Propose)
            .unwrap()
            .enabled
    );
    assert!(
        service
            .set("above-cap".into(), IssueAutonomyLevel::Propose, None)
            .is_err()
    );
}

#[test]
fn downgrade_fences_existing_permits_without_changing_collection_and_replay_is_noop() {
    let h = Harness::new();
    approved_work(&h);
    let before_collection = h
        .fixture
        .store()
        .ensure_collection(&h.fixture.workspace)
        .unwrap();
    let service = autonomy(&h);
    let changed = service
        .set("off".into(), IssueAutonomyLevel::Off, None)
        .unwrap();
    assert!(changed.changed);
    assert_eq!(
        h.fixture
            .store()
            .ensure_collection(&h.fixture.workspace)
            .unwrap(),
        before_collection
    );
    {
        let state = h.execution.state.lock().unwrap();
        assert!(state.revoked);
        assert_eq!(state.control.authorization_epoch, 1);
        assert_eq!(state.control_commands.len(), 1);
        let old = &state.grants[0];
        assert!(
            state
                .control
                .require_epoch(old.authorization_epoch)
                .is_err()
        );
    }
    assert!(
        !service
            .set("off".into(), IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
    assert!(
        !service
            .set("off-again".into(), IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
    assert_eq!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control
            .authorization_epoch,
        1
    );
    assert_eq!(
        h.fixture
            .store()
            .issue_autonomy_selection(&h.fixture.workspace)
            .unwrap()
            .revision,
        1
    );

    let confirmation = confirm(&service, "propose", IssueAutonomyLevel::Propose);
    service
        .set(
            "propose".into(),
            IssueAutonomyLevel::Propose,
            Some(&confirmation),
        )
        .unwrap();
    h.tick(true).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    assert_eq!(h.decision().request.state(), IssueDecisionState::Open);
    assert_eq!(
        h.decision().request.definition().basis.authorization_epoch,
        2
    );
    h.answer("start");
    h.tick(true).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 2);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn crashed_revocation_keeps_pending_fence_until_startup_recovery_finishes() {
    let h = Harness::new();
    approved_work(&h);
    let work_order_id = h.work().unwrap().order.work_order_id;
    h.execution.state.lock().unwrap().fail_revoke_work_order = Some(work_order_id);
    let service = autonomy(&h);
    assert!(
        service
            .set("off-crash".into(), IssueAutonomyLevel::Off, None)
            .is_err()
    );
    let pending = h
        .fixture
        .store()
        .issue_autonomy_selection(&h.fixture.workspace)
        .unwrap();
    assert_eq!(pending.revision, 0);
    assert_eq!(pending.pending.as_ref().unwrap().command_id, "off-crash");
    assert!(
        resolve_runtime_policy(
            &h.fixture.workspace,
            h.fixture.store().as_ref(),
            &IssueAutomationPolicyService::new(h.policy.clone()),
        )
        .is_err()
    );
    let IssueAutonomyProjection::Ready(state) = service.state() else {
        panic!("expected pending state");
    };
    assert!(state.pending);
    assert!(state.choices.iter().all(|choice| !choice.enabled));
    assert_eq!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control
            .authorization_epoch,
        1
    );

    h.execution.state.lock().unwrap().fail_revoke_work_order = None;
    autonomy(&h).recover_before_runtime().unwrap();
    let recovered = h
        .fixture
        .store()
        .issue_autonomy_selection(&h.fixture.workspace)
        .unwrap();
    assert_eq!(recovered.requested_level, Some(IssueAutonomyLevel::Off));
    assert_eq!(recovered.revision, 1);
    assert!(recovered.pending.is_none());
    assert!(
        h.fixture
            .store()
            .pending_issue_interventions(&h.fixture.workspace)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control
            .authorization_epoch,
        1
    );
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    assert!(
        !autonomy(&h)
            .set("off-crash".into(), IssueAutonomyLevel::Off, None)
            .unwrap()
            .changed
    );
}

#[test]
fn changing_level_preserves_every_existing_control_flag() {
    let h = Harness::new();
    approved_work(&h);
    let before = IssueWorkControl {
        held: true,
        taken_over: true,
        cancelled: true,
        tracked: false,
        authorization_epoch: 7,
    };
    h.execution.state.lock().unwrap().control = before.clone();
    autonomy(&h)
        .set("off-controlled".into(), IssueAutonomyLevel::Off, None)
        .unwrap();
    assert_eq!(
        h.execution.state.lock().unwrap().control,
        IssueWorkControl {
            authorization_epoch: 8,
            ..before
        }
    );
}

#[test]
fn pending_and_committed_selection_fence_all_previous_assessment_write_paths() {
    use crate::domain::issue_automation::policy::IssueAutomationGateAction;
    let h = Harness::new();
    approved_work(&h);
    let store = h.fixture.store();
    let workspace = &h.fixture.workspace;
    let work = h.work().unwrap();
    let (fence, _, assessment) = h.coordinator().current(&work.order.issue, &|_| {}).unwrap();
    let mut definition = store
        .get_issue_decision(workspace, &work.decision_id)
        .unwrap()
        .unwrap()
        .request
        .definition()
        .clone();
    definition.question =
        IssueDecisionSharedText::new_shared("추가 확인 결정을 기록할까요?").unwrap();
    let open = h
        .decisions
        .ensure_request(
            IssueDecisionId::new("open-before-autonomy").unwrap(),
            definition.clone(),
        )
        .unwrap()
        .record;
    let preview = h
        .decisions
        .prepare_confirmation(open.request.id().as_str(), "start")
        .unwrap();
    let confirmation = VerifiedHumanConfirmation::from_verified(
        preview.target,
        format!("desktop-confirmation:{}", "d".repeat(32)),
    )
    .unwrap();
    let answered = open
        .request
        .answer_for_gate(
            &open.request.definition().basis,
            IssueAutomationGateAction::Human,
            IssueDecisionAgentAnswer {
                option: "start".into(),
                note: None,
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-21T00:00:00Z".into(),
            },
            Some(confirmation.reference()),
        )
        .unwrap()
        .request;
    // A new Open request tests admission; the existing Open request tests answer persistence.
    // Replaying the already Answered work approval would exercise neither write boundary.
    definition.question =
        IssueDecisionSharedText::new_shared("새 판정 요청을 추가할까요?").unwrap();
    let new_request = IssueDecisionRequest::new(
        IssueDecisionId::new("not-yet-persisted").unwrap(),
        definition,
    )
    .unwrap();
    let intent = crate::domain::issue_automation::autonomy::IssueAutonomyIntent {
        command_id: "fence-admission".into(),
        level: IssueAutonomyLevel::Off,
        expected_revision: 0,
        policy_version: assessment.basis().automation_policy_version.clone(),
    };
    store.begin_issue_autonomy(workspace, &intent).unwrap();
    let assert_old_writes_rejected = || {
        assert_eq!(
            store.save_triage(workspace, fence, &assessment).unwrap(),
            IssueTriageSaveResult::Conflict
        );
        assert_eq!(
            store
                .ensure_issue_decision(workspace, fence, &assessment, &new_request)
                .unwrap(),
            IssueDecisionSaveResult::Conflict
        );
        assert_eq!(
            store
                .save_issue_decision(
                    workspace,
                    IssueDecisionFence {
                        triage: fence,
                        decision_revision: open.revision
                    },
                    &assessment,
                    &answered,
                )
                .unwrap(),
            IssueDecisionSaveResult::Conflict
        );
        assert_eq!(
            store
                .save_issue_work(workspace, fence, &assessment, &work)
                .unwrap(),
            IssueWorkSaveResult::Conflict
        );
    };
    assert_old_writes_rejected();
    autonomy(&h).recover_before_runtime().unwrap();
    assert_old_writes_rejected();
}

#[test]
fn recovery_finishing_the_same_retriage_is_accepted_without_rewriting_its_receipt() {
    let h = Harness::new();
    approved_work(&h);
    let store = h.fixture.store();
    let workspace = h.fixture.workspace.clone();
    let execution = h.execution.clone();
    h.execution.state.lock().unwrap().after_revoke = Some(Box::new(move || {
        crate::application::service::issue_intervention::recover_pending_controls(
            &workspace,
            store.as_ref(),
            execution.as_ref(),
        )
        .unwrap();
    }));
    let result = autonomy(&h)
        .set("race-recovery".into(), IssueAutonomyLevel::Off, None)
        .unwrap();
    assert!(result.changed);
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.control.authorization_epoch, 1);
    let control_id = state.control_commands.keys().next().unwrap().clone();
    drop(state);
    let stored = h
        .fixture
        .store()
        .get_issue_intervention(&h.fixture.workspace, &control_id)
        .unwrap()
        .unwrap();
    let receipt = stored.receipt.unwrap();
    assert!(
        !receipt.changed,
        "the first recovery receipt must remain immutable"
    );
    assert_eq!(receipt.authorization_epoch, 1);
    assert_eq!(
        h.fixture
            .store()
            .issue_autonomy_selection(&h.fixture.workspace)
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn competing_control_after_recovery_keeps_autonomy_pending_and_first_receipt_immutable() {
    use crate::domain::issue_automation::work_control::IssueWorkControlAction;
    let h = Harness::new();
    approved_work(&h);
    let store = h.fixture.store();
    let workspace = h.fixture.workspace.clone();
    let execution = h.execution.clone();
    let work_order_id = h.work().unwrap().order.work_order_id;
    h.execution.state.lock().unwrap().after_revoke = Some(Box::new(move || {
        crate::application::service::issue_intervention::recover_pending_controls(
            &workspace,
            store.as_ref(),
            execution.as_ref(),
        )
        .unwrap();
        execution
            .apply_control(
                &workspace,
                &work_order_id,
                "competing-hold",
                IssueWorkControlAction::Hold,
            )
            .unwrap();
    }));
    assert!(
        autonomy(&h)
            .set("race-new-control".into(), IssueAutonomyLevel::Off, None)
            .is_err()
    );
    let state = h.execution.state.lock().unwrap();
    assert!(state.control.held);
    assert_eq!(state.control.authorization_epoch, 2);
    let control_id = state
        .control_commands
        .keys()
        .find(|id| id.starts_with("autonomy-"))
        .unwrap()
        .clone();
    drop(state);
    let stored = h
        .fixture
        .store()
        .get_issue_intervention(&h.fixture.workspace, &control_id)
        .unwrap()
        .unwrap();
    let receipt = stored.receipt.unwrap();
    assert!(!receipt.changed);
    assert_eq!(receipt.authorization_epoch, 1);
    let selection = h
        .fixture
        .store()
        .issue_autonomy_selection(&h.fixture.workspace)
        .unwrap();
    assert_eq!(selection.revision, 0);
    assert_eq!(selection.pending.unwrap().command_id, "race-new-control");
}

mod fences;
