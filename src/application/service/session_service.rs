use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::domain::recent_sessions::SessionCatalog;

#[derive(Clone)]
pub struct SessionService {
    session_catalog_port: Arc<dyn SessionCatalogPort>,
}

impl SessionService {
    pub fn new(session_catalog_port: Arc<dyn SessionCatalogPort>) -> Self {
        Self {
            session_catalog_port,
        }
    }

    pub fn load_recent_sessions(&self, limit: usize) -> Result<SessionCatalog> {
        self.session_catalog_port.load_recent_sessions(limit)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::domain::conversation::ConversationSnapshot;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};

    #[derive(Default)]
    struct FakeCodexAppServerPort {
        limits: Mutex<Vec<usize>>,
    }

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            unreachable!("startup context is not used in session service tests")
        }

        fn load_recent_sessions(&self, limit: usize) -> Result<SessionCatalog> {
            self.limits
                .lock()
                .expect("session limit mutex poisoned")
                .push(limit);
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }

        fn load_conversation_snapshot(&self, _thread_id: &str) -> Result<ConversationSnapshot> {
            unreachable!("conversation snapshots are not used in session service tests")
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            unreachable!("new-thread streaming is not used in session service tests")
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            unreachable!("turn streaming is not used in session service tests")
        }
    }

    #[test]
    fn load_recent_sessions_delegates_requested_limit() {
        let port = Arc::new(FakeCodexAppServerPort::default());
        let service = SessionService::new(port.clone());

        service
            .load_recent_sessions(25)
            .expect("load recent sessions should succeed");

        assert_eq!(
            *port.limits.lock().expect("session limit mutex poisoned"),
            vec![25]
        );
    }
}
