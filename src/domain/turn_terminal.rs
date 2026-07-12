use crate::domain::text::compact_whitespace_detail;

const ERROR_MESSAGE_MAX_LEN: usize = 2_048;
const ERROR_DETAIL_MAX_LEN: usize = 4_096;
const PROTOCOL_VALUE_MAX_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurnError {
    pub message: String,
    pub additional_details: Option<String>,
    pub info: Option<ConversationTurnErrorInfo>,
}

impl ConversationTurnError {
    pub fn new(
        message: impl AsRef<str>,
        additional_details: Option<impl AsRef<str>>,
        info: Option<ConversationTurnErrorInfo>,
    ) -> Self {
        Self {
            message: bounded_text(message.as_ref(), ERROR_MESSAGE_MAX_LEN),
            additional_details: additional_details
                .map(|details| bounded_text(details.as_ref(), ERROR_DETAIL_MAX_LEN)),
            info,
        }
    }

    pub fn summary(&self) -> String {
        bounded_text(&self.message, ERROR_MESSAGE_MAX_LEN)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationTurnErrorInfo {
    ContextWindowExceeded,
    SessionBudgetExceeded,
    UsageLimitExceeded,
    ServerOverloaded,
    CyberPolicy,
    InternalServerError,
    Unauthorized,
    BadRequest,
    ThreadRollbackFailed,
    SandboxError,
    Other,
    HttpConnectionFailed { http_status_code: Option<u16> },
    ResponseStreamConnectionFailed { http_status_code: Option<u16> },
    ResponseStreamDisconnected { http_status_code: Option<u16> },
    ResponseTooManyFailedAttempts { http_status_code: Option<u16> },
    ActiveTurnNotSteerable { turn_kind: String },
    Unknown(String),
}

impl ConversationTurnErrorInfo {
    pub fn active_turn_not_steerable(turn_kind: impl AsRef<str>) -> Self {
        Self::ActiveTurnNotSteerable {
            turn_kind: bounded_protocol_value(turn_kind.as_ref()),
        }
    }

    pub fn unknown(value: impl AsRef<str>) -> Self {
        Self::Unknown(bounded_protocol_value(value.as_ref()))
    }

    pub fn label(&self) -> &str {
        match self {
            Self::ContextWindowExceeded => "contextWindowExceeded",
            Self::SessionBudgetExceeded => "sessionBudgetExceeded",
            Self::UsageLimitExceeded => "usageLimitExceeded",
            Self::ServerOverloaded => "serverOverloaded",
            Self::CyberPolicy => "cyberPolicy",
            Self::InternalServerError => "internalServerError",
            Self::Unauthorized => "unauthorized",
            Self::BadRequest => "badRequest",
            Self::ThreadRollbackFailed => "threadRollbackFailed",
            Self::SandboxError => "sandboxError",
            Self::Other => "other",
            Self::HttpConnectionFailed { .. } => "httpConnectionFailed",
            Self::ResponseStreamConnectionFailed { .. } => "responseStreamConnectionFailed",
            Self::ResponseStreamDisconnected { .. } => "responseStreamDisconnected",
            Self::ResponseTooManyFailedAttempts { .. } => "responseTooManyFailedAttempts",
            Self::ActiveTurnNotSteerable { .. } => "activeTurnNotSteerable",
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConversationTurnItemsView {
    NotLoaded,
    Summary,
    #[default]
    Full,
    Unknown(String),
}

impl ConversationTurnItemsView {
    pub fn unknown(value: impl AsRef<str>) -> Self {
        Self::Unknown(bounded_protocol_value(value.as_ref()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationTurnTerminalUncertainty {
    NonRetryErrorGraceExpired,
    InProgressStatus,
    UnknownStatus(String),
    MissingFailedError,
    StatusErrorContradiction { status: String },
    MissingRequiredIdentity { field: String },
    ProtocolInconsistency(String),
}

impl ConversationTurnTerminalUncertainty {
    pub fn unknown_status(status: impl AsRef<str>) -> Self {
        Self::UnknownStatus(bounded_protocol_value(status.as_ref()))
    }

    pub fn status_error_contradiction(status: impl AsRef<str>) -> Self {
        Self::StatusErrorContradiction {
            status: bounded_protocol_value(status.as_ref()),
        }
    }

    pub fn missing_required_identity(field: impl AsRef<str>) -> Self {
        Self::MissingRequiredIdentity {
            field: bounded_protocol_value(field.as_ref()),
        }
    }

    pub fn protocol_inconsistency(detail: impl AsRef<str>) -> Self {
        Self::ProtocolInconsistency(bounded_protocol_value(detail.as_ref()))
    }

    pub fn summary(&self) -> String {
        match self {
            Self::NonRetryErrorGraceExpired => {
                "non-retry error grace expired without a final turn".to_string()
            }
            Self::InProgressStatus => "terminal notification reported inProgress".to_string(),
            Self::UnknownStatus(status) => format!("unknown terminal status: {status}"),
            Self::MissingFailedError => "failed terminal status omitted its error".to_string(),
            Self::StatusErrorContradiction { status } => {
                format!("terminal status {status} unexpectedly included an error")
            }
            Self::MissingRequiredIdentity { field } => {
                format!("terminal notification omitted required identity: {field}")
            }
            Self::ProtocolInconsistency(detail) => detail.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationTurnTerminalOutcome {
    Completed,
    Interrupted,
    Failed {
        error: ConversationTurnError,
    },
    Unknown {
        reason: ConversationTurnTerminalUncertainty,
        observed_error: Option<ConversationTurnError>,
    },
}

impl ConversationTurnTerminalOutcome {
    pub const fn status_label(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Failed { .. } => "failed",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub fn observed_error(&self) -> Option<&ConversationTurnError> {
        match self {
            Self::Failed { error } => Some(error),
            Self::Unknown { observed_error, .. } => observed_error.as_ref(),
            Self::Completed | Self::Interrupted => None,
        }
    }

    pub fn status_error_summary(&self) -> String {
        match self {
            Self::Completed => "completed".to_string(),
            Self::Interrupted => "interrupted".to_string(),
            Self::Failed { error } => format!("failed: {}", error.summary()),
            Self::Unknown {
                reason,
                observed_error,
            } => match observed_error {
                Some(error) => format!("unknown: {}; {}", reason.summary(), error.summary()),
                None => format!("unknown: {}", reason.summary()),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationTurnApplicationDeliveryFailure {
    Full,
    DeadlineExceeded,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConversationTurnApplicationDelivery {
    #[default]
    Pending,
    Confirmed,
    Unconfirmed(ConversationTurnApplicationDeliveryFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversationTurnObservations {
    pub changed_planning_file_paths: Vec<String>,
}

impl ConversationTurnObservations {
    pub fn new(changed_planning_file_paths: Vec<String>) -> Self {
        Self {
            changed_planning_file_paths,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurnTerminalReceipt {
    pub thread_id: String,
    pub turn_id: String,
    pub outcome: ConversationTurnTerminalOutcome,
    pub items_view: ConversationTurnItemsView,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub observations: ConversationTurnObservations,
    pub application_delivery: ConversationTurnApplicationDelivery,
}

impl ConversationTurnTerminalReceipt {
    pub fn new(
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        outcome: ConversationTurnTerminalOutcome,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
            outcome,
            items_view: ConversationTurnItemsView::default(),
            started_at: None,
            completed_at: None,
            duration_ms: None,
            observations: ConversationTurnObservations::default(),
            application_delivery: ConversationTurnApplicationDelivery::Pending,
        }
    }

    pub fn completed(
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        changed_planning_file_paths: Vec<String>,
    ) -> Self {
        Self::new(
            thread_id,
            turn_id,
            ConversationTurnTerminalOutcome::Completed,
        )
        .with_observations(ConversationTurnObservations::new(
            changed_planning_file_paths,
        ))
    }

    pub fn with_turn_metadata(
        mut self,
        items_view: ConversationTurnItemsView,
        started_at: Option<i64>,
        completed_at: Option<i64>,
        duration_ms: Option<i64>,
    ) -> Self {
        self.items_view = items_view;
        self.started_at = started_at;
        self.completed_at = completed_at;
        self.duration_ms = duration_ms;
        self
    }

    pub fn with_observations(mut self, observations: ConversationTurnObservations) -> Self {
        self.observations = observations;
        self
    }

    pub fn with_application_delivery(
        mut self,
        application_delivery: ConversationTurnApplicationDelivery,
    ) -> Self {
        self.application_delivery = application_delivery;
        self
    }

    pub fn is_completed_and_confirmed(&self) -> bool {
        matches!(&self.outcome, ConversationTurnTerminalOutcome::Completed)
            && self.application_delivery == ConversationTurnApplicationDelivery::Confirmed
    }

    pub fn status_error_summary(&self) -> String {
        let outcome = self.outcome.status_error_summary();
        match self.application_delivery {
            ConversationTurnApplicationDelivery::Confirmed => outcome,
            ConversationTurnApplicationDelivery::Pending => {
                format!("recovery pending: upstream {outcome}; application delivery pending")
            }
            ConversationTurnApplicationDelivery::Unconfirmed(reason) => format!(
                "recovery pending: upstream {outcome}; application delivery unconfirmed ({})",
                reason.label()
            ),
        }
    }
}

impl ConversationTurnApplicationDeliveryFailure {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::DeadlineExceeded => "deadline exceeded",
            Self::Disconnected => "disconnected",
        }
    }
}

fn bounded_protocol_value(value: &str) -> String {
    bounded_text(value, PROTOCOL_VALUE_MAX_LEN)
}

fn bounded_text(value: &str, max_len: usize) -> String {
    compact_whitespace_detail(value, max_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_receipt_requires_confirmed_application_delivery() {
        let receipt = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            vec!["docs/plan.md".to_string()],
        );
        assert!(!receipt.is_completed_and_confirmed());

        let confirmed =
            receipt.with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        assert!(confirmed.is_completed_and_confirmed());
        assert_eq!(
            confirmed.observations.changed_planning_file_paths,
            vec!["docs/plan.md".to_string()]
        );
    }

    #[test]
    fn unknown_protocol_values_are_bounded() {
        let info = ConversationTurnErrorInfo::unknown("x".repeat(PROTOCOL_VALUE_MAX_LEN + 50));
        let ConversationTurnErrorInfo::Unknown(value) = info else {
            panic!("expected unknown error info");
        };
        assert_eq!(value.chars().count(), PROTOCOL_VALUE_MAX_LEN);
        assert!(value.ends_with("..."));
    }

    #[test]
    fn observations_cannot_promote_a_non_completed_outcome() {
        let receipt = ConversationTurnTerminalReceipt::new(
            "thread-1",
            "turn-1",
            ConversationTurnTerminalOutcome::Interrupted,
        )
        .with_observations(ConversationTurnObservations::new(vec![
            "docs/plan.md".to_string(),
        ]))
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        assert!(!receipt.is_completed_and_confirmed());
        assert_eq!(receipt.status_error_summary(), "interrupted");
    }

    #[test]
    fn status_error_summary_preserves_unknown_reason_and_observed_error() {
        let outcome = ConversationTurnTerminalOutcome::Unknown {
            reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
            observed_error: Some(ConversationTurnError::new(
                "server disconnected",
                None::<&str>,
                Some(ConversationTurnErrorInfo::ResponseStreamDisconnected {
                    http_status_code: Some(502),
                }),
            )),
        };

        assert_eq!(
            outcome.status_error_summary(),
            "unknown: non-retry error grace expired without a final turn; server disconnected"
        );
    }

    #[test]
    fn unconfirmed_delivery_summary_preserves_upstream_outcome_and_recovery_state() {
        let receipt = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            vec!["docs/plan.md".to_string()],
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Unconfirmed(
            ConversationTurnApplicationDeliveryFailure::Disconnected,
        ));

        assert_eq!(
            receipt.status_error_summary(),
            "recovery pending: upstream completed; application delivery unconfirmed (disconnected)"
        );
        assert!(!receipt.is_completed_and_confirmed());
    }
}
