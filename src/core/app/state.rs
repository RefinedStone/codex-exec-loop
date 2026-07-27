use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationRuntimeSnapshot, ConversationSnapshot,
    ParallelModeProjection, RevisionedPlanningParallelProjection, SessionCatalogReadySnapshot,
    SessionCatalogSnapshot, StartupReadySnapshot, StartupSnapshot,
};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::RuntimeProjection;
use crate::domain::recent_sessions::{SessionCatalog, SessionRenameRequest};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AppState {
    // One immutable read-model authority backs explicit reads and dispatch outcomes. Mutations
    // compare before Arc::make_mut so no-op inputs preserve allocation identity and avoid cloning
    // loaded conversation, catalog, or parallel payloads.
    current: Arc<AppSnapshot>,
}

impl AppState {
    pub(super) fn new() -> Self {
        Self {
            current: Arc::new(AppSnapshot::initial()),
        }
    }

    pub(super) fn snapshot(&self) -> AppSnapshot {
        self.current.as_ref().clone()
    }

    pub(super) fn shared_snapshot(&self) -> Arc<AppSnapshot> {
        Arc::clone(&self.current)
    }

    pub(super) fn revisioned_planning_parallel_projection(
        &self,
    ) -> RevisionedPlanningParallelProjection {
        RevisionedPlanningParallelProjection {
            revision: self.current.revision,
            planning_parallel: self.current.planning_parallel.clone(),
        }
    }

    pub(super) fn parallel_mode_projection(&self) -> ParallelModeProjection {
        self.current.planning_parallel.parallel_mode.clone()
    }

    pub(super) fn mark_startup_loading(&mut self) {
        let current = Arc::make_mut(&mut self.current);
        current.startup = StartupSnapshot::Loading;
        current.revision += 1;
    }

    pub(super) fn apply_startup_result(
        &mut self,
        result: Result<Box<StartupReadySnapshot>, String>,
    ) {
        let current = Arc::make_mut(&mut self.current);
        current.startup = match result {
            Ok(ready) => StartupSnapshot::Ready(ready),
            Err(message) => StartupSnapshot::Failed { message },
        };
        current.revision += 1;
    }

    pub(super) fn mark_session_catalog_loading(&mut self) {
        let current = Arc::make_mut(&mut self.current);
        current.session_catalog = SessionCatalogSnapshot::Loading;
        current.revision += 1;
    }

    pub(super) fn apply_session_catalog_result(
        &mut self,
        result: Result<SessionCatalogReadySnapshot, String>,
    ) {
        let current = Arc::make_mut(&mut self.current);
        current.session_catalog = match result {
            Ok(ready) => SessionCatalogSnapshot::Ready(ready),
            Err(message) => SessionCatalogSnapshot::Failed { message },
        };
        current.revision += 1;
    }

    pub(super) fn apply_session_rename(&mut self, request: &SessionRenameRequest) -> bool {
        let catalog_changed = if let SessionCatalogSnapshot::Ready(ready) =
            &self.current.session_catalog
            && let SessionCatalog::Ready {
                recent_sessions, ..
            } = ready.catalog.as_ref()
        {
            recent_sessions
                .items
                .iter()
                .find(|session| session.id == request.thread_id)
                .is_some_and(|session| session.name.as_deref() != Some(request.name.as_str()))
        } else {
            false
        };
        let conversation_changed =
            if let ConversationSnapshot::Ready(ready) = &self.current.conversation {
                ready.thread_id == request.thread_id
                    && (ready.title != request.name || ready.conversation.title != request.name)
            } else {
                false
            };
        if !catalog_changed && !conversation_changed {
            return false;
        }

        let current = Arc::make_mut(&mut self.current);
        if catalog_changed
            && let SessionCatalogSnapshot::Ready(ready) = &mut current.session_catalog
            && let SessionCatalog::Ready {
                recent_sessions, ..
            } = ready.catalog.as_mut()
            && let Some(session) = recent_sessions
                .items
                .iter_mut()
                .find(|session| session.id == request.thread_id)
        {
            session.name = Some(request.name.clone());
        }
        if conversation_changed
            && let ConversationSnapshot::Ready(ready) = &mut current.conversation
        {
            ready.title = request.name.clone();
            ready.conversation.title = request.name.clone();
        }
        current.revision += 1;
        true
    }

    pub(super) fn mark_conversation_loading(&mut self) {
        let current = Arc::make_mut(&mut self.current);
        current.conversation = ConversationSnapshot::Loading;
        current.revision += 1;
    }

    pub(super) fn apply_conversation_result(
        &mut self,
        result: Result<Box<ConversationReadySnapshot>, String>,
    ) {
        let loaded_successfully = result.is_ok();
        let current = Arc::make_mut(&mut self.current);
        current.conversation = match result {
            Ok(ready) => ConversationSnapshot::Ready(ready),
            Err(message) => ConversationSnapshot::Failed { message },
        };
        if loaded_successfully {
            current
                .planning_parallel
                .clear_planning_runtime_projection();
        }
        current.revision += 1;
    }

    pub(super) fn reset_conversation(&mut self) {
        let current = Arc::make_mut(&mut self.current);
        current.conversation = ConversationSnapshot::Idle;
        current
            .planning_parallel
            .clear_planning_runtime_projection();
        current.revision += 1;
    }

    pub(super) fn apply_conversation_runtime_snapshot(
        &mut self,
        snapshot: ConversationRuntimeSnapshot,
    ) -> bool {
        if self.current.conversation_runtime == snapshot {
            return false;
        }
        let current = Arc::make_mut(&mut self.current);
        current.conversation_runtime = snapshot;
        current.revision += 1;
        true
    }

    pub(super) fn apply_planning_runtime_projection(
        &mut self,
        workspace_directory: String,
        projection: Box<RuntimeProjection>,
    ) -> bool {
        if self
            .current
            .planning_parallel
            .planning_runtime_workspace_directory
            .as_ref()
            == Some(&workspace_directory)
            && self.current.planning_parallel.planning_runtime == projection
        {
            return false;
        }
        let current = Arc::make_mut(&mut self.current);
        current
            .planning_parallel
            .planning_runtime_workspace_directory = Some(workspace_directory);
        current.planning_parallel.planning_runtime = projection;
        current.revision += 1;
        true
    }

    pub(super) fn planning_runtime_workspace_directory(&self) -> Option<&str> {
        self.current
            .planning_parallel
            .planning_runtime_workspace_directory
            .as_deref()
    }

    pub(super) fn apply_parallel_readiness_projection(
        &mut self,
        snapshot: Option<Box<ParallelModeReadinessSnapshot>>,
    ) -> bool {
        if self.current.planning_parallel.parallel_mode.readiness == snapshot {
            return false;
        }
        let current = Arc::make_mut(&mut self.current);
        current.planning_parallel.parallel_mode.readiness = snapshot;
        current.revision += 1;
        true
    }

    pub(super) fn apply_parallel_supervisor_projection(
        &mut self,
        snapshot: Option<Box<ParallelModeSupervisorSnapshot>>,
    ) -> bool {
        if self.current.planning_parallel.parallel_mode.supervisor == snapshot {
            return false;
        }
        let current = Arc::make_mut(&mut self.current);
        current.planning_parallel.parallel_mode.supervisor = snapshot;
        current.revision += 1;
        true
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::{
        AppEvent, ConversationSnapshot, CoreDispatchOutcome, PlanningParallelProjection,
        SessionCatalogSnapshot, StartupSnapshot,
    };

    #[test]
    fn new_state_projects_initial_snapshot() {
        assert_eq!(AppState::new().snapshot(), AppSnapshot::initial());
        assert_eq!(AppState::default().snapshot(), AppSnapshot::initial());
    }

    #[test]
    fn shared_snapshot_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<AppSnapshot>();
        assert_send_sync::<Arc<AppSnapshot>>();
        assert_send_sync::<CoreDispatchOutcome>();
        assert_send_sync::<AppEvent>();
    }

    #[test]
    fn shared_snapshot_forks_only_when_state_changes() {
        let mut state = AppState::new();
        let initial = state.shared_snapshot();

        assert!(Arc::ptr_eq(&initial, &state.shared_snapshot()));

        state.mark_startup_loading();
        let loading = state.shared_snapshot();

        assert!(!Arc::ptr_eq(&initial, &loading));
        assert_eq!(initial.as_ref(), &AppSnapshot::initial());
        assert_eq!(loading.revision, 1);
        assert_eq!(loading.startup, StartupSnapshot::Loading);
    }

    #[test]
    fn identical_planning_projection_keeps_shared_snapshot_identity() {
        let mut state = AppState::new();
        let projection = Box::new(RuntimeProjection::invalid("blocked"));
        assert!(
            state.apply_planning_runtime_projection(
                "/tmp/workspace".to_string(),
                projection.clone(),
            )
        );
        let projected = state.shared_snapshot();

        assert!(
            !state.apply_planning_runtime_projection("/tmp/workspace".to_string(), projection,)
        );

        assert!(Arc::ptr_eq(&projected, &state.shared_snapshot()));
        assert_eq!(projected.revision, 1);
    }

    #[test]
    fn startup_loading_advances_revision() {
        let mut state = AppState::new();

        state.mark_startup_loading();

        assert_eq!(
            state.snapshot(),
            AppSnapshot {
                revision: 1,
                startup: StartupSnapshot::Loading,
                session_catalog: SessionCatalogSnapshot::Idle,
                conversation: ConversationSnapshot::Idle,
                conversation_runtime: ConversationRuntimeSnapshot::initial(),
                planning_parallel: PlanningParallelProjection::initial(),
            }
        );
    }

    #[test]
    fn revisioned_planning_parallel_projection_is_a_coherent_narrow_state_slice() {
        let mut state = AppState::new();
        state.mark_startup_loading();
        state.mark_session_catalog_loading();
        state.mark_conversation_loading();
        let runtime_projection = RuntimeProjection::invalid("blocked");
        state.apply_planning_runtime_projection(
            "/tmp/workspace".to_string(),
            Box::new(runtime_projection.clone()),
        );

        assert_eq!(
            state.revisioned_planning_parallel_projection(),
            RevisionedPlanningParallelProjection {
                revision: 4,
                planning_parallel: PlanningParallelProjection {
                    planning_runtime_workspace_directory: Some("/tmp/workspace".to_string()),
                    planning_runtime: Box::new(runtime_projection),
                    parallel_mode: ParallelModeProjection::default(),
                },
            }
        );
    }

    #[test]
    fn session_loading_advances_revision() {
        let mut state = AppState::new();

        state.mark_session_catalog_loading();

        assert_eq!(
            state.snapshot(),
            AppSnapshot {
                revision: 1,
                startup: StartupSnapshot::Idle,
                session_catalog: SessionCatalogSnapshot::Loading,
                conversation: ConversationSnapshot::Idle,
                conversation_runtime: ConversationRuntimeSnapshot::initial(),
                planning_parallel: PlanningParallelProjection::initial(),
            }
        );
    }

    #[test]
    fn conversation_loading_advances_revision() {
        let mut state = AppState::new();

        state.mark_conversation_loading();

        assert_eq!(
            state.snapshot(),
            AppSnapshot {
                revision: 1,
                startup: StartupSnapshot::Idle,
                session_catalog: SessionCatalogSnapshot::Idle,
                conversation: ConversationSnapshot::Loading,
                conversation_runtime: ConversationRuntimeSnapshot::initial(),
                planning_parallel: PlanningParallelProjection::initial(),
            }
        );
    }

    #[test]
    fn resetting_conversation_discards_the_previous_workspace_projection() {
        let mut state = AppState::new();
        state.apply_planning_runtime_projection(
            "/tmp/old".to_string(),
            Box::new(RuntimeProjection::invalid("old workspace")),
        );

        state.reset_conversation();

        assert_eq!(
            *state.snapshot().planning_parallel.planning_runtime,
            RuntimeProjection::uninitialized()
        );
        assert!(
            state
                .snapshot()
                .planning_parallel
                .planning_runtime_workspace_directory
                .is_none()
        );
    }

    #[test]
    fn failed_results_advance_revision_and_project_messages() {
        let mut state = AppState::new();

        state.apply_startup_result(Err("startup failed".to_string()));
        state.apply_session_catalog_result(Err("catalog failed".to_string()));
        state.apply_conversation_result(Err("conversation failed".to_string()));

        assert_eq!(
            state.snapshot(),
            AppSnapshot {
                revision: 3,
                startup: StartupSnapshot::Failed {
                    message: "startup failed".to_string(),
                },
                session_catalog: SessionCatalogSnapshot::Failed {
                    message: "catalog failed".to_string(),
                },
                conversation: ConversationSnapshot::Failed {
                    message: "conversation failed".to_string(),
                },
                conversation_runtime: ConversationRuntimeSnapshot::initial(),
                planning_parallel: PlanningParallelProjection::initial(),
            }
        );
    }
}
