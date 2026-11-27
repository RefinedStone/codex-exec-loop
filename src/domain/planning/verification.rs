//! Compatibility path for independent host verification facts.
pub(crate) use akra_verification::verification::validate_time;
pub use akra_verification::verification::{
    MAX_TASK_VERIFICATION_STREAM_BYTES, TaskVerificationAttempt, TaskVerificationCurrentState,
    TaskVerificationExecutionManifest, TaskVerificationExecutionOutcome,
    TaskVerificationNotRunReason, TaskVerificationObservation, TaskVerificationPhase,
    TaskVerificationReceipt, TaskVerificationRecipe, TaskVerificationResult,
    TaskVerificationStream,
};
