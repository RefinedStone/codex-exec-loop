use crate::domain::planning::PostTurnContinuationPermit;

/// Long verification only observes cancellation. Bounded authority commits
/// participate in the existing continuation/epoch serialization boundary.
pub trait TaskVerificationContinuation: Send + Sync {
    fn is_current(&self) -> bool;
    fn with_current_commit(&self, operation: &mut dyn FnMut()) -> bool;
}

impl TaskVerificationContinuation for PostTurnContinuationPermit {
    fn is_current(&self) -> bool {
        PostTurnContinuationPermit::is_current(self)
    }

    fn with_current_commit(&self, operation: &mut dyn FnMut()) -> bool {
        self.with_current(operation).is_some()
    }
}

pub(crate) fn with_verification_commit<T>(
    continuation: &dyn TaskVerificationContinuation,
    operation: impl FnOnce() -> T,
) -> Option<T> {
    let mut operation = Some(operation);
    let mut value = None;
    let admitted = continuation.with_current_commit(&mut || {
        if let Some(operation) = operation.take() {
            value = Some(operation());
        }
    });
    admitted.then_some(value).flatten()
}
