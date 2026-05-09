use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModePoolResetPolicy {
    ProtectLive,
    ForceDisposable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModePoolResetRunId(pub String);

impl ParallelModePoolResetRunId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModePoolResetSlotAction {
    Reset,
    PreserveLive,
    SkipMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelModePoolResetSlotOutcome {
    Succeeded,
    Failed,
    Blocked,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModePoolResetSlotReport {
    pub slot_id: String,
    pub action: ParallelModePoolResetSlotAction,
    pub outcome: ParallelModePoolResetSlotOutcome,
    pub reason: String,
}

impl ParallelModePoolResetSlotReport {
    pub fn new(
        slot_id: impl Into<String>,
        action: ParallelModePoolResetSlotAction,
        outcome: ParallelModePoolResetSlotOutcome,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            slot_id: slot_id.into(),
            action,
            outcome,
            reason: reason.into(),
        }
    }

    pub fn reset_succeeded(&self) -> bool {
        self.action == ParallelModePoolResetSlotAction::Reset
            && self.outcome == ParallelModePoolResetSlotOutcome::Succeeded
    }

    pub fn blocked_live(&self) -> bool {
        self.action == ParallelModePoolResetSlotAction::PreserveLive
            && self.outcome == ParallelModePoolResetSlotOutcome::Blocked
    }

    pub fn reset_failed(&self) -> bool {
        self.action == ParallelModePoolResetSlotAction::Reset
            && self.outcome == ParallelModePoolResetSlotOutcome::Failed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelModePoolResetReport {
    pub run_id: ParallelModePoolResetRunId,
    pub policy: ParallelModePoolResetPolicy,
    pub slot_reports: Vec<ParallelModePoolResetSlotReport>,
    pub reset_session_keys: Vec<String>,
    pub reset_queue_item_ids: Vec<String>,
}

impl ParallelModePoolResetReport {
    pub fn new(run_id: ParallelModePoolResetRunId, policy: ParallelModePoolResetPolicy) -> Self {
        Self {
            run_id,
            policy,
            slot_reports: Vec::new(),
            reset_session_keys: Vec::new(),
            reset_queue_item_ids: Vec::new(),
        }
    }

    pub fn succeeded_reset_slot_ids(&self) -> Vec<String> {
        self.slot_reports
            .iter()
            .filter(|report| report.reset_succeeded())
            .map(|report| report.slot_id.clone())
            .collect()
    }

    pub fn succeeded_reset_slot_count(&self) -> usize {
        self.slot_reports
            .iter()
            .filter(|report| report.reset_succeeded())
            .count()
    }

    pub fn live_blocker_count(&self) -> usize {
        self.slot_reports
            .iter()
            .filter(|report| report.blocked_live())
            .count()
    }

    pub fn failed_reset_count(&self) -> usize {
        self.slot_reports
            .iter()
            .filter(|report| report.reset_failed())
            .count()
    }

    pub fn has_live_blockers(&self) -> bool {
        self.live_blocker_count() > 0
    }

    pub fn has_reset_failures(&self) -> bool {
        self.failed_reset_count() > 0
    }
}
