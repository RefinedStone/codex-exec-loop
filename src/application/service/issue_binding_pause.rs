//! 바인딩별 Pause/Resume. 허가 회수와 진행 중인 효과의 정리가 끝난 뒤 완료를 기록한다.
use super::issue_automation_policy::IssueAutomationPolicyService;
use super::issue_binding_admission::IssueBindingAdmission;
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationDisplay, IssueConfirmationError, IssueConfirmationPreview,
    IssueConfirmationTarget, VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionPort;
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_automation::autonomy::validate_command_id;
use crate::domain::issue_automation::binding_pause::*;
use anyhow::{Result, anyhow, bail};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod recovery;

type Waker = Arc<dyn Fn() + Send + Sync>;
const POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Default)]
struct RuntimeState {
    pending: bool,
    next_deadline: Option<Instant>,
    after_binding: Option<IssueAutomationBindingId>,
    work_cursors: HashMap<String, String>,
    waker: Option<Waker>,
}

pub struct IssueBindingPauseService {
    workspace: String,
    store: Arc<dyn IssueAutomationStorePort>,
    policy: IssueAutomationPolicyService,
    execution: Arc<dyn IssueWorkExecutionPort>,
    admission: Arc<IssueBindingAdmission>,
    command_gate: Mutex<()>,
    runtime: Mutex<RuntimeState>,
}

impl IssueBindingPauseService {
    pub fn new(
        workspace: String,
        store: Arc<dyn IssueAutomationStorePort>,
        policy: IssueAutomationPolicyService,
        execution: Arc<dyn IssueWorkExecutionPort>,
        admission: Arc<IssueBindingAdmission>,
    ) -> Self {
        Self {
            workspace,
            store,
            policy,
            execution,
            admission,
            command_gate: Mutex::new(()),
            runtime: Mutex::new(RuntimeState::default()),
        }
    }

    pub fn state(&self, binding_id: &IssueAutomationBindingId) -> Result<IssueBindingPauseState> {
        self.store
            .issue_binding_pause_state(&self.workspace, binding_id)
    }

    fn current_resume_basis(
        &self,
        binding_id: &IssueAutomationBindingId,
    ) -> Result<IssueBindingResumeBasis> {
        let binding = self
            .store
            .get_binding(&self.workspace, binding_id)?
            .ok_or_else(|| anyhow!("issue binding is unavailable"))?;
        let (policy, autonomy_revision) = super::issue_autonomy::resolve_runtime_policy(
            &self.workspace,
            self.store.as_ref(),
            &self.policy,
        )?;
        let basis = IssueBindingResumeBasis {
            binding_revision: binding.revision,
            autonomy_revision,
            policy_version: policy.version,
            effective_level: policy.policy.binding.github.mode,
        };
        basis.validate().map_err(|reason| anyhow!(reason))?;
        Ok(basis)
    }

    fn target(
        &self,
        command_id: &str,
        binding_id: &IssueAutomationBindingId,
        expected_revision: u64,
        resume_basis: IssueBindingResumeBasis,
    ) -> IssueConfirmationTarget {
        IssueConfirmationTarget::BindingResume {
            workspace: self.workspace.clone(),
            command_id: command_id.to_owned(),
            binding_id: binding_id.to_string(),
            expected_revision,
            resume_basis,
        }
    }

    fn validate_current(current: &IssueBindingPauseState, expected_revision: u64) -> Result<()> {
        if current.revision != expected_revision {
            bail!("issue binding pause revision is stale");
        }
        if current.pending.is_some() {
            bail!("issue binding pause change is pending");
        }
        Ok(())
    }

    pub fn prepare_confirmation(
        &self,
        command_id: &str,
        binding_id: &IssueAutomationBindingId,
        paused: bool,
        expected_revision: u64,
    ) -> Result<IssueConfirmationPreview> {
        validate_command_id(command_id).map_err(|reason| anyhow!(reason))?;
        validate_binding_pause_id(binding_id).map_err(|reason| anyhow!(reason))?;
        let current = self.state(binding_id)?;
        Self::validate_current(&current, expected_revision)?;
        if paused || !current.paused {
            bail!("only resuming a paused issue binding requires confirmation");
        }
        let basis = self.current_resume_basis(binding_id)?;
        Ok(IssueConfirmationPreview {
            target: self.target(command_id, binding_id, expected_revision, basis.clone()),
            display: IssueConfirmationDisplay {
                message: "이 바인딩의 이슈 자동화를 다시 시작할까요?".into(),
                detail: format!(
                    "바인딩: {binding_id}\n현재 자율 수준: {:?}\n현재 정책으로 새 허가를 판정합니다. 정지 전에 쓰던 허가와 지연된 요청은 재사용하지 않으며, 작업별 보류·인계·취소 상태는 유지합니다.",
                    basis.effective_level
                ),
                confirm_label: "자동화 재개".into(),
            },
        })
    }

    pub fn set(
        &self,
        command_id: String,
        binding_id: IssueAutomationBindingId,
        paused: bool,
        expected_revision: u64,
        confirmation: Option<&VerifiedHumanConfirmation>,
    ) -> Result<StoredIssueBindingPauseCommand> {
        validate_command_id(&command_id).map_err(|reason| anyhow!(reason))?;
        validate_binding_pause_id(&binding_id).map_err(|reason| anyhow!(reason))?;
        let _guard = self
            .command_gate
            .lock()
            .map_err(|_| anyhow!("issue binding pause command gate is unavailable"))?;
        if let Some(stored) = self
            .store
            .get_issue_binding_pause_command(&self.workspace, &command_id)?
        {
            if stored.intent.binding_id != binding_id
                || stored.intent.paused != paused
                || stored.intent.expected_revision != expected_revision
            {
                bail!("issue binding pause command ID was reused");
            }
            // Replay returns the immutable first command, never a receipt fabricated from
            // current state or a new policy/confirmation. Pending recovery is scheduled below.
            self.admission
                .register_if_absent(&self.state(&binding_id)?)?;
            self.refresh_schedule()?;
            return Ok(stored);
        }
        let current = self.state(&binding_id)?;
        Self::validate_current(&current, expected_revision)?;
        self.admission.register_if_absent(&current)?;
        let resume_basis = if paused {
            if confirmation.is_some() {
                bail!("pausing an issue binding does not accept a resume confirmation");
            }
            None
        } else {
            let basis = self.current_resume_basis(&binding_id)?;
            let target = self.target(&command_id, &binding_id, expected_revision, basis.clone());
            if let Some(confirmation) = confirmation {
                confirmation.require_target(&target)?;
            }
            if current.paused {
                confirmation.ok_or(IssueConfirmationError::Required)?;
            }
            Some(basis)
        };
        let intent = IssueBindingPauseIntent {
            command_id,
            binding_id: binding_id.clone(),
            paused,
            expected_revision,
            resume_basis,
        };
        intent.validate().map_err(|reason| anyhow!(reason))?;
        // Count this work before accepting it so an error after commit cannot let Native idle
        // out before it discovers the durable pending command and its outbox.
        self.schedule(true, Duration::ZERO);
        let accepted = self.admission.begin_transition(&binding_id, || {
            let stored = self
                .store
                .begin_issue_binding_pause(&self.workspace, &intent)?;
            let state = IssueBindingPauseState {
                binding_id: binding_id.clone(),
                revision: stored.accepted_revision,
                paused: stored
                    .receipt
                    .as_ref()
                    .map_or(current.paused, |receipt| receipt.paused),
                pending: stored.receipt.is_none().then(|| stored.intent.clone()),
            };
            Ok((stored, state))
        })?;
        self.refresh_schedule()?;
        Ok(accepted)
    }

    pub fn set_waker(&self, waker: Waker) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.waker = Some(waker.clone());
        }
        self.admission.set_waker(waker);
    }

    pub fn has_pending(&self) -> bool {
        self.runtime
            .lock()
            .map(|state| state.pending)
            .unwrap_or(true)
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.runtime
            .lock()
            .ok()
            .and_then(|state| state.next_deadline)
    }

    fn schedule(&self, pending: bool, delay: Duration) {
        let waker = self.runtime.lock().ok().and_then(|mut state| {
            state.pending = pending;
            state.next_deadline = pending.then(|| Instant::now() + delay);
            state.waker.clone()
        });
        if let Some(waker) = waker {
            waker();
        }
    }

    fn refresh_schedule(&self) -> Result<()> {
        let pending = !self
            .store
            .pending_issue_binding_pause_commands(&self.workspace, None, 1)?
            .is_empty()
            || !self
                .store
                .pending_issue_binding_pause_events(&self.workspace, 1)?
                .is_empty();
        self.schedule(pending, POLL_INTERVAL);
        Ok(())
    }

    pub fn pending_events(&self, limit: usize) -> Result<Vec<IssueBindingPauseEvent>> {
        let events = self
            .store
            .pending_issue_binding_pause_events(&self.workspace, limit)?;
        for event in &events {
            let current = self.state(&event.receipt.binding_id)?;
            self.admission.reconcile_completed_event(&current, event)?;
        }
        Ok(events)
    }

    pub fn acknowledge_event(&self, event: &IssueBindingPauseEvent) -> Result<()> {
        self.store
            .acknowledge_issue_binding_pause_event(&self.workspace, event)?;
        self.refresh_schedule()
    }
}
