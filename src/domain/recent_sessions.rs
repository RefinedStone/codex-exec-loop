use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone)]
pub struct RecentSessions {
    pub items: Vec<SessionSummary>,
    pub warnings: Vec<String>,
    pub next_cursor: Option<String>,
}
