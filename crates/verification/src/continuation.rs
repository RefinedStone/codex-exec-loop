/// Long verification only observes cancellation. Bounded authority commits
/// participate in the existing continuation/epoch serialization boundary.
pub trait TaskVerificationContinuation: Send + Sync {
    fn is_current(&self) -> bool;
    fn with_current_commit(&self, operation: &mut dyn FnMut()) -> bool;
}
