//! 기존 검증 종료를 변경하지 않고 후속 기록과 영수증을 함께 등록한다.
use serde::{Deserialize, Serialize};

use super::PlanningWorkOrderRecord;
use crate::domain::parallel_mode::PrValidationRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningValidationRecoveryKind {
    ObserveAgain,
    QueueRecovery,
}

/// observed_*는 새 등록의 CAS 입력이며 완료 영수증의 불변 payload에는 포함하지 않는다.
#[derive(Debug, Clone, Copy)]
pub struct PlanningValidationRecoveryRequest<'a> {
    pub command_id: &'a str,
    pub work_order_id: &'a str,
    pub terminal_record_id: &'a str,
    pub kind: PlanningValidationRecoveryKind,
    pub authorization_epoch: u64,
    pub gate_revision: u64,
    pub policy_version: &'a str,
    pub original: &'a PrValidationRecord,
    pub observed_work_order: &'a PlanningWorkOrderRecord,
    pub observed_planning_revision: i64,
    pub is_current_terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningValidationRecoveryReceipt {
    pub command_id: String,
    pub terminal_record_id: String,
    pub outcome: PlanningValidationRecoveryOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanningValidationRecoveryOutcome {
    Registered { validation_record_key: String },
    Blocked { reason: String },
}
