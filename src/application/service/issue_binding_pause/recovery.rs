use super::*;
use crate::domain::issue_automation::coordinator::IssueWorkCheckpoint;
use crate::domain::issue_automation::intervention::IssueInterventionReceipt;
use crate::domain::issue_automation::work_control::IssueWorkControlAction;

impl IssueBindingPauseService {
    /// 자동화 graph가 시작하기 전에 저장된 paused/pending 상태를 모두 닫아 둔다.
    /// 페이지 단위로 복구하며 기존 실행을 재개하거나 새 허가를 만들지 않는다.
    pub fn recover_before_runtime(&self) -> Result<()> {
        let result = (|| {
            let mut cursor = None;
            loop {
                let page = self.store.list_issue_binding_pause_states(
                    &self.workspace,
                    cursor.as_ref(),
                    ISSUE_BINDING_PAUSE_PAGE_LIMIT,
                )?;
                for state in &page {
                    self.admission.register_if_absent(state)?;
                }
                if page.len() < ISSUE_BINDING_PAUSE_PAGE_LIMIT {
                    break;
                }
                let next = page.last().expect("a full page").binding_id.clone();
                if cursor.as_ref() == Some(&next) {
                    bail!("issue binding restore cursor did not advance");
                }
                cursor = Some(next);
            }
            self.refresh_schedule()
        })();
        if result.is_err() {
            self.admission.close_all();
        }
        result
    }

    /// 한 tick에는 명령 8개, 명령마다 work 64개까지만 처리한다.
    pub fn poll(&self) -> Result<bool> {
        let _command_guard = match self.command_gate.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                bail!("issue binding pause command gate is unavailable")
            }
        };
        let result = self.poll_page();
        if result.is_err() {
            self.schedule(true, Duration::from_secs(1));
        }
        result
    }

    fn poll_page(&self) -> Result<bool> {
        let after = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("issue binding pause runtime is unavailable"))?
            .after_binding
            .clone();
        let commands =
            self.store
                .pending_issue_binding_pause_commands(&self.workspace, after.as_ref(), 8)?;
        let mut changed = false;
        let mut retry = false;
        for command in &commands {
            match self.process_command(command) {
                Ok(completed) => changed |= completed,
                Err(error) => {
                    retry = true;
                    tracing::warn!(binding_id = %command.intent.binding_id, error = %error,
                        "issue binding pause remains pending; unrelated bindings may continue");
                }
            }
        }
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| anyhow!("issue binding pause runtime is unavailable"))?;
            runtime.after_binding = if commands.len() < 8 {
                None
            } else {
                commands
                    .last()
                    .map(|command| command.intent.binding_id.clone())
            };
        }
        self.refresh_schedule()?;
        if retry && self.has_pending() {
            self.schedule(true, Duration::from_secs(1));
        }
        Ok(changed)
    }

    fn process_command(&self, command: &StoredIssueBindingPauseCommand) -> Result<bool> {
        let intent = &command.intent;
        intent.validate().map_err(|reason| anyhow!(reason))?;
        let current = self.state(&intent.binding_id)?;
        self.admission.register_if_absent(&current)?;
        if current.pending.as_ref() != Some(intent) || current.revision != command.accepted_revision
        {
            bail!("issue binding pause recovery command is stale");
        }
        let after = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("issue binding pause runtime is unavailable"))?
            .work_cursors
            .get(&intent.command_id)
            .cloned();
        let work = self.store.pending_issue_binding_pause_work(
            &self.workspace,
            intent,
            after.as_deref(),
            ISSUE_BINDING_PAUSE_PAGE_LIMIT,
        )?;
        for checkpoint in &work {
            self.fence_work(intent, checkpoint)?;
            self.runtime
                .lock()
                .map_err(|_| anyhow!("issue binding pause runtime is unavailable"))?
                .work_cursors
                .insert(
                    intent.command_id.clone(),
                    checkpoint.order.work_order_id.clone(),
                );
        }
        if work.len() == ISSUE_BINDING_PAUSE_PAGE_LIMIT {
            return Ok(false);
        }
        let finished = self.admission.finish_if_drained(
            &intent.binding_id,
            command.accepted_revision,
            || {
                // Current file policy is read while the binding remains closed. The storage
                // transaction independently rechecks binding/autonomy revisions and all fences.
                let completion = if intent.paused
                    || self.current_resume_basis(&intent.binding_id).ok().as_ref()
                        == intent.resume_basis.as_ref()
                {
                    IssueBindingPauseCompletion::Apply
                } else {
                    IssueBindingPauseCompletion::ResumeSuperseded
                };
                let stored =
                    self.store
                        .finish_issue_binding_pause(&self.workspace, intent, completion)?;
                let receipt = stored
                    .receipt
                    .as_ref()
                    .ok_or_else(|| anyhow!("issue binding completion has no receipt"))?;
                let state = IssueBindingPauseState {
                    binding_id: receipt.binding_id.clone(),
                    revision: receipt.revision,
                    paused: receipt.paused,
                    pending: None,
                };
                Ok((stored, state))
            },
        )?;
        if finished.is_some() {
            self.runtime
                .lock()
                .map_err(|_| anyhow!("issue binding pause runtime is unavailable"))?
                .work_cursors
                .remove(&intent.command_id);
        }
        Ok(finished.is_some())
    }

    fn fence_work(
        &self,
        intent: &IssueBindingPauseIntent,
        work: &IssueWorkCheckpoint,
    ) -> Result<()> {
        let pending = self
            .store
            .claim_issue_binding_pause_fence(&self.workspace, intent, work)?;
        if pending.receipt.is_some() {
            // A generic crash recoverer may have stored the owned Retriage receipt before
            // this service performed its explicit permit revocation.
            self.execution
                .revoke(&self.workspace, &work.order.work_order_id)?;
            return Ok(());
        }
        let request = pending.request;
        let control = self.execution.apply_control(
            &self.workspace,
            &work.order.work_order_id,
            &request.command_id,
            IssueWorkControlAction::Retriage,
        )?;
        self.execution
            .revoke(&self.workspace, &work.order.work_order_id)?;
        let receipt = IssueInterventionReceipt {
            issue: work.order.issue.clone(),
            command_id: request.command_id.clone(),
            action: "retriage_issue".into(),
            work_order_id: Some(work.order.work_order_id.clone()),
            authorization_epoch: control.authorization_epoch,
            changed: true,
            delivery: None,
            handoff: None,
        };
        if let Err(error) = self
            .store
            .finish_issue_intervention(&self.workspace, &receipt)
        {
            // Preserve the first receipt when another recoverer applied this same owned
            // command. Comparing every other field prevents accepting another work's fence.
            let previous = self
                .store
                .get_issue_intervention(&self.workspace, &request.command_id)?;
            if let Some(mut previous) = previous
                .filter(|previous| previous.request == request)
                .and_then(|previous| previous.receipt)
            {
                previous.changed = receipt.changed;
                if previous == receipt
                    && self
                        .execution
                        .control(&self.workspace, &work.order.work_order_id)?
                        == control
                {
                    return Ok(());
                }
            }
            return Err(error);
        }
        Ok(())
    }
}
