use anyhow::Result;

use crate::domain::recent_sessions::SessionCatalog;

pub trait SessionCatalogPort: Send + Sync {
    fn load_recent_sessions(&self, limit: usize) -> Result<SessionCatalog>;
}
