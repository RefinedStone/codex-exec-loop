//! 작업 지시별 자동 실행 소유권과 승인 세대다. 이슈나 UI에 의존하지 않는다.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// Immutable lineage and the execution epoch captured before delayed worker preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkerActionIdentity {
    pub work_order_id: String,
    pub origin: String,
    pub authorization_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanningWorkControl {
    pub held: bool,
    pub taken_over: bool,
    pub cancelled: bool,
    pub tracked: bool,
    pub authorization_epoch: u64,
}

impl Default for PlanningWorkControl {
    fn default() -> Self {
        Self {
            held: false,
            taken_over: false,
            cancelled: false,
            tracked: true,
            authorization_epoch: 0,
        }
    }
}

impl PlanningWorkControl {
    pub fn allows_automation(&self) -> bool {
        !self.held && !self.taken_over && !self.cancelled && self.tracked
    }

    pub fn apply(&self, action: PlanningWorkControlAction) -> Result<Self> {
        let mut next = self.clone();
        match action {
            PlanningWorkControlAction::Hold => next.held = true,
            PlanningWorkControlAction::Release => next.held = false,
            PlanningWorkControlAction::TakeOver => next.taken_over = true,
            PlanningWorkControlAction::Cancel => next.cancelled = true,
            PlanningWorkControlAction::Track => next.tracked = true,
            PlanningWorkControlAction::Untrack => next.tracked = false,
            PlanningWorkControlAction::Retriage => {}
        }
        if next != *self || action == PlanningWorkControlAction::Retriage {
            next.authorization_epoch = self
                .authorization_epoch
                .checked_add(1)
                .filter(|epoch| *epoch <= i64::MAX as u64)
                .ok_or_else(|| anyhow::anyhow!("work authorization epoch exhausted"))?;
        }
        Ok(next)
    }

    pub fn require_epoch(&self, epoch: u64) -> Result<()> {
        if !self.allows_automation() || self.authorization_epoch != epoch {
            bail!("permit_stale: work control or authorization epoch changed");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningWorkControlAction {
    Hold,
    Release,
    TakeOver,
    Cancel,
    Track,
    Untrack,
    Retriage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanningWorkInstruction {
    pub command_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkInstructionAdmission {
    Queued { changed: bool },
    Active,
}

/// A lease generation can start one worker only, including retries before TurnStarted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkOrderLaunchError {
    AlreadyClaimed,
}

impl std::fmt::Display for PlanningWorkOrderLaunchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyClaimed => formatter
                .write_str("work-order launch was already claimed for this lease generation"),
        }
    }
}

impl std::error::Error for PlanningWorkOrderLaunchError {}

pub trait PlanningWorkControlRepositoryPort: Send + Sync {
    /// The limit applies to distinct tasks that claim their first launch. A resumed task keeps
    /// its original attempt, and changing the limit never interrupts an already running turn.
    fn set_work_order_execution_limit(
        &self,
        workspace: &str,
        work_order_id: &str,
        max_attempts: u32,
    ) -> Result<()>;

    fn started_work_order_task_ids(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Vec<String>>;

    fn load_work_order_control(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<PlanningWorkControl>;
    /// The command outcome commits atomically with the control effect. Absence
    /// proves there was no effect; a read failure does not.
    fn load_work_order_control_command(
        &self,
        _workspace: &str,
        _work_order_id: &str,
        _command_id: &str,
        _action: PlanningWorkControlAction,
    ) -> Result<Option<PlanningWorkControl>> {
        bail!("work control command history is unavailable")
    }
    /// 신규 작업 등록 전에도 안정 work_order_id를 예약할 수 있다.
    /// 같은 command_id 재시도는 당시 결과를 돌려주고, 다른 내용으로 재사용하면 거절한다.
    fn apply_work_order_control(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
        action: PlanningWorkControlAction,
    ) -> Result<PlanningWorkControl>;
    fn apply_confirmed_work_order_control(
        &self,
        _workspace: &str,
        _request: &ConfirmedWorkControl<'_>,
    ) -> Result<Option<PlanningWorkControl>> {
        bail!("confirmed work control is unavailable")
    }
    fn append_work_order_instruction(
        &self,
        workspace: &str,
        work_order_id: &str,
        instruction: &PlanningWorkInstruction,
    ) -> Result<PlanningWorkInstructionAdmission>;
    fn load_work_order_instructions(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Vec<PlanningWorkInstruction>>;
    fn work_order_cleanup_allowed_for_task(&self, workspace: &str, task_id: &str) -> Result<bool>;
}

pub struct ConfirmedWorkControl<'a> {
    pub work_order_id: &'a str,
    pub command_id: &'a str,
    pub action: PlanningWorkControlAction,
    pub expected_epoch: u64,
    pub authorize: &'a dyn Fn() -> Result<bool>,
}
