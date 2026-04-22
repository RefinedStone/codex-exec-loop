use anyhow::Result;

use crate::domain::recent_sessions::RecentSessions;

pub trait SessionCatalogPort: Send + Sync {
    fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions>;
}
