use anyhow::Result;

use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};

pub trait SessionCatalogPort: Send + Sync {
    fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog>;
}
