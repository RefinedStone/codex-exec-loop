//! The executor's fresh account quota decision, independent of Start authority.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAccountQuota {
    Allowed,
    Exhausted,
    Unknown,
}

pub trait IssueAccountQuotaPort: Send + Sync {
    /// Perform exactly one current account quota read. Never start a turn or use a cached update.
    fn read(&self) -> IssueAccountQuota;
}

pub use crate::domain::issue_automation::collection::{
    IssueQuotaHold, IssueQuotaLane, IssueQuotaWait,
};

#[derive(Debug, Clone, Copy)]
pub struct IssueQuotaWaitChange {
    pub lane: IssueQuotaLane,
    pub expected_revision: u64,
    pub hold: Option<IssueQuotaHold>,
}
