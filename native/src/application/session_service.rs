use anyhow::Result;

use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::infrastructure::app_server_client::{AppServerClient, ThreadListParams};

#[derive(Clone)]
pub struct SessionService {
    app_server_client: AppServerClient,
}

impl SessionService {
    pub fn new(app_server_client: AppServerClient) -> Self {
        Self { app_server_client }
    }

    pub fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions> {
        let mut connection = self.app_server_client.open_connection()?;
        connection.initialize()?;
        let thread_list = connection.list_threads(ThreadListParams {
            limit: Some(limit),
            ..ThreadListParams::default()
        })?;
        let warnings = connection.finish();

        let items = thread_list
            .data
            .into_iter()
            .map(SessionSummary::from)
            .collect::<Vec<_>>();

        Ok(RecentSessions {
            items,
            warnings,
            next_cursor: thread_list.next_cursor,
        })
    }
}
