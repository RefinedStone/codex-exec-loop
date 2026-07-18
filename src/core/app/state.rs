use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationState, PlanningParallelProjection,
    SessionCatalogReadySnapshot, SessionCatalogState, StartupReadySnapshot, StartupState,
};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::RuntimeProjection;
use crate::domain::recent_sessions::{SessionCatalog, SessionRenameRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    revision: u64,
    startup: StartupState,
    session_catalog: SessionCatalogState,
    conversation: ConversationState,
    planning_parallel: PlanningParallelProjection,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            revision: 0,
            startup: StartupState::Idle,
            session_catalog: SessionCatalogState::Idle,
            conversation: ConversationState::Idle,
            planning_parallel: PlanningParallelProjection::initial(),
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            revision: self.revision,
            startup: self.startup.snapshot(),
            session_catalog: self.session_catalog.snapshot(),
            conversation: self.conversation.snapshot(),
            planning_parallel: self.planning_parallel.clone(),
        }
    }

    pub fn mark_startup_loading(&mut self) {
        self.startup = StartupState::Loading;
        self.advance_revision();
    }

    pub fn apply_startup_result(&mut self, result: Result<Box<StartupReadySnapshot>, String>) {
        self.startup = match result {
            Ok(ready) => StartupState::Ready(ready),
            Err(message) => StartupState::Failed(message),
        };
        self.advance_revision();
    }

    pub fn mark_session_catalog_loading(&mut self) {
        self.session_catalog = SessionCatalogState::Loading;
        self.advance_revision();
    }

    pub fn apply_session_catalog_result(
        &mut self,
        result: Result<SessionCatalogReadySnapshot, String>,
    ) {
        self.session_catalog = match result {
            Ok(ready) => SessionCatalogState::Ready(ready),
            Err(message) => SessionCatalogState::Failed(message),
        };
        self.advance_revision();
    }

    pub fn apply_session_rename(&mut self, request: &SessionRenameRequest) -> bool {
        let mut changed = false;
        if let SessionCatalogState::Ready(ready) = &mut self.session_catalog
            && let SessionCatalog::Ready {
                recent_sessions, ..
            } = ready.catalog.as_mut()
            && let Some(session) = recent_sessions
                .items
                .iter_mut()
                .find(|session| session.id == request.thread_id)
            && session.name.as_deref() != Some(request.name.as_str())
        {
            session.name = Some(request.name.clone());
            changed = true;
        }
        if let super::ConversationState::Ready(ready) = &mut self.conversation
            && ready.thread_id == request.thread_id
        {
            if ready.title != request.name {
                ready.title = request.name.clone();
                changed = true;
            }
            if ready.conversation.title != request.name {
                ready.conversation.title = request.name.clone();
                changed = true;
            }
        }
        if changed {
            self.advance_revision();
        }
        changed
    }

    pub fn mark_conversation_loading(&mut self) {
        self.conversation = ConversationState::Loading;
        self.advance_revision();
    }

    pub fn apply_conversation_result(
        &mut self,
        result: Result<Box<ConversationReadySnapshot>, String>,
    ) {
        self.conversation = match result {
            Ok(ready) => ConversationState::Ready(ready),
            Err(message) => ConversationState::Failed(message),
        };
        self.advance_revision();
    }

    pub fn reset_conversation(&mut self) {
        self.conversation = ConversationState::Idle;
        self.advance_revision();
    }

    pub fn apply_planning_runtime_projection(
        &mut self,
        projection: Box<RuntimeProjection>,
    ) -> bool {
        let changed = self
            .planning_parallel
            .apply_planning_runtime_projection(projection);
        if changed {
            self.advance_revision();
        }
        changed
    }

    pub fn apply_parallel_readiness_projection(
        &mut self,
        snapshot: Option<Box<ParallelModeReadinessSnapshot>>,
    ) -> bool {
        let changed = self
            .planning_parallel
            .apply_parallel_readiness_snapshot(snapshot);
        if changed {
            self.advance_revision();
        }
        changed
    }

    pub fn apply_parallel_supervisor_projection(
        &mut self,
        snapshot: Option<Box<ParallelModeSupervisorSnapshot>>,
    ) -> bool {
        let changed = self
            .planning_parallel
            .apply_parallel_supervisor_snapshot(snapshot);
        if changed {
            self.advance_revision();
        }
        changed
    }

    fn advance_revision(&mut self) {
        self.revision += 1;
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
    use crate::core::app::{ConversationSnapshot, SessionCatalogSnapshot, StartupSnapshot};

    #[test]
    fn new_state_projects_initial_snapshot() {
        assert_eq!(AppState::new().snapshot(), AppSnapshot::initial());
        assert_eq!(AppState::default().snapshot(), AppSnapshot::initial());
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
                planning_parallel: PlanningParallelProjection::initial(),
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
                planning_parallel: PlanningParallelProjection::initial(),
            }
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
                planning_parallel: PlanningParallelProjection::initial(),
            }
        );
    }
}
