use super::*;
use crate::application::port::inbound::stored_command_port::issue_confirmation::VerifiedHumanConfirmation;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_work_execution_port::*;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_binding_admission::IssueBindingAdmission;
use crate::application::service::issue_binding_pause::IssueBindingPauseService;
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::work_control::{IssueWorkControl, IssueWorkControlAction};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Policy {
    available: AtomicBool,
    text: Mutex<String>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            available: AtomicBool::new(true),
            text: Mutex::new(
                "[binding.github]\nmode='gated'\n[authority]\nautonomy_cap='gated'\n".into(),
            ),
        }
    }
}
impl IssueAutomationPolicySourcePort for Policy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        if !self.available.load(Ordering::SeqCst) {
            bail!("fixture policy unavailable");
        }
        Ok(Some(self.text.lock().unwrap().clone()))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

#[derive(Default)]
struct ControlState {
    control: IssueWorkControl,
    commands: BTreeMap<String, (String, IssueWorkControlAction, IssueWorkControl)>,
}
#[derive(Default)]
struct Execution {
    state: Mutex<ControlState>,
    revokes: AtomicUsize,
}
impl IssueWorkExecutionPort for Execution {
    fn snapshot(&self, _: &str, _: &str) -> Result<IssueWorkExecutionSnapshot> {
        bail!("unexpected snapshot")
    }
    fn submit(&self, _: &str, _: &IssueWorkOrder) -> Result<Vec<String>> {
        bail!("unexpected submit")
    }
    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> Result<()> {
        bail!("unexpected grant")
    }
    fn revoke(&self, _: &str, _: &str) -> Result<()> {
        self.revokes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn control(&self, _: &str, _: &str) -> Result<IssueWorkControl> {
        Ok(self.state.lock().unwrap().control.clone())
    }
    fn apply_control(
        &self,
        _: &str,
        work: &str,
        command_id: &str,
        action: IssueWorkControlAction,
    ) -> Result<IssueWorkControl> {
        let mut state = self.state.lock().unwrap();
        if let Some((original_work, original_action, original)) = state.commands.get(command_id) {
            assert_eq!(original_work, work);
            assert_eq!(*original_action, action);
            return Ok(original.clone());
        }
        let next = state.control.apply(action)?;
        state.control = next.clone();
        state
            .commands
            .insert(command_id.into(), (work.into(), action, next.clone()));
        Ok(next)
    }
}

struct ServiceFixture {
    disk: Fixture,
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    policy: Arc<Policy>,
    execution: Arc<Execution>,
    admission: Arc<IssueBindingAdmission>,
    work: IssueWorkCheckpoint,
    binding: IssueAutomationBindingId,
}
impl ServiceFixture {
    fn new(label: &str) -> Self {
        let disk = Fixture::new(label);
        let store = Arc::new(disk.adapter());
        let issue = super::super::super::tests::reference(disk.cwd(), "I_service");
        let work = super::super::super::test_support::seed_issue_work(
            store.as_ref(),
            &issue,
            &["task-service".into()],
        )
        .unwrap();
        let binding = store
            .resolve_issue_work_binding(disk.cwd(), &work.order.work_order_id)
            .unwrap()
            .unwrap();
        Self {
            disk,
            store,
            work,
            binding,
            policy: Arc::new(Policy::default()),
            execution: Arc::new(Execution::default()),
            admission: Arc::new(IssueBindingAdmission::new()),
        }
    }
    fn service(&self, admission: Arc<IssueBindingAdmission>) -> IssueBindingPauseService {
        IssueBindingPauseService::new(
            self.disk.cwd().into(),
            self.store.clone(),
            IssueAutomationPolicyService::new(self.policy.clone()),
            self.execution.clone(),
            admission,
        )
    }
}

#[test]
fn binding_pause_service_waits_for_old_effects_and_recovery_reuses_the_owned_retriage() {
    let fixture = ServiceFixture::new("binding-pause-live-drain");
    let service = fixture.service(fixture.admission.clone());
    service.recover_before_runtime().unwrap();
    let generation = fixture.admission.capture(&fixture.binding).unwrap();
    let old_effect = fixture
        .admission
        .acquire(&fixture.binding, generation)
        .unwrap();
    fixture.policy.available.store(false, Ordering::SeqCst);
    let accepted = service
        .set("pause".into(), fixture.binding.clone(), true, 0, None)
        .unwrap();
    assert!(accepted.receipt.is_none());
    assert!(
        fixture
            .admission
            .acquire(&fixture.binding, generation)
            .is_err()
    );
    assert!(!service.poll().unwrap());
    assert_eq!(
        fixture
            .execution
            .control(fixture.disk.cwd(), &fixture.work.order.work_order_id)
            .unwrap()
            .authorization_epoch,
        1
    );
    assert!(
        fixture
            .store
            .get_issue_binding_pause_command(fixture.disk.cwd(), "pause")
            .unwrap()
            .unwrap()
            .receipt
            .is_none()
    );
    assert!(service.pending_events(64).unwrap().is_empty());
    drop(old_effect);
    assert!(service.poll().unwrap());
    let completed = fixture
        .store
        .get_issue_binding_pause_command(fixture.disk.cwd(), "pause")
        .unwrap()
        .unwrap();
    assert_eq!(
        completed.receipt.as_ref().unwrap().outcome,
        IssueBindingPauseOutcome::Paused
    );
    assert_eq!(
        service
            .set("pause".into(), fixture.binding.clone(), true, 0, None)
            .unwrap(),
        completed
    );
    drop(service);
    let recovered_gate = Arc::new(IssueBindingAdmission::new());
    let recovered = fixture.service(recovered_gate.clone());
    recovered.recover_before_runtime().unwrap();
    assert!(recovered_gate.capture(&fixture.binding).is_err());
    assert!(!recovered.poll().unwrap());
    assert_eq!(fixture.execution.state.lock().unwrap().commands.len(), 1);
    assert_eq!(
        fixture
            .execution
            .control(fixture.disk.cwd(), &fixture.work.order.work_order_id)
            .unwrap()
            .authorization_epoch,
        1
    );
    assert!(fixture.execution.revokes.load(Ordering::SeqCst) > 0);
    let events = recovered.pending_events(64).unwrap();
    assert_eq!(events.len(), 1);
    recovered.acknowledge_event(&events[0]).unwrap();
    assert!(!recovered.has_pending());
}

#[test]
fn binding_resume_policy_drift_keeps_the_binding_closed_and_preserves_human_control_flags() {
    let fixture = ServiceFixture::new("binding-resume-superseded-service");
    fixture.execution.state.lock().unwrap().control = IssueWorkControl {
        held: true,
        taken_over: true,
        tracked: false,
        ..Default::default()
    };
    let service = fixture.service(fixture.admission.clone());
    service.recover_before_runtime().unwrap();
    service
        .set("pause".into(), fixture.binding.clone(), true, 0, None)
        .unwrap();
    assert!(service.poll().unwrap());
    assert!(
        service
            .set("resume".into(), fixture.binding.clone(), false, 1, None)
            .is_err()
    );
    let preview = service
        .prepare_confirmation("resume", &fixture.binding, false, 1)
        .unwrap();
    // Only the existing verified-capability fixture is used; this is not a native broker proof test.
    let verified = VerifiedHumanConfirmation::from_verified(
        preview.target,
        format!("desktop-confirmation:{}", "b".repeat(32)),
    )
    .unwrap();
    let pending = service
        .set(
            "resume".into(),
            fixture.binding.clone(),
            false,
            1,
            Some(&verified),
        )
        .unwrap();
    assert!(pending.receipt.is_none());
    fixture
        .policy
        .text
        .lock()
        .unwrap()
        .push_str("\n[budget]\nmax_running=1\n");
    assert!(service.poll().unwrap());
    let result = fixture
        .store
        .get_issue_binding_pause_command(fixture.disk.cwd(), "resume")
        .unwrap()
        .unwrap();
    assert_eq!(
        result.receipt.unwrap().outcome,
        IssueBindingPauseOutcome::ResumeSuperseded
    );
    assert!(fixture.admission.capture(&fixture.binding).is_err());
    let control = fixture
        .execution
        .control(fixture.disk.cwd(), &fixture.work.order.work_order_id)
        .unwrap();
    assert!(control.held && control.taken_over && !control.tracked);
    assert_eq!(control.authorization_epoch, 2);
    assert_eq!(fixture.execution.state.lock().unwrap().commands.len(), 2);
}

#[test]
fn binding_pause_recovery_explicitly_revokes_after_a_generic_recoverer_already_saved_the_receipt() {
    use crate::domain::issue_automation::intervention::IssueInterventionReceipt;
    let fixture = ServiceFixture::new("binding-pause-generic-recovery");
    let service = fixture.service(fixture.admission.clone());
    service.recover_before_runtime().unwrap();
    let accepted = service
        .set("pause".into(), fixture.binding.clone(), true, 0, None)
        .unwrap();
    let owned = fixture
        .store
        .claim_issue_binding_pause_fence(fixture.disk.cwd(), &accepted.intent, &fixture.work)
        .unwrap();
    let control = fixture
        .execution
        .apply_control(
            fixture.disk.cwd(),
            &fixture.work.order.work_order_id,
            &owned.request.command_id,
            IssueWorkControlAction::Retriage,
        )
        .unwrap();
    fixture
        .store
        .finish_issue_intervention(
            fixture.disk.cwd(),
            &IssueInterventionReceipt {
                issue: fixture.work.order.issue.clone(),
                command_id: owned.request.command_id,
                action: "retriage_issue".into(),
                work_order_id: Some(fixture.work.order.work_order_id.clone()),
                authorization_epoch: control.authorization_epoch,
                changed: false,
                delivery: None,
                handoff: None,
            },
        )
        .unwrap();
    assert_eq!(fixture.execution.revokes.load(Ordering::SeqCst), 0);
    drop(service);
    let recovered = fixture.service(Arc::new(IssueBindingAdmission::new()));
    recovered.recover_before_runtime().unwrap();
    assert!(recovered.poll().unwrap());
    assert_eq!(fixture.execution.revokes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.execution.state.lock().unwrap().commands.len(), 1);
    assert_eq!(
        fixture
            .store
            .get_issue_binding_pause_command(fixture.disk.cwd(), "pause")
            .unwrap()
            .unwrap()
            .receipt
            .unwrap()
            .outcome,
        IssueBindingPauseOutcome::Paused
    );
}

#[test]
fn binding_resume_committed_without_a_response_reopens_only_from_its_exact_completed_event() {
    use crate::domain::issue_automation::intervention::IssueInterventionReceipt;

    let fixture = ServiceFixture::new("binding-resume-lost-finish-response");
    let service = fixture.service(fixture.admission.clone());
    service.recover_before_runtime().unwrap();
    service
        .set("pause".into(), fixture.binding.clone(), true, 0, None)
        .unwrap();
    assert!(service.poll().unwrap());
    let old_events = service.pending_events(64).unwrap();
    assert_eq!(old_events.len(), 1);
    let old_pause = old_events[0].clone();
    assert_eq!(old_pause.receipt.outcome, IssueBindingPauseOutcome::Paused);

    let preview = service
        .prepare_confirmation("resume", &fixture.binding, false, 1)
        .unwrap();
    let verified = VerifiedHumanConfirmation::from_verified(
        preview.target,
        format!("desktop-confirmation:{}", "c".repeat(32)),
    )
    .unwrap();
    let accepted = service
        .set(
            "resume".into(),
            fixture.binding.clone(),
            false,
            1,
            Some(&verified),
        )
        .unwrap();
    assert!(accepted.receipt.is_none());
    assert_eq!(accepted.accepted_revision, 2);
    assert!(fixture.admission.capture(&fixture.binding).is_err());

    let owned = fixture
        .store
        .claim_issue_binding_pause_fence(fixture.disk.cwd(), &accepted.intent, &fixture.work)
        .unwrap();
    let control = fixture
        .execution
        .apply_control(
            fixture.disk.cwd(),
            &fixture.work.order.work_order_id,
            &owned.request.command_id,
            IssueWorkControlAction::Retriage,
        )
        .unwrap();
    fixture
        .execution
        .revoke(fixture.disk.cwd(), &fixture.work.order.work_order_id)
        .unwrap();
    fixture
        .store
        .finish_issue_intervention(
            fixture.disk.cwd(),
            &IssueInterventionReceipt {
                issue: fixture.work.order.issue.clone(),
                command_id: owned.request.command_id,
                action: "retriage_issue".into(),
                work_order_id: Some(fixture.work.order.work_order_id.clone()),
                authorization_epoch: control.authorization_epoch,
                changed: true,
                delivery: None,
                handoff: None,
            },
        )
        .unwrap();

    // Persist the actual finish, but do not deliver its response to the memory admission gate.
    let completed = fixture
        .store
        .finish_issue_binding_pause(
            fixture.disk.cwd(),
            &accepted.intent,
            IssueBindingPauseCompletion::Apply,
        )
        .unwrap();
    let receipt = completed.receipt.as_ref().unwrap();
    assert_eq!(receipt.outcome, IssueBindingPauseOutcome::Resumed);
    let stored_state = service.state(&fixture.binding).unwrap();
    assert!(stored_state.allows_automation());
    assert_eq!(stored_state.revision, 2);
    assert!(
        fixture
            .store
            .pending_issue_binding_pause_commands(fixture.disk.cwd(), None, 64)
            .unwrap()
            .is_empty()
    );
    assert!(fixture.admission.capture(&fixture.binding).is_err());
    assert!(!service.poll().unwrap());
    assert!(fixture.admission.capture(&fixture.binding).is_err());

    // The older unacknowledged Pause is encountered first and cannot settle this Resume.
    assert_eq!(service.pending_events(1).unwrap(), vec![old_pause.clone()]);
    assert!(fixture.admission.capture(&fixture.binding).is_err());
    let events = service.pending_events(64).unwrap();
    assert_eq!(events.len(), 2);
    let resumed = events
        .iter()
        .find(|event| event.receipt.command_id == accepted.intent.command_id)
        .unwrap()
        .clone();
    assert_eq!(&resumed.receipt, receipt);
    assert_eq!(fixture.admission.capture(&fixture.binding).unwrap(), 2);
    assert!(fixture.admission.acquire(&fixture.binding, 0).is_err());
    drop(fixture.admission.acquire(&fixture.binding, 2).unwrap());

    assert_eq!(service.pending_events(64).unwrap(), events);
    for event in [&old_pause, &resumed] {
        service.acknowledge_event(event).unwrap();
        service.acknowledge_event(event).unwrap();
        assert_eq!(fixture.admission.capture(&fixture.binding).unwrap(), 2);
        assert_eq!(service.state(&fixture.binding).unwrap(), stored_state);
    }
    assert!(service.pending_events(64).unwrap().is_empty());
    assert!(!service.has_pending());
    assert_eq!(
        fixture
            .store
            .finish_issue_binding_pause(
                fixture.disk.cwd(),
                &accepted.intent,
                IssueBindingPauseCompletion::Apply,
            )
            .unwrap(),
        completed
    );
    assert_eq!(fixture.execution.state.lock().unwrap().commands.len(), 2);
    assert_eq!(fixture.execution.revokes.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture
            .execution
            .control(fixture.disk.cwd(), &fixture.work.order.work_order_id)
            .unwrap()
            .authorization_epoch,
        2
    );
}
