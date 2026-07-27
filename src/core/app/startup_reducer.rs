use super::StartupCheckCorrelation;

#[derive(Debug, Clone)]
pub(super) struct StartupFeatureReducer {
    next_generation: u64,
    in_flight: Option<StartupCheckCorrelation>,
}

impl StartupFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            next_generation: 1,
            in_flight: None,
        }
    }

    pub(super) fn begin(
        &mut self,
        workspace_directory: impl Into<String>,
    ) -> StartupCheckCorrelation {
        let correlation = StartupCheckCorrelation::new(
            take_generation(&mut self.next_generation, "startup check"),
            workspace_directory,
        );
        self.in_flight = Some(correlation.clone());
        correlation
    }

    pub(super) fn accept(&mut self, correlation: &StartupCheckCorrelation) -> bool {
        if self.in_flight.as_ref() != Some(correlation) {
            return false;
        }
        self.in_flight = None;
        true
    }
}

impl Default for StartupFeatureReducer {
    fn default() -> Self {
        Self::new()
    }
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

    fn startup(generation: u64, workspace_directory: &str) -> StartupCheckCorrelation {
        StartupCheckCorrelation::new(generation, workspace_directory)
    }

    #[test]
    fn begin_uses_monotonic_generations_even_for_the_same_workspace() {
        let mut reducer = StartupFeatureReducer::new();

        assert_eq!(reducer.begin("/workspace"), startup(1, "/workspace"));
        assert_eq!(reducer.begin("/workspace"), startup(2, "/workspace"));
    }

    #[test]
    fn completion_requires_the_exact_active_correlation() {
        let mut reducer = StartupFeatureReducer::new();
        let stale = reducer.begin("/workspace");
        let active = reducer.begin("/workspace");

        assert!(!reducer.accept(&stale), "ABA-stale completion must fail");
        assert!(reducer.accept(&active), "exact completion must settle");
        assert!(
            !reducer.accept(&active),
            "duplicate completion must fail after settlement"
        );
    }

    #[test]
    #[should_panic(expected = "startup check generation exhausted")]
    fn generation_overflow_preserves_the_existing_failure_message() {
        let mut reducer = StartupFeatureReducer {
            next_generation: u64::MAX,
            in_flight: None,
        };

        reducer.begin("/workspace");
    }
}
