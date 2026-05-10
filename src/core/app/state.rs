use super::{
    AppSnapshot, ConversationReadySnapshot, ConversationState, SessionCatalogReadySnapshot,
    SessionCatalogState, StartupReadySnapshot, StartupState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    revision: u64,
    startup: StartupState,
    session_catalog: SessionCatalogState,
    conversation: ConversationState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            revision: 0,
            startup: StartupState::Idle,
            session_catalog: SessionCatalogState::Idle,
            conversation: ConversationState::Idle,
        }
    }

    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            revision: self.revision,
            startup: self.startup.snapshot(),
            session_catalog: self.session_catalog.snapshot(),
            conversation: self.conversation.snapshot(),
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
            }
        );
    }
}
