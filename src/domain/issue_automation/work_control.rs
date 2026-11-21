//! 이슈 작업의 실행 소유권과 개입 명령이다. 특정 실행기의 저장·런타임 타입을 사용하지 않는다.
use super::MAX_ISSUE_AUTOMATION_REVISION;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkControl {
    pub held: bool,
    pub taken_over: bool,
    pub cancelled: bool,
    pub tracked: bool,
    pub authorization_epoch: u64,
}

impl Default for IssueWorkControl {
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

impl IssueWorkControl {
    pub fn allows_automation(&self) -> bool {
        !self.held && !self.taken_over && !self.cancelled && self.tracked
    }

    pub fn apply(&self, action: IssueWorkControlAction) -> Result<Self, IssueWorkControlError> {
        let mut next = self.clone();
        match action {
            IssueWorkControlAction::Hold => next.held = true,
            IssueWorkControlAction::Release => next.held = false,
            IssueWorkControlAction::TakeOver => next.taken_over = true,
            IssueWorkControlAction::Cancel => next.cancelled = true,
            IssueWorkControlAction::Track => next.tracked = true,
            IssueWorkControlAction::Untrack => next.tracked = false,
            IssueWorkControlAction::Retriage => {}
        }
        if next != *self || action == IssueWorkControlAction::Retriage {
            next.authorization_epoch = self
                .authorization_epoch
                .checked_add(1)
                .filter(|epoch| *epoch <= MAX_ISSUE_AUTOMATION_REVISION)
                .ok_or(IssueWorkControlError::AuthorizationEpochExhausted)?;
        }
        Ok(next)
    }

    pub fn require_epoch(&self, epoch: u64) -> Result<(), IssueWorkControlError> {
        if !self.allows_automation() || self.authorization_epoch != epoch {
            return Err(IssueWorkControlError::StaleAuthorization);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueWorkControlAction {
    Hold,
    Release,
    TakeOver,
    Cancel,
    Track,
    Untrack,
    Retriage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkInstruction {
    pub command_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueWorkInstructionAdmission {
    Queued { changed: bool },
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueWorkControlError {
    AuthorizationEpochExhausted,
    StaleAuthorization,
}

impl std::fmt::Display for IssueWorkControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthorizationEpochExhausted => {
                formatter.write_str("work authorization epoch exhausted")
            }
            Self::StaleAuthorization => {
                formatter.write_str("permit_stale: work control or authorization epoch changed")
            }
        }
    }
}

impl std::error::Error for IssueWorkControlError {}
