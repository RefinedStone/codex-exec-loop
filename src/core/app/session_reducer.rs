use super::{
    ConversationLoadCorrelation, SessionCatalogLoadCorrelation, SessionCatalogLoadIntent,
    SessionCatalogLoadMode, SessionCatalogSnapshot, SessionRenameAdmission,
    SessionRenameCorrelation,
};
use crate::domain::recent_sessions::SessionRenameRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SessionCatalogLoadReduction {
    Unchanged,
    Deferred,
    Started {
        correlation: SessionCatalogLoadCorrelation,
    },
}

#[derive(Debug, Clone)]
pub(super) struct SessionFeatureReducer {
    next_catalog_load_generation: u64,
    active_catalog_load: Option<SessionCatalogLoadCorrelation>,
    next_rename_generation: u64,
    active_rename: Option<SessionRenameCorrelation>,
    deferred_catalog_load: Option<SessionCatalogLoadIntent>,
}

impl SessionFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            next_catalog_load_generation: 1,
            active_catalog_load: None,
            next_rename_generation: 1,
            active_rename: None,
            deferred_catalog_load: None,
        }
    }

    pub(super) fn reduce_catalog_load(
        &mut self,
        intent: SessionCatalogLoadIntent,
        current_catalog: &SessionCatalogSnapshot,
    ) -> SessionCatalogLoadReduction {
        let has_active_load = self.active_catalog_load.is_some();
        if self
            .active_catalog_load
            .as_ref()
            .is_some_and(|active| active.matches_target(&intent))
        {
            return SessionCatalogLoadReduction::Unchanged;
        }
        if intent.mode == SessionCatalogLoadMode::EnsureLoaded
            && !has_active_load
            && !matches!(current_catalog, SessionCatalogSnapshot::Idle)
        {
            return SessionCatalogLoadReduction::Unchanged;
        }
        if self.active_rename.is_some() {
            self.deferred_catalog_load = Some(intent);
            return SessionCatalogLoadReduction::Deferred;
        }

        let correlation = SessionCatalogLoadCorrelation::new(
            take_generation(
                &mut self.next_catalog_load_generation,
                "session catalog load",
            ),
            intent.limit,
            intent.workspace_directory,
        );
        self.active_catalog_load = Some(correlation.clone());
        SessionCatalogLoadReduction::Started { correlation }
    }

    pub(super) fn reduce_rename(
        &mut self,
        request: SessionRenameRequest,
        conversation_load_blocker: Option<ConversationLoadCorrelation>,
    ) -> SessionRenameAdmission {
        if let Some(active_correlation) = self.active_rename.clone() {
            return SessionRenameAdmission::RejectedActive { active_correlation };
        }
        if let Some(active_correlation) = self.active_catalog_load.clone() {
            return SessionRenameAdmission::RejectedCatalogLoading { active_correlation };
        }
        if let Some(active_correlation) = conversation_load_blocker {
            return SessionRenameAdmission::RejectedConversationLoading { active_correlation };
        }

        let correlation = SessionRenameCorrelation::new(
            take_generation(&mut self.next_rename_generation, "session rename"),
            request,
        );
        self.active_rename = Some(correlation.clone());
        SessionRenameAdmission::Accepted { correlation }
    }

    pub(super) fn accept_catalog_completion(
        &mut self,
        correlation: &SessionCatalogLoadCorrelation,
    ) -> bool {
        if self.active_catalog_load.as_ref() != Some(correlation) {
            return false;
        }
        self.active_catalog_load = None;
        true
    }

    pub(super) fn accept_rename_completion(
        &mut self,
        correlation: &SessionRenameCorrelation,
    ) -> bool {
        if self.active_rename.as_ref() != Some(correlation) {
            return false;
        }
        self.active_rename = None;
        true
    }

    pub(super) fn active_rename_matches_thread(&self, thread_id: &str) -> bool {
        self.active_rename
            .as_ref()
            .is_some_and(|rename| rename.request.thread_id == thread_id)
    }

    pub(super) fn has_active_rename(&self) -> bool {
        self.active_rename.is_some()
    }

    pub(super) fn take_deferred_catalog_load(&mut self) -> Option<SessionCatalogLoadIntent> {
        self.deferred_catalog_load.take()
    }

    #[cfg(test)]
    pub(super) fn active_catalog_load_for_test(&self) -> Option<&SessionCatalogLoadCorrelation> {
        self.active_catalog_load.as_ref()
    }

    #[cfg(test)]
    pub(super) fn active_rename_for_test(&self) -> Option<&SessionRenameCorrelation> {
        self.active_rename.as_ref()
    }

    #[cfg(test)]
    pub(super) fn has_deferred_catalog_load_for_test(&self) -> bool {
        self.deferred_catalog_load.is_some()
    }
}

impl Default for SessionFeatureReducer {
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

    fn ensure(limit: usize, workspace: &str) -> SessionCatalogLoadIntent {
        SessionCatalogLoadIntent::ensure_loaded(limit, workspace)
    }

    fn refresh(limit: usize, workspace: &str) -> SessionCatalogLoadIntent {
        SessionCatalogLoadIntent::refresh(limit, workspace)
    }

    fn idle_catalog() -> SessionCatalogSnapshot {
        SessionCatalogSnapshot::Idle
    }

    fn settled_catalog() -> SessionCatalogSnapshot {
        SessionCatalogSnapshot::Failed {
            message: "unavailable".to_string(),
        }
    }

    fn catalog(generation: u64, limit: usize, workspace: &str) -> SessionCatalogLoadCorrelation {
        SessionCatalogLoadCorrelation::new(generation, limit, workspace)
    }

    fn rename(generation: u64, thread_id: &str, name: &str) -> SessionRenameCorrelation {
        SessionRenameCorrelation::new(generation, SessionRenameRequest::new(thread_id, name))
    }

    #[test]
    fn ensure_is_idle_only_but_refresh_restarts_settled_catalogs() {
        let mut reducer = SessionFeatureReducer::new();

        assert_eq!(
            reducer.reduce_catalog_load(ensure(10, "/workspace"), &settled_catalog()),
            SessionCatalogLoadReduction::Unchanged
        );
        assert_eq!(
            reducer.reduce_catalog_load(refresh(10, "/workspace"), &settled_catalog()),
            SessionCatalogLoadReduction::Started {
                correlation: catalog(1, 10, "/workspace"),
            }
        );
    }

    #[test]
    fn identical_catalog_target_coalesces_without_consuming_generation() {
        let mut reducer = SessionFeatureReducer::new();
        reducer.reduce_catalog_load(refresh(10, "/workspace"), &idle_catalog());

        assert_eq!(
            reducer.reduce_catalog_load(ensure(10, "/workspace"), &idle_catalog()),
            SessionCatalogLoadReduction::Unchanged
        );
        assert_eq!(
            reducer.reduce_catalog_load(refresh(20, "/workspace"), &idle_catalog()),
            SessionCatalogLoadReduction::Started {
                correlation: catalog(2, 20, "/workspace"),
            }
        );
    }

    #[test]
    fn rename_admission_owns_session_blockers_and_accepts_root_context() {
        let mut active_rename = SessionFeatureReducer::new();
        assert_eq!(
            active_rename.reduce_rename(SessionRenameRequest::new("thread-1", "Renamed"), None,),
            SessionRenameAdmission::Accepted {
                correlation: rename(1, "thread-1", "Renamed"),
            }
        );
        assert!(matches!(
            active_rename.reduce_rename(SessionRenameRequest::new("thread-2", "Other"), None,),
            SessionRenameAdmission::RejectedActive { .. }
        ));

        let mut catalog_loading = SessionFeatureReducer::new();
        catalog_loading.reduce_catalog_load(refresh(10, "/workspace"), &idle_catalog());
        assert_eq!(
            catalog_loading.reduce_rename(SessionRenameRequest::new("thread-1", "Renamed"), None,),
            SessionRenameAdmission::RejectedCatalogLoading {
                active_correlation: catalog(1, 10, "/workspace"),
            }
        );

        let mut conversation_loading = SessionFeatureReducer::new();
        let blocker = ConversationLoadCorrelation::new(7, "thread-1");
        assert_eq!(
            conversation_loading.reduce_rename(
                SessionRenameRequest::new("thread-1", "Renamed"),
                Some(blocker.clone()),
            ),
            SessionRenameAdmission::RejectedConversationLoading {
                active_correlation: blocker,
            }
        );
    }

    #[test]
    fn completions_require_the_exact_active_correlation_once_across_aba() {
        let mut reducer = SessionFeatureReducer::new();
        reducer.reduce_catalog_load(refresh(10, "/workspace-a"), &idle_catalog());
        reducer.reduce_catalog_load(refresh(20, "/workspace-b"), &idle_catalog());
        reducer.reduce_catalog_load(refresh(10, "/workspace-a"), &idle_catalog());

        assert!(!reducer.accept_catalog_completion(&catalog(1, 10, "/workspace-a")));
        assert!(!reducer.accept_catalog_completion(&catalog(2, 20, "/workspace-b")));
        assert!(reducer.accept_catalog_completion(&catalog(3, 10, "/workspace-a")));
        assert!(!reducer.accept_catalog_completion(&catalog(3, 10, "/workspace-a")));

        let accepted = reducer.reduce_rename(SessionRenameRequest::new("thread-1", "First"), None);
        let SessionRenameAdmission::Accepted { correlation: first } = accepted else {
            panic!("rename should be accepted");
        };
        assert!(reducer.accept_rename_completion(&first));
        let accepted = reducer.reduce_rename(SessionRenameRequest::new("thread-1", "Second"), None);
        let SessionRenameAdmission::Accepted {
            correlation: second,
        } = accepted
        else {
            panic!("second rename should be accepted");
        };
        assert!(!reducer.accept_rename_completion(&first));
        assert!(reducer.accept_rename_completion(&second));
        assert!(!reducer.accept_rename_completion(&second));
    }

    #[test]
    fn rename_defers_only_the_latest_refresh_intent() {
        let mut reducer = SessionFeatureReducer::new();
        reducer.reduce_rename(SessionRenameRequest::new("thread-1", "Renamed"), None);

        assert_eq!(
            reducer.reduce_catalog_load(ensure(10, "/ignored"), &settled_catalog()),
            SessionCatalogLoadReduction::Unchanged
        );
        assert_eq!(
            reducer.reduce_catalog_load(refresh(10, "/first"), &settled_catalog()),
            SessionCatalogLoadReduction::Deferred
        );
        assert_eq!(
            reducer.reduce_catalog_load(refresh(20, "/latest"), &settled_catalog()),
            SessionCatalogLoadReduction::Deferred
        );
        assert_eq!(
            reducer.take_deferred_catalog_load(),
            Some(refresh(20, "/latest"))
        );
        assert_eq!(reducer.take_deferred_catalog_load(), None);
    }
}
