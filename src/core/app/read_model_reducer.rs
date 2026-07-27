use super::{
    DirectionsMaintenanceLoadCorrelation, ParallelPeekLoadCorrelation,
    QueueAuthorityLoadCorrelation, ReviewCenterLoadCorrelation,
};

#[derive(Debug, Clone)]
pub(super) struct ReadModelFeatureReducer {
    next_parallel_peek_load_generation: u64,
    active_parallel_peek_load: Option<ParallelPeekLoadCorrelation>,
    next_review_center_load_generation: u64,
    active_review_center_load: Option<ReviewCenterLoadCorrelation>,
    next_queue_authority_load_generation: u64,
    active_queue_authority_load: Option<QueueAuthorityLoadCorrelation>,
    next_directions_maintenance_load_generation: u64,
    active_directions_maintenance_load: Option<DirectionsMaintenanceLoadCorrelation>,
}

impl ReadModelFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            next_parallel_peek_load_generation: 1,
            active_parallel_peek_load: None,
            next_review_center_load_generation: 1,
            active_review_center_load: None,
            next_queue_authority_load_generation: 1,
            active_queue_authority_load: None,
            next_directions_maintenance_load_generation: 1,
            active_directions_maintenance_load: None,
        }
    }

    pub(super) fn begin_parallel_peek_load(
        &mut self,
        requested_thread_id: impl Into<String>,
    ) -> ParallelPeekLoadCorrelation {
        let correlation = ParallelPeekLoadCorrelation::new(
            take_generation(
                &mut self.next_parallel_peek_load_generation,
                "parallel peek load",
            ),
            requested_thread_id,
        );
        self.active_parallel_peek_load = Some(correlation.clone());
        correlation
    }

    pub(super) fn accept_parallel_peek_load(
        &mut self,
        correlation: &ParallelPeekLoadCorrelation,
    ) -> bool {
        accept_exact(&mut self.active_parallel_peek_load, correlation)
    }

    pub(super) fn begin_review_center_load(
        &mut self,
        workspace_directory: impl Into<String>,
        active_thread_id: Option<String>,
    ) -> ReviewCenterLoadCorrelation {
        let correlation = ReviewCenterLoadCorrelation::new(
            take_generation(
                &mut self.next_review_center_load_generation,
                "review center load",
            ),
            workspace_directory,
            active_thread_id,
        );
        self.active_review_center_load = Some(correlation.clone());
        correlation
    }

    pub(super) fn accept_review_center_load(
        &mut self,
        correlation: &ReviewCenterLoadCorrelation,
    ) -> bool {
        accept_exact(&mut self.active_review_center_load, correlation)
    }

    pub(super) fn begin_queue_authority_load(
        &mut self,
        workspace_directory: impl Into<String>,
        active_thread_id: Option<String>,
    ) -> QueueAuthorityLoadCorrelation {
        let correlation = QueueAuthorityLoadCorrelation::new(
            take_generation(
                &mut self.next_queue_authority_load_generation,
                "queue authority load",
            ),
            workspace_directory,
            active_thread_id,
        );
        self.active_queue_authority_load = Some(correlation.clone());
        correlation
    }

    pub(super) fn accept_queue_authority_load(
        &mut self,
        correlation: &QueueAuthorityLoadCorrelation,
    ) -> bool {
        accept_exact(&mut self.active_queue_authority_load, correlation)
    }

    pub(super) fn begin_directions_maintenance_load(
        &mut self,
        workspace_directory: impl Into<String>,
    ) -> DirectionsMaintenanceLoadCorrelation {
        let correlation = DirectionsMaintenanceLoadCorrelation::new(
            take_generation(
                &mut self.next_directions_maintenance_load_generation,
                "directions maintenance load",
            ),
            workspace_directory,
        );
        self.active_directions_maintenance_load = Some(correlation.clone());
        correlation
    }

    pub(super) fn accept_directions_maintenance_load(
        &mut self,
        correlation: &DirectionsMaintenanceLoadCorrelation,
    ) -> bool {
        accept_exact(&mut self.active_directions_maintenance_load, correlation)
    }
}

impl Default for ReadModelFeatureReducer {
    fn default() -> Self {
        Self::new()
    }
}

fn accept_exact<T: PartialEq>(active: &mut Option<T>, correlation: &T) -> bool {
    if active.as_ref() != Some(correlation) {
        return false;
    }
    *active = None;
    true
}

fn take_generation(next_generation: &mut u64, operation: &str) -> u64 {
    let generation = *next_generation;
    *next_generation = generation
        .checked_add(1)
        .unwrap_or_else(|| panic!("{operation} generation exhausted"));
    generation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_peek_rejects_stale_duplicate_and_same_target_aba_completions() {
        let mut reducer = ReadModelFeatureReducer::new();
        let stale = reducer.begin_parallel_peek_load("thread-1");
        let active = reducer.begin_parallel_peek_load("thread-1");

        assert_eq!(stale.generation, 1);
        assert_eq!(active.generation, 2);
        assert!(
            !reducer.accept_parallel_peek_load(&stale),
            "ABA-stale completion must fail"
        );
        assert!(reducer.accept_parallel_peek_load(&active));
        assert!(
            !reducer.accept_parallel_peek_load(&active),
            "duplicate completion must fail"
        );
    }

    #[test]
    fn review_center_rejects_stale_duplicate_and_same_target_aba_completions() {
        let mut reducer = ReadModelFeatureReducer::new();
        let stale = reducer.begin_review_center_load("/workspace", Some("thread-1".to_string()));
        let active = reducer.begin_review_center_load("/workspace", Some("thread-1".to_string()));

        assert_eq!(stale.generation, 1);
        assert_eq!(active.generation, 2);
        assert!(
            !reducer.accept_review_center_load(&stale),
            "ABA-stale completion must fail"
        );
        assert!(reducer.accept_review_center_load(&active));
        assert!(
            !reducer.accept_review_center_load(&active),
            "duplicate completion must fail"
        );
    }

    #[test]
    fn queue_authority_rejects_stale_duplicate_and_same_target_aba_completions() {
        let mut reducer = ReadModelFeatureReducer::new();
        let stale = reducer.begin_queue_authority_load("/workspace", Some("thread-1".to_string()));
        let active = reducer.begin_queue_authority_load("/workspace", Some("thread-1".to_string()));

        assert_eq!(stale.generation, 1);
        assert_eq!(active.generation, 2);
        assert!(
            !reducer.accept_queue_authority_load(&stale),
            "ABA-stale completion must fail"
        );
        assert!(reducer.accept_queue_authority_load(&active));
        assert!(
            !reducer.accept_queue_authority_load(&active),
            "duplicate completion must fail"
        );
    }

    #[test]
    fn directions_rejects_stale_duplicate_and_same_target_aba_completions() {
        let mut reducer = ReadModelFeatureReducer::new();
        let stale = reducer.begin_directions_maintenance_load("/workspace");
        let active = reducer.begin_directions_maintenance_load("/workspace");

        assert_eq!(stale.generation, 1);
        assert_eq!(active.generation, 2);
        assert!(
            !reducer.accept_directions_maintenance_load(&stale),
            "ABA-stale completion must fail"
        );
        assert!(reducer.accept_directions_maintenance_load(&active));
        assert!(
            !reducer.accept_directions_maintenance_load(&active),
            "duplicate completion must fail"
        );
    }

    #[test]
    fn read_model_generation_overflow_messages_remain_stable() {
        assert_generation_overflow(
            |reducer| {
                reducer.next_parallel_peek_load_generation = u64::MAX;
                reducer.begin_parallel_peek_load("thread-1");
            },
            "parallel peek load generation exhausted",
        );
        assert_generation_overflow(
            |reducer| {
                reducer.next_review_center_load_generation = u64::MAX;
                reducer.begin_review_center_load("/workspace", None);
            },
            "review center load generation exhausted",
        );
        assert_generation_overflow(
            |reducer| {
                reducer.next_queue_authority_load_generation = u64::MAX;
                reducer.begin_queue_authority_load("/workspace", None);
            },
            "queue authority load generation exhausted",
        );
        assert_generation_overflow(
            |reducer| {
                reducer.next_directions_maintenance_load_generation = u64::MAX;
                reducer.begin_directions_maintenance_load("/workspace");
            },
            "directions maintenance load generation exhausted",
        );
    }

    fn assert_generation_overflow(
        mutate: impl FnOnce(&mut ReadModelFeatureReducer),
        expected: &str,
    ) {
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mutate(&mut ReadModelFeatureReducer::new());
        }))
        .expect_err("generation overflow must fail closed");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied());

        assert_eq!(message, Some(expected));
    }
}
