use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAdminCommandAction {
    RetryNow,
    Pause,
    Resume,
    QueueRemediation,
    Acknowledge,
}

impl PrValidationAdminCommandAction {
    pub const fn key(self) -> &'static str {
        match self {
            Self::RetryNow => "retry_now",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::QueueRemediation => "queue_remediation",
            Self::Acknowledge => "acknowledge",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value.trim() {
            "retry_now" => Some(Self::RetryNow),
            "pause" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "queue_remediation" => Some(Self::QueueRemediation),
            "acknowledge" => Some(Self::Acknowledge),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationAdminCommandRequest {
    pub command_id: String,
    pub record_key: String,
    pub action: PrValidationAdminCommandAction,
    pub expected_revision: u64,
}

impl PrValidationAdminCommandRequest {
    pub fn new(
        command_id: impl Into<String>,
        record_key: impl Into<String>,
        action: PrValidationAdminCommandAction,
        expected_revision: u64,
    ) -> Result<Self, String> {
        let command_id = command_id.into().trim().to_string();
        let record_key = record_key.into().trim().to_string();
        if command_id.is_empty() || command_id.len() > 160 {
            return Err("validation command id must contain 1 to 160 bytes".to_string());
        }
        if record_key.is_empty() || record_key.len() > 200 {
            return Err("validation record key must contain 1 to 200 bytes".to_string());
        }
        Ok(Self {
            command_id,
            record_key,
            action,
            expected_revision,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAdminCommandState {
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationAdminCommandRejection {
    NotFound,
    StaleRevision,
    IdempotencyConflict,
    ObserveModeAdmission,
    InvalidState,
    RateLimitActive,
    NoActionableFinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrValidationAdminCommandResult {
    pub command_id: String,
    pub record_key: String,
    pub action: PrValidationAdminCommandAction,
    pub state: PrValidationAdminCommandState,
    pub rejection: Option<PrValidationAdminCommandRejection>,
    pub duplicate: bool,
    pub expected_revision: u64,
    pub observed_revision: Option<u64>,
    pub board_revision: i64,
    pub message: String,
    pub applied_at: String,
}

pub trait PrValidationCommandPort: Send + Sync {
    fn execute(
        &self,
        request: PrValidationAdminCommandRequest,
    ) -> Result<PrValidationAdminCommandResult>;
}
