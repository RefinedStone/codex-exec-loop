use crate::domain::conversation::ConversationTurnOptions;
use std::collections::VecDeque;

/// The thread that was active when an interactive default was selected.  The
/// selection belongs to this exact thread even if the operator changes
/// sessions before the background persistence effect completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPreferenceThreadTarget {
    pub workspace_directory: String,
    pub thread_id: String,
}

/// A request to persist the in-memory interactive conversation choice.  The
/// global TOML update and optional current-thread update are deliberately one
/// ordered effect, while their outcomes remain independently reportable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPreferencePersistenceRequest {
    /// `None` records a current-directory lookup failure without preventing an
    /// independently valid current-thread persistence attempt.
    pub global_workspace_directory: Option<String>,
    pub active_thread: Option<ConversationPreferenceThreadTarget>,
    pub options: ConversationTurnOptions,
}

/// Exact identity for an ordered persistence operation.  Generation prevents
/// an old asynchronous completion from settling a newer queued request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPreferencePersistenceCorrelation {
    pub generation: u64,
    pub request: ConversationPreferencePersistenceRequest,
}

/// The global and per-thread stores are independent failure domains.  A
/// successful in-memory choice is never rolled back because either write
/// fails, and the inbound adapter can explain each result precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPreferencePersistenceResult {
    pub global: Result<String, String>,
    pub current_thread: Option<Result<(), String>>,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationPreferenceFeatureReducer {
    next_generation: u64,
    active: Option<ConversationPreferencePersistenceCorrelation>,
    queued: VecDeque<ConversationPreferencePersistenceCorrelation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConversationPreferencePersistenceSettlement {
    pub(super) next: Option<ConversationPreferencePersistenceCorrelation>,
}

impl ConversationPreferenceFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
            queued: VecDeque::new(),
        }
    }

    /// Start one writer immediately or append the request behind the active
    /// writer.  Do not coalesce: model-picker A→B→A and repeated selections
    /// must retain their request order for both durable targets.
    pub(super) fn enqueue(
        &mut self,
        request: ConversationPreferencePersistenceRequest,
    ) -> Option<ConversationPreferencePersistenceCorrelation> {
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .expect("conversation preference persistence generation exhausted");
        let correlation = ConversationPreferencePersistenceCorrelation {
            generation,
            request,
        };
        if self.active.is_none() {
            self.active = Some(correlation.clone());
            Some(correlation)
        } else {
            self.queued.push_back(correlation);
            None
        }
    }

    pub(super) fn complete(
        &mut self,
        correlation: &ConversationPreferencePersistenceCorrelation,
    ) -> Option<ConversationPreferencePersistenceSettlement> {
        if self.active.as_ref() != Some(correlation) {
            return None;
        }
        self.active = None;
        let next = self.queued.pop_front();
        self.active = next.clone();
        Some(ConversationPreferencePersistenceSettlement { next })
    }
}

impl Default for ConversationPreferenceFeatureReducer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::ConversationReasoningEffort;

    #[test]
    fn coordinator_preserves_every_selection_in_request_order() {
        let mut coordinator = ConversationPreferenceFeatureReducer::new();
        let first = coordinator
            .enqueue(request("gpt-5.6-sol", "thread-a"))
            .expect("the first write should start");
        assert!(
            coordinator
                .enqueue(request("gpt-5.6-terra", "thread-a"))
                .is_none()
        );
        assert!(
            coordinator
                .enqueue(request("gpt-5.6-sol", "thread-b"))
                .is_none()
        );

        let second = coordinator
            .complete(&first)
            .expect("the active writer should settle")
            .next
            .expect("the next queued selection should start");
        assert_eq!(second.generation, 2);
        assert_eq!(
            second.request.options.model.as_deref(),
            Some("gpt-5.6-terra")
        );

        let third = coordinator
            .complete(&second)
            .expect("the second writer should settle")
            .next
            .expect("the final queued selection should start");
        assert_eq!(third.generation, 3);
        assert_eq!(
            third
                .request
                .active_thread
                .as_ref()
                .map(|target| target.thread_id.as_str()),
            Some("thread-b")
        );
        assert!(
            coordinator
                .complete(&third)
                .expect("the final writer should settle")
                .next
                .is_none()
        );
    }

    #[test]
    fn coordinator_rejects_stale_completion_before_settling_the_next_write() {
        let mut coordinator = ConversationPreferenceFeatureReducer::new();
        let first = coordinator
            .enqueue(request("gpt-5.6-sol", "thread-a"))
            .expect("the first write should start");
        coordinator.enqueue(request("gpt-5.6-terra", "thread-a"));
        let second = coordinator
            .complete(&first)
            .expect("the first writer should settle")
            .next
            .expect("the queued writer should start");

        assert!(coordinator.complete(&first).is_none());
        assert!(coordinator.complete(&second).is_some());
    }

    fn request(model: &str, thread_id: &str) -> ConversationPreferencePersistenceRequest {
        ConversationPreferencePersistenceRequest {
            global_workspace_directory: Some("/tmp/workspace".to_string()),
            active_thread: Some(ConversationPreferenceThreadTarget {
                workspace_directory: "/tmp/workspace".to_string(),
                thread_id: thread_id.to_string(),
            }),
            options: ConversationTurnOptions {
                model: Some(model.to_string()),
                reasoning_effort: Some(ConversationReasoningEffort::Medium),
            },
        }
    }
}
