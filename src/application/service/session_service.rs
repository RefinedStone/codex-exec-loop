use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};

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

    pub fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
        self.session_catalog_port.load_session_catalog(request)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};

    #[derive(Default)]
    struct FakeSessionCatalogPort {
        requests: Mutex<Vec<SessionCatalogRequest>>,
    }

    impl SessionCatalogPort for FakeSessionCatalogPort {
        fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
            self.requests
                .lock()
                .expect("session request mutex poisoned")
                .push(request);
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }
    }

    #[test]
    fn load_session_catalog_delegates_capability_request() {
        let port = Arc::new(FakeSessionCatalogPort::default());
        let service = SessionService::new(port.clone());

        service
            .load_session_catalog(SessionCatalogRequest::for_workspace(25, "/tmp/root"))
            .expect("load session catalog should succeed");

        assert_eq!(
            *port
                .requests
                .lock()
                .expect("session request mutex poisoned"),
            vec![SessionCatalogRequest::for_workspace(25, "/tmp/root")]
        );
    }
}
