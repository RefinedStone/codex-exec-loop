//! Compatibility path for independent host verification facts.
#[cfg(test)]
pub(crate) use akra_verification::verification::TaskVerificationStream;
pub(crate) use akra_verification::verification::validate_time;
pub use akra_verification::verification::{
    TaskVerificationAttempt, TaskVerificationCurrentState, TaskVerificationExecutionManifest,
    TaskVerificationExecutionOutcome, TaskVerificationNotRunReason, TaskVerificationObservation,
    TaskVerificationPhase, TaskVerificationReceipt, TaskVerificationRecipe, TaskVerificationResult,
};
