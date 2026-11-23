//! Durable delegation state and routing decisions.

pub(crate) mod policy;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const ISSUE_DRAFT_REVIEW_FEATURE: &str = "issue_draft_review";
pub(crate) const ISSUE_ASSESSMENT_FEATURE: &str = "issue_assessment";

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
    Expired,
}

impl State {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Expired
        )
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<Attempt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<Fallback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<i64>,
    #[serde(default)]
    pub session: ChatSession,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_backend: Option<String>,
    #[serde(default)]
    pub can_cancel: bool,
    #[serde(default)]
    pub can_retry: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatSession {
    #[default]
    New,
    Continue {
        chat_url: String,
    },
}

impl ChatSession {
    pub(crate) fn selected_url(&self) -> Option<&str> {
        match self {
            Self::New => None,
            Self::Continue { chat_url } => Some(chat_url),
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.selected_url().is_none_or(|url| {
            url.strip_prefix("https://chatgpt.com/c/")
                .is_some_and(|id| {
                    id.len() == 36
                        && id.bytes().enumerate().all(|(index, byte)| {
                            if matches!(index, 8 | 13 | 18 | 23) {
                                byte == b'-'
                            } else {
                                byte.is_ascii_hexdigit()
                            }
                        })
                })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MyTurn {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Unavailable,
    Quota,
    Transient,
    Interrupted,
    NeedsHuman,
    Rejected,
    ResultInvalid,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    #[default]
    ReadOnly,
    Idempotent,
    SideEffecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Browser,
    Stream,
    Callback,
    Coordinator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub source: EvidenceSource,
    pub at: i64,
    pub delivery: Delivery,
    pub phase: Phase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Attempt {
    pub id: String,
    pub backend: String,
    pub backend_kind: String,
    pub model: Option<String>,
    pub delivery: Delivery,
    pub phase: Phase,
    pub reason: Option<EndReason>,
    pub retry_at: Option<i64>,
    pub observations: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Recovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumes: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    None,
    InProgress,
    Exhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Fallback {
    pub from: String,
    pub reason: EndReason,
    pub to: String,
    pub at: i64,
}

impl Attempt {
    pub(crate) fn record(&mut self, evidence: Evidence) {
        self.delivery = evidence.delivery;
        self.phase = evidence.phase;
        self.observations.push(evidence);
        if self.observations.len() > 16 {
            self.observations.remove(0);
        }
    }
}

pub(crate) fn can_fall_back(
    effect: Effect,
    delivery: Delivery,
    recovery_exhausted: bool,
    previous_stopped: bool,
) -> bool {
    match delivery {
        Delivery::NotSent => true,
        Delivery::Sending | Delivery::Uncertain => false,
        Delivery::Accepted | Delivery::Observed => match effect {
            Effect::ReadOnly => recovery_exhausted,
            Effect::Idempotent => recovery_exhausted && previous_stopped,
            Effect::SideEffecting => false,
        },
    }
}

pub(crate) fn select_backend<T>(
    choices: impl IntoIterator<Item = (T, bool, Option<i64>)>,
    now: i64,
) -> Option<T> {
    choices
        .into_iter()
        .find_map(|(backend, ready, closed_until)| {
            (ready && closed_until.is_none_or(|until| now >= until)).then_some(backend)
        })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NextAction {
    Expire,
    Stop,
    Select,
    Observe,
    None,
}

pub(crate) fn next_action(job: &Delegation, stop_pending: bool, now: i64) -> NextAction {
    if stop_pending {
        NextAction::Stop
    } else if job.state.terminal() {
        NextAction::None
    } else if now >= job.effective_deadline() {
        NextAction::Expire
    } else if job.state == State::Queued && !job.outcome_unknown {
        NextAction::Select
    } else {
        NextAction::Observe
    }
}

impl Delegation {
    pub(crate) fn effective_deadline(&self) -> i64 {
        self.deadline
            .unwrap_or_else(|| self.created_at.saturating_add(policy::DEFAULT_DEADLINE_MS))
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatBackendStatus {
    #[serde(flatten)]
    pub status: BackendStatus,
    #[serde(default, alias = "accountMatch")]
    pub account_match: AccountMatch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<TunnelStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit: Option<Circuit>,
}

impl ChatBackendStatus {
    pub(crate) fn unavailable(reason: &str) -> Self {
        Self {
            status: BackendStatus {
                state: BackendState::Unavailable,
                reason: Some(reason.into()),
            },
            account_match: AccountMatch::Unknown,
            tunnel: None,
            circuit: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountMatch {
    #[default]
    Unknown,
    Match,
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Circuit {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TunnelStatus {
    pub host: bool,
    pub manual: Option<bool>,
    pub readiness: Option<TunnelReadiness>,
    pub install: Option<TunnelInstall>,
    pub connection: Option<TunnelConnection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TunnelReadiness {
    Unknown,
    Ready,
    LoginRequired,
    Challenge,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TunnelInstall {
    Ready,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TunnelConnection {
    Connected,
    Offline,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodexBackendStatus {
    #[serde(flatten)]
    pub status: BackendStatus,
    pub local_reads: bool,
    pub verified_text_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit: Option<Circuit>,
}

impl CodexBackendStatus {
    pub(crate) fn from_status(status: BackendStatus, verified_text_only: bool) -> Self {
        Self {
            status,
            local_reads: true,
            verified_text_only,
            circuit: None,
        }
    }

    pub(crate) fn gateway_unavailable() -> Self {
        Self::from_status(
            BackendStatus {
                state: BackendState::Unavailable,
                reason: Some("gateway_unavailable".into()),
            },
            false,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendState {
    Ready,
    Unavailable,
}
