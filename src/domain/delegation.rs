//! State exposed by the text-only delegated chat ledger.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Dispatching,
    Running,
    Interrupted,
    AwaitingHuman,
    Succeeded,
    Failed,
    Cancelled,
}

impl State {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    NotSent,
    Sending,
    Accepted,
    Observed,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Working,
    Complete,
    Stopped,
    Failed,
    Disconnected,
    Blocked,
    Unreachable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Delegation {
    pub id: String,
    pub caller: String,
    pub state: State,
    pub delivery: Delivery,
    pub phase: Phase,
    pub reason: Option<String>,
    pub conversation_url: Option<String>,
    pub outcome_unknown: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Delegation {
    pub fn apply_observation(&mut self, delivery: Delivery, phase: Phase) {
        if delivery == Delivery::NotSent && phase == Phase::Unknown {
            return;
        }
        self.delivery = match (self.delivery, delivery) {
            (Delivery::Observed, Delivery::Accepted | Delivery::Sending | Delivery::NotSent) => {
                Delivery::Observed
            }
            (Delivery::Uncertain | Delivery::Accepted, Delivery::Sending | Delivery::NotSent) => {
                self.delivery
            }
            _ => delivery,
        };
        self.phase = phase;
        self.outcome_unknown = self.delivery == Delivery::Uncertain;
        self.state = match phase {
            Phase::Blocked => State::AwaitingHuman,
            Phase::Stopped | Phase::Failed if !self.outcome_unknown => State::Failed,
            Phase::Disconnected | Phase::Unreachable => State::Interrupted,
            Phase::Working | Phase::Complete
                if matches!(self.delivery, Delivery::Accepted | Delivery::Observed) =>
            {
                State::Running
            }
            _ if self.outcome_unknown => State::AwaitingHuman,
            _ => State::Dispatching,
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackendStatus {
    pub state: BackendState,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendState {
    Ready,
    Unavailable,
}
