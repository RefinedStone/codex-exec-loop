use serde::{Deserialize, Serialize};

/// Repository-local policy for the durable PR validation scheduler.
///
/// `observe` records provider evidence without admitting Planning Queue work. `remediate` is the
/// only mode that may create a correlated task. `off` retains all durable history while stopping
/// new provider observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationSchedulerMode {
    Off,
    #[default]
    Observe,
    Remediate,
}

impl PrValidationSchedulerMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "off" => Ok(Self::Off),
            "observe" => Ok(Self::Observe),
            "remediate" => Ok(Self::Remediate),
            _ => Err(
                "akra.prValidationMode must be exactly `off`, `observe`, or `remediate`"
                    .to_string(),
            ),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Observe => "observe",
            Self::Remediate => "remediate",
        }
    }

    pub const fn admits_remediation(self) -> bool {
        matches!(self, Self::Remediate)
    }
}

/// Stable scheduler-facing error vocabulary. Provider details and credentials never enter this
/// value; Admin and later read models can safely project it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrValidationPollErrorClass {
    RetryableProvider,
    AuthenticationBlocked,
    PolicyBlocked,
    IdentityFailed,
    AdmissionRetryable,
    IntegrityFailed,
}

impl PrValidationPollErrorClass {
    pub const fn label(self) -> &'static str {
        match self {
            Self::RetryableProvider => "retryable_provider",
            Self::AuthenticationBlocked => "authentication_blocked",
            Self::PolicyBlocked => "policy_blocked",
            Self::IdentityFailed => "identity_failed",
            Self::AdmissionRetryable => "admission_retryable",
            Self::IntegrityFailed => "integrity_failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "retryable_provider" => Ok(Self::RetryableProvider),
            "authentication_blocked" => Ok(Self::AuthenticationBlocked),
            "policy_blocked" => Ok(Self::PolicyBlocked),
            "identity_failed" => Ok(Self::IdentityFailed),
            "admission_retryable" => Ok(Self::AdmissionRetryable),
            "integrity_failed" => Ok(Self::IntegrityFailed),
            _ => Err(format!("unknown PR validation poll error class `{value}`")),
        }
    }

    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RetryableProvider | Self::AuthenticationBlocked | Self::AdmissionRetryable
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_mode_is_strict_and_observe_is_the_safe_default() {
        assert_eq!(
            PrValidationSchedulerMode::default(),
            PrValidationSchedulerMode::Observe
        );
        assert_eq!(
            PrValidationSchedulerMode::parse("remediate").unwrap(),
            PrValidationSchedulerMode::Remediate
        );
        assert!(PrValidationSchedulerMode::parse("ON").is_err());
        assert!(!PrValidationSchedulerMode::Observe.admits_remediation());
    }

    #[test]
    fn poll_error_storage_labels_round_trip() {
        for class in [
            PrValidationPollErrorClass::RetryableProvider,
            PrValidationPollErrorClass::AuthenticationBlocked,
            PrValidationPollErrorClass::PolicyBlocked,
            PrValidationPollErrorClass::IdentityFailed,
            PrValidationPollErrorClass::AdmissionRetryable,
            PrValidationPollErrorClass::IntegrityFailed,
        ] {
            assert_eq!(
                PrValidationPollErrorClass::parse(class.label()).unwrap(),
                class
            );
        }
    }
}
