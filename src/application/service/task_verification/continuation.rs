use crate::domain::planning::PostTurnContinuationPermit;

pub use akra_verification::continuation::TaskVerificationContinuation;

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
