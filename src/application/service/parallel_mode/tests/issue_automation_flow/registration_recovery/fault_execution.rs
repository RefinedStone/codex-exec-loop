use anyhow::{Result, bail};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::application::port::outbound::issue_work_execution_port::*;
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::execution::IssueWorkExecutionObservation;
use crate::domain::issue_automation::work_control::{
    IssueWorkControl, IssueWorkControlAction, IssueWorkInstruction, IssueWorkInstructionAdmission,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum SubmissionFault {
    BeforeCommit,
    AfterCommit,
}

#[derive(Default)]
pub(super) struct SubmissionProbe {
    pub attempts: AtomicUsize,
    pub committed: AtomicUsize,
}

/// Only the response boundary is interrupted; registration and every other operation use
/// the real Akra adapter and its SQLite planning authority.
pub(super) struct InterruptedSubmission {
    inner: Arc<dyn IssueWorkExecutionPort>,
    fault: SubmissionFault,
    injected: AtomicBool,
    probe: Arc<SubmissionProbe>,
}

impl InterruptedSubmission {
    pub fn new(
        inner: Arc<dyn IssueWorkExecutionPort>,
        fault: SubmissionFault,
        probe: Arc<SubmissionProbe>,
    ) -> Self {
        Self {
            inner,
            fault,
            injected: AtomicBool::new(false),
            probe,
        }
    }
}

impl IssueWorkExecutionPort for InterruptedSubmission {
    fn submit(&self, workspace: &str, order: &IssueWorkOrder) -> Result<Vec<String>> {
        self.probe.attempts.fetch_add(1, Ordering::SeqCst);
        let inject = !self.injected.swap(true, Ordering::SeqCst);
        if inject && matches!(self.fault, SubmissionFault::BeforeCommit) {
            bail!("injected interruption before the planning registration");
        }
        let task_ids = self.inner.submit(workspace, order)?;
        self.probe.committed.fetch_add(1, Ordering::SeqCst);
        if inject && matches!(self.fault, SubmissionFault::AfterCommit) {
            bail!("injected acknowledgement loss after the planning registration committed");
        }
        Ok(task_ids)
    }

    fn recover_execution(
        &self,
        workspace: &str,
        request: &IssueWorkRecoveryRequest,
    ) -> Result<IssueWorkRecoveryReceipt> {
        self.inner.recover_execution(workspace, request)
    }

    fn answer_question(
        &self,
        workspace: &str,
        request: &IssueWorkAnswerRequest,
    ) -> Result<IssueWorkAnswerReceipt> {
        self.inner.answer_question(workspace, request)
    }

    fn question_answer_receipt(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
    ) -> Result<Option<IssueWorkAnswerReceipt>> {
        self.inner
            .question_answer_receipt(workspace, work_order_id, command_id)
    }

    fn execution_limit(
        &self,
        workspace: &str,
        work_order_id: &str,
        max_attempts: u32,
    ) -> Result<()> {
        self.inner
            .execution_limit(workspace, work_order_id, max_attempts)
    }

    fn permit_gate_revision(&self, workspace: &str, work_order_id: &str) -> Result<u64> {
        self.inner.permit_gate_revision(workspace, work_order_id)
    }

    fn observe_execution(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Option<IssueWorkExecutionObservation>> {
        self.inner.observe_execution(workspace, work_order_id)
    }

    fn control(&self, workspace: &str, work_order_id: &str) -> Result<IssueWorkControl> {
        self.inner.control(workspace, work_order_id)
    }

    fn apply_control(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
        action: IssueWorkControlAction,
    ) -> Result<IssueWorkControl> {
        self.inner
            .apply_control(workspace, work_order_id, command_id, action)
    }

    fn queue_instruction(
        &self,
        workspace: &str,
        work_order_id: &str,
        instruction: &IssueWorkInstruction,
    ) -> Result<IssueWorkInstructionAdmission> {
        self.inner
            .queue_instruction(workspace, work_order_id, instruction)
    }

    fn has_active_lease(&self, workspace: &str, work_order_id: &str) -> Result<bool> {
        self.inner.has_active_lease(workspace, work_order_id)
    }

    fn conversation_thread_id(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Option<String>> {
        self.inner.conversation_thread_id(workspace, work_order_id)
    }

    fn snapshot(&self, workspace: &str, work_order_id: &str) -> Result<IssueWorkExecutionSnapshot> {
        self.inner.snapshot(workspace, work_order_id)
    }

    fn task_ids(&self, workspace: &str, work_order_id: &str) -> Result<Vec<String>> {
        self.inner.task_ids(workspace, work_order_id)
    }

    fn grant(&self, workspace: &str, request: &IssueWorkPermitRequest) -> Result<()> {
        self.inner.grant(workspace, request)
    }

    fn revoke(&self, workspace: &str, work_order_id: &str) -> Result<()> {
        self.inner.revoke(workspace, work_order_id)
    }
}
